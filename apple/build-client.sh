#!/bin/sh
# Builds the client library (client/) for iPhone and the iPhone simulator, as OneClient.xcframework.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# The C and assembly in the client's dependencies, built for the iPhone app's oldest iOS (project.yml).
export IPHONEOS_DEPLOYMENT_TARGET=26.0
for target in aarch64-apple-ios aarch64-apple-ios-sim; do
	cargo build --release -p one-client --lib --target "$target"
done
out=apple/build/OneClient.xcframework
rm -rf "$out" apple/build/include
mkdir -p apple/build/include
cp client/one_client.h apple/build/include/
cat > apple/build/include/module.modulemap <<'MAP'
module OneClient {
    header "one_client.h"
    export *
}
MAP
xcodebuild -create-xcframework \
	-library target/aarch64-apple-ios/release/libone_client.a -headers apple/build/include \
	-library target/aarch64-apple-ios-sim/release/libone_client.a -headers apple/build/include \
	-output "$out"
echo "built $out"
