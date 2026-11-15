#!/usr/bin/env bash
# Shows a network's edge working, on this machine alone: nothing is deployed and no account is used. The Workers
# run in `wrangler dev`, the bucket is a local S3 store (rustfs), and the edge, a server and a client each get a
# network of their own (namespaces), wired together only as each step says. With the repo's tools:
#
#     mise exec -- core/edge/test/edge.sh
#
# It needs Linux with unprivileged user namespaces, `socat`, which stands on the wire to the edge and writes
# down what passes, and `xvfb-run`, `dbus-run-session` and `xdotool`, for the app on a screen of its own.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
edge=$(dirname "$here")
root=$(cd "$edge/../.." && pwd)

if [[ ${1-} != inside ]]; then
  (cd "$root" && cargo build --release)
  (cd "$root/ui" && bun install --frozen-lockfile && bun build.js)
  (cd "$root/core/server" && mix deps.get && mix compile)
  (cd "$edge" && bun install --frozen-lockfile)
  # The rest runs as root of namespaces of its own, and everything it starts ends with it.
  exec unshare --user --map-root-user --net --mount --pid --fork --kill-child --mount-proc "$0" inside
fi

# Everything this makes is in this folder, which it leaves behind for its logs.
work=$(mktemp -d)
trap 'echo; echo "The logs are in $work"' EXIT
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
at() { local namespace=$1; shift; nsenter --net=/run/netns/"$namespace" "$@"; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
# Waits for a line in a log that something started is still writing.
wait_for() { timeout 180 grep -q -m1 -E "$1" <(tail -F -n +1 "$2"); }
# The bytes the wire to the edge has carried so far.
carried() { wc -c <"$work/wire"; }
# How often the relay has been woken by something to pass on, as `wrangler dev` counts what its Workers do.
woken() {
  at edge curl -s -X POST http://127.0.0.1:8787/cdn-cgi/local/explorer/api/local/observability/query \
    -H 'content-type: application/json' -d "{\"sql\":\"SELECT count(*) FROM spans WHERE name = 'hibernatableWebSocket'\"}" |
    sed -E 's/.*"rows":\[\[([0-9]+).*/\1/'
}

# Three networks. The edge is 10.0.0.1 to the other two, and passes nothing between them.
mount -t tmpfs tmpfs /run
for namespace in edge server client; do
  ip netns add $namespace
  at $namespace ip link set lo up
done
at edge ip addr add 10.0.0.1/32 dev lo
at edge sh -c 'echo 0 > /proc/sys/net/ipv4/ip_forward'
# wire <a> <b> <net>: a wire between two namespaces, each end named for the other, .1 at a and .2 at b.
wire() {
  ip link add "$2" netns "$1" type veth peer name "$1" netns "$2"
  at "$1" ip addr add "$3.1/24" dev "$2" && at "$1" ip link set dev "$2" up
  at "$2" ip addr add "$3.2/24" dev "$1" && at "$2" ip link set dev "$1" up
}
wire edge server 10.1.0 && at server ip route add default via 10.1.0.1
wire edge client 10.2.0 && at client ip route add default via 10.2.0.1

# The bucket, with credentials made up for this run. Only the signer is given them, with the secret the network's
# first server shows.
access=$(random 8) secret=$(random 24) first=$(random 16)
mkdir "$work/bucket"
RUSTFS_ACCESS_KEY=$access RUSTFS_SECRET_KEY=$secret at edge rustfs server --address 10.0.0.1:9000 "$work/bucket" >"$work/bucket.log" 2>&1 &
at edge curl -s -o /dev/null --retry 60 --retry-all-errors --retry-delay 1 http://10.0.0.1:9000/
(cd "$edge" && ACCESS=$access SECRET=$secret at edge bun -e '
  import { AwsClient } from "aws4fetch";
  const store = new AwsClient({ accessKeyId: process.env.ACCESS, secretAccessKey: process.env.SECRET, service: "s3", region: "auto" });
  const made = await store.fetch("http://10.0.0.1:9000/network", { method: "PUT" });
  if (!made.ok) throw new Error(`the store answered ${made.status}`);')

stamped() { while IFS= read -r line; do printf '%(%T)T %s\n' -1 "$line"; done; }

# The edge: its Workers. The signer's secrets are in the environment, which only `wrangler dev` reads them from.
(cd "$edge" && BUCKET=http://10.0.0.1:9000/network ACCESS_KEY_ID=$access SECRET_ACCESS_KEY=$secret FIRST_SERVER=$first \
  WRANGLER_SEND_METRICS=false exec setsid nsenter --net=/run/netns/edge \
  bun run dev --ip 0.0.0.0 --port 8787 --persist-to "$work/edge") > >(stamped >"$work/edge.log") 2>&1 &
at edge curl -s -o /dev/null --retry 60 --retry-all-errors --retry-delay 1 http://10.0.0.1:8787/ping
# Servers and clients reach the edge through a wire that writes down what it carries.
at edge socat -v TCP-LISTEN:8788,bind=10.0.0.1,fork,reuseaddr,nodelay TCP:10.0.0.1:8787,nodelay 2>"$work/wire" &
at_address=http://10.0.0.1:8788

# start_server <name> [options]: a server of the network whose home is the folder's <name>. Returns once it's
# running, or has said it isn't one of the network's. The network is one being built, where nobody signs in: a
# passkey is for a name, which an address that is a number has none of, so every client here asks as its first owner.
export ONE_AS=owner
start_server() {
  local name=$1
  shift
  (cd "$root/core/server" && exec setsid nsenter --net=/run/netns/server mix network "$work/network" "$name" "$@" --development --owner owner --address $at_address) >"$work/$name.log" 2>&1 &
  wait_for "A server is running|isn't one of the network's" "$work/$name.log"
}
# The client that calls: the network's first, until the app is open as that one, and then a second.
calls=$work/network/client
one() { ONE_HOME=$calls at client "$root/target/release/one" "$@"; }
timed() { (cd "$edge" && ONE_HOME=$calls at client bun test/calls.js "$root/target/release/one" "$@"); }

say "The edge serves the web client's files"
at client curl -s http://10.0.0.1:8787/ | grep '<p>'

say "A server and a client that each reach the edge, and not each other"
echo "a server that doesn't show the secret the edge was started with: $(cd "$root/core/server" && at server mix network "$work/unknown" --address $at_address 2>&1 | grep -m1 -o "This server isn't one of the network's yet" || true)"
start_server server --first "$first"
at client ping -c1 -W1 10.1.0.2 >/dev/null && echo "the client reaches the server" || echo "the client can't reach the server at 10.1.0.2"
echo "the server is told the address and the first server's secret, and nothing of the bucket's credentials. Its client is told the address and no server: its home holds $(ls "$work/network/client" | tr '\n' ' ')"

ONE_FOR=network one deploy --name tally --files-from "$root/core/server/test/plugins/tally" >/dev/null

say "A tool call through the relay, which can't read it"
note=relayed-$(random 8)
before=$(carried) woke=$(woken)
one tally add --note "$note"
echo "the relay was woken $(($(woken) - woke)) times to token that on"
echo "the note in the call ($note) is in $(tail -c +$((before + 1)) "$work/wire" | grep -a -c "$note" || true) lines of the $(($(carried) - before)) bytes the wire to the edge carried meanwhile"
one tally list

say "A machine on no list"
# The list a real client's token names, which the stranger says it's on too.
on_list=$(bun -e 'const { token } = await Bun.file(process.argv[1]).json(); console.log(JSON.parse(atob(token.replace(/-/g, "+").replace(/_/g, "/"))).list)' "$work/network/client/network")
(cd "$edge" && at client bun test/stranger.js $at_address "$on_list")
mkdir "$work/stranger"
printf '{"address":"%s","token":"%s"}' $at_address "$(printf '{"list":"%s"}' "$on_list" | base64 -w0)" >"$work/stranger/network"
echo "a client with a token no server signed, asking which servers to reach:"
ONE_HOME=$work/stranger at client "$root/target/release/one" tally list 2>&1 || true
echo "a server with a key of its own, starting on this network with the first server's secret: $(cd "$root/core/server" && at server mix network "$work/stranger-server" --address $at_address --first "$first" 2>&1 | grep -m1 -o "This server isn't one of the network's yet" || true)"

say "A spare is let in by one step, as on a folder: started, it says how, and started again, it acts"
start_server spare
let_in=$(grep -o 'add_server --key [0-9a-f]*' "$work/spare.log")
ONE_FOR=network one $let_in
start_server spare
one services | grep -o '"servers":\[[^]]*\]' | tail -1

say "A server with no bucket credentials writes and reads through signed access"
(cd "$root/core/server" && at server mix run --no-start -e "
  Application.put_env(:one_core, :home, \"$work/network/server\")
  Application.put_env(:one_core, :bucket, {OneCore.Bucket.S3, \"$at_address\"})
  Code.eval_file(\"$here/bucket.exs\")")

say "The relay sleeps with its connections open"
# Nothing an idle network says reaches it, so after a quiet while it's asleep, and a call wakes it.
sleep 20
echo "a call gets through them:"
one tally list
wait_for '"woke","connections":[1-9]' "$work/edge.log"
grep -E 'relay.*woke' "$work/edge.log" | tail -3

say "What an hour asks of the relay, by a minute of each"
# The minute is what's measured.
hourly() { local woke=$(woken); sleep 60; echo "$((($(woken) - woke) * 60)) times an hour"; }
echo "with two servers connected and nobody calling, it's woken $(hourly)"
woke=$(woken)
(echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' && sleep 60) | one mcp >/dev/null
echo "by a client that asks once and stays open for a minute, as an assistant's does: $(($(woken) - woke)) times in all"

# The app, on a screen of its own in the client's network. It says when it has drawn, and when a server told it
# something changed. A window nothing shows is what its page takes for hidden.
: >"$work/app.log"
ONE_HOME=$work/network/client at client dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
  echo "DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY" >"$1/screen"
  exec "$0" 2> >(while IFS= read -r line; do echo "${EPOCHREALTIME/[.,]/} $line"; done >"$1/app.log")
' "$root/target/release/one" "$work" >/dev/null 2>&1 &
wait_for "home opened" "$work/app.log"
window() { at client env $(cat "$work/screen") xdotool search --name "The One App" "$@" >/dev/null; }
# A second client of the network, which a server gives a token as it gave the first: two programs can't be one
# client at the relay, and the app is the first.
(cd "$root/core/server" && at server mix run --no-start -e "
  Application.put_env(:one_core, :home, \"$work/network/server\")
  {:ok, key} = OneCore.Iroh.key(\"$work/network/elsewhere/key\")
  token = OneCore.Edge.token(key)
  File.write!(\"$work/network/elsewhere/network\", JSON.encode!(%{address: \"$at_address\", token: token}))")
calls=$work/network/elsewhere
sleep 5
echo "with the app open and on screen, and nothing changing: $(hourly)"

say "A change made elsewhere reaches the app on screen"
# told <command...>: calls a tool as the second client, and says how long after that began the app was told.
told() {
  local woke=$(woken) lines=$(wc -l <"$work/app.log") began=${EPOCHREALTIME/[.,]/}
  one "$@" >/dev/null
  local done=${EPOCHREALTIME/[.,]/}
  if timeout 5 grep -q -m1 "said something changed" <(tail -n +$((lines + 1)) -f "$work/app.log"); then
    local told=$(tail -n +$((lines + 1)) "$work/app.log" | grep -m1 "said something changed" | cut -d' ' -f1)
    sleep 3
    echo "the call took $(((done - began) / 1000)) ms, the app was told $(((told - began) / 1000)) ms after it began, and the relay was woken $(($(woken) - woke)) times for all of it"
  else
    echo "the call took $(((done - began) / 1000)) ms, the app wasn't told, and the relay was woken $(($(woken) - woke)) times for all of it"
  fi
}
echo "through the relay, with nothing between the three but this machine:"
told tally add --note water
sleep 5
told tally add --note water

say "An app that's hidden is told nothing, and asks nothing of the relay"
window windowunmap && sleep 5
told tally add --note water
echo "with the app open and hidden, and nothing changing: $(hourly)"
window windowmap && sleep 5
echo "on screen again:"
told tally add --note water

# `ping` is a call that only goes there and back, and `tally_list` a tool's, which reads the bucket twice.
timings() {
  local woke=$(woken)
  timed ping
  timed tools/call tally_list
  echo "the relay was woken $(($(woken) - woke)) times by those 102 calls"
}
far() {
  for end in "edge server" "server edge" "edge client" "client edge"; do
    set -- $end
    at "$1" tc qdisc "$far" dev "$2" root netem delay 10ms
  done
}

say "Timings through the relay, with nothing between the three but this machine"
timings
say "Timings through the relay, with the edge 10 ms from each of the two"
far=add far
timings
sleep 5
told tally add --note water

say "The same pair with a wire between them: the call goes direct, with neither told"
wire server client 10.3.0
# Each end notices its new wire, and tells the other through the relay where else to try.
sleep 10
timings
say "Timings direct, with nothing between the three but this machine"
far=del far
timings

say "What the edge was asked, by path"
grep -o '\(GET\|POST\) /[a-z/]*' "$work/edge.log" | sort | uniq -c
