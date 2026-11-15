defmodule OneCore.Telemetry do
  @moduledoc """
  What a server records of what it and its clients do, in OpenTelemetry's shapes, for whatever plugin added for the
  network hears `recorded`: Insights, which The One App ships with, keeps it and shows it.

  - A span for each tool call: its plugin, its tool, whose it was as an owner's prefix (`users/alice`), how long it
    took and how it ended, with the error's message if it raised. A call passed to another server is one trace
    across both: the span's context goes with it (`traceparent`, W3C's), and the other's span is its child.
  - Numbers, summed and spread as OpenTelemetry's exponential histograms each minute: how long each tool took, and
    whatever a plugin measures, such as AI Gateway's tokens.
  - Log lines: what a plugin records, a server starting or being asked to update, and every warning and error said
    to `Logger`. A crash is kept as its exception's banner and never as the state or message it crashed on.
  - What a client says of itself (`client/2`): how long an app took from its tap to its first drawing, and calls
    that failed before a server answered.

  None of it is inputs, answers, secrets or anything a person wrote: names, sizes, durations and outcomes only.
  Spans are made with OpenTelemetry's own API (`OpenTelemetry.Tracer`), which this is the span processor of, and
  the rest with `record/3`. What's recorded is held here and handed on once a minute, or as often as
  `OTEL_BSP_SCHEDULE_DELAY` says in milliseconds, as one batch: so a call costs a few microseconds, and nothing is
  written to the bucket for it. With no plugin hearing `recorded`, nothing is recorded at all.
  """
  use GenServer
  require OpenTelemetry.Tracer, as: Tracer
  require Record
  alias OneCore.{Builds, Hooks, Server, Servers}

  @behaviour :otel_span_processor

  Record.defrecordp(:span, Record.extract(:span, from_lib: "opentelemetry/include/otel_span.hrl"))
  Record.defrecordp(:event, Record.extract(:event, from_lib: "opentelemetry/include/otel_span.hrl"))
  Record.defrecordp(:status, Record.extract(:status, from_lib: "opentelemetry_api/include/opentelemetry.hrl"))

  @spans __MODULE__.Spans
  @logs __MODULE__.Logs
  @numbers __MODULE__.Numbers

  # How much of each a batch holds at most. Past that, only spans that failed are kept, and the rest are counted:
  # the numbers still count every call.
  @most_spans 2000
  @most_logs 500
  # A histogram's scale: each bucket is 2^(1/8) wide, so a quantile read from them is within about 4%.
  @scale 3
  # How long a text that's recorded may be, in characters: an error's message, an attribute.
  @longest 1000
  @billion 1_000_000_000

  @doc """
  Records, for Insights, as whatever's running: a log line, a number, or a span around some work.

      OneCore.Telemetry.record("photos.resized", %{"size" => "large"})          # a log line
      OneCore.Telemetry.record("photos.bytes", %{"size" => "large"}, 52_134)    # a number
      OneCore.Telemetry.record("resize", %{"size" => "large"}, fn -> ... end)   # a span: gives what fun gives

  `name` says what it is, in OpenTelemetry's dotted lowercase. `attributes` are names, sizes and outcomes: never
  what someone wrote or was given, and never a secret. A log line inside a tool call is an event of its span. A
  number is summed and spread over each minute, by its name and attributes, and read back as quantiles.
  """
  def record(name, attributes \\ %{}) do
    if on?() do
      if Tracer.current_span_ctx() |> OpenTelemetry.Span.is_recording(),
        do: Tracer.add_event(name, attributes(attributes)),
        else: log("INFO", name, nil, attributes(attributes))
    end

    :ok
  end

  def record(name, attributes, fun) when is_function(fun, 0) do
    if on?() do
      Tracer.with_span name, %{attributes: attributes(attributes)} do
        fun.()
      end
    else
      fun.()
    end
  end

  def record(name, attributes, value) when is_number(value) do
    if on?(), do: number(name, attributes(attributes), value)
    :ok
  end

  @doc """
  Runs `fun`, a tool call of `plugin`'s (`nil` for one of One Core's own) for `who`, in a span of its own, and
  measures how long it took. `fun` gives what `OneCore.Plugins.Calls.call/5` does, which says how it ended.
  """
  def tool(plugin, tool, who, fun) do
    if on?() do
      attributes = %{
        "one.plugin" => plugin || "one",
        "one.tool" => tool,
        "one.for" => OneCore.User.owner(who) || "link"
      }

      began = System.monotonic_time()

      Tracer.with_span "#{plugin || "one"} #{tool}", %{attributes: attributes, kind: :server} do
        try do
          ended = fun.()
          outcome(ended)
          measured(attributes, began, ended)
          ended
        catch
          kind, reason ->
            Tracer.set_status(:error, short(Exception.format_banner(kind, reason, __STACKTRACE__)))
            measured(attributes, began, :raised)
            :erlang.raise(kind, reason, __STACKTRACE__)
        end
      end
    else
      fun.()
    end
  end

  defp outcome({:ok, _answer}), do: Tracer.set_status(:ok, "")
  defp outcome({:error, why}), do: Tracer.set_status(:error, short(why))
  defp outcome({:passed_on, %{"isError" => true, "content" => [%{"text" => why} | _]}}), do: passed(short(why))
  defp outcome({:passed_on, _result}), do: passed(nil)
  defp outcome(:passed_back), do: Tracer.set_attribute("one.passed_back", true)
  defp outcome(_other), do: :ok

  # Another server ran it, and recorded it as a span of its own under this one.
  defp passed(why) do
    Tracer.set_attribute("one.passed_on", true)
    if why, do: Tracer.set_status(:error, why), else: Tracer.set_status(:ok, "")
  end

  # How long a call that ran here took. One passed on is measured where it ran.
  defp measured(_attributes, _began, {:passed_on, _result}), do: :ok
  defp measured(_attributes, _began, :passed_back), do: :ok

  defp measured(attributes, began, ended) do
    took = System.convert_time_unit(System.monotonic_time() - began, :native, :microsecond) / 1000
    outcome = if match?({:ok, _answer}, ended), do: "ok", else: "error"
    number("one.tool.duration", attributes |> Map.delete("one.for") |> Map.put("one.outcome", outcome), took)
  end

  @doc """
  What a client said of itself (`notifications/one/recorded`): `opened`, each view it opened and in how many
  milliseconds from its tap to its first drawing, and `failed`, each call that failed before a server answered it,
  by its tool. `device` is the client's key, of which only the start is kept.
  """
  def client(said, device) do
    if on?() do
      kind = short(said["client"] || "client", 40)

      for %{"view" => view, "ms" => ms} <- List.wrap(said["opened"]),
          is_binary(view) and is_number(ms),
          do: number("one.client.open.duration", %{"one.view" => short(view, 80), "one.client" => kind}, ms)

      for %{"tool" => tool, "why" => why} <- List.wrap(said["failed"]), is_binary(tool) and is_binary(why) do
        attributes = %{
          "one.tool" => short(tool, 80),
          "one.client" => kind,
          "one.device" => String.slice(device || "", 0, 8)
        }

        log("WARN", "one.client.failed", short(why), attributes)
      end
    end

    :ok
  end

  @doc "The W3C `traceparent` of the span running here, for a request to another server, or `nil` with none."
  def traceparent do
    case :otel_propagator_text_map.inject([]) do
      [] -> nil
      headers -> List.keyfind(headers, "traceparent", 0, {nil, nil}) |> elem(1)
    end
  end

  @doc "Carries on the trace that `traceparent`, from another server, is in: what runs here next is its child."
  def follow(nil), do: :ok

  def follow(traceparent) when is_binary(traceparent),
    do: :otel_propagator_text_map.extract([{"traceparent", traceparent}])

  def follow(_), do: :ok

  # Whether anything keeps what's recorded: a plugin added for the network hears it. Asked of what's kept of
  # who hooks into what, so it costs a lookup.
  defp on? do
    case :ets.whereis(Hooks) do
      :undefined -> false
      _ -> Hooks.hooked?("recorded", "network")
    end
  end

  ## The span processor

  @impl :otel_span_processor
  def on_start(_ctx, span, _config), do: span

  @impl :otel_span_processor
  def on_end(span() = ended, _config) do
    failed = match?(status(code: :error), span(ended, :status))

    cond do
      :ets.whereis(@spans) == :undefined -> :dropped
      not on?() -> :dropped
      :ets.info(@spans, :size) < @most_spans or failed -> :ets.insert(@spans, {unique(), ended})
      true -> dropped()
    end
  end

  def on_end(_span, _config), do: {:error, :invalid_span}

  defp dropped do
    number("one.spans.dropped", %{}, 1)
    :dropped
  end

  @impl :otel_span_processor
  def force_flush(_config) do
    send(__MODULE__, :flush)
    :ok
  end

  ## Logger

  @doc false
  # A `:logger` handler: it keeps every warning and error. A crash is its exception's banner, never what Elixir said
  # of it, which holds the state and the message it crashed on.
  def log(%{level: level, msg: msg, meta: meta}, _config) do
    if :logger.compare_levels(level, :warning) != :lt and on?() do
      severity = if level == :warning, do: "WARN", else: "ERROR"

      body =
        case meta do
          %{crash_reason: {reason, stacktrace}} when is_list(stacktrace) -> crashed(:error, reason, stacktrace)
          %{crash_reason: reason} -> crashed(:exit, reason, [])
          _ -> said(msg)
        end

      if body do
        if Tracer.current_span_ctx() |> OpenTelemetry.Span.is_recording(),
          do: Tracer.add_event("log", %{"severity" => severity, "body" => short(body)}),
          else: log(severity, nil, short(body), %{})
      end
    end
  end

  # A crash as its exception says it: "RuntimeError: Bomb went off".
  defp crashed(kind, reason, stacktrace),
    do: String.replace_prefix(Exception.format_banner(kind, reason, stacktrace), "** ", "")

  defp said({:string, text}), do: IO.chardata_to_string(text)
  defp said({format, arguments}) when is_list(arguments), do: IO.chardata_to_string(:io_lib.format(format, arguments))
  # A report is a term, which may hold anything: only a crash's reason is kept of one, above.
  defp said({:report, _report}), do: nil

  defp log(severity, name, body, attributes) do
    if :ets.whereis(@logs) != :undefined and :ets.info(@logs, :size) < @most_logs do
      context = Tracer.current_span_ctx()
      recording = OpenTelemetry.Span.is_recording(context)

      :ets.insert(@logs, {
        unique(),
        %{
          "time" => System.os_time(:nanosecond),
          "severity" => severity,
          "eventName" => name,
          "body" => body,
          "attributes" => attributes,
          "traceId" => if(recording, do: OpenTelemetry.Span.hex_trace_id(context)),
          "spanId" => if(recording, do: OpenTelemetry.Span.hex_span_id(context))
        }
      })
    end
  end

  ## Numbers

  # A number is counted in its bucket of an exponential histogram, by its name and attributes, and summed in
  # billionths, so a cost of a ten-thousandth of a dollar adds up, all with ETS's own counters: no process is asked.
  defp number(name, attributes, value) do
    if :ets.whereis(@numbers) != :undefined do
      key = {name, attributes}
      bucket = if value > 0, do: ceil(:math.log2(value) * 2 ** @scale) - 1, else: :zero
      :ets.update_counter(@numbers, {key, bucket}, 1, {{key, bucket}, 0})
      :ets.update_counter(@numbers, {key, :sum}, round(value * @billion), {{key, :sum}, 0})
    end

    true
  end

  ## The batch

  # Started by One Core, not by OpenTelemetry: a span processor with a `start_link/1` is started by it.
  def child_spec(_), do: %{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}

  @impl GenServer
  def init(nil) do
    for table <- [@spans, @logs], do: :ets.new(table, [:named_table, :public, :ordered_set, write_concurrency: true])
    :ets.new(@numbers, [:named_table, :public, :set, write_concurrency: true, decentralized_counters: true])
    :ok = :logger.add_handler(__MODULE__, __MODULE__, %{level: :warning})
    # That this server started, with the build it runs, is the first batch's whoever hears it: nothing has looked
    # at who hears yet.
    started = %{"time" => System.os_time(:nanosecond), "severity" => "INFO", "eventName" => "one.server.started"}

    :ets.insert(
      @logs,
      {unique(), Map.merge(started, %{"attributes" => %{"one.build" => Builds.build()}, "body" => nil})}
    )

    every = every()
    Process.send_after(self(), :flush, every)
    {:ok, %{from: System.os_time(:nanosecond), every: every}}
  end

  @impl GenServer
  def terminate(_why, _state), do: :logger.remove_handler(__MODULE__)

  @impl GenServer
  def handle_info(:flush, state) do
    to = System.os_time(:nanosecond)
    batch = %{"spans" => taken(@spans, &spanned/1), "logs" => taken(@logs, & &1), "metrics" => numbers(state.from, to)}

    if Enum.any?(Map.values(batch), &(&1 != [])) do
      about =
        Map.merge(batch, %{
          "server" => Server.key(),
          "machine" => Servers.name(Server.key()),
          "build" => Builds.build(),
          "from" => state.from,
          "to" => to
        })

      Hooks.heard("recorded", about, {nil, "network"})
    end

    Process.send_after(self(), :flush, state.every)
    {:noreply, %{state | from: to}}
  end

  # How often what's recorded is handed on, in milliseconds: OpenTelemetry's own setting for a batch of spans, or a
  # minute, so a busy server writes a blob a minute and an idle one none.
  defp every do
    case Integer.parse(System.get_env("OTEL_BSP_SCHEDULE_DELAY", "")) do
      {every, ""} when every > 0 -> every
      _ -> 60_000
    end
  end

  # What a table holds, taken out of it: what's recorded meanwhile stays for the next batch.
  defp taken(table, shape) do
    case :ets.last(table) do
      :"$end_of_table" ->
        []

      last ->
        rows = :ets.select(table, [{{:"$1", :"$2"}, [{:"=<", :"$1", last}], [:"$2"]}])
        :ets.select_delete(table, [{{:"$1", :_}, [{:"=<", :"$1", last}], [true]}])
        Enum.map(rows, shape)
    end
  end

  # Each number's histogram over the batch, as OpenTelemetry's exponential histogram's data point.
  defp numbers(from, to) do
    keys = :ets.select(@numbers, [{{{:"$1", :_}, :_}, [], [:"$1"]}]) |> Enum.uniq()

    for {name, attributes} = key <- keys do
      counts = for {{_key, bucket}, count} <- :ets.match_object(@numbers, {{key, :_}, :_}), do: {bucket, count}
      for {bucket, _count} <- counts, do: :ets.take(@numbers, {key, bucket})
      {sum, counts} = Map.pop(Map.new(counts), :sum, 0)
      {zero, buckets} = Map.pop(counts, :zero, 0)

      %{
        "name" => name,
        "attributes" => attributes,
        "start" => from,
        "time" => to,
        "count" => zero + Enum.sum(Map.values(buckets)),
        "sum" => sum / @billion,
        "scale" => @scale,
        "zeroCount" => zero,
        "buckets" => Enum.sort(for {bucket, count} <- buckets, do: [bucket, count])
      }
    end
  end

  # A span as it's kept: OpenTelemetry's fields, with its times in nanoseconds since 1970.
  defp spanned(ended) do
    span(
      trace_id: trace,
      span_id: id,
      parent_span_id: parent,
      name: name,
      kind: kind,
      start_time: start,
      end_time: finish,
      attributes: attributes,
      events: events,
      status: status
    ) = ended

    %{
      "traceId" => :io_lib.format("~32.16.0b", [trace]) |> IO.iodata_to_binary(),
      "spanId" => :io_lib.format("~16.16.0b", [id]) |> IO.iodata_to_binary(),
      "parentSpanId" => if(is_integer(parent), do: :io_lib.format("~16.16.0b", [parent]) |> IO.iodata_to_binary()),
      "name" => to_string(name),
      "kind" => to_string(kind || :internal),
      "start" => :opentelemetry.timestamp_to_nano(start),
      "end" => :opentelemetry.timestamp_to_nano(finish),
      "attributes" => attributes(:otel_attributes.map(attributes)),
      "status" => ended(status),
      "events" =>
        for event(system_time_native: at, name: name, attributes: attributes) <- :otel_events.list(events) do
          %{
            "time" => :opentelemetry.timestamp_to_nano(at),
            "name" => to_string(name),
            "attributes" => attributes(:otel_attributes.map(attributes))
          }
        end
    }
  end

  defp ended(status(code: code, message: message)), do: %{"code" => to_string(code), "message" => message}
  defp ended(_), do: %{"code" => "unset", "message" => ""}

  # Attributes as JSON holds them, each text cut short.
  defp attributes(attributes) do
    Map.new(attributes, fn {name, value} -> {to_string(name), value(value)} end)
  end

  defp value(value) when is_binary(value), do: short(value)
  defp value(value) when is_number(value) or is_boolean(value), do: value
  defp value(value) when is_atom(value), do: to_string(value)
  defp value(value) when is_list(value), do: Enum.map(value, &value/1)
  defp value(value), do: short(inspect(value))

  defp short(text, longest \\ @longest), do: String.slice(to_string(text), 0, longest)

  defp unique, do: System.unique_integer([:monotonic])
end
