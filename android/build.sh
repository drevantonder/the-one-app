#!/bin/sh
# Builds the Android app as a release APK (signed with the debug key until it has a release key of its own), from
# config.toml's white label, or $CONFIG's: CONFIG=path/to/config.toml ./build.sh. → build/<cli>.apk
set -eu
cd "$(dirname "$0")"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
./gradlew --quiet assembleRelease
config="${CONFIG:-../config.toml}"
cli=$(sed -n '/^\[/q; s/^cli *= *"\([^"]*\)".*/\1/p' "$config")
mkdir -p build
cp app/build/outputs/apk/release/app-release.apk "build/$cli.apk"
echo "build/$cli.apk"
