defmodule Telemetry.Forward do
  @moduledoc """
  A copy of what's kept, sent where the network's owner says, as OTLP over HTTP to any OpenTelemetry endpoint
  (Grafana Cloud, Honeycomb, a collector of their own): each export as it's kept, posted to `<endpoint>/v1/<signal>`
  in its protobuf, a minute after it was, as Telemetry's alerts read it (`Telemetry.Alerts.evaluate/1`). The headers
  the endpoint wants, its key among them, are a Vault item's of the owner who said where, switched on for Telemetry,
  filled in by Vault's `fetch` without Telemetry seeing them. It's off until an owner says where, with `forward`, and nothing is sent again that the endpoint missed.
  """
  require Logger
  alias Telemetry.{Alerts, Store}
  alias OneCore.Call

  @doc """
  Where a copy is sent: its `endpoint`, the Vault `item` and the `headers` filled from it, and `who` said so, whose
  item it is, or `nil`.
  """
  def where(call) do
    Alerts.transaction(call, fn db ->
      case Alerts.query(db, "select endpoint, who, item, headers from forward", []) do
        [row] -> %{endpoint: row["endpoint"], who: row["who"], item: row["item"], headers: JSON.decode!(row["headers"])}
        [] -> nil
      end
    end)
  end

  @doc """
  Sends a copy to `endpoint` from now, with `headers` filled in from the Vault `item`, or stops with no endpoint:
  only the network's owners may. It first sends an empty export, so what's wrong with it is said now.
  """
  def set(call, nil, _item, _headers) do
    owns!(call)
    Alerts.transaction(call, &Alerts.query(&1, "delete from forward", []))
    Call.changed(call)
    %{endpoint: nil, item: nil}
  end

  def set(call, endpoint, item, headers) do
    owns!(call)
    if not (endpoint =~ ~r{^https?://[^/]+}), do: raise("The endpoint is an address, as https://otlp.example.com/otlp.")
    endpoint = String.trim_trailing(endpoint, "/")
    who = OneCore.User.owner(call.actor)
    # An empty export request is a whole one, of nothing.
    post(call, %{endpoint: endpoint, who: who, item: item, headers: headers || %{}}, "traces", "")

    Alerts.transaction(call, fn db ->
      Alerts.query(db, "insert or replace into forward (id, endpoint, who, item, headers) values (1, ?, ?, ?, ?)", [
        endpoint,
        who,
        item,
        JSON.encode!(headers || %{})
      ])
    end)

    Call.changed(call)
    %{endpoint: endpoint, item: item}
  end

  @doc "Sends each of the exports kept as `names`, if a copy is sent anywhere."
  def send(_call, []), do: :ok

  def send(call, names) do
    if to = where(call) do
      for {name, otlp} <- Store.raw(call.own, names), signal = Store.signal(name), signal != nil do
        try do
          post(call, to, signal, otlp)
        rescue
          error -> Logger.warning("Telemetry couldn't send a copy to #{to.endpoint}: #{Exception.message(error)}")
        end
      end
    end

    :ok
  end

  defp post(call, to, signal, otlp) do
    request = %{
      "item" => to.item,
      "url" => "#{to.endpoint}/v1/#{signal}",
      "method" => "POST",
      "headers" => Map.put(to.headers, "content-type", "application/x-protobuf"),
      "body_base64" => Base.encode64(otlp)
    }

    # It's the item of whoever said where, as their own call to the Vault would use it.
    case Call.tool(%{call | subject: to.who, actor: nil}, "vault", "fetch", request) do
      %{"status" => status} when status in 200..299 -> :ok
      %{"status" => status} -> raise "#{to.endpoint} answered #{status}"
    end
  end

  defp owns!(call),
    do: if(not Call.owns?(%{call | subject: "network"}), do: raise("Only the network's owners choose where a copy is sent."))
end
