# NoeSQL Zig storage migration

## Current ownership

The migration is complete at the source-language boundary:

- **Noqeri** owns database semantics and native decision modules.
- **Rust** owns host/runtime orchestration and safe ABI wrappers.
- **Zig** owns page storage, WAL, indexes, snapshots, locking, recovery, and checkpoints.
- **C/C++** are not part of the NoeSQL source tree.

The former C++ page-store prototype and compatibility shim have been removed rather than preserved as a second storage engine.

## Durable formats

Zig owns the canonical storage formats:

- `NQDB0002` pages
- `NQTX0002` WAL
- `NQRS0001` snapshot manifest

Rust's bounded `NQRP0001` snapshot payload is stored through the Zig pager rather than written directly to disk.

## Native Noqeri path

The direct native qualification path is:

```text
Rust -> Noqeri scalar semantics -> Rust pager orchestration -> Zig durability
```

The scalar Noqeri boundary is intentional. Arbitrary aggregate cross-language ABI claims remain blocked until Noqeri's backend qualifies them.

## Removal gates now enforced

`make no-c-family` rejects C/C++ source and headers from NoeSQL.

CI also runs the same gate after the native chain so generated or accidentally reintroduced compatibility sources cannot silently become part of the repository.

## Remaining storage qualification

Removing the old prototype does not make the engine production-qualified by itself. Remaining work includes:

1. concurrent-writer stress and lock fairness;
2. crash/power-loss testing across WAL boundaries;
3. fuzzing malformed pages, WAL records, and snapshot manifests;
4. explicit legacy-format migration tooling and fixtures;
5. broader platform qualification;
6. differential query and durability testing against independent oracles.

Legacy formats must never be silently interpreted as current `.noedb` storage.
