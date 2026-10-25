#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Zig pager benchmark is currently Linux-only" >&2
  exit 2
fi

command -v zig >/dev/null || { echo 'Requires Zig 0.15.2' >&2; exit 2; }
command -v cargo >/dev/null || { echo 'Requires Rust/Cargo' >&2; exit 2; }
[[ "$(zig version)" == '0.15.2' ]] || { echo 'Requires Zig 0.15.2' >&2; exit 2; }

zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
cargo run --release \
  --manifest-path Native/rust/Cargo.toml \
  --features sqlite-benchmark \
  --bin benchmark_compare_sqlite -- \
  "$workdir" "${1:-250}"
