#!/bin/sh
# Builds iroh's relay protocol (protos/) into relay_protos.js and relay_protos_bg.wasm beside this, which are
# committed, so nothing that builds the node needs a WebAssembly toolchain: run it when protos/ changes. Needs the
# wasm32-unknown-unknown target and wasm-bindgen at Cargo.lock's version; wasm-opt, if there, makes it smaller.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
target="$here/../../target/relay-protos"
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path "$here/protos/Cargo.toml" --target-dir "$target"
wasm-bindgen --target web --no-typescript --out-dir "$here" "$target/wasm32-unknown-unknown/release/relay_protos.wasm"
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int -o "$here/relay_protos_bg.wasm" "$here/relay_protos_bg.wasm"
fi
