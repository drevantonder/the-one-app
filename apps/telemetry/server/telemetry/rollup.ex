defmodule Telemetry.Rollup do
  @moduledoc """
  An hour of what's kept, rolled up once it's over, as observability stores roll up what they're sent: all that
  `Telemetry.Read` needs of it but a trace whole, in the shape `Telemetry.Otlp` lays exports out in, so a look over
  hours reads one small blob for each rather than every export, decoded.

  - Data points of a metric with the same attributes are added up minute by minute, whichever server they're from.
  - The spans kept are those that failed or were refused, and the clients' spans of the traces they're in, which say
    who asked; their events go.
  - The log lines kept are the warnings and errors logged outside a call.

  It's kept as Erlang's external term format, compressed.
  """
  alias Telemetry.Read

  # What `Telemetry.Read` reads of a span's or a log line's attributes: whose a call was, how it ended, which app's
  # page, and who the client is (`Telemetry.Read.from/1`).
  @attributes ~w(one.for one.outcome one.forwarded one.app service.name host.name mcp.client.name user_agent.name)

  @doc "The rollup of `batches`, an hour's exports laid flat, as one batch."
  def of(batches) do
    cut = cut(batches)
    failed = for s <- cut["spans"], failed?(s), into: MapSet.new(), do: s["traceId"]
    %{cut | "spans" => Enum.filter(cut["spans"], &(failed?(&1) or MapSet.member?(failed, &1["traceId"])))}
  end

  @doc """
  What `Telemetry.Read` needs of `batches`, as one batch: as their rollup (`of/1`), but every client's span, since
  the span that failed of its trace may be in another export.
  """
  def cut(batches) do
    %{
      "spans" =>
        for(s <- Enum.flat_map(batches, & &1["spans"]), failed?(s) or s["kind"] == "client",
          do: %{s | "attributes" => Map.take(s["attributes"], @attributes), "events" => []}
        ),
      "logs" =>
        for(l <- Enum.flat_map(batches, & &1["logs"]), l["severity"] in ["WARN", "ERROR"], l["spanId"] == nil,
          do: %{l | "attributes" => Map.take(l["attributes"], @attributes)}
        ),
      "metrics" =>
        batches
        |> Enum.flat_map(& &1["metrics"])
        |> Enum.group_by(&{&1["name"], &1["attributes"], div(&1["time"], 60_000_000_000)})
        |> Enum.map(fn {_minute, points} -> points |> Read.added() |> Map.delete("server") end)
    }
  end

  def encode(rollup), do: :erlang.term_to_binary(rollup, compressed: 6)
  def decode(data), do: :erlang.binary_to_term(data, [:safe])

  defp failed?(span), do: span["status"]["code"] == "error" or span["attributes"]["one.outcome"] == "refused"
end
