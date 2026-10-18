#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != "Linux" ]]; then echo "Zig pager v2 is Linux-only" >&2; exit 2; fi
command -v zig >/dev/null || { echo 'Requires Zig 0.15.2' >&2; exit 2; }
[[ "$(zig version)" == '0.15.2' ]] || { echo 'Requires Zig 0.15.2' >&2; exit 2; }
(cd Native/zig && zig build -Doptimize=ReleaseSafe)
cc -O2 -Wall -Wextra -Werror -I Native/zig Benchmarks/benchmark-zig-v2.c \
    Native/zig/zig-out/lib/libnoesql_zig_pages.a -lsqlite3 -lm -lpthread -ldl \
    -o Native/zig/zig-out/benchmark-zig-v2
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
Native/zig/zig-out/benchmark-zig-v2 "$workdir" "${1:-250}"
