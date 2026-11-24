"""Fetch the FULL dispatcher-derived persistent manifests (bounded, trimmed output).

Reads the per-biz ``dispatch_summary.json`` for the derived URLs, verifies each
manifest's size + MD5 against the dispatcher's own NDJSON descriptor, and saves
only small samples (never the full blob) to ``samples/``.

All three manifests are fetched: res, data and silence. The live 2026-11-23
probe fetched only res+data; the silence descriptor (proto root 3 -> 20) is
captured here as required by docs/03 §7.2 step 3.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import Counter

from common import SAMPLES, get, make_session, save_json

HOTFIX_MARKERS = ("ctable", "blocks", ".pck", ".usm", ".cuepoint", "svc_catalog", ".blk")
BIZ = ("hk4e_global", "hk4e_cn", "hk4e_bilibili")


def load_summary(biz: str) -> dict:
    prefix = "" if biz == "hk4e_global" else f"{biz}."
    return json.loads((SAMPLES / f"{prefix}dispatch_summary.json").read_text(encoding="utf-8"))


def expected_manifest(summary: dict, name: str) -> dict | None:
    for entry in summary["manifests"][name]["sample"]:
        if entry.get("remoteName") in (name, "data_versions"):
            return entry
    return None


def is_hotfix(remote_name: str) -> bool:
    return any(m in remote_name.lower() for m in HOTFIX_MARKERS)


def fetch_and_analyze(session, url: str, name: str, expect: dict | None, prefix: str) -> dict:
    r = get(session, url, timeout=60)
    out = {
        "url": url,
        "status": r.status_code,
        "content_type": r.headers.get("content-type"),
        "bytes": len(r.content),
        "md5": hashlib.md5(r.content).hexdigest(),
    }
    print(f"[{prefix}{name}] HTTP {r.status_code} {out['content_type']} {out['bytes']}B md5={out['md5']}")
    if expect:
        out["expected_size"] = expect.get("fileSize")
        out["expected_md5"] = expect.get("md5")
        out["expected_hash"] = expect.get("hash")
        out["size_matches"] = out["bytes"] == expect.get("fileSize")
        out["md5_matches"] = out["md5"].lower() == str(expect.get("md5", "")).lower()
        print(
            f"         size_match={out['size_matches']} md5_match={out['md5_matches']} "
            f"expected_size={out['expected_size']} expected_md5={out['expected_md5']}"
        )
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
        if is_hotfix(rn):
            hotfix.append(
                {k: o.get(k) for k in ("remoteName", "md5", "hash", "fileSize", "isPatch") if k in o}
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
    (SAMPLES / f"{prefix}{name}.full.head.ndjson").write_text(
        "\n".join(json.dumps(o, ensure_ascii=False) for o in objs[:40]) + "\n",
        encoding="utf-8",
    )
    (SAMPLES / f"{prefix}{name}.hotfix.sample.json").write_text(
        json.dumps(hotfix[:300], indent=2, ensure_ascii=False), encoding="utf-8"
    )
    # One representative hotfix row per biz/manifest (the tests' hotfix fixture).
    one = next((h for h in hotfix if h.get("isPatch")), hotfix[0] if hotfix else None)
    if one:
        (SAMPLES / f"{prefix}{name}.hotfix_row.json").write_text(
            json.dumps(one, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        out["one_hotfix_row"] = one
    # A compact name->md5/hash table for cross-matching Sophon manifests later.
    (SAMPLES / f"{prefix}{name}.names.tsv").write_text(
        "\n".join(
            f"{o.get('remoteName','')}\t{o.get('md5','')}\t{o.get('hash','')}\t{o.get('fileSize','')}"
            for o in objs
        ),
        encoding="utf-8",
    )
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--biz", default="hk4e_global", choices=BIZ)
    args = ap.parse_args()
    biz = args.biz
    prefix = "" if biz == "hk4e_global" else f"{biz}."

    summary = load_summary(biz)
    urls = summary["derived_urls"]
    s = make_session()
    results = {}
    for name, url_key in (
        ("res_versions_external", "res_versions_external_url"),
        ("data_versions", "data_versions_url"),
        ("silence_data_versions", "silence_data_versions_url"),
    ):
        url = urls.get(url_key)
        if not url:
            print(f"[{prefix}{name}] no derived URL — skipping")
            continue
        results[name] = fetch_and_analyze(
            s, url, name, expected_manifest(summary, name), prefix
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
        print(f"[{prefix}base_revision] HTTP {r.status_code} {r.headers.get('content-type')} {r.text[:100]!r}")

    save_json(f"{prefix}full_manifests_summary.json", results)
    checks = {
        k: (v.get("size_matches"), v.get("md5_matches"))
        for k, v in results.items()
        if "md5_matches" in v
    }
    print(f"[verdict] {biz}: {checks}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
