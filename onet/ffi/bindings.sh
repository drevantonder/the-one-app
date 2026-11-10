#!/bin/sh
# onet's bindings for an app, from the definition in src/lib.rs as this machine builds it: bindings.sh
# <swift> <folder> (uniffi makes Kotlin and others too, but no app uses them).
set -eu
language=$1 out=$(mkdir -p "$2" && cd "$2" && pwd)
cd "$(dirname "$0")/../.."
cargo build -q -p onet-ffi
case $(uname) in Darwin) lib=target/debug/libonet_ffi.dylib ;; *) lib=target/debug/libonet_ffi.so ;; esac
find "$out" -mindepth 1 -delete
cargo run -q -p onet-ffi --features bindgen --bin uniffi-bindgen -- generate --library "$lib" --language "$language" --out-dir "$out" --no-format
