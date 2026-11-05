#!/bin/sh
# Builds the MyGo iPhone app (spike), on a Mac with Xcode, Go and MyGo's CLI (egoist/mygo PR #940):
#   build.sh [simulator|device]
# The Rust client goes into ../client as a .syso, with uniffi's header beside it, for its cgo loader (-tags cgoffi):
# MyGo links the app's Go as a c-archive, which carries a package's .syso files but not the libraries #cgo LDFLAGS name.
set -eu
kind=${1:-simulator}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
case $kind in
simulator) target=aarch64-apple-ios-sim platform=ios/simulator ld=ios-simulator ;;
device) target=aarch64-apple-ios platform=ios ld=ios ;;
*) echo "build.sh [simulator|device]" >&2; exit 1 ;;
esac
(cd "$root" && IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -q -p one-client-ffi --release --target $target)
# uniffi's header, made where the definition builds (bindings.sh), unless one is there already: HEADER=1 makes it.
if [ "${HEADER:-}" = 1 ] || [ ! -f "$here/../client/one_client_ffiFFI.h" ]; then
	gen=$(mktemp -d)
	"$root/client/ffi/bindings.sh" swift "$gen"
	cp "$gen/one_client_ffiFFI.h" "$here/../client/"
	rm -rf "$gen"
fi
rm -f "$here"/../client/*.syso
xcrun ld -r -arch arm64 -platform_version $ld 17.0 17.0 -all_load -o "$here/../client/one_client_ffi_ios_arm64.syso" "$root/target/$target/release/libone_client_ffi.a"
cd "$here"
GOFLAGS=-tags=cgoffi mygo build -platform $platform
