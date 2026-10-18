# Reference storage pathname hardening (2026-10-18)

The experimental JavaScript reference store now refuses to overwrite a pre-existing snapshot symlink or multiply-linked file, both before taking the checkpoint and immediately before rename. The source-size restriction and bounded descriptor reads remain in place. The tests add new-file and checkpoint-substitution regressions.

This **does not** close the full hostile-filesystem threat model. The final check and rename are not atomic with respect to a concurrent attacker, and parent directories are not securely traversed with directory file descriptors. Store experimental data only in a private directory inaccessible to untrusted writers. SHA-256 is a corruption checksum, not a MAC; no authorization or encryption is provided.

Full native Noqeri engine integration, MVCC/isolation proofs, indexed page storage, robust crash recovery, cross-platform tests, benchmarks and external review remain necessary before claiming maturity or production readiness.

Run the checks locally:

```sh
npm test
npm run test:cpp-bridge
```

The focused hardening tests are in `tests/noesql-path-hardening.test.mjs`. No benchmark or native Noqeri compiler result is claimed by this change.
