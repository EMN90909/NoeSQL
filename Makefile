AR ?= ar

.PHONY: test native native-chain stage2-chain test-zig test-rust test-rust-zig stress benchmark-native-sqlite benchmark-parser-reads test-sqlite test-noqeri-framework-local no-js no-c-family clean

test: no-js no-c-family test-zig test-rust test-rust-zig

no-js:
	@set -eu; found=$$(find . \
		-path './.git' -prune -o \
		-path './.ci-noqeri' -prune -o \
		-path './.ci-noqeri-build' -prune -o \
		-type f \( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \) -print -quit); \
	if [ -n "$$found" ]; then echo "JavaScript file remains: $$found" >&2; exit 1; fi

no-c-family:
	@set -eu; found=$$(find . \
		-path './.git' -prune -o \
		-path './.ci-noqeri' -prune -o \
		-path './.ci-noqeri-build' -prune -o \
		-type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.h' -o -name '*.hh' -o -name '*.hpp' \) -print -quit); \
	if [ -n "$$found" ]; then echo "C/C++ source remains in NoeSQL: $$found" >&2; exit 1; fi

test-zig:
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out test

test-rust:
	cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets

test-rust-zig:
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out -Doptimize=ReleaseSafe
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager

# Optional live SQLite compatibility suite (requires libsqlite3-dev).
# Runs the actual Rust -> Zig durable path plus fresh-file SQLite import.
test-sqlite: no-js no-c-family
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out -Doptimize=ReleaseSafe
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo test --locked --manifest-path Native/rust/Cargo.toml --all-targets --features zig-pager,sqlite-adapter

# Live Noqeri native web -> Rust session-v2 -> NQDB0003 integration.
# NO GitHub Actions or mock provider. NOQERI_WEB must point to a locally
# built Noqeri web executable; requires Linux x86-64 and Python 3.
test-noqeri-framework-local: no-js no-c-family
	@test -n "$(NOQERI_WEB)" && test -x "$(NOQERI_WEB)" || \
		(echo "Set NOQERI_WEB=/absolute/path/to/noqeri-web" >&2; exit 2)
	@test "$(uname -s)" = Linux || \
		(echo "Real native provider test currently requires Linux" >&2; exit 2)
	cargo test --locked --manifest-path Native/rust/Cargo.toml --lib session_bindings::tests
	cargo test --locked --manifest-path Native/rust/Cargo.toml --lib session_provider::tests
	cargo build --locked --release --manifest-path Native/rust/Cargo.toml --lib --bin noesql
	python3 tests/noqeri_framework_session_v2.py \
		"$(NOQERI_WEB)" \
		"$(CURDIR)/Native/rust/target/release/noesql" \
		"$(CURDIR)/Native/rust/target/release/libnoesql_runtime.so"

# Three independent Rust↔Zig executions; the Zig ABI test binary itself
# exercises 96 write/reopen/checkpoint cycles and corruption rejection cases.
stress: no-js no-c-family
	@set -eu; for iteration in 1 2 3; do \
		echo "NoeSQL durability stress pass $iteration / 3"; \
		$(MAKE) test-zig test-rust-zig; \
	done

# Profile only the in-memory immutable B+ access path against a full scan.
# Do not use this benchmark as an on-disk throughput or MVCC latency claim.
benchmark-bplus: no-js no-c-family
	cargo run --release --locked --manifest-path Native/rust/Cargo.toml --bin benchmark_v3_index

# Isolated Rust compiler/parser and read execution microbenchmark.
# This is NOT a substitute for a multi-client transactional load test.
benchmark-parser-reads: no-js no-c-family
	cargo run --release --locked --manifest-path Native/rust/Cargo.toml --bin benchmark_parser_reads

native:
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out -Doptimize=ReleaseSafe
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager

stage2-chain: no-js no-c-family
	@test -n "$(NOQERI_REPO)" || (echo "Set NOQERI_REPO=/absolute/path/to/Noqeri" >&2; exit 2)
	@test -x "$(NOQERI_NQRI_VM)" || (echo "Set NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out -Doptimize=ReleaseSafe
	mkdir -p Native/rust/native-stage2
	test "$(sha256sum "$(NOQERI_REPO)/Compiler/selfhost/trusted-stage1.nqri" | awk '{print $$1}')" = "$(awk '{print $$1}' "$(NOQERI_REPO)/Compiler/selfhost/trusted-stage1.sha256")"
	"$(NOQERI_NQRI_VM)" compile-manifest \
		"$(NOQERI_REPO)/Compiler/selfhost/trusted-stage1.nqri" \
		"$(NOQERI_REPO)" \
		"$(NOQERI_REPO)/Compiler/selfhost/stage2-sources.txt" \
		Native/rust/native-stage2/noqeri-stage2.nqri 3
	"$(NOQERI_NQRI_VM)" compile-manifest \
		Native/rust/native-stage2/noqeri-stage2.nqri \
		"$(NOQERI_REPO)" \
		"$(NOQERI_REPO)/Compiler/selfhost/stage2-sources.txt" \
		Native/rust/native-stage2/noqeri-stage3.nqri 3
	"$(NOQERI_NQRI_VM)" diff Native/rust/native-stage2/noqeri-stage2.nqri Native/rust/native-stage2/noqeri-stage3.nqri
	"$(NOQERI_NQRI_VM)" compile \
		Native/rust/native-stage2/noqeri-stage2.nqri \
		Database/noesql_stage2_bridge.noqeri \
		Native/rust/native-stage2/noesql_stage2_bridge.nqri 3
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
	NOESQL_NQRI_VM="$(NOQERI_NQRI_VM)" \
	NOESQL_STAGE2_MODULE="$(CURDIR)/Native/rust/native-stage2/noesql_stage2_bridge.nqri" \
		cargo run --manifest-path Native/rust/Cargo.toml --features stage2-nqri --bin stage2_chain_smoke

native-chain: no-js no-c-family
	@test -n "$(NOQERI_COMPILER)" || (echo "Set NOQERI_COMPILER=/absolute/path/to/noqeri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig --prefix Native/zig/zig-out -Doptimize=ReleaseSafe
	rm -rf Native/rust/native-lib
	cargo run --manifest-path Native/rust/Cargo.toml --bin build_native -- \
		"$(NOQERI_COMPILER)" \
		"$(CURDIR)/Database/noesql_native_bridge.noqeri" \
		"$(CURDIR)/Native/rust/native-lib"
	NOESQL_NOQERI_LIB_DIR="$(CURDIR)/Native/rust/native-lib" \
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo test --manifest-path Native/rust/Cargo.toml --all-targets --features noqeri-native
	NOESQL_NOQERI_LIB_DIR="$(CURDIR)/Native/rust/native-lib" \
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo run --manifest-path Native/rust/Cargo.toml --features noqeri-native --bin native_chain_smoke

# Run the actual native Noqeri AOT + Rust + Zig WAL path against a local
# SQLite build. Both engines update/read/checkpoint a durable single bool row.
benchmark-native-sqlite: no-js no-c-family
	@test -n "$(NOQERI_COMPILER)" || (echo "Set NOQERI_COMPILER=/absolute/path/to/noqeri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
	mkdir -p Native/rust/native-lib
	cargo run --locked --manifest-path Native/rust/Cargo.toml --bin build_native -- \
		"$(NOQERI_COMPILER)" "$(CURDIR)/Database/noesql_native_bridge.noqeri" \
		"$(CURDIR)/Native/rust/native-lib"
	NOESQL_NOQERI_LIB_DIR="$(CURDIR)/Native/rust/native-lib" \
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo run --release --locked --manifest-path Native/rust/Cargo.toml \
		--features noqeri-native --bin benchmark_compare_sqlite

clean:
	rm -rf zig-out .zig-cache Native/zig/zig-out Native/zig/.zig-cache Native/rust/target Native/rust/native-lib Native/rust/native-stage2
