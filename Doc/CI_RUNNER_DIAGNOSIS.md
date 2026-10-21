# Private GitHub Actions runner qualification blocker

The observed `NoeSQL .noedb session integration` Actions jobs on 2026-10-21 completed with `conclusion=failure`, **zero executed steps**, `runner_id=0`, and no runner name. The job log endpoint did not yield a normal build log. This means the native/session tests **did not run**; it is not evidence that those tests passed or failed.

This symptom cannot be resolved by changing NoeSQL C++ or JavaScript logic alone. Repository/account maintainers must verify GitHub Actions runner eligibility and provisioning in **Repository → Settings → Actions → General** and the account's **Billing and licensing → Actions** settings. Check Actions workflow-run annotations for disabled Actions, unavailable hosted runners, billing limits, or enterprise restrictions. If hosted runners are intentionally unavailable, configure an authorized self-hosted runner and explicitly update the workflow's `runs-on` label. Do not disable tests to get a green badge.

The identical native and reference checks can run on a trusted local Linux machine:

```bash
node --version          # Node 22+
bash scripts/verify-ecosystem-local.sh
```

This executes `tests/noesql-session.test.mjs`, `tests/noesql-migrate-noedb.test.mjs`, `tests/noesql-canonical-source.test.mjs` and `make -C Native/cpp test`. It requires a C++17 compiler, OpenSSL and SQLite development headers, and the existing NoeSQL reference-store prerequisites.

**Release gate:** require recorded passing test output on the *actual integration commit* before merging or publishing. Stage-2/Noqeri-to-native linking and native Zig E2E remain separate gates even after these scoped tests succeed.

## Cross-repository privacy

`EMN90909/NoeSQL` is private. The public Registry must not vendor its proprietary source or request private secrets during ordinary public build/test. The Registry's manual private integration workflow uses a read-only `NOESQL_READ_TOKEN` secret and an isolated checkout; a maintainer must explicitly configure the secret. Avoid embedding credential values in git or logs.
