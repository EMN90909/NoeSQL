# Trusted native Noqeri ↔ NoeSQL relational ABI (experimental)

This additive Rust `staticlib` symbol allows an AOT-compiled Noqeri native
executable to invoke the complete Rust NoeSQL relational evaluator and Zig WAL
without a CLI subprocess. It uses the real relational parser/executor: tables,
joins, aggregates, constraints, transactions, and existing planner behavior.
**It is not a Noqeri NIR relational code-generation pipeline, and is not
production authentication.**

## Symbols and memory layout

- `noesql_relational_abi_version() -> u32` returns `1`.
- `noesql_relational_exec_v1(path_ptr, path_len, source_ptr, source_len,
  bindings_ptr, binding_count, output_ptr, output_capacity, written_ptr) -> i32`.
- Every length is native `usize`, pointers are borrowed for the duration
  of the synchronous call. Return status: `0` success; `-1` invalid arguments,
  `-2` invalid UTF8, `-3` output too small, `-4` invalid bindings,
  `-5` relational/parser/storage failure.
- Bindings are C-layout `{ name: *const u8, name_len: usize,
  value: *const u8, value_len: usize, kind: u32 }`. Maximum 256 named
  parameters and 1 MiB per parameter.
- Kinds: 0 NULL (empty payload), 1 boolean (true/false), 2 i64 decimal text,
  3 UTF8 text, 4 exact fixed-scale decimal, 5 date, 6 datetime, 7 uuid,
  8 JSON, 9 binary hex text. Extended values are type-checked against the
  schema when executed, not interpolated.
- Output is a UTF8 JSON object `{"affected":N,"rows":[...]}`; decimal results
  are JSON **strings** to preserve precision.
- Response space must be supplied up front (maximum 16 MiB). Overflow fails
  **before commit**. Do not retry mutating calls blindly following unknown
  transport errors. Locks/commit use the Zig v2 WAL and current durability
  semantics; they do not implement durable MVCC.

## Security and correctness boundary

This is strictly a *trusted* embedded API. It does not accept a principal or
check durable per-tenant grants. Restrict filesystem permissions and never
route untrusted user queries directly to this function. Wrong foreign pointers
violate the unsafe C ABI contract.

Run `make test` and the native Noqeri/Zig integration matrix on an enabled
GitHub Actions runner. The existing runner allocation failure currently
prevents qualification. The initial tests exercise exact binding parsing and
non-commit on response overflow, not full native Noqeri NIR parity.
