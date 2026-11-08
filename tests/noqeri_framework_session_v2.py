#!/usr/bin/env python3
"""Live Noqeri native web -> NoeSQL session-v2 -> NQDB0003 regression.

This is an opt-in LOCAL integration test; it does not use GitHub Actions,
mock NoeSQL callback tables, Node.js, or SQLite. Requires a separately
built Noqeri native-web executable and Rust NoeSQL shared library.
"""
import http.client
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
from pathlib import Path

if len(sys.argv) != 4:
    raise SystemExit(
        "usage: python3 tests/noqeri_framework_session_v2.py "
        "<noqeri-web-binary> <noesql-cli> <libnoesql_runtime.so>"
    )

web, cli, provider = (Path(arg).resolve() for arg in sys.argv[1:])
for binary in (web, cli, provider):
    if not binary.is_file():
        raise SystemExit(f"missing built runtime binary: {binary}")

with tempfile.TemporaryDirectory(prefix="noesql-noqeri-v2-") as td:
    root = Path(td)
    # NoeSQL v3 is never silently created by the native web query provider.
    # Operators must explicitly create/bootstrap their schema.
    private = root / "private"
    private.mkdir(mode=0o700)
    database = private / "app.noedb"
    schema = root / "schema.noeqr"
    schema.write_text(
        "table users {\n id:int key auto\n name:text required unique\n}"
        '\nusers add {name:"Amina"}\n', encoding="utf8"
    )
    create = subprocess.run(
        [str(cli), "v3-create", str(database), str(schema)],
        capture_output=True, text=True, timeout=30,
    )
    assert create.returncode == 0, (create.stdout, create.stderr)

    app = root / "app"
    app.mkdir()
    (app / "main.noqeri").write_text(
        f'''
database {{
 engine: "noesql"
 path: "{database}"
}}
query people = "users get name sort name"
route "/" {{
    main {{
        h1 {{ "Real NoeSQL v3" }}
        p {{ people }}
        form on.submit {{
            validate(name, 1, 64)
            db.execute("users add {{name:$name}}", name: form.name)
        }} {{
            input name: "name" type: "text" {{}}
            button type: "submit" {{ "Save" }}
        }}
    }}
}}
''', encoding="utf8"
    )
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]

    env = dict(os.environ)
    env["NOESQL_NATIVE_PROVIDER"] = str(provider)
    env["NOESQL_BIN"] = "/nonexistent/forbidden-fallback"
    env.pop("NOQERI_WEB_REQUIRE_ORIGIN", None)
    server = subprocess.Popen(
        [str(web), "serve-app", str(app), str(port), str(root / "web.nqkv")],
        env=env, cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )

    def request(method, payload=None, cookie=None):
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=6)
        headers = {}
        if payload is not None:
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        if cookie is not None:
            headers["Cookie"] = cookie
        conn.request(method, "/", payload, headers)
        response = conn.getresponse()
        status = response.status
        headers = dict(response.getheaders())
        content = response.read().decode("utf-8", errors="replace")
        conn.close()
        return status, headers, content

    def form_fields(html):
        token = re.search(r'name="_nq_csrf" value="([^"]+)"', html)
        action = re.search(r'name="_nq_action" value="([^"]+)"', html)
        assert token and action, html[:1000]
        return token.group(1), action.group(1)

    def durable_generation():
        check = subprocess.run(
            [str(cli), "v3-check", str(database)],
            capture_output=True, text=True, timeout=20
        )
        assert check.returncode == 0, (check.stdout, check.stderr)
        matched = re.search(r"generation=(\d+)", check.stdout)
        assert matched, check.stdout
        return int(matched.group(1))

    try:
        for _ in range(80):
            if server.poll() is not None:
                raise AssertionError(
                    "Noqeri web process exited: " + server.stderr.read().decode()
                )
            try:
                if request("GET")[0] == 200:
                    break
            except OSError:
                time.sleep(0.075)
        else:
            raise AssertionError("Noqeri web server did not become ready")

        status, headers, home = request("GET")
        assert status == 200 and "Amina" in home, (status, home)
        cookie = headers["Set-Cookie"].split(";", 1)[0]
        token, action = form_fields(home)
        assert durable_generation() == 1

        # Payloads are JSON string bindings, not compiled into NoeSQL source.
        user_text = 'Nairobi " & <script>alert(1)</script>'
        payload = urllib.parse.urlencode({
            "_nq_action": action, "_nq_csrf": token, "name": user_text
        })
        assert request("POST", payload, cookie)[0] == 303
        assert durable_generation() == 2
        status, _, home = request("GET", cookie=cookie)
        assert status == 200 and "Amina" in home and "Nairobi" in home, (status, home)
        assert "<script>alert" not in home, "HTML injection not escaped"

        # A unique-constraint violation must not advance the durable HEAD.
        token, action = form_fields(home)
        duplicate = urllib.parse.urlencode({
            "_nq_action": action, "_nq_csrf": token, "name": "Amina"
        })
        status, _, _ = request("POST", duplicate, cookie)
        assert status == 503, status
        assert durable_generation() == 2
        read = subprocess.run(
            [str(cli), "query", str(database), "users get id,name sort id"],
            capture_output=True, text=True, timeout=20
        )
        assert read.returncode == 0 and read.stdout.count('"id":') == 2, (
            read.stdout, read.stderr
        )
    finally:
        server.terminate()
        try:
            server.communicate(timeout=4)
        except subprocess.TimeoutExpired:
            server.kill()
            server.communicate()

print("PASS: real Noqeri native-web -> session-v2 provider -> NoeSQL v3")
