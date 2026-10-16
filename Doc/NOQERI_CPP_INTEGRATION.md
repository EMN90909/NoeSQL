# NoeSQL engine integration: Noqeri, C++, and JavaScript reference

## Ownership

NoeSQL uses three deliberately separate layers:

1. **Noqeri (`Database/*.nqr`) owns database semantics.** The lexer, IR verification, executor, row/value rules, and future page/WAL implementation belong in the Noqeri source tree. The source of truth for language/compiler conventions is [EMN90909/Noqeri](https://github.com/EMN90909/Noqeri).
2. **C++17 (`Native/cpp/`) is a narrow native ABI bridge.** It validates ABI versions and buffer limits, then forwards requests to an engine function exported by a compiled Noqeri artifact. It must not implement a second parser, query planner, transaction manager, or storage engine.
3. **JavaScript (`Runtime/noesql-reference.mjs`) remains the executable reference oracle.** Use it for fast local semantic tests and differential conformance while the native compiler output is being integrated. It is not the production engine and its experimental `.nqd` snapshot format is not the future native file format.

The historical C++ compiler under the Noqeri repository's `bootstrap/cpp17-stage0` branch is compiler bootstrap provenance, not a replacement for Noqeri database semantics. Follow the Noqeri repository's current self-hosting rules and do not claim self-hosting or native engine parity until actual compiled artifacts pass the proof/conformance gates.

## Current bridge contract

`Native/cpp/noesql_engine_abi.h` defines ABI version 1:

- a compiled Noqeri engine exports a `noesql_engine_v1` function table;
- the bridge rejects unsupported ABI versions and invalid pointers;
- source is bounded to 1 MiB and output buffers to 16 MiB;
- the bridge resets output length on failure and rejects reported output lengths larger than the caller's buffer;
- the callback owns all parsing/execution semantics and must never write beyond `output_capacity`.

The callback currently has to be supplied by a compiled native engine. This repository increment does **not** claim that the Noqeri `.nqr` modules already export this exact ABI or that the native engine is callable from the existing CLI. That compiler/export glue is the next integration gate.

## Local C++ bridge checks

Requires a C++17 compiler:

```sh
make -C Native/cpp test
```

The test uses a fake echo callback to verify ABI forwarding, version rejection, argument validation, input limits, and oversized output-length rejection. It tests the bridge contract only; it is not a Noqeri compiler or database-engine test.

## Native integration sequence

1. Build a trusted Noqeri compiler using the steps in [Noqeri's README](https://github.com/EMN90909/Noqeri#build-locally).
2. Compile the NoeSQL `.nqr` modules with that compiler; resolve diagnostics instead of assuming source files compile.
3. Add a small Noqeri-exported entry point whose C ABI exactly matches `noesql_engine_execute_fn`, or generate a versioned shim from the compiler's supported export mechanism.
4. Link the resulting artifact with `Native/cpp/noesql_engine_abi.cpp`; never duplicate database semantics in C++.
5. Add shared `.noe` fixtures and run each fixture against the JavaScript reference and the native artifact, comparing result values and stable error codes.
6. Add native persistence, transaction rollback, concurrent access, and fault-injection crash tests. The reference store's process-kill tests are not proof of native durability.
7. Only after these gates pass should the CLI select the native engine by default. Until then, keep the reference engine explicitly labelled experimental.

## No GitHub Actions requirement

Run the C++ bridge check and Node.js reference tests locally. Do not treat source presence, a successful C++ bridge build, or README documentation as evidence that the Noqeri native database engine is compiled, linked, durable, or production-ready.
