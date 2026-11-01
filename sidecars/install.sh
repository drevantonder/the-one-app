#!/bin/sh
# Builds a sidecar and installs it as the one-<sidecar> service on a device,
# here or over ssh: ./install.sh whatsapp [host]. On Linux it's a systemd
# service; on a Mac, a launch agent of the user's. CONFIG=path/to/config.toml
# installs it as <cli>-<sidecar>, for that edition's node: its port, CLI and
# data folder (the node service's <CLI>_HOME, if it has one).
set -eu
config=${CONFIG:+$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")}
cd "$(dirname "$0")/$1"
sidecar=$1 name=one-$1 node=one env=
host=${2:-}
on() { if [ -n "$host" ]; then ssh "$host" "$@" </dev/null; else sh -c "$*"; fi; }
if [ -n "$config" ]; then
	field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
	cli=$(field cli) name=$(field cli)-$1 node=$(field cli)
	upper=$(echo "$cli" | tr a-z A-Z)
	home=$(on "systemctl show -p Environment $cli 2>/dev/null | tr ' ' '\n' | sed -n 's/^${upper}_HOME=//p'")
	home=${home:-'$HOME'/.local/share/$cli}
	env="ONE_ADDR=127.0.0.1:$(field port) ONE_CLI=$cli $(echo "$sidecar" | tr a-z A-Z)_DATA=$home/$sidecar"
fi
# Installed by the server's owner who runs this, and allowed there if an owner removed it before.
allow() { on "~/.local/bin/$node sidecars add $sidecar" || echo "$node sidecars add $sidecar failed: the sidecar serves once an owner allows it there" >&2; }
set -- $(on uname -sm)
os=$(echo "$1" | tr A-Z a-z)
case $2 in x86_64) arch=amd64 ;; *) arch=arm64 ;; esac
CGO_ENABLED=0 GOOS=$os GOARCH=$arch go build -ldflags "-X main.built=$(git rev-parse --short=12 HEAD)$(git diff --quiet HEAD -- . || echo +dirty)" -o "$name" .
if [ -n "$host" ]; then scp -q "$name" "$host:/tmp/$name"; else cp "$name" "/tmp/$name"; fi
rm "$name"
if [ "$os" = darwin ]; then
[ -z "$config" ] || { echo "CONFIG installs on Linux only, for now" >&2; exit 1; }
on "name=$name"'
set -e
mkdir -p ~/.local/bin ~/Library/LaunchAgents ~/Library/Logs
mv /tmp/$name ~/.local/bin/$name
plist=~/Library/LaunchAgents/app.one.$name.plist
cat >$plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>app.one.$name</string>
<key>ProgramArguments</key><array><string>$HOME/.local/bin/$name</string></array>
<key>EnvironmentVariables</key><dict><key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string><key>HOME</key><string>$HOME</string></dict>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>StandardErrorPath</key><string>$HOME/Library/Logs/$name.log</string>
</dict></plist>
EOF
launchctl bootout gui/$(id -u)/app.one.$name 2>/dev/null || true
launchctl bootstrap gui/$(id -u) $plist'
allow
exit
fi
on "name=$name"'
set -e
user=$(id -un)
mkdir -p ~/.local/bin
mv /tmp/$name ~/.local/bin/$name
sudo tee /etc/systemd/system/$name.service >/dev/null <<EOF
[Unit]
Description=$name
After='"$node"'.service
Wants='"$node"'.service

[Service]
User=$user
ExecStart=$HOME/.local/bin/$name
Restart=always
RestartSec=2
Environment=HOME=$HOME
$(for e in '"$env"'; do echo "Environment=$e"; done)

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable $name
sudo systemctl restart $name'
allow
