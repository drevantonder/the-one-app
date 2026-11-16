#!/usr/bin/env bash
# Tests a server installs on a machine as one thing, with the one command, and updates in place, on Linux, on a Mac
# and on Windows. On a network with its edge on this machine (`tooling/network --edge`), a second machine, a home
# folder of its own whose PATH has no Erlang, Elixir, Rust or Bun on it, or the Windows PC named, and with only
# `one` from a build: turns itself into a server of the network's (`one serve on`), which the system keeps running,
# a real systemd user unit on Linux, a LaunchAgent on a Mac and a scheduled task on Windows; runs with its person's
# login PATH, or on Windows their environment; shows other users of the machine no secret and listens
# as no Erlang node; runs a call and deploys an app through that server alone; is killed and started again by the
# system; takes a newer build when it's asked, while the network's other server answers; goes back by itself from a
# build that can't start; and is removed, leaving nothing behind. The builds are made by `tooling/release` from this
# repo's commit and two made on top of it.
#
#     cargo build --release && mise exec -- core/server/test/install.sh [<a Windows PC, as ssh reaches it>]
#
# Windows' builds are made here (`tooling/release`), and the PC is reached by ssh, which carries the network's edge
# and bucket to it at the same addresses as here. It's the PC's person's own: the server is theirs, in a folder of
# its own, and once Task Scheduler starts it again after it's killed it does so within a minute. Not yet run on a PC:
# Windows' server, its task and its wall are built and type-checked, and haven't run on Windows' hardware.
#
# Each server is the system's under a name of its key, `one-server-<key>.service`, `app.theone.server.<key>` or
# `The One App server <key>`, so none that's already on the machine is touched. A Mac needs GNU's tail, as `gtail` (Homebrew's coreutils). It takes
# about fifteen minutes, most of it building.
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../../.." && pwd)
pc=${1-}
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
      [[ -z $pc ]] || ps1 <<<"Remove-Item -Recurse -Force -LiteralPath \"$machine\" -ErrorAction SilentlyContinue"
      for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }
size() { du -sh "$1" | cut -f1; }

say "A network, with its edge here"
"$root/tooling/network" "$folder/network" --edge >"$folder/network.log" 2>&1 &
started+=($!)
grep -q -m1 "A network is running" <("$tail" -F --pid=$! -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
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
    git -C "$clone" -c user.name=test -c user.email=test@localhost commit -q --allow-empty -am "$1"
  fi
  (cd "$clone" && PATH=$root/target/release:$PATH ONE_HOME=$client ONE_AS=owner ONE_FOR=network tooling/release "server/$kind" "$program/$kind") >"$folder/release-$1.log" 2>&1 ||
    { tail -30 "$folder/release-$1.log" >&2; exit 1; }
  grep -E "^(server|one|one-cli)/" "$folder/release-$1.log" | sed 's/^/  /' >&2
  "$clone/tooling/version"
}

# The second machine's kind, as builds are named, which `one` it runs, and what the system keeps the server running
# with there. What's there is listed, sized and looked for with `there_*`.
kind="$(uname -s | sed -e s/Linux/linux/ -e s/Darwin/macos/)-$(uname -m | sed s/arm64/aarch64/)" program=one
there_ls() { ls "$1" | paste -sd' ' -; }
there_size() { du -sh "$1" | cut -f1; }
there_exists() { [[ -e $1 ]]; }
there_files() { (cd "$1" && find . -type f | sort); }
command_lines() { ps -o args= -p "$1"; }
process_names() { ps -o comm= -p "$1"; }
kill_server() { kill -9 "$1"; }
if [[ -n $pc ]]; then
  kind=windows-x86_64 program=one-cli
  # A PowerShell script, from its input, run on the PC.
  ps1() { ssh "$pc" "powershell -NoProfile -NonInteractive -EncodedCommand $({ echo '$ProgressPreference = "SilentlyContinue"'; cat; } | iconv -t utf-16le | base64 -w0)" | tr -d '\r'; }
  # A file or folder of this machine's put on the PC, by where it is there.
  there() { scp -q -r "$1" "$pc:${machine//\\//}/" && echo "$machine\\$(basename "$1")"; }
  there_ls() { ps1 <<<"(Get-ChildItem -Name -LiteralPath '$1') -join ' '"; }
  there_size() { ps1 <<<"'{0:N0} MB' -f ((Get-ChildItem -Recurse -File -LiteralPath '$1' | Measure-Object Length -Sum).Sum / 1MB)"; }
  there_exists() { [[ $(ps1 <<<"Test-Path -LiteralPath '$1'") == True ]]; }
  there_files() { ps1 <<<"Get-ChildItem -Recurse -File -LiteralPath '$1' | ForEach-Object { \$_.FullName.Substring('$1'.Length) } | Sort-Object"; }
  command_lines() { ps1 <<<"(Get-CimInstance Win32_Process -Filter 'ProcessId = $(sed 's/,/ or ProcessId = /g' <<<"$1")').CommandLine"; }
  process_names() { ps1 <<<"(Get-Process -Id $1).ProcessName"; }
  kill_server() { ps1 <<<"Stop-Process -Force -Id $1"; }
  data=
  named() { service="The One App server ${1:0:12}"; }
  task() { ssh "$pc" "schtasks /query /tn \"$service\" /fo csv /nh /v" | tr -d '\r'; }
  running() { task | grep -q '"Running"'; }
  # The program Task Scheduler runs, and everything started from the folder it's in.
  main_pid() { ps1 <<<"(Get-CimInstance Win32_Process -Filter \"Name = 'one.exe'\" | Where-Object ExecutablePath -eq '$machine\one.exe').ProcessId"; }
  restarts() { echo "by its task, last at $(task | cut -d, -f6 | tr -d '"')"; }
  processes() { ps1 <<<"(Get-CimInstance Win32_Process | Where-Object { \$_.ExecutablePath -like '$machine\*' }).ProcessId -join ','"; }
  listening() { ps1 <<<"Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object OwningProcess -in @($1) | Out-String"; }
  its_path() { echo "(its person's own, as they log on)"; }
  mark() { cursor=$(($(ps1 <<<"@(Get-Content -LiteralPath '$log').Count") + 1)); }
  said() {
    local wait=; [[ ${1-} != -f ]] || wait=-Wait
    ps1 <<<"Get-Content -LiteralPath '$log' $wait | Select-Object -Skip $((cursor - 1))"
  }
  remove() { ssh "$pc" "schtasks /end /tn \"$service\" & schtasks /delete /tn \"$service\" /f" >/dev/null 2>&1 || true; }
  gone() { ! ssh "$pc" "schtasks /query /tn \"$service\"" >/dev/null 2>&1; }
elif [[ $kind == linux-* ]]; then
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
# It's one of the network's devices: a network being built gives its device a token as it's started, as `mix
# network` gives its own client one. Made here, and on Windows copied to the PC.
token() {
  (cd "$root/core/server" && mix run --no-start -e "
    {:ok, key} = OneCore.Iroh.key(\"$1/key\")
    Application.put_env(:one_core, :home, \"$folder/network/server\")
    File.write!(\"$1/network\", JSON.encode!(%{address: \"$address\", token: OneCore.Edge.token(key)}))
    IO.puts(key)")
}
address=$(grep -o '"address":"[^"]*"' "$client/network" | cut -d'"' -f4)
if [[ -n $pc ]]; then
  machine=$(ps1 <<<"(New-Item -ItemType Directory -Force (Join-Path \$env:LOCALAPPDATA 'Temp\\$(basename "$folder")')).FullName")
  home=$machine\\home
  log=$home\\server\\server.log
  mkdir -p "$folder/pc/home"
  key=$(token "$folder/pc/home")
  gzip -dc "$CARGO_TARGET_DIR/builds/$a/one-cli-$kind.gz" >"$folder/pc/one.exe"
  there "$folder/pc/one.exe" >/dev/null && there "$folder/pc/home" >/dev/null
  # The network's edge and bucket, which the PC reaches at the same addresses, through ssh.
  bucket=$(owner download --target "$kind" | grep -o 'http://[^/"]*' | head -1 | sed 's|http://||')
  gateway=${address##*:}
  ssh -N -o ExitOnForwardFailure=yes -R "$gateway:localhost:$gateway" -R "${bucket##*:}:$bucket" "$pc" &
  started+=($!)
  on_machine() { timeout 300 ssh "$pc" "set ONE_HOME=$home&& set ONE_AS=owner&& set ONE_FOR=network&& \"$machine\\one.exe\" $*" | tr -d '\r'; }
  for tool in erl elixir mix cargo rustc bun; do
    ! ssh "$pc" "where $tool" >/dev/null 2>&1 || { echo "$tool is on the PC's PATH" && exit 1; }
  done
else
  mkdir -p "$machine/home" "$machine/bin"
  gzip -dc "$CARGO_TARGET_DIR/builds/$a/one-$kind.gz" >"$machine/bin/one"
  chmod +x "$machine/bin/one"
  home=$machine/home/$data/app.theone.client
  key=$(token "$home")
  there() { echo "$1"; }
  # Each command there is given five minutes.
  on_machine() {
    timeout 300 env -i HOME="$machine/home" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR-}" \
      DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS-}" ONE_AS=owner ONE_FOR=network "$machine/bin/one" "$@"
  }
  for tool in erl elixir mix cargo rustc bun; do
    ! env -i PATH=/usr/bin:/bin sh -c "command -v $tool" >/dev/null || { echo "$tool is on the machine's PATH" && exit 1; }
  done
fi
echo "its device: $key"
echo "one --version: $(on_machine --version)"

# Waits for the server to say something, from where it was when `mark` was last asked.
logged() { timeout 600 grep -q -m1 -E "$1" <(said -f) || { said | tail -20; exit 1; }; }
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
command_lines "$procs" | grep -o -- "-setcookie [^ ]*" | sort -u | sed 's/^/  /'
command_lines "$procs" | grep -o -- "-setcookie [^ ]*" | grep -qv -- "-setcookie no-distribution$" &&
  { echo "a cookie that isn't the release's word is on a command line" && exit 1; }
# It reaches others by its key over UDP, and nothing listens as an Erlang node: no epmd, no distribution port.
listening=$(listening "$procs")
[[ -z $listening ]] || { echo "it listens on TCP: $listening" && exit 1; }
process_names "$procs" | grep -q epmd && { echo "epmd runs with it" && exit 1; }
echo "  it listens on no TCP port, and no epmd runs with it"

say "Through it alone: the network's other server is frozen"
frozen=$(pgrep -f -- "network $folder/network server ")
kill -STOP "$frozen"
mkdir -p "$folder/hello/server" "$folder/hello/client"
cat >"$folder/hello/server/hello.ex" <<'EOF'
defmodule Hello do
  @behaviour OneCore.Plugin

  @impl true
  def apps, do: [%{name: "Hello", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools, do: [%{name: "hi", read_only: true, description: "Says which server answered", inputs: %{type: "object"}}]

  @impl true
  def call("hi", _inputs, _call), do: "Hello from #{OneCore.Server.key()}"
end
EOF
echo '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>' >"$folder/hello/client/icon.svg"
echo '<!doctype html><title>Hello</title><script type="module" src="./app.jsx"></script>' >"$folder/hello/client/index.html"
cat >"$folder/hello/client/app.jsx" <<'EOF'
import { render } from "@solidjs/web"
render(() => <p>Hello</p>, document.body)
EOF
began=$(now)
on_machine deploy --name hello --files-from "$(there "$folder/hello")"
echo "deployed, its app built with the Bun the server fetched, in $(since "$began")"
began=$(now)
on_machine hello hi
echo "called in $(since "$began")"
kill -CONT "$frozen"
frozen=

say "Killed, it's started again by the system"
mark
pid=$(main_pid)
kill_server "$pid"
echo "killed $pid, as kill -9 does"
logged "A server is running"
echo "running again as $(main_pid), restarted $(restarts) time(s), answering as $(build_of "$server")"

say "A newer build, published and taken when it's asked"
b=$(release newer true)
mark
owner update --server "$server"
began=$(now)
echo "while it updates, the network's other server answers: $(owner builds | grep -o '"asked":"[^"]*"' | sed "s/$first/the network's first server/")"
logged "The server is on the build"
said | grep -E "^The server is on the build" | sed 's/^/  /'
echo "in $(since "$began"); it answers as $(build_of "$server"), and the machine's one is $(on_machine --version)"
echo "kept: $(there_ls "$home/server/builds")"

say "A build that can't start, which it goes back from by itself"
c=$(release broken "perl -pi -e 's/  def serve do/  def serve do\n    raise \"this build can'\''t start\"/' core/server/lib/one_core/release.ex")
mark
owner update --server "$server"
logged "Went back to"
said | grep -E "can't start|Went back to" | sed 's/^/  /'
logged "A server is running"
echo "it answers as $(build_of "$server"), not $c"

say "Sizes"
echo "  the server installed: $(there_size "$home/server/builds/$b") (build $b), and its Bun, fetched once: $(there_size "$home/server/home/bun")"
echo "  what an update downloads: $(size "$CARGO_TARGET_DIR/builds/$b/server-$kind.tar.gz") for the server, $(size "$CARGO_TARGET_DIR/builds/$b/$program-$kind.gz") for one"

say "one serve off"
began=$(now)
mark
on_machine serve off || { said | tail -20; exit 1; }
echo "in $(since "$began")"
gone || { echo "the system still has $service" && exit 1; }
! there_exists "$home/server" || { echo "left behind: $home/server" && exit 1; }
owner builds | grep -q "\"server\":\"$server\"" && { echo "it's still on the network's list" && exit 1; }
echo "the system has no $service, the machine has nothing of it, and the network's list doesn't name it: what's left is"
service=
there_files "$machine" | sed 's/^/  /'
