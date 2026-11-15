#!/usr/bin/env bash
# Proves a server installs on a machine as one thing, with the one command, and updates in place, on Linux and on
# a Mac. On a network with its address on this machine (`./network --address`), a second machine, a home folder of
# its own whose PATH has no Erlang, Elixir, Rust or Bun on it, and with only `one` from a build: turns itself into a
# server of the network's (`one serve on`), which the system keeps running, a real systemd user unit on Linux and
# a LaunchAgent on a Mac; runs with its person's login PATH; shows other users of the machine no secret and listens
# as no Erlang node; runs a call and deploys an app through that server alone; is killed and started again by the
# system; takes a newer build when it's asked, while the network's other server answers; goes back by itself from a
# build that can't start; and is removed, leaving nothing behind. The builds are made by `./release` from this
# repo's commit and two made on top of it.
#
#     cargo build --release && mise exec -- server/test/install.sh
#
# Each server is the system's under a name of its key, `one-server-<key>.service` or `app.theone.server.<key>`, so
# none that's already on the machine is touched. A Mac needs GNU's tail, as `gtail` (Homebrew's coreutils). It takes
# about fifteen minutes, most of it building.
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../.." && pwd)
# Short, as a Mac keeps the path of a socket in its folder under 104 bytes.
folder=$(mktemp -d /tmp/one-install.XXXXXX)
one=$root/target/release/one
# The three builds are made in clones of this repo, sharing what Rust has built already.
export CARGO_TARGET_DIR=$root/target
machine=$folder/machine
service= frozen=
started=()
# GNU's tail, which follows a file for as long as a process runs.
tail=$(command -v gtail || command -v tail)
trap '[[ -z $frozen ]] || kill -CONT "$frozen"
      [[ -z $service ]] || remove
      for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }
size() { du -sh "$1" | cut -f1; }

say "A network, with its address here"
"$root/network" "$folder/network" --address >"$folder/network.log" 2>&1 &
started+=($!)
grep -q -m1 "A network is running" <("$tail" -f --pid=$! -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
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
  (cd "$clone" && ONE_HOME=$client ONE_AS=owner ONE_FOR=network ./release "$kind") >"$folder/release-$1.log" 2>&1 ||
    { tail -30 "$folder/release-$1.log" >&2; exit 1; }
  grep -E "^(server|one)/" "$folder/release-$1.log" | sed 's/^/  /' >&2
  "$clone/build-name"
}

# This kind of machine, as builds are named, and what the system keeps the server running with here.
kind="$(uname -s | sed -e s/Linux/linux/ -e s/Darwin/macos/)-$(uname -m | sed s/arm64/aarch64/)"
if [[ $kind == linux-* ]]; then
  data=.local/share
  named() { service=one-server-${1:0:12}.service; }
  running() { [[ $(systemctl --user is-active "$service") == active && $(systemctl --user is-enabled "$service") == enabled ]]; }
  main_pid() { systemctl --user show -p MainPID --value "$service"; }
  restarts() { systemctl --user show -p NRestarts --value "$service"; }
  # Every process of the server's.
  processes() { paste -sd, - <"/sys/fs/cgroup$(systemctl --user show -p ControlGroup --value "$service")/cgroup.procs"; }
  listening() { ss -Htlnp | grep -E "pid=(${1//,/|})," || true; }
  its_path() { systemctl --user show -p Environment --value "$service" | tr ' ' '\n' | sed -n 's/^PATH=//p'; }
  # What it has said, from where it was when `mark` was last asked.
  mark() { cursor=$(journalctl --user -u "$service" -n 0 --show-cursor | grep -o 's=.*'); }
  said() { journalctl --user -u "$service" -o cat --after-cursor="$cursor" "$@"; }
  remove() { systemctl --user cat "$service" >/dev/null 2>&1 && systemctl --user disable --now "$service"; }
  gone() { ! systemctl --user cat "$service" >/dev/null 2>&1 && [[ -z $(find "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user" -name "$service") ]]; }
else
  data="Library/Application Support"
  domain=gui/$(id -u)
  named() { service=app.theone.server.${1:0:12}; }
  running() { launchctl print "$domain/$service" | grep -q "state = running"; }
  main_pid() { launchctl print "$domain/$service" | awk '/^\tpid = / { print $3 }'; }
  restarts() { launchctl print "$domain/$service" | awk '/^\truns = / { print $3 - 1 }'; }
  processes() {
    local all=$(main_pid) next=$(main_pid)
    while next=$(pgrep -P "${next//$'\n'/,}" | paste -sd, -) && [[ -n $next ]]; do all=$all,$next; done
    echo "$all"
  }
  listening() { lsof -nP -a -iTCP -sTCP:LISTEN -p "$1" | tail -n +2 || true; }
  its_path() { plutil -extract EnvironmentVariables.PATH raw -o - "$machine/home/Library/LaunchAgents/$service.plist"; }
  log=$machine/home/$data/app.theone.client/server/server.log
  mark() { cursor=$(($(wc -l <"$log") + 1)); }
  said() { if [[ ${1-} == -f ]]; then "$tail" -n "+$cursor" -f --pid=$$ "$log" 2>/dev/null; else tail -n "+$cursor" "$log"; fi; }
  remove() { launchctl bootout "$domain/$service" 2>/dev/null || true; }
  gone() { ! launchctl print "$domain/$service" >/dev/null 2>&1 && [[ ! -e $machine/home/Library/LaunchAgents/$service.plist ]]; }
fi

say "Building and publishing this commit"
began=$(now)
a=$(release this)
echo "build $a, in $(since "$began")"

say "The second machine: its own home, a PATH with no Erlang, Elixir, Rust or Bun, and one from the build"
mkdir -p "$machine/home" "$machine/bin"
gzip -dc "$CARGO_TARGET_DIR/builds/$a/one-$kind.gz" >"$machine/bin/one"
chmod +x "$machine/bin/one"
home=$machine/home/$data/app.theone.client
# It's one of the network's devices: a network being built gives its device a pass as it's started, as `mix
# network` gives its own client one.
key=$(cd "$root/server" && mix run --no-start -e "
  {:ok, key} = OneCore.Iroh.key(\"$home/key\")
  Application.put_env(:one_core, :home, \"$folder/network/server\")
  File.write!(\"$home/network\", JSON.encode!(%{address: \"$(grep -o '"address":"[^"]*"' "$client/network" | cut -d'"' -f4)\", pass: OneCore.Address.pass(key)}))
  IO.puts(key)")
# Each command there is given five minutes.
on_machine() {
  timeout 300 env -i HOME="$machine/home" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR-}" \
    DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS-}" ONE_AS=owner ONE_FOR=network "$machine/bin/one" "$@"
}
for tool in erl elixir mix cargo rustc bun; do
  ! env -i PATH=/usr/bin:/bin sh -c "command -v $tool" >/dev/null || { echo "$tool is on the machine's PATH" && exit 1; }
done
echo "its device: $key"
echo "one --version: $(on_machine --version)"

# Waits for the server to say something, from where it was when `mark` was last asked.
heard() { timeout 600 grep -q -m1 -E "$1" <(said -f) || { said | tail -20; exit 1; }; }
build_of() { on_machine builds --server "$1" | grep -o '"build":"[^"]*"' | head -1 | cut -d'"' -f4; }

say "one serve on"
began=$(now)
on_machine serve on | tee "$folder/serve-on"
server=$(grep -o "Its key: [0-9a-f]*" "$folder/serve-on" | grep -o "[0-9a-f]*$")
named "$server"
running || { echo "the system isn't running $service" && exit 1; }
echo "in $(since "$began"); the system runs it as $service, and starts it whenever it starts"
echo "with its person's login PATH, not the bare one it was set up from: $(its_path)"
echo "it answers by its key, as build $(build_of "$server")"
owner builds | grep -o "\"server\":\"$server\"" >/dev/null && echo "it's on the network's list of servers"

say "What other users of the machine see of it"
procs=$(processes)
# Every process of the server, as ps shows it to anyone: its cookie is the word the release names, never a secret.
ps -o pid=,args= -p "$procs" | grep -o -- "-setcookie [^ ]*" | sort -u | sed 's/^/  /'
ps -o args= -p "$procs" | grep -o -- "-setcookie [^ ]*" | grep -qv -- "-setcookie no-distribution$" &&
  { echo "a cookie that isn't the release's word is on a command line" && exit 1; }
# It reaches others by its key over UDP, and nothing listens as an Erlang node: no epmd, no distribution port.
listening=$(listening "$procs")
[[ -z $listening ]] || { echo "it listens on TCP: $listening" && exit 1; }
ps -o comm= -p "$procs" | grep -q epmd && { echo "epmd runs with it" && exit 1; }
echo "  it listens on no TCP port, and no epmd runs with it"

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

say "Killed, it's started again by the system"
mark
pid=$(main_pid)
kill -9 "$pid"
echo "killed $pid with kill -9"
heard "A server is running"
echo "running again as $(main_pid), restarted $(restarts) time(s), answering as $(build_of "$server")"

say "A newer build, published and taken when it's asked"
b=$(release newer true)
mark
owner update --server "$server"
began=$(now)
echo "while it updates, the network's other server answers: $(owner builds | grep -o '"asked":"[^"]*"' | sed "s/$first/the network's first server/")"
heard "The server is on the build"
said | grep -E "^The server is on the build" | sed 's/^/  /'
echo "in $(since "$began"); it answers as $(build_of "$server"), and the machine's one is $(on_machine --version)"
echo "kept: $(ls "$home/server/builds" | paste -sd' ' -)"

say "A build that can't start, which it goes back from by itself"
c=$(release broken "perl -pi -e 's/  def serve do/  def serve do\n    raise \"this build can'\''t start\"/' server/lib/one_core/release.ex")
mark
owner update --server "$server"
heard "didn't start and answer"
said | grep -E "can't start|didn't start and answer" | sed 's/^/  /'
heard "A server is running"
echo "it answers as $(build_of "$server"), not $c"

say "Sizes"
echo "  the server installed: $(size "$home/server/builds/$b") (build $b), and its Bun, fetched once: $(size "$home/server/home/bun")"
echo "  one: $(size "$machine/bin/one")"
echo "  what an update downloads: $(size "$CARGO_TARGET_DIR/builds/$b/server-$kind.tar.gz") for the server, $(size "$CARGO_TARGET_DIR/builds/$b/one-$kind.gz") for one"

say "one serve off"
began=$(now)
mark
on_machine serve off || { said | tail -20; exit 1; }
echo "in $(since "$began")"
gone || { echo "the system still has $service" && exit 1; }
[[ ! -e $home/server ]] || { echo "left behind: $home/server" && exit 1; }
owner builds | grep -q "\"server\":\"$server\"" && { echo "it's still on the network's list" && exit 1; }
echo "the system has no $service, the machine has nothing of it, and the network's list doesn't name it: what's left is"
service=
(cd "$machine" && find . -type f | sort | sed 's/^/  /')
