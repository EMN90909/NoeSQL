CXX ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Wpedantic -Werror
CRYPTO_LIBS ?= -lcrypto
TEST_BIN ?= /tmp/noesql-cpp-abi-test
PAGE_TEST_BIN ?= /tmp/noesql-cpp-page-test
CLI_BIN ?= /tmp/noesql-native
BENCH_BIN ?= /tmp/noesql-cpp-page-bench
ABI_TEST_BIN ?= /tmp/noesql-storage-abi-test

.PHONY: test test-native benchmark native-cli clean

test: test-native
	$(CXX) $(CXXFLAGS) noesql_engine_abi.cpp test_abi.cpp -o $(TEST_BIN)
	$(TEST_BIN)
	rm -f $(TEST_BIN)

test-native:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp test_page_store.cpp -o $(PAGE_TEST_BIN) $(CRYPTO_LIBS)
	$(PAGE_TEST_BIN)
	rm -f $(PAGE_TEST_BIN)
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp noesql_storage_abi.cpp test_storage_abi.cpp -o $(ABI_TEST_BIN) $(CRYPTO_LIBS)
	$(ABI_TEST_BIN)
	rm -f $(ABI_TEST_BIN)

native-cli:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp noesql_native_cli.cpp -o $(CLI_BIN) $(CRYPTO_LIBS)
	@echo "Built $(CLI_BIN)"

benchmark:
	$(CXX) $(CXXFLAGS) noesql_page_store.cpp benchmark_page_store.cpp -o $(BENCH_BIN) $(CRYPTO_LIBS)
	$(BENCH_BIN)
	rm -f $(BENCH_BIN)

clean:
	rm -f $(TEST_BIN) $(PAGE_TEST_BIN) $(BENCH_BIN)
