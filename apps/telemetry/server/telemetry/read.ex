defmodule Telemetry.Read do
  @moduledoc """
  What's kept, read: errors by kind, tools by how long they took, a trace whole, a metric over time and what AI
  cost, from OTLP laid flat (`Telemetry.Otlp`). Numbers are OpenTelemetry's histograms, with explicit buckets, which
  add up by their buckets and give quantiles within a bucket; times out of here are milliseconds.
  """

  @doc """
  The errors among `batches`, each kind once (`kind/1`), most recent first: failed spans by their name, and
  warnings and errors that were logged (`span: false`), whichever server logged them. A span another server ran is
  that one's. Each is `from` the client that asked, or where none did, the server or the edge it happened on. A call refused for what it was given or who asked is the caller's mistake, not an error. `shown` says
  whether whose a call was may be shown.
  """
  def errors(batches, shown) do
    all = Enum.flat_map(batches, & &1["spans"])
    # A client's request that a server answered with an error is that server's span, said to be from the client.
    clients = for %{"kind" => "client"} = span <- all, into: %{}, do: {span["traceId"], from(span)}
    failed_traces = for span <- all, span["kind"] == "server", failed?(span) or refused?(span), into: MapSet.new(), do: span["traceId"]

    spans =
      for span <- all,
          failed?(span),
          span["attributes"]["one.forwarded"] != true,
          span["kind"] != "client" or not MapSet.member?(failed_traces, span["traceId"]) do
        %{
          where: span["name"],
          message: span["status"]["message"],
          at: ms(span["end"]),
          took: took(span),
          trace: span["traceId"],
          whose: whose(span, shown),
          server: span["server"],
          from: from(span) || clients[span["traceId"]] || span["server"],
          span: true
        }
      end

    # What was logged during a call is that call's, read in its trace: what failed of it is its span.
    logs =
      for log <- Enum.flat_map(batches, & &1["logs"]), log["severity"] in ["WARN", "ERROR"], log["spanId"] == nil do
        %{
          where: logged(log),
          message: log["body"],
          at: ms(log["time"]),
          took: nil,
          trace: log["traceId"],
          whose: nil,
          server: log["server"],
          from: from(log) || log["server"],
          span: false
        }
      end

    (spans ++ logs)
    |> Enum.group_by(&{&1.span, &1.where, kind(&1.message)})
    |> Enum.map(fn {{span, where, _kind}, all} ->
      latest = Enum.max_by(all, & &1.at)
      servers = all |> Enum.map(& &1.server) |> Enum.reject(&is_nil/1) |> Enum.uniq() |> Enum.sort()

      %{
        where: if(span or servers == [], do: where, else: "#{where} on #{Enum.join(servers, ", ")}"),
        message: message(latest.message),
        count: length(all),
        at: latest.at,
        took: latest.took,
        trace: latest.trace,
        server: latest.server,
        from: latest.from,
        whose: latest.whose,
        span: latest.span
      }
    end)
    |> Enum.sort_by(& &1.at, :desc)
  end

  @doc """
  How long each tool took, slowest first by its 95th percentile, then how long each app took to open: its calls,
  its failures, its 50th, 95th and 99th percentiles in milliseconds, and its 95th over time, about 24 steps from
  `from` to `to`. `names` is what a person calls each app.
  """
  def slowest(batches, from, to, names) do
    points = Enum.flat_map(batches, & &1["metrics"])
    step = max(div(to - from, 24), 60_000_000_000)

    tools =
      for {{app, tool}, points} <- Enum.group_by(Enum.filter(points, &(&1["name"] == "one.tool.duration")), &tool/1) do
        failed = for p <- points, p["attributes"]["one.outcome"] == "error", reduce: 0, do: (sum -> sum + p["count"])

        quantiles = quantiles(added(points))

        Map.merge(quantiles, %{
          app: app,
          tool: tool,
          called: Map.get(names, app, app),
          failed: failed,
          # A tool that keeps its caller waiting this long should let them carry on meanwhile.
          should_be_task: quantiles.p95 > 10_000 and not Enum.any?(points, &(&1["attributes"]["one.task"] == true)),
          over_time: over_time(points, from, to, step)
        })
      end

    opens =
      for {view, points} <-
            Enum.group_by(
              Enum.filter(points, &(&1["name"] == "one.client.open.duration")),
              & &1["attributes"]["one.view"]
            ) do
        Map.merge(quantiles(added(points)), %{
          view: view,
          called: Map.get(names, view, view),
          failed: 0,
          over_time: over_time(points, from, to, step)
        })
      end

    Enum.sort_by(tools, & &1.p95, :desc) ++ Enum.sort_by(opens, & &1.p95, :desc)
  end

  @minute 60_000_000_000

  defp tool(point), do: {point["attributes"]["one.app"], point["attributes"]["one.tool"]}

  defp over_time(points, from, to, step) do
    for {_at, within} <- steps(points, from, to, step), do: if(within == [], do: nil, else: quantiles(added(within)).p95)
  end

  # Each step's start from `from` to `to`, and the points in it: those after its start, up to its end. Steps start on
  # a whole minute and are whole minutes, as a rollup adds a metric's points up by the minute they end
  # (`Telemetry.Rollup`), so none is split.
  defp steps(points, from, to, step) do
    {from, step} = {from - rem(from, @minute), step + rem(@minute - rem(step, @minute), @minute)}
    within = Enum.group_by(for(p <- points, p["time"] > from, do: p), &div(&1["time"] - from - 1, step))
    for {at, index} <- Enum.with_index(from..to//step), do: {at, Map.get(within, index, [])}
  end

  @doc "Calls and failed calls in about 48 steps from `from` to `to`: each step's start in milliseconds and its counts."
  def calls(batches, from, to) do
    step = max(div(to - from, 48), 60_000_000_000)
    points = for b <- batches, p <- b["metrics"], p["name"] == "one.tool.duration", do: p

    for {at, within} <- steps(points, from, to, step) do
      failed = for p <- within, p["attributes"]["one.outcome"] == "error", reduce: 0, do: (sum -> sum + p["count"])
      %{at: ms(at), calls: Enum.sum_by(within, & &1["count"]), failed: failed}
    end
  end

  @doc """
  One trace's spans, by when each started: its depth under the span it's in, its server, its start since the
  trace's and its duration in milliseconds, its outcome and its events.
  """
  def trace(batches, id, shown) do
    spans = for b <- batches, s <- b["spans"], s["traceId"] == id, do: s
    logs = for b <- batches, l <- b["logs"], l["traceId"] == id, do: l

    case spans do
      [] ->
        []

      spans ->
        began = Enum.min(Enum.map(spans, & &1["start"]))
        parents = Map.new(spans, &{&1["spanId"], &1["parentSpanId"]})
        depth = fn span -> depth(parents, span["parentSpanId"], 0) end

        spans
        |> Enum.sort_by(& &1["start"])
        |> Enum.map(fn span ->
          logged =
            for l <- logs,
                l["spanId"] == span["spanId"],
                do: %{name: l["eventName"] || l["severity"], at: ms(l["time"] - began), body: l["body"]}

          %{
            span: span["spanId"],
            parent: span["parentSpanId"],
            name: span["name"],
            depth: depth.(span),
            server: from(span) || span["server"],
            client: from(span) != nil,
            start: ms(span["start"] - began),
            took: took(span),
            outcome: if(failed?(span), do: "error", else: "ok"),
            message: span["status"]["message"],
            attributes:
              Map.delete(span["attributes"], if(shown.(span["attributes"]["one.for"]), do: nil, else: "one.for")),
            events:
              Enum.map(span["events"], &%{name: &1["name"], at: ms(&1["time"] - began), attributes: &1["attributes"]}) ++
                logged
          }
        end)
    end
  end

  defp depth(parents, parent, depth) when is_map_key(parents, parent), do: depth(parents, parents[parent], depth + 1)
  defp depth(_parents, _parent, depth), do: depth

  @doc "Every metric there is, with the names of its attributes and how many it counted."
  def metrics(batches) do
    for {name, points} <- Enum.group_by(Enum.flat_map(batches, & &1["metrics"]), & &1["name"]) do
      %{
        name: name,
        attributes: points |> Enum.flat_map(&Map.keys(&1["attributes"])) |> Enum.uniq() |> Enum.sort(),
        count: Enum.sum_by(points, & &1["count"])
      }
    end
    |> Enum.sort_by(& &1.name)
  end

  @doc """
  The metric `name`, of the data with all of `attributes`, in steps of `step` minutes (about 48 steps if `nil`) from
  `from` to `to`: each step's start in milliseconds, its count, its sum and its quantiles.
  """
  def series(batches, name, attributes, from, to, step) do
    step = if step, do: step * 60_000_000_000, else: max(div(to - from, 48), 60_000_000_000)

    points =
      for b <- batches,
          p <- b["metrics"],
          p["name"] == name,
          Enum.all?(attributes, fn {k, v} -> p["attributes"][k] == v end),
          do: p

    if points == [],
      do: raise("Nothing's recorded of #{name} with those attributes then. `metric` with no name lists what is.")

    %{
      name: name,
      steps:
        for {at, within} <- steps(points, from, to, step) do
          Map.put(if(within == [], do: %{count: 0}, else: quantiles(added(within))), :at, ms(at))
        end
    }
  end

  @doc """
  What AI used, per model per day, newest first: tokens in and out, and what it cost where the endpoint said, as apps
  record them (OpenTelemetry's `gen_ai.client.token.usage`, and `gen_ai.client.cost` in US dollars).
  """
  def spending(batches) do
    points =
      for b <- batches, p <- b["metrics"], p["name"] in ["gen_ai.client.token.usage", "gen_ai.client.cost"], do: p

    points
    |> Enum.group_by(&{day(&1["time"]), &1["attributes"]["gen_ai.request.model"]})
    |> Enum.map(fn {{day, model}, points} ->
      sum = fn name, type ->
        for p <- points,
            p["name"] == name,
            type == nil or p["attributes"]["gen_ai.token.type"] == type,
            reduce: 0,
            do: (s -> s + p["sum"])
      end

      cost =
        if Enum.any?(points, &(&1["name"] == "gen_ai.client.cost")),
          do: Float.round(sum.("gen_ai.client.cost", nil) / 1, 6)

      %{
        day: day,
        model: model,
        input: round(sum.("gen_ai.client.token.usage", "input")),
        output: round(sum.("gen_ai.client.token.usage", "output")),
        cost: cost
      }
    end)
    |> Enum.sort_by(&{&1.day, &1.model}, :desc)
  end

  # Where a log line is from, as a person reads it: an app's page or a client that failed, or the server that logged
  # it.
  defp logged(%{"eventName" => "exception", "attributes" => %{"one.app" => app}}), do: "#{app} page"
  defp logged(%{"eventName" => name}) when is_binary(name) and name != "", do: name
  defp logged(%{"severity" => "WARN"}), do: "Warning"
  # One kind of log line is the same on every server: which servers it was on is said once (`errors/2`).
  defp logged(_log), do: "Error"

  @doc """
  A kind of error, by its message: its first line, as Elixir's banner says it whether or not it starts `** `, up to
  where its stack starts, an S3 error's XML as its `Code`, and what changes from one to the next, numbers, ids and
  processes, taken out, as `Telemetry.Alerts` takes them out. So one cause is one kind however many servers, builds and requests it was on.
  """
  def kind(message) do
    message
    |> message()
    |> String.replace(~r/, (\([\w]+ [^)]*\) )?[^,]*\.exs?:\d+:.*$/, "")
    |> String.replace(~r/#PID<[^>]*>/, "#PID<…>")
    |> String.replace(~r/[0-9a-f]{8,}/i, "…")
    |> String.replace(~r/\d+/, "#")
    |> String.slice(0, 300)
  end

  # A message as it's shown: its first line, without the `** ` Elixir puts before a banner, and an S3 error's XML as
  # its `Code`.
  defp message(text) do
    text
    |> first_line()
    |> String.replace_prefix("** ", "")
    |> String.replace(~r/\\?"?<\?xml.*?<Code>([^<]+)<\/Code>.*?<\/Error>\\?"?/, "\\1")
  end

  @doc """
  Which client a record is from, as a person reads it, from what the client said of itself: "App on Andre's
  iPhone", "claude-code (MCP) on linux". `nil` for a server's own.
  """
  def from(%{"attributes" => %{"service.name" => service} = attributes}) when service not in ["one-server", "one-edge"] do
    on = if attributes["host.name"], do: " on #{attributes["host.name"]}", else: ""

    case service do
      "app" -> "App#{on}"
      "mcp" -> "#{attributes["mcp.client.name"] || "An assistant"} (MCP)#{on}"
      "cli" -> "one#{on}"
      "web" -> "Web, #{attributes["user_agent.name"] || "a browser"}"
      other -> "#{other}#{on}"
    end
  end

  def from(_record), do: nil

  ## Histograms

  @doc false
  # Histograms of one metric added up, bucket by bucket: they're counted in the same buckets (`OneCore.Otlp.buckets/0`).
  # A sum's points add up as one with no buckets.
  def added([first | _] = points) do
    buckets = points |> Enum.map(&(&1["buckets"] || [])) |> Enum.zip_with(&Enum.sum/1)

    Map.merge(first, %{
      "start" => Enum.min(Enum.map(points, & &1["start"])),
      "time" => Enum.max(Enum.map(points, & &1["time"])),
      "count" => Enum.sum_by(points, & &1["count"]),
      "sum" => Enum.sum_by(points, & &1["sum"]),
      "buckets" => buckets,
      "min" => points |> Enum.map(& &1["min"]) |> Enum.reject(&is_nil/1) |> Enum.min(fn -> nil end),
      "max" => points |> Enum.map(& &1["max"]) |> Enum.reject(&is_nil/1) |> Enum.max(fn -> nil end)
    })
  end

  @doc false
  # A histogram's count, sum, and its 50th, 95th and 99th percentiles, each placed in its bucket as a straight line
  # from the bucket's lower bound to its upper, the lowest and the highest bound being the smallest and largest seen.
  def quantiles(%{"count" => count} = histogram) do
    %{
      count: count,
      sum: Float.round(histogram["sum"] / 1, 3),
      p50: quantile(histogram, 0.5),
      p95: quantile(histogram, 0.95),
      p99: quantile(histogram, 0.99)
    }
  end

  defp quantile(%{"count" => 0}, _q), do: nil
  defp quantile(%{"buckets" => []} = histogram, _q), do: Float.round(histogram["sum"] / histogram["count"], 3)

  defp quantile(%{"count" => count, "buckets" => buckets, "bounds" => bounds} = histogram, q) do
    low = histogram["min"] || 0
    high = histogram["max"] || List.last(bounds) || 0
    edges = Enum.zip([low | bounds], bounds ++ [high])
    rank = q * count

    Enum.zip(buckets, edges)
    |> Enum.reduce_while(0, fn {n, {from, to}}, below ->
      if n > 0 and below + n >= rank do
        {from, to} = {max(from, low), min(to, high)}
        {:halt, {:at, from + (to - from) * (rank - below) / n}}
      else
        {:cont, below + n}
      end
    end)
    |> case do
      {:at, at} -> Float.round(at / 1, 3)
      _ -> Float.round(high / 1, 3)
    end
  end

  ## Spans

  defp failed?(span), do: span["status"]["code"] == "error"
  defp refused?(span), do: span["attributes"]["one.outcome"] == "refused"
  defp took(span), do: Float.round((span["end"] - span["start"]) / 1_000_000, 3)
  defp whose(span, shown), do: if(shown.(span["attributes"]["one.for"]), do: span["attributes"]["one.for"])
  defp first_line(nil), do: ""
  defp first_line(text), do: text |> String.split("\n", parts: 2) |> hd()
  defp ms(ns), do: div(ns, 1_000_000)
  defp day(ns), do: ns |> div(1_000_000_000) |> DateTime.from_unix!() |> DateTime.to_date() |> Date.to_iso8601()
end
