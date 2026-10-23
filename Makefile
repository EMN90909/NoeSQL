.PHONY: test native test-zig test-rust test-cpp no-js clean

test: no-js test-zig test-rust test-cpp

no-js:
	@set -eu; found=$$(find . -type f \( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \) -not -path './.git/*' -print -quit); \
	if [ -n "$$found" ]; then echo "JavaScript file remains: $$found" >&2; exit 1; fi

test-zig:
	zig build --build-file Native/zig/build.zig test

test-rust:
	cargo test --manifest-path Native/rust/Cargo.toml --all-targets

test-cpp:
	$(MAKE) -C Native/cpp test

native:
	zig build --build-file Native/zig/build.zig -Doptimize=ReleaseSafe
	cargo build --release --manifest-path Native/rust/Cargo.toml
	$(MAKE) -C Native/cpp native-cli

clean:
	rm -rf zig-out .zig-cache Native/zig/zig-out Native/zig/.zig-cache Native/rust/target
	$(MAKE) -C Native/cpp clean
