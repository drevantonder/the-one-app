#!/usr/bin/env bash
# Proves, with real servers on this machine and a folder as their bucket, that a database outlives the server
# writing it. Writes go through the tracker while servers are stopped with `kill -9`, frozen and started again, and
# after each of those every write that was answered must be there, once.
#
#     cargo build -p shell && mise exec -- server/test/takeover.sh
#
# ONE names the client's program, if it isn't target/debug/one.
set -euo pipefail
set -m # Each server is a job with a process group of its own, so all of it is stopped at once.

cd "$(dirname "$0")/.."
one=$(realpath "${ONE:-../target/debug/one}")
folder=$(mktemp -d)
bucket=$folder/bucket
answered=$folder/answered
declare -A group seen
names=$folder/names
touch "$answered"
trap 'for server in "${!group[@]}"; do stop "$server" >/dev/null; done; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }
median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.1f ms", all[int((NR + 1) / 2)] / 1000 }'; }

# What each server named has logged about the lease since this was last asked, with servers by their names here.
said() {
  for server; do
    tail -n +$((${seen[$server]:-0} + 1)) "$folder/$server.log" | grep "lease" |
      sed -f "$names" -e "s/^.*\[info\] [^:]*: /  $server: /" || true
    seen[$server]=$(wc -l <"$folder/$server.log")
  done
}

# Starts a server, and returns once it says it's running. The client `only-<server>` asks it and no other, and the
# client `client` asks every server, as `mix network` leaves it.
start() {
  mix network "$folder" "$1" >"$folder/$1.log" 2>&1 &
  group[$1]=$!
  grep -q -m1 "A server is running" <(tail -f --pid=$! -n +1 "$folder/$1.log")
  local key
  key=$(grep -o "running in [^ ]*: [0-9a-f]*" "$folder/$1.log" | grep -o "[0-9a-f]*$")
  echo "s/$key/$1/g" >>"$names"
  seen[$1]=0
  mkdir -p "$folder/only-$1"
  echo "[$(cat "$bucket/servers/$key/address")]" >"$folder/only-$1/servers"
  echo "server $1 is running"
}

stop() {
  kill -9 -- "-${group[$1]}"
  wait "${group[$1]}" 2>/dev/null || true
  unset "group[$1]"
  echo "server $1 is stopped, with kill -9"
}

# One write through a client, numbered so that it can be found afterwards. Says so if it wasn't answered.
number=0
attempt() {
  if ONE_HOME=$folder/$1 "$one" tracker add --tracker proof --note "write $2" >/dev/null 2>"$folder/refused-$2"; then
    echo "$2" >>"$answered"
  else
    echo "write $2 wasn't answered: $(head -c 120 "$folder/refused-$2")"
    return 1
  fi
}
write() {
  number=$((number + 1))
  attempt "$1" "$number"
}

# Writes over one connection, as an agent's MCP session or an app does, each timed into the file named.
stream() {
  coproc MCP { ONE_HOME=$folder/$1 "$one" mcp; }
  for _ in $(seq "$2"); do
    number=$((number + 1))
    local began answer
    began=$(now)
    echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tracker_add","arguments":' \
      "{\"tracker\":\"proof\",\"note\":\"write $number\"}}}" >&"${MCP[1]}"
    read -r answer <&"${MCP[0]}"
    echo $(($(now) - began)) >>"$3"
    [[ $answer == *'"result"'* && $answer != *isError* ]] && echo "$number" >>"$answered"
  done
  exec {MCP[1]}>&-
  wait "$MCP_PID"
}

# What's in the database, read through a client, against what was answered.
check() {
  ONE_HOME=$folder/$1 "$one" tracker list --tracker proof |
    grep -o '"write [0-9]*"' | grep -o '[0-9]*' | sort >"$folder/there"
  local lost twice
  lost=$(comm -23 <(sort "$answered") "$folder/there" | wc -l)
  twice=$(uniq -d "$folder/there" | wc -l)
  echo "$number written, $(wc -l <"$answered") answered, $(wc -l <"$folder/there") there: $lost lost, $twice twice"
  [ "$lost" = 0 ] && [ "$twice" = 0 ]
}

say "Two servers on one folder as the bucket"
start a
ONE_HOME=$folder/client "$one" deploy --name tracker --files-from ../plugins/tracker
write client
start b
echo "a ran the first write, so it holds the lease"

say "How long a write takes until it's answered, over one connection"
stream only-a 100 "$folder/at-holder"
echo "$(median "$folder/at-holder") at the server holding the lease"
stream only-b 100 "$folder/at-spare"
echo "$(median "$folder/at-spare") at the spare, which passes it on"

say "kill -9 of the server holding the lease, in the middle of a stream of writes"
for _ in $(seq 10); do write client; done
number=$((number + 1))
attempt client "$number" &
killed=$(now)
stop a
wait $! || true
until write client; do :; done
echo "the next answered write came $(since "$killed") after the kill"
for _ in $(seq 10); do write client; done
said b
check client

say "The first server comes back"
start a
for _ in $(seq 5); do write only-a; done
said a
check only-a

say "A server that still thinks it holds the lease: b is frozen, a takes over, b wakes and is written to"
kill -STOP -- "-${group[b]}"
frozen=$(now)
until write only-a; do :; done
echo "a answered a write $(since "$frozen") after b froze"
kill -CONT -- "-${group[b]}"
for _ in $(seq 5); do write only-b; done
said a b
check only-b

say "Two spares take over at once"
start c
write only-b
write only-c
said a b c >/dev/null
stop a
killed=$(now)
both=
until [ -n "$both" ]; do
  attempt only-b $((number + 1)) &
  first=$!
  attempt only-c $((number + 2)) &
  second=$!
  number=$((number + 2))
  both=yes
  wait $first || both=
  wait $second || both=
done
echo "both were answered $(since "$killed") after the kill"
said b c
check only-c

say "A fresh server starts from a snapshot and the log after it"
for _ in $(seq 40); do write only-b; done
start d
said b c >/dev/null
stop b
stop c
until write only-d; do :; done
echo "snapshots: $(ls "$bucket/users/anyone/own/tracker/snapshot" | sed 's/^0*//' | tr '\n' ' ')"
said d
check only-d
