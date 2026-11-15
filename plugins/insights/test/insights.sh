#!/usr/bin/env bash
# Proves, on a whole network with a spare (`./network <folder> --spare`), what Insights promises: tool calls, a tool
# that fails, a service that crashes, a slow tool, an app opening on a client and an AI Gateway call through a fake
# model are recorded by the servers and kept by Insights, whose tools answer with the right counts and quantiles; a
# call passed from one server to the other reads back as one trace; what's older than 14 days is let go and an
# hour that's over is put together; and what was given, answered or kept secret appears nowhere in what Insights
# kept. It says what recording costs a call, with Insights on and off.
#
#     mise exec -- plugins/insights/test/insights.sh [--shots <folder>]
#
# With --shots it takes pictures of Insights in Settings, and of AI Gateway's Spending, on a phone and a desktop,
# light and dark, into <folder>.
# Port numbers are free ones, so it runs beside anything else.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is the fake model server.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
export ONE_HOME=$folder/client
shots=
[[ ${1:-} == --shots ]] && shots=$(mkdir -p "$2" && cd "$2" && pwd)
# Each server hands on what it recorded every second, not every minute, so the proof needn't wait.
OTEL_BSP_SCHEDULE_DELAY=1000 "$root/network" "$folder" --spare >"$folder/network.log" 2>&1 &
started=($!)
stop() {
  local left
  for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done
  wait 2>/dev/null || true
  for left in $(pgrep -f "$folder" || true); do tail --pid="$left" -f /dev/null; done
  rm -rf "$folder"
}
trap stop EXIT
grep -q -m1 "A network is running" <(tail -f --pid="${started[0]}" -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
one=$root/target/release/one

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.3f", all[int((NR + 1) / 2)] / 1000 }'; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
free_port() { bun -e 'const server = Bun.serve({ port: 0, fetch: () => new Response() }); console.log(server.port); server.stop()'; }
declare -A key
key_of() { key[$1]=$(grep -o -E "running in [^ ]*: [0-9a-f]{64}" "$folder/$1.log" | head -1 | grep -o -E "[0-9a-f]{64}$"); }
key_of server && key_of spare

# What's given, answered and kept secret carries a marker. Nothing prints it: what Insights kept is searched for it.
marker=insights-proof-$(random 8)
mkdir "$folder/secrets"
printf '%s-%s' "$marker" "$(random 8)" >"$folder/secrets/key"

# A command as alice, through either server or `via <server>` through that one alone.
as() {
  local home=$folder/client
  [[ $1 == via ]] && home=$folder/only-$2 && shift 2
  ONE_AS=alice ONE_HOME=$home "$one" "$@" 2>&1
}
as_owner() { ONE_AS=owner ONE_FOR=network "$one" "$@" 2>&1; }
shown() { echo "  $1  $(sed -e "s|$folder|<folder>|g" -e "s/$marker/<marker>/g" <<<"${*:2}" | cut -c1-160)" && echo "           -> $(sed -e "s|$folder|<folder>|g" -e "s/$marker/<marker>/g" <<<"$said" | cut -c1-300)"; }
may() { said=$("$@") && shown "may    " "$@" || { echo "this was refused, and shouldn't be: $* -> $said" && exit 1; }; }
refused() { ! said=$("$@") && shown "refused" "$@" || { echo "this was answered, and shouldn't be: $* -> $said" && exit 1; }; }
# Stops the proof unless what it's given holds: `holds test "$(of .count)" = 20`.
holds() { "$@" || { echo "that isn't so: $*" && exit 1; }; }
# What a JSON answer says at a path: `of .count`, `of .0.count`.
of() { python3 -I -c 'import json, sys
v = json.loads(sys.argv[1])
for k in sys.argv[2].strip(".").split("."):
    if k: v = v[int(k)] if k.lstrip("-").isdigit() else v[k]
print(v if isinstance(v, str) else json.dumps(v))' "$said" "$1"; }
pick() { python3 -I -c "import json, sys; xs = json.loads(sys.argv[1]); print(json.dumps(next(x for x in xs if $1)))" "$said"; }
# Insights has what the servers recorded once each has handed it on, a second after.
handed_on() { sleep 3; }

# A plugin to prove with. `echo` answers how long what it was given is, and records that as a number and a log
# line; `fail` refuses; `slow` takes 300 ms; `count` keeps a count in its database, which the server holding its
# lease writes, so a call through the other is passed on; `bomb` lights the service `Probe.Bomb` with what it was
# given, and the service crashes holding it.
mkdir -p "$folder/probe/server"
cat >"$folder/probe/server/probe.ex" <<'EOF'
defmodule Probe do
  @behaviour OneCore.Plugin

  @impl true
  def uses, do: ["database"]

  @impl true
  def tools do
    text = %{type: "object", properties: %{text: %{type: "string"}}}

    [
      %{name: "ping", read_only: true, description: "Answers, doing nothing", inputs: %{type: "object"}},
      %{name: "echo", description: "Says how long the text is", inputs: text},
      %{name: "fail", description: "Refuses", inputs: text},
      %{name: "slow", read_only: true, description: "Takes 300 ms", inputs: %{type: "object"}},
      %{name: "count", description: "Counts, or refuses", inputs: %{type: "object", properties: %{fail: %{type: "boolean"}}}},
      %{name: "bomb", description: "Lights the bomb", inputs: text}
    ]
  end

  @impl true
  def call("ping", _inputs, _call), do: "pong"

  def call("echo", %{"text" => text}, _call) do
    OneCore.Telemetry.record("probe.echoed", %{"kind" => "text"})
    OneCore.Telemetry.record("probe.bytes", %{"kind" => "text"}, byte_size(text))
    "#{byte_size(text)} bytes"
  end

  def call("fail", _inputs, _call), do: raise("Probe refused on purpose")

  def call("slow", _inputs, _call) do
    OneCore.Telemetry.record("probe.nap", %{}, fn -> Process.sleep(300) end)
    "slept"
  end

  def call("count", inputs, call) do
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, ["create table counted (at integer)"], fn db ->
      if inputs["fail"], do: raise("Count refused on purpose")
      database.query(db, "insert into counted values (1)")
      "counted"
    end)
  end

  def call("bomb", %{"text" => text}, call), do: OneCore.Service.call(call, Probe.Bomb, :light, [text])
end

defmodule Probe.Bomb do
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def called, do: "Probe's bomb"

  @impl OneCore.Service
  def needs, do: "Nothing"

  def light(service, text) do
    GenServer.cast(service, {:light, text})
    "lit"
  end

  @impl GenServer
  def init(_), do: {:ok, nil}

  @impl GenServer
  def handle_cast({:light, text}, _state) do
    send(self(), :explode)
    {:noreply, %{holding: text}}
  end

  @impl GenServer
  def handle_info(:explode, _state), do: raise("Bomb went off")
end
EOF

say "Calls, a failing tool, a crashing service, a slow tool and a call passed between servers"
may as_owner deploy --name probe --files-from "$folder/probe"
for _ in $(seq 20); do as probe echo --text "$marker said this" >/dev/null; done
echo "  probe echo, 20 times, each given the marker"
refused as probe fail --text "$marker"
refused as probe fail --text "$marker"
for _ in $(seq 5); do as probe slow >/dev/null; done
echo "  probe slow, 5 times"
may as via server probe count
refused as via spare probe count --fail
may as_owner install_service --plugin probe --service Probe.Bomb --server "${key[server]}"
may as probe bomb --text "$marker"

say "A client says an app opened, and that a call failed before a server answered"
opened='{"jsonrpc":"2.0","method":"notifications/one/recorded","params":{"client":"linux","opened":[{"view":"home","ms":412},{"view":"probe","ms":180}],"failed":[{"tool":"probe_echo","why":"No server is answering."}]}}'
ONE_AS=alice "$one" mcp <<<"$opened" >/dev/null
echo "  home opened in 412 ms, probe in 180 ms, and probe_echo failed unanswered"

say "An AI Gateway call, through a fake model that says what it cost"
hosted=$(free_port)
(KEY=$(cat "$folder/secrets/key") SLOW=1 exec bun "$root/plugins/ai-gateway/test/fake.js" "$hosted") >>"$folder/fake.log" 2>&1 &
started+=($!)
curl -s -o /dev/null --retry 30 --retry-all-errors --retry-delay 1 "http://127.0.0.1:$hosted/seen"
mkdir -p "$folder/phone" && cp "$folder/only-server/servers" "$folder/phone/servers"
phone() { ONE_AS=alice ONE_HOME=$folder/phone "$one" "$@" 2>&1; }
may phone encryption let_in
may phone vault grant --servers "[\"${key[server]}\"]"
may phone vault set --item "Fake Cloud" --field url --value "http://127.0.0.1:$hosted/hosted/v1" --secret false --hosts "[\"127.0.0.1:$hosted\"]"
may phone vault set --item "Fake Cloud" --field api_key --value-from "$folder/secrets/key"
may phone vault use --item "Fake Cloud" --plugin ai-gateway
for _ in 1 2 3; do phone ai-gateway chat --model "Fake Cloud/cloud-text" --messages "[{\"role\":\"user\",\"content\":\"$marker\"}]" >/dev/null; done
echo "  Fake Cloud/cloud-text, 3 times, asked the marker"
handed_on

say "Insights' tools answer with the right counts and quantiles"
may as insights slowest
echoed=$(pick 'x.get("tool") == "echo"') slow=$(pick 'x.get("tool") == "slow"') failed=$(pick 'x.get("tool") == "fail"') home=$(pick 'x.get("view") == "home"')
said=$echoed && holds test "$(of .count)" = 20
echo "  probe echo was called 20 times"
said=$failed && holds test "$(of .count)/$(of .failed)" = 2/2
echo "  probe fail was called twice, and failed twice"
# A quantile is the middle of the histogram's bucket it's in, which is within about 4.5% of what was measured.
said=$slow && holds python3 -I -c 'import json, sys; s = json.loads(sys.argv[1]); assert s["count"] == 5 and 285 <= s["p50"] <= 345 and 285 <= s["p95"] <= 345, s' "$said"
echo "  probe slow's median and 95th percentile are about 300 ms: $(of .p50) and $(of .p95) ms"
said=$home && holds python3 -I -c 'import json, sys; s = json.loads(sys.argv[1]); assert s["count"] == 1 and 390 <= s["p50"] <= 435, s' "$said"
echo "  home opened once, in about 412 ms: $(of .p50) ms"
may as insights metric --name probe.bytes
holds python3 -I -c 'import json, sys; s = json.loads(sys.argv[1])["steps"]; n = sum(x["count"] for x in s); t = sum(x.get("sum", 0) for x in s); assert n == 20 and t == 20 * int(sys.argv[2]), (n, t)' "$said" "$(printf '%s said this' "$marker" | wc -c)"
echo "  probe.bytes counted 20 texts, and summed their bytes"

may as insights errors
for message in "Probe refused on purpose" "Count refused on purpose" "Bomb went off" "No server is answering."; do
  [[ $said == *"$message"* ]] || { echo "the errors don't say: $message" && exit 1; }
done
said=$(pick 'x["message"] == "Probe refused on purpose"') && holds test "$(of .count)" = 2
echo "  each error is there once, with how often: probe fail twice, the passed-on count once, the bomb, the unanswered call"
may as insights errors
said=$(pick 'x["where"] == "probe count"')
trace=$(of .trace)

say "A call passed from the spare to the server holding the lease reads back as one trace"
may as insights trace --id "$trace"
holds python3 -I -c '
import json, sys
spans = json.loads(sys.argv[1])["spans"]
assert {s["server"] for s in spans} == {"server", "spare"}, spans
spare = next(s for s in spans if s["server"] == "spare")
server = next(s for s in spans if s["server"] == "server")
assert server["parent"] == spare["span"] and server["depth"] == 1, spans
assert server["outcome"] == "error" and server["message"] == "Count refused on purpose", server
' "$said"
echo "  its spans are on both servers, the server's under the spare's, which passed it on"

say "What AI used, from AI Gateway's calls"
may as insights spending
said=$(pick 'x["model"] == "Fake Cloud/cloud-text"')
holds test "$(of .input)/$(of .cost)" = 30/0.0012
echo "  Fake Cloud/cloud-text: $(of .input) tokens in, $(of .output) out, \$$(of .cost), today"

say "What's kept: nothing given, answered or secret"
kept=$folder/bucket/network/own/insights/batches
dump() { erl -noshell -eval 'lists:foreach(fun(F) -> {ok, B} = file:read_file(F), io:format("~tp~n", [binary_to_term(B)]) end, init:get_plain_arguments()), halt().' -extra "$@"; }
dump $(find "$kept" -type f) >"$folder/kept.txt"
echo "  $(find "$kept" -type f | wc -l) batches kept, $(wc -c <"$folder/kept.txt") bytes as text, holding the marker: $(grep -c -F -e "$marker" "$folder/kept.txt" || true) times"
holds test "$(grep -c -F -e "$marker" "$folder/kept.txt" || true)" = 0
holds grep -q "Bomb went off" "$folder/kept.txt"
echo "  the bomb's crash is kept as its message, and not the state it held"

say "An hour that's over is put together, and what's older than 14 days is let go"
batch=$(find "$kept" -type f | head -1)
old=$kept/$(date -u -d '20 days ago' +%F)/03 && mkdir -p "$old"
earlier=$kept/$(date -u -d '2 hours ago' +%F/%H) && mkdir -p "$earlier"
size=$(wc -c <"$batch")
for planted in "$old/1000-planted-$size" "$earlier/1000-plantedA-$size"; do cp "$batch" "$planted"; done
other=$(find "$kept" -type f -newer "$batch" | grep -v planted | head -1)
cp "$other" "$earlier/2000-plantedB-$(wc -c <"$other")"
echo "  planted one batch 20 days ago, and two two hours ago"
may as insights tidy
holds test "$(find "$old" -type f | wc -l)" = 0
echo "  the one from 20 days ago is gone"
holds test "$(find "$earlier" -type f | wc -l)/$(find "$earlier" -type f -name 'hour-*' | wc -l)" = 1/1
echo "  the two of two hours ago are one hour now"
may as insights tidy
holds test "$(of .put_together)" = 0
echo "  and tidying again does nothing"

say "What recording costs a call: a tool that does nothing, through one server over one connection, the median of 300"
timed() {
  local times=$folder/times-$1 began answer
  coproc MCP { ONE_AS=alice ONE_HOME=$folder/only-server "$one" mcp; }
  for _ in $(seq 300); do
    began=$(now)
    echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"probe_ping","arguments":{}}}' >&"${MCP[1]}"
    read -r answer <&"${MCP[0]}"
    echo $(($(now) - began)) >>"$times"
  done
  exec {MCP[1]}>&-
  wait "$MCP_PID"
  [[ $answer == *'"result"'* && $answer != *isError* ]] || { echo "probe_ping wasn't answered: $answer"; exit 1; }
  median "$times"
}
on=$(timed on)
may as_owner remove --name insights
off=$(timed off)
echo "  with Insights: $on ms. Without it, recording nothing: $off ms."

if [[ -n $shots ]]; then
  say "Pictures of Insights in Settings, into $shots"
  may as_owner add --plugin network/plugins/insights
  for _ in 1 2 3; do as probe slow >/dev/null; as probe echo --text "$marker" >/dev/null; done
  handed_on
  (cd "$root/core/edge" && ONE_AS=owner bun "$root/clients/shell/test/window.js" "$shots" "$root/plugins/insights/test/shots.js")
  (cd "$root/core/edge" && ONE_AS=alice ONE_HOME=$folder/phone bun "$root/clients/shell/test/window.js" "$shots" "$root/plugins/insights/test/gateway.js")
  ls "$shots"
fi

echo
echo "Insights did all it promises."
