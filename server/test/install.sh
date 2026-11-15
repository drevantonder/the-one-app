#!/usr/bin/env bash
# Proves a server installs on a machine as one thing, with the one command, and updates in place. On a network
# with its address on this machine (`./network --address`), a second machine, a home folder of its own whose PATH
# has no Erlang, Elixir, Rust or Bun on it, and with only `one` from a build: turns itself into a server of the
# network's (`one serve on`), which a real systemd user unit keeps running; runs a call and deploys an app through
# that server alone; is killed and started again by systemd; takes a newer build when it's asked, while the
# network's other server answers; goes back by itself from a build that can't start; and is removed, leaving
# nothing behind. The builds are made by `./release` from this repo's commit and two made on top of it.
#
#     cargo build --release && mise exec -- server/test/install.sh
#
# It uses systemd's user units, each under a name of the server's key, and takes about fifteen minutes, most of
# it building.
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../.." && pwd)
folder=$(mktemp -d)
one=$root/target/release/one
# What the three builds are made in: each a clone of this repo, sharing what Rust has built already.
export CARGO_TARGET_DIR=$root/target/proof
machine=$folder/machine
unit= frozen=
started=()
trap '[[ -z $frozen ]] || kill -CONT "$frozen"
      [[ -z $unit ]] || ! systemctl --user cat "$unit" >/dev/null 2>&1 || systemctl --user disable --now "$unit"
      for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }
size() { du -sh "$1" | cut -f1; }

say "A network, with its address here"
"$root/network" "$folder/network" --address >"$folder/network.log" 2>&1 &
started+=($!)
grep -q -m1 "A network is running" <(tail -f --pid=$! -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
client=$folder/network/client
# Its owner, at a device of theirs that isn't the second machine.
owner() { ONE_HOME=$client ONE_AS=owner ONE_FOR=network "$one" "$@"; }
first=$(grep -o "running in [^ ]*: [0-9a-f]*" "$folder/network/server.log" | grep -o "[0-9a-f]*$")
echo "its server: $first"

# A build of this repo's commit, with `change` made on top of it and committed first if there is one, published
# to the network by its owner. Says the build's name.
release() {
  local clone=$folder/build-$1
  git clone -q --no-checkout "$root" "$clone"
  git -C "$clone" checkout -q "$(git -C "$root" rev-parse HEAD)"
  if [[ -n ${2-} ]]; then
    (cd "$clone" && eval "$2")
    git -C "$clone" -c user.name=proof -c user.email=proof@localhost commit -q --allow-empty -am "$1"
  fi
  (cd "$clone" && ONE_HOME=$client ONE_AS=owner ONE_FOR=network ./release linux-x86_64) >"$folder/release-$1.log" 2>&1 ||
    { tail -30 "$folder/release-$1.log" >&2; exit 1; }
  grep -E "^(server|one)/" "$folder/release-$1.log" | sed 's/^/  /' >&2
  git -C "$clone" describe --always --abbrev=12 --exclude='*'
}

say "Building and publishing this commit"
began=$(now)
a=$(release this)
echo "build $a, in $(since "$began")"

say "The second machine: its own home, a PATH with no Erlang, Elixir, Rust or Bun, and one from the build"
mkdir -p "$machine/home" "$machine/bin"
gzip -dc "$CARGO_TARGET_DIR/builds/$a/one-linux-x86_64.gz" >"$machine/bin/one"
chmod +x "$machine/bin/one"
home=$machine/home/.local/share/app.theone.client
# It's one of the network's devices: a network being built gives its device a pass as it's started, as `mix
# network` gives its own client one.
key=$(cd "$root/server" && mix run --no-start -e "
  {:ok, key} = OneCore.Iroh.key(\"$home/key\")
  Application.put_env(:one_core, :home, \"$folder/network/server\")
  File.write!(\"$home/network\", JSON.encode!(%{address: \"$(grep -o '"address":"[^"]*"' "$client/network" | cut -d'"' -f4)\", pass: OneCore.Address.pass(key)}))
  IO.puts(key)")
# Each command there is given five minutes.
on_machine() {
  timeout 300 env -i HOME="$machine/home" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
    DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS-}" ONE_AS=owner ONE_FOR=network "$machine/bin/one" "$@"
}
for tool in erl elixir mix cargo rustc bun; do
  ! env -i PATH=/usr/bin:/bin sh -c "command -v $tool" >/dev/null || { echo "$tool is on the machine's PATH" && exit 1; }
done
echo "its device: $key"
echo "one --version: $(on_machine --version)"

# Waits for the unit's log to say something, from where it was when `mark` was last asked.
mark() { cursor=$(journalctl --user -u "$unit" -n 0 --show-cursor | grep -o 's=.*'); }
heard() {
  timeout 600 grep -q -m1 -E "$1" <(journalctl --user -u "$unit" -f -o cat --after-cursor="$cursor") ||
    { journalctl --user -u "$unit" -o cat --after-cursor="$cursor" | tail -20; exit 1; }
}
build_of() { on_machine builds --server "$1" | grep -o '"build":"[^"]*"' | head -1 | cut -d'"' -f4; }

say "one serve on"
began=$(now)
on_machine serve on | tee "$folder/serve-on"
server=$(grep -o "Its key: [0-9a-f]*" "$folder/serve-on" | grep -o "[0-9a-f]*$")
unit=one-server-${server:0:12}.service
echo "in $(since "$began"); systemd says it's $(systemctl --user is-active "$unit"), and enabled: $(systemctl --user is-enabled "$unit")"
echo "it answers by its key, as build $(build_of "$server")"
owner builds | grep -o "\"server\":\"$server\"" >/dev/null && echo "it's on the network's list of servers"

say "Through it alone: the network's other server is frozen"
frozen=$(pgrep -f -- "network $folder/network server ")
kill -STOP "$frozen"
mkdir -p "$folder/hello/app"
cat >"$folder/hello/hello.ex" <<'EOF'
defmodule Hello do
  @behaviour OneCore.Plugin

  @impl true
  def apps, do: [%{name: "Hello", page: "app/index.html", icon: "icon.svg"}]

  @impl true
  def tools, do: [%{name: "hi", read_only: true, description: "Says which server answered", inputs: %{type: "object"}}]

  @impl true
  def call("hi", _inputs, _call), do: "Hello from #{OneCore.Server.key()}"
end
EOF
echo '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>' >"$folder/hello/icon.svg"
echo '<!doctype html><title>Hello</title><script type="module" src="./app.jsx"></script>' >"$folder/hello/app/index.html"
cat >"$folder/hello/app/app.jsx" <<'EOF'
import { render } from "@solidjs/web"
render(() => <p>Hello</p>, document.body)
EOF
began=$(now)
on_machine deploy --name hello --files-from "$folder/hello"
echo "deployed, its app built with the Bun the server fetched, in $(since "$began")"
began=$(now)
on_machine hello hi
echo "called in $(since "$began")"
kill -CONT "$frozen"
frozen=

say "Killed, it's started again by systemd"
mark
pid=$(systemctl --user show -p MainPID --value "$unit")
kill -9 "$pid"
echo "killed $pid with kill -9"
heard "A server is running"
echo "running again as $(systemctl --user show -p MainPID --value "$unit"), restarted $(systemctl --user show -p NRestarts --value "$unit") time(s), answering as $(build_of "$server")"

say "A newer build, published and taken when it's asked"
b=$(release newer true)
mark
owner update --server "$server"
began=$(now)
echo "while it updates, the network's other server answers: $(owner builds | grep -o '"asked":"[^"]*"' | sed "s/$first/the network's first server/")"
heard "The server is on the build"
journalctl --user -u "$unit" -o cat --after-cursor="$cursor" | grep -E "^The server is on the build" | sed 's/^/  /'
echo "in $(since "$began"); it answers as $(build_of "$server"), and the machine's one is $(on_machine --version)"
echo "kept: $(ls "$home/server/builds" | paste -sd' ')"

say "A build that can't start, which it goes back from by itself"
c=$(release broken "sed -i 's/  def serve do/  def serve do\n    raise \"this build can'\''t start\"/' server/lib/one_core/release.ex")
mark
owner update --server "$server"
heard "didn't start and answer"
journalctl --user -u "$unit" -o cat --after-cursor="$cursor" | grep -E "can't start|didn't start and answer" | sed 's/^/  /'
heard "A server is running"
echo "it answers as $(build_of "$server"), not $c"

say "Sizes"
echo "  the server installed: $(size "$home/server/builds/$b") (build $b), and its Bun, fetched once: $(size "$home/server/home/bun")"
echo "  one: $(size "$machine/bin/one")"
echo "  what an update downloads: $(size "$CARGO_TARGET_DIR/builds/$b/server-linux-x86_64.tar.gz") for the server, $(size "$CARGO_TARGET_DIR/builds/$b/one-linux-x86_64.gz") for one"

say "one serve off"
began=$(now)
on_machine serve off || { journalctl --user -u "$unit" -o cat -n 20; exit 1; }
echo "in $(since "$began")"
systemctl --user cat "$unit" >/dev/null 2>&1 && { echo "systemd still has $unit" && exit 1; }
left=$(find "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user" -name "$unit")
[[ -z $left ]] || { echo "left behind: $left" && exit 1; }
[[ ! -e $home/server ]] || { echo "left behind: $home/server" && exit 1; }
owner builds | grep -q "\"server\":\"$server\"" && { echo "it's still on the network's list" && exit 1; }
echo "systemd has no $unit, the machine has nothing of it, and the network's list doesn't name it: what's left is"
unit=
(cd "$machine" && find . -type f | sort | sed 's/^/  /')
