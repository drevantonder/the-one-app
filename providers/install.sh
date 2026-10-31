#!/bin/sh
# Builds a provider and installs it as the one-<provider> service on a device,
# here or over ssh: ./install.sh whatsapp [host]. On Linux it's a systemd
# service; on a Mac, a launch agent of the user's.
set -eu
cd "$(dirname "$0")/$1"
name=one-$1
host=${2:-}
on() { if [ -n "$host" ]; then ssh "$host" "$@" </dev/null; else sh -c "$*"; fi; }
set -- $(on uname -sm)
os=$(echo "$1" | tr A-Z a-z)
case $2 in x86_64) arch=amd64 ;; *) arch=arm64 ;; esac
CGO_ENABLED=0 GOOS=$os GOARCH=$arch go build -o "$name" .
if [ -n "$host" ]; then scp -q "$name" "$host:/tmp/$name"; else cp "$name" "/tmp/$name"; fi
rm "$name"
if [ "$os" = darwin ]; then
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
