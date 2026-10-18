# NoeSQL 0.7.6: component API 1.0.0

**This is an experimental source milestone, not four certified stable components and not a NoeSQL v1.0 database.**

## Implemented
- Transaction pager: test-only fault points before/after WAL append and checkpoint rename; replays only valid checksummed commits; historical page-id and duplicate validation even after checkpoint.
- Index: logarithmic lower and upper search for sorted integer-key range positioning, without changing the fixed-capacity physical page format.
- Noqeri compiler proof verifier: exact Stage-2 source closure, expected revision and stage artifact digests; still cannot authenticate a self-reported proof without independent signed provenance.
- Zig versus SQLite benchmark: separate page C library installed and linked; verify actual SQLite WAL+FULL settings prior to measurement.
- Package: 0.7.6 version/exports, npm pack inspection while private. npm publication and integrity verified installation are not claimed.

## Required verification before any v1 production statement
1. Linux Zig 0.15.2: `zig build test && zig build -Doptimize=ReleaseSafe` from `Native/zig/`. Verify `nm -g zig-out/lib/libnoesql_zig_pages.a` exposes `noesql_zig_page_open`.
2. Run `npm test` and `npm run test:component-contract`; C benchmark: `bash scripts/benchmark-zig-v2.sh 250`, repeated independently on identical SSD and process isolation. No benchmark results exist yet.
3. Verify a trusted Noqeri stage0->stage1->stage2->rebuild proof, then compile real `.nqr` typed SQL statement parser/executor into a native binary and link it against the Zig page ABI. The Noqeri portable Stage-2 CLI currently documents only source checking/restricted browser output; source declarations do not satisfy this proof.
4. Crash/ENOSPC/short-write fault injection at every WAL & checkpoint syscall and physical recovery; backup restore, multi-process concurrency, authenticated encryption, resource ceilings, row authorization, persistent planner/index growth beyond 337 integer keys.
5. Cross-platform CI with real job steps and logs, reproducible signed artifacts/SBOM and verified npm publication to make `npm install noesql@0.7.6` work.

**Do not upgrade release qualification or claim full Noqeri + Zig SQL semantics based on package or component version labels.**
