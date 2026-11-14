# What the proofs here share: real servers of one network on this machine, with a folder as their bucket, started
# and stopped by name. A proof sources this, and everything it makes is in the folder made here, which goes when
# the proof ends.
#
# The client's program is the one `cargo build --release` builds.
set -euo pipefail
set -m # Each server is a job with a process group of its own, so all of it is stopped at once.

cd "$(dirname "${BASH_SOURCE[0]}")/.."
one=$(realpath ../target/release/one)
folder=$(mktemp -d)
bucket=$folder/bucket
declare -A group key seen
trap 'for server in "${!group[@]}"; do stop "$server" >/dev/null; done; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }

# Starts a server, and returns once it says it's running. A server after the first isn't one of the network's
# until a server that is lets it in, which is asked here as the server itself says to. The client `only-<server>`
# asks it and no other, and the client `client` asks every server, as `mix network` leaves it. `seen` is how much
# of the server's log a proof has read.
start() {
  mix network "$folder" "$1" >"$folder/$1.log" 2>&1 &
  group[$1]=$!
  if ! grep -q -m1 "A server is running" <(tail -f --pid=$! -n +1 "$folder/$1.log"); then
    local let_in
    let_in=$(grep -o 'add_server --key [0-9a-f]*' "$folder/$1.log") || { cat "$folder/$1.log"; exit 1; }
    ONE_HOME=$folder/client "$one" $let_in >/dev/null
    start "$1"
    return
  fi
  key[$1]=$(grep -o "running in [^ ]*: [0-9a-f]*" "$folder/$1.log" | grep -o "[0-9a-f]*$")
  seen[$1]=0
  mkdir -p "$folder/only-$1"
  echo "[$(cat "$bucket/servers/${key[$1]}/address")]" >"$folder/only-$1/servers"
  echo "server $1 is running"
}

stop() {
  kill -9 -- "-${group[$1]}"
  wait "${group[$1]}" 2>/dev/null || true
  unset "group[$1]"
  echo "server $1 is stopped, with kill -9"
}
