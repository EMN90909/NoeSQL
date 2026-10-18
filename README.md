# NoeSQL

NoeSQL is an experimental, beginner-friendly database language and a developing native database engine for [Noqeri](https://github.com/EMN90909/Noqeri).

> **Development snapshot — NOT production ready.** The JavaScript reference parser/storage and optional SQLite/PostgreSQL adapters are experimental. The Noqeri-native `.nqr` lexer/IR/executor/WAL/security modules have **not** been compiled and integrated with a verified Noqeri bootstrap compiler. Do not store important production data in the reference `.nqd` format.

## Engine architecture: Noqeri + C++ + JavaScript

- **Noqeri (`Database/*.nqr`)** is intended to own the parser, IR verification, execution semantics, and future native storage/WAL implementation. Follow the compiler and language conventions in [the Noqeri repository](https://github.com/EMN90909/Noqeri).
- **C++17 (`Native/cpp/`)** provides a small versioned C ABI bridge for a compiled Noqeri engine. It checks ABI compatibility and buffer bounds; it must not become a second SQL/database implementation.
- **JavaScript (`Runtime/noesql-reference.mjs`)** remains the reference oracle for semantic tests and future differential testing against the native engine.

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

This preview implements bounded **inner joins**, typed **count/sum/min/max**, `group`, and safe **add-column** migrations in the JavaScript reference engine and optional SQL adapters. Joined columns use `table.column` names. It does **not** implement native Noqeri compilation, outer joins, physical persisted indexes, decimal/currency types, concurrent snapshot isolation, authenticated principals or production WAL recovery.

```sh
npm test
npm run benchmark:reference
```

The benchmark runs an in-memory reference/SQLite parity workload and prints measurements for the current machine; it is **not** a native Noqeri or durability benchmark.

## Layout

- `Database/*.nqr`: native Noqeri source, unverified until compiled.
- `Native/cpp/`: C++17 ABI bridge and local contract test.
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

Native bootstrap and compiler tests; AST-to-NoeIR integration; the C++ ABI export/link path; native durable page storage and WAL recovery; enforceable authentication/authorization; real transaction isolation and crash tests; physical indexes, joins and query planner; cross-platform security, fuzz and performance testing. See `Doc/NOESQL_IMPLEMENTATION_STATUS.md` and `Doc/NOESQL_SECURITY_GATES.md`.


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
