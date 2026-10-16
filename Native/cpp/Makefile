CXX ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Wpedantic -Werror
TEST_BIN ?= /tmp/noesql-cpp-abi-test

.PHONY: test clean

test:
	$(CXX) $(CXXFLAGS) noesql_engine_abi.cpp test_abi.cpp -o $(TEST_BIN)
	$(TEST_BIN)
	rm -f $(TEST_BIN)

clean:
	rm -f $(TEST_BIN)
