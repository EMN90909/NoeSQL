#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "NoeSQL native validation: Noqeri + Rust + Zig"
command -v zig >/dev/null || { echo "Missing Zig 0.15.2" >&2; exit 2; }
command -v cargo >/dev/null || { echo "Missing Rust/Cargo" >&2; exit 2; }
if [[ "$(zig version)" != "0.15.2" ]]; then
    echo "Zig 0.15.2 required; found $(zig version)" >&2
    exit 2
fi
make no-js no-c-family
cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets
zig build --build-file Native/zig/build.zig test -Doptimize=Debug
echo "NoeSQL native reference, WAL, recovery and index tests completed"
