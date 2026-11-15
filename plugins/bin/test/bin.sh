#!/usr/bin/env bash
# Tests the bin end to end, on a whole network on this machine (`./network <folder>`), its bucket a folder:
# alice removes a repo, and her locker, which is an encrypted blob and an encrypted database. Each is gone from
# every listing and from the bucket at its name, bob, who reads the repo, can neither remove nor restore it, and
# each comes back as it was: the repo clones with its history, and the locker opens on the server she chose, with
# its row. Then the clock says 30 days have passed, and what was removed is erased from the bucket for good, while
# what was removed since stays.
#
#     cargo build --release && mise exec -- plugins/bin/test/bin.sh
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
"$root/network" "$folder" >"$folder/network.log" 2>&1 &
network=$!
trap 'kill -- "-$network" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -f --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }
(cd "$root" && cargo build --release -p encryption_client --example recovery_key 2>/dev/null)

export PATH=$root/target/release:$PATH GIT_CONFIG_GLOBAL=$folder/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --global init.defaultBranch main
git config --global user.name "Alice Smith" && git config --global user.email alice@example.com
trap 'echo "failed at line $LINENO: $BASH_COMMAND"' ERR
cd "$folder"

say() { printf '\n== %s\n' "$*"; }
# A command as someone, and for a group or the network if it says: `as alice bin list`, `as owner for network ...`.
as() {
  local who=$1 asked=
  shift
  [[ $1 == for ]] && asked=$2 && shift 2
  ${asked:+env ONE_FOR=$asked} env ONE_AS="$who" ONE_HOME="$folder/client" "$@"
}
may() { said=$("$@" 2>&1) && echo "  may      $* -> $(cut -c1-300 <<<"$said")" || { echo "this was refused, and shouldn't be: $* -> $said" && exit 1; }; }
refused() { if said=$("$@" 2>&1); then echo "this should have been refused: $* -> $said" && exit 1; else echo "  refused  $* -> $(grep -v '^fatal: unable\|^error: failed\|^To one' <<<"$said" | head -2 | tr '\n' ' ' | cut -c1-300)"; fi; }
# The blobs in the bucket under a prefix.
blobs() { if [[ -d $folder/bucket/$1 ]]; then find "$folder/bucket/$1" -type f | wc -l; else echo 0; fi; }
server=$(grep -o -E "running in [^ ]*: [0-9a-f]{64}" "$folder/server.log" | head -1 | grep -o -E "[0-9a-f]{64}$")
repo=users/alice/made/repos/notes
locker=users/alice/made/locker/notes

say "alice has a repo, which bob reads, and a locker, encrypted on the server she chose"
as owner for network one deploy --name locker --files-from "$root/plugins/vault/test/plugins/locker" >/dev/null
may as alice one repos make --name notes
as alice git clone -q one::$repo notes 2>&1
echo "# Notes" >notes/README.md && git -C notes add -A && git -C notes commit -qm "Start the notes"
echo "- buy milk" >>notes/README.md && git -C notes commit -qam "Buy milk"
as alice git -C notes push -q -u origin main 2>&1
may as alice one relations share --artifact $repo --relation viewer --who users/bob
may as alice one encryption let_in
as alice "$root/target/release/examples/recovery_key" "$folder/recovery" >/dev/null
may as alice one locker grant --servers "[\"$server\"]"
printf 'the note in her locker' >"$folder/note"
may as alice one locker keep --text-from "$folder/note"
may as alice one locker read
read_before=$said
echo "  in the bucket: $(blobs $repo) blobs of the repo, $(blobs $locker) of the locker"

say "Only whoever owns them removes them"
refused as bob one repos remove --repo $repo
refused as bob one bin remove --artifact $locker
refused as carol one bin remove --artifact $repo

say "alice removes both, one through Repos and one straight in the bin"
may as alice one repos remove --repo $repo
may as alice one bin remove --artifact $locker
may as alice one repos list
[[ $said != *notes* ]] && echo "  her repo isn't listed"
may as bob one repos list
[[ $said != *notes* ]] && echo "  nor for bob"
refused as alice git clone -q one::$repo gone
refused as bob one repos log --repo $repo
refused as alice one locker read
echo "  in the bucket at their names: $(blobs $repo) blobs of the repo, $(blobs $locker) of the locker; in the bin: $(blobs users/alice/made/bin)"
[[ $(blobs $repo) == 0 && $(blobs $locker) == 0 ]]
may as alice one bin list

say "Whoever couldn't remove them can't restore them, or see them in the bin"
refused as bob one repos restore --repo $repo
refused as bob one bin restore --artifact $locker
may as bob one bin list
[[ $said == "[]" ]]

say "alice brings both back, as they were"
may as alice one bin restore --artifact $repo
may as alice one repos list
[[ $said == *notes* ]] && echo "  her repo is listed again, though it came back through the bin and not Repos"
as alice git clone -q one::$repo back 2>&1
echo "  cloned: $(git -C back log --format=%s | tr '\n' ',') $(git -C back fsck --no-progress 2>&1 | wc -l) problems"
[[ $(git -C back rev-parse HEAD) == $(git -C notes rev-parse HEAD) ]]
may as bob one repos log --repo $repo
refused as alice one repos restore --repo $repo
may as alice one bin restore --artifact $locker
may as alice one locker read
[[ $said == "$read_before" && $said == *"\"rows\":[\""* && $said == *"$server"* ]] &&
  echo "  the same note as a blob and as its database's row, opened on the server she chose"
may as alice one locker keep --text-from "$folder/note"
echo "  and its database takes writes again"

say "After 30 days, what was removed is erased for good, and what was removed since isn't"
may as alice one repos remove --repo $repo
may as alice one bin remove --artifact $locker
may as alice one repos make --name later
may as alice one repos remove --repo users/alice/made/repos/later
# The clock: what was removed says it was removed 31 days ago.
for line in "$folder/bucket/network/own/bin/removed/"*; do
  grep -q -e "$repo\"" -e "$locker\"" "$line" || continue
  python3 -I -c 'import json, sys; line = json.load(open(sys.argv[1])); line["at"] -= 31 * 86400000; json.dump(line, open(sys.argv[1], "w"))' "$line"
done
may as owner for network one bin erase
may as alice one bin list
[[ $said == *later* && $said != *"$repo\""* && $said != *"$locker\""* ]]
echo "  in the bucket: $(blobs $repo) blobs of the repo, $(blobs $locker) of the locker, $(blobs users/alice/made/bin) in the bin (the later repo's)"
[[ $(blobs $repo) == 0 && $(blobs $locker) == 0 && $(blobs users/alice/made/bin) -gt 0 ]]
refused as alice one bin restore --artifact $repo
refused as alice one repos restore --repo $repo
may as alice one repos make --name notes
echo "  its name is free again"
