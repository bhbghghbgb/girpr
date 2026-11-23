"""Definitive byte-level check: are dispatcher *hotfix* md5s in any Sophon manifest?

Pulls all Sophon build manifests advertised for the current branch AND its
``diff_tags`` (older builds), then asks for each dispatcher row:

  * ``same_path_same_md5`` — bytes already in the base Sophon build (not a hotfix)
  * ``hotfix_md5_in_sophon`` — the hotfix bytes are themselves in a Sophon manifest
  * ``hotfix_md5_absent`` — no Sophon manifest carries those bytes

Also reports the single ``Persistent`` path seen in the game manifest and the
dispatcher rows explicitly flagged ``isPatch: true``.
"""
from __future__ import annotations

import gzip
import json
import sys

import zstandard

from common import SAMPLES, get, make_session, parse_fields, save_json
from crossmatch import normalize_sophon, parse_sophon_files
from probe import build_url


def load_dispatcher_rows() -> list[dict]:
    rows = []
    for name in ("res_versions_external", "data_versions"):
        p = SAMPLES / f"{name}.names.tsv"
        for ln in p.read_text(encoding="utf-8").splitlines():
            parts = ln.split("\t")
            if len(parts) >= 3:
                rows.append({"manifest": name, "remoteName": parts[0],
                             "md5": parts[1].lower(), "hash": parts[2],
                             "fileSize": parts[3] if len(parts) > 3 else ""})
    return rows


def main() -> int:
    s = make_session()
    probe = json.loads((SAMPLES / "probe_summary.json").read_text(encoding="utf-8"))
    branch = probe["branch_packages"][0]["main"]
    rows = load_dispatcher_rows()
    print(f"[dispatcher] {len(rows)} rows")

    # Collect every Sophon manifest from current + diff_tag builds.
    sophon_md5: dict[str, list[str]] = {}
    sophon_paths: dict[str, str] = {}
    manifests_seen = []
    for tag in [None] + list(branch.get("diff_tags") or []):
        url = build_url(branch, tag)
        r = get(s, url, timeout=60)
        print(f"[getBuild] tag={tag or branch.get('tag')} HTTP {r.status_code}")
        if r.status_code != 200 or r.json().get("retcode") != 0:
            print("           skipped:", r.text[:160])
            continue
        d = r.json()["data"]
        for m in d.get("manifests", []):
            mid = m["manifest"]["id"]
            purl = m["manifest_download"]["url_prefix"].rstrip("/") + "/" + mid
            r = get(s, purl, timeout=120)
            if r.status_code != 200:
                print(f"[manifest] HTTP {r.status_code} (skipped)")
                continue
            body = r.content
            if body[:2] == b"\x1f\x8b":
                body = gzip.decompress(body)
            elif body[:4] == b"\x28\xb5\x2f\xfd":
                body = zstandard.ZstdDecompressor().decompress(body)
            files = parse_sophon_files(body)
            manifests_seen.append({"build_tag": tag or branch.get("tag"),
                                   "field": m.get("matching_field"), "id": mid,
                                   "file_count": len(files), "url": purl})
            for f in files:
                sophon_md5.setdefault(f["md5"], []).append(f["file"])
                sophon_paths.setdefault(normalize_sophon(f["file"]), f["md5"])
            print(f"[manifest] {m.get('matching_field')} tag={tag or branch.get('tag')} "
                  f"{len(files)} files, md5 pool now {len(sophon_md5)}")

    categorized = {"same_path_same_md5": [], "same_path_diff_md5_hotfix_md5_present": [],
                   "same_path_diff_md5_hotfix_md5_absent": [], "path_not_in_sophon": [],
                   "hotfix_md5_elsewhere_in_sophon": []}
    for row in rows:
        np = normalize_sophon(row["remoteName"])
        base_md5 = sophon_paths.get(np)
        if base_md5 is None:
            categorized["path_not_in_sophon"].append(row)
            if row["md5"] in sophon_md5:
                categorized["hotfix_md5_elsewhere_in_sophon"].append(
                    {**row, "sophon_locations": sophon_md5[row["md5"]][:5]})
        elif base_md5 == row["md5"]:
            categorized["same_path_same_md5"].append(row)
        else:
            if row["md5"] in sophon_md5:
                categorized["same_path_diff_md5_hotfix_md5_present"].append(
                    {**row, "base_md5": base_md5, "sophon_locations": sophon_md5[row["md5"]][:5]})
            else:
                categorized["same_path_diff_md5_hotfix_md5_absent"].append(
                    {**row, "base_md5": base_md5})

    print("\n=== RESULTS ===")
    for k, v in categorized.items():
        print(f"{k}: {len(v)}")
    print("\nHotfix md5 present in Sophon (first 10):")
    for r in categorized["same_path_diff_md5_hotfix_md5_present"][:10]:
        print("  ", r)
    print("\nDispatcher md5 found at another Sophon path (first 10):")
    for r in categorized["hotfix_md5_elsewhere_in_sophon"][:10]:
        print("  ", r)

    # Rows whose md5 is absent from *every* Sophon manifest (candidate true
    # hotfix-only files, not just base files at a different path).
    md5_absent_entirely = [r for r in rows if r["md5"] not in sophon_md5]
    print(f"\nDispatcher rows whose md5 is absent from ALL Sophon builds: {len(md5_absent_entirely)}")
    for r in md5_absent_entirely[:20]:
        print("  ", r)

    # isPatch:true rows in the dispatcher's own NDJSON (sampled first 40 lines).
    p = SAMPLES / "full_manifests_summary.json"
    full = json.loads(p.read_text(encoding="utf-8"))
    patch_rows = []
    for name in ("res_versions_external", "data_versions"):
        patch_rows += [o for o in full[name]["first_40"] if o.get("isPatch")]

    result = {
        "manifests_seen": manifests_seen,
        "dispatcher_rows": len(rows),
        "counts": {k: len(v) for k, v in categorized.items()},
        "md5_absent_from_all_sophon_builds_count": len(md5_absent_entirely),
        "md5_absent_from_all_sophon_builds": md5_absent_entirely[:100],
        "hotfix_md5_present_sample": categorized["same_path_diff_md5_hotfix_md5_present"][:30],
        "hotfix_md5_absent_sample": categorized["same_path_diff_md5_hotfix_md5_absent"][:30],
        "md5_at_other_path_sample": categorized["hotfix_md5_elsewhere_in_sophon"][:30],
        "isPatch_true_in_first40_sample": patch_rows,
        "persistent_named_sophon_paths": [f for f in sophon_paths if "persistent" in f],
    }
    save_json("hotfix_check_summary.json", result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
