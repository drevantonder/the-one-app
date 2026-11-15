#!/usr/bin/env bash
# Proves, on a network on this machine (`./network <folder>`), what AI Gateway promises, by trying each against a
# fake model server (core/test/ai-gateway/fake.js) standing in for each kind of place: LM Studio on this machine,
# a host that speaks OpenAI's chat completions, and Workers AI. A model on the person's own machines answers when
# nothing is set; a job's list is tried in order, and the next answers when one is off; text, an image, a structured
# answer, an error and a slow answer each come back in one shape. The hosts' key is a Vault item, and it's never in
# an answer, a log or the bucket.
#
#     mise exec -- core/test/ai-gateway.sh
#
# Port 1234, LM Studio's own, has to be free. A model server already running here (Ollama, say) is listed too, and
# with nothing set it may be the one that answers.
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each fake.

root=$(cd "$(dirname "$0")/../.." && pwd)
folder=$(mktemp -d)
"$root/network" "$folder" >"$folder/network.log" 2>&1 &
started=($!)
stop() {
  local left
  for job in "${started[@]}"; do kill -- "-$job" 2>/dev/null || true; done
  wait 2>/dev/null || true
  for left in $(pgrep -f "$folder" || true); do tail --pid="$left" -f /dev/null; done
  rm -rf "$folder"
}
trap stop EXIT
grep -q -m1 "A network is running" <(tail -f --pid="${started[0]}" -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
one=$root/target/release/one
server=$(grep -o -E "running in [^ ]*: [0-9a-f]{64}" "$folder/server.log" | head -1 | grep -o -E "[0-9a-f]{64}$")

say() { printf '\n== %s\n' "$*"; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
free_port() { bun -e 'const server = Bun.serve({ port: 0, fetch: () => new Response() }); console.log(server.port); server.stop()'; }

# The hosts' key carries a marker, and lives in a file. Nothing prints it: everything is searched for it at the end.
marker=ai-gateway-proof-$(random 8)
mkdir "$folder/secrets"
printf '%s-%s' "$marker" "$(random 8)" >"$folder/secrets/key"
holding() { { grep -r -a -l -F -e "$marker" "$@" 2>/dev/null || true; } | wc -l; }

# Alice's phone: a client of her own, which holds her key.
mkdir -p "$folder/phone" && cp "$folder/only-server/servers" "$folder/phone/servers"
on() { ONE_AS=alice ONE_HOME=$folder/phone "$one" "$@" 2>&1 | tee -a "$folder/said"; }
as_owner() { ONE_AS=owner ONE_FOR=network ONE_HOME=$folder/client "$one" "$@" 2>&1 | tee -a "$folder/said"; }
shown() { echo "  $1  ${*:2}" && echo "           -> $(sed "s|$folder|<folder>|g" <<<"$said" | cut -c1-400)"; }
may() { said=$("$@") && shown "may    " "$@" || { echo "this was refused, and shouldn't be: $* -> $said" && exit 1; }; }
refused() { ! said=$("$@") && shown "refused" "$@" || { echo "this was answered, and shouldn't be: $* -> $said" && exit 1; }; }
answered_by() { [[ $said == *"\"model\":\"$1\""* ]] || { echo "that wasn't answered by $1" && exit 1; }; }

fake() { (KEY=$(cat "$folder/secrets/key") SLOW=20 exec bun "$root/core/test/ai-gateway/fake.js" "$1") >>"$folder/fake-$1.log" 2>&1 & started+=($!) && fakes[$1]=$!; }
declare -A fakes
hosted=$(free_port)
fake 1234
fake "$hosted"
curl -s -o /dev/null --retry 30 --retry-all-errors --retry-delay 1 "http://127.0.0.1:1234/seen"
curl -s -o /dev/null --retry 30 --retry-all-errors --retry-delay 1 "http://127.0.0.1:$hosted/seen"
echo "LM Studio's stand-in is at 127.0.0.1:1234; the host's and Workers AI's at 127.0.0.1:$hosted"

# A picture of a receipt, as a phone sends one: a small PNG.
image=$(python3 -I -c '
import base64, struct, zlib
w, h = 48, 64
raw = b"".join(b"\x00" + bytes([250, 250, 240] * w) for _ in range(h))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
print(base64.b64encode(png).decode())')
bytes=$(base64 -d <<<"$image" | wc -c)
looking='[{"role":"user","content":[{"type":"text","text":"What is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,'$image'"}}]}]'
hello='[{"role":"user","content":"Hello"}]'
schema='{"type":"object","properties":{"vendor":{"type":"string"},"total":{"type":"number"}},"required":["vendor","total"]}'

say "A machine of hers runs AI Gateway's service, which lists what its model servers have"
may as_owner install_service --plugin ai-gateway --service AiGateway.Local --server "$server"
may on ai-gateway models
[[ $said == *'"id":"local/fake-vision"'*'"vision":true'* && $said != *fake-embedding* ]]
echo "  LM Studio's stand-in's two chat models are there, and its embedding model isn't"

say "With nothing set, a model on her own machines answers, and never a hosted one"
may on ai-gateway chat --messages "$hello"
[[ $said == *'"model":"local/'* && $said == *'"runs_on":"server"'* ]]
may on ai-gateway chat --messages "$looking"
[[ $said == *'"model":"local/'* ]]

say "A host and Workers AI are Vault items of hers, switched on for AI Gateway"
may on encryption let_in
may on vault grant --servers "[\"$server\"]"
may on vault set --item "Fake Cloud" --field url --value "http://127.0.0.1:$hosted/hosted/v1" --secret false --hosts "[\"127.0.0.1:$hosted\"]"
may on vault set --item "Fake Cloud" --field api_key --value-from "$folder/secrets/key"
may on vault set --item "Workers AI" --field url --value "http://127.0.0.1:$hosted/client/v4/accounts/proof/ai/v1" --secret false --hosts "[\"127.0.0.1:$hosted\"]"
may on vault set --item "Workers AI" --field api_key --value-from "$folder/secrets/key"
may on ai-gateway models
[[ $said != *'"name":"Fake Cloud"'* ]] && echo "  nothing's switched on for AI Gateway yet, so neither is listed"
may on vault use --item "Fake Cloud" --plugin ai-gateway
may on vault use --item "Workers AI" --plugin ai-gateway
may on ai-gateway models
[[ $said == *'"id":"Fake Cloud/cloud-vision","name":"cloud-vision","structured":true,"vision":true'* ]]
[[ $said == *'"id":"Workers AI/@cf/fake/vision"'* && $said != *painter* ]]
echo "  each says what it does: Workers AI's painter, which makes no text, isn't one"
may on ai-gateway chat --messages "$hello"
[[ $said == *'"model":"local/'* ]] && echo "  and with nothing set, her own machine still answers"

say "A job tries its models in order: text, an image and a structured answer, in one shape"
may on ai-gateway set --job vision --models '["Fake Cloud/cloud-vision", "local/fake-vision"]'
may on ai-gateway chat --messages "$looking"
answered_by "Fake Cloud/cloud-vision"
[[ $said == *"An image of $bytes bytes"* ]] && echo "  the host was sent the whole picture ($bytes bytes)"
may on ai-gateway chat --model "Workers AI/@cf/fake/vision" --messages "$looking" --schema "$schema"
[[ $said == *'"json":{"total":12.5,"vendor":"Proof Cafe"}'* ]]
may on ai-gateway chat --model local/fake-text --messages "$hello"
answered_by local/fake-text

say "An error is said, with the key it said back blotted out; a slow answer is waited for"
refused on ai-gateway chat --model "Fake Cloud/broken" --messages "$hello"
[[ $said == *'[secret]'* ]] && echo "  the host said her key back, and the answer has it blotted out"
began=$SECONDS
may on ai-gateway chat --model "Fake Cloud/slow" --messages "$hello"
answered_by "Fake Cloud/slow"
echo "  it took $((SECONDS - began)) seconds"

say "When the first is off, the next answers"
may on ai-gateway set --job chat --models '["local/fake-text", "Fake Cloud/cloud-text"]'
may on ai-gateway chat --messages "$hello"
answered_by local/fake-text
kill -- "-${fakes[1234]}" && wait "${fakes[1234]}" 2>/dev/null || true
echo "  LM Studio's stand-in is stopped"
may on ai-gateway chat --messages "$hello"
answered_by "Fake Cloud/cloud-text"
may on ai-gateway models
may on ai-gateway set --job vision --models '[]'

say "Everyone's list is the network's, and hers comes first"
may as_owner ai-gateway set --job summary --models '["local/fake-text"]'
may on ai-gateway models
python3 -I -c 'import json, sys; jobs = {j["job"]: j["whose"] for j in json.loads(sys.argv[1])["jobs"]}; assert jobs == {"chat": "yours", "summary": "everyone'"'"'s"}, jobs' "$said"

say "Her key was sent, and is nowhere it shouldn't be"
seen=$(curl -s "http://127.0.0.1:$hosted/seen")
echo "  the host had it right $(grep -o '"right":[0-9]*' <<<"$seen" | cut -d: -f2) times, and wrong $(grep -o '"wrong":[0-9]*' <<<"$seen" | cut -d: -f2)"
[[ $seen == *'"wrong":0'* ]]
echo "  files holding it, of every answer, every log and the bucket: $(holding "$folder/said" "$folder"/*.log "$folder/bucket")"
[[ $(holding "$folder/said" "$folder"/*.log "$folder/bucket") == 0 ]]
echo
echo "AI Gateway did all it promises."
