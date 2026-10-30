#!/bin/sh
# Installs this build's node as a system service on a Linux machine, replacing whatever ran before.
# Run on the machine itself, from the directory holding the binary: sh install-linux.sh ./one
set -eu
bin=${1:?usage: install-linux.sh <binary>}
cli=$(basename "$bin")
name=$("$bin" help | head -1 | cut -d: -f1)
data="$HOME/.local/share/$cli"

sudo systemctl stop "$cli" 2>/dev/null || true
if [ -d "$data" ] && [ ! -f "$data/device.json" ]; then
	mv "$data" "$data-legacy"
	echo "kept the previous data in $data-legacy"
fi
mkdir -p "$HOME/.local/bin"
install -m 755 "$bin" "$HOME/.local/bin/$cli"

sudo tee "/etc/systemd/system/$cli.service" >/dev/null <<EOF
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
sudo systemctl daemon-reload
sudo systemctl enable --now "$cli"
sleep 2
"$HOME/.local/bin/$cli" status
