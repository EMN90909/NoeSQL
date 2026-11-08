#!/usr/bin/env bash
# Local Linux x86-64 engineering qualification; NO GitHub Actions involved.
# Passing these tests is necessary, not sufficient, for a production release.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

die() { printf 'NoeSQL qualification blocked: %s\n' "$*" >&2; exit 2; }
for cmd in uname make cargo rustc zig sha256sum tee python3; do
  command -v "$cmd" >/dev/null 2>&1 || die "missing $cmd"
done
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] ||
  die "this release gate currently supports Linux x86-64 only"
[[ "$(zig version)" == "0.15.2" ]] || die "pin Zig to 0.15.2"
[[ -n "${NOQERI_COMPILER:-}" && -x "$NOQERI_COMPILER" ]] ||
  die "set NOQERI_COMPILER to a trusted executable Noqeri compiler"
[[ -n "${NOQERI_REPO:-}" && -f "$NOQERI_REPO/Compiler/selfhost/trusted-stage1.nqri" ]] ||
  die "set NOQERI_REPO to the pinned local Noqeri source tree"
[[ -n "${NOQERI_NQRI_VM:-}" && -x "$NOQERI_NQRI_VM" ]] ||
  die "set NOQERI_NQRI_VM to the trusted NQRI runtime executable"
[[ -n "${NOQERI_WEB:-}" && -x "$NOQERI_WEB" ]] ||
  die "set NOQERI_WEB to the locally compiled native Noqeri web executable"

EVIDENCE="${NOESQL_EVIDENCE_DIR:-$PWD/.qualification}"
mkdir -p "$EVIDENCE"
LOG="$EVIDENCE/local-engineering.log"
printf 'NoeSQL local qualification: source=%s\n' "$(pwd)" | tee -a "$LOG"
printf 'Rust: %s\n' "$(rustc --version)" | tee -a "$LOG"
printf 'Cargo: %s\n' "$(cargo --version)" | tee -a "$LOG"
printf 'Zig: %s\n' "$(zig version)" | tee -a "$LOG"
if command -v git >/dev/null 2>&1; then
  printf 'Git: %s\n' "$(git rev-parse HEAD 2>/dev/null || echo unknown)" | tee -a "$LOG"
fi

run() {
  printf '\n> ' | tee -a "$LOG"
  printf '%q ' "$@" | tee -a "$LOG"
  printf '\n' | tee -a "$LOG"
  "$@" 2>&1 | tee -a "$LOG"
  # pipefail preserves the tested command failure.
}
run make test
run make stress
run make test-sqlite
run make native-chain NOQERI_COMPILER="$NOQERI_COMPILER"
run make stage2-chain NOQERI_REPO="$NOQERI_REPO" NOQERI_NQRI_VM="$NOQERI_NQRI_VM"
run make test-noqeri-framework-local NOQERI_REPO="$NOQERI_REPO" NOQERI_WEB="$NOQERI_WEB"
run make test-noeqr-rest-local
run python3 tests/v3_vacuum_cli.py "$PWD/Native/rust/target/release/noesql"
run make benchmark-parser-reads
run make benchmark-bplus

printf '\nLocal engineering suites completed; logs: %s\n' "$LOG" | tee -a "$LOG"
printf '%s\n' 'NOT production approval: independent crash/power-fault, long multi-process soak,' \
  'security assessment, cross-platform qualification, and release provenance remain required.' \
  | tee -a "$LOG"
