#!/usr/bin/env bash
# Tests, with two real servers on this machine and a folder as their bucket, that a plugin's service runs on the
# machine it's installed on and is called from wherever the rest of its plugin runs. The services are the two
# plugins': `folder`, a test plugin that watches a folder, and `load`, which brings a program in Rust. They're installed on
# server b only, and called through server a. It also tests that a network nobody uses reads nothing from its
# bucket, and that b picks up what it wasn't told: behind the next call, and when it starts again.
#
#     cargo build --release && mise exec -- core/server/test/services.sh
#
# The machine needs rustc, for `load`, and Python, to count what's read of the bucket. It takes about seven minutes,
# five of them with nothing used.
source "$(dirname "$0")/servers.sh"

median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.2f", all[int((NR + 1) / 2)] / 1000 }'; }

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

# How many times the bucket is read in the seconds given: files and folders opened in it, by anything.
reads() {
  python3 -I - "$bucket" "$1" <<'EOF'
import ctypes, os, select, struct, sys, time
libc = ctypes.CDLL(None)
watch = libc.inotify_init()
for folder in [here for here, _, _ in os.walk(sys.argv[1])]:
    libc.inotify_add_watch(watch, folder.encode(), 0x20)  # IN_OPEN
end, opened = time.time() + float(sys.argv[2]), 0
while (left := end - time.time()) > 0:
    if select.select([watch], [], [], left)[0]:
        events = os.read(watch, 65536)
        while events:
            opened += 1
            events = events[16 + struct.unpack_from("iIII", events)[3]:]
print(opened)
EOF
}

# A plugin of this test's own: a tool that calls nothing, to time the others against, one that kills a service's
# process on the server asked, as a plugin's code is free to, and a service that can be made to never answer again.
mkdir -p "$folder/probe/server"
cat >"$folder/probe/server/probe.ex" <<'EOF'
defmodule Probe do
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
    Process.exit(OneCore.Plugins.Process.whereis("network/plugins/" <> plugin, service), :kill)
    "Killed #{service}'s process."
  end

  def call("hang", _inputs, call), do: OneCore.Service.call(call, Probe.Stuck, :hang)
  def call("version", _inputs, call), do: OneCore.Service.call(call, Probe.Stuck, :version)
end

defmodule Probe.Stuck do
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def called, do: "Nothing much"

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
ONE_FOR=network ask a deploy --name folder --files-from test/plugins/folder
ONE_FOR=network ask a deploy --name probe --files-from "$folder/probe"
echo "What each service needs, which its machine's owner reads before installing it:"
ask a services
echo "Called before any server runs them:"
ask a load now || true

say "Each service is installed on server b only, by asking server a"
ask a install_service --plugin folder --service Folder.Watcher --server "${key[b]}"
ask a install_service --plugin load --service Load.Meter --server "${key[b]}"
ask a services

say "The plugin's tools, called through server a, reach the services on server b"
mkdir "$folder/watched"
ask a folder watch --folder "$folder/watched"
touch "$folder/watched/first.pdf"
began=$(now)
until ask a folder list | grep -q first.pdf; do :; done
echo "a file put in the folder was listed through a $(since "$began") later: $(ask a folder list | sed "s|$folder|...|g")"
ask a folder watching | sed "s|$folder|...|g"
ask a load now

say "What a call to a service adds, over one connection: the median of 200 calls of each, in ms"
timed b probe_ping ping-b
timed b folder_watching watching-b
timed a probe_ping ping-a
timed a folder_watching watching-a
timed b load_now load-b
timed a load_now load-a
echo "$(median "$folder/ping-b")  a tool that calls nothing, through b"
echo "$(median "$folder/watching-b")  a tool that calls a service on the same server, through b"
echo "$(median "$folder/ping-a")  a tool that calls nothing, through a"
echo "$(median "$folder/watching-a")  a tool that calls a service on the other server, through a"
echo "$(median "$folder/load-b")  a tool that calls a service that asks its program, through b"
echo "$(median "$folder/load-a")  the same on the other server, through a"

say "The plugins are updated while their services hold what they hold"
before=$(ask a folder watching | grep -o '"since":"[^"]*"')
cp -r test/plugins/folder "$folder/folder-2"
sed -i 's/"arrived" => watch.arrived}/"arrived" => watch.arrived, "updated" => true}/' "$folder/folder-2/server/folder/watcher.ex"
ONE_FOR=network ask a deploy --name folder --files-from "$folder/folder-2"
ask a folder watching | sed "s|$folder|...|g"
[ "$(ask a folder watching | grep -o '"since":"[^"]*"')" = "$before" ] || { echo "folder lost what it held"; exit 1; }
echo "folder: the new code answers, watching since the same moment, with the 1 arrival it had counted"
touch "$folder/watched/second.pdf"
until ask a folder list | grep -q second.pdf; do :; done
echo "and a file that arrives after the update is listed: $(ask a folder list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"

before=$(ask a load now | grep -o '"since":[0-9]*')
cp -r ../../plugins/load "$folder/load-2"
sed -i 's/right now/at this moment/' "$folder/load-2/server/load.ex"
ONE_FOR=network ask a deploy --name load --files-from "$folder/load-2"
[ "$(ask a load now | grep -o '"since":[0-9]*')" = "$before" ] || { echo "load's program didn't carry on"; exit 1; }
echo "load: its Elixir changed, and its program carried on: measured $before"
sed -i 's/{{\\"processors\\"/{{\\"built\\":2,\\"processors\\"/' "$folder/load-2/server/meter/main.rs"
began=$(now)
ONE_FOR=network ask a deploy --name load --files-from "$folder/load-2"
echo "load: its program's source changed, so it was built and started again, in the $(since "$began") the deploy took:"
ask a load now
echo "and the folder it was built in before is gone: $(ls "$folder/b/services/network/plugins/load/Load.Meter/server/meter" | wc -l) is left"

say "A service that has stopped answering doesn't hold up its plugin's update"
ask a install_service --plugin probe --service Probe.Stuck --server "${key[b]}"
ask a probe version
ask a probe hang >/dev/null
sed -i 's/{:reply, 1, state}/{:reply, 2, state}/' "$folder/probe/server/probe.ex"
began=$(now)
ONE_FOR=network ask a deploy --name probe --files-from "$folder/probe"
echo "the update took $(since "$began"): the service had 5 seconds to move to it, and was stopped"
until ask a probe version 2>&1 | grep -q "version 2"; do :; done
echo "it answered again $(since "$began") after the update began: $(ask a probe version)"

say "The service's process is killed, and then its program: both come back, and the server stays up"
echo "A service that stops is started again at once, or 5 seconds after it last started if that's later."
before=$(ask a folder watching | grep -o '"since":"[^"]*"')
ask b probe kill --plugin folder --service Folder.Watcher
began=$(now)
until ask a folder watching 2>&1 | grep -q '"since"'; do :; done
echo "folder answered again $(since "$began") later, watching the folder it's told at its start: $(ask a folder watching | sed "s|$folder|...|g")"
touch "$folder/watched/third.pdf"
until ask a folder list | grep -q third.pdf; do :; done
echo "and lists a file that arrives after it: $(ask a folder list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"

program() { pgrep -f "$folder/b/services/network/plugins/load/" || true; }
echo "load's program is the process $(program)"
ask b probe kill --plugin load --service Load.Meter
began=$(now)
until ask a load now 2>&1 | grep -q '"since"'; do :; done
echo "the service's process was killed: it answered again $(since "$began") later, with a program of its own, the process $(program)"
kill -9 "$(program)"
echo "the program was killed with kill -9. Asked at once: $(ask a load now || true)"
began=$(now)
until ask a load now 2>&1 | grep -q '"since"'; do :; done
echo "it answered again $(since "$began") later, from the process $(program): $(ask a load now)"
kill -0 "${group[b]}"
echo "server b is the process it was all along, and answers: $(ask b probe ping)"

say "Nobody uses the network for five minutes, which is how long a server goes before looking again at whoever it's asked for"
echo "the bucket was read $(reads 305) times meanwhile"

say "b is frozen, and probe is deployed again through a, which can't tell b"
kill -STOP -- "-${group[b]}"
sed -i 's/{:reply, 2, state}/{:reply, 3, state}/' "$folder/probe/server/probe.ex"
began=$(now)
ONE_FOR=network ask a deploy --name probe --files-from "$folder/probe"
echo "the deploy took $(since "$began"): a gave up on b after 3 seconds"
kill -CONT -- "-${group[b]}"
seen[b]=$(wc -l <"$folder/b.log")
echo "b wakes, and nothing tells it. Asked through b, a call runs on what b has: $(ask b probe version)"
grep -q -m1 "probe: version" <(tail -f --pid="${group[b]}" -n +$((seen[b] + 1)) "$folder/b.log")
echo "and b looked behind it: $(tail -n +$((seen[b] + 1)) "$folder/b.log" | grep -o "network/plugins/probe: version.*")"
echo "the next call runs on that: $(ask b probe version)"

say "The owner deploys a plugin of their own, whose service is installed on b, and nobody calls it through b"
mkdir -p "$folder/echo/server"
cat >"$folder/echo/server/echo.ex" <<'EOF'
defmodule Echo do
  @behaviour OneCore.Plugin

  @impl true
  def tools, do: [%{name: "hello", read_only: true, description: "Asks this plugin's service", inputs: %{type: "object"}}]

  @impl true
  def call("hello", _inputs, call), do: OneCore.Service.call(call, Echo.Hello, :hello)
end

defmodule Echo.Hello do
  use GenServer
  @behaviour OneCore.Service

  @impl OneCore.Service
  def called, do: "Nothing much"

  @impl OneCore.Service
  def needs, do: "Nothing"

  def hello(_service), do: "Hello from version 1 on #{OneCore.Service.server()}"

  @impl GenServer
  def init(_), do: {:ok, nil}
end
EOF
ask a deploy --name echo --files-from "$folder/echo"
ask a install_service --plugin echo --service Echo.Hello --server "${key[b]}"
ask a echo hello

say "The server that runs the services is stopped, and the caller is told"
stop b
began=$(now)
echo "asked at once, on the connection a still had to b: $(ask a folder watching || true) ($(since "$began"))"
began=$(now)
echo "asked again: $(ask a folder watching || true) ($(since "$began"))"
began=$(now)
echo "and again: $(ask a load now || true) ($(since "$began"))"
echo "what arrived is still listed, through a: $(ask a folder list | grep -o '"name":"[^"]*"' | tr '\n' ' ')"
echo "nothing of b is left running: $(program | wc -l) programs"

say "While b is stopped, the owner's plugin is deployed again through a"
sed -i 's/version 1/version 2/' "$folder/echo/server/echo.ex"
ask a deploy --name echo --files-from "$folder/echo"

say "The server comes back, and its services with it, on what was deployed while it was stopped"
start b
ask a folder watching | sed "s|$folder|...|g"
ask a load now
ask a echo hello
