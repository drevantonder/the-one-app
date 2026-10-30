#!/bin/sh
# Builds the WhatsApp provider and installs it as the one-whatsapp service on
# a Linux device, here or over ssh: ./install.sh [host]
set -eu
cd "$(dirname "$0")"
host=${1:-}
CGO_ENABLED=0 GOOS=linux go build -o one-whatsapp .
on() { if [ -n "$host" ]; then ssh "$host" "$@"; else sh -c "$*"; fi; }
if [ -n "$host" ]; then scp -q one-whatsapp "$host":/tmp/one-whatsapp; else cp one-whatsapp /tmp/one-whatsapp; fi
rm one-whatsapp
on 'set -e
user=$(id -un)
mkdir -p ~/.local/bin ~/.local/share/one/whatsapp
mv /tmp/one-whatsapp ~/.local/bin/one-whatsapp
sudo tee /etc/systemd/system/one-whatsapp.service >/dev/null <<EOF
[Unit]
Description=The One App: WhatsApp
After=one.service
Wants=one.service

[Service]
User=$user
ExecStart=$HOME/.local/bin/one-whatsapp
Restart=always
RestartSec=2
Environment=HOME=$HOME

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable --now one-whatsapp
sudo systemctl restart one-whatsapp'
