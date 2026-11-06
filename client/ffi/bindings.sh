#!/bin/sh
# The client's bindings for an app, from the definition in src/lib.rs as this machine builds it: bindings.sh
# <swift|kotlin> <folder>.
set -eu
language=$1 out=$(mkdir -p "$2" && cd "$2" && pwd)
cd "$(dirname "$0")/../.."
cargo build -q -p one-client-ffi
case $(uname) in Darwin) lib=target/debug/libone_client_ffi.dylib ;; *) lib=target/debug/libone_client_ffi.so ;; esac
find "$out" -mindepth 1 -delete
cargo run -q -p one-client-ffi --features bindgen --bin uniffi-bindgen -- generate --library "$lib" --language "$language" --out-dir "$out" --no-format
