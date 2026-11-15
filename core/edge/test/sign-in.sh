#!/usr/bin/env bash
# Tests signing in, on a whole network on this machine that people sign in to (`tooling/network <folder> --edge --owner
# alice`), with nothing deployed. People join with a passkey, in a real browser each, and each sees only their own. A
# new device shows a code and a link that carries it, and is nobody's until its person opens that where they sign in and
# says the code they're shown matches: with nothing more where they're signed in already, and after their passkey where
# they aren't. Saying stop refuses it, and the device is told. A link a stranger sends shows the stranger's code, and
# lets in nothing unless it's said to match. A code that has ended, one someone else opened first, too many wrong ones,
# through a browser or a tool, and a key asking on a code that isn't its own are refused. One that's removed is refused
# the next thing it asks, by every server, and is let in again as someone else's at once. A stranger who knows a
# server's key and where it is, and dials it with a key of their own, is refused. A first owner who has joined is given
# no link to join by again. A server that's off is known to be, and isn't waited for. An assistant somewhere else is let
# in at /mcp as MCP's authorization says, acts as whoever let it in, and is one of their devices. A network without the
# plugin that decides who may is its first owner's alone, and one without the plugin that signs people in answers
# nobody. It says what a call costs once a device is someone's. A device or assistant of a person's lets in their next
# by its code, with no browser; given by someone else's, it's theirs; and only a server acting for that person says so.
# Each person goes by the name they gave as they joined, which another user is told.
#
#     mise exec -- core/edge/test/sign-in.sh
#
# It needs `xvfb-run`, `dbus-run-session` and `xdotool`, for the app on a screen of its own.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each server started here.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
"$root/tooling/network" "$folder" --edge --owner alice >"$folder/network.log" 2>&1 &
network=$!
started=("$network")
# The people leave when nothing more is asked of them, and the rest is stopped.
trap '[[ -z ${to_people-} ]] || exec {to_people}>&-; for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -F --pid=$network -n +1 "$folder/network.log" 2>/dev/null) || { cat "$folder/network.log" "$folder/server.log"; exit 1; }
one=$root/target/release/one
address=$(grep -o -m1 'http://localhost:[0-9]*' "$folder/network.log")

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
# What's printed names no folder, key or link of this run.
plain() { sed -E -e "s|$folder|<folder>|g" -e "s|$root/||g" -e "s|$address|<address>|g" -e 's/[0-9a-f]{64}/<key>/g' -e 's/(invite=)[0-9a-f]+/\1…/' -e 's/"added":"[^"]*"/"added":"…"/g' -e "s/$(hostname)/<this machine>/g"; }

# People, each in a browser of their own with a passkey of their own: `browser alice <address>` has alice open it
# and answer the one question the page asks.
mkfifo "$folder/to-people" "$folder/from-people"
(cd "$root/core/edge" && exec bun test/people.js <"$folder/to-people" >"$folder/from-people") &
exec {to_people}>"$folder/to-people" {from_people}<"$folder/from-people"
browser() {
  echo "$*" >&"$to_people"
  read -r said <&"$from_people"
  echo "  $1 ${3:+(${3}) }opens $(plain <<<"$2")"
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
# A device that isn't in asks something, and says its code and its link, and waits for its person's answer:
# `asks laptop sign-in devices` starts it, and `link` and `code` are what it showed. `answered laptop` waits for it
# to end, and shows what it said after.
asks() {
  device "$@" >"$folder/$1.said" 2>&1 &
  asking=$!
  grep -q -m1 "user_code=" <(tail -F --pid="$asking" -n +1 "$folder/$1.said" 2>/dev/null)
  link=$(grep -o -m1 'http://[^ ]*user_code=[A-Z0-9]*' "$folder/$1.said")
  code=$(grep -o -m1 'shows [A-Z0-9-]*' "$folder/$1.said" | cut -d' ' -f2)
  echo "  $1, which isn't in, is asked: one ${*:2}"
  echo "           -> $(plain <"$folder/$1.said" | tr '\n' ' ')"
}
answered() {
  wait "$asking" && echo "  $1 is let in, and answers:" || echo "  $1 isn't let in, and says:"
  echo "           -> $(tail -n +3 "$folder/$1.said" | plain | tr '\n' ' ')"
}
# Whose key a device's home holds, as servers name it.
key_of() { (cd "$root/core/server" && mix run --no-start -e 'IO.puts(elem(OneCore.Iroh.key(hd(System.argv())), 1))' "$1/key" | tail -1); }
# Changes what the edge keeps of a device's code: `recode <code> <change>`.
recode() {
  (cd "$root/core/edge" && bun x wrangler d1 execute network-sign-in --local --persist-to "$folder/edge" -c parts/sign-in/wrangler.jsonc \
    --command "update deviceCode set $2 where userCode = '${1//-/}'") >/dev/null
}
# Ends a device's code, as half an hour would.
ends() { recode "$1" "expiresAt = '2000-01-01T00:00:00.000Z'"; }
# Stops the network's server, and starts it again as it was started with whatever else is given.
restart() {
  local server
  server=$(pgrep -f "mix network $folder server " | head -1)
  kill -9 "$server"
  tail --pid="$server" -f /dev/null
  (cd "$root/core/server" && exec mix network "$folder" server --address "$address" "$@") >"$folder/server.log" 2>&1 &
  server=$!
  started+=("$server")
  # It's stopped by its number from here on, and the shell says nothing of it.
  disown "$server"
  grep -q -m1 "A server is running" <(tail -F --pid="$server" -n +1 "$folder/server.log")
}

say "Nobody is anybody yet: the network's first owner joins with a passkey, by the link its first server gave"
joins=$(grep -o -m1 "http://[^ ]*invite=[0-9a-f]*" "$folder/server.log")
browser alice "$joins"
browser mallory "$joins"

say "A new device shows a code, and its person lets it in where they're signed in, by saying the code matches"
asks laptop sign-in devices
browser alice "$link" matches
answered laptop

say "Someone else joins by an invitation, and each sees only their own"
invited=$(device laptop sign-in invite --name bob)
echo "  alice invites bob from her laptop: $(plain <<<"$invited")"
browser bob "$(grep -o "http://[^ ]*" <<<"$invited")"
asks bobs-phone sign-in devices
phones=$link phone=$code phone_asking=$asking

say "A stranger sends bob a link made for the stranger's own device: it shows the stranger's code, not his phone's, and lets nothing in unless he says it matches"
asks strangers-laptop sign-in devices
echo "  bob's phone shows $phone, and the stranger's laptop $code"
browser bob "$link" look
kill -0 "$asking" && echo "  the stranger's laptop, having been shown to bob, still waits, nobody's"
browser bob "$link" stop
answered strangers-laptop
link=$phones asking=$phone_asking
browser bob "$link" matches
answered bobs-phone

say "Each has a plugin of their own to keep notes with, deployed from their own device"
may device laptop deploy --name tally --files-from "$root/core/server/test/plugins/tally"
may device laptop tally add --note "Alice's"
refused device bobs-phone tally list
may device bobs-phone deploy --name tally --files-from "$root/core/server/test/plugins/tally"
may device bobs-phone tally add --note "Bob's"
may device bobs-phone tally list
may device laptop tally list
refused device bobs-phone sign-in invite --name carol
echo "  what each is told of their own devices and passkeys:"
may device laptop sign-in devices
may device bobs-phone sign-in devices

say "Each goes by the name they gave as they joined, which another user of the network is told"
may device laptop sign-in people --users '["users/alice", "users/bob"]'
may device bobs-phone sign-in name --called "Robert"
may device laptop sign-in people --users '["users/bob"]'
may device bobs-phone sign-in people

say "Where its person isn't signed in, they answer their passkey first, and then see the code"
browser alice signs-out
asks desktop sign-in devices
browser alice "$link" matches
answered desktop

say "A device's token to the relay lasts an hour, and the next is asked of a server before it ends"
# How many minutes a device's token has left, by when it says it ends. Ending it here is saying it ended earlier,
# which the client believes of its own token.
left() { bun -e 'const { token } = await Bun.file(process.argv[1]).json(); console.log(Math.round((JSON.parse(Buffer.from(token, "base64url")).until - Date.now()) / 60000))' "$folder/$1/network"; }
echo "  desktop's ends in $(left desktop) minutes"
bun -e 'const file = Bun.file(process.argv[1]), told = await file.json(), token = JSON.parse(Buffer.from(told.token, "base64url"))
  await Bun.write(file, JSON.stringify({ ...told, token: Buffer.from(JSON.stringify({ ...token, until: Date.now() })).toString("base64url") }))' "$folder/desktop/network"
may device desktop sign-in devices
echo "  once it has ended, desktop is given the next, which ends in $(left desktop) minutes"

say "Someone else who's signed in can't answer a code that was opened first by another"
asks shared-tv sign-in devices
browser alice "$link" look
browser bob "$link" force

say "A link whose code has ended is refused, and so is the device that showed it"
# The TV is asleep while its code ends, so that what the link says is seen before the TV asks again.
kill -STOP -- "-$asking"
ends "$code"
echo "  the TV is asleep, and its code ends"
browser bob "$link" matches
kill -CONT -- "-$asking"
answered shared-tv

say "Too many wrong codes are refused, as Better Auth's device authorization limits them"
invited=$(device laptop sign-in invite --name carol)
browser carol "$(grep -o "http://[^ ]*" <<<"$invited")"
for guess in 1 2 3 4 5 6; do browser carol "$address/sign-in/?user_code=WRONG00$guess"; done

say "A key is let in only on a code it asked for itself, and that its person answered"
(cd "$root/core/edge" && bun test/keys.js "$address") | sed 's/^/  /'

say "The app starts on the same sign-in page with its code, and shows its person's home screen once they've said it matches"
# A web view has no passkeys, so the page's button there hands its link to the device's own browser. What stands
# in for that browser here writes down what it was asked to open, and alice opens that in hers.
mkdir "$folder/tablet" "$folder/browser" && printf '{"address":"%s"}' "$address" >"$folder/tablet/network"
printf '#!/bin/sh\necho "$1" >"%s"\n' "$folder/opened" >"$folder/browser/xdg-open" && chmod +x "$folder/browser/xdg-open"
: >"$folder/screen" >"$folder/app.log" >"$folder/opened"
before=$(wc -l <"$folder/edge.log")
PATH=$folder/browser:$PATH ONE_HOME=$folder/tablet dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
  echo "DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY" >"$1/screen"
  exec "$0" 2>"$1/app.log"' "$one" "$folder" >/dev/null 2>&1 &
app=$!
started+=("$app")
disown "$app"
grep -q -m1 DISPLAY <(tail -F --pid="$app" -n +1 "$folder/screen" 2>/dev/null)
window() { env $(cat "$folder/screen") xdotool search --sync --onlyvisible --name "The One App" "$@" >/dev/null; }
window
echo "  the app's window is open on a device that isn't in, and its home screen has opened $(grep -c "home opened" "$folder/app.log") times"
# The page has drawn once it asks for the mark it shows, and its one button is its only link, which Tab reaches first.
grep -q -m1 "mark.png" <(tail -f -n +$((before + 1)) "$folder/edge.log")
window windowfocus --sync key --delay 300 Tab Return
grep -q -m1 "sign-in" <(tail -F -n +1 "$folder/opened" 2>/dev/null)
echo "  its button is pressed, and it asks the device's browser to open $(plain <"$folder/opened")"
browser alice "$(cat "$folder/opened")" matches
timeout 30 grep -q -m1 "home opened" <(tail -f -n +1 "$folder/app.log")
echo "  without being touched again, the window says: $(grep -o -m1 "home opened" "$folder/app.log")"
kill -- "-$app"

say "A device that's removed is refused the next thing it asks, and keeps nothing of its person's"
kept() { { find "$folder/$1/cache" "$folder/$1/files" -type f 2>/dev/null || true; } | wc -l; }
echo "  desktop keeps $(kept desktop) answers it was given for alice"
may device laptop sign-in remove_device --device "$(key_of "$folder/desktop")"
refused device desktop sign-in devices
[[ $(kept desktop) == 0 ]] || { echo "the removed device still keeps $(kept desktop) of what alice was answered" && exit 1; }
! grep -q '"token"' "$folder/desktop/network" || { echo "the removed device still keeps its token" && exit 1; }
echo "  it keeps none of what it was answered for alice, nor its token"
may device laptop sign-in devices

say "Let in as someone else's, it's theirs the moment they say its code matches"
invited=$(device laptop sign-in invite --name dave)
browser dave "$(grep -o "http://[^ ]*" <<<"$invited")"
asks desktop sign-in devices
browser dave "$link" matches
answered desktop
grep -q '"who":"users/dave"' "$folder/desktop/network" || { echo "desktop doesn't know it's dave's" && exit 1; }
echo "  without its token being renewed, desktop knows it's dave's, and its answer above is his devices"

say "A stranger who knows a server's key and where it is, and dials it with a key of their own, is refused"
echo "  the stranger knows $(plain <"$folder/only-server/servers" | cut -c1-90)…"
refused stranger sign-in devices
ONE_AS=alice refused stranger sign-in devices
ONE_FOR=network refused stranger deploy --name mine --files-from "$root/core/server/test/plugins/tally"

say "An assistant somewhere else is let in at /mcp as MCP's authorization says, by the person it then acts as"
echo "  asked with no token, the edge answers:"
curl -s -i -m 20 -X POST "$address/mcp" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' |
  grep -i -E "^HTTP|^www-authenticate|jsonrpc" | plain | sed 's/^/    /'
coproc ASSISTANT { cd "$root/core/edge" && exec bun test/assistant.js "$address" tally_list 2>&1; }
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

say "A device of a person's lets in their next by the code it shows, with a tool, and no browser opens it"
opened() { tail -n +$((before + 1)) "$folder/edge.log" | grep -c "GET /sign-in/auth/device?" || true; }
before=$(wc -l <"$folder/edge.log")
asks workstation sign-in devices
may device laptop sign-in let_in --code "$code"
answered workstation
echo "  the page that shows a code where someone signs in was opened $(opened) times for it"
refused device laptop sign-in let_in --code "$code"
echo "  once the device has learned its answer, its code is gone"

say "So does an assistant alice let in at /mcp"
asks build-box sign-in devices
echo "sign-in_let_in {\"code\":\"$code\"}" >&"${ASSISTANT[1]}"
read -r said <&"${ASSISTANT[0]}" && echo "  the assistant: $(plain <<<"$said")"
answered build-box
echo "  the page that shows a code where someone signs in was opened $(opened) times for both"

say "Someone else's device given the code makes the new device theirs, as saying it matches in their browser does"
asks kiosk sign-in devices
may device bobs-phone sign-in let_in --code "$code"
answered kiosk

say "A code that has ended, one that was stopped, one already answered, one someone else opened first, and what isn't a code are each refused with their own sentence"
# Each device sleeps while its code is asked about, as one would between the times it asks for its answer: once
# it has learned that answer, the edge forgets its code.
asks old-tablet sign-in devices
kill -STOP -- "-$asking"
ends "$code"
refused device laptop sign-in let_in --code "$code"
kill -CONT -- "-$asking"
answered old-tablet
asks spare-phone sign-in devices
browser alice "$link" stop
refused device laptop sign-in let_in --code "$code"
answered spare-phone
asks hall-tv sign-in devices
kill -STOP -- "-$asking"
may device laptop sign-in let_in --code "$code"
refused device laptop sign-in let_in --code "$code"
kill -CONT -- "-$asking"
answered hall-tv
asks guest-pc sign-in devices
kill -STOP -- "-$asking"
# Bob opening its link where he's signed in, which the browsers here have opened too often for Better Auth by now.
recode "$code" "userId = (select id from user where name = 'bob')"
echo "  bob opens its link where he's signed in"
refused device laptop sign-in let_in --code "$code"
ends "$code"
kill -CONT -- "-$asking"
answered guest-pc
refused device laptop sign-in let_in --code NOPE-1234

say "Only a server acting for a person tells the edge to let a device in as them"
acting() { (cd "$root/core/edge" && bun test/acting.js "$address" "$folder/server-key" "$@"); }
server_key=$(acting)
told() { echo "  $4: $(acting "$1" "$2" "$3" | plain)"; }
asks lab-pc sign-in devices
told unsigned users/alice "$code" "signing nothing, for alice"
told signed users/alice "$code" "signed by a key that's on no list, for alice"
may device bobs-phone add_server --key "$server_key"
told signed users/alice "$code" "signed by a server acting for bob, for alice"
told signed users/bob "$code" "signed by that server, for bob"
answered lab-pc
may device bobs-phone remove_server --key "$server_key"

say "Once alice removes the assistant, it's nobody's"
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
mkdir -p "$folder/ping/server" && mv "$folder/ping.ex" "$folder/ping/server/"
ONE_FOR=network device laptop deploy --name ping --files-from "$folder/ping" >/dev/null
direct=$(key_of "$folder/only-server")
mkdir "$folder/direct" && cp "$folder/only-server/key" "$folder/direct/" && printf '{"address":"%s"}' "$address" >"$folder/direct/network"
asks direct sign-in devices >/dev/null
device laptop sign-in let_in --code "$code" >/dev/null
answered direct >/dev/null
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
  (cd "$root/core/server" && exec mix network "$folder" spare --address "$address") >"$folder/spare.log" 2>&1 &
  spare=$!
  started+=("$spare")
  disown "$spare"
  grep -q -m1 -E "A server is running|isn't one of" <(tail -F --pid="$spare" -n +1 "$folder/spare.log" 2>/dev/null)
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

say "A server that's off is known to be, by the relay, so a call to it isn't waited for and says when it was last seen"
spare_key=$(key_of "$folder/spare")
began=$(now)
ONE_FOR=network refused device laptop remove_service --plugin sign-in --service none --server "$spare_key"
echo "  in $((($(now) - began) / 1000)) ms, where reaching a server takes up to 3000"
echo "  and alice's devices say which is on: $(device laptop sign-in devices | grep -o '"seen":"[^"]*"' | sed -E 's/"seen":"(on|[^"]*)"/\1/; s/^2.*/a time/' | sort | uniq -c | tr -s ' \n' ' ')"

say "Five codes no device shows, and the next is refused for a quarter of an hour, whatever it is"
for wrong in NOPE-1235 NOPE-1236 NOPE-1237 NOPE-1238; do device bobs-phone sign-in let_in --code "$wrong" >/dev/null 2>&1 || true; done
refused device bobs-phone sign-in let_in --code NOPE-1239
asks desk-pc sign-in devices
refused device bobs-phone sign-in let_in --code "$code"
may device laptop sign-in let_in --code "$code"
answered desk-pc

say "Its first owner has joined, so a server that starts again gives no link to join as them"
restart --owner alice
echo "  the server is killed and started again with --owner alice, and its log has $(grep -c "joins at" "$folder/server.log") links to join by"

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
