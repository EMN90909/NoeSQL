.PHONY: test native native-chain test-zig test-rust test-cpp no-js clean

test: no-js test-zig test-rust test-cpp

no-js:
	@set -eu; found=$$(find . 		-path './.git' -prune -o 		-path './.ci-noqeri' -prune -o 		-path './.ci-noqeri-build' -prune -o 		-type f \( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \) -print -quit); 	if [ -n "$$found" ]; then echo "JavaScript file remains: $$found" >&2; exit 1; fi

test-zig:
	zig build --build-file Native/zig/build.zig test

test-rust:
	cargo test --manifest-path Native/rust/Cargo.toml --all-targets

test-cpp:
	$(MAKE) -C Native/cpp test

native:
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" 		cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager
	$(MAKE) -C Native/cpp native-cli

native-chain: no-js
	@test -n "$(NOQERI_COMPILER)" || (echo "Set NOQERI_COMPILER=/absolute/path/to/noqeri" >&2; exit 2)
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
	rm -rf Native/rust/native-lib
	cargo run --manifest-path Native/rust/Cargo.toml --bin build_native -- 		"$(NOQERI_COMPILER)" 		"$(CURDIR)/Database/noesql_native_bridge.noqeri" 		"$(CURDIR)/Native/rust/native-lib"
	NOESQL_NOQERI_LIB_DIR="$(CURDIR)/Native/rust/native-lib" 	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" 		cargo test --manifest-path Native/rust/Cargo.toml --all-targets --features noqeri-native
	NOESQL_NOQERI_LIB_DIR="$(CURDIR)/Native/rust/native-lib" 	NOESQL_ZIG_LIB_DIR="$(CURDIR)/Native/zig/zig-out/lib" 		cargo run --manifest-path Native/rust/Cargo.toml --features noqeri-native --bin native_chain_smoke

clean:
	rm -rf zig-out .zig-cache Native/zig/zig-out Native/zig/.zig-cache Native/rust/target Native/rust/native-lib
	$(MAKE) -C Native/cpp clean
