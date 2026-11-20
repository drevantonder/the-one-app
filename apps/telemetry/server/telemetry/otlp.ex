defmodule Telemetry.Otlp do
  @moduledoc """
  OTLP as it's kept, read: an export request of traces, metrics or logs in OTLP's protobuf, decoded by the encoders
  OpenTelemetry's own exporter brings, and laid flat for `Telemetry.Read` and `Telemetry.Alerts`, each span, log
  line and data point with its resource's attributes under its own, ids in hex and times in nanoseconds since 1970:

  - a span: `traceId`, `spanId`, `parentSpanId`, `name`, `kind` (`server`, `client`, `internal`…), `start`, `end`,
    `attributes`, `status` (`code` `ok` or `error`, and `message`), `events` (`name`, `time`, `attributes`), and
    `server`, the machine it's from;
  - a log line: `time`, `severity` (`INFO`, `WARN`, `ERROR`), `eventName`, `body`, `attributes`, `traceId`, `spanId`
    and `server`;
  - a data point: `name`, `attributes`, `start`, `time`, `count`, `sum`, and a histogram's `bounds`, `buckets`, `min`
    and `max`. A sum's `count` is its value.
  """

  # What a client's export says of the client, kept on each of its metric's points (`points/1`): its build too, as its
  # spans and log lines keep, so a number is told apart by the build that measured it.
  @client ~w(one.device one.for service.name service.version os.type)

  @messages %{
    "traces" => {:opentelemetry_exporter_trace_service_pb, :export_trace_service_request},
    "metrics" => {:opentelemetry_exporter_metrics_service_pb, :export_metrics_service_request},
    "logs" => {:opentelemetry_exporter_logs_service_pb, :export_logs_service_request}
  }

  @doc "The signals there are."
  def signals, do: Map.keys(@messages)

  @doc "An export request of `signal`, as its resource's spans, logs and data points, laid flat."
  def decode(signal, otlp) do
    {module, message} = @messages[signal]
    request = module.decode_msg(otlp, message)
    empty = %{"spans" => [], "logs" => [], "metrics" => []}

    case signal do
      "traces" -> %{empty | "spans" => for(r <- request[:resource_spans] || [], s <- spans(r), do: s)}
      "logs" -> %{empty | "logs" => for(r <- request[:resource_logs] || [], l <- logs(r), do: l)}
      "metrics" -> %{empty | "metrics" => for(r <- request[:resource_metrics] || [], p <- points(r), do: p)}
    end
  end

  @doc """
  An export request of `signal` with `attributes` said of each of its resources, over what it said of itself: as a
  client's are, by the device and person its request proved it is.
  """
  def stamp(signal, otlp, attributes) do
    {module, message} = @messages[signal]
    request = module.decode_msg(otlp, message)
    said = for {key, value} <- attributes, do: %{key: key, value: %{value: {:string_value, value}}}
    resources = String.to_atom("resource_#{if signal == "traces", do: "spans", else: signal}")

    request
    |> Map.update(resources, [], fn all ->
      for r <- all do
        resource = r[:resource] || %{attributes: []}
        kept = Enum.reject(resource[:attributes] || [], &Map.has_key?(attributes, &1.key))
        %{r | resource: %{resource | attributes: kept ++ said}}
      end
    end)
    |> module.encode_msg(message)
  end

  defp spans(%{resource: resource, scope_spans: scopes}) do
    under = attributes(resource)

    for scope <- scopes, span <- scope.spans do
      %{
        "traceId" => hex(span.trace_id),
        "spanId" => hex(span.span_id),
        "parentSpanId" => if(span[:parent_span_id] in [nil, ""], do: nil, else: hex(span.parent_span_id)),
        "name" => span.name,
        "kind" => kind(span[:kind]),
        "start" => span.start_time_unix_nano,
        "end" => span.end_time_unix_nano,
        "attributes" => Map.merge(under, attributes(span[:attributes])),
        "status" => status(span[:status]),
        "events" =>
          for(event <- span[:events] || [],
            do: %{"name" => event.name, "time" => event.time_unix_nano, "attributes" => attributes(event[:attributes])}
          ),
        "server" => under["host.name"]
      }
    end
  end

  defp logs(%{resource: resource, scope_logs: scopes}) do
    under = attributes(resource)

    for scope <- scopes, log <- scope.log_records do
      attributes = Map.merge(under, attributes(log[:attributes]))
      time = if log[:time_unix_nano] in [nil, 0], do: log[:observed_time_unix_nano], else: log.time_unix_nano
      # A log line the SDK made of what was logged here says when it was logged as when it was seen.
      time = if log[:observed_time_unix_nano] not in [nil, 0], do: min(time, log.observed_time_unix_nano), else: time

      %{
        "time" => time,
        "severity" => severity(log[:severity_number]),
        "eventName" => if(log[:event_name] in [nil, ""], do: attributes["event.name"], else: log.event_name),
        "body" => value(log[:body]),
        "attributes" => Map.drop(attributes, ~w(otel_trace_id otel_span_id otel_trace_flags event.name)),
        "traceId" => if(log[:trace_id] in [nil, ""], do: nil, else: hex(log.trace_id)),
        "spanId" => if(log[:span_id] in [nil, ""], do: nil, else: hex(log.span_id)),
        "server" => under["host.name"]
      }
    end
  end

  defp points(%{resource: resource, scope_metrics: scopes}) do
    under = attributes(resource)
    # A client's export says which device and person it's from (`stamp/3`), and which client on what system: each of
    # its points keeps that, to be split by, as its spans do. A server's says only which server, `server`.
    client = if under["one.device"], do: Map.take(under, @client), else: %{}

    for scope <- scopes, metric <- scope.metrics, point <- data(metric.data) do
      Map.merge(point, %{"name" => metric.name, "server" => under["host.name"], "attributes" => Map.merge(client, point["attributes"])})
    end
  end

  defp data({:histogram, %{data_points: points}}) do
    for p <- points do
      %{
        "attributes" => attributes(p[:attributes]),
        "start" => p.start_time_unix_nano,
        "time" => p.time_unix_nano,
        "count" => p.count,
        "sum" => p[:sum] || 0,
        "bounds" => p[:explicit_bounds] || [],
        "buckets" => p[:bucket_counts] || [],
        "min" => p[:min],
        "max" => p[:max]
      }
    end
  end

  defp data({kind, %{data_points: points}}) when kind in [:sum, :gauge] do
    for p <- points do
      value = elem(p.value, 1)

      %{
        "attributes" => attributes(p[:attributes]),
        "start" => p[:start_time_unix_nano] || p.time_unix_nano,
        "time" => p.time_unix_nano,
        "count" => round(value),
        "sum" => value
      }
    end
  end

  defp data(_other), do: []

  # What the SDK says of the process it ran in is the same for all of a server's, and nobody's to read.
  defp attributes(nil), do: %{}

  defp attributes(%{} = resource) do
    resource[:attributes]
    |> attributes()
    |> Map.reject(fn {key, _value} -> String.starts_with?(key, ["process.", "telemetry.sdk."]) end)
  end

  defp attributes(all), do: Map.new(all, &{to_string(&1.key), value(&1[:value])})

  defp value(nil), do: nil
  defp value(%{value: {:kvlist_value, %{values: all}}}), do: attributes(all)
  defp value(%{value: {:array_value, %{values: all}}}), do: Enum.map(all, &value/1)
  defp value(%{value: {_kind, value}}), do: value
  defp value(%{}), do: nil

  defp kind(kind) do
    case kind do
      :SPAN_KIND_SERVER -> "server"
      :SPAN_KIND_CLIENT -> "client"
      :SPAN_KIND_PRODUCER -> "producer"
      :SPAN_KIND_CONSUMER -> "consumer"
      _ -> "internal"
    end
  end

  defp status(%{code: :STATUS_CODE_ERROR} = status), do: %{"code" => "error", "message" => status[:message] || ""}
  defp status(_status), do: %{"code" => "ok", "message" => ""}

  # OpenTelemetry's severity numbers, by their names: ERROR and FATAL are errors, WARN warnings.
  defp severity(number) do
    case to_string(number) do
      "SEVERITY_NUMBER_" <> level when level in ~w(ERROR ERROR2 ERROR3 ERROR4 FATAL FATAL2 FATAL3 FATAL4) -> "ERROR"
      "SEVERITY_NUMBER_WARN" <> _ -> "WARN"
      _ -> "INFO"
    end
  end

  defp hex(bytes), do: Base.encode16(bytes, case: :lower)
end
