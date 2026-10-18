# NoeSQL embedded and web-server runtime (0.7.5 release candidate)

**Status: experimental integration; not a production approval.** This is an optional, explicitly **SQLite-backed** runtime for Node.js websites and embedded server/desktop applications. It does not make the existing C++/Noqeri `.nqd` page kernel as fast as SQLite, does not imply independent NoeSQL durability guarantees, and does not implement browser-local storage. SQLite and NoeSQL native `.nqd` database files are **not interchangeable**.

## Why this backend

The current independent native `.nqd` prototype rewrites a full image at every commit. SQLite already provides incremental pages, physical B-trees and secondary indexes, transactional WAL, and reader snapshots. The optional backend gives developers these working features now without sacrificing the beginner-friendly NoeSQL syntax. `Runtime/noesql-embedded.mjs` delegates existing `.noe` schema/CRUD to the repository's `createSqliteAdapter` and adds an expressive multi-line reporting dialect compiled to validated prepared SQLite statements. There are **no new npm packages**.

Requires Node.js 22.13+ and its still-experimental `node:sqlite` API (tested locally with Node 22.16.0 and SQLite 3.49.1). The embedded API is synchronous: complex scans can block the Node.js event loop. Use worker threads and a trusted service boundary for heavier workloads. The runtime has not been qualified across OSes or high-load web deployments.

## Embedded quick start

```js
import {openNoeSQLEmbedded} from './Runtime/noesql-embedded.mjs'
const db=await openNoeSQLEmbedded({file:'/var/lib/myapp/content.sqlite',maxRows:100})

// Run ONCE as a migration. The catalog persists type/named-query definitions.
db.run(`table users {
  id: int key auto
  name: text required
}
query everyone() { users get id, name }`)
db.run('users add {name:"Ada"}')
db.createIndex('users',['name'])
console.log(db.query('from users\nselect id, name\nwhere name == $name',{name:'Ada'}))
db.close()
```

On subsequent starts, **do not re-run a `table` declaration**. The internal `_noesql_catalog` stores the original compiler AST when schema/named-query declarations commit, and reloads it when reopening the file. Migrations should be versioned and applied once. Databases created using older SQLite adapters may lack the catalog; they need a separately verified schema import rather than automatically replaying CREATE TABLE. You can call `db.named('everyone')` after reopening a cataloged file.

`db.transaction(tx => { ... })` executes a **synchronous** atomic batch. Operations are `tx.insert(table, values)`, `tx.update(table, values, where)`, `tx.remove(table, where)`, and `tx.query(source,params)`. Writes require nonempty equality `where` objects; no accidental update-all/delete-all. Do not call `db.run()` inside `transaction()`, since the original `.noe` adapter owns its own SQLite transaction. `db.database` is an explicitly trusted server-side migration escape hatch and MUST NOT be exposed over HTTP.

### Advanced query language

One clause per line, readable from top to bottom:

```text
from orders
left join users on orders.user_id == users.id
left join teams on users.team_id == teams.id
select teams.name as team, count(orders.id) as purchases, sum(orders.amount) as revenue
where orders.status == $status
where users.active == $active
group teams.name
having purchases >= $minimum
sort revenue desc, team asc
take 20
skip 0
```

Call `db.query(source,{status:'paid',active:true,minimum:2})`. Supported features:

- 1 to 4 **INNER, LEFT, RIGHT or FULL** equijoins. SQLite 3.39+ required for RIGHT/FULL.
- Typed named parameters `$name`; no caller-supplied raw SQL, table names, operators, or query fragments.
- `select *` for unjoined tables; qualified projections with optional `as` aliases; `distinct`.
- `count`, `sum`, `avg`, `min`, `max`; `group` and `having` with validated aliases.
- `where` / `having` equality, inequality, comparisons, `is null` / `is not null`; `and` within a clause and repeated filters.
- Multi-key `sort asc|desc`, `take`, `skip`, enforced row ceilings, schema-checked identifiers, LRU query and mutation caches, and `db.explain()`.
- Indexed reads use physical SQLite B-trees; `db.createIndex()` creates a real SQLite index.

Unsupported today: arbitrary subqueries, unions, window functions, CTEs, custom expressions, nested OR groups, stored procedures, online schema migration, and transparent parity with every PostgreSQL SQL feature. The independent Noqeri compiler/parser does **not** yet understand this reporting dialect. The query language remains preview-only, and should not be confused with the entire standard SQL grammar.

## Website/API integration

`Runtime/noesql-web.mjs` exports `createNoeSQLHttpHandler({database,keys,requestsPerMinute,maxBodyBytes})`. `Examples/embedded-web.mjs` demonstrates running a private Node HTTP service bound to localhost. Deploy it **behind an authenticated application and a TLS reverse proxy**. Configure:

```sh
export NOESQL_DB_PATH=/var/lib/myapp/content.sqlite
export NOESQL_READER_KEY="$(openssl rand -hex 32)"
export NOESQL_WRITER_KEY="$(openssl rand -hex 32)"
export NOESQL_ALLOWED_TABLES=users,orders
node Examples/embedded-web.mjs
```

- `GET /health`: liveness only.
- `POST /v1/query`: `{"query":"from users\nselect id, name\ntake 10","params":{}}`.
- `POST /v1/mutate`: `{"operations":[{"kind":"insert","table":"users","values":{"name":"Ada"}}]}`.
- Header: `Authorization: Bearer <server-side-secret>` and `Content-Type: application/json`.
- Reader secrets cannot call mutate; writer secrets can. Optional `tables` allowlists restrict both reads (including joins) and mutations; the example requires `NOESQL_ALLOWED_TABLES`. Unknown credentials are rejected using constant-time digest comparisons.
- Limits: request size, per-key in-process requests/minute, at most 100 mutations per request, parameter size, allowed query structure, and output rows.

**Security restrictions:** Do not put either key in browser JavaScript, HTML, or a public `VITE_*`/`NEXT_PUBLIC_*` variable. These static API keys are server-to-server service credentials, **not end-user login, tenant RLS, row-level permissions, or a full authorization system**. The application must authenticate its users, authorize allowed tables/rows and operations, prevent public untrusted query text reaching this private API, configure TLS, and apply independent ingress rate limits. In-process per-key throttling does not cover invalid-token floods or multiple replicas. There is currently no database-level query CPU/memory interrupt: a computationally expensive aggregate may block the synchronous Node thread. Run behind a controlled work queue/isolation boundary. Never advertise the HTTP handler as a production-hardened public database API.

**Hosting:** The `.sqlite` database, WAL, and shared-memory files must be on a persistent writable **local** filesystem. Do not run SQLite WAL on network filesystems or share it between separate host machines. On Render and similar PaaS platforms, ephemeral container storage is **not** a durable database volume; configure persistent disk or use an external managed datastore before storing customer data. Horizontal web replicas cannot share a single local SQLite WAL file; use an explicit central backend design.

## Testing and benchmarks

```sh
node --test tests/noesql-advanced.test.mjs tests/noesql-web.test.mjs tests/noesql-sqlite-durability.test.mjs
node --test tests/noesql-embedded-integration.test.mjs # needs existing Runtime/noesql-adapters.mjs
npm run benchmark:embedded
npm test
make -C Native/cpp test
```

Local test coverage includes 13 advanced/web/WAL tests (including forced SIGKILL after commit), plus a separate catalog integration test that must be run in the full repository checkout. `Benchmarks/embedded-vs-sqlite.mjs` compares direct prepared SQLite operations with NoeSQL's additional validation/compiler API **using the same SQLite engine**, in-memory. It measures language/API overhead, NOT independent storage-engine speed or disk-durability throughput. Results vary across machines and dataset sizes. In one Node 22.16.0 run on 10,000 records and 3,000 reads, direct SQLite completed about 384k lookups/s vs 107k NoeSQL-readable lookups/s, while 5,000-row batches reached about 1.02M rows/s direct vs 264k through NoeSQL. It is **not true** that the NoeSQL API matches SQLite's raw speed yet. Production load tests, filesystem durability failure injection, resource exhaustion/fuzzing, independent audits, backups/restore, and native Noqeri integration remain incomplete.

## Release qualification

Remain at **v0.7.5 release-candidate** until remaining release gates pass. In particular, this optional SQLite-backed mode must not be presented as a completed native page-engine rewrite or certified secure multi-tenant web database. See `Doc/NOESQL_075_RELEASE_GATES.md`.
