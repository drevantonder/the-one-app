#!/bin/sh
# Builds a provider and installs it as the one-<provider> service on a Linux
# device, here or over ssh: ./install.sh whatsapp [host]
set -eu
cd "$(dirname "$0")/$1"
name=one-$1
host=${2:-}
CGO_ENABLED=0 GOOS=linux go build -o "$name" .
on() { if [ -n "$host" ]; then ssh "$host" "$@"; else sh -c "$*"; fi; }
if [ -n "$host" ]; then scp -q "$name" "$host:/tmp/$name"; else cp "$name" "/tmp/$name"; fi
rm "$name"
on "name=$name"'
set -e
user=$(id -un)
mkdir -p ~/.local/bin
mv /tmp/$name ~/.local/bin/$name
sudo tee /etc/systemd/system/$name.service >/dev/null <<EOF
[Unit]
Description=$name
After=one.service
Wants=one.service

[Service]
User=$user
ExecStart=$HOME/.local/bin/$name
Restart=always
RestartSec=2
Environment=HOME=$HOME

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable $name
sudo systemctl restart $name'
