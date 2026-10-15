# NoeSQL

NoeSQL is an experimental, beginner-friendly database language and a developing native database engine for [Noqeri](https://github.com/EMN90909/Noqeri).

> **Development snapshot — NOT production ready.** The JavaScript reference parser/storage and optional SQLite/PostgreSQL adapters are experimental. The Noqeri-native `.nqr` lexer/IR/executor/WAL/security modules have **not** been compiled and integrated with a verified Noqeri bootstrap compiler. Do not store important production data in the reference `.nqd` format.

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
- `Runtime/*.mjs`: experimental JavaScript reference implementation and optional adapters.
- `Grammar/`: language grammar.
- `tests/`: reference tests, fixtures and pending native fixtures.
- `Benchmarks/`: native benchmark *source*; no measured performance claims.
- `Doc/`: compatibility details, threat model, remaining production-readiness gates.

## Run reference tests

Requires Node.js 22 or newer (SQLite tests use `node:sqlite`). Run from the repository root:

```sh
node --test tests/noesql-*.test.mjs
```

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

Native bootstrap and compiler tests; AST-to-NoeIR integration; native durable page storage and WAL recovery; enforceable authentication/authorization; real transaction isolation and crash tests; physical indexes, joins and query planner; cross-platform security, fuzz and performance testing. See `Doc/NOESQL_IMPLEMENTATION_STATUS.md` and `Doc/NOESQL_SECURITY_GATES.md`.
