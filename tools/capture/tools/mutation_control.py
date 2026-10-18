#!/usr/bin/env python3
"""
Mutation control for the differ.

A normaliser that scrubs too much reports "identical" for two bundles that
differ, and every conclusion drawn from it is then worthless.  So: take a known
fingerprint, inject the exact kinds of change the refactor actually made, and
require the differ to catch each one.

The mutations are the real ones, not invented:
  * stored_as filled in on an api row        (what 7771323 does)
  * a request header value changed          (what cd0e876 does)
  * a blob's bytes changed                  (what 54ff12d / 12ef18a could do)
  * a selfcheck assertion's wording moved   (what 43f66fa pins)
  * a document added and one removed        (a layout change)
  * attempts / bytes_in on a request row    (what 527833c / 844f5c2 do)
  * paths.tsv rows reordered                (a sort comparator drifting)
"""
import json
import shutil
import subprocess
import os
import sys
import tempfile
from pathlib import Path

H = Path(
    os.environ.get("GIRCAP_PROBE_HOME", str(Path(tempfile.gettempdir()) / "gircap-probe"))
)
SRC = H / "fp" / "default" / "884eeea"
WORK = H / "fp" / ".mutants"


def run(a, b):
    r = subprocess.run(
        [
            sys.executable,
            str(Path(__file__).with_name("bundle_differ.py")),
            "diff",
            str(a),
            str(b),
            "--limit",
            "0",
        ],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise SystemExit(f"differ failed: {r.stderr}")
    return json.loads(r.stdout)


def fresh():
    if WORK.exists():
        shutil.rmtree(WORK)
    shutil.copytree(SRC, WORK)
    return WORK


def check(name, mutate):
    d = fresh()
    mutate(d)
    res = run(SRC, d)
    ok = not res["identical"]
    print(f"  {'CAUGHT ' if ok else 'MISSED!'} {name}")
    if not ok:
        print("        the differ called these two bundles identical")
    return ok


# --------------------------------------------------------------- the mutations


def rows_of(d):
    """The request log, parsed.  Scrubbed tokens are put back into a shape JSON
    accepts, so the mutation is about the field under test and nothing else."""
    p = d / "api" / "requests.ndjson"
    txt = p.read_text(encoding="utf-8")
    txt = txt.replace('"elapsed_ms":<MS>', '"elapsed_ms":0')
    rows = [json.loads(line) for line in txt.splitlines()]
    return p, rows


def write_rows(p, rows):
    p.write_text(
        "\n".join(json.dumps(r, separators=(",", ":")) for r in rows) + "\n", encoding="utf-8"
    )


def m_stored_as(d):
    p, rows = rows_of(d)
    n = 0
    for r in rows:
        if r["kind"] == "api" and r["stored_as"] is None:
            r["stored_as"] = "api/getGameConfigs.json"
            n += 1
    assert n, "no api row with a null stored_as to mutate"
    write_rows(p, rows)


def m_header(d):
    p = d / "api" / "requests.ndjson"
    t = p.read_text(encoding="utf-8")
    assert '"accept-encoding":"gzip"' in t, "the baseline records this header"
    p.write_text(t.replace('"accept-encoding":"gzip"', '"accept-encoding":"identity"', 1), "utf-8")


def m_blob(d):
    blobs = sorted((d / "chunks" / "blobs").iterdir())
    b = blobs[0]
    raw = bytearray(b.read_bytes())
    raw[-1] ^= 0xFF
    b.write_bytes(bytes(raw))


def m_wording(d):
    p = d / "verify" / "selfcheck.json"
    t = p.read_text(encoding="utf-8")
    old = "manifest.checksum == id 32-hex suffix"
    assert old in t, "the assertion wording this narrows is not present"
    p.write_text(t.replace(old, "manifest.checksum matches the id", 1), encoding="utf-8")


def m_add_doc(d):
    (d / "verify" / "findings.json").write_text("{}\n", encoding="utf-8")


def m_drop_doc(d):
    (d / "diff" / "reuse_pool.json").unlink()


def m_attempts(d):
    p, rows = rows_of(d)
    n = 0
    for r in rows:
        if r["kind"] == "probe":
            r["attempts"] = 2
            n += 1
    assert n, "no probe row to mutate"
    write_rows(p, rows)


def m_bytes_in(d):
    p, rows = rows_of(d)
    n = 0
    for r in rows:
        if r["status"] != 200:
            r["bytes_in"] = 377
            n += 1
    assert n, "no non-200 row to mutate"
    write_rows(p, rows)


def m_reorder(d):
    """A sort comparator drifting: same rows, different order."""
    p = d / "diff" / "paths.tsv"
    lines = p.read_text(encoding="utf-8").splitlines()
    assert len(lines) > 3, "paths.tsv is too short to reorder"
    p.write_text("\n".join([lines[0]] + list(reversed(lines[1:]))) + "\n", encoding="utf-8")


def m_nothing(d):
    pass


MUTATIONS = [
    ("stored_as filled in on an api row   (7771323)", m_stored_as),
    ("a request header value changed     (cd0e876)", m_header),
    ("a chunk blob's bytes changed", m_blob),
    ("an assertion's wording narrowed   (43f66fa)", m_wording),
    ("a document added", m_add_doc),
    ("a document removed", m_drop_doc),
    ("attempts=2 on a probe row         (527833c)", m_attempts),
    ("bytes_in set on a 404 row         (844f5c2)", m_bytes_in),
    ("paths.tsv rows reordered          (sort drift)", m_reorder),
]

if __name__ == "__main__":
    print("mutation control for the differ (source: 884eeea baseline)\n")
    results = [(name, check(name, fn)) for name, fn in MUTATIONS]

    # Negative control: an unmutated copy must read identical, or a control that
    # always fires proves nothing.
    d = fresh()
    same = run(SRC, d)["identical"]
    print(f"  {'OK    ' if same else 'BROKEN!'} unmutated copy reads identical")

    if WORK.exists():
        shutil.rmtree(WORK)
    caught = sum(1 for _, ok in results if ok)
    print(f"\n{caught}/{len(results)} mutations caught; "
          f"negative control {'passed' if same else 'FAILED'}")
    sys.exit(0 if caught == len(results) and same else 1)