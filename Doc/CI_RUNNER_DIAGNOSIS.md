# GitHub Actions runner and local build qualification

## Current blocker

The native-validation and session-integration workflows have repeatedly completed with `conclusion=failure` before recording a single job step. The observed job metadata has no normal runner assignment or usable build log. That is a **runner-provisioning/account-eligibility symptom**, not a compiler error report. Source-code changes cannot guarantee a GitHub-hosted machine will be assigned.

Do not mark the native migration green until the exact PR head has an **executing and passing** GitHub Actions run or equivalent recorded local builds and conformance output.

## Repository/account checks

With repository admin access, inspect:

1. **NoeSQL → Actions → the failed run → annotations** for runner assignment, billing, or disabled-workflow reasons.
2. **NoeSQL → Settings → Actions → General** for Actions enablement and allowed actions.
3. **GitHub account → Billing and licensing → Actions** for account spending limits, payment verification, and minute quota.
4. Runner eligibility for this private repository. If hosted runners are unavailable, provision an **authorized trusted** Linux x86-64 self-hosted runner and configure its labels intentionally. Never run untrusted pull-request code on a sensitive host.

The workflows in this PR use `ubuntu-24.04`, support manual `workflow_dispatch`, pin Zig to **0.15.2**, and pin the separate Noqeri compiler source to **4842fa34dc06afe39b4ba216b52c8ec7bb576984**. Changing runner labels alone will not fix disabled Actions or account billing restrictions.

## Local execution gates

On Linux x86-64 with a working Rust toolchain, Zig 0.15.2, CMake, and a C++17 compiler **only for bootstrapping the separate Noqeri compiler**:

```bash
bash scripts/verify-ecosystem-local.sh
zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
NOESQL_ZIG_LIB_DIR="$PWD/Native/zig/zig-out/lib" \
  cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager
```

Then build the separate Noqeri compiler, run the Stage-2/Stage-3 hash/equality proof, compile the NoeSQL AOT module, and execute the two native chains and cross-mode test as laid out in `.github/workflows/reference-tests.yml`.

## Release limits

The repository now contains **zero C/C++ source/header files**. NoeSQL nevertheless depends on the separate Noqeri Stage-0 bootstrap toolchain for the current AOT route. C ABI linkage is a calling convention, not a claim that NoeSQL itself contains C source.

Storage qualification still requires full crash/power-loss testing, concurrency interleavings, fuzz/property tests, filesystem/platform checks, and format-migration fixtures. No database can be guaranteed free from all corruption merely by passing its test suite.


## 2026-10-25: What can and cannot be fixed in repository code

The hosted jobs on main and benchmark branches continue failing before runner
allocation: `runner_id=0`, no job steps and no build/test logs. Adjusting
`zig build`, Rust source, job steps, or checkout actions cannot repair an
account-level runner/billing authorization denial. GitHub repository owners
must inspect **Settings > Actions > General / Runners** and account
**Billing and licensing > Actions**. If the job details report a failed
payment, exhausted spending budget, or Actions minute eligibility restriction,
resolve that account problem or provision an authorized self-hosted Linux runner.

Do **not** label NoeSQL as passing CI while runner assignment is blocked.
After the runner is active, run `make test`, `make stress`, native Noqeri
Stage-2/AOT compatibility checks and the correctness-gated SQLite benchmark.
Treat failures as real compiler/runtime defects and fix them based on full logs.

Additional data-integrity risks addressed in fix/snapshot-data-integrity-20261025:
a manifest that references a missing payload page must return corruption rather
than NOT_FOUND; snapshot shrink rewrites now overwrite old pages with zeros in
the same transaction; a snapshot containing unexpected trailing nonzero pages
must fail validation; snapshot checkpoint files containing undeclared trailing
bytes must be rejected; index pages require canonical padding; raw-row code
rejects unmarked populated stores; read-only CLI queries must not create
spurious WAL transactions.

These are source changes and *proposed* regression tests, not a claim that
Zig/Rust/Noqeri builds have passed. Memory and disk sanitization is not
cryptographic erasure of historical WAL blocks or flash storage.
