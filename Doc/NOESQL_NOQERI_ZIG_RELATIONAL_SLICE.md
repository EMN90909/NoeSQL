# Noqeri → Zig relational source-level integration

`Database/noesql_zig_relational.nqr` is a **new actual typed execution slice** tying together `noeApiValidate` (tenant/auth policy), `noeExecute` (Noqeri typed relational IR), `noeRowEncode`/`noeRowDecode` (versioned row format), and the Zig v2 pager C ABI. It supports one two-column integer/boolean row at a time on one page and GET/INSERT/SET/DELETE with Zig durable commit, rollback on errors and read verification. It never stores raw SQL text in the page engine. All data remains experimental and separate from older disk formats.

**Important:** this is real Noqeri-owned source integration, **not a built native executable**, a complete Noqeri statement frontend, or multirow scalable relational storage. There is no trusted stage0 stage1→stage2→rebuild evidence in the current checkout and the linked native executable has not been produced. The module therefore does **not** satisfy the end-to-end release gate `npm run qualify:native-e2e`; user data must not be placed in this experimental format. The module's scope is intentionally narrower than the reference language's catalog, joins, group, indexes, and expression grammar.

The expected next native compiler step remains build provenance and an object writer/linker with verified machine code, then native CLI integration using this bounded entrypoint. See `Doc/NOESQL_NATIVE_NPM_PUBLICATION_GATE.md` for publication conditions.

Native smoke fixture: `tests/noesql_zig_relational.nqr`. No compiler-produced object or runnable binary has been verified.
