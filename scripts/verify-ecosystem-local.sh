#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${NODE:=node}"
echo "NoeSQL reference/store/session validation"
"$NODE" --test   tests/noesql-session.test.mjs   tests/noesql-migrate-noedb.test.mjs   tests/noesql-canonical-source.test.mjs
echo "NoeSQL v1/v2 native ABI and storage tests"
make -C Native/cpp test
echo "NoeSQL scoped verification complete"
