#!/bin/sh
# Puts your checkout's build of a service in another language on a device, here or over ssh, and switches it on there:
# ./install.sh whatsapp [host], for trying it before it's sent out. It's built for the device from its folder here and
# put at ~/.local/bin/<cli>-<name>, where the device's oned runs it as a part of its own (src/oned.rs), started again
# on its new build, until a release of it newer than this build is published (`<cli> update` sends each out). Switching
# it on needs none of this: oned fetches its newest release. CONFIG=path/to/config.toml puts it there for that white
# label: its CLI.
set -eu
config=${CONFIG:+$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")}
cd "$(dirname "$0")"
service=$1 cli=one
host=${2:-}
on() { if [ -n "$host" ]; then ssh "$host" "$@" </dev/null; else sh -c "$*"; fi; }
[ -d "$service" ] || { echo "$service isn't a service in another language here: $cli services on $service switches one of this program's on" >&2; exit 1; }
if [ -n "$config" ]; then
	cli=$(sed -n '/^\[/q; s/^cli *= *"\{0,1\}\([^"]*\)"\{0,1\} *$/\1/p' "$config")
fi
name=$cli-$service
set -- $(on uname -sm)
os=$(echo "$1" | tr A-Z a-z)
case $2 in x86_64) arch=amd64 ;; *) arch=arm64 ;; esac
on "test -x ~/.local/bin/$cli" || { echo "$cli isn't on ${host:-this device}: install it there first" >&2; exit 1; }
(cd "$service" && CGO_ENABLED=0 GOOS=$os GOARCH=$arch go build -ldflags "-X main.built=$(git rev-parse --short=12 HEAD)$(git diff --quiet HEAD -- . || echo +dirty)" -o "../$name" .)
# Put in place in one step, so oned never starts half of it.
if [ -n "$host" ]; then scp -q "$name" "$host:.local/bin/$name.new"; else cp "$name" ~/.local/bin/"$name.new"; fi
rm "$name"
on "chmod 755 ~/.local/bin/$name.new && mv ~/.local/bin/$name.new ~/.local/bin/$name && ~/.local/bin/$cli services on $service"
