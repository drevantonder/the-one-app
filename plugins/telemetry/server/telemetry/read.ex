defmodule Telemetry.Read do
  @moduledoc """
  What's kept, read: errors by kind, tools by how long they took, a trace whole, a metric over time and what AI
  cost. Numbers are OpenTelemetry's exponential histograms, which add up by their buckets and give quantiles to
  within a few percent; times out of here are milliseconds.
  """

  # The most spans an hour put together keeps: every trace that failed, then the slowest.
  @most_spans 20_000
  @most_logs 5_000

  @doc "Batches as one: their spans, their logs, and each metric's histograms added up by its name and attributes."
  def merge(batches) do
    spans = Enum.flat_map(batches, & &1["spans"])

    %{
      "from" => Enum.min(Enum.map(batches, & &1["from"])),
      "to" => Enum.max(Enum.map(batches, & &1["to"])),
      "spans" => fewest(spans),
      "logs" => batches |> Enum.flat_map(& &1["logs"]) |> Enum.take(-@most_logs),
      "metrics" =>
        batches
        |> Enum.flat_map(& &1["metrics"])
        |> Enum.group_by(&{&1["name"], &1["attributes"]})
        |> Enum.map(fn {_key, points} -> added(points) end)
    }
  end

  # As many whole traces as fit: those that failed first, then the slowest.
  defp fewest(spans) when length(spans) <= @most_spans, do: spans

  defp fewest(spans) do
    spans
    |> Enum.group_by(& &1["traceId"])
    |> Enum.sort_by(fn {_id, spans} -> {not Enum.any?(spans, &failed?/1), -Enum.sum_by(spans, &took/1)} end)
    |> Enum.reduce_while([], fn {_id, trace}, acc ->
      if length(acc) + length(trace) > @most_spans, do: {:halt, acc}, else: {:cont, trace ++ acc}
    end)
  end

  @doc """
  The errors among `batches`, each kind once, most recent first: failed spans by their name and message, and
  warnings and errors that were logged (`span: false`). A span another server ran is that one's. `shown` says whether whose a
  call was may be shown.
  """
  def errors(batches, shown) do
    all = Enum.flat_map(batches, & &1["spans"])
    # A client's request that a server answered with an error is that server's span, said to be from the client.
    clients = for %{"kind" => "client"} = span <- all, into: %{}, do: {span["traceId"], from(span)}
    failed_traces = for span <- all, span["kind"] == "server", failed?(span), into: MapSet.new(), do: span["traceId"]

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
          from: from(span) || clients[span["traceId"]],
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
          from: from(log),
          span: false
        }
      end

    (spans ++ logs)
    |> Enum.group_by(&{&1.where, first_line(&1.message)})
    |> Enum.map(fn {{where, message}, all} ->
      latest = Enum.max_by(all, & &1.at)

      %{
        where: where,
        message: message,
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
  `from` to `to`. `names` is what a person calls each plugin.
  """
  def slowest(batches, from, to, names) do
    points = Enum.flat_map(batches, & &1["metrics"])
    step = max(div(to - from, 24), 60_000_000_000)

    tools =
      for {{plugin, tool}, points} <- Enum.group_by(Enum.filter(points, &(&1["name"] == "one.tool.duration")), &tool/1) do
        failed = for p <- points, p["attributes"]["one.outcome"] == "error", reduce: 0, do: (sum -> sum + p["count"])

        Map.merge(quantiles(added(points)), %{
          plugin: plugin,
          tool: tool,
          called: Map.get(names, plugin, plugin),
          failed: failed,
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
          called: Map.get(names, view, if(view == "home-screen", do: "Home Screen", else: view)),
          failed: 0,
          over_time: over_time(points, from, to, step)
        })
      end

    Enum.sort_by(tools, & &1.p95, :desc) ++ Enum.sort_by(opens, & &1.p95, :desc)
  end

  defp tool(point), do: {point["attributes"]["one.plugin"], point["attributes"]["one.tool"]}

  defp over_time(points, from, to, step) do
    for at <- from..to//step do
      within = Enum.filter(points, &(&1["time"] > at and &1["time"] <= at + step))
      if within == [], do: nil, else: quantiles(added(within)).p95
    end
  end

  @doc "Calls and failed calls in about 48 steps from `from` to `to`: each step's start in milliseconds and its counts."
  def calls(batches, from, to) do
    step = max(div(to - from, 48), 60_000_000_000)
    points = for b <- batches, p <- b["metrics"], p["name"] == "one.tool.duration", do: p

    for at <- from..to//step do
      within = Enum.filter(points, &(&1["time"] > at and &1["time"] <= at + step))
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
        for at <- from..to//step do
          within = Enum.filter(points, &(&1["time"] > at and &1["time"] <= at + step))
          Map.put(if(within == [], do: %{count: 0}, else: quantiles(added(within))), :at, ms(at))
        end
    }
  end

  @doc """
  What AI used, per model per day, newest first: tokens in and out, and what it cost where the endpoint said, as plugins
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
  defp logged(%{"eventName" => "exception", "attributes" => %{"one.plugin" => plugin}}), do: "#{plugin} page"
  defp logged(%{"eventName" => name, "attributes" => %{"service.name" => _}}), do: name
  defp logged(%{"severity" => "WARN", "server" => server}), do: "Warning on #{server}"
  defp logged(%{"server" => server}), do: "Error on #{server}"

  @doc """
  Which client a record is from, as a person reads it, from what the client said of itself: "App on Andre's
  iPhone", "claude-code (MCP) on linux". `nil` for a server's own.
  """
  def from(%{"attributes" => %{"service.name" => service} = attributes}) do
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
  # Histograms of one metric added up: OpenTelemetry's exponential histograms of one scale add by their buckets.
  def added([first | _] = points) do
    buckets =
      points
      |> Enum.flat_map(& &1["buckets"])
      |> Enum.reduce(%{}, fn [bucket, count], all -> Map.update(all, bucket, count, &(&1 + count)) end)

    %{
      first
      | "start" => Enum.min(Enum.map(points, & &1["start"])),
        "time" => Enum.max(Enum.map(points, & &1["time"])),
        "count" => Enum.sum_by(points, & &1["count"]),
        "sum" => Enum.sum_by(points, & &1["sum"]),
        "zeroCount" => Enum.sum_by(points, & &1["zeroCount"]),
        "buckets" => Enum.sort(for {bucket, count} <- buckets, do: [bucket, count])
    }
  end

  @doc false
  # A histogram's count, sum, and its 50th, 95th and 99th percentiles, each the middle of the bucket it's in.
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

  defp quantile(%{"count" => count, "zeroCount" => zero, "buckets" => buckets, "scale" => scale}, q) do
    rank = q * count
    base = :math.pow(2, :math.pow(2, -scale))

    if rank <= zero do
      0.0
    else
      Enum.reduce_while(buckets, zero, fn [bucket, n], below ->
        if below + n >= rank, do: {:halt, {:at, bucket}}, else: {:cont, below + n}
      end)
      |> case do
        {:at, bucket} -> Float.round(:math.pow(base, bucket + 0.5), 3)
        _ -> Float.round(:math.pow(base, hd(List.last(buckets)) + 0.5), 3)
      end
    end
  end

  ## Spans

  defp failed?(span), do: span["status"]["code"] == "error"
  defp took(span), do: Float.round((span["end"] - span["start"]) / 1_000_000, 3)
  defp whose(span, shown), do: if(shown.(span["attributes"]["one.for"]), do: span["attributes"]["one.for"])
  defp first_line(nil), do: ""
  defp first_line(text), do: text |> String.split("\n", parts: 2) |> hd()
  defp ms(ns), do: div(ns, 1_000_000)
  defp day(ns), do: ns |> div(1_000_000_000) |> DateTime.from_unix!() |> DateTime.to_date() |> Date.to_iso8601()
end
