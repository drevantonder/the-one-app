defmodule Telemetry.Record do
  @moduledoc """
  Records, as Telemetry's tool `record` takes them, put through OpenTelemetry's API on this server, so its SDK exports
  them with the rest (`OneCore.Otlp`): an app that runs in its own VM records this way, where one that runs in the
  server's calls the API itself. Each is
  - with a `value`, a number: a histogram's, by its name and attributes, as `gen_ai.client.token.usage` is;
  - with `took`, in milliseconds, something that ended at its `time`: a span under the span running here, or in the
    trace its `traceId` says, with an `error` if it failed;
  - otherwise a log line, an error if it has an `error`, under the span running here.

  Names, sizes and outcomes only: never what anyone wrote, was given or answered, and never a secret.
  """
  alias OpenTelemetryAPIExperimental.Histogram
  require Histogram
  require OpenTelemetry.Tracer, as: Tracer

  # How long a text that's recorded may be, in characters: an error's message, an attribute.
  @longest 1000
  # How many atoms the VM keeps room for, past which a record's new metric isn't made.
  @room 100_000

  @doc "Records each of `records`, with `attributes` over what each says, at its `time` in ms since 1970, or now."
  def record(records, attributes \\ %{}) do
    for %{"name" => name} = record <- records, is_binary(name) do
      attributes = Map.merge(attributes(record["attributes"] || %{}), attributes)
      time = if is_integer(record["time"]), do: record["time"] * 1_000_000, else: System.os_time(:nanosecond)

      case record do
        %{"value" => value} when is_number(value) -> metric(name, value, attributes)
        %{"took" => took} when is_number(took) -> span(name, time - round(took * 1_000_000), time, record, attributes)
        _ -> log(name, time, record["error"], attributes)
      end
    end

    :ok
  end

  defp metric(name, value, attributes) do
    if atom = instrument(name), do: Histogram.record(atom, value, attributes)
  end

  # The name of the histogram a metric's recorded in, made the first time it's recorded. Its name is an atom, which the VM never
  # lets go of: a name that isn't one yet is made one only while the VM has room for many more.
  defp instrument(name) do
    meter = :opentelemetry_experimental.get_meter(:opentelemetry.get_application_scope(__MODULE__))

    with true <- name =~ ~r/^[A-Za-z][A-Za-z0-9\/_.\-]{0,254}$/,
         atom when is_atom(atom) <- atom(name) do
      if :otel_meter.lookup_instrument(meter, atom) == :undefined,
        do: Histogram.create(atom, %{advisory_params: %{explicit_bucket_boundaries: OneCore.Otlp.buckets()}})

      atom
    else
      _ -> nil
    end
  end

  defp atom(name) do
    String.to_existing_atom(name)
  rescue
    ArgumentError -> if :erlang.system_info(:atom_count) < :erlang.system_info(:atom_limit) - @room, do: String.to_atom(name)
  end

  # A span of what took a while, in the trace a client's says it's in, under the span it says, or else under the
  # span running here.
  defp span(name, start, ended, record, attributes) do
    parent =
      case record do
        %{"traceId" => <<trace::binary-32>>, "spanId" => <<span::binary-16>>} ->
          Tracer.set_current_span(OpenTelemetry.Ctx.new(), :otel_tracer.from_remote_span(hex(trace), hex(span), 1))

        _ ->
          OpenTelemetry.Ctx.get_current()
      end

    kind = if record["kind"] == "client", do: :client, else: :internal
    span = Tracer.start_span(parent, short(name), %{kind: kind, start_time: native(start), attributes: attributes})
    if is_binary(record["error"]), do: OpenTelemetry.Span.set_status(span, OpenTelemetry.status(:error, short(record["error"])))
    OpenTelemetry.Span.end_span(span, native(ended))
  end

  # A log line, which the SDK exports as it's a record (`OneCore.Otlp.private/2`).
  defp log(name, time, error, attributes) do
    level = if is_binary(error), do: :error, else: :info
    metadata = %{record?: true, attributes: Map.put(attributes, "event.name", short(name)), time: div(time, 1000)}
    :logger.log(level, if(is_binary(error), do: short(error), else: short(name)), metadata)
  end

  # A time in nanoseconds since 1970 as the SDK's, the VM's monotonic time.
  defp native(ns), do: System.convert_time_unit(ns, :nanosecond, :native) - System.time_offset(:native)

  defp hex(text), do: String.to_integer(text, 16)

  # Attributes as OpenTelemetry holds them, each text cut short.
  defp attributes(attributes), do: Map.new(attributes, fn {name, value} -> {to_string(name), value(value)} end)

  defp value(value) when is_binary(value), do: short(value)
  defp value(value) when is_number(value) or is_boolean(value), do: value
  defp value(value) when is_list(value), do: Enum.map(value, &value/1)
  defp value(value), do: short(inspect(value))

  defp short(text), do: String.slice(to_string(text), 0, @longest)
end
