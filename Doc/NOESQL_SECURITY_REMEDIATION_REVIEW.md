# NoeSQL security review and initial remediation (2026-10-18)

Method: Vibe Code Security Reviewer threat modeling, authentication/authorization, API abuse, API key exposure, secret protection, storage, and verification checks.
Scope: current repository NoeSQL (JavaScript embedded/web, legacy C++ storage, Zig bridge, Noqeri native source) compared against uploaded 18-finding audit.

## Threat boundaries
Assets: user data, metadata catalog, database password verifiers, WAL/checkpoints, compiler provenance, prepared plan and executable output.
Attackers: anonymous HTTP clients, compromised or overly privileged API-key holders, untrusted filesystem writers, malformed scripts/pages, compromised build inputs.
High trust: internal SQLite metadata table, native file IO and lock state, Noqeri compiler seed, Zig ABI callback, HTTP authorization check.

| ID | Severity / confidence | Evidence and impact | Mitigation / verification |
|---|---|---|---|
| S01 | HIGH / confirmed | Public CRUD accepted `_noesql_catalog`; writer can corrupt metadata and deny restart. | Reject `_noesql_` for CRUD and index operations; negative tests for insert/update/delete/index and clean reopen. |
| S02 | HIGH / confirmed | Native 0-byte WAL after crash blocked future write (availability). | Clean under exclusive native lock, sync directory; C++ regression test. |
| S03 | HIGH / confirmed | Seed mere existence and arbitrary command exit could fabricate a qualified release. | Hash-pin compiler seed and make native-integrated durable-engine gate fail-closed pending authentic proof. |
| S04 | HIGH / known design gap | Key-only HTTP auth and in-process rate limits are not end-user permissions or safe direct public exposure. | Keep behind TLS, authenticated application boundary, external ingress throttling; add tenant/RLS/cancel and audit before public server. |
| S05 | MEDIUM / confirmed | Legacy paths check final symlink but not all hostile-parent-directory races. | Require private trusted filesystem directory; future openat/renameat dirfd model and attack tests. |
| S06 | HIGH / known design gap | Unkeyed checksums cannot resist a malicious file writer; verifiers at rest can face offline attack. | Specify encryption/key-management model and privilege isolation; do not claim tamper-proof or encrypted data. |
| S07 | HIGH / incomplete evidence | Native Noqeri sources not integrated/compiled; no data plane proof for auth, cancellation, bounds. | Negative mutation/access cases against real compiled binary, property tests, ASan/UBSan and fault corpus. |
| S08 | MEDIUM / confirmed | Journal mode in SQLite was previously assumed, not verified. | Verify actual WAL/FULL for disk-backed runtime and report in-memory backend truthfully. |

## What changed / not changed
The first batch only addresses S01, S02 and S08 in code, and strengthens S03's guard. It does NOT establish a secure public database service, independent WAL durability, encryption, tenant isolation, or compiled Noqeri native conformance.
No production secrets were inspected or exported. Existing constant-time scrypt verifiers, parameterized SQL, and request size caps remain valuable defense layers.

## Release blocker
Require negative authorization proof, native crash/IO fault and source-format fuzz corpus, Zig/Noqeri compiled integration, verified CI, independent security review, and reproducible clean artifact. Do not tag v1.0 on static changes alone.
