#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "NoeSQL native validation: Noqeri + C++ + Rust + Zig"
make no-js
zig build test
echo "NoeSQL native validation complete"
