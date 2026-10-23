CXX ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Wpedantic -Werror
CRYPTO_LIBS ?= -lcrypto
TEST_BIN ?= /tmp/noesql-cpp-abi-test
PAGE_TEST_BIN ?= /tmp/noesql-cpp-page-test
CLI_BIN ?= /tmp/noesql-native
BENCH_BIN ?= /tmp/noesql-cpp-page-bench
ABI_TEST_BIN ?= /tmp/noesql-storage-abi-test
FAULT_TEST_BIN ?= /tmp/noesql-prod-fault-gate-test
COMPARE_BIN ?= /tmp/noesql-sqlite-compare
NOQERI_ZIG_TEST_BIN ?= /tmp/noesql-noqeri-zig-bridge-test

.PHONY: test test-native benchmark benchmark-compare native-cli clean

test: test-native
	$(CXX) $(CXXFLAGS) noesql_noqeri_zig_bridge.cpp test_noqeri_zig_bridge.cpp -I. -I../zig -o $(NOQERI_ZIG_TEST_BIN)
	$(NOQERI_ZIG_TEST_BIN)
	rm -f $(NOQERI_ZIG_TEST_BIN)
	$(CXX) $(CXXFLAGS) noesql_session_abi.cpp test_session_abi.cpp -o /tmp/noesql-session-abi-test
	/tmp/noesql-session-abi-test
	rm -f /tmp/noesql-session-abi-test
	$(CXX) $(CXXFLAGS) noesql_engine_abi.cpp test_abi.cpp -o $(TEST_BIN)
	$(TEST_BIN)
	rm -f $(TEST_BIN)

test-native:
	$(CXX) $(CXXFLAGS) -DNOESQL_TEST_FAULT_INJECTION noesql_page_store.cpp test_page_store.cpp -o $(PAGE_TEST_BIN) $(CRYPTO_LIBS)
	$(PAGE_TEST_BIN)
	rm -f $(PAGE_TEST_BIN)
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp noesql_storage_abi.cpp test_storage_abi.cpp -o $(ABI_TEST_BIN) $(CRYPTO_LIBS)
	$(ABI_TEST_BIN)
	rm -f $(ABI_TEST_BIN)
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp test_production_fault_gate.cpp -o $(FAULT_TEST_BIN) $(CRYPTO_LIBS)
	$(FAULT_TEST_BIN)
	rm -f $(FAULT_TEST_BIN)

benchmark-compare:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp benchmark_compare_sqlite.cpp -o $(COMPARE_BIN) $(CRYPTO_LIBS) -lsqlite3
	$(COMPARE_BIN)
	rm -f $(COMPARE_BIN)

native-cli:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp noesql_native_cli.cpp -o $(CLI_BIN) $(CRYPTO_LIBS)
	@echo "Built $(CLI_BIN)"

benchmark:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp benchmark_page_store.cpp -o $(BENCH_BIN) $(CRYPTO_LIBS)
	$(BENCH_BIN)
	rm -f $(BENCH_BIN)

clean:
	rm -f $(TEST_BIN) $(PAGE_TEST_BIN) $(BENCH_BIN) $(FAULT_TEST_BIN) $(COMPARE_BIN) $(ABI_TEST_BIN) $(NOQERI_ZIG_TEST_BIN)
