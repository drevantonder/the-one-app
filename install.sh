#!/bin/sh
# Installs the newest build for this machine, if the release key signed it: on Linux the program, with <cli>d, the
# device, run as a system service, and on a desktop the app where there's one; on a Mac the app, which sets up <cli> and
# <cli>d as it's signed in. Run again to update; on a Mac, `<cli> update` does.
# `sh install.sh <binary>` installs a binary you built instead.
set -eu
# Where its builds are, a folder laid out as a network's /updates/ is (oned/update/src/latest.rs), and config.toml's cli
# and release_key: whoever serves it fills them in (oned_update::install).
from=''
cli=''
key=''
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$HOME/.local/bin"

# What's newest at $from, kept as $tmp/record, if the release key signed it: the record `<cli> update` reads, checked as
# it checks it, with OpenSSH, which macOS and Linux both have.
newest() {
	[ -n "$from" ] || { echo "this installer doesn't say where builds are: run the one your network's address serves (curl -fsSL <address>/install | sh)" >&2; exit 1; }
	command -v ssh-keygen >/dev/null || { echo "checking the download needs ssh-keygen: install OpenSSH (openssh-client on Debian and Ubuntu)" >&2; exit 1; }
	curl -fsSL "$from/newest" -o "$tmp/newest" || { echo "couldn't read what's newest at $from" >&2; exit 1; }
	sed '/^-----BEGIN SSH SIGNATURE-----$/,$d' "$tmp/newest" > "$tmp/record"
	sed -n '/^-----BEGIN SSH SIGNATURE-----$/,$p' "$tmp/newest" > "$tmp/record.sig"
	echo "release namespaces=\"release-latest\" $key" > "$tmp/signers"
	ssh-keygen -Y verify -f "$tmp/signers" -I release -n release-latest -s "$tmp/record.sig" < "$tmp/record" >/dev/null 2>&1 ||
		{ echo "what's newest at $from isn't signed with the release key, so nothing is installed" >&2; exit 1; }
}

# The hash the record names $1 by, if it names it.
named() {
	awk -v name="$1" 'NF == 6 && $1 == name { print $4; exit }' "$tmp/record"
}

# Downloads what the record names $1 (node/<target>, …) as $tmp/$2, if it's what the record names: its hash.
fetch() {
	sha=$(named "$1")
	[ -n "$sha" ] || { echo "there's no build for this machine at $from ($1)" >&2; exit 1; }
	echo "Downloading $1"
	curl -fsSL "$from/$sha" -o "$tmp/$2" || { echo "couldn't download $1 from $from" >&2; exit 1; }
	if command -v sha256sum >/dev/null; then got=$(sha256sum "$tmp/$2"); else got=$(shasum -a 256 "$tmp/$2"); fi
	[ "${got%% *}" = "$sha" ] || { echo "$1 isn't what the signed record names, so it isn't installed" >&2; exit 1; }
}

case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) echo "no release for $(uname -m)" >&2; exit 1 ;;
esac

if [ "$(uname -s)" = Darwin ]; then
	newest
	fetch "app/$arch-apple-darwin" app.zip
	ditto -x -k "$tmp/app.zip" "$tmp/app"
	app=$(cd "$tmp/app" && ls -d -- *.app | head -1)
	osascript -e "quit app id \"app.$cli.mac\"" 2>/dev/null || true
	mkdir -p "$HOME/Applications"
	rm -rf "$HOME/Applications/$app"
	mv "$tmp/app/$app" "$HOME/Applications/"
	open "$HOME/Applications/$app"
	echo "Installed ~/Applications/$app. Signing in there sets up $cli and ${cli}d, this device."
else
	# The program, and its app runtime (<cli>-apps), which ${cli}d starts beside it.
	if [ $# -gt 0 ]; then
		bin=$1 cli=$(basename "$1")
		runtime=$(dirname "$1")/$cli-apps
	else
		newest
		fetch "node/$arch-unknown-linux-gnu" program
		fetch "service/apps/$arch-unknown-linux-gnu" runtime
		bin=$tmp/program runtime=$tmp/runtime
		chmod +x "$bin"
	fi
	name=$("$bin" help | head -1 | cut -d: -f1)
	sudo=$([ "$(id -u)" = 0 ] || echo sudo)
	units=/etc/systemd/system
	if [ -d /run/systemd/system ]; then $sudo systemctl stop "${cli}d" 2>/dev/null || true; fi
	install -m 755 "$bin" "$HOME/.local/bin/$cli"
	if [ -e "$runtime" ]; then install -m 755 "$runtime" "$HOME/.local/bin/$cli-apps"; fi
	ln -sf "$cli" "$HOME/.local/bin/git-remote-$cli"
	ln -sf "$cli" "$HOME/.local/bin/${cli}d"
	# On a desktop, the app too where there's one, started once: it puts itself in the launcher and opens at login.
	if [ $# -eq 0 ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && [ -n "$(named "app/$arch-unknown-linux-gnu")" ]; then
		fetch "app/$arch-unknown-linux-gnu" app
		install -m 755 "$tmp/app" "$HOME/.local/bin/$cli-app"
		(setsid "$HOME/.local/bin/$cli-app" >/dev/null 2>&1 &)
		echo "Installed ~/.local/bin/$cli-app, the app, which is in your launcher."
	fi
	if [ -d /run/systemd/system ]; then
		# Its environment (a custom build's <CLI>_HOME) as it was, when installed again.
		env=$(sed -n 's/^Environment=//p' "$units/${cli}d.service" 2>/dev/null | grep -v '^HOME=' || true)
		# The app runtime answers every client of the network, so it keeps its memory and, outside a container (where it can't
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
		# Services run each in its sandbox, as its own user, which ${cli}d starts by a unit polkit lets it.
		if script=$("$HOME/.local/bin/$cli" services sandbox); then printf '%s\n' "$script" | $sudo sh; fi
		$sudo systemctl enable --now "${cli}d"
		echo "Installed ~/.local/bin/$cli, with ${cli}d running as a service."
	else
		echo "Installed ~/.local/bin/$cli. This machine has no systemd: run ${cli}d, this device, yourself"
	fi
fi

case :$PATH: in *:"$HOME/.local/bin":*) ;; *) echo "Add ~/.local/bin to your PATH." ;; esac
echo "
Next, on your first device:  $cli create <name> --as <your name>
Or, to join a network:       $cli join <pairing code>   ($cli pair on a device in it gives one)"
