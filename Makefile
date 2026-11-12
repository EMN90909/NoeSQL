NOQERI ?= /home/antigravity/EMN90909/Noqeri/build/noqeri

.PHONY: test no-js no-c-family no-rust-zig clean run

test: no-js no-c-family no-rust-zig
	@echo "=== Running NoeSQL Pure Noqeri Test Suite ==="
	@for f in tests/noesql_lexer.noqeri \
	          tests/noesql_native_api.noqeri \
	          tests/noesql_native_index.noqeri \
	          tests/noesql_native_ir.noqeri \
	          tests/noesql_native_security.noqeri \
	          tests/noesql_zig_relational.noqeri \
	          tests/test_wal_noqeri.noqeri \
	          tests/test_btree_noqeri.noqeri \
	          tests/test_wal_engine_lifecycle.noqeri \
	          tests/test_wal_rollback.noqeri \
	          tests/test_wal_streaming_multistmt.noqeri \
	          tests/test_wal_torn_tail.noqeri \
	          tests/test_adversarial_fk.noqeri \
	          tests/test_durability_fsync.noqeri \
	          tests/test_injection_resistance.noqeri \
	          tests/test_fuzz_storage_parser.noqeri \
	          tests/test_security_auth.noqeri \
	          tests/test_backup_restore.noqeri \
	          Programs/noesql.noqeri; do \
		echo "Running $$f ..."; \
		$(NOQERI) run "$$f" || exit 1; \
	done
	@python3 tests/test_concurrent_workload.py || exit 1
	@python3 tests/test_durability_crash_kill.py || exit 1
	@python3 tests/test_wal_cross_process_refresh.py || exit 1
	@python3 tests/test_wal_checkpoint_isolation.py || exit 1
	@echo "=== All Pure Noqeri Tests Passed ==="

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

no-rust-zig:
	@set -eu; found=$$(find . \
		-path './.git' -prune -o \
		-path './.ci-noqeri' -prune -o \
		-path './.ci-noqeri-build' -prune -o \
		-type f \( -name '*.rs' -o -name '*.zig' \) -print -quit); \
	if [ -n "$$found" ]; then echo "Rust or Zig file remains in NoeSQL: $$found" >&2; exit 1; fi

run:
	$(NOQERI) run Programs/noesql.noqeri

clean:
	rm -rf *.noedb *.backup *.restored *.lck tmp_*
