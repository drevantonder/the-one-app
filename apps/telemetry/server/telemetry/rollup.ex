defmodule Telemetry.Rollup do
  @moduledoc """
  An hour of what's kept, rolled up as it's kept, as observability stores roll up what they're sent: all that
  `Telemetry.Read` needs of it but a trace whole, in the shape `Telemetry.Otlp` lays exports out in, so a look over
  hours reads one small blob for each rather than every export, decoded.

  - Data points of a metric with the same attributes are added up by the minute they end in, after its start and up
    to its end, as `Telemetry.Read` steps through them, whichever server they're from.
  - The spans kept are those that failed or were refused, and the clients', which say who asked: a client exports
    only those that failed or were slow. Their events go.
  - The log lines kept are the warnings and errors logged outside a call.

  It's kept as Erlang's external term format, compressed.
  """
  alias Telemetry.Read

  # What `Telemetry.Read` reads of a span's or a log line's attributes: whose a call was, how it ended, which app's
  # page, and who the client is (`Telemetry.Read.from/1`).
  @attributes ~w(one.for one.outcome one.forwarded one.app service.name host.name mcp.client.name user_agent.name)

  @doc "The rollup of `batches`, exports laid flat or rollups, as one batch."
  def of(batches) do
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
        |> Enum.group_by(&{&1["name"], &1["attributes"], div(&1["time"] - 1, 60_000_000_000)})
        |> Enum.map(fn {_minute, points} -> points |> Read.added() |> Map.delete("server") end)
    }
  end

  def encode(rollup), do: :erlang.term_to_binary(rollup, compressed: 6)
  def decode(data), do: :erlang.binary_to_term(data, [:safe])

  defp failed?(span), do: span["status"]["code"] == "error" or span["attributes"]["one.outcome"] == "refused"
end
