#!/usr/bin/env python3
"""Offline NoeSQL query-script and Noqeri native HTTP ABI compatibility gate.

No GitHub Actions, HTTP access, C/C++/JS runtime or database mocks.
Use the exact local Noqeri source checkout from which the native client
compiler is built. This is a source-compatibility gate, NOT a compiled E2E test.
"""
from pathlib import Path
import re
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: python3 scripts/check_noeqr_noqeri_http.py /path/to/Noqeri")
root = Path(__file__).resolve().parents[1]
noqeri = Path(sys.argv[1]).resolve(strict=True)
http = noqeri / "Lib/std/http/client.noqeri"
http_source = http.read_text(encoding="utf-8")

expected = {
    "http_client_json_post": "url: string, body: string, timeoutMillis: i64",
    "http_client_status": "handle: usize",
    "http_client_body": "handle: usize",
    "http_client_close": "handle: usize",
}
for function, arguments in expected.items():
    pattern = re.compile(
        r"\bexport\s+function\s+" + re.escape(function) +
        r"\s*\(\s*" + re.escape(arguments) + r"\s*\)\s*:"
    )
    if not pattern.search(http_source):
        raise SystemExit(f"Noqeri HTTP client ABI mismatch: {function}({arguments})")
if 'extern function httpClientRequestEx(' not in http_source:
    raise SystemExit("Noqeri extended native HTTP host binding is absent")

illegal = sorted(
    file.relative_to(root).as_posix()
    for file in root.rglob("*.noe")
    if ".git" not in file.parts and "target" not in file.parts
)
if illegal:
    raise SystemExit(
        "Legacy .noe database source remains (rename to .noeqr):\n" +
        "\n".join(illegal[:50])
    )
scripts = sorted(root.rglob("*.noeqr"))
if not scripts:
    raise SystemExit("no .noeqr database source fixture exists")

example = (root / "examples/noeqr_http_client.noqeri").read_text(encoding="utf-8")
if 'import "std/http/client"' not in example:
    raise SystemExit("NoeSQL Noqeri client example imports an incompatible module")
for function in expected:
    if function not in example:
        raise SystemExit(f"NoeSQL native client example no longer calls {function}")

print(
    "PASS: .noeqr source scripts, .noqeri application example and "
    "current Noqeri native HTTP client signatures match"
)
print("NOTE: static gate only; compile and run the native client/server E2E separately")
