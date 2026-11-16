defmodule WebRTC do
  @moduledoc """
  WebRTC: what a plugin needs to stream something to a person's app as live video, a screen most of all. It hands
  out ICE servers for one stream: TURN credentials of Cloudflare Realtime's, made with the TURN key in the Vault of
  whoever this is installed for (`Cloudflare TURN`, its `key_id` and `api_token`), which no code here ever holds.

  A stream's two ends reach each other only through TURN (`iceTransportPolicy: "relay"`), so neither learns the
  other's addresses, and every network can reach Cloudflare's. Signalling is the streaming plugin's own: its tool
  takes the app's SDP offer, whole, with its candidates, and answers with its end's SDP answer, as WHEP does, so a
  stream is two tool calls (`ice_servers`, then the offer) and needs nothing pushed.

  Only a plugin gets them: it's the plugin that knows who may see what it streams.
  """
  @behaviour OneCore.Plugin
  alias OneCore.Call

  @item "Cloudflare TURN"
  # How long a stream's credentials last. A stream that outlives them loses its relay, and its app connects again.
  @ttl 4 * 3600

  @impl true
  def permissions, do: ~w(vault_fetch)

  @impl true
  def tools do
    [
      %{
        name: "ice_servers",
        description:
          "For a plugin's code that streams to a person's app over WebRTC: ICE servers for one stream, TURN only, as RTCPeerConnection takes them (`iceServers`), good for #{div(@ttl, 3600)} hours. Both ends take them, with iceTransportPolicy relay. Not for a person or an agent.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            iceServers: %{
              type: "array",
              description: "As RTCPeerConnection takes them",
              items: %{
                type: "object",
                properties: %{
                  urls: %{type: "array", items: %{type: "string"}},
                  username: %{type: "string"},
                  credential: %{type: "string"}
                },
                required: ["urls"]
              }
            }
          },
          required: ["iceServers"]
        }
      }
    ]
  end

  @impl true
  def call("ice_servers", _inputs, %Call{caller: nil}),
    do: raise("ICE servers are for a plugin's code, which knows who may see what it streams")

  def call("ice_servers", _inputs, call) do
    request = %{
      "item" => @item,
      "method" => "POST",
      "url" => "https://rtc.live.cloudflare.com/v1/turn/keys/{{key_id}}/credentials/generate-ice-servers",
      "headers" => %{"authorization" => "Bearer {{api_token}}", "content-type" => "application/json"},
      "body" => JSON.encode!(%{ttl: @ttl})
    }

    # The key is whoever this is installed for's, the network's most often, which every member uses.
    case Call.tool(%{call | subject: call.installed}, "vault", "fetch", request) do
      %{"status" => 201, "body" => body} -> %{"iceServers" => turn(JSON.decode!(body)["iceServers"])}
      %{"status" => status, "body" => body} -> raise "Cloudflare refused TURN credentials (#{status}): #{body}"
    end
  end

  # TURN over UDP, over TCP, and over TLS on 443 where only that gets out: each a way to the same relay, which an end
  # gathers through before it answers. The others' ports, and STUN, which is no use to a relay, are left out.
  defp turn(servers) do
    for server <- servers,
        urls = Enum.filter(List.wrap(server["urls"]), &(&1 =~ ~r/^turn:.*:3478\?transport=(udp|tcp)$|^turns:.*:443\?transport=tcp$/)),
        urls != [],
        do: Map.put(server, "urls", urls)
  end
end
