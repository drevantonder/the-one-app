#!/bin/sh
# The client's bindings for an app, from the definition in src/lib.rs as this machine builds it: bindings.sh
# <swift|kotlin|cs> <folder>. C# needs uniffi-bindgen-cs, of the version Cargo.toml pins uniffi for:
#   cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.1+v0.31.0
set -eu
language=$1 out=$(mkdir -p "$2" && cd "$2" && pwd)
cd "$(dirname "$0")/../.."
cargo build -q -p one-client-ffi
case $(uname) in Darwin) lib=target/debug/libone_client_ffi.dylib ;; *) lib=target/debug/libone_client_ffi.so ;; esac
find "$out" -mindepth 1 -delete
if [ "$language" = cs ]; then
	uniffi-bindgen-cs --library "$lib" --out-dir "$out" --no-format
else
	cargo run -q -p one-client-ffi --features bindgen --bin uniffi-bindgen -- generate --library "$lib" --language "$language" --out-dir "$out" --no-format
fi
