#!/usr/bin/env bash
# Proves, on a whole network on this machine (`./network <folder>`), what backups promise, by doing each. A network
# with two people, a group, databases with rows, files and a Vault item is backed up to a folder, through the
# service on its machine, and to S3-compatible storage (a local S3 store, as `./network --edge` runs one),
# while writes go on. What changed is backed up again, and only that is sent. Then the bucket is destroyed, and
# the network restored into an empty one from each place with nothing but the recovery key, a server started on
# it, and everything is back and the Vault opens. A file deleted before the last backup is brought back into the
# running network. Another key restores nothing, and neither place holds a name or the marker. A check reads a
# backup back whole, and finds the one object that was changed, which the next backup sends again. It says how big
# and how long each step was.
#
#     mise exec -- server/test/backups.sh
set -euo pipefail
set -m # The network is a job with a process group of its own, and so is each server started here.

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
(cd "$root" && cargo build --release -p client --example recovery_key 2>/dev/null)

say() { printf '\n== %s\n' "$*"; }
random() { od -An -tx1 -N"$1" /dev/urandom | tr -d ' \n'; }
free_port() { (cd "$root/edge" && bun -e 'const server = Bun.serve({ port: 0, fetch: () => new Response() }); console.log(server.port); server.stop()'); }
size() { du -sb "$1" | awk '{ printf "%.1f MB", $1 / 1e6 }'; }
shown() { sed -e "s|$folder|<folder>|g" | cut -c1-400 | sed 's/^/    /'; }
# A command as a user, and for a group or the network if it says, on the network's client: `as alice shelf all`,
# `as owner for network backups now`.
as() {
  local who=$1 asked=
  shift
  [[ $1 == for ]] && asked=$2 && shift 2
  ${asked:+env ONE_FOR=$asked} env ONE_AS="$who" ONE_HOME="$folder/client" "$one" "$@"
}
backups() { as owner for network backups "$@"; }
# What each of them keeps, as one line to compare.
everything() { for who in alice bob "alice for groups/family"; do as $who shelf all 2>/dev/null; echo; done; }

# The server is stopped by its own process, and started again as `./network` started it.
server() { pgrep -f "mix network $folder server --name server " || true; }
down() {
  local server
  server=$(server)
  kill "$server"
  tail --pid="$server" -f /dev/null
}
up() {
  (cd "$root/server" && exec mix network "$folder" server --name server --development --owner owner) >>"$folder/server.log" 2>&1 &
  local server=$!
  started+=("$server")
  disown "$server"
  grep -q -m1 "A server is running" <(tail -f --pid="$server" -n 0 "$folder/server.log")
}

marker=backups-proof-$(random 8)
mkdir "$folder/secrets"
printf '%s-vault' "$marker" >"$folder/secrets/vault"
holding() { { grep -r -a -l -F -e "$marker" "$@" 2>/dev/null || true; } | wc -l; }

say "A network with two people, a group, databases with rows, files and a Vault item"
as owner for network deploy --name shelf --files-from "$root/server/test/plugins/shelf" | shown
as alice relations group --name family | shown
as alice relations share --artifact groups/family --relation member --who users/bob | shown
for n in 1 2 3; do as alice shelf add --text "alice's note $n" >/dev/null; done
as alice shelf put --name diary.txt --text "$marker, in a file nothing encrypts" >/dev/null
as alice shelf put --name photo.jpg --size 2000000 >/dev/null
as alice shelf put --name letter.pdf --size 300000 >/dev/null
as bob shelf add --text "bob's note" >/dev/null
as bob shelf put --name song.mp3 --size 1000000 >/dev/null
as alice for groups/family shelf add --text "the family's note" >/dev/null
as alice for groups/family shelf put --name holiday.mov --size 3000000 >/dev/null
as alice encryption let_in | shown
as alice vault grant --servers "[\"$(ls "$folder/bucket/servers")\"]" | shown
as alice vault set --item Marker --field secret --value-from "$folder/secrets/vault" | shown
echo "  the bucket: $(size "$folder/bucket"), $(find "$folder/bucket" -type f -not -path '*/.*' | wc -l) blobs, of which $(holding "$folder/bucket") hold the marker: alice's diary.txt, which nothing encrypts"

say "The network's key, whose recovery key restores, and the two places"
as owner encryption let_in | shown
as owner for network encryption let_in | shown
ONE_AS=owner ONE_FOR=network ONE_HOME=$folder/client "$root/target/release/examples/recovery_key" "$folder/secrets/network"
ONE_AS=owner ONE_HOME=$folder/client "$root/target/release/examples/recovery_key" "$folder/secrets/owner"
echo "  the owner's window shows the network's recovery key once, which they write down (kept in a file here)"
mkdir "$folder/disk"
as owner for network install_service --plugin backups --service Backups.Disk | shown
backups add --folder "$folder/disk" --name Disk | shown
# The S3-compatible storage: a local store, with keys made up for this run, which only the network's Vault holds.
s3=127.0.0.1:$(free_port) access=$(random 8) secret=$(random 24)
mkdir "$folder/s3"
RUSTFS_ACCESS_KEY=$access RUSTFS_SECRET_KEY=$secret rustfs server --address "$s3" "$folder/s3" >"$folder/s3.log" 2>&1 &
started+=($!)
curl -s -o /dev/null --retry 60 --retry-all-errors --retry-delay 1 "http://$s3/"
(cd "$root/edge" && ACCESS=$access SECRET=$secret BUCKET=http://$s3/backups bun -e '
  import { AwsClient } from "aws4fetch";
  const { ACCESS: accessKeyId, SECRET: secretAccessKey, BUCKET } = process.env;
  const made = await new AwsClient({ accessKeyId, secretAccessKey, service: "s3", region: "auto" }).fetch(BUCKET, { method: "PUT" });
  if (!made.ok) throw new Error(`the store answered ${made.status}`);')
printf 'ACCESS_KEY_ID=%s\nSECRET_ACCESS_KEY=%s\n' "$access" "$secret" >"$folder/secrets/s3"
printf '%s' "$secret" >"$folder/secrets/s3-secret"
as owner for network vault grant --servers "[\"$(ls "$folder/bucket/servers")\"]" | shown
as owner for network vault set --item "Backup storage" --field access_key_id --value "$access" --secret false --hosts "[\"$s3\"]" | shown
as owner for network vault set --item "Backup storage" --field secret_access_key --value-from "$folder/secrets/s3-secret" | shown
as owner for network vault use --item "Backup storage" --plugin backups | shown
backups add --endpoint "http://$s3" --bucket backups/network --item "Backup storage" --name Storage | shown
as alice for network backups places 2>&1 | shown || echo "    (alice, who doesn't own the network, was refused)"

say "A backup to each, while bob writes a note after note"
(for n in $(seq 1 300); do ONE_AS=bob ONE_HOME=$folder/client "$one" shelf add --text "written meanwhile $n" >/dev/null; done) &
writing=$!
backups now | shown
wait $writing
echo "  bob wrote 300 notes meanwhile. The places: Disk $(size "$folder/disk"), Storage $(size "$folder/s3/backups")"
first=$(backups places 2>/dev/null | grep -o '"name":"[0-9]\{16\}"' | head -1 | cut -d'"' -f4)

say "Things change and go, and the next backup sends only what changed"
as alice shelf delete --name letter.pdf | shown
as alice shelf change --text "alice's note 2" --to "alice's note 2, changed" | shown
as bob shelf put --name voice.m4a --size 200000 | shown
as alice for groups/family shelf add --text "the family's second note" | shown
backups now | shown
before=$(everything)
echo "  what each of them keeps now, which the restore from the disk is compared with:"
shown <<<"$before"

say "One deleted file is brought back into the running network"
as alice shelf all | grep -q letter.pdf || echo "  alice's letter.pdf is gone, deleted before the last backup"
backups restore --place Disk --backup "$first" --blobs users/alice/made/shelf/files/letter.pdf | shown
as alice shelf all | grep -q letter.pdf && echo "  and it's back"
backups restore --place Disk --backup "$first" --blobs users/alice/ | shown

say "The bucket is destroyed, and restored from the disk with the recovery key alone"
down
rm -rf "$folder/bucket"
echo "  the server is stopped and the bucket is gone: $(ls "$folder" | grep -c '^bucket$' || true) buckets"
mkdir "$folder/bucket"
(time -p "$one" restore --from "$folder/disk" --to "$folder/bucket" --key-from "$folder/secrets/network") 2>&1 | grep -v '^user\|^sys' | shown
up
echo "  the server is started on it: $(size "$folder/bucket")"
[[ $(everything) == "$before" ]] && echo "  every row and file of alice's, bob's and the family's is as it was at the last backup"
as alice vault list | shown
as alice vault list | grep -q '"Marker"' && echo "  alice's Vault opens, with her item in it"
echo "  the places came back with it: $(backups places | grep -o '"name":"\(Disk\|Storage\)"' | cut -d'"' -f4 | paste -sd' ')"

say "Destroyed again, and restored from S3-compatible storage, as the first backup there was, made while bob wrote"
down
rm -rf "$folder/bucket"
mkdir "$folder/bucket"
oldest=$("$one" restore --from "http://$s3/backups/network" --from-keys "$folder/secrets/s3" --list | head -1)
(time -p "$one" restore --from "http://$s3/backups/network" --from-keys "$folder/secrets/s3" --backup "$oldest" --to "$folder/bucket" --key-from "$folder/secrets/network") 2>&1 | grep -v '^user\|^sys' | shown
up
meanwhile=$(as bob shelf all | grep -o 'written meanwhile [0-9]*' | grep -o '[0-9]*$' | paste -sd' ')
echo "  bob's notes written meanwhile, as that backup has them: ${meanwhile:0:60}..."
[[ $meanwhile == "$(seq -s' ' 1 "$(wc -w <<<"$meanwhile")")" ]] && echo "  the first $(wc -w <<<"$meanwhile") of the 300, with none missing: his database as it was at one moment"
as alice shelf all | grep -q letter.pdf && echo "  and alice's letter.pdf, which she deleted after it"
as alice vault list | grep -q '"Marker"' && echo "  alice's Vault opens, with her item in it"

say "Another key restores nothing"
mkdir "$folder/empty"
"$one" restore --from "$folder/disk" --to "$folder/empty" --key-from "$folder/secrets/owner" 2>&1 | shown || true
printf 'AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH-IIII-JJJJ-KKKK-LLLL-MMMQ' >"$folder/secrets/guess"
"$one" restore --from "http://$s3/backups/network" --from-keys "$folder/secrets/s3" --to "$folder/empty" --key-from "$folder/secrets/guess" 2>&1 | shown || true
echo "  files in the bucket they'd have gone to: $(find "$folder/empty" -type f | wc -l)"
[[ $(find "$folder/empty" -type f | wc -l) == 0 ]]

say "Whoever holds a place reads nothing there"
echo "  files in the bucket that hold the marker: $(holding "$folder/bucket"). At the places:"
for place in disk s3; do
  named=$({ grep -r -a -l -E -e "users/|groups/|shelf|photo\.jpg|letter\.pdf|Marker" "$folder/$place" 2>/dev/null || true; } | wc -l)
  echo "    $place: $(find "$folder/$place" -type f | wc -l) files, of which $(holding "$folder/$place") hold the marker and $named a blob's name; $(find "$folder/$place" | grep -c -E 'users|groups|shelf|\.jpg|\.pdf' || true) paths name one"
  [[ $(holding "$folder/$place") == 0 && $named == 0 ]]
done
echo "  a file's name at the disk: $(find "$folder/disk/objects" -type f -print -quit | sed "s|$folder|<folder>|")"

say "A check reads the newest backup this network knows of back whole, and finds the one object that was changed"
backups check | shown
object=$(find "$folder/disk/objects" -type f -print -quit)
printf 'x' | dd of="$object" bs=1 seek=40 conv=notrunc 2>/dev/null
echo "  one byte of $(basename "$object") at the disk is changed"
backups check --place Disk 2>&1 | shown || true
backups places | grep -o '"problem":[^}]*}' | head -1 | shown || true
echo "  what was changed is gone from the disk, and the next backup sends it again:"
backups now --place Disk | shown
backups check --place Disk | shown
