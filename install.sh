#!/bin/sh
# Installs The One App from the latest release:
#   curl -fsSL theoneapp.drevan.me | sh
set -eu
url=https://github.com/drevantonder/the-one-app/releases/latest/download
os=$(uname -s | tr '[:upper:]' '[:lower:]')
case $(uname -m) in
x86_64 | amd64) arch=amd64 ;;
aarch64 | arm64) arch=arm64 ;;
armv7l | armv6l) arch=arm ;;
*) echo "no release for $(uname -m)" >&2; exit 1 ;;
esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$HOME/.local/bin"
if [ "$os" = darwin ]; then
	curl -fsSL "$url/TheOneApp-macos.zip" -o "$tmp/app.zip"
	ditto -x -k "$tmp/app.zip" "$tmp"
	mkdir -p "$HOME/Applications"
	rm -rf "$HOME/Applications/TheOneApp.app"
	mv "$tmp/TheOneApp.app" "$HOME/Applications/"
	ln -sf "$HOME/Applications/TheOneApp.app/Contents/MacOS/one" "$HOME/.local/bin/one"
	echo "installed ~/Applications/TheOneApp.app: open it to start the node"
else
	curl -fsSL "$url/one-$os-$arch" -o "$tmp/one"
	chmod +x "$tmp/one"
	mv "$tmp/one" "$HOME/.local/bin/one"
	echo "installed ~/.local/bin/one: start the node with one serve"
fi
