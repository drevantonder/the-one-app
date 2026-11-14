#!/usr/bin/env bash
# Proves, on a whole network with a spare (`./network <folder> --spare`), who may do what and that the clock calls
# a scheduled job once. As users of a network being built: what one made is read by another only once a relation
# tuple says so, by a group's member, and by a sharing link's holder until its tuple is removed. A plugin reaches
# only what both it and its user may. A plugin is whoever's deployed it, and has an artifact of its own for each
# user, group and the network it's added for. One Core's own tools are whoever's may. Anyone brings a server, which
# acts for them alone until its owner chooses who else and they agree, runs only their plugins and passes on the
# rest. A plugin that decides refuses a call with its reason, and one that breaks is undone. Claims' approver is a
# tuple, and a job that's being called when the server calling it is killed is called once by the spare. It says
# what a check costs a call.
#
#     mise exec -- server/test/who.sh [--address]
#
# With --address the network has its address on this machine too, so its servers reach the bucket only through the
# signer there, which is what refuses a server the blobs of someone it doesn't act for.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each server started here.

root=$(cd "$(dirname "$0")/../.." && pwd)
folder=$(mktemp -d)
"$root/network" "$folder" --spare "$@" >"$folder/network.log" 2>&1 &
network=$!
started=("$network")
# `./network` stops what it started when it's stopped.
trap 'for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -f --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
one=$root/target/release/one
# The network's address, if it has one, as each of its servers is started with it.
at_address=()
[[ ! -e $folder/client/network ]] || at_address=(--address "$(grep -o '"address":"[^"]*"' "$folder/client/network" | cut -d'"' -f4)")

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.3f ms", all[int((NR + 1) / 2)] / 1000 }'; }

# A command as a user of the network, or `link <link>` as a sharing link's holder, and for a group or the network if
# it says: `as alice notes read --note ...`, `as alice for groups/family notes write ...`.
as() {
  local who=$1 asked=
  shift
  if [[ $who == link ]]; then
    who=$1
    shift
    ONE_LINK=$who ONE_HOME=$folder/client "$one" "$@"
  else
    [[ $1 == for ]] && asked=$2 && shift 2
    ${asked:+env ONE_FOR=$asked} env ONE_AS=$who ONE_HOME=${home:-$folder/client} "$one" "$@"
  fi
}
# The same through one server and no other: `via alices as alice diary write ...`.
via() { home=$folder/only-$1 "${@:2}"; }
# Each server's key by its name, which is how it's said here.
declare -A key
key_of() { key[$1]=$(grep -o -E "(running in [^ ]*: |add_server --key )[0-9a-f]{64}" "$folder/$1.log" | head -1 | grep -o -E "[0-9a-f]{64}$"); }
named() { local names=() server; for server in "${!key[@]}"; do names+=(-e "s/${key[$server]}/the $server/g"); done; sed "${names[@]}"; }
key_of server && key_of spare
# Starts a server that is someone's own, and returns once it's running or has said it isn't theirs yet.
start_for() {
  (cd "$root/server" && exec mix network "$folder" "$1" --for "$2" --development "${at_address[@]}") >"$folder/$1.log" 2>&1 &
  started+=($!)
  grep -q -m1 -E "A server is running|isn't one of" <(tail -f --pid=$! -n +1 "$folder/$1.log")
  key_of "$1"
}
# One tool call over the web at the network's address, as whoever holds a sharing link, with what it's answered.
# What it says of itself is someone else, which the address doesn't pass on.
web() {
  local asked answer
  asked="{\"name\":\"$3\",\"arguments\":$4,\"_meta\":{\"as\":\"anyone\",\"for\":\"network\",\"who\":\"users/anyone\"}}"
  answer=$(curl -s -m 20 -X POST "${at_address[1]}/link/$2" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":$asked}")
  sed -E 's/.*"(text|message)":"(([^"\\]|\\.)*)".*/\2/' <<<"$answer"
  [[ $answer == *'"content"'* && $answer != *'"isError":true'* ]]
}
# What a command says is shown with it. `may` is one that has to be answered, and `refused` one that mustn't be.
shown() {
  echo "  $1  $(echo "${*:2}" | sed -e "s|$folder|<folder>|g" -e 's/\(link [a-z/]*[A-Za-z0-9_-]\{6\}\)[A-Za-z0-9_-]*/\1…/' | named)"
  echo "           -> $(named <<<"$said")"
}
may() { said=$("$@" 2>&1) && shown "may    " "$@" || { echo "this was refused, and shouldn't be: $* -> $said" && exit 1; }; }
refused() { ! said=$("$@" 2>&1) && shown "refused" "$@" || { echo "this was answered, and shouldn't be: $* -> $said" && exit 1; }; }

# The plugins this proves with. `notes` makes each note an artifact of whoever wrote it. `reader` reads notes it
# didn't make. `gate` decides calls. `ticker` has a job, whose call takes a second before it writes when it was
# due and which server made it.
mkdir -p "$folder"/{notes,reader,gate,ticker,diary,stuck,lost}
# Two plugins that decide and are broken: one decides every call, and one who reaches what.
for broken in "Stuck call" "Lost reach"; do
  set -- $broken
  cat >"$folder/${1,,}/${1,,}.ex" <<EOF
defmodule $1 do
  @behaviour OneCore.Plugin

  @impl true
  def hooks, do: ["$2"]

  @impl true
  def hook("$2", _about, _call), do: raise("$1 is broken.")
end
EOF
done
set --
cat >"$folder/notes/notes.ex" <<'EOF'
defmodule Notes do
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @impl true
  def tools do
    note = %{type: "string", description: "The note's name"}
    text = %{type: "string", description: "What it says"}

    [
      %{name: "ping", read_only: true, description: "Answers, calling nothing", inputs: %{type: "object"}},
      %{
        name: "write",
        description: "Writes a note of your own, and gives its name",
        inputs: %{type: "object", properties: %{id: %{type: "string"}, text: text}, required: ["id", "text"]}
      },
      %{
        name: "read",
        read_only: true,
        by_link: true,
        description: "Reads a note",
        inputs: %{type: "object", properties: %{note: note}, required: ["note"]}
      },
      %{
        name: "change",
        description: "Changes what a note says",
        inputs: %{type: "object", properties: %{note: note, text: text}, required: ["note", "text"]}
      }
    ]
  end

  @impl true
  def call("ping", _inputs, _call), do: "pong"

  def call("write", %{"id" => id, "text" => text}, call) do
    note = Call.make(call, id)
    Artifact.write(note, "text", text)
    Artifact.name(note)
  end

  def call("read", %{"note" => note}, call) do
    {:ok, text, _tag} = Artifact.read(Call.reach(call, note), "text")
    text
  end

  def call("change", %{"note" => note, "text" => text}, call) do
    note = Call.reach(call, note, "editor")
    {:ok, _was, tag} = Artifact.read(note, "text")
    :ok = Artifact.replace(note, "text", text, tag)
    "Changed."
  end
end
EOF
cat >"$folder/reader/reader.ex" <<'EOF'
defmodule Reader do
  @behaviour OneCore.Plugin

  @impl true
  def tools do
    note = %{type: "string", description: "The note's name"}
    [%{name: "read", read_only: true, description: "Reads a note that another plugin made", inputs: %{type: "object", properties: %{note: note}, required: ["note"]}}]
  end

  @impl true
  def call("read", %{"note" => note}, call) do
    {:ok, text, _tag} = OneCore.Artifact.read(OneCore.Call.reach(call, note), "text")
    text
  end
end
EOF
cat >"$folder/gate/gate.ex" <<'EOF'
defmodule Gate do
  @behaviour OneCore.Plugin

  @impl true
  def hooks, do: ["call"]

  @impl true
  def hook("call", %{"plugin" => "notes", "tool" => "write", "inputs" => %{"text" => text}}, call) do
    if text =~ ~r/password/i, do: {:refuse, "Gate: a note is no place for a password, #{call.for}."}, else: :ok
  end

  def hook("call", _about, _call), do: :ok
end
EOF
cat >"$folder/diary/diary.ex" <<'EOF'
defmodule Diary do
  @behaviour OneCore.Plugin
  alias OneCore.Artifact

  @impl true
  def tools do
    text = %{type: "object", properties: %{text: %{type: "string", description: "What to write"}}, required: ["text"]}
    [%{name: "write", description: "Writes an entry in the diary of whoever this is added for", inputs: text}]
  end

  @impl true
  def call("write", %{"text" => text}, %{own: own} = call) do
    Artifact.write(own, "#{length(Artifact.list(own)) + 1}", text)
    "#{Artifact.name(own)} has #{length(Artifact.list(own))}: ran for #{call.for}, asked by #{call.by}, on #{call.server}"
  end
end
EOF
cat >"$folder/ticker/ticker.ex" <<'EOF'
defmodule Ticker do
  @behaviour OneCore.Plugin
  @every 5
  @schema ["create table ticks (due text, at text, server text)"]

  @impl true
  def uses, do: ["database"]

  @impl true
  def jobs, do: [%{tool: "tick", every: @every}]

  @impl true
  def tools do
    [
      %{name: "tick", description: "Writes when it was due, and which server called it", inputs: %{type: "object"}},
      %{name: "ticks", read_only: true, description: "Lists the ticks", inputs: %{type: "object"}}
    ]
  end

  @impl true
  def call("tick", _inputs, call) do
    now = DateTime.utc_now()
    time = &Calendar.strftime(&1, "%H:%M:%S")
    due = DateTime.from_unix!(div(DateTime.to_unix(now), @every) * @every)
    Process.sleep(1000)
    database = OneCore.Call.plugin(call, "database")

    database.transaction(call, @schema, fn db ->
      database.query(db, "insert into ticks (due, at, server) values (?, ?, ?)", [time.(due), time.(now), call.server])
    end)

    "Ticked."
  end

  def call("ticks", _inputs, call) do
    database = OneCore.Call.plugin(call, "database")
    database.transaction(call, @schema, &database.query(&1, "select due, at, server from ticks order by rowid"))
  end
end
EOF
# They're deployed for the network, so they're everyone's.
for plugin in notes reader ticker; do as anyone for network deploy --name $plugin --files-from "$folder/$plugin" >/dev/null; done

say "One can't read the other's artifact until a tuple says so"
note=$(as alice notes write --id shopping --text "Buy milk")
echo "  alice wrote $note"
may as alice notes read --note "$note"
refused as bob notes read --note "$note"
may as alice relations share --artifact "$note" --relation viewer --who users/bob
may as bob notes read --note "$note"
refused as bob notes change --note "$note" --text "Buy beer"
refused as bob relations share --artifact "$note" --relation viewer --who users/carol

say "A group's member can"
may as alice relations group --name family
may as alice relations share --artifact "$note" --relation editor --who groups/family
refused as carol notes read --note "$note"
may as alice relations share --artifact groups/family --relation member --who users/carol
may as carol notes change --note "$note" --text "Buy oat milk"
may as alice notes read --note "$note"

say "A sharing link's holder can, until its tuple is removed"
made=$(as alice relations link --artifact "$note")
link=$(grep -o '"link":"[^"]*"' <<<"$made" | cut -d'"' -f4)
holder=$(grep -o '"who":"[^"]*"' <<<"$made" | cut -d'"' -f4)
echo "  alice made a link, whose holder the tuple names $holder"
may as link "$link" notes read --note "$note"
refused as link "${link}x" notes read --note "$note"
refused as link "$link" notes change --note "$note" --text "Buy nothing"
refused as link "$link" tracker list
if [[ ${#at_address[@]} -gt 0 ]]; then
  echo "  the same holder comes in at the network's address over the web, with no signing in, at /link/<the link>:"
  may web link "$link" notes_read "{\"note\":\"$note\"}"
  refused web link "$link" notes_change "{\"note\":\"$note\",\"text\":\"Buy nothing\"}"
  refused web link "$link" notes_write '{"id":"mine","text":"As the owner?"}'
  refused web link "$link" relations_share "{\"artifact\":\"$note\",\"relation\":\"owner\",\"who\":\"everyone\"}"
  refused web link "$link" remove '{"name":"relations"}'
  refused web link "${link}x" notes_read "{\"note\":\"$note\"}"
  refused web link "nobody/at-all" notes_read "{\"note\":\"$note\"}"
else
  echo "  (with --address, the same holder comes in at the network's address over the web)"
fi
may as alice relations unshare --artifact "$note" --relation viewer --who "$holder"
refused as link "$link" notes read --note "$note"
[[ ${#at_address[@]} -eq 0 ]] || refused web link "$link" notes_read "{\"note\":\"$note\"}"

say "A plugin can't reach what its user can't, or what it has no relation to itself"
refused as dave notes read --note "$note"
refused as alice reader read --note "$note"
may as alice relations share --artifact "$note" --relation viewer --who network/plugins/reader
may as alice reader read --note "$note"
refused as dave reader read --note "$note"

say "A plugin is whoever's deployed it, and has an artifact of its own for each one it's added for"
may as alice deploy --name diary --files-from "$folder/diary"
may as alice diary write --text "Dear diary"
refused as bob diary write --text "Dear diary"
refused as bob add --plugin users/alice/plugins/diary
may as alice relations share --artifact users/alice/plugins/diary --relation viewer --who everyone
may as bob add --plugin users/alice/plugins/diary
may as bob diary write --text "Mine"
may as alice for groups/family add --plugin users/alice/plugins/diary
may as carol for groups/family diary write --text "Ours"
refused as dave for groups/family diary write --text "Theirs"
may as anyone for network add --plugin users/alice/plugins/diary --name journal
may as dave journal write --text "Everyone's"
refused as alice reader read --note users/bob/own/diary
may as bob remove --name diary
refused as bob diary write --text "Mine again"
may as alice diary write --text "Still here"
may as bob add --plugin users/alice/plugins/diary

say "One Core's own tools are whoever's may, decided by the same hooks"
refused as bob deploy --name diary --files-from "$folder/diary"
refused as carol for groups/family remove --name diary
refused as bob for network deploy --name notes --files-from "$folder/notes"
refused as bob for network remove --name relations
refused as bob for network add_server --key "$(printf 'ab%.0s' {1..32})"
refused as bob install_service --plugin inbox --service Inbox.Watcher
may as alice for groups/family remove --name diary

say "Anyone brings a server, which is theirs alone until they choose who else it acts for, and each agrees"
start_for alices users/alice
echo "  a server started for alice says: $(grep -o "This server isn't one of users/alice's yet" "$folder/alices.log")"
refused as bob for users/alice add_server --key "${key[alices]}"
may as alice add_server --key "${key[alices]}"
start_for alices users/alice
grep -q "A server is running" "$folder/alices.log" && echo "  started again, it's running"
may via alices as alice diary write --text "At home"
may via alices as alice journal write --text "With everyone"
refused via alices as bob diary write --text "At alice's"
refused as bob add_server --key "${key[alices]}"
refused as bob add_member --server "${key[alices]}" --who users/bob
refused as bob install_service --plugin load --service Load.Meter --server "${key[alices]}"
if [[ ${#at_address[@]} -gt 0 ]]; then
  echo "  alice's server has no credentials for the bucket, and asks the signer for each of these:"
  (cd "$root/server" && mix run --no-start -e '
    [folder, address | blobs] = System.argv()
    Application.put_env(:one_core, :home, Path.join(folder, "alices"))
    Application.put_env(:one_core, :for, "users/alice")
    Application.put_env(:one_core, :bucket, {OneCore.Bucket.S3, address})
    Application.ensure_all_started(:req)
    for blob <- blobs do
      said = try do
        case OneCore.Bucket.read(blob) do
          {:ok, data, _tag} -> "read, #{byte_size(data)} bytes"
          :absent -> "allowed, and there is none"
        end
      rescue
        refused in OneCore.Bucket.Refused -> String.replace(refused.message, ~r/^.*refused: /, "refused by the signer: ")
      end
      IO.puts("    #{blob}: #{said}")
    end' "$folder" "${at_address[1]}" users/alice/own/diary/1 users/alice/added users/bob/own/diary/1 \
    users/bob/made/notes/list/text network/own/relations/log/0000000000000001 groups/family/own/diary/1 \
    users/bob/added network/plugins/notes/newest 2>&1 | grep -v "^$")
else
  echo "  (on a folder nothing stands between a server and the bucket: with --address, the signer refuses it the rest)"
fi
may as alice add_member --server "${key[alices]}" --who users/bob
may as bob add_server --key "${key[alices]}"
may via alices as bob diary write --text "At alice's"
may as bob remove_server --key "${key[alices]}"
refused via alices as bob diary write --text "At alice's again"

say "A hook that decides refuses a call with its reason"
may as alice notes write --id diary --text "My password is hunter2"
as anyone for network deploy --name gate --files-from "$folder/gate" >/dev/null
echo "  gate is deployed for the network, and decides every call"
refused as alice notes write --id secrets --text "My password is hunter2"
may as alice notes write --id plans --text "Go for a walk"
as anyone for network remove --name gate >/dev/null

say "Claims' approver is whoever a tuple says"
expense=$(as alice claims record --amount 12.5 --note Taxi | grep -o '"id":[0-9]*' | head -1 | cut -d: -f2)
as alice claims gather --expenses "[$expense]" --title Sydney >/dev/null
claim=$(as alice claims list | grep -o '"id":[0-9]*' | head -1 | cut -d: -f2)
as alice claims send --claim "$claim" >/dev/null
echo "  alice sent claim $claim"
refused as bob claims approve --claim "$claim"
refused as alice relations share --artifact network/own/claims --relation editor --who users/alice
may as anyone relations share --artifact network/own/claims --relation editor --who users/bob
said=$(as bob claims approve --claim "$claim" | grep -o '"status":"[a-z_]*"') && echo "  may      as bob claims approve --claim $claim" && echo "           -> $said"

say "What a check costs a call, over one connection as an agent's MCP session keeps"
# Each server has a client that asks it and no other. The server writing the relation tuples' database answers a
# check itself, and the other passes the check on to it. That someone may is kept by the server that asked, so the
# first call of each asks and the rest don't: each line is the median of 300 calls.
# A tool called 300 times as a user through one server, each call timed.
timed() {
  local times=$folder/times-$1-$2-$3 began answer
  coproc MCP { ONE_AS=$2 ONE_HOME=$folder/only-$1 "$one" mcp; }
  for _ in $(seq 300); do
    began=$(now)
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$3\",\"arguments\":$4}}" >&"${MCP[1]}"
    read -r answer <&"${MCP[0]}"
    echo $(($(now) - began)) >>"$times"
  done
  exec {MCP[1]}>&-
  wait "$MCP_PID"
  [[ $answer == *'"result"'* && $answer != *isError* ]] || { echo "$3 as $2 wasn't answered: $answer"; exit 1; }
  median "$times"
}
reads="{\"note\":\"$note\"}"
for server in server spare; do
  if grep -q "own/relations/: holds its lease" "$folder/$server.log"; then where="writes the tuples' database"; else where="passes each check on"; fi
  echo "  through the $server, which $where:"
  # Something else is shared, so what either server kept of who may is forgotten.
  as alice relations share --artifact "$note" --relation viewer --who users/erin >/dev/null
  echo "    $(timed $server alice notes_ping "{}")  a tool that does nothing"
  echo "    $(timed $server alice notes_read "$reads")  alice reads her own note: nothing is asked"
  echo "    $(timed $server bob notes_read "$reads")  bob reads it: one check, of bob, asked once"
  echo "    $(timed $server bob reader_read "$reads")  bob reads it with reader: a check of reader too, asked once"
done

say "A plugin that decides and breaks has refused, for whoever it was added for, and has no say in its own removal"
may as alice deploy --name stuck --files-from "$folder/stuck"
refused as alice notes read --note "$note"
refused as alice remove --name diary
may as bob notes read --note "$note"
may as alice remove --name stuck
may as alice notes read --note "$note"

say "When what broke is what says who may, whoever runs a server for them starts it without that plugin"
may as anyone for network deploy --name lost --files-from "$folder/lost"
refused as bob notes read --note "$note"
may as alice notes read --note "$note"
refused as anyone for network remove --name lost
pkill -9 -f "network $folder server "
(cd "$root/server" && exec mix network "$folder" server --development "${at_address[@]}" --without lost) >"$folder/server.log" 2>&1 &
started+=($!)
grep -q -m1 "A server is running" <(tail -f --pid=$! -n +1 "$folder/server.log")
echo "  the server is killed and started again with --without lost, and says: $(grep -o "lost is removed.*" "$folder/server.log")"
may as bob notes read --note "$note"

say "A scheduled job is called once across a takeover"
# The job is due every 5 seconds, and its call takes a second. The server making the calls is killed in the middle
# of one, so that call wrote nothing.
ticks() { ONE_HOME=$folder/client "$one" ticker ticks | tr '}' '\n' | sed -n 's/.*"at":"\([^"]*\)","due":"\([^"]*\)","server":"\([0-9a-f]*\)".*/\2 \1 \3/p'; }
named() { sed -e "s/${key[server]}/server/" -e "s/${key[spare]}/spare/"; }
# The clock has to pass: until the job has been called a few times, and then until just after it's next due.
sleep 11
sleep "$(awk -v now="$(now)" 'BEGIN { printf "%.3f", (5000000 - now % 5000000 + 400000) / 1000000 }')"
calling=$(ticks | tail -1 | cut -d' ' -f3)
[[ $calling == "${key[server]}" ]] && killed=server || killed=spare
pkill -9 -f "network $folder $killed "
echo "  the $killed, which was making the calls, is killed at $(date -u +%H:%M:%S) with kill -9, in the middle of one"
sleep 16
ticks | named | awk '{ printf "  due %s  called at %s by the %s\n", $1, $2, $3 }'
twice=$(ticks | cut -d' ' -f1 | sort | uniq -d | wc -l)
after=$(ticks | named | awk -v killed="$killed" '$3 != killed' | wc -l)
echo "  $(ticks | wc -l) calls, $twice of them for a time that already had one, and $after by the other server after the kill"
[[ $twice == 0 && $after -ge 2 ]]
