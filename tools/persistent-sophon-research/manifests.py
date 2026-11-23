"""Fetch the FULL dispatcher-derived persistent manifests (bounded, trimmed output).

Reads ``samples/dispatch_summary.json`` for the derived URLs, verifies each
manifest's size + MD5 against the dispatcher's own NDJSON description, and saves
only small samples (never the full blob) to ``samples/``.
"""
from __future__ import annotations

import hashlib
import json
import sys
from collections import Counter

from common import SAMPLES, get, make_session, save_json

HOTFIX_MARKERS = ("ctable", "blocks", ".pck", ".usm", ".cuepoint", "svc_catalog", ".blk")


def load_summary() -> dict:
    return json.loads((SAMPLES / "dispatch_summary.json").read_text(encoding="utf-8"))


def expected_manifest(summary: dict, name: str) -> dict | None:
    for entry in summary["manifests"][name]["sample"]:
        if entry.get("remoteName") == name:
            return entry
    return None


def fetch_and_analyze(session, url: str, name: str, expect: dict | None) -> dict:
    r = get(session, url, timeout=60)
    out = {
        "url": url,
        "status": r.status_code,
        "content_type": r.headers.get("content-type"),
        "bytes": len(r.content),
        "md5": hashlib.md5(r.content).hexdigest(),
    }
    print(f"[{name}] HTTP {r.status_code} {out['content_type']} {out['bytes']}B md5={out['md5']}")
    if expect:
        out["expected_size"] = expect.get("fileSize")
        out["expected_md5"] = expect.get("md5")
        out["expected_hash"] = expect.get("hash")
        out["size_matches"] = out["bytes"] == expect.get("fileSize")
        out["md5_matches"] = out["md5"].lower() == str(expect.get("md5", "")).lower()
        print(f"         size_match={out['size_matches']} md5_match={out['md5_matches']}")
    if r.status_code != 200:
        out["body_head"] = r.text[:300]
        return out
    text = r.content.decode("utf-8", "replace")
    lines = [ln for ln in text.splitlines() if ln.strip()]
    objs = []
    for ln in lines:
        try:
            o = json.loads(ln)
            if isinstance(o, dict):
                objs.append(o)
        except json.JSONDecodeError:
            pass
    keys = Counter(k for o in objs for k in o)
    hotfix = []
    for o in objs:
        rn = str(o.get("remoteName", ""))
        if any(m in rn.lower() for m in HOTFIX_MARKERS):
            hotfix.append(
                {k: o.get(k) for k in ("remoteName", "md5", "hash", "fileSize") if k in o}
            )
    out.update(
        {
            "lines": len(lines),
            "parsed_objects": len(objs),
            "keys_seen": dict(keys),
            "hotfix_like_count": len(hotfix),
            "hotfix_like_sample": hotfix[:120],
            "first_40": objs[:40],
        }
    )
    (SAMPLES / f"{name}.full.head.ndjson").write_text(
        "\n".join(json.dumps(o, ensure_ascii=False) for o in objs[:40]) + "\n",
        encoding="utf-8",
    )
    (SAMPLES / f"{name}.hotfix.sample.json").write_text(
        json.dumps(hotfix[:300], indent=2, ensure_ascii=False), encoding="utf-8"
    )
    # A compact name->md5/hash table for cross-matching Sophon manifests later.
    (SAMPLES / f"{name}.names.tsv").write_text(
        "\n".join(
            f"{o.get('remoteName','')}\t{o.get('md5','')}\t{o.get('hash','')}\t{o.get('fileSize','')}"
            for o in objs
        ),
        encoding="utf-8",
    )
    return out


def main() -> int:
    summary = load_summary()
    urls = summary["derived_urls"]
    s = make_session()
    results = {}
    for name, url_key in (
        ("res_versions_external", "res_versions_external_url"),
        ("data_versions", "data_versions_url"),
    ):
        results[name] = fetch_and_analyze(
            s, urls[url_key], name, expected_manifest(summary, name)
        )

    base_rev_url = urls.get("base_revision_url")
    if base_rev_url:
        r = get(s, base_rev_url, timeout=30)
        results["base_revision"] = {
            "url": base_rev_url,
            "status": r.status_code,
            "content_type": r.headers.get("content-type"),
            "body": r.text[:200],
        }
        print(f"[base_revision] HTTP {r.status_code} {r.headers.get('content-type')} {r.text[:100]!r}")

    save_json("full_manifests_summary.json", results)
    return 0


if __name__ == "__main__":
    sys.exit(main())
