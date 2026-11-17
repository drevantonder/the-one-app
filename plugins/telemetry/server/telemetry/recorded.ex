defmodule Telemetry.Recorded do
  @moduledoc """
  What this server heard for Telemetry, in OpenTelemetry's shapes, held here and handed to a server acting for
  whoever Telemetry was installed for as one batch a minute, or as often as `OTEL_BSP_SCHEDULE_DELAY` says in
  milliseconds, which keeps it (`Telemetry.keep/2`): so a call costs a few microseconds, writes nothing and asks no
  other server, and an idle server sends nothing at all. There's one of these on each server where something
  Telemetry observes happened, for whoever Telemetry was installed for, and its batches carry that server's name.

  - A span for each tool call, from the hook `called`: its plugin, its tool, whose it was as an owner's prefix
    (`users/alice`), how long it took and how it ended. A call passed on is a span on both servers, the one that
    ran it under the one that passed it on.
  - Numbers, summed and spread as OpenTelemetry's exponential histograms: how long each tool took, and what a
    plugin records (`record/2`) or a client reports.
  - Log lines: what a plugin records, and every warning and error said to `Logger` on the server, which Telemetry
    installed for the network hears. A crash is kept as its exception's banner and never as the state or message it
    crashed on.
  - What a client reported of itself (the hook `measured`), as records: its requests that failed or were slow, each
    a span its server's span of it is under, how long no server answered it, how long its views took to open, and
    its pages' errors and its own crashes.

  None of it is inputs, answers, secrets or anything a person wrote: names, sizes, durations and outcomes only.
  """
  use GenServer
  alias OneCore.{Call, Servers, Service, Trace, User}

  # How much of each a batch holds at most. Past that, only spans that failed are kept, and the rest are counted:
  # the numbers still count every call.
  @most_spans 2000
  @most_logs 500
  # A histogram's scale: each bucket is 2^(1/8) wide, so a quantile read from them is within about 4%.
  @scale 3
  # How long a text that's recorded may be, in characters: an error's message, an attribute.
  @longest 1000

  @doc "A tool call, as the hook `called` told it."
  def called(call, %{"trace" => trace, "took" => took} = about) do
    plugin = about["plugin"] || "one"
    attributes = %{"one.plugin" => plugin, "one.tool" => about["tool"]}
    # Whether its plugin makes it an MCP task, which a slow tool that isn't should be (`Read.slowest/4`).
    attributes = if about["task"] == true, do: Map.put(attributes, "one.task", true), else: attributes
    now = System.os_time(:nanosecond)
    error = about["error"]
    forwarded = about["forwarded"] == true

    span =
      Map.merge(trace, %{
        "name" => "#{plugin} #{about["tool"]}",
        "kind" => "server",
        "start" => now - took * 1000,
        "end" => now,
        "attributes" =>
          Map.merge(attributes, %{"one.for" => User.owner(call.subject) || "link"})
          |> Map.merge(if(forwarded, do: %{"one.forwarded" => true}, else: %{})),
        "status" => %{"code" => if(error, do: "error", else: "ok"), "message" => short(error || "")},
        "events" => []
      })

    # A call passed on is measured where it ran.
    numbers =
      if forwarded,
        do: [],
        else: [
          {"one.tool.duration", Map.put(attributes, "one.outcome", if(error, do: "error", else: "ok")), took / 1000}
        ]

    received(call, spans: [span], numbers: numbers)
  end

  @doc """
  What a client reported of itself, as the hook `measured` told it: records as `record` takes them, each with the
  `time` the client made it, which may be well before it could tell a server. Each is the device's and its
  person's, as the client's request proved, whatever it says of itself.
  """
  def measured(call, %{"records" => records, "device" => device, "who" => who}) do
    proved = %{"one.device" => device, "one.for" => User.owner(who || "") || "link"}

    records =
      for %{"name" => name} = record <- records, is_binary(name), is_map(record["attributes"] || %{}) do
        attributes = Map.merge(record["attributes"] || %{}, proved)
        time = if is_integer(record["time"]), do: record["time"] * 1_000_000, else: System.os_time(:nanosecond)
        %{record | "attributes" => attributes} |> Map.put("time", time)
      end

    received(call, records(records, %{}))
  end

  @doc """
  What a plugin's tool call recorded, by Telemetry' tool `record`: each a number (`value`), a span that ended now
  (`took`, in milliseconds, with an `error` if it failed), or else a log line, all under the call that recorded it.
  """
  def record(call, records) do
    # The span running here is the call to `record`: what it records is of the call that made it.
    trace = Trace.current() || %{}
    now = System.os_time(:nanosecond)

    received(
      call,
      records(Enum.map(records, &Map.put(&1, "time", now)), %{
        "traceId" => trace["traceId"],
        "spanId" => trace["parentSpanId"]
      })
    )
  end

  # Records as spans, numbers and logs, under the span `under`, or each in the trace and span it says it is, as a
  # client's are.
  defp records(records, under) do
    {spans, numbers, logs} =
      Enum.reduce(records, {[], [], []}, fn record, {spans, numbers, logs} ->
        attributes = attributes(record["attributes"] || %{})
        now = record["time"]
        trace = record["traceId"] || under["traceId"]

        case record do
          %{"value" => value} when is_number(value) ->
            {spans, [{record["name"], attributes, value} | numbers], logs}

          %{"took" => took} when is_number(took) ->
            span = %{
              "traceId" => if(is_binary(trace) and trace =~ ~r/^[0-9a-f]{32}$/, do: trace, else: hex(16)),
              "spanId" =>
                if(is_binary(record["spanId"]) and record["spanId"] =~ ~r/^[0-9a-f]{16}$/,
                  do: record["spanId"],
                  else: hex(8)
                ),
              "parentSpanId" => under["spanId"],
              "name" => short(record["name"]),
              "kind" => if(record["kind"] == "client", do: "client", else: "internal"),
              "start" => now - round(took * 1_000_000),
              "end" => now,
              "attributes" => attributes,
              "status" => %{
                "code" => if(record["error"], do: "error", else: "ok"),
                "message" => short(record["error"] || "")
              },
              "events" => []
            }

            {[span | spans], numbers, logs}

          %{"error" => error} when is_binary(error) ->
            {spans, numbers,
             [%{log("ERROR", short(record["name"]), short(error), attributes, under) | "time" => now} | logs]}

          _ ->
            {spans, numbers, [%{log("INFO", short(record["name"]), nil, attributes, under) | "time" => now} | logs]}
        end
      end)

    [spans: spans, numbers: numbers, logs: logs]
  end

  defp received(call, received) do
    process = Call.process(call, __MODULE__, call.installed, {call.installed, call.server})
    GenServer.cast(process, {:received, received})
  end

  defp log(severity, name, body, attributes, trace) do
    %{
      "time" => System.os_time(:nanosecond),
      "severity" => severity,
      "eventName" => name,
      "body" => body,
      "attributes" => attributes,
      "traceId" => trace && trace["traceId"],
      "spanId" => trace && trace["spanId"]
    }
  end

  ## Logger

  @doc false
  # A `:logger` handler, run where something logged: it keeps every warning and error. A crash is its exception's
  # banner, never what Elixir said of it, which holds the state and the message it crashed on.
  def log(%{level: level, msg: msg, meta: meta}, %{config: %{to: to}}) do
    if :logger.compare_levels(level, :warning) != :lt do
      body =
        case meta do
          %{crash_reason: {reason, stacktrace}} when is_list(stacktrace) -> crashed(:error, reason, stacktrace)
          %{crash_reason: reason} -> crashed(:exit, reason, [])
          _ -> message(msg)
        end

      severity = if level == :warning, do: "WARN", else: "ERROR"
      if body, do: GenServer.cast(to, {:received, logs: [log(severity, nil, short(body), %{}, Trace.current())]})
    end
  end

  # A crash as its exception says it: "RuntimeError: Bomb went off".
  defp crashed(kind, reason, stacktrace),
    do: String.replace_prefix(Exception.format_banner(kind, reason, stacktrace), "** ", "")

  defp message({:string, text}), do: IO.chardata_to_string(text)
  defp message({format, arguments}) when is_list(arguments), do: IO.chardata_to_string(:io_lib.format(format, arguments))
  # A report is a term, which may hold anything: only a crash's reason is kept of one, above.
  defp message({:report, _report}), do: nil

  # A server's own warnings and errors are the network's, so the Telemetry installed for the network hears them. The
  # handler is this module as it is now, so it's added again each time the plugin moves to newer code.
  defp listen(%{installed: "network"}) do
    :logger.remove_handler(__MODULE__)
    :ok = :logger.add_handler(__MODULE__, __MODULE__, %{level: :warning, config: %{to: self()}})
  end

  defp listen(_state), do: :ok

  ## The batch

  @impl true
  def init({installed, server}) do
    every = every()
    Process.send_after(self(), :flush, every)
    state = %{installed: installed, server: server, every: every, from: System.os_time(:nanosecond)}
    listen(state)
    {:ok, Map.merge(state, empty())}
  end

  @impl true
  def code_change(_vsn, state, _extra) do
    listen(state)
    {:ok, state}
  end

  @impl true
  def terminate(_why, %{installed: "network"}), do: :logger.remove_handler(__MODULE__)
  def terminate(_why, _state), do: :ok

  @impl true
  def handle_cast({:received, received}, state) do
    state = Enum.reduce(Keyword.get(received, :spans, []), state, &span/2)
    state = Enum.reduce(Keyword.get(received, :numbers, []), state, &number/2)
    logs = Enum.take(Keyword.get(received, :logs, []), @most_logs - state.log_count)
    {:noreply, %{state | logs: logs ++ state.logs, log_count: state.log_count + length(logs)}}
  end

  @impl true
  def handle_info(:flush, state) do
    to = System.os_time(:nanosecond)
    batch = %{"spans" => state.spans, "logs" => state.logs, "metrics" => metrics(state.numbers, state.from, to)}

    if Enum.any?(Map.values(batch), &(&1 != [])) do
      machine = Servers.name(state.server) || String.slice(state.server, 0, 8)

      batch = Map.merge(batch, %{"server" => state.server, "machine" => machine, "from" => state.from, "to" => to})
      Service.plugin(self(), state.installed, :keep, [batch])
    end

    Process.send_after(self(), :flush, state.every)
    {:noreply, Map.merge(%{state | from: to}, empty())}
  end

  defp empty, do: %{spans: [], span_count: 0, logs: [], log_count: 0, numbers: %{}}

  defp span(span, state) do
    if state.span_count < @most_spans or span["status"]["code"] == "error",
      do: %{state | spans: [span | state.spans], span_count: state.span_count + 1},
      else: number({"one.spans.dropped", %{}, 1}, state)
  end

  # A number is counted in its bucket of an exponential histogram, by its name and attributes.
  defp number({name, attributes, value}, state) do
    bucket = if value > 0, do: ceil(:math.log2(value) * 2 ** @scale) - 1, else: :zero

    numbers =
      Map.update(state.numbers, {name, attributes}, {value, %{bucket => 1}}, fn {sum, counts} ->
        {sum + value, Map.update(counts, bucket, 1, &(&1 + 1))}
      end)

    %{state | numbers: numbers}
  end

  # How often what's recorded is kept, in milliseconds: OpenTelemetry's own setting for a batch of spans, or a
  # minute, so a busy server writes a blob a minute and an idle one none.
  defp every do
    case Integer.parse(System.get_env("OTEL_BSP_SCHEDULE_DELAY", "")) do
      {every, ""} when every > 0 -> every
      _ -> 60_000
    end
  end

  # Each number's histogram over the batch, as OpenTelemetry's exponential histogram's data point.
  defp metrics(numbers, from, to) do
    for {{name, attributes}, {sum, counts}} <- numbers do
      {zero, buckets} = Map.pop(counts, :zero, 0)

      %{
        "name" => name,
        "attributes" => attributes,
        "start" => from,
        "time" => to,
        "count" => zero + Enum.sum(Map.values(buckets)),
        "sum" => sum,
        "scale" => @scale,
        "zeroCount" => zero,
        "buckets" => Enum.sort(for {bucket, count} <- buckets, do: [bucket, count])
      }
    end
  end

  # Attributes as JSON holds them, each text cut short.
  defp attributes(attributes), do: Map.new(attributes, fn {name, value} -> {to_string(name), value(value)} end)

  defp value(value) when is_binary(value), do: short(value)
  defp value(value) when is_number(value) or is_boolean(value), do: value
  defp value(value) when is_list(value), do: Enum.map(value, &value/1)
  defp value(value), do: short(inspect(value))

  defp short(text, longest \\ @longest), do: String.slice(to_string(text), 0, longest)

  defp hex(bytes), do: Base.encode16(:crypto.strong_rand_bytes(bytes), case: :lower)
end
