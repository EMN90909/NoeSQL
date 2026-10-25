#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
echo "The former Zig/C benchmark harness has been retired." >&2
echo "Use make test and make native-chain for qualification." >&2
echo "No current comparative benchmark against SQLite is available." >&2
exit 2
