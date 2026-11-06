#!/usr/bin/env python3
"""Fail-closed local ABI drift check against an explicitly checked-out Noqeri.

Checks the structural C++/Rust native session-v2 field order and provider
loading contract. Static checks do NOT replace a compiler or live E2E test.
"""
import re
import sys
from pathlib import Path

if len(sys.argv) != 2:
    raise SystemExit("usage: python3 scripts/check_noqeri_session_abi.py /path/to/Noqeri")

root = Path(__file__).resolve().parents[1]
noqeri = Path(sys.argv[1]).resolve()
header = noqeri / "Compiler/bootstrap/web_native_noesql_v2.hpp"
native_host = noqeri / "Compiler/bootstrap/web_native_db.hpp"
rust = root / "Native/rust/src/lib.rs"
provider = root / "Native/rust/src/session_provider.rs"

for file in (header, native_host, rust, provider):
    if not file.is_file():
        raise SystemExit(f"missing required source: {file}")

cpp = header.read_text(encoding="utf-8")
web = native_host.read_text(encoding="utf-8")
rs = rust.read_text(encoding="utf-8")
impl = provider.read_text(encoding="utf-8")

expected = [
    "abi_version", "context", "open", "close", "prepare", "release",
    "execute", "query", "begin", "commit", "rollback", "last_error",
]

cpp_struct = re.search(r"struct\s+NqNoeSqlSessionV2\s*\{(.*?)\};", cpp, re.S)
rust_struct = re.search(r"pub\s+struct\s+NoeSqlSessionV2\s*\{(.*?)\n\}", rs, re.S)
if cpp_struct is None or rust_struct is None:
    raise SystemExit("Noqeri C++ or NoeSQL Rust v2 session struct disappeared")

cpp_fields = []
for line in cpp_struct.group(1).splitlines():
    line = line.strip()
    if not line or line.startswith("//"):
        continue
    callback = re.search(r"\(\*(\w+)\)", line)
    if callback:
        cpp_fields.append(callback.group(1))
    else:
        match = re.search(r"\b(\w+)\s*;", line)
        if match:
            cpp_fields.append(match.group(1))

rust_fields = re.findall(r"^\s*pub\s+(\w+)\s*:", rust_struct.group(1), re.M)

for origin, fields in (("Noqeri C++", cpp_fields), ("NoeSQL Rust", rust_fields)):
    if fields != expected:
        raise SystemExit(
            f"ABI drift: {origin} has {fields!r}, expected {expected!r}"
        )

required = [
    "noesql_session_v2_get",
    "api->abi_version!=2",
    "api->prepare",
    "api->execute",
    "api->query",
    "api->begin",
    "api->commit",
    "api->rollback",
]
missing = [name for name in required if name not in web]
if missing:
    raise SystemExit(f"Noqeri web bridge contract changed: {missing}")
if "pub const SESSION_ABI_VERSION: u32 = 2;" not in rs:
    raise SystemExit("NoeSQL no longer advertises exact ABI version 2")
if "pub extern \"C\" fn noesql_session_v2_get()" not in impl:
    raise SystemExit("NoeSQL native getter export missing")
if "crate-type" not in (root / "Native/rust/Cargo.toml").read_text(encoding="utf-8"):
    raise SystemExit("Rust library build target missing")
if "NOESQL_NATIVE_PROVIDER" not in web:
    raise SystemExit("Noqeri web no longer advertises native provider loading")

print("PASS: native Noqeri C++ session-v2 layout matches Rust provider")
print("NOTE: static ABI contract only; compile both projects and run live E2E")
