#!/usr/bin/env bash
# Proves, with two real servers on this machine and a folder as their bucket, that a plugin's service runs on the
# machine it's installed on and is called from wherever the rest of its plugin runs. The services are the two
# plugins': `inbox`, which watches a folder, and `load`, which brings a program in Rust. They're installed on
# server b only, and called through server a.
#
#     cargo build -p shell && mise exec -- server/test/services.sh
#
# ONE names the client's program, if it isn't target/debug/one. The machine needs rustc, for `load`.
set -euo pipefail
set -m # Each server is a job with a process group of its own, so all of it is stopped at once.

cd "$(dirname "$0")/.."
one=$(realpath "${ONE:-../target/debug/one}")
folder=$(mktemp -d)
bucket=$folder/bucket
declare -A group key
trap 'for server in "${!group[@]}"; do stop "$server" >/dev/null; done; rm -rf "$folder"' EXIT

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
since() { echo "$((($(now) - $1) / 1000)) ms"; }
median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.2f", all[int((NR + 1) / 2)] / 1000 }'; }

# Starts a server, and returns once it says it's running. The client `only-<server>` asks it and no other.
start() {
  mix network "$folder" "$1" >"$folder/$1.log" 2>&1 &
  group[$1]=$!
  grep -q -m1 "A server is running" <(tail -f --pid=$! -n +1 "$folder/$1.log")
  key[$1]=$(grep -o "running in [^ ]*: [0-9a-f]*" "$folder/$1.log" | grep -o "[0-9a-f]*$")
  mkdir -p "$folder/only-$1"
  echo "[$(cat "$bucket/servers/${key[$1]}/address")]" >"$folder/only-$1/servers"
  echo "server $1 is running: ${key[$1]}"
}

stop() {
  kill -9 -- "-${group[$1]}"
  wait "${group[$1]}" 2>/dev/null || true
  unset "group[$1]"
  echo "server $1 is stopped, with kill -9"
}

# A tool called through the server named, with the servers' keys shown by their names here.
ask() {
  local server=$1
  shift
  ONE_HOME=$folder/only-$server "$one" "$@" 2>&1 | sed -e "s/${key[a]}/a/g" -e "s/${key[b]:-none}/b/g"
}

# A tool called over one connection, as an agent's MCP session or an app does, each call timed into the file named.
timed() {
  coproc MCP { ONE_HOME=$folder/only-$1 "$one" mcp; }
  local began answer
  for _ in $(seq 200); do
    began=$(now)
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":{}}}" >&"${MCP[1]}"
    read -r answer <&"${MCP[0]}"
    echo $(($(now) - began)) >>"$folder/$3"
  done
  exec {MCP[1]}>&-
  wait "$MCP_PID"
  [[ $answer == *'"result"'* && $answer != *isError* ]] || { echo "$2 through $1 wasn't answered: $answer"; exit 1; }
}

# A plugin of this proof's own: a tool that calls nothing, to time the others against, one that kills a service's
# process on the server asked, as a plugin's code is free to, and a service that can be made to never answer again.
mkdir "$folder/proof"
cat >"$folder/proof/proof.ex" <<'EOF'
defmodule Proof do
  @behaviour OneCore.Plugin

  @impl true
  def tools do
    [
      %{name: "ping", read_only: true, description: "Answers, calling nothing", inputs: %{type: "object"}},
      %{
        name: "kill",
        description: "Kills the process of a plugin's service on the server asked",
        inputs: %{type: "object", properties: %{plugin: %{type: "string"}, service: %{type: "string"}}}
      },
      %{name: "hang", description: "Makes this plugin's service never answer again", inputs: %{type: "object"}},
      %{name: "version", description: "Asks this plugin's service its version", inputs: %{type: "object"}}
    ]
  end

  @impl true
  def call("ping", _inputs, _call), do: "pong"

  def call("kill", %{"plugin" => plugin, "service" => service}, _call) do
    Process.exit(OneCore.Services.Instance.whereis(plugin, service), :kill)
    "Killed #{service}'s process."
  end

  def call("hang", _inputs, call), do: OneCore.Service.call(call, Proof.Stuck, :hang)
  def call("version", _inputs, call), do: OneCore.Service.call(call, Proof.Stuck, :version)
end

defmodule Proof.Stuck do
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def needs, do: "Nothing"

  def version(service), do: "Its service is at version #{GenServer.call(service, :version)}."
  def hang(service), do: GenServer.cast(service, :hang)

  @impl GenServer
  def init(_), do: {:ok, nil}

  @impl GenServer
  def handle_call(:version, _from, state), do: {:reply, 1, state}

  @impl GenServer
  def handle_cast(:hang, state) do
    Process.sleep(:infinity)
    {:noreply, state}
  end
end
EOF

say "Two servers on one folder as the bucket, and two plugins that each have a service"
start a
start b
for plugin in inbox load; do ask a deploy --name $plugin --files-from ../plugins/$plugin; done
ask a deploy --name proof --files-from "$folder/proof"
echo "What each service needs, which its machine's owner reads before installing it:"
ask a services
echo "Called before any server runs them:"
ask a load now || true

say "Each service is installed on server b only, by asking server a"
ask a install_service --plugin inbox --service Inbox.Watcher --server "${key[b]}"
ask a install_service --plugin load --service Load.Meter --server "${key[b]}"
ask a services

say "The plugin's tools, called through server a, reach the services on server b"
mkdir "$folder/inbox"
ask a inbox watch --folder "$folder/inbox"
touch "$folder/inbox/first.pdf"
began=$(now)
until ask a inbox list | grep -q first.pdf; do :; done
echo "a file put in the folder was listed through a $(since "$began") later: $(ask a inbox list | sed "s|$folder|...|g")"
ask a inbox watching | sed "s|$folder|...|g"
ask a load now

say "What a call to a service adds, over one connection: the median of 200 calls of each, in ms"
timed b proof_ping ping-b
timed b inbox_watching watching-b
timed a proof_ping ping-a
timed a inbox_watching watching-a
timed b load_now load-b
timed a load_now load-a
echo "$(median "$folder/ping-b")  a tool that calls nothing, through b"
echo "$(median "$folder/watching-b")  a tool that calls a service on the same server, through b"
echo "$(median "$folder/ping-a")  a tool that calls nothing, through a"
echo "$(median "$folder/watching-a")  a tool that calls a service on the other server, through a"
echo "$(median "$folder/load-b")  a tool that calls a service that asks its program, through b"
echo "$(median "$folder/load-a")  the same on the other server, through a"

say "The plugins are updated while their services hold what they hold"
before=$(ask a inbox watching | grep -o '"since":"[^"]*"')
cp -r ../plugins/inbox "$folder/inbox-2"
sed -i 's/"arrived" => watch.arrived}/"arrived" => watch.arrived, "updated" => true}/' "$folder/inbox-2/inbox/watcher.ex"
ask a deploy --name inbox --files-from "$folder/inbox-2"
ask a inbox watching | sed "s|$folder|...|g"
[ "$(ask a inbox watching | grep -o '"since":"[^"]*"')" = "$before" ] || { echo "inbox lost what it held"; exit 1; }
echo "inbox: the new code answers, watching since the same moment, with the 1 arrival it had counted"
touch "$folder/inbox/second.pdf"
until ask a inbox list | grep -q second.pdf; do :; done
echo "and a file that arrives after the update is listed: $(ask a inbox list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"

before=$(ask a load now | grep -o '"since":[0-9]*')
cp -r ../plugins/load "$folder/load-2"
sed -i 's/right now/at this moment/' "$folder/load-2/load.ex"
ask a deploy --name load --files-from "$folder/load-2"
[ "$(ask a load now | grep -o '"since":[0-9]*')" = "$before" ] || { echo "load's program didn't carry on"; exit 1; }
echo "load: its Elixir changed, and its program carried on: measured $before"
sed -i 's/{{\\"processors\\"/{{\\"built\\":2,\\"processors\\"/' "$folder/load-2/meter/main.rs"
began=$(now)
ask a deploy --name load --files-from "$folder/load-2"
echo "load: its program's source changed, so it was built and started again, in the $(since "$began") the deploy took:"
ask a load now
echo "and the folder it was built in before is gone: $(ls "$folder/b/services/load/Load.Meter/meter" | wc -l) is left"

say "A service that has stopped answering doesn't hold up its plugin's update"
ask a install_service --plugin proof --service Proof.Stuck --server "${key[b]}"
ask a proof version
ask a proof hang >/dev/null
sed -i 's/{:reply, 1, state}/{:reply, 2, state}/' "$folder/proof/proof.ex"
began=$(now)
ask a deploy --name proof --files-from "$folder/proof"
echo "the update took $(since "$began"): the service had 5 seconds to move to it, and was stopped"
until ask a proof version 2>&1 | grep -q "version 2"; do :; done
echo "it answered again $(since "$began") after the update began: $(ask a proof version)"

say "The service's process is killed, and then its program: both come back, and the server stays up"
echo "A service that stops is started again at once, or 5 seconds after it last started if that's later."
before=$(ask a inbox watching | grep -o '"since":"[^"]*"')
ask b proof kill --plugin inbox --service Inbox.Watcher
began=$(now)
until ask a inbox watching 2>&1 | grep -q '"since"'; do :; done
echo "inbox answered again $(since "$began") later, watching the folder it's told at its start: $(ask a inbox watching | sed "s|$folder|...|g")"
touch "$folder/inbox/third.pdf"
until ask a inbox list | grep -q third.pdf; do :; done
echo "and lists a file that arrives after it: $(ask a inbox list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"

program() { pgrep -f "$folder/b/services/load/" || true; }
echo "load's program is the process $(program)"
ask b proof kill --plugin load --service Load.Meter
began=$(now)
until ask a load now 2>&1 | grep -q '"since"'; do :; done
echo "the service's process was killed: it answered again $(since "$began") later, with a program of its own, the process $(program)"
kill -9 "$(program)"
echo "the program was killed with kill -9. Asked at once: $(ask a load now || true)"
began=$(now)
until ask a load now 2>&1 | grep -q '"since"'; do :; done
echo "it answered again $(since "$began") later, from the process $(program): $(ask a load now)"
kill -0 "${group[b]}"
echo "server b is the process it was all along, and answers: $(ask b proof ping)"

say "The server that runs the services is stopped, and the caller is told"
stop b
began=$(now)
echo "asked at once, on the connection a still had to b: $(ask a inbox watching || true) ($(since "$began"))"
began=$(now)
echo "asked again: $(ask a inbox watching || true) ($(since "$began"))"
began=$(now)
echo "and again: $(ask a load now || true) ($(since "$began"))"
echo "what arrived is still listed, through a: $(ask a inbox list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"
echo "nothing of b is left running: $(program | wc -l) programs"

say "The server comes back, and its services with it"
start b
ask a inbox watching | sed "s|$folder|...|g"
ask a load now
