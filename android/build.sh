#!/bin/sh
# Builds the Android app as a release APK (signed with the debug key until it has a release key of its own), from
# config.toml's white label, or $CONFIG's: CONFIG=path/to/config.toml ./build.sh. → build/<cli>.apk
# Links to the white label's address open the app; for one that asks where, give the addresses whose links should:
# LINKS=https://one.example.com ./build.sh. Each address lists the app's certificate with `kernel set android`.
set -eu
# Gradle reads it too, from another folder: so wherever it was given from, the same file.
if [ -n "${CONFIG:-}" ]; then CONFIG="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"; export CONFIG; fi
cd "$(dirname "$0")"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
# No daemon: one left idle holds GBs on machines many agents share, and a warm one built no faster there.
./gradlew --quiet --no-daemon assembleRelease
config="${CONFIG:-../config.toml}"
cli=$(sed -n '/^\[/q; s/^cli *= *"\([^"]*\)".*/\1/p' "$config")
mkdir -p build
cp app/build/outputs/apk/release/app-release.apk "build/$cli.apk"
echo "build/$cli.apk"
