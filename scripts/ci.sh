#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown
cargo test --workspace
cargo build -p core --target wasm32-unknown-unknown
