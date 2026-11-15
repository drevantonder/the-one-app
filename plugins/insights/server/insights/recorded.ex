defmodule Insights.Recorded do
  @moduledoc """
  What this server heard for Insights, in OpenTelemetry's shapes, held here and kept as one blob a minute, or as
  often as `OTEL_BSP_SCHEDULE_DELAY` says in milliseconds: so a call costs a few microseconds and writes nothing,
  and an idle server writes nothing at all. There's one of these on each server, for whoever Insights was added for.

  - A span for each tool call, from the hook `called`: its plugin, its tool, whose it was as an owner's prefix
    (`users/alice`), how long it took and how it ended. A call passed on is a span on both servers, the one that
    ran it under the one that passed it on.
  - Numbers, summed and spread as OpenTelemetry's exponential histograms: how long each tool took, how long a
    client took to open a view (the hook `measured`), and what a plugin records (`record/2`).
  - Log lines: what a plugin records, calls a client couldn't get answered, and every warning and error said to
    `Logger` on the server, which Insights added for the network hears. A crash is kept as its exception's banner
    and never as the state or message it crashed on.

  None of it is inputs, answers, secrets or anything a person wrote: names, sizes, durations and outcomes only.
  """
  use GenServer
  alias OneCore.{Call, Servers, Trace, User}

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
    now = System.os_time(:nanosecond)
    error = about["error"]
    passed_on = about["passed_on"] == true

    span =
      Map.merge(trace, %{
        "name" => "#{plugin} #{about["tool"]}",
        "kind" => "server",
        "start" => now - took * 1000,
        "end" => now,
        "attributes" =>
          Map.merge(attributes, %{"one.for" => User.owner(call.for) || "link"})
          |> Map.merge(if(passed_on, do: %{"one.passed_on" => true}, else: %{})),
        "status" => %{"code" => if(error, do: "error", else: "ok"), "message" => short(error || "")},
        "events" => []
      })

    # A call passed on is measured where it ran.
    numbers =
      if passed_on,
        do: [],
        else: [
          {"one.tool.duration", Map.put(attributes, "one.outcome", if(error, do: "error", else: "ok")), took / 1000}
        ]

    heard(call, spans: [span], numbers: numbers)
  end

  @doc "What a client measured of itself, as the hook `measured` told it."
  def measured(call, said) do
    kind = short(said["client"] || "client", 40)

    numbers =
      for %{"view" => view, "ms" => ms} <- List.wrap(said["opened"]),
          is_binary(view) and is_number(ms),
          do: {"one.client.open.duration", %{"one.view" => short(view, 80), "one.client" => kind}, ms}

    logs =
      for %{"tool" => tool, "why" => why} <- List.wrap(said["failed"]),
          is_binary(tool) and is_binary(why),
          do: log("WARN", "one.client.failed", short(why), %{"one.tool" => short(tool, 80), "one.client" => kind}, nil)

    heard(call, numbers: numbers, logs: logs)
  end

  @doc """
  What a plugin's tool call recorded, by Insights' tool `record`: each a number (`value`), a span that ended now
  (`took`, in milliseconds, with an `error` if it failed), or else a log line, all under the call that recorded it.
  """
  def record(call, records) do
    # The span running here is the call to `record`: what it records is of the call that made it.
    trace = Trace.current() || %{}
    under = %{"traceId" => trace["traceId"], "spanId" => trace["parentSpanId"]}
    now = System.os_time(:nanosecond)

    {spans, numbers, logs} =
      Enum.reduce(records, {[], [], []}, fn record, {spans, numbers, logs} ->
        attributes = attributes(record["attributes"] || %{})

        case record do
          %{"value" => value} when is_number(value) ->
            {spans, [{record["name"], attributes, value} | numbers], logs}

          %{"took" => took} when is_number(took) ->
            span = %{
              "traceId" => under["traceId"] || hex(16),
              "spanId" => hex(8),
              "parentSpanId" => under["spanId"],
              "name" => short(record["name"]),
              "kind" => "internal",
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

          _ ->
            {spans, numbers, [log("INFO", short(record["name"]), nil, attributes, under) | logs]}
        end
      end)

    heard(call, spans: spans, numbers: numbers, logs: logs)
  end

  defp heard(call, heard) do
    process = Call.process(call, __MODULE__, call.added, {call.own, call.added, call.server})
    GenServer.cast(process, {:heard, heard})
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
          _ -> said(msg)
        end

      severity = if level == :warning, do: "WARN", else: "ERROR"
      if body, do: GenServer.cast(to, {:heard, logs: [log(severity, nil, short(body), %{}, Trace.current())]})
    end
  end

  # A crash as its exception says it: "RuntimeError: Bomb went off".
  defp crashed(kind, reason, stacktrace),
    do: String.replace_prefix(Exception.format_banner(kind, reason, stacktrace), "** ", "")

  defp said({:string, text}), do: IO.chardata_to_string(text)
  defp said({format, arguments}) when is_list(arguments), do: IO.chardata_to_string(:io_lib.format(format, arguments))
  # A report is a term, which may hold anything: only a crash's reason is kept of one, above.
  defp said({:report, _report}), do: nil

  # A server's own warnings and errors are the network's, so the Insights added for the network hears them. The
  # handler is this module as it is now, so it's added again each time the plugin moves to newer code.
  defp listen(%{added: "network"}) do
    :logger.remove_handler(__MODULE__)
    :ok = :logger.add_handler(__MODULE__, __MODULE__, %{level: :warning, config: %{to: self()}})
  end

  defp listen(_state), do: :ok

  ## The batch

  @impl true
  def init({own, added, server}) do
    every = every()
    Process.send_after(self(), :flush, every)
    state = %{own: own, added: added, server: server, every: every, from: System.os_time(:nanosecond)}
    listen(state)
    {:ok, Map.merge(state, empty())}
  end

  @impl true
  def code_change(_vsn, state, _extra) do
    listen(state)
    {:ok, state}
  end

  @impl true
  def terminate(_why, %{added: "network"}), do: :logger.remove_handler(__MODULE__)
  def terminate(_why, _state), do: :ok

  @impl true
  def handle_cast({:heard, heard}, state) do
    state = Enum.reduce(Keyword.get(heard, :spans, []), state, &span/2)
    state = Enum.reduce(Keyword.get(heard, :numbers, []), state, &number/2)
    logs = Enum.take(Keyword.get(heard, :logs, []), @most_logs - state.log_count)
    {:noreply, %{state | logs: logs ++ state.logs, log_count: state.log_count + length(logs)}}
  end

  @impl true
  def handle_info(:flush, state) do
    to = System.os_time(:nanosecond)
    batch = %{"spans" => state.spans, "logs" => state.logs, "metrics" => metrics(state.numbers, state.from, to)}

    if Enum.any?(Map.values(batch), &(&1 != [])) do
      machine = Servers.name(state.server) || String.slice(state.server, 0, 8)

      Insights.Store.keep(
        state.own,
        Map.merge(batch, %{"server" => state.server, "machine" => machine, "from" => state.from, "to" => to})
      )
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
