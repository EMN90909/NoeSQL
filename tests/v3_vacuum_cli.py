#!/usr/bin/env python3
"""Real binary smoke test for v3 verify/backup/reclaim/restore/continued writes.

No GitHub Actions, mock engine, network, JavaScript or third-party packages.
Requires a separately compiled NoeSQL CLI with native v3 support.
"""
from pathlib import Path
import os
import subprocess
import sys
import tempfile

if len(sys.argv) != 2:
    raise SystemExit("usage: python3 tests/v3_vacuum_cli.py /absolute/path/to/noesql")
binary = Path(sys.argv[1]).resolve(strict=True)
if not binary.is_file() or not os.access(binary, os.X_OK):
    raise SystemExit(f"executable NoeSQL CLI required: {binary}")

with tempfile.TemporaryDirectory(prefix="noesql-reclaim-test-") as home:
    private = Path(home) / "database-private"
    private.mkdir(mode=0o700)
    os.chmod(private, 0o700)
    current = private / "live.noedb"
    backup = private / "backup.noedb"
    schema = Path(home) / "schema.noe"
    schema.write_text(
        "table entries {\n id:int key auto\n owner:int required }\n"
        "index owner_idx on entries(owner)\n"
        "entries add {owner:1}\n", encoding="utf-8"
    )

    def run(*args, good=True):
        result = subprocess.run(
            [str(binary), *(str(arg) for arg in args)],
            capture_output=True, text=True, timeout=60
        )
        if good and result.returncode:
            raise AssertionError((args, result.stdout, result.stderr))
        if not good and result.returncode == 0:
            raise AssertionError(("unexpected success", args, result.stdout))
        return result.stdout

    run("v3-create", current, schema)
    assert "generation=1" in run("v3-check", current)
    for count in range(2, 13):
        assert "affected=1" in run("v3-exec", current, f"entries add {{owner:{count}}}")
    assert "generation=12" in run("v3-check", current)
    before = run("v3-query", current, "entries get id,owner sort id")
    assert before.count('"owner":') == 12, before

    # A logical independent backup must survive deletion of historical
    # generation/index files in the SOURCE. The original db remains live.
    run("backup", current, backup)
    assert "generation=1" in run("v3-check", backup)
    assert run("v3-query", backup, "entries get id,owner sort id") == before

    plan = run("v3-vacuum", current)
    assert "DRY-RUN" in plan
    assert "obsolete_generations=11" in plan, plan
    assert "generation=12" in run("v3-check", current)

    # Any unexpected file must make the destructive command refuse
    # BEFORE deleting even the known old generations.
    extra = Path(str(current) + ".versions") / "external-backup.txt"
    extra.write_text("do not delete me", encoding="utf-8")
    run("v3-vacuum", current, "--apply", "--discard-history", good=False)
    assert extra.read_text(encoding="utf-8") == "do not delete me"
    assert "obsolete_generations=11" in plan
    extra.unlink()

    applied = run("v3-vacuum", current, "--apply", "--discard-history")
    assert "APPLIED" in applied, applied
    assert "obsolete_generations=11" in applied
    assert "generation=12" in run("v3-check", current)
    assert run("v3-query", current, "entries get id,owner sort id") == before
    assert run("v3-query", backup, "entries get id,owner sort id") == before
    second = run("v3-vacuum", current)
    assert "obsolete_generations=0" in second, second
    assert "obsolete_index_blobs=0" in second, second

    # A new generation can be published against the RECLAIMED index roots.
    run("v3-exec", current, "entries add {owner:13}")
    assert "generation=13" in run("v3-check", current)
    assert run("v3-query", current, "entries get id,owner sort id").count('"owner":') == 13
    assert run("v3-query", backup, "entries get id,owner sort id").count('"owner":') == 12

print("PASS: v3 verified logical backup, dry-run, fail-closed vacuum, reclaim, reopen, new write")
