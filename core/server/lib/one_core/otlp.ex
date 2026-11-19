defmodule OneCore.Otlp do
  @moduledoc """
  Where this server's OpenTelemetry SDK exports to: what it gathered of its spans, metrics and logs, a batch a minute
  (`config/runtime.exs`), as OTLP's protobuf, encoded by OpenTelemetry's own exporters' encoders, and told to the
  apps installed for the network that observe `exported`, as Telemetry does to keep it. So the SDK is the server's,
  one per VM as OpenTelemetry has it, and an app can't start it: the apps only say where what it exports goes.

  What it exports is names, sizes, durations and outcomes, never what anyone wrote, was given or answered: a log
  line is only a warning or an error, or what's logged as a record (`record?: true` in its metadata, as Telemetry's
  `record` does), and a crash is kept as its exception's banner, never the state or message it crashed on
  (`private/2`).
  """
  alias OneCore.{Hooks, Server, Servers}
  alias OpenTelemetryAPIExperimental.{Counter, Histogram}
  require Counter
  require Histogram

  @doc """
  The buckets a duration in milliseconds is counted in: a quantile read from them is within its bucket, finer where a
  person tells one from another.
  """
  def buckets, do: [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000, 30_000, 60_000, 300_000]

  @doc false
  # Starts what exports logs: the SDK's `:logger` handler, which takes only what `private/2` lets through. And makes
  # the instruments One Core measures with.
  def start do
    Histogram.create(:"one.tool.duration", %{
      unit: :ms,
      description: "How long a tool call took, where it ran",
      advisory_params: %{explicit_bucket_boundaries: buckets()}
    })

    Counter.create(:"one.hook.timeouts", %{description: "An app that didn't answer a hook in time"})

    Histogram.create(:"one.server.connect.duration", %{
      unit: :ms,
      description: "How long connecting to another server took, by the path it's on (direct or relay), or unreachable",
      advisory_params: %{explicit_bucket_boundaries: buckets()}
    })

    :ok =
      :logger.add_handler(:otel, :otel_log_handler, %{
        level: :info,
        filter_default: :log,
        filters: [private: {&__MODULE__.private/2, nil}],
        exporter: {__MODULE__, :logs},
        scheduled_delay_ms: Application.fetch_env!(:opentelemetry, :processors)[:otel_batch_processor].scheduled_delay_ms
      })
  end

  @doc false
  # A `:logger` filter: a warning or an error, or a record, with only what it said and its trace, and a crash as its
  # exception's banner. A report is a term, which may hold anything: only a crash's reason is kept of one.
  def private(%{level: level, msg: msg, meta: meta} = event, _config) do
    record = meta[:record?] == true

    if record or :logger.compare_levels(level, :warning) != :lt do
      body =
        case meta do
          %{crash_reason: {reason, stacktrace}} when is_list(stacktrace) -> crashed(:error, reason, stacktrace)
          %{crash_reason: reason} -> crashed(:exit, reason, [])
          _ -> message(msg)
        end

      kept = Map.take(meta, [:time, :otel_trace_id, :otel_span_id, :otel_trace_flags])
      attributes = if record, do: meta[:attributes] || %{}, else: %{}
      if body, do: %{event | msg: {:string, String.slice(body, 0, 1000)}, meta: Map.merge(attributes, kept)}, else: :stop
    else
      :stop
    end
  end

  defp crashed(kind, reason, stacktrace),
    do: String.replace_prefix(Exception.format_banner(kind, reason, stacktrace), "** ", "")

  defp message({:string, text}), do: IO.chardata_to_string(text)
  defp message({format, arguments}) when is_list(arguments), do: IO.chardata_to_string(:io_lib.format(format, arguments))
  defp message({:report, _report}), do: nil

  ## The exporter, for each of the SDK's signals

  @doc false
  def init(signal), do: {:ok, signal}

  @doc false
  def shutdown(_signal), do: :ok

  @doc false
  def export(spans, resource, :traces),
    do:
      exported(
        "traces",
        :otel_otlp_traces.to_proto(spans, resource(resource)),
        &:opentelemetry_exporter_trace_service_pb.encode_msg(&1, :export_trace_service_request)
      )

  def export(metrics, resource, :metrics),
    do:
      exported(
        "metrics",
        :otel_otlp_metrics.to_proto(metrics, resource(resource)),
        &:opentelemetry_exporter_metrics_service_pb.encode_msg(&1, :export_metrics_service_request)
      )

  def export({logs, config}, resource, :logs),
    do:
      exported(
        "logs",
        :otel_otlp_logs.to_proto(logs, resource(resource), config),
        &:opentelemetry_exporter_logs_service_pb.encode_msg(&1, :export_logs_service_request)
      )

  defp exported(_signal, :empty, _encode), do: :ok

  defp exported(signal, request, encode) do
    otlp = Base.encode64(IO.iodata_to_binary(encode.(request)))
    Hooks.observe("exported", %{"signal" => signal, "otlp" => otlp}, {nil, Servers.owner()})
  end

  # The SDK's resource, with which server this is, by its key and its machine's name, which it knows once it runs.
  defp resource(resource) do
    key = Server.key()
    said = %{"one.server" => key, "host.name" => Servers.name(key) || String.slice(key, 0, 8)}
    :otel_resource.merge(:otel_resource.create(said), resource)
  end
end
