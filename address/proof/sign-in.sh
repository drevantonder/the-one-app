#!/usr/bin/env bash
# Proves signing in, on a whole network on this machine that people sign in to (`./network <folder> --address
# --owner alice`), with nothing deployed. Two people join with a passkey, in a real browser each, and each sees
# only their own. A device is nobody's until its person signs in for it, a second one is let in from where they're
# already signed in with nothing typed, and one that's removed is refused the next thing it asks, by every server. A stranger who
# knows a server's key and where it is, and dials it with a key of their own, is refused. An assistant somewhere
# else is let in at /mcp as MCP's authorization says, acts as whoever let it in, and is one of their devices. A
# network without the plugin that decides who may is its first owner's alone, and one without the plugin that
# signs people in answers nobody. It says what a call costs once a device is someone's.
#
#     mise exec -- address/proof/sign-in.sh
#
# It needs `xvfb-run`, `dbus-run-session` and `xdotool`, for the app on a screen of its own.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each server started here.

root=$(cd "$(dirname "$0")/../.." && pwd)
folder=$(mktemp -d)
"$root/network" "$folder" --address --owner alice >"$folder/network.log" 2>&1 &
network=$!
started=("$network")
# The people leave when nothing more is asked of them, and the rest is stopped.
trap '[[ -z ${to_people-} ]] || exec {to_people}>&-; for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -f --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log" "$folder/server.log"; exit 1; }
one=$root/target/release/one
address=$(grep -o -m1 'http://localhost:[0-9]*' "$folder/network.log")

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
# What's printed names no folder, key or link of this run.
plain() { sed -E -e "s|$folder|<folder>|g" -e "s|$root/||g" -e "s|$address|<address>|g" -e 's/[0-9a-f]{64}/<key>/g' -e 's/(invite=)[0-9a-f]+/\1…/' -e 's/"added":"[^"]*"/"added":"…"/g'; }

# People, each in a browser of their own with a passkey of their own: `browser alice <address>` has alice open it
# and answer the one question the page asks.
mkfifo "$folder/to-people" "$folder/from-people"
(cd "$root/address" && exec bun proof/people.js <"$folder/to-people" >"$folder/from-people") &
exec {to_people}>"$folder/to-people" {from_people}<"$folder/from-people"
browser() {
  echo "$1 $2" >&"$to_people"
  read -r said <&"$from_people"
  echo "  $1 opens $(plain <<<"$2")"
  echo "           -> $(plain <<<"$said")"
}
# A device, by the name of its home here: a new one knows the network's address and nothing else.
device() {
  [[ -e $folder/$1 ]] || { mkdir "$folder/$1" && printf '{"address":"%s"}' "$address" >"$folder/$1/network"; }
  ONE_HOME=$folder/$1 "$one" "${@:2}"
}
# A stranger: a client with a key of its own that's told where the server is and nothing of the address, so it
# reaches the server directly. `mix network` leaves one, `only-server`.
stranger() { ONE_HOME=$folder/only-server "$one" "$@"; }
# What a command says is shown with it. `may` is one that has to be answered, and `refused` one that mustn't be.
shown() { echo "  $1  ${*:2}" | plain && echo "           -> $(plain <<<"$said" | tr '\n' ' ')"; }
may() { said=$("$@" 2>&1) && shown "may    " "$@" || { echo "this was refused, and shouldn't be: $* -> $said" && exit 1; }; }
refused() { ! said=$("$@" 2>&1) && shown "refused" "$@" || { echo "this was answered, and shouldn't be: $* -> $said" && exit 1; }; }
# Where a device that isn't in says to let it in.
let_in() { { device "$1" sign-in devices 2>&1 || true; } | grep -o 'http://[^ ]*'; }
# Stops the network's server, and starts it again as it was started with whatever else is given.
restart() {
  local server
  server=$(pgrep -f "mix network $folder server " | head -1)
  kill -9 "$server"
  tail --pid="$server" -f /dev/null
  (cd "$root/server" && exec mix network "$folder" server --address "$address" "$@") >"$folder/server.log" 2>&1 &
  server=$!
  started+=("$server")
  # It's stopped by its number from here on, and the shell says nothing of it.
  disown "$server"
  grep -q -m1 "A server is running" <(tail -f --pid="$server" -n +1 "$folder/server.log")
}

say "Nobody is anybody yet: the network's first owner joins with a passkey, by the link its first server gave"
refused device laptop sign-in devices
joins=$(grep -o -m1 "http://[^ ]*invite=[0-9a-f]*" "$folder/server.log")
browser alice "$joins"
browser mallory "$joins"

say "A device is nobody's until its person signs in for it, in their browser, with nothing typed"
link=$(let_in laptop)
browser alice "$link"
may device laptop sign-in devices

say "Someone else joins by an invitation, and each sees only their own"
refused device bobs-phone sign-in devices
invited=$(device laptop sign-in invite --name bob)
echo "  alice invites bob from her laptop: $(plain <<<"$invited")"
browser bob "$(grep -o "http://[^ ]*" <<<"$invited")"
browser bob "$(let_in bobs-phone)"
echo "  each has a plugin of their own to keep notes with, deployed from their own device:"
may device laptop deploy --name tally --files-from "$root/server/test/plugins/tally"
may device laptop tally add --note "Alice's"
refused device bobs-phone tally list
may device bobs-phone deploy --name tally --files-from "$root/server/test/plugins/tally"
may device bobs-phone tally add --note "Bob's"
may device bobs-phone tally list
may device laptop tally list
refused device bobs-phone sign-in invite --name carol
echo "  what each is told of their own devices and passkeys:"
may device laptop sign-in devices
may device bobs-phone sign-in devices

say "A second device is let in from where its person is already signed in"
refused device desktop sign-in devices
link=$(let_in desktop)
browser alice "$link"
may device desktop sign-in devices

say "The app starts on the same sign-in page, and shows its person's home screen once they've signed in for it"
# A web view has no passkeys, so the page's button there hands the page to the device's own browser. What stands
# in for that browser here writes down what it was asked to open, and alice opens that in hers.
mkdir "$folder/tablet" "$folder/browser" && printf '{"address":"%s"}' "$address" >"$folder/tablet/network"
printf '#!/bin/sh\necho "$1" >"%s"\n' "$folder/opened" >"$folder/browser/xdg-open" && chmod +x "$folder/browser/xdg-open"
: >"$folder/screen" >"$folder/app.log" >"$folder/opened"
PATH=$folder/browser:$PATH ONE_HOME=$folder/tablet dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
  echo "DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY" >"$1/screen"
  exec "$0" 2>"$1/app.log"' "$one" "$folder" >/dev/null 2>&1 &
started+=($!)
grep -q -m1 DISPLAY <(tail -f --pid=$! -n +1 "$folder/screen")
window() { env $(cat "$folder/screen") xdotool search --sync --onlyvisible --name "The One App" "$@" >/dev/null; }
window
echo "  the app's window is open on a device that isn't in, and its home screen has opened $(grep -c "home opened" "$folder/app.log") times"
# The page takes a moment to draw, and its one button is in the middle of the window, 447 down.
sleep 4
env $(cat "$folder/screen") xdotool mousemove 512 447 sleep 0.5 click 1
grep -q -m1 "sign-in" <(tail -f -n +1 "$folder/opened")
echo "  its button is pressed, and it asks the device's browser to open $(plain <"$folder/opened")"
browser alice "$(cat "$folder/opened")"
window windowfocus
timeout 30 grep -q -m1 "home opened" <(tail -f -n +1 "$folder/app.log")
echo "  alice comes back to the window, and it says: $(grep -o -m1 "home opened" "$folder/app.log")"

say "A device that's removed is refused the next thing it asks"
may device laptop sign-in remove_device --device "$(grep -o 'device=[0-9a-f]*' <<<"$link" | cut -d= -f2)"
refused device desktop sign-in devices
may device laptop sign-in devices

say "A stranger who knows a server's key and where it is, and dials it with a key of their own, is refused"
echo "  the stranger knows $(plain <"$folder/only-server/servers" | cut -c1-90)…"
refused stranger sign-in devices
ONE_AS=alice refused stranger sign-in devices
ONE_FOR=network refused stranger deploy --name mine --files-from "$root/server/test/plugins/tally"

say "An assistant somewhere else is let in at /mcp as MCP's authorization says, by the person it then acts as"
echo "  asked with no token, the address answers:"
curl -s -i -m 20 -X POST "$address/mcp" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' |
  grep -i -E "^HTTP|^www-authenticate|jsonrpc" | plain | sed 's/^/    /'
coproc ASSISTANT { cd "$root/address" && exec bun proof/assistant.js "$address" tally_list 2>&1; }
assistant_pid=$ASSISTANT_PID
started+=("$assistant_pid")
read -r authorize <&"${ASSISTANT[0]}"
read -r said <&"${ASSISTANT[0]}" && echo "  the assistant, an MCP client that knows only the address, is $said"
browser alice "${authorize#authorize }"
read -r said <&"${ASSISTANT[0]}" && echo "  the assistant: $said"
read -r said <&"${ASSISTANT[0]}" && echo "  the assistant: $(plain <<<"$said")"
echo "  it's among alice's devices, and not bob's:"
may device laptop sign-in devices
assistant=$(grep -o '"id":"[0-9a-f]*","kind":"assistant"' <<<"$said" | cut -d'"' -f4)
may device bobs-phone sign-in devices
may device laptop sign-in remove_device --device "$assistant"
echo "tally_list" >&"${ASSISTANT[1]}"
read -r said <&"${ASSISTANT[0]}" && echo "  the assistant, once she removed it, with the token it had and then the one it renews that with: $said"
exec {ASSISTANT[1]}>&-
wait "$assistant_pid" 2>/dev/null || true

say "What a call costs once a device is someone's, over one connection as an agent's MCP session keeps"
# A device that reaches the server directly, as `only-server` does, which alice lets in. Each call is placed by its
# device's key, which the server asked the plugin about once and kept: the median of 300 calls.
cat >"$folder/ping.ex" <<'EOF'
defmodule Ping do
  @behaviour OneCore.Plugin

  @impl true
  def tools, do: [%{name: "ping", read_only: true, description: "Answers, calling nothing", inputs: %{type: "object"}}]

  @impl true
  def call("ping", _inputs, _call), do: "pong"
end
EOF
mkdir "$folder/ping" && mv "$folder/ping.ex" "$folder/ping/"
ONE_FOR=network device laptop deploy --name ping --files-from "$folder/ping" >/dev/null
direct=$(cd "$root/server" && mix run --no-start -e 'IO.puts(elem(OneCore.Iroh.key(hd(System.argv())), 1))' "$folder/only-server/key" | tail -1)
browser alice "$address/sign-in#device=$direct&name=direct"
coproc MCP { ONE_HOME=$folder/only-server "$one" mcp; }
for _ in $(seq 300); do
  began=$(now)
  echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ping_ping","arguments":{}}}' >&"${MCP[1]}"
  read -r answer <&"${MCP[0]}"
  echo $(($(now) - began)) >>"$folder/times"
done
exec {MCP[1]}>&-
wait "$MCP_PID"
[[ $answer == *'"result"'* && $answer != *isError* ]] || { echo "ping wasn't answered: $answer"; exit 1; }
echo "  $(sort -n "$folder/times" | awk '{ all[NR] = $1 } END { printf "%.3f ms", all[int((NR + 1) / 2)] / 1000 }')  a tool that does nothing, called by a device that signed in"

say "With a second server, a device that's removed is nobody's at both, the next thing it asks of each"
# The spare is let in as any server is, by the network's owner from a device of hers, and started again. The
# device alice just let in is asked through each server alone: the same key, told where one server is.
spare() {
  (cd "$root/server" && exec mix network "$folder" spare --address "$address") >"$folder/spare.log" 2>&1 &
  spare=$!
  started+=("$spare")
  disown "$spare"
  grep -q -m1 -E "A server is running|isn't one of" <(tail -f --pid="$spare" -n +1 "$folder/spare.log")
}
spare
ONE_FOR=network may device laptop $(grep -o 'add_server --key [0-9a-f]*' "$folder/spare.log")
spare
cp "$folder/only-server/key" "$folder/only-spare/key"
through() { ONE_HOME=$folder/only-$1 "$one" "${@:2}"; }
may through server ping ping
may through spare ping ping
may device laptop sign-in remove_device --device "$direct"
refused through server ping ping
refused through spare ping ping
kill -9 "$spare"

say "Without the plugin that decides who may, the network is its first owner's alone"
restart --without relations
echo "  the server is killed and started again with --without relations, and says: $(grep -o "relations is removed.*" "$folder/server.log")"
refused device bobs-phone tally list
may device laptop tally list
ONE_FOR=network device laptop deploy --name relations --files-from "$root/plugins/relations" >/dev/null
echo "  alice deploys it again from her laptop, and bob has his own back:"
may device bobs-phone tally list

say "Without the plugin that signs people in, a network answers nobody"
restart --without sign-in
echo "  the server is killed and started again with --without sign-in, and says: $(grep -o "sign-in is removed.*" "$folder/server.log")"
refused device laptop sign-in devices
refused device bobs-phone sign-in devices
refused stranger sign-in devices
