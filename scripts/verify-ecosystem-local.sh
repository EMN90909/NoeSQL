#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "NoeSQL native validation: Noqeri + Rust + Zig"
make no-js no-native-c-family
zig build test
echo "NoeSQL native validation complete"
