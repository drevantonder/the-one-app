defmodule WebRTC do
  @moduledoc """
  WebRTC: what an app needs to stream something to a person's app as live video, a screen most of all. It hands
  out ICE servers for one stream: TURN credentials of Cloudflare Realtime's, made with the TURN key in the Vault of
  whoever this is installed for (`Cloudflare TURN`, its `key_id` and `api_token`), which no code here ever holds.

  A stream's two ends reach each other only through TURN (`iceTransportPolicy: "relay"`), so neither learns the
  other's addresses, and every network can reach Cloudflare's. Signalling is the streaming app's own: its tool
  takes the app's SDP offer, whole, with its candidates, and answers with its end's SDP answer, as WHEP does, so a
  stream is two tool calls (`ice_servers`, then the offer) and needs nothing pushed.

  The sender is this app's program, `server/bin/stream/`, one for every kind of screen: `sender` gives where each
  kind of machine's build is published, which the streaming app's code on that machine downloads once, checks and
  runs. Its `src/main.rs` says what it takes.

  Only an app gets them: it's the app that knows who may see what it streams.
  """
  @behaviour OneCore.App
  alias OneCore.Call

  @item "Cloudflare TURN"
  # How long a stream's credentials last. A stream that outlives them loses its relay, and its app connects again.
  @ttl 4 * 3600

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.22.18", "vault" => "to use your Cloudflare TURN key"}

  @impl true
  def tools do
    [
      %{
        name: "ice_servers",
        description:
          "For an app's code that streams to a person's app over WebRTC: ICE servers for one stream, TURN only, as RTCPeerConnection takes them (`iceServers`), good for #{div(@ttl, 3600)} hours. Both ends take them, with iceTransportPolicy relay. Not for a person or an agent.",
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
      },
      %{
        name: "sender",
        description:
          "For an app's code that streams to a person's app over WebRTC: the program that sends a screen, `stream <file>`, built for each kind of machine (linux-x86_64, linux-aarch64, macos-aarch64, windows-x86_64), as its `url` and `sha256`. The app's code on that machine downloads it once, checks it, and runs it with the viewer's offer, the ICE servers and its screen's source. Not for a person or an agent.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          description: "By kind of machine",
          additionalProperties: %{
            type: "object",
            properties: %{url: %{type: "string"}, sha256: %{type: "string"}},
            required: ["url", "sha256"]
          }
        }
      }
    ]
  end

  @impl true
  def call(tool, _inputs, %Call{caller: nil}) when tool in ~w(ice_servers sender),
    do: raise("WebRTC's #{tool} is for an app's code, which knows who may see what it streams")

  # As published (`server/checksums.json`), each build named by its path.
  def call("sender", _inputs, _call) do
    for {"server/bin/stream/" <> kind, build} <- JSON.decode!(OneCore.App.file(__MODULE__, "server/checksums.json")),
        into: %{},
        do: {kind, Map.take(build, ["url", "sha256"])}
  end

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
