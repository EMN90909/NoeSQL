# NoeSQL

NoeSQL is an experimental, beginner-friendly database language and a developing native database engine for [Noqeri](https://github.com/EMN90909/Noqeri).

> **Development snapshot — NOT production ready.** The JavaScript reference parser/storage and optional SQLite/PostgreSQL adapters are experimental. The Noqeri-native `.nqr` lexer/IR/executor/WAL/security modules have **not** been compiled and integrated with a verified Noqeri bootstrap compiler. Do not store important production data in the reference `.nqd` format.

## Current architecture and the Noqeri + Zig migration

- **Noqeri (`Database/*.nqr`)** is intended to own the parser, IR verification, execution semantics, and future native storage/WAL implementation. Follow the compiler and language conventions in [the Noqeri repository](https://github.com/EMN90909/Noqeri).
- **Zig 0.15.2 (`Native/zig/`)** is the new versioned C ABI bridge for a future compiled Noqeri engine. It enforces callback, ABI and buffer bounds. The Zig bridge does **not** yet run relational NoeSQL or persist pages.
- **C++17 (`Native/cpp/`)** remains the existing experimental native page-store and legacy bridge until byte-compatible Zig storage, recovery, auth, and fault tests are proven. The C++ kernel is not removed in this increment to avoid risking existing `.nqd` data.
- **JavaScript (`Runtime/noesql-reference.mjs`)** remains the reference oracle for semantic tests and future differential testing against the native engine.

Both the legacy C++ ABI bridge and new Zig ABI bridge have standalone fake-callback tests. Run `npm run test:zig` with Zig 0.15.2 and `npm run qualify:zig` for a fail-closed compiled-engine release gate. See [the Noqeri + Zig migration plan](Doc/NOESQL_ZIG_MIGRATION.md).

The ABI bridge and local fake-callback test are now present. The compiled Noqeri export, linker integration, and native CLI dispatch are **not yet connected or verified**. See [the integration plan](Doc/NOQERI_CPP_INTEGRATION.md). Run the C++ bridge test locally with `make -C Native/cpp test`; it verifies the bridge contract only, not the native database engine.

## Language example

```noe
db school
table users {
  id: int key auto
  name: text required
  age: int
}
users add {name:"Amina", age:21}
users get name, age where age >= 18 sort name take 10
```

## Readable joins and reporting (reference preview)

Use one logical line per query statement. No SQL `SELECT`/`FROM` boilerplate:

```noe
table teams {
  id: int key auto
  name: text required
}
table tickets {
  id: int key auto
  team_id: ref teams required
  effort: int required
}
teams add {name:"Build"}
tickets add {team_id:1,effort:5}
tickets get teams.name, count(*) as tickets, sum(effort) as effort join teams on team_id == teams.id group teams.name sort teams.name
alter tickets add { resolved: bool required = false }
```

This preview implements bounded **inner and left joins**, typed **count/sum/min/max**, `group`, aggregate `having`, and safe **add-column** migrations in the JavaScript reference engine and optional SQL adapters. Joined columns use `table.column` names. It does **not** implement the verified native Noqeri SQL executor, a cost-based SQL planner, physical reference indexes, decimal/currency types, MVCC, or production-qualified crash recovery.

```sh
npm test
npm run benchmark:reference
```

The benchmark runs an in-memory reference/SQLite parity workload and prints measurements for the current machine; it is **not** a native Noqeri or durability benchmark.

## Unified `.noedb` Zig page format (experimental)

The reference `.noedb` store and native Zig pager now share **NQDB0002 checkpoints and NQTX0002 WAL framing**. On Linux the reference adapter uses the same persistent flock lock as Zig. Native pages expose a checksummed `NQRS0001` application-state envelope, which the Zig ABI can validate. To test the Node codec: `npm run test:noedb`. The actual compiled Zig read/write round trip is checked by the Zig CI job using `scripts/verify-zig-page-roundtrip.mjs`, and must run successfully before claiming native interoperability qualification. `npm run qualify:native-e2e` still fails closed without the trusted Noqeri Stage-2 compiler and full compiled native SQL engine.

Legacy reference `.nqd`/`.nqdb` files remain separate and are converted explicitly: `node Runtime/noesql-cli.mjs migrate old.nqdb new.noedb`. The source is preserved, and an existing destination is rejected. The reference bridge supports at most 15 payload pages per atomic write and is not suitable for production data. See [wire format, migration and limitations](Doc/NOESQL_UNIFIED_NOEDB_FORMAT.md).

## Layout

- `Database/*.nqr`: native Noqeri source, unverified until compiled.
- `Native/zig/`: Zig ABI bridge + unit tests, migration work in progress.
- `Native/cpp/`: legacy C++17 page kernel and compatibility tests (not yet retired).
- `Runtime/*.mjs`: experimental JavaScript reference implementation and optional adapters.
- `Grammar/`: language grammar.
- `tests/`: reference tests, fixtures and pending native fixtures.
- `Benchmarks/`: native benchmark *source*; no measured performance claims.
- `Doc/`: compatibility details, threat model, integration plan, and remaining production-readiness gates.

## Run reference tests

Requires Node.js 22 or newer (SQLite tests use `node:sqlite`). Run from the repository root:

```sh
node --test tests/noesql-*.test.mjs
```

The acceptance suite `tests/noesql-acceptance.test.mjs` exercises persisted create/insert/read/update/delete, failed multi-statement rollback, and real POSIX subprocess `SIGKILL` at the reference store's critical commit/checkpoint hooks. These are process-crash tests, **not** power-loss or hardware-failure certification. A killed writer leaves its lock file behind by design; inspect the owner/state before manually removing a stale lock.

Reference CLI:

```sh
node Runtime/noesql-cli.mjs run ./school.nqd tests/school.noe
node Runtime/noesql-cli.mjs query ./school.nqd adults '{"minAge":20}'
node Runtime/noesql-cli.mjs verify ./school.nqd
```

Use test data only. SQLite/PostgreSQL adapters are optional; no GitHub Actions are required.

## Provenance

This source is synced from [Noqeri PR #12](https://github.com/EMN90909/Noqeri/pull/12), branch `feature/noesql-reference-and-native-lexer`, snapshot `713ca09af0b60ce1f1ba416ed8190174706d41c8` (15 Oct 2026).

## Release blockers

Native bootstrap and compiler tests; AST-to-NoeIR integration; the Zig ABI export/link path with compiled Noqeri; legacy C++ page-store migration; native durable page storage and WAL recovery; enforceable authentication/authorization; real transaction isolation and crash tests; physical indexes, joins and query planner; cross-platform security, fuzz and performance testing. See `Doc/NOESQL_IMPLEMENTATION_STATUS.md` and `Doc/NOESQL_SECURITY_GATES.md`.


## Cross-repository integration update (October 17, 2026)

Noqeri main now includes experimental NoeSQL native source modules, JavaScript reference runtime, WAL helpers, adapter files, and tests imported from PRs #11 and #12, without replacing more recent compiler/bootstrap implementations. The Noqeri Stage-2 CLI exposes `noqeri noe` as a reference runner; this **does not** imply native Noqeri code has been built or executed.

This repository's WAL frame functions now reject transaction IDs outside the unsigned 64-bit range, sequences outside the unsigned 32-bit range, and non-binary payloads rather than coercing arbitrary strings. Corresponding regression cases are in `tests/noesql-wal-validation.test.mjs`.

Run `npm test` and `make -C Native/cpp test` from a clean checkout, then compare results with Noqeri's embedded reference runtime. A full test run was not possible on the available worker due to GitHub DNS resolution failure. These are **experimental reference components, not a production database**.


## Native binary page-storage prototype (18 October 2026)

A separate, POSIX-only **C++17 page-store prototype** now lives at `Native/cpp/noesql_page_store.*`. It provides a 4 KiB versioned binary file format, persisted B+ tree key lookup, single-writer/shared-reader `flock` isolation, full-image redo WAL and checkpoint recovery, password-derived scrypt verifiers with revocable sessions, read-only/write roles, bounded operation quotas, a C storage ABI, crash-injection tests, and a local performance benchmark.

```sh
npm run test:native
npm run benchmark:native
make -C Native/cpp native-cli
```

**Important:** The C++ storage prototype is **not yet connected to the Noqeri-compiled relational SQL executor**. Its `.nqd` format is incompatible with the JavaScript reference `.nqd` format; no migration exists. This is not MVCC, is not cross-platform certified, and rewrites the complete page image on each commit. It must not store production or irreplaceable data. See [Native page engine technical scope and release gates](Doc/NOESQL_NATIVE_PAGE_ENGINE.md).

## Version 0.7.5 release candidate (18 October 2026)

**0.7.5-rc.1 is not stable or production-ready.** The native page kernel remains a separate experimental key/value layer rather than the compiled Noqeri relational engine. This iteration gates crash-test exits to test builds, caps index verification depth, adds a production-build fault-gate test, and includes an optional on-machine SQLite WAL/FULL comparator. The native tests passed locally; PostgreSQL parity and full cross-platform correctness have not been established.

See [the 0.7.5 release gates](Doc/NOESQL_075_RELEASE_GATES.md), [native SQLite benchmark methodology](Benchmarks/README-native.md), and [language usability comparison](Doc/NOESQL_SYNTAX_COMPARISON.md). No stable v0.7.5 tag is justified by these tests.


## Embedded and website storage (0.7.5-rc.2 preview)

A new optional **SQLite-backed** NoeSQL runtime is available for Node.js websites, desktop apps and server-side embedded databases. It preserves the existing readable `.noe` adapter and adds an advanced reporting dialect with JOIN (inner/left/right/full), GROUP, HAVING, typed parameters, indexed query plans and bounded results. Persistent compiler/catalog metadata survives application restart. See [the embedded/web guide](Doc/NOESQL_EMBEDDED_WEB.md).

```js
import {openNoeSQLEmbedded} from './Runtime/noesql-embedded.mjs'
const store = await openNoeSQLEmbedded({file:'/var/lib/app/content.sqlite'})
const rows = store.query('from users\\nselect id, name\\nwhere id == $id', {id:1})
store.close()
```

The private HTTP example is in `Examples/embedded-web.mjs`. Never expose database service keys to browsers. To test: `npm run test:embedded`; to benchmark the language overhead against direct prepared SQLite: `npm run benchmark:embedded`. These results are **not a claim that the native NoeSQL engine matches SQLite performance**. The independent native `.nqd` page kernel is unchanged, SQLite WAL must use a persistent local disk, and neither backend has completed all production release gates. Node's `node:sqlite` API remains experimental in Node.js 22.


## v1.0 remediation in progress: language/data extensions (18 October 2026)

Language scripts now prefer **`.nqd`** (the former `.noe` remains a CLI compatibility alias). New database data files prefer **`.nqdb`**. Existing experimental JSON or native-binary **`.nqd` data** stays explicitly supported by its respective legacy reader; it is *not* automatically interpreted as a script or converted. Pass script and database paths in the correct CLI positions. **Back up legacy data before migration**; no lossless cross-format migration tool exists yet.

Example reference-only command: `node Runtime/noesql-cli.mjs run ./scratch.nqdb tests/school.nqd`.

This change begins hardening but does **not** make NoeSQL a unified native Noqeri+Zig database. Zig adds a **read-only legacy WAL verifier** alongside its legacy page verifier. C++ remains the compatibility storage kernel; JS and SQLite remain experimental adapters. Native relational integration, incremental WAL, snapshot transactions, cross-platform fault testing, and actual compiler bootstrap proof are still blockers. See [v1 remediation plan](Doc/NOESQL_V1_REMEDIATION_PLAN.md) and [security assessment](Doc/NOESQL_SECURITY_REMEDIATION_REVIEW.md).


## Stage 2 native Zig pager (experimental)

New `Native/zig/src/pager_v2.zig` adds Linux-only checksummed incremental WAL transactions, replay, checkpoint, bounded in-process snapshots, and a separate v2 C page ABI. `Native/zig/src/index_v2.zig` introduces a bounded physically persisted unique numeric index. `Database/noesql_parser.nqr` is synchronized from the current Noqeri source, while `scripts/verify-noqeri-bootstrap.mjs` validates pinned Stage-2 proof inputs. **No native Noqeri statement compiler/executor has been linked**; the old C++ and JS compatibility paths remain. Do not claim release qualification. See [Stage 2 storage and blockers](Doc/NOESQL_STAGE2_ZIG_STORAGE.md).

## v0.7.6: component API 1.0.0 hardening

The experimental Zig pager, fixed-capacity index, Noqeri compiler-proof checker, and narrow SQLite comparison benchmark have a **component API 1.0.0** development milestone. This does not mean the components are storage-engine production-qualified. The on-disk NQDB0002 magic and C ABI v2 remain unchanged to preserve Stage 2 compatibility.

The Zig build now installs `libnoesql_zig_pages.a` containing `noesql_zig_page_*` exports independently from the Noqeri callback bridge. The C benchmark links the pager library rather than the callback-only library. Test-only fault injection and stricter WAL frame validation cover additional recovery paths, while the index has O(log n) range positioning.

**npm status:** package entry points and CLI are defined for inspection via `npm pack --dry-run`; the repository remains `private:true` and is **not verified to be published to npm**. The command `npm install noesql@0.7.6` must not be advertised as successful until actual registry publication and installation are confirmed.

**Compiler status:** Noqeri's current portable Stage-2 CLI can check source but has not produced a verified, linkable native relational executor targeting the Zig pager. NoeSQL retains explicit experimental adapters; this 0.7.6 update does not silently switch them to the incomplete native path.

See [component hardening and outstanding integration gates](Doc/NOESQL_076_COMPONENT_GATES.md).

## Native executor and npm release qualification (follow-up)

`npm run qualify:native-e2e` now requires a **genuine pinned native Noqeri-produced binary** with a Noqeri compiler Stage-2 proof, and runs a durable Zig-backed .nqd CRUD/restart oracle comparison. The runner **fails** with the current unverified Noqeri compiler; a JavaScript reference CLI is explicitly rejected as the native binary. Its documented protocol includes `identity --json`, `run`, `query`, `verify` and `checkpoint`. No native runtime implementing that full protocol has been built or verified in this release.

`npm run test:npm-pack` tests a **locally packed tarball** by installing it into a fresh offline project and executing the current SQLite-backed NoeSQL API. This is distinct from testing a real npm registry release. NoeSQL remains `private:true`, no verified npm publication credentials are connected, and a production-compatible license file must be selected by the repository owner before releasing sources publicly. `npm run qualify:zig` rejects caller-selected fake conformance commands and retains fail-closed durability and cross-platform gates.

See `Doc/NOESQL_NATIVE_NPM_PUBLICATION_GATE.md`.


## Public npm publication preparation

The v0.7.6 package now carries GPL-3.0-only licensing and public npm metadata, matching the upstream Noqeri GPL-3.0-only source license. The prior `private:true` blocker has been removed, **but publication is still blocked by `prepublishOnly`** until a release maintainer explicitly authorizes the exact commit and validates native Noqeri→Zig execution. Distribution of copied upstream or third-party code remains subject to copyright/notice review; this metadata change does not grant rights to code owned by others.

The `npm` trusted-publishing workflow is manually triggered, targets the experimental `next` dist-tag, and cannot succeed without a verified compiler seed, native executable, runner tests, and npm publisher identity. The private GitHub repository cannot generate npm's public-source provenance attestations. No public npm publication is claimed in this source commit.
