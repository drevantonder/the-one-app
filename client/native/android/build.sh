#!/bin/bash
# Builds the Android app (client/native: Tauri's window, in the Gradle project here, with its Kotlin) as an APK, from
# config.toml's custom build or $CONFIG's: CONFIG=path/to/config.toml build.sh [debug | release] → the APK's path.
#   debug    (the default) the app's Rust in the dev profile, for `one throwaway client android`
#   release  the app's Rust optimized and the APK shrunk, signed with the debug key until the app has a release key of
#            its own: the key the Kotlin app was signed with, so it installs over it, keeping its sign-in
# ABIS (some of arm64-v8a x86_64) picks the processors: both, or for a debug build the one it's for. Links to the custom
# build's address open the app; for one that asks where, LINKS=https://one.example.com names the addresses whose links
# should (each lists the app's certificate with `cloudflare set android`).
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
android=$native/android
cd "$native"
mode=${1:-debug}
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
ndk=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)
bin=$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin
case $mode in debug) profile=debug release= task=assembleDebug ;; release) profile=release release=--release task=assembleRelease ;; *) echo "build.sh [debug | release]"; exit 1 ;; esac
generated=$android/app/build/generated
mkdir -p "$generated/wry"
# What Tauri's build needs of the project: where its Gradle files are made, and the Kotlin its web view is made of (one
# package, `app.client`, as the app's own is), named for the library.
export TAURI_ANDROID_PROJECT_PATH=$android WRY_ANDROID_PACKAGE=app.client WRY_ANDROID_LIBRARY=one_native WRY_ANDROID_KOTLIN_FILES_OUT_DIR=$generated/wry
for abi in ${ABIS:-arm64-v8a x86_64}; do
	case $abi in arm64-v8a) target=aarch64-linux-android ;; x86_64) target=x86_64-linux-android ;; *) echo "no target for $abi" >&2; exit 1 ;; esac
	upper=$(echo "$target" | tr a-z- A-Z_) lower=$(echo "$target" | tr - _)
	env "CARGO_TARGET_${upper}_LINKER=$bin/${target}29-clang" "CC_$lower=$bin/${target}29-clang" "AR_$lower=$bin/llvm-ar" \
		CONFIG=$config ONE_BUILD=$BUILD_COMMIT cargo build -q --lib --target "$target" $release
	mkdir -p "$generated/jniLibs/$abi"
	cp "target/$target/$profile/libone_native.so" "$generated/jniLibs/$abi/"
done
cd "$android"
# No daemon: one left idle holds GBs on machines many agents share, and a warm one built no faster there. Its single-use
# one is listed in this checkout only, so another's `gradlew --stop` can't stop this build.
CONFIG=$config ./gradlew --quiet --no-daemon -Dorg.gradle.daemon.registry.base="$PWD/.gradle/daemon" "$task" >&2
mkdir -p "$native/target/android"
cp "app/build/outputs/apk/$mode/app-$mode.apk" "$native/target/android/$CONFIG_CLI.apk"
echo "$native/target/android/$CONFIG_CLI.apk"
