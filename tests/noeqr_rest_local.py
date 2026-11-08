#!/usr/bin/env python3
"""Local binary test of NoeSQL .noeqr file handling and Noqeri-compatible REST.

Requires an already compiled NoeSQL Rust CLI. No GitHub Actions, Node, mock
database, external network, or third-party Python dependencies.
"""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

if len(sys.argv) != 2:
    raise SystemExit("usage: python3 tests/noeqr_rest_local.py /absolute/path/to/noesql")
binary = Path(sys.argv[1]).resolve(strict=True)
TOKEN = "noesql-test-secret-0123456789-0123456789"

def cli(*args, success=True):
    result = subprocess.run(
        [str(binary), *map(str, args)], capture_output=True, text=True, timeout=40
    )
    if success and result.returncode:
        raise AssertionError((args, result.stdout, result.stderr))
    if not success and result.returncode == 0:
        raise AssertionError(("unexpected CLI success", args))
    return result.stdout + result.stderr

with tempfile.TemporaryDirectory(prefix="noesql-noeqr-http-") as home:
    private = Path(home) / "private"
    private.mkdir(mode=0o700)
    os.chmod(private, 0o700)
    scripts = private / "scripts"
    scripts.mkdir(mode=0o700)
    os.chmod(scripts, 0o700)
    db = private / "app.noedb"
    schema = scripts / "schema.noeqr"
    schema.write_text(
        "table people {\n id:int key auto\n name:text required unique }\n"
        "people add {name:\"Ada\"}\n", encoding="utf-8"
    )
    os.chmod(schema, 0o600)
    query = scripts / "people.noeqr"
    query.write_text("people get id,name where name == $name", encoding="utf-8")
    os.chmod(query, 0o600)
    migrations = scripts / "migration.noeqr"
    migrations.write_text(
        "table groups { id:int key auto\n name:text required }\n", encoding="utf-8"
    )
    os.chmod(migrations, 0o600)
    legacy = scripts / "legacy.noe"
    legacy.write_text("people get id", encoding="utf-8")

    assert "ok:" in cli("check", schema)
    assert ".noeqr" in cli("check", legacy, success=False)
    assert "NQDB0003 created" in cli("v3-create", db, schema)
    # query-file intentionally requires values for named placeholders; use a
    # parameterless standalone file for CLI verification.
    (scripts / "all.noeqr").write_text(
        "people get id,name sort id", encoding="utf-8"
    )
    assert "Ada" in cli("query-file", db, scripts / "all.noeqr")
    assert "affected=" in cli("schema-file", db, migrations)

    # A real separate process is required; no in-process mock HTTP adapter.
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = os.environ.copy()
    env["NOESQL_API_TOKEN"] = TOKEN
    server = subprocess.Popen(
        [str(binary), "serve", str(db), str(port), "--scripts-root", str(scripts)],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True
    )
    def request(method, route, body=None, token=None, content_type="application/json"):
        headers = {"Host": "127.0.0.1"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if body is not None:
            headers["Content-Type"] = content_type
        payload = json.dumps(body).encode() if body is not None else None
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=4)
        try:
            conn.request(method, route, body=payload, headers=headers)
            response = conn.getresponse()
            data = response.read()
            assert len(data) <= 1024 * 1024 + 4096
            assert response.getheader("Cache-Control") == "no-store"
            return response.status, json.loads(data)
        finally:
            conn.close()
    try:
        for _ in range(80):
            try:
                status, _ = request("GET", "/health")
                if status == 200:
                    break
            except (OSError, ConnectionError):
                if server.poll() is not None:
                    raise AssertionError("REST server exited before accepting requests")
                time.sleep(0.05)
        else:
            raise AssertionError("REST server did not accept loopback requests")
        assert request("GET", "/v1/status")[0] == 401
        assert request("GET", "/v1/status", token="incorrect-token")[0] == 401
        status, details = request("GET", "/v1/status", token=TOKEN)
        assert status == 200 and details["format"] == "NQDB0003"

        # Noqeri native std.http.client supports JSON POST but not custom
        # headers: the JSON-body secret makes this first-party client usable.
        body = {"token": TOKEN, "script": "people.noeqr", "params": {"name": "Ada"}}
        status, result = request("POST", "/v1/query", body)
        assert status == 200 and result["count"] == 1, result
        assert result["rows"][0]["name"] == "Ada", result

        malicious = "Zed\"; people delete where id >= 1 --"
        body = {"token": TOKEN, "source": "people add {name:$name}",
                "params": {"name": malicious}}
        status, result = request("POST", "/v1/execute", body)
        assert status == 200 and result["affected"] == 1, result
        status, result = request("POST", "/v1/query",
            {"source": "people get id,name sort id", "params": {}}, token=TOKEN)
        assert status == 200 and result["count"] == 2, result
        assert any(row["name"] == malicious for row in result["rows"])
        assert any(row["name"] == "Ada" for row in result["rows"])

        # The query route is read-only, and HTTP cannot run DDL.
        assert request("POST", "/v1/query",
            {"token": TOKEN, "source": "people add {name:\"Eve\"}"})[0] == 403
        assert request("POST", "/v1/execute",
            {"token": TOKEN, "script": "migration.noeqr"})[0] == 403
        assert request("POST", "/v1/query",
            {"source": "people get id", "token": "not-authorized"})[0] == 401
        assert request("POST", "/v1/query",
            {"source": "people get id", "script": "../wrong.noeqr"}, token=TOKEN)[0] == 400
        assert request("POST", "/v1/query",
            {"source": "people get id"}, token=TOKEN, content_type="text/plain")[0] == 415
        # Raw HTTP regression: reject ambiguous message framing, browser
        # Origin requests and chunked transfer before executing any source.
        def raw_status(wire):
            with socket.create_connection(("127.0.0.1", port), timeout=3) as conn:
                conn.sendall(wire.encode("ascii"))
                conn.shutdown(socket.SHUT_WR)
                line = conn.recv(4096).split(b"\r\n", 1)[0].decode("ascii")
                return int(line.split(" ")[1])
        # DNS rebinding and arbitrary virtual Host values must not gain
        # access to the private local REST service, even with a valid token.
        assert raw_status(
            "GET /v1/status HTTP/1.1\r\nHost: database.attacker.example\r\n"
            "Authorization: Bearer " + TOKEN + "\r\n\r\n"
        ) == 400
        assert raw_status(
            "GET /v1/status HTTP/1.1\r\nHost: localhost:99999\r\n"
            "Authorization: Bearer " + TOKEN + "\r\n\r\n"
        ) == 400
        assert raw_status(
            "GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            "Authorization: Bearer " + TOKEN + "\r\n\r\n"
        ) == 200
        assert raw_status(
            "POST /v1/query HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            "Content-Length: 0\r\nContent-Length: 0\r\n\r\n"
        ) == 400
        assert raw_status(
            "POST /v1/query HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            "Transfer-Encoding: chunked\r\n\r\n"
        ) == 400
        assert raw_status(
            "GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            "Origin: https://untrusted.example\r\n"
            "Authorization: Bearer " + TOKEN + "\r\n\r\n"
        ) == 403
        assert "generation=" in cli("v3-check", db)
    finally:
        server.terminate()
        try:
            server.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.communicate(timeout=5)

print("PASS: noeqr CLI, local authenticated HTTP queries and writes, typed bindings, script rules")
