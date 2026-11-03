#!/bin/sh
# Installs a sidecar as the one-<sidecar> service on a device, here or over ssh: ./install.sh whatsapp [host]. It's a
# service endpoint of its own (src/service.rs), which needs no node there, only this program: one with a folder here is
# a Go program, built for the device and started by `one sidecars host`; any other is one this program hosts (`one
# sidecars serve`). It's for the One App of the device you run this on, and shows a code that one of your devices
# approves it with (`one devices approve <code>`), which this prints. On Linux it's a systemd service; on a Mac, a
# launch agent of the user's. CONFIG=path/to/config.toml installs it as <cli>-<sidecar>, for that white label: its CLI
# and data folder (the node service's <CLI>_HOME there, if it has one).
set -eu
config=${CONFIG:+$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")}
cd "$(dirname "$0")"
sidecar=$1 name=one-$1 node=one env= home='$HOME/.local/share/one'
host=${2:-}
on() { if [ -n "$host" ]; then ssh "$host" "$@" </dev/null; else sh -c "$*"; fi; }
if [ -n "$config" ]; then
	field() { sed -n "/^\[/q; s/^$1 *= *\"\{0,1\}\([^\"]*\)\"\{0,1\} *$/\1/p" "$config"; }
	cli=$(field cli) name=$(field cli)-$1 node=$(field cli)
	upper=$(echo "$cli" | tr a-z A-Z)
	home=$(on "systemctl show -p Environment $cli 2>/dev/null | tr ' ' '\n' | sed -n 's/^${upper}_HOME=//p'")
	home=${home:-'$HOME'/.local/share/$cli}
	env="${upper}_HOME=$home"
	[ ! -d "$sidecar" ] || env="$env $(echo "$sidecar" | tr a-z- A-Z_)_DATA=$home/$sidecar"
fi
# The One App it's for: this device's.
link=$($node sidecars install-link)
set -- $(on uname -sm)
os=$(echo "$1" | tr A-Z a-z)
case $2 in x86_64) arch=amd64 ;; *) arch=arm64 ;; esac
# This program, there: copied from here if it isn't, when here is the same kind of machine.
if ! on "test -x ~/.local/bin/$node"; then
	[ "$(uname -sm)" = "$*" ] || { echo "$node isn't on ${host:-this device}: install it there first" >&2; exit 1; }
	on "mkdir -p ~/.local/bin"
	if [ -n "$host" ]; then scp -q "$(command -v "$node")" "$host:.local/bin/$node"; else cp "$(command -v "$node")" ~/.local/bin/; fi
fi
if [ -d "$sidecar" ]; then
	(cd "$sidecar" && CGO_ENABLED=0 GOOS=$os GOARCH=$arch go build -ldflags "-X main.built=$(git rev-parse --short=12 HEAD)$(git diff --quiet HEAD -- . || echo +dirty)" -o "../$name" .)
	if [ -n "$host" ]; then scp -q "$name" "$host:/tmp/$name"; else cp "$name" "/tmp/$name"; fi
	rm "$name"
	run='$HOME/.local/bin/'"$node"' sidecars host '"$sidecar"' $HOME/.local/bin/'"$name"' --install '"$link" install='mv /tmp/$name ~/.local/bin/$name'
else
	# A Go program it was before goes.
	run='$HOME/.local/bin/'"$node"' sidecars serve '"$sidecar"' --install '"$link" install='rm -f ~/.local/bin/$name'
fi
# Says the code to approve it with, once it's asked; or that it's approved already.
code() {
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		c=$(on "cat $home/services/$sidecar/code 2>/dev/null" || true)
		[ -z "$c" ] || { echo "Approve $sidecar on one of your devices: $node devices approve $c"; return; }
		sleep 1
	done
	echo "$sidecar is running, approved already."
}
if [ "$os" = darwin ]; then
[ -z "$config" ] || { echo "CONFIG installs on Linux only, for now" >&2; exit 1; }
on "name=$name"'
set -e
mkdir -p ~/.local/bin ~/Library/LaunchAgents ~/Library/Logs
'"$install"'
plist=~/Library/LaunchAgents/app.one.$name.plist
cat >$plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>app.one.$name</string>
<key>ProgramArguments</key><array>$(for a in '"$run"'; do echo "<string>$a</string>"; done)</array>
<key>EnvironmentVariables</key><dict><key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string><key>HOME</key><string>$HOME</string></dict>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>StandardErrorPath</key><string>$HOME/Library/Logs/$name.log</string>
</dict></plist>
EOF
launchctl bootout gui/$(id -u)/app.one.$name 2>/dev/null || true
launchctl bootstrap gui/$(id -u) $plist'
code
exit
fi
on "name=$name"'
set -e
user=$(id -un)
mkdir -p ~/.local/bin
'"$install"'
sudo tee /etc/systemd/system/$name.service >/dev/null <<EOF
[Unit]
Description=$name

[Service]
User=$user
ExecStart='"$run"'
Restart=always
RestartSec=2
Environment=HOME=$HOME
$(for e in '"$env"'; do echo "Environment=$e"; done)

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable $name
sudo systemctl restart $name
sleep 3
systemctl is-active --quiet $name || { journalctl -u $name -n 5 --no-pager; exit 1; }'
code
