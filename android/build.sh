#!/bin/sh
# Builds the Android app as a release APK (signed with the debug key until it has a release key of its own), from
# config.toml's custom build, or $CONFIG's: CONFIG=path/to/config.toml ./build.sh. → build/<cli>.apk
# Links to the custom build's address open the app; for one that asks where, give the addresses whose links should:
# LINKS=https://one.example.com ./build.sh. Each address lists the app's certificate with `cloudflare set android`.
set -eu
# Gradle reads it too, from another folder: so wherever it was given from, the same file.
if [ -n "${CONFIG:-}" ]; then CONFIG="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"; export CONFIG; fi
cd "$(dirname "$0")"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
# The client library (client/ffi), for phones' processors (ABIS: some of arm64-v8a x86_64), and its Kotlin, which the
# app builds with (app/build.gradle.kts).
ndk=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)
bin=$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin
generated=app/build/generated/client
for abi in ${ABIS:-arm64-v8a x86_64}; do
	case $abi in arm64-v8a) target=aarch64-linux-android ;; x86_64) target=x86_64-linux-android ;; *) echo "no target for $abi" >&2; exit 1 ;; esac
	upper=$(echo "$target" | tr a-z- A-Z_) lower=$(echo "$target" | tr - _)
	env "CARGO_TARGET_${upper}_LINKER=$bin/${target}29-clang" "CC_$lower=$bin/${target}29-clang" "AR_$lower=$bin/llvm-ar" \
		cargo build -q -p one-client-ffi --release --target "$target" --manifest-path ../Cargo.toml
	mkdir -p "$generated/jniLibs/$abi"
	cp "../target/$target/release/libone_client_ffi.so" "$generated/jniLibs/$abi/"
done
../client/ffi/bindings.sh kotlin "$generated/kotlin"
# No daemon: one left idle holds GBs on machines many agents share, and a warm one built no faster there. Its
# single-use one is listed in this checkout only, so another's `gradlew --stop` can't stop this build.
./gradlew --quiet --no-daemon -Dorg.gradle.daemon.registry.base="$PWD/.gradle/daemon" assembleRelease
config="${CONFIG:-../config.toml}"
cli=$(sed -n '/^\[/q; s/^cli *= *"\([^"]*\)".*/\1/p' "$config")
mkdir -p build
cp app/build/outputs/apk/release/app-release.apk "build/$cli.apk"
echo "build/$cli.apk"
