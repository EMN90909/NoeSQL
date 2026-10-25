.PHONY: test native native-chain stage2-chain test-zig test-rust no-js no-native-c-family clean

test: no-js no-native-c-family test-zig test-rust

no-js:
	@set -eu; found=$$(find . \
		-path './.git' -prune -o \
		-path './.ci-noqeri' -prune -o \
		-path './.ci-noqeri-build' -prune -o \
		-type f \( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \) -print -quit); \
	if [ -n "$$found" ]; then echo "JavaScript file remains: $$found" >&2; exit 1; fi

no-native-c-family:
	@set -eu; found=$$(find . \
		-path './.git' -prune -o \
		-path './.ci-noqeri' -prune -o \
		-path './.ci-noqeri-build' -prune -o \
		-type f \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.hpp' -o -name '*.hxx' \) -print -quit); \
	if [ -n "$$found" ]; then echo "C/C++ implementation source remains in NoeSQL: $$found" >&2; exit 1; fi; \
	if [ -d Native/cpp ]; then echo "Native/cpp must not exist" >&2; exit 1; fi

test-zig:
	zig build --build-file Native/zig/build.zig test

test-rust:
	cargo test --manifest-path Native/rust/Cargo.toml --all-targets

native:
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" \
		cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager

stage2-chain: no-js no-native-c-family
	@test -n "$(NOQERI_REPO)" || (echo "Set NOQERI_REPO=/absolute/path/to/Noqeri" >&2; exit 2)
	@test -x "$(NOQERI_NQRI_VM)" || (echo "Set NOQERI_NQRI_VM=/absolute/path/to/noqeri-nqri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
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

native-chain: no-js no-native-c-family
	@test -n "$(NOQERI_COMPILER)" || (echo "Set NOQERI_COMPILER=/absolute/path/to/noqeri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
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

clean:
	rm -rf zig-out .zig-cache Native/zig/zig-out Native/zig/.zig-cache Native/rust/target Native/rust/native-lib Native/rust/native-stage2
