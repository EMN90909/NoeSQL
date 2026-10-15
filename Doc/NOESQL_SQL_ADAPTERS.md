# NoeSQL optional SQLite / PostgreSQL adapters (experimental)

The authoritative NoeSQL database format remains `.nqd`. Neither SQLite nor PostgreSQL is required by the NoeSQL reference storage engine, and this compatibility code is **not** the native Noqeri execution engine.

`Runtime/noesql-adapters.mjs` compiles a bounded subset of NoeSQL's parsed AST into SQL with **bound values** and **validated quoted identifiers**, rather than interpolating application-provided values. It supports `table`, ordinary composite `index`, `add`, `get`, predicate-filtered `set` and `delete`, `atomic` blocks and named `query` declarations. It rejects unsupported SQL types and expressions instead of silently changing their meaning. Statements in each `run` call use an SQL transaction and roll back on failure. SQLite uses null-last ordering with primary-key tie-breaking for sorted results, and enforces foreign-key references with `PRAGMA foreign_keys=ON` and returns decoded typed booleans. Prepared query parameters are type checked.

### SQLite (Node.js 22+)

```js
import { createSqliteAdapter } from './Runtime/noesql-adapters.mjs'
const db = await createSqliteAdapter('school.sqlite')
try {
  db.run(`table learners { id: int key auto
    name: text required
    age: int
  }
  query adults(minAge: int = 18) {
    learners get name, age where age >= $minAge sort name
  }
  learners add { name: "Amina", age: 20 }`)
  console.log(db.query('adults', {minAge: 18}).rows)
} finally { db.close() }
```

Node's built-in `node:sqlite` is experimental in supported Node 22 builds; that provider API needs its own compatibility release gate.

### PostgreSQL (optional `pg` package and live server)

```js
import {createPostgresAdapter} from './Runtime/noesql-adapters.mjs'
const db = await createPostgresAdapter({connectionString:process.env.DATABASE_URL})
try { console.log(await db.run('table learners {id: int key auto\nname: text required\n}')) }
finally { await db.close() }
```

The adapter uses PostgreSQL positional `$1...` parameters. SQL translation is covered by offline tests, but **a live PostgreSQL deployment, concurrency, types and migrations have not been tested**.

### Known limits

- Type surface is `int`, `bool`, `text`, and single-integer-key `ref`. No decimal/money, array/JSON/vector, multi-key FKs, schema migrations, full relationship paths, analytical joins or grouping yet.
- `int` is the JS-safe-integer subset; full portable signed 64-bit coverage requires a native bigint/row conversion layer.
- `run` transactions are atomic on the SQL server, but the adapter keeps an in-memory schema/query catalog that is not persisted to an adapter metadata table. Reopening a SQLite/PostgreSQL connection does not automatically reconstruct typed named-query definitions.
- The adapters parse NoeSQL using the experimental JavaScript conformance oracle; they are **not** Noqeri-native and are not a production release substitute.
- Do not claim this work establishes verified full-stack, OS, native-compilation, cryptographic WAL, or crash-safe engine readiness.

### Local verification (no GitHub Actions)

```sh
node --test tests/noesql-*.test.mjs
```

For a native release also run the compiled Noqeri tests and the stage1->stage2->stage3 self-hosting proof on each supported platform, and fault-inject the actual native `.nqd` WAL host provider.
