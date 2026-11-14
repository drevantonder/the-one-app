#!/usr/bin/env bash
# Shows a network's address working, on this machine alone: nothing is deployed and no account is used. The Workers
# run in `wrangler dev`, the bucket is a local S3 store (rustfs), and the address, a server and a client each get a
# network of their own (namespaces), wired together only as each step says. With the repo's tools:
#
#     mise exec -- address/proof/proof.sh
#
# It needs Linux with unprivileged user namespaces, and `socat`, which stands on the wire to the address and writes
# down what passes.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
address=$(dirname "$here")
root=$(dirname "$address")

if [[ ${1-} != inside ]]; then
  (cd "$root" && cargo build --release)
  (cd "$root/server" && mix deps.get && mix compile)
  (cd "$address" && bun install --frozen-lockfile)
  # The rest runs as root of namespaces of its own, and everything it starts ends with it.
  exec unshare --user --map-root-user --net --mount --pid --fork --kill-child --mount-proc "$0" inside
fi

work=$(mktemp -d)
trap 'rm -f "$address"/signer/.dev.vars "$address"/way-in/.dev.vars; echo; echo "The logs are in $work"' EXIT
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
at() { local namespace=$1; shift; nsenter --net=/run/netns/"$namespace" "$@"; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
# Waits for a line in a log that something started is still writing.
wait_for() { timeout 180 grep -q -m1 -E "$1" <(tail -f -n +1 "$2"); }
# The bytes the wire to the address has carried so far.
carried() { wc -c <"$work/wire"; }
# How often the relay has been woken by something to pass on, as `wrangler dev` counts what its Workers do.
woken() {
  at address curl -s -X POST http://127.0.0.1:8787/cdn-cgi/local/explorer/api/local/observability/query \
    -H 'content-type: application/json' -d "{\"sql\":\"SELECT count(*) FROM spans WHERE name = 'hibernatableWebSocket'\"}" |
    sed -E 's/.*"rows":\[\[([0-9]+).*/\1/'
}

# Three networks. The address is 10.0.0.1 to the other two, and passes nothing between them.
mount -t tmpfs tmpfs /run
for namespace in address server client; do
  ip netns add $namespace
  at $namespace ip link set lo up
done
at address ip addr add 10.0.0.1/32 dev lo
at address sh -c 'echo 0 > /proc/sys/net/ipv4/ip_forward'
# wire <a> <b> <net>: a wire between two namespaces, each end named for the other, .1 at a and .2 at b.
wire() {
  ip link add "$2" netns "$1" type veth peer name "$1" netns "$2"
  at "$1" ip addr add "$3.1/24" dev "$2" && at "$1" ip link set dev "$2" up
  at "$2" ip addr add "$3.2/24" dev "$1" && at "$2" ip link set dev "$1" up
}
wire address server 10.1.0 && at server ip route add default via 10.1.0.1
wire address client 10.2.0 && at client ip route add default via 10.2.0.1

# The bucket, with credentials made up for this run. Only the signer is given them.
access=$(random 8) secret=$(random 24)
mkdir "$work/bucket"
RUSTFS_ACCESS_KEY=$access RUSTFS_SECRET_KEY=$secret at address rustfs server --address 10.0.0.1:9000 "$work/bucket" >"$work/bucket.log" 2>&1 &
at address curl -s -o /dev/null --retry 60 --retry-all-errors --retry-delay 1 http://10.0.0.1:9000/
(cd "$address" && ACCESS=$access SECRET=$secret at address bun -e '
  import { AwsClient } from "aws4fetch";
  const store = new AwsClient({ accessKeyId: process.env.ACCESS, secretAccessKey: process.env.SECRET, service: "s3", region: "auto" });
  const made = await store.fetch("http://10.0.0.1:9000/network", { method: "PUT" });
  if (!made.ok) throw new Error(`the store answered ${made.status}`);')
printf 'BUCKET=http://10.0.0.1:9000/network\nACCESS_KEY_ID=%s\nSECRET_ACCESS_KEY=%s\n' "$access" "$secret" >"$address/signer/.dev.vars"

# Stops what was started in a group of its own, all of it.
stop() {
  [[ -z $1 ]] && return
  kill -- -"$1" 2>/dev/null
  wait "$1" 2>/dev/null || true
}
stamped() { while IFS= read -r line; do printf '%(%T)T %s\n' -1 "$line"; done; }

# The address: the five Workers, started again each time this is called.
start_address() {
  stop "${address_pid-}"
  (cd "$address" && WRANGLER_SEND_METRICS=false exec setsid nsenter --net=/run/netns/address \
    bun run dev --ip 0.0.0.0 --port 8787 --persist-to "$work/address") > >(stamped >"$work/address.log") 2>&1 &
  address_pid=$!
  at address curl -s -o /dev/null --retry 60 --retry-all-errors --retry-delay 1 http://10.0.0.1:8787/ping
}
# Servers and clients reach the address through a wire that writes down what it carries.
at address socat -v TCP-LISTEN:8788,bind=10.0.0.1,fork,reuseaddr,nodelay TCP:10.0.0.1:8787,nodelay 2>"$work/wire" &
at_address=http://10.0.0.1:8788

start_server() {
  stop "${server_pid-}"
  (cd "$root/server" && exec setsid nsenter --net=/run/netns/server mix network "$work/network" --address $at_address) >"$work/server.log" 2>&1 &
  server_pid=$!
  wait_for "A server is running" "$work/server.log"
}
one() { ONE_HOME=$work/network/client at client "$root/target/release/one" "$@"; }
mcp() { at client curl -s -m 20 -w ' (HTTP %{http_code})\n' -X POST "$1/mcp" -H 'content-type: application/json' -d "$2"; }
timed() { (cd "$address" && ONE_HOME=$work/network/client at client bun proof/calls.js "$root/target/release/one" "$@"); }
list='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tracker_list","arguments":{}}}'

say "The address serves the web client's files, and the way in for outsiders is shut without the development flag"
start_address
at client curl -s http://10.0.0.1:8787/ | grep '<p>'
mcp http://10.0.0.1:8787 "$list"

echo 'DEVELOPMENT_WITHOUT_SIGN_IN=true' >"$address/way-in/.dev.vars"
start_address

say "A server and a client that each reach the address, and not each other"
start_server
at client ping -c1 -W1 10.1.0.2 >/dev/null && echo "the client reaches the server" || echo "the client can't reach the server at 10.1.0.2"
echo "the server's settings name no bucket credentials, only the address: $(grep -c . "$address/signer/.dev.vars") lines of them are the signer's alone"
one deploy --name tracker --files-from "$root/plugins/tracker"

say "A tool call through the relay, which can't read it"
note=relayed-$(random 8)
before=$(carried) woke=$(woken)
one tracker add --tracker water --amount 2 --note "$note"
echo "the relay was woken $(($(woken) - woke)) times to pass that on"
echo "the note in the call ($note) is in $(tail -c +$((before + 1)) "$work/wire" | grep -a -c "$note" || true) lines of the $(($(carried) - before)) bytes the wire to the address carried meanwhile"
one tracker list --tracker water

say "A machine on no list"
(cd "$address" && at client bun proof/stranger.js $at_address)
mkdir "$work/stranger"
cp "$work/network/client/servers" "$work/stranger/servers"
printf '{"address":"%s","pass":"%s"}' $at_address "$(printf '{"list":"users/anyone/"}' | base64 -w0)" >"$work/stranger/network"
echo "iroh's own client with a pass no server signed, asking the server through the relay:"
ONE_HOME=$work/stranger at client "$root/target/release/one" tracker list 2>&1 || true
echo "a server with a key of its own, starting on this network:"
(cd "$root/server" && at server mix network "$work/stranger-server" --address $at_address 2>&1 | grep -m1 -o 'The signer.*' || true)

say "An outsider's call over the web reaches a tool"
note=outside-$(random 8)
before=$(carried)
mcp $at_address '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tracker_add","arguments":{"tracker":"water","note":"'"$note"'"}}}'
echo "this one the address reads, to pass it on: its note ($note) is in $(tail -c +$((before + 1)) "$work/wire" | grep -a -c "$note" || true) lines of what the wire carried"
mcp $at_address "$list"

say "A server with no bucket credentials writes and reads through signed access"
(cd "$root/server" && at server mix run --no-start -e "
  Application.put_env(:one_core, :home, \"$work/network/server\")
  Application.put_env(:one_core, :bucket, {OneCore.Bucket.S3, \"$at_address\"})
  Code.eval_file(\"$here/bucket.exs\")")

say "The relay sleeps with its connections open"
wait_for '"woke","connections":[1-9]' "$work/address.log"
grep -E 'relay.*woke' "$work/address.log" | tail -3
echo "and a call still gets through them:"
one tracker list

say "What a minute of nothing asks of the relay"
# The minute is what's measured.
woke=$(woken)
sleep 60
echo "with the server connected and nobody calling, it was woken $(($(woken) - woke)) times"
woke=$(woken)
(echo '{"jsonrpc":"2.0","id":1,"method":"ping"}' && sleep 60) | one mcp >/dev/null
echo "with a client's connection to the server held open through it too, $(($(woken) - woke)) times"

# `ping` is a call that only goes there and back, and `tracker_list` a tool's, which reads the bucket twice.
timings() {
  local woke=$(woken)
  timed ping
  timed tools/call tracker_list
  echo "the relay was woken $(($(woken) - woke)) times by those 102 calls"
}
far() {
  for end in "address server" "server address" "address client" "client address"; do
    set -- $end
    at "$1" tc qdisc "$far" dev "$2" root netem delay 10ms
  done
}

say "Timings through the relay, with nothing between the three but this machine"
timings
say "Timings through the relay, with the address 10 ms from each of the two"
far=add far
timings

say "The same pair with a wire between them: the call goes direct, with neither told"
wire server client 10.3.0
# Each end notices its new wire, and tells the other through the relay where else to try.
sleep 10
timings
say "Timings direct, with nothing between the three but this machine"
far=del far
timings

say "What the address was asked, by path"
grep -o '\(GET\|POST\) /[a-z/]*' "$work/address.log" | sort | uniq -c
