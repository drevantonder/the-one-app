#!/usr/bin/env bash
# Proves, in the real app, what a plugin's app can reach: its client's bridge, and nothing else. Two plugins are
# deployed to a network on this machine. The first's app keeps something of its own. The second's tries every way
# it has to the shell's page, to the app's Rust half, and to the first app, and tells its plugin what came of each,
# which this then reads.
#
#     cargo build --release && mise exec -- client/shell/test/isolation.sh
#
# It needs Linux with `xvfb-run`, `dbus-run-session` and `xdotool`: the app runs on a screen of its own, with a
# session bus of its own, and its icons are pressed there.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
source "$here/../../../server/test/servers.sh"

mkdir "$folder/kept" "$folder/prober"
cat >"$folder/kept/kept.ex" <<'PLUGIN'
defmodule Kept do
  @behaviour OneCore.Plugin

  @impl true
  def apps, do: [%{name: "Kept", page: "app.html", icon: "icon.svg"}]
end
PLUGIN
cat >"$folder/kept/app.html" <<'PAGE'
<!doctype html>
<title>Kept</title>
<p id="secret">only Kept reads this</p>
<script>localStorage.setItem("secret", "only Kept reads this")</script>
PAGE

# What the app found is held by a process of the plugin's, which answers whoever waits for it.
cat >"$folder/prober/prober.ex" <<'PLUGIN'
defmodule Prober do
  @behaviour OneCore.Plugin

  @impl true
  def apps, do: [%{name: "Prober", page: "app.html", icon: "icon.svg"}]

  @impl true
  def tools do
    [
      %{name: "found", description: "Keeps what the app found", inputs: %{type: "object"}},
      %{name: "wait", description: "Gives what the app found, once it has", inputs: %{type: "object"}}
    ]
  end

  @impl true
  def call("found", found, call), do: GenServer.call(held(call), {:found, found})
  def call("wait", _inputs, call), do: GenServer.call(held(call), :wait, :infinity)

  defp held(call), do: OneCore.Call.process(call, Prober.Held, nil, nil)
end

defmodule Prober.Held do
  use GenServer

  @impl true
  def init(nil), do: {:ok, %{found: nil, waiting: []}}

  @impl true
  def handle_call({:found, found}, _from, held) do
    for waiting <- held.waiting, do: GenServer.reply(waiting, found)
    {:reply, "Kept.", %{found: found, waiting: []}}
  end

  def handle_call(:wait, from, %{found: nil} = held), do: {:noreply, %{held | waiting: [from | held.waiting]}}
  def handle_call(:wait, _from, held), do: {:reply, held.found, held}
end
PLUGIN
cp "$here/prober.html" "$folder/prober/app.html"
cp "$here/prober.js" "$folder/prober/app.js"
for plugin in kept prober; do echo '<svg xmlns="http://www.w3.org/2000/svg"/>' >"$folder/$plugin/icon.svg"; done

say "A network whose first two apps are Kept and Prober"
start a
# The home screen lists apps by their plugins' names, so these two come before any the network started with.
ONE_HOME=$folder/client "$one" deploy --name a-kept --files-from "$folder/kept"
ONE_HOME=$folder/client "$one" deploy --name a-prober --files-from "$folder/prober"

say "The app, on a screen of its own: Kept is started behind the home screen, and Prober is opened"
ONE_HOME=$folder/client dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
  "$0" 2>"$1/app.log" &
  app=$!
  # The home screen says when it has drawn.
  grep -q -m1 "home opened" <(tail -f --pid=$app -n +1 "$1/app.log")
  window=$(xdotool search --sync --onlyvisible --name "The One App" | head -1)
  # An app starts as a finger lands on its icon, and shows only if the finger lifts there. The icons are 72 wide
  # with 8 between them, from 32 in and 56 down. The screen takes a moment to pass each move on, which is waited.
  xdotool mousemove --window "$window" 68 86 sleep 0.5 mousedown 1 sleep 0.5 mousemove --window "$window" 600 600 sleep 0.5 mouseup 1
  xdotool mousemove --window "$window" 148 86 sleep 0.5 click 1
  # If Prober got out of its frame, nothing is left to tell its plugin, and this gives up.
  timeout 60 env ONE_HOME=$1/client "$0" a-prober wait >"$1/found.json" || echo "{}" >"$1/found.json"
  kill $app
' "$one" "$folder"

say "What Prober found"
(cd "$here/../../../address" && bun -e '
  const found = JSON.parse(await Bun.file(process.argv[1]).text());
  let reached = 0;
  for (const [tried, { reached: did, said }] of Object.entries(found)) {
    console.log(`${did ? "REACHED" : "refused"}  ${tried}: ${said}`);
    reached += did;
  }
  if (!Object.keys(found).length) throw new Error("Prober found nothing, so nothing was proved");
  if (reached) throw new Error(`${reached} of what an app must not reach were reached`);
  console.log(`\nAll ${Object.keys(found).length} were refused, and its own tool was still called through the bridge.`);
' "$folder/found.json")
