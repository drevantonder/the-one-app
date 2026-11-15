#!/usr/bin/env bash
# Proves Repos with real git, on a whole network with a spare and its edge on this machine (`./network <folder>
# --spare --edge`), whose bucket is an S3 store reached only through the signer there. As alice, bob and carol,
# and the group family: alice clones, commits, pushes and fetches from two devices; a 50 MB repo with history is
# pushed, then one small commit, with what each wrote to the bucket; two pushes to one branch at once, through the
# two servers, where one wins and the other is told to fetch, and nothing is lost; carol is refused clone and push;
# bob suggests changes, which alice takes and declines; bob pushes to the family's repo as one of its members; a
# server is killed in the middle of a push, which is pushed again through the spare and lands once; and the spare
# serves what the first server took. It says how long each took.
#
#     cargo build --release && mise exec -- plugins/repos/test/repos.sh
set -euo pipefail
set -m # The network is a job with a process group of its own.

root=$(cd "$(dirname "$0")/../../.." && pwd)
folder=$(mktemp -d)
"$root/network" "$folder" --spare --edge >"$folder/network.log" 2>&1 &
network=$!
trap 'kill -- "-$network" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$folder"' EXIT
grep -q -m1 "A network is running" <(tail -f --pid=$network -n +1 "$folder/network.log") || { cat "$folder/network.log"; exit 1; }

# `one` and git as the proof's people: git finds `git-remote-one` beside `one`, and reads only the config here.
export PATH=$root/target/release:$PATH GIT_CONFIG_GLOBAL=$folder/gitconfig GIT_CONFIG_NOSYSTEM=1
git config --global init.defaultBranch main
git config --global advice.detachedHead false
git config --global user.name "Alice Smith" && git config --global user.email alice@example.com
trap 'echo "failed at line $LINENO: $BASH_COMMAND"' ERR
work=$folder/work
mkdir "$work"
cd "$work"

say() { printf '\n== %s\n' "$*"; }
now() { echo "${EPOCHREALTIME/[.,]/}"; }
took() { echo "$((($(now) - $1) / 1000)) ms"; }
bucket() { du -sb "$folder/bucket" | cut -f1; }
mb() { awk -v bytes="$1" 'BEGIN { printf "%.1f MB", bytes / 1e6 }'; }
# A command as someone, on one of their devices: `client`, which reaches the network through its edge, or
# `only-server` and `only-spare`, which each reach that server straight.
as() {
  local who=$1 device=$2
  shift 2
  ONE_AS=$who ONE_HOME=$folder/$device "$@"
}
# What should be refused is, and is shown with why.
refused() { if said=$("$@" 2>&1); then echo "this should have been refused: $*" && exit 1; else echo "  refused: $(grep -v '^fatal: unable\|^error: failed\|^To one' <<<"$said" | head -2 | tr '\n' ' ')"; fi; }
commit() { git -C "$1" add -A && git -C "$1" -c user.name="$2" -c user.email="$2@example.com" commit -qm "$3"; }

say "alice makes a repo, and pushes to it from one device and fetches from another"
as alice client one repos make --name notes
began=$(now)
as alice client git clone -q one::users/alice/made/repos/notes notes 2>&1
echo "cloned the empty repo in $(took "$began")"
echo "# Notes" >notes/README.md && commit notes alice "Start the notes"
began=$(now)
as alice client git -C notes push -q -u origin main 2>&1
echo "pushed from the laptop (through the edge) in $(took "$began")"
began=$(now)
as alice only-server git clone -q one::users/alice/made/repos/notes phone 2>&1
echo "cloned on the phone (straight to the server) in $(took "$began"): $(cat phone/README.md)"
echo "- buy milk" >>phone/README.md && commit phone alice "Buy milk"
as alice only-server git -C phone push -q 2>&1
began=$(now)
as alice client git -C notes pull -q 2>&1
echo "the laptop fetched the phone's commit in $(took "$began"): $(git -C notes log --format=%s | tr '\n' ',')"

say "A 50 MB repo with history, then one small commit"
as alice client one repos make --name big
git init -q big
for number in $(seq 20); do
  head -c 2500000 /dev/urandom >"big/data-$number.bin" && commit big alice "Add data $number"
done
git -C big remote add origin one::users/alice/made/repos/big
before=$(bucket) began=$(now)
as alice client git -C big push -q -u origin main 2>&1
echo "pushed $(mb "$(du -sb big/.git/objects | cut -f1)") of git objects, 20 commits, in $(took "$began"): the bucket grew $(mb $(($(bucket) - before)))"
echo "one small line" >big/small.txt && commit big alice "One small commit"
before=$(bucket) began=$(now)
as alice client git -C big push -q 2>&1
echo "pushed one small commit in $(took "$began"): the bucket grew $(($(bucket) - before)) bytes"
began=$(now)
as alice only-spare git clone -q one::users/alice/made/repos/big big-again 2>&1
echo "cloned it whole through the spare in $(took "$began"): $(git -C big-again rev-list --count HEAD) commits, $(git -C big-again fsck --no-progress 2>&1 | wc -l) problems"

say "Two pushes to one branch at once, one through each server"
as alice client git clone -q one::users/alice/made/repos/notes one
as alice client git clone -q one::users/alice/made/repos/notes two
echo one >one/one.txt && commit one alice "From one"
echo two >two/two.txt && commit two alice "From two"
(as alice only-server git -C one push 2>&1 | sed 's/^/  one: /') &
first=$!
(as alice only-spare git -C two push 2>&1 | sed 's/^/  two: /') &
# One of them is refused, whichever lost: that is what this shows, and it is handled below.
wait $first $! || true
for side in one two; do
  if [[ $(git -C $side rev-parse HEAD) != $(git -C $side rev-parse origin/main) ]]; then
    echo "$side lost, so it fetches, rebases and pushes again"
    as alice client git -C $side pull -q --rebase 2>&1 && as alice client git -C $side push -q 2>&1
  fi
done
as alice client git -C notes pull -q 2>&1
echo "main has: $(git -C notes log --format=%s | tr '\n' ',')"

say "carol may not read alice's repo, and bob may read but not push"
refused as carol client git clone -q one::users/alice/made/repos/notes carols
as alice client one relations share --artifact users/alice/made/repos/notes --relation viewer --who users/bob
as bob client git clone -q one::users/alice/made/repos/notes bobs 2>&1
echo "bob cloned it"
echo "- a line from bob" >>bobs/README.md && commit bobs bob "Add bob's line"
refused as bob client git -C bobs push origin main
refused as carol client one repos log --repo users/alice/made/repos/notes

say "bob suggests changes, and alice takes one and declines another"
as bob client git -C bobs push -q origin HEAD:refs/suggestions/bobs-line 2>&1
echo "alice's own" >notes/alice.txt && commit notes alice "Add alice's file" && as alice client git -C notes push -q 2>&1
git -C bobs checkout -q -b idea origin/main 2>/dev/null || git -C bobs checkout -q -b idea
echo "maybe" >bobs/idea.txt && commit bobs bob "An idea"
as bob client git -C bobs push -q origin HEAD:refs/suggestions/idea 2>&1
as alice client one repos suggestions --repo users/alice/made/repos/notes | grep -o '"name":"[^"]*","state":"[^"]*"\|"title":"[^"]*"' | paste -sd' '
as alice client one repos take --repo users/alice/made/repos/notes --suggestion bobs-line
as alice client one repos decline --repo users/alice/made/repos/notes --suggestion idea
refused as bob client one repos take --repo users/alice/made/repos/notes --suggestion idea
as alice client git -C notes pull -q 2>&1
echo "main, as git log --graph has it:"
git -C notes log --graph --format='%s' | sed 's/^/  /'

say "bob pushes to the family's repo, as one of its members"
as alice client one relations group --name family
as alice client one relations share --artifact groups/family --relation member --who users/bob
ONE_FOR=groups/family as alice client one repos make --name chores
as bob client git clone -q one::groups/family/made/repos/chores chores 2>&1
echo "- dishes" >chores/list.md && commit chores bob "Start the chores"
as bob client git -C chores push -q -u origin main 2>&1 && echo "bob pushed to groups/family/made/repos/chores"
refused as carol client git clone -q one::groups/family/made/repos/chores carols-chores

say "A server killed in the middle of a push, and the push made again through the spare"
for number in $(seq 4); do head -c 5000000 /dev/urandom >"big/more-$number.bin"; done
commit big alice "Add 20 MB more"
pushing=$(git -C big rev-parse HEAD)
server=$(pgrep -f "mix network $folder server")
(as alice only-server git -C big push -q 2>&1 | sed 's/^/  the push: /') &
push=$!
grep -q -m1 "repos: taking a push" <(tail -f -n 0 "$folder/server.log")
kill -9 "$server"
echo "the first server was killed as it took the push"
wait $push || true
began=$(now)
as alice client git -C big push -q 2>&1
echo "pushed again through the spare in $(took "$began")"
as alice only-spare git -C big-again pull -q 2>&1
echo "the spare serves it: $(git -C big-again rev-parse HEAD | grep -c "$pushing") head as pushed, $(git -C big-again rev-list --count HEAD) commits, $(git -C big-again fsck --no-progress 2>&1 | wc -l) problems"
as alice only-spare one repos log --repo users/alice/made/repos/big | grep -o '"subject":"[^"]*"' | head -3 | paste -sd' '
