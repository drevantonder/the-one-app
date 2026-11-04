#!/bin/sh
# Installs the latest release for this machine, if its signature is the release key's: on Linux the program, with
# <cli>d, the device, run as a system service; on a Mac the app, which is the Mac's <cli>d. Run again to update.
# `sh install.sh <binary>` installs a binary you built instead.
set -eu
# config.toml's repo, cli and release_key; a test keeps them the same.
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
	# Git reaches your repos' addresses through it, by this name.
	ln -sf "$HOME/Applications/$app/Contents/MacOS/$cli" "$HOME/.local/bin/git-remote-$cli"
	# oned already running keeps the old binary until it restarts.
	launchctl kickstart -k "gui/$(id -u)/app.$cli.mac.oned" 2>/dev/null || true
	open "$HOME/Applications/$app"
	echo "Installed ~/Applications/$app, which runs ${cli}d."
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
	units=/etc/systemd/system
	if [ -d /run/systemd/system ]; then $sudo systemctl stop "${cli}d" "$cli" 2>/dev/null || true; fi
	# The old app's data has no device key; keep it aside rather than mistake it for this one's.
	if [ -d "$data" ] && [ ! -f "$data/device.json" ]; then
		mv "$data" "$data-previous"
		echo "Kept the previous data in $data-previous"
	fi
	install -m 755 "$bin" "$HOME/.local/bin/$cli"
	ln -sf "$cli" "$HOME/.local/bin/git-remote-$cli"
	ln -sf "$cli" "$HOME/.local/bin/${cli}d"
	if [ -d /run/systemd/system ]; then
		# A machine that ran its node as `$cli serve`, beside a unit per service, moves to ${cli}d (ARCHITECTURE.md's
		# Moving a machine to oned): the node's environment kept, and each service's unit stopped and gone, the service
		# turned on in ${cli}d once it answers, with the key and folder it had. A copy of this program a unit ran as a
		# service goes; a program in another language stays, for ${cli}d to run.
		env=$(sed -n 's/^Environment=//p' "$units/$cli.service" "$units/${cli}d.service" 2>/dev/null | grep -v '^HOME=' | sort -u || true)
		parts=
		for unit in "$units/$cli"-*.service; do
			[ -f "$unit" ] || continue
			part=$(basename "$unit" .service)
			part=${part#"$cli"-}
			run=$(sed -n 's/^ExecStart=//p' "$unit")
			$sudo systemctl disable --now "$cli-$part" 2>/dev/null || true
			$sudo rm -f "$unit"
			case $run in *" sidecars serve "* | *" services serve "*) [ "${run%% *}" = "$HOME/.local/bin/$cli" ] || rm -f "${run%% *}" ;; esac
			parts="$parts $part"
		done
		if [ -f "$units/$cli.service" ]; then
			$sudo systemctl disable "$cli" 2>/dev/null || true
			$sudo rm -f "$units/$cli.service"
		fi
		# The node answers every client of the install, so it keeps its memory and, outside a container (where it can't
		# be lowered), is the last thing killed when the machine runs short.
		oom=""
		systemd-detect-virt -cq 2>/dev/null || oom="OOMScoreAdjust=-900"
		$sudo tee "$units/${cli}d.service" >/dev/null <<EOF
[Unit]
Description=$name, this device
After=network-online.target
Wants=network-online.target

[Service]
User=$(id -un)
ExecStart=$HOME/.local/bin/${cli}d
Restart=always
RestartSec=2
# An update's ${cli}d takes over from the one before, then tells systemd it's the main process.
NotifyAccess=all
MemoryLow=768M
$oom
Environment=HOME=$HOME
$(for e in $env; do echo "Environment=$e"; done)

[Install]
WantedBy=multi-user.target
EOF
		$sudo systemctl daemon-reload
		$sudo systemctl enable --now "${cli}d"
		for part in $parts; do
			on=
			for _ in $(seq 60); do "$HOME/.local/bin/$cli" services on "$part" >/dev/null 2>&1 && on=1 && break; sleep 2; done
			if [ -n "$on" ]; then echo "Moved $part into ${cli}d."; else echo "Couldn't switch $part on in ${cli}d: run $cli services on $part" >&2; fi
		done
		echo "Installed ~/.local/bin/$cli, with ${cli}d running as a service."
	else
		echo "Installed ~/.local/bin/$cli. This machine has no systemd: run ${cli}d, this device, yourself"
	fi
fi

case :$PATH: in *:"$HOME/.local/bin":*) ;; *) echo "Add ~/.local/bin to your PATH." ;; esac
echo "
Next, on your first device:  $cli create <name>
Or, to join a space:         $cli join <pairing code>   ($cli pair on a device in it gives one)
Or, to look around first:    $cli try                   (a new install to try things in; $cli try stop deletes it)"
