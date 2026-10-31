#!/bin/sh
# Installs the latest release for this machine, checked against its checksums.txt: on Linux the node, run as a
# system service; on a Mac the app, with its node inside. Run again to update. `sh install.sh <binary>` installs a
# binary you built instead.
set -eu
repo=drevantonder/the-one-app # brand.toml's repo; a test keeps them the same
url=https://github.com/$repo/releases/latest/download
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$HOME/.local/bin"

get() { curl -fsSL "$url/$1" -o "$tmp/$1"; }
# The release's file for this machine: the one in checksums.txt whose name ends in $1, checked.
fetch() {
	get checksums.txt
	asset=$(sed -n "s/^[0-9a-f]*  \(.*$1\)$/\1/p" "$tmp/checksums.txt" | head -1)
	[ -n "$asset" ] || { echo "the latest release has nothing for this machine ($1)" >&2; exit 1; }
	echo "Downloading $asset"
	get "$asset"
	if command -v sha256sum >/dev/null; then sum=sha256sum; else sum="shasum -a 256"; fi
	(cd "$tmp" && grep "  $asset\$" checksums.txt | $sum -c - >/dev/null) || { echo "$asset doesn't match checksums.txt" >&2; exit 1; }
}

case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "no release for $(uname -m)" >&2; exit 1 ;;
esac

if [ "$(uname -s)" = Darwin ]; then
	fetch -macos.zip
	ditto -x -k "$tmp/$asset" "$tmp/app"
	app=$(cd "$tmp/app" && ls -d -- *.app | head -1)
	cli=$(ls "$tmp/app/$app/Contents/Library/LaunchAgents" | sed -n 's/^app\.\(.*\)\.mac\.node\.plist$/\1/p')
	osascript -e "quit app id \"app.$cli.mac\"" 2>/dev/null || true
	mkdir -p "$HOME/Applications"
	rm -rf "$HOME/Applications/$app"
	mv "$tmp/app/$app" "$HOME/Applications/"
	ln -sf "$HOME/Applications/$app/Contents/MacOS/$cli" "$HOME/.local/bin/$cli"
	# A node already running keeps the old binary until it restarts.
	launchctl kickstart -k "gui/$(id -u)/app.$cli.mac.node" 2>/dev/null || true
	open "$HOME/Applications/$app"
	echo "Installed ~/Applications/$app, which runs the node."
else
	if [ $# -gt 0 ]; then
		bin=$1 cli=$(basename "$1")
	else
		fetch "-$arch-unknown-linux-gnu"
		bin=$tmp/$asset cli=${asset%-"$arch"-unknown-linux-gnu}
		chmod +x "$bin"
	fi
	name=$("$bin" help | head -1 | cut -d: -f1)
	data="$HOME/.local/share/$cli"
	sudo=$([ "$(id -u)" = 0 ] || echo sudo)
	if [ -d /run/systemd/system ]; then $sudo systemctl stop "$cli" 2>/dev/null || true; fi
	# The old app's data has no device key; keep it aside rather than mistake it for this one's.
	if [ -d "$data" ] && [ ! -f "$data/device.json" ]; then
		mv "$data" "$data-legacy"
		echo "Kept the previous data in $data-legacy"
	fi
	install -m 755 "$bin" "$HOME/.local/bin/$cli"
	if [ -d /run/systemd/system ]; then
		$sudo tee "/etc/systemd/system/$cli.service" >/dev/null <<EOF
[Unit]
Description=$name node
After=network-online.target
Wants=network-online.target

[Service]
User=$(id -un)
ExecStart=$HOME/.local/bin/$cli serve
Restart=always
RestartSec=2
Environment=HOME=$HOME

[Install]
WantedBy=multi-user.target
EOF
		$sudo systemctl daemon-reload
		$sudo systemctl enable --now "$cli"
		echo "Installed ~/.local/bin/$cli, running as the $cli service."
	else
		echo "Installed ~/.local/bin/$cli. This machine has no systemd: run the node with $cli serve"
	fi
fi

case :$PATH: in *:"$HOME/.local/bin":*) ;; *) echo "Add ~/.local/bin to your PATH." ;; esac
echo "
Next, on your first device:  $cli create <name>
Or, to join a space:         $cli join <pairing code>   ($cli pair on a device in it gives one)"
