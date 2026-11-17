#!/usr/bin/env bash
# Tests, on a whole network with a standby and its edge on this machine (`tooling/network <folder> --standby --edge`),
# what encryption and the Vault promise, by trying each, for a person, a group and the network. What's typed on a device is encrypted there, and the
# bytes in the bucket hold none of it, for a blob and for a database. A server that wasn't chosen can't open
# either and takes no lease; a chosen one can, and stops when it's taken off. The network's owner writes every
# relation tuple there is and still reads nothing. An agent's tools never answer a secret. A plugin uses an item
# switched on for it, and another is refused. An app signs its person in to an outside service, in a browser, and
# its tokens are kept, used and renewed. A new device is let in by one that holds the key, and with every device
# gone, by the recovery key. It says what a call costs.
#
#     mise exec -- plugins/vault/test/vault.sh
#
# Every secret made here carries a marker, and lives in a file of the test's folder. Nothing prints one: what
# was said by every tool, every log and the whole bucket are searched for the marker at the end.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each server started here.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
"$root/tooling/network" "$folder" --standby --edge >"$folder/network.log" 2>&1 &
network=$!
started=("$network")
# `tooling/network` stops what it started when it's stopped. The bucket's store writes as it stops, so the folder goes
# once everything that was started in it has.
stop() {
  local left
  for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done
  wait 2>/dev/null || true
  for left in $(pgrep -f "$folder" || true); do tail --pid="$left" -f /dev/null; done
  rm -rf "$folder"
}
trap stop EXIT
grep -q -m1 "A network is running" <(tail -F --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
one=$root/target/release/one
(cd "$root" && cargo build --release -p encryption_client --example recovery_key 2>/dev/null)
address=$(grep -o '"address":"[^"]*"' "$folder/client/network" | cut -d'"' -f4)

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
median() { sort -n "$1" | awk '{ all[NR] = $1 } END { printf "%.3f ms", all[int((NR + 1) / 2)] / 1000 }'; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
free_port() { (cd "$root/core/edge" && bun -e 'const server = Bun.serve({ port: 0, fetch: () => new Response() }); console.log(server.port); server.stop()'); }

# The secrets: each a file, whose text starts with the marker.
marker=vault-test-$(random 8)
mkdir "$folder/secrets"
for secret in note github stripe pasted wifi storage; do printf '%s-%s-%s' "$marker" "$secret" "$(random 8)" >"$folder/secrets/$secret"; done
# The one the outside service knows its client by.
printf '%s-client-secret' "$marker" >"$folder/secrets/client"
# How many files under these hold the marker.
holding() { { grep -r -a -l -F -e "$marker" "$@" 2>/dev/null || true; } | wc -l; }

# Each server's key by its name, which is how it's said here.
declare -A key
key_of() { key[$1]=$(grep -o -E "(running in [^ ]*: |add_server --key )[0-9a-f]{64}" "$folder/$1.log" | head -1 | grep -o -E "[0-9a-f]{64}$"); }
key_of server && key_of standby
named() { local names=() name; for name in "${!key[@]}"; do names+=(-e "s/${key[$name]}/<the $name>/g"); done; sed "${names[@]}"; }

# A device is a client with a key of its own, in a home of its own. It's told where the network's servers are, and
# asks whichever answers first, or with `via <server>`, that one and no other.
reached() { sed -e 's/^\[//' -e 's/\]$//' "$folder/only-$1/servers"; }
device() { mkdir -p "$folder/devices/$1" && printf '[%s,%s]' "$(reached server)" "$(reached standby)" >"$folder/devices/$1/servers"; }
# A command on a device, as a user, and for a group or the network if it says: `on phone as alice vault list`,
# `on desk via standby as owner for network deploy ...`. Everything any of them is answered is kept, to search.
on() {
  local home=$folder/devices/$1 only= subject=
  shift
  [[ $1 == via ]] && only=$2 && shift 2
  local who=$2
  shift 2
  [[ $1 == for ]] && subject=$2 && shift 2
  if [[ -n $only ]]; then printf '[%s]' "$(reached "$only")" >"$home/servers"; else printf '[%s,%s]' "$(reached server)" "$(reached standby)" >"$home/servers"; fi
  ${subject:+env ONE_SUBJECT=$subject} env ONE_AS="$who" ONE_HOME="$home" "$one" "${@}" 2>&1 | tee -a "$folder/output"
}
shown() {
  echo "  $1  $(echo "${*:2}" | sed -e "s|$folder|<folder>|g" | named)"
  echo "           -> $(named <<<"$output" | cut -c1-400)"
}
may() { output=$("$@") && shown "may    " "$@" || { echo "this was refused, and shouldn't be: $* -> $output" && exit 1; }; }
refused() { ! output=$("$@") && shown "refused" "$@" || { echo "this was answered, and shouldn't be: $* -> $output" && exit 1; }; }

# What a page at the network's address says once it has drawn, in a browser: its heading and what's under it.
browser() {
  (cd "$root/core/edge" && bun -e '
    import { chromium } from "playwright";
    const browser = await chromium.launch();
    const page = await browser.newPage();
    await page.goto(process.argv[1]);
    await page.locator(".ui-sign-in :is(.done, .problem)").waitFor();
    const at = new URL(page.url());
    console.log(`    ${at.origin}${at.pathname} says: "${(await page.locator(".ui-sign-in :is(h1, .done, .problem)").allTextContents()).join(" / ")}"`);
    await browser.close();
  ' "$1") | tee -a "$folder/output"
}

# A server is stopped by its own process, and started again as `tooling/network` started it.
down() {
  local server
  server=$(pgrep -f "mix network $folder $1 --name $1 ")
  kill -9 "$server"
  tail --pid="$server" -f /dev/null
  echo "  the $1 is stopped, with kill -9"
}
up() {
  (cd "$root/core/server" && exec mix network "$folder" "$1" --name "$1" --development --address "$address") >>"$folder/$1.log" 2>&1 &
  local server=$!
  started+=("$server")
  # It's stopped here by its process, which is nothing for the shell to say.
  disown "$server"
  grep -q -m1 "A server is running" <(tail -f --pid="$server" -n 0 "$folder/$1.log")
  echo "  the $1 is started again"
}

for name in phone laptop tablet desk bobs den attic; do device $name; done
vault=users/alice/made/vault/items
locker=users/alice/made/locker/notes
servers() { local list; list=$(printf '"%s",' "${@/#/}"); echo "[${list%,}]"; }

# The plugins this tests with: a locker that keeps one note encrypted, as a blob and in a database, and plain
# notes, which nothing encrypts.
mkdir -p "$folder/notes/server"
cat >"$folder/notes/server/notes.ex" <<'EOF'
defmodule Notes do
  @behaviour OneCore.Plugin

  @impl true
  def icon, do: "icon.svg"

  @impl true
  def tools do
    text = %{type: "object", properties: %{text: %{type: "string"}}, required: ["text"]}
    [%{name: "write", description: "Writes a note that nothing encrypts", inputs: text, outputs: %{type: "object"}}]
  end

  @impl true
  def call("write", %{"text" => text}, call) do
    OneCore.Artifact.write(OneCore.Call.make(call, "plain"), "note", text)
    %{}
  end
end
EOF
on desk as owner for network deploy --name locker --files-from "$root/plugins/vault/test/plugins/locker" >/dev/null
printf -- '---\nname: %s\ndescription: Read before testing with %s.\nmetadata:\n  category: Utilities\n---\n' notes notes >"$folder/notes/SKILL.md"
cp "$root/plugins/vault/test/plugins/locker/icon.svg" "$folder/notes/"
on desk as owner for network deploy --name notes --files-from "$folder/notes" >/dev/null

say "Alice's key is made on her phone, which then chooses the server to open her Vault and her locker"
refused on phone as alice vault list
may on phone as alice encryption approve
ONE_AS=alice ONE_HOME=$folder/devices/phone "$root/target/release/examples/recovery_key" "$folder/secrets/recovery"
echo "  her phone's window shows her recovery key once, which she writes down ($(wc -c <"$folder/secrets/recovery") characters, kept in a file here)"
may on phone as alice vault grant --servers "$(servers "${key[server]}")"
may on phone as alice locker grant --servers "$(servers "${key[server]}")"
may on phone as alice permissions shared --artifact $vault

say "What's typed on a device is encrypted there, and the bytes in the bucket hold none of it"
may on phone as alice locker keep --text-from "$folder/secrets/note"
may on phone as alice vault set --item GitHub --field token --value-from "$folder/secrets/github" --hosts '["api.github.example"]'
may on phone as alice vault set --item GitHub --field user --value octocat --secret false
may on phone as alice vault list
control=plain-control-$(random 8)
may on phone as alice notes write --text "$control"
echo "  the bucket's blobs of her locker, an encrypted blob and an encrypted database, and of her Vault:"
(cd "$folder/bucket" && find . -type d \( -path "*$locker/*" -o -path "*$vault/*" \) | sed -E -e 's|^\./[^/]*/||' -e 's|/xl\.meta$||' | grep -E '/(note|keys/[^/]+|log/[0-9]+|snapshot/[0-9]+)$' | sort | sed 's/^/    /' | named)
echo "  files in the bucket that hold the marker every secret starts with: $(holding "$folder/bucket")"
echo "  files in the bucket that hold the note nothing encrypted, found the same way: $({ grep -r -a -l -F -e "$control" "$folder/bucket" || true; } | wc -l)"
[[ $(holding "$folder/bucket") == 0 && $({ grep -r -a -l -F -e "$control" "$folder/bucket" || true; } | wc -l) -ge 1 ]]
may on phone as alice locker read
note=$(sha256sum <"$folder/secrets/note" | cut -d' ' -f1)
[[ $output == *"\"blob\":\"$note\""* && $output == *"\"rows\":[\"$note\"]"* ]] && echo "  which is the SHA-256 of the note she typed, both times: the server she chose opened the blob and the row"

say "A server that wasn't chosen can't open them, and doesn't take the database's lease"
may on phone via standby as alice vault list
echo "  the standby passed that on to the server. With the server gone, it has nobody to pass it to:"
down server
refused on phone as alice vault list
refused on phone as alice locker read
echo "  the standby reads the same bytes, as a server of the network reads any, and its own key opens nothing:"
(cd "$root/core/server" && mix run --no-start -e '
  [folder, address, artifact, chosen] = System.argv()
  Application.put_env(:one_core, :home, Path.join(folder, "standby"))
  Application.put_env(:one_core, :bucket, {OneCore.Bucket.S3, address})
  Application.ensure_all_started(:req)
  {:ok, paths} = OneCore.Folder.files("../../plugins/encryption")
  {:ok, _, _} = OneCore.Plugins.Version.load("encryption", Map.new(paths, &{&1, File.read!("../../plugins/encryption/" <> &1)}))
  {:ok, own} = Cipher.key(OneCore.Server.key_file())
  {:ok, standby} = Cipher.public(own)
  {:ok, slot, _tag} = OneCore.Bucket.read("#{artifact}/keys/#{chosen}")
  {:ok, entry, _tag} = OneCore.Bucket.read("#{artifact}/log/0000000000000002")
  IO.puts("    #{artifact}/keys/<the server>: read, #{byte_size(slot)} bytes; opened with the standby'"'"'s key: #{inspect(Cipher.open_keys(own, "one/encryption/1/keys\n#{artifact}\n#{standby}", slot))}")
  IO.puts("    #{artifact}/log/0000000000000002: read, #{byte_size(entry)} bytes; as a log entry: #{try do :erlang.binary_to_term(entry, [:safe]); "read" rescue _ -> "not one it can read" end}")
' "$folder" "$address" "$vault" "${key[server]}" 2>&1 | grep -v "^$")
echo "  log entries of her Vault's database written by the standby: $(grep -c "$vault/: taking its lease" "$folder/standby.log" || true)"
! grep -q "$vault/: taking its lease" "$folder/standby.log"
up server

say "A chosen server can, and stops when it's taken off"
may on phone as alice vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
may on phone as alice locker grant --servers "$(servers "${key[server]}" "${key[standby]}")"
down server
may on phone as alice vault list
may on phone as alice locker read
[[ $output == *"\"blob\":\"$note\""* && $output == *"${key[standby]}"* ]]
echo "  the standby took the lease of her Vault's database: $(grep -o "$vault/: taking its lease at [0-9]*: ok" "$folder/standby.log" | tail -1)"
up server
may on phone as alice vault grant --servers "$(servers "${key[server]}")"
may on phone as alice locker grant --servers "$(servers "${key[server]}")"
may on phone as alice permissions shared --artifact $vault
may on phone as alice vault unlocked_on
down server
refused on phone as alice vault list
refused on phone as alice locker read
up server
may on phone as alice vault set --item GitHub --field note --value "kept after the standby was taken off" --secret false
echo "  what's written since is under a new epoch's key, which the standby was never handed"

say "The network's owner writes any relation tuple, and reads nothing of hers"
refused on desk as owner for users/alice vault list
may on desk as owner permissions share --artifact users/alice --relation member --who users/owner
may on desk as owner permissions share --artifact $vault --relation owner --who users/owner
refused on desk as owner for users/alice vault list
refused on desk as owner for users/alice encryption key
refused on desk as owner for users/alice vault grant --servers "$(servers "${key[standby]}")"
may on desk as owner permissions share --artifact $vault --relation editor --who "servers/${key[standby]}"
echo "  a tuple names the standby now, which the owner wrote. No device of his holds her key, so it was handed none:"
down server
refused on phone as alice vault list
up server
may on desk as owner permissions unshare --artifact $vault --relation editor --who "servers/${key[standby]}"

say "An agent keeps what's pasted, asks its person for the rest, and uses it without seeing it"
service=$(free_port)
(cd "$root/core/edge" && MARKER=$marker exec bun "$root/plugins/vault/test/service.js" "$service") >"$folder/service.log" 2>&1 &
started+=($!)
curl -s --retry 30 --retry-all-errors --retry-delay 1 "http://127.0.0.1:$service/seen" >/dev/null
echo "  an outside service runs on this machine, at 127.0.0.1:$service"
may on laptop as alice vault set --item Pasted --field password --value-from "$folder/secrets/pasted"
response=$(on laptop as alice vault request --title "Stripe key" --fields '{"key":{"secret":true}}' --hosts "[\"127.0.0.1:$service\"]" --why "To read your invoices")
output=$response shown "may    " on laptop as alice vault request --title "Stripe key" --fields '{"key":{"secret":true}}' --why "To read your invoices"
request=$(grep -o '"id":"[^"]*"' <<<"$response" | cut -d'"' -f4)
echo "  she opens the link in a browser:"
browser "$address/vault/request?id=$request"
may on laptop as alice vault status --id "$request"
refused on laptop as alice vault fetch --item "Stripe key" --url "http://127.0.0.1:$service/me"
printf '{"key":"%s"}' "$(cat "$folder/secrets/stripe")" >"$folder/secrets/answer"
may on phone as alice vault answer --id "$request" --values-from "$folder/secrets/answer"
may on laptop as alice vault status --id "$request"
may on laptop as alice vault list
may on laptop as alice vault fetch --item "Stripe key" --url "http://127.0.0.1:$service/echo" --headers '{"x-key":"{{key}}"}'
refused on laptop as alice vault fetch --item "Stripe key" --url "http://127.0.0.1:1/me" --headers '{"x-key":"{{key}}"}'
refused on laptop as alice vault fetch --item Pasted --url "http://127.0.0.1:$service/echo" --headers '{"x-key":"{{password}}"}'

say "A plugin uses an item that's switched on for it, and another is refused"
mkdir -p "$folder/fake/server"
sed "s/PORT/$service/g" "$root/plugins/vault/test/plugins/fake/server/fake.ex" >"$folder/fake/server/fake.ex"
cp "$root/plugins/vault/test/plugins/fake/icon.svg" "$folder/fake/"
for plugin in fake nosy; do
  printf -- '---\nname: %s\ndescription: Read before testing with %s.\nmetadata:\n  category: Utilities\n---\n' $plugin $plugin >"$folder/fake/SKILL.md"
  on desk as owner for network deploy --name $plugin --files-from "$folder/fake" >/dev/null
done
may on phone as alice vault use --item GitHub --plugin fake
may on phone as alice fake digest --item GitHub --field token
[[ $output == *"\"$(sha256sum <"$folder/secrets/github" | cut -d' ' -f1)\""* ]] && echo "  which is the SHA-256 of the token she typed: the plugin's program was handed the secret itself"
refused on phone as alice nosy digest --item GitHub --field token
refused on phone as alice fake digest --item Pasted --field password
may on phone as alice vault use --item GitHub --plugin fake --on false
refused on phone as alice fake digest --item GitHub --field token

say "A plugin keeps a secret it made itself, which only it uses"
may on phone as alice fake keep --item Session
digest=$output
may on phone as alice fake digest --item Session --field key
[[ $output == "$digest" ]] && echo "  which is the SHA-256 the plugin gave as it kept it"
refused on phone as alice nosy digest --item Session --field key
refused on phone as alice nosy keep --item Session
may on phone as alice vault list
may on phone as alice fake forget --item Session
refused on phone as alice fake digest --item Session --field key

say "An app signs its person in to an outside service, and its tokens are kept, used and renewed"
may on phone as alice vault set --item "Fake client" --field client_id --value test-client --secret false --hosts "[\"127.0.0.1:$service\"]"
may on phone as alice vault set --item "Fake client" --field client_secret --value-from "$folder/secrets/client"
link=$(on laptop as alice fake sign_in | grep -o '"url":"[^"]*"' | cut -d'"' -f4)
echo "  may      on laptop as alice fake sign_in"
echo "           -> $(sed -E -e 's/(state|code_challenge)=[^&]*/\1=…/g' <<<"$link")"
echo "  she opens it in her browser, where the service signs her in and sends her back to the network's address:"
browser "$link"
may on laptop as alice vault list
may on laptop as alice fake me
refused on laptop as alice nosy me
echo "  the service has seen: $(curl -s "http://127.0.0.1:$service/seen")"
# Its tokens last two seconds, whatever it said, and the clock has to pass.
sleep 3
may on laptop as alice fake me
echo "  once its token had ended, which the service refused once: $(curl -s "http://127.0.0.1:$service/seen")"
[[ $(curl -s "http://127.0.0.1:$service/seen") == *'"renewed":1'*'"refused":1'*'"without_verifier":0'* ]]
refused on laptop as alice vault fetch --item Fake --url "http://127.0.0.1:$service/me" --headers '{"authorization":"Bearer {{access_token}}"}'
may on phone as alice vault agents --item Fake
may on laptop as alice vault fetch --item Fake --url "http://127.0.0.1:$service/me" --headers '{"authorization":"Bearer {{access_token}}"}'
[[ $output == *'[secret]'* ]] && echo "  the service said her token back, and the answer has it blotted out"

say "A new device is let in by one that holds her key, and with every device gone, by her recovery key"
refused on laptop as alice vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
may on laptop as alice encryption ask
asking=$(on phone as alice encryption key | grep -o '"asking":\[[^]]*\]' | grep -o '[0-9a-f]\{64\}')
key[laptop]=$asking
may on phone as alice encryption approve --device "$asking" --name laptop
may on laptop as alice vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
may on laptop as alice vault grant --servers "$(servers "${key[server]}")"
rm -rf "$folder/devices/phone" "$folder/devices/laptop"
echo "  her phone and her laptop are gone, with their keys. On a tablet she signs in on:"
may on tablet as alice vault list
[[ $output == *GitHub* && $output == *Fake* ]]
may on tablet as alice fake me
refused on tablet as alice vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
printf '%s' "AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH-IIII-JJJJ-KKKK-LLLL-MMMQ" >"$folder/secrets/guess"
refused on tablet as alice encryption approve --key-from "$folder/secrets/guess"
may on tablet as alice encryption approve --key-from "$folder/secrets/recovery"
may on tablet as alice vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
down server
may on tablet as alice vault list
[[ $output == *GitHub* && $output == *Fake* && $output == *Stripe* ]]
up server
echo "  someone with the whole bucket, and a key of their own making:"
(cd "$root/core/server" && mix run --no-start -e '
  [folder, address, artifact | chosen] = System.argv()
  Application.put_env(:one_core, :home, Path.join(folder, "standby"))
  Application.put_env(:one_core, :bucket, {OneCore.Bucket.S3, address})
  Application.ensure_all_started(:req)
  slots = for server <- chosen, {:ok, slot, _tag} <- [OneCore.Bucket.read("#{artifact}/keys/#{server}")], do: slot
  theirs = Path.join(folder, "stranger/key")
  {:ok, _key} = OneCore.Iroh.key(theirs)
  {:ok, paths} = OneCore.Folder.files("../../plugins/encryption")
  {:ok, _, _} = OneCore.Plugins.Version.load("encryption", Map.new(paths, &{&1, File.read!("../../plugins/encryption/" <> &1)}))
  {:ok, own} = Cipher.key(theirs)
  {:ok, stranger} = Cipher.public(own)
  opened = for slot <- slots, match?({:ok, _keys}, Cipher.open_keys(own, "one/encryption/1/keys\n#{artifact}\n#{stranger}", slot)), do: slot
  IO.puts("    of the #{length(slots)} copies of her Vault'"'"'s keys in the bucket, each encrypted to a server she chose, theirs opens #{length(opened)}")
' "$folder" "$address" "$vault" "${key[server]}" "${key[standby]}" 2>&1 | grep -v "^$")
echo "  and no file of the bucket holds the marker, as below"

say "A group's Vault and the network's, each set up and opened by an owner, and the network's by its recovery key"
may on den as alice permissions group --name family
may on den as alice permissions share --artifact groups/family --relation member --who users/bob
may on den as alice for groups/family encryption approve
may on den as alice for groups/family vault grant --servers "$(servers "${key[server]}")"
may on den as alice for groups/family vault set --item Wifi --field password --value-from "$folder/secrets/wifi"
may on den as alice for groups/family vault list
[[ $output == *Wifi* ]]
refused on bobs as bob for groups/family vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
refused on bobs as bob for groups/family encryption approve
may on den as owner for network encryption approve
ONE_AS=owner ONE_SUBJECT=network ONE_HOME=$folder/devices/den "$root/target/release/examples/recovery_key" "$folder/secrets/network-recovery"
echo "  the den's window shows the network's recovery key once, which its owner writes down"
may on den as owner for network vault grant --servers "$(servers "${key[server]}")"
may on den as owner for network vault set --item Storage --field secret_key --value-from "$folder/secrets/storage"
rm -rf "$folder/devices/den"
echo "  the den is gone, with its keys. On the attic, the network's owner:"
refused on attic as owner for network vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
may on attic as owner for network encryption approve --key-from "$folder/secrets/network-recovery"
may on attic as owner for network vault grant --servers "$(servers "${key[server]}" "${key[standby]}")"
may on attic as owner for network vault list
[[ $output == *Storage* && $output != *Wifi* && $output != *GitHub* ]]

say "No secret was answered, logged or stored anywhere but encrypted"
# The chosen servers each keep a working copy of a database they write, on their own disk, which is the one place
# outside the bucket a secret is: everything else of the network is searched. The secrets' own files are the test's.
everything=$(find "$folder" -type f -not -path "$folder/secrets/*" -not -path "*/database/copy.sqlite*")
echo "  what every tool answered every device, here: $(wc -l <"$folder/output") lines, of which $({ grep -c -F -e "$marker" "$folder/output" || true; }) hold the marker"
echo "  the servers' logs, the edge's and the service's: $(cat "$folder"/*.log | wc -l) lines, of which $({ cat "$folder"/*.log | grep -c -a -F -e "$marker" || true; }) hold the marker"
echo "  every file of the network (its bucket, its edge's storage, its servers' and its clients' homes): $(wc -l <<<"$everything"), of which $(xargs -d '\n' grep -a -l -F -e "$marker" <<<"$everything" 2>/dev/null | wc -l) hold the marker"
[[ $(xargs -d '\n' grep -a -l -F -e "$marker" <<<"$everything" 2>/dev/null | wc -l) == 0 ]]
echo "  the working copies a chosen server keeps of the databases it writes, on its own disk: $(find "$folder" -path "*/database/copy.sqlite" | sed "s|$folder/||" | paste -sd' ')"

say "What a call costs, over one connection as an agent's MCP session keeps"
# A tool called 300 times as alice through the server, each call timed: each line is the median.
timed() {
  local times=$folder/times-$1 began response
  printf '[%s]' "$(reached server)" >"$folder/devices/tablet/servers"
  coproc MCP { ONE_AS=alice ONE_HOME=$folder/devices/tablet "$one" mcp; }
  for _ in $(seq 300); do
    began=$(now)
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" >&"${MCP[1]}"
    read -r response <&"${MCP[0]}"
    echo $(($(now) - began)) >>"$times"
  done
  exec {MCP[1]}>&-
  wait "$MCP_PID"
  [[ $response == *'"result"'* && $response != *isError* ]] || { echo "$1 wasn't answered"; exit 1; }
  median "$times"
}
echo "    $(timed locker_ping "{}")  a tool that does nothing"
echo "    $(timed vault_list "{}")  the Vault's list: an encrypted database, which the server holds open"
echo "    $(timed fake_digest '{"item":"Fake","field":"access_token"}')  a plugin is handed one of an item's secrets, and runs a program on it"
echo "    $(timed locker_read "{}")  an encrypted blob read from the bucket, through the presigner, and a row of an encrypted database"
