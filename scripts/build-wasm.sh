#!/usr/bin/env bash
# Builds everything the frontend needs from Rust:
# - TypeScript types  -> frontend/lib/types (decoder and node)
# - WASM package      -> frontend/lib/invoice-wasm
set -euo pipefail

cd "$(dirname "$0")/.."

cargo test --quiet -p invoice-wasm --features ts export_bindings
cargo test --quiet -p node-core --features ts export_bindings
wasm-pack build crates/invoice-wasm --release --target web --out-dir ../../../frontend/lib/invoice-wasm

# wasm-pack ignores its own output; the frontend commits it so it can build without a Rust toolchain.
rm -f ../frontend/lib/invoice-wasm/.gitignore
