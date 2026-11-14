#!/bin/sh
# Builds the extension component and puts it next to extension.toml as extension.wasm.
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp "target/wasm32-wasip2/release/$(echo "{{package}}" | tr - _).wasm" extension.wasm
echo "built $(pwd)/extension.wasm"
