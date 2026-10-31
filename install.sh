#!/bin/sh
# Installs the latest release for this machine, if its signature is the release key's: on Linux the node, run as a
# system service; on a Mac the app, with its node inside. Run again to update. `sh install.sh <binary>` installs a
# binary you built instead.
set -eu
# brand.toml's repo, cli and release_key; a test keeps them the same.
repo=drevantonder/the-one-app
cli=one
key="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJxXC8FYsJQXptCREC6fvoYzO1sNLdOHtbsAnmUTljCM"
url=https://github.com/$repo/releases/latest/download
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$HOME/.local/bin"

# Downloads the release's file $1 and checks its signature, $1.sig, with OpenSSH, which macOS and Linux both have.
fetch() {
	command -v ssh-keygen >/dev/null || { echo "checking the download needs ssh-keygen: install OpenSSH (openssh-client on Debian and Ubuntu)" >&2; exit 1; }
	echo "Downloading $1"
	for f in "$1" "$1.sig"; do
		curl -fsSL "$url/$f" -o "$tmp/$f" || { echo "the latest release has nothing for this machine ($1)" >&2; exit 1; }
	done
	echo "release namespaces=\"release\" $key" > "$tmp/signers"
	ssh-keygen -Y verify -f "$tmp/signers" -I release -n release -s "$tmp/$1.sig" < "$tmp/$1" >/dev/null 2>&1 ||
		{ echo "$1 isn't signed with the release key, so it isn't installed" >&2; exit 1; }
}

case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "no release for $(uname -m)" >&2; exit 1 ;;
esac

if [ "$(uname -s)" = Darwin ]; then
	fetch "$cli-macos.zip"
	ditto -x -k "$tmp/$cli-macos.zip" "$tmp/app"
	app=$(cd "$tmp/app" && ls -d -- *.app | head -1)
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
		fetch "$cli-$arch-unknown-linux-gnu"
		bin=$tmp/$cli-$arch-unknown-linux-gnu
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
