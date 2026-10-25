# Private GitHub Actions runner qualification blocker

The latest NoeSQL pull-request runs on 25 October 2026 show the same infrastructure failure seen earlier: both the native validation job and the .noedb session job finish with `conclusion=failure`, `runner_id=0`, no runner name, and an empty step list.

That means GitHub never provisioned an `ubuntu-latest` runner. No checkout, compiler, test, or build command executed. These runs are therefore neither passing nor failing evidence for the Noqeri + Rust + Zig migration.

## What maintainers must check

Verify GitHub Actions eligibility and provisioning in Repository -> Settings -> Actions -> General and in the account's Actions billing/usage settings. Review workflow-run annotations for disabled Actions, hosted-runner availability, spending limits, or organization restrictions.

If GitHub-hosted runners are intentionally unavailable, configure an authorized self-hosted Linux runner and update the workflow `runs-on` label explicitly. Do not remove tests or weaken the source-stack gates to manufacture a green badge.

## Trusted local validation

On a Linux machine with Rust, Cargo, Zig 0.15.2, GNU Make, a C assembler/linker for Noqeri's current AOT assembly output, and a working Noqeri compiler:

```bash
make test

make native-chain \
  NOQERI_COMPILER=/absolute/path/to/noqeri

make stage2-chain \
  NOQERI_REPO=/absolute/path/to/Noqeri \
  NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri
```

The source-stack gates are:

```bash
make no-js no-native-c-family
```

They reject JavaScript and C/C++ implementation sources from NoeSQL. The two `.h` files under `Native/zig/` are ABI declarations for the Zig storage library, not a C implementation layer.

## Release gate

Require recorded passing output on the exact integration commit before calling the migration green-tested or production-qualified. The PR may be structurally mergeable while runner provisioning is broken, but mergeability is not test evidence.

Native self-hosted Stage-2 provenance remains a separate Noqeri compiler milestone. The current direct AOT path is deliberately scalar and Linux x86-64 qualified until Noqeri's portable native export ABI is broader.
