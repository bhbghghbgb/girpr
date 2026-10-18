#!/usr/bin/env python3
"""
Cross-commit differ for the `gircap` artifact bundle.

Two jobs:

1. `fingerprint`  - normalise one bundle into a canonical, comparable form.
2. `diff`         - compare two fingerprints and report what actually moved.

The volatile set (timestamps, elapsed_ms, the capture id, the --out path) is
*discovered empirically* by comparing two captures of one binary; it is not
assumed.  Everything else is compared byte-for-byte, so a field that appears,
disappears or changes value is loud.
"""
import argparse
import gzip
import io
import json
import os
import re
import sys
from pathlib import Path

# --------------------------------------------------------------------------- #
# volatile-token scrubbing
# --------------------------------------------------------------------------- #

_TS = re.compile(rb"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z")
_ELAPSED = re.compile(rb'("elapsed_ms"\s*:\s*)\d+')
# The mock binds 127.0.0.1:0 and gets an ephemeral port, which the bundle then
# records in MANIFEST.json -> biz.hyp_base, in every getBuild body, in every
# manifest meta document, in every patchmap url, and in every request row.
# Without this, two probes of *different* commits differ on 13 documents for no
# reason at all - which is exactly the false positive that would invent
# boundaries.
_PORT = re.compile(rb"127\.0\.0\.1:\d+")


def _repl(m, tok):
    return m.expand(tok.encode())


def scrub(b, capture_id, out_root):
    """Replace the tokens that differ between two runs of *one* binary."""
    for spelling in (out_root, out_root.replace("\\", "\\\\")):
        if spelling:
            b = b.replace(spelling.encode(), b"<OUT>")
    if capture_id:
        b = b.replace(capture_id.encode(), b"<CAPTURE_ID>")
    b = _TS.sub(lambda m: b"<TS>", b)
    b = _ELAPSED.sub(lambda m: m.group(1) + b"<MS>", b)
    b = _PORT.sub(b"127.0.0.1:<PORT>", b)
    return b


# --------------------------------------------------------------------------- #
# cascade exemptions
# --------------------------------------------------------------------------- #
#
# `api/requests.ndjson` is not byte-reproducible (ts, elapsed_ms), so its own
# digest moves, and four documents *record* that digest.  Those rows are blanked
# rather than compared; every other byte in those documents is still compared.
#
# The list is HEAD's.  `--strict-exemptions` asserts each entry actually matched,
# so an exemption that stops matching is an error rather than a silent hole.

DERIVED = [
    "api/requests.ndjson",
    "MANIFEST.json",
    "README.md",
    "verify/selfcheck.json",
]


def blank_rows(doc, paths):
    """Blank bytes/md5/sha256 on the `files[]`/`entries[]` row naming `paths`."""
    out, in_row, hits = [], False, 0
    for line in doc.splitlines(keepends=True):
        t = line.strip()
        if t.startswith('"path"'):
            in_row = any(f'"{p}"' in t for p in paths)
            hits += 1 if in_row else 0
        derived = in_row and t.split(":")[0].strip() in ('"bytes"', '"md5"', '"sha256"')
        out.append(_blank(line) if derived else line)
        if in_row and t.startswith("}"):
            in_row = False
    return "".join(out), hits


def blank_key(doc, key, minimum=1):
    out, hits = [], 0
    for line in doc.splitlines(keepends=True):
        if line.lstrip().startswith(f'"{key}"'):
            out.append(_blank(line))
            hits += 1
        else:
            out.append(line)
    return "".join(out), hits


def blank_prose(doc, marker):
    """Blank the number immediately preceding `marker`."""
    out, hits = [], 0
    for line in doc.splitlines(keepends=True):
        at = line.find(marker)
        if at > 0:
            head = line[:at]
            digits = len(head) - len(head.rstrip("0123456789"))
            if digits:
                out.append(head[: len(head) - digits] + "<N>" + line[at:])
                hits += 1
                continue
        out.append(line)
    return "".join(out), hits


def _blank(line):
    at = line.find(":")
    return f"{line[:at]}: <blank>\n" if at >= 0 else line


def normalize_doc(rel, body, capture_id, out_root, strict, volatile=None):
    n = scrub(body, capture_id, out_root)
    s = n.decode("utf-8", "replace")
    report = {}
    volatile = volatile or set()
    if rel == "MANIFEST.json":
        s, h = blank_rows(s, volatile | {"api/requests.ndjson"})
        s, h2 = blank_key(s, "bytes_on_disk", 1)
        report["blanked"] = h + h2
    elif rel == "README.md":
        s, h = blank_prose(s, " B on disk")
        report["blanked"] = h
    elif rel == "verify/checksums.json":
        s, h = blank_rows(s, volatile | set(DERIVED))
        s, h2 = blank_key(s, "total_bytes", 1)
        report["blanked"] = h + h2
    elif rel == "verify/selfcheck.json":
        s, h = blank_prose(s, " B, every digest re-hashed")
        report["blanked"] = h
    elif rel in DERIVED:
        report["blanked"] = 0
    if strict and rel in ("MANIFEST.json", "verify/checksums.json"):
        if report["blanked"] == 0:
            raise SystemExit(
                f"strict: {rel} matched no exemption row - the exemption list has "
                f"stopped lining up with the document and would compare nothing"
            )
    return s.encode("utf-8"), report


# --------------------------------------------------------------------------- #
# fingerprint
# --------------------------------------------------------------------------- #


def find_bundle(root):
    for dirpath, _dirnames, filenames in os.walk(root):
        if "MANIFEST.json" in filenames:
            return Path(dirpath)
    raise SystemExit(f"no bundle under {root}")


def fingerprint(bundle, out_root, strict=False, digest_gz=False, raw=False):
    bundle = find_bundle(bundle) if not Path(bundle, "MANIFEST.json").is_file() else Path(bundle)
    capture_id = bundle.name

    raws = {}
    for dirpath, _d, filenames in os.walk(bundle):
        for fn in filenames:
            p = Path(dirpath, fn)
            raws[str(p.relative_to(bundle)).replace("\\", "/")] = p.read_bytes()

    # Which files' *own bytes* carry a run-local value?  Their content is
    # comparable after scrubbing, but the digest other documents record for them
    # is not - and a digest is exactly the field a change to the tool would move.
    # Detected, not listed, so the exemption cannot go stale.
    volatile = {
        rel
        for rel, b in raws.items()
        if b"127.0.0.1:" in b or rel in DERIVED
    }

    files = {}
    for rel, raw_bytes in raws.items():
        body = raw_bytes
        if rel.endswith(".gz"):
            if digest_gz:
                import hashlib

                files[rel] = b"<GZBYTES " + hashlib.sha256(body).hexdigest().encode() + b">"
                continue
            body = gzip.decompress(body)
        if raw:
            files[rel] = body
        else:
            files[rel], _ = normalize_doc(rel, body, capture_id, out_root, strict, volatile)
    return files


def save(fp, dest):
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    for rel, body in fp.items():
        p = dest / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(body)


# --------------------------------------------------------------------------- #
# diff
# --------------------------------------------------------------------------- #


def load(p):
    p = Path(p)
    if p.is_dir():
        out = {}
        for f in sorted(p.rglob("*")):
            if f.is_file():
                out[str(f.relative_to(p)).replace("\\", "/")] = f.read_bytes()
        return out
    return json.loads(p.read_text())


def diff(a, b, label_a="A", label_b="B", limit=40):
    only_a = sorted(set(a) - set(b))
    only_b = sorted(set(b) - set(a))
    changed = sorted(k for k in set(a) & set(b) if a[k] != b[k])
    same = len(set(a) & set(b)) - len(changed)
    if not (only_a or only_b or changed):
        return {"identical": True, "same": same, "files": len(a)}
    return {
        "identical": False,
        "same": same,
        "files": len(a),
        "only_a": only_a,
        "only_b": only_b,
        "changed": changed,
        "details": {k: _detail(a[k], b[k]) for k in changed[:limit]},
        "detail_truncated": max(0, len(changed) - limit),
    }


def _detail(x, y, ctx=90):
    """A compact, readable account of the first few differing regions."""
    n = min(len(x), len(y))
    spans, i, shown = [], 0, 0
    while i < n and shown < 6:
        if x[i] != y[i]:
            lo = max(0, i - 30)
            hi = min(n, i + 90)
            spans.append(
                {
                    "at": i,
                    "a": x[lo:hi].decode("utf-8", "replace"),
                    "b": y[lo:hi].decode("utf-8", "replace"),
                }
            )
            shown += 1
            i = hi
        else:
            i += 1
    if len(x) != len(y):
        spans.append({"at": min(len(x), len(y)), "len": [len(x), len(y)]})
    return spans


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["fingerprint", "diff", "save"])
    ap.add_argument("a")
    ap.add_argument("b", nargs="?", default=None)
    ap.add_argument("--out", default=None)
    ap.add_argument("--out-root", default=None)
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--digest-gz", action="store_true")
    ap.add_argument("--raw", action="store_true")
    ap.add_argument("--limit", type=int, default=40)
    args = ap.parse_args()

    if args.mode in ("fingerprint", "save"):
        fp = fingerprint(
            args.a, args.out_root or str(Path(args.a)), args.strict, args.digest_gz, args.raw
        )
        if args.out:
            save(fp, args.out)
        print(json.dumps({"files": len(fp), "out": args.out}, indent=2))
    else:
        res = diff(load(args.a), load(args.b), limit=args.limit)
        print(json.dumps(res, indent=2))


if __name__ == "__main__":
    main()