"""Precisely cross-match the live Sophon manifests against dispatcher Persistent names.

The earlier ``probe.py`` 'hotfix-like' counter was intentionally broad (it even
matched ``hpatchz.dll``). This script does byte-level matching:

  * exact relative-path match with the SAME md5  -> the server has those exact
    bytes in a Sophon chunk manifest;
  * exact relative-path match with a DIFFERENT md5 -> base-install version only;
  * md5 match at any path -> same bytes shipped as a different file.

Path normalisation mirrors Collapse's ``GetRelativePathByRemoteName``
(``Fetch.Persistent.cs:239-278``): dispatcher ``blocks/00/x.blk`` corresponds to
Sophon ``GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/x.blk``.
"""
from __future__ import annotations

import gzip
import json
import sys
from collections import defaultdict
from pathlib import Path

import zstandard

from common import SAMPLES, get, make_session, parse_fields, save_json
from probe import build_url

SOPHON_PREFIXES = ("GenshinImpact_Data/", "GenshinImpact_Data/Persistent/")


def normalize_sophon(path: str) -> str:
    p = path.replace("\\", "/").lstrip("/")
    if p.startswith("GenshinImpact_Data/"):
        p = p[len("GenshinImpact_Data/") :]
    if p.startswith("StreamingAssets/AssetBundles/"):
        p = p[len("StreamingAssets/AssetBundles/") :]
    elif p.startswith("StreamingAssets/"):
        p = p[len("StreamingAssets/") :]
    elif p.startswith("Persistent/"):
        p = "PERSISTENT:" + p[len("Persistent/") :]
    return p.lower()


def parse_sophon_files(proto: bytes) -> list[dict]:
    root = parse_fields(proto)
    files = []
    for wt, buf in root.get(1, []):
        if wt != 2:
            continue
        f = parse_fields(buf)
        def s(n):
            v = f.get(n, [(0, b"")])[0][1]
            return v.decode("utf-8", "replace") if isinstance(v, bytes) else str(v)
        def i(n):
            v = f.get(n, [(0, 0)])[0][1]
            return v if isinstance(v, int) else 0
        files.append({"file": s(1), "size": i(4), "md5": s(5).lower(), "chunks": len(f.get(2, []))})
    return files


def load_dispatcher() -> list[dict]:
    rows = []
    for name in ("res_versions_external", "data_versions"):
        p = SAMPLES / f"{name}.names.tsv"
        if not p.exists():
            continue
        for ln in p.read_text(encoding="utf-8").splitlines():
            parts = ln.split("\t")
            if len(parts) >= 3 and parts[0]:
                rows.append({"manifest": name, "remoteName": parts[0],
                             "md5": parts[1].lower(), "hash": parts[2],
                             "fileSize": parts[3] if len(parts) > 3 else ""})
    return rows


def main() -> int:
    s = make_session()
    disp = load_dispatcher()
    print(f"[dispatcher] {len(disp)} persistent asset rows")
    disp_by_path = defaultdict(list)
    disp_by_md5 = defaultdict(list)
    for d in disp:
        disp_by_path[normalize_sophon(d["remoteName"])].append(d)
        disp_by_md5[d["md5"]].append(d)

    probe = json.loads((SAMPLES / "probe_summary.json").read_text(encoding="utf-8"))
    branches = probe["branch_packages"]
    out: dict = {"dispatcher_rows": len(disp), "builds": []}
    for b in branches:
        pkg = b.get("main") or {}
        if not pkg:
            continue
        data = get_json = None
        url = build_url(pkg)
        r = get(s, url, timeout=60)
        print(f"[getBuild] HTTP {r.status_code} {url}")
        if r.status_code != 200:
            continue
        build = r.json()["data"]
        for m in build.get("manifests", []):
            mid = m["manifest"]["id"]
            purl = m["manifest_download"]["url_prefix"].rstrip("/") + "/" + mid
            r = get(s, purl, timeout=120)
            print(f"[manifest] HTTP {r.status_code} {len(r.content)}B {m.get('matching_field')}")
            body = r.content
            if body[:2] == b"\x1f\x8b":
                body = gzip.decompress(body)
            elif body[:4] == b"\x28\xb5\x2f\xfd":
                body = zstandard.ZstdDecompressor().decompress(body)
            files = parse_sophon_files(body)

            persistent_paths = [f for f in files if "persistent" in f["file"].lower()]
            same_bytes = []
            path_only = []
            md5_other_path = []
            for f in files:
                np = normalize_sophon(f["file"])
                for d in disp_by_path.get(np, []):
                    (same_bytes if d["md5"] == f["md5"] else path_only).append(
                        {"sophon_file": f["file"], "sophon_md5": f["md5"],
                         "dispatcher_remoteName": d["remoteName"],
                         "dispatcher_md5": d["md5"], "manifest": d["manifest"]}
                    )
                if f["md5"] in disp_by_md5 and np not in disp_by_path:
                    md5_other_path.append(
                        {"sophon_file": f["file"], "dispatcher_remoteName": disp_by_md5[f["md5"]][0]["remoteName"]}
                    )
            rec = {
                "field": m.get("matching_field"), "manifest_id": mid,
                "manifest_url": purl, "file_count": len(files),
                "persistent_path_count": len(persistent_paths),
                "persistent_path_sample": [f["file"] for f in persistent_paths[:20]],
                "same_path_same_md5_count": len(same_bytes),
                "same_path_same_md5_sample": same_bytes[:30],
                "same_path_diff_md5_count": len(path_only),
                "same_path_diff_md5_sample": path_only[:30],
                "md5_match_other_path_count": len(md5_other_path),
                "md5_match_other_path_sample": md5_other_path[:20],
                "path_prefix_counts": {},
            }
            prefixes = defaultdict(int)
            for f in files:
                p = f["file"].replace("\\", "/")
                prefix = "/".join(p.split("/")[:3])
                prefixes[prefix] += 1
            rec["path_prefix_counts"] = dict(sorted(prefixes.items(), key=lambda x: -x[1])[:20])
            out["builds"].append(rec)
            print(f"    field={rec['field']}: files={len(files)} "
                  f"Persistent-paths={len(persistent_paths)} "
                  f"SAME_PATH+SAME_MD5={len(same_bytes)} SAME_PATH+DIFF_MD5={len(path_only)} "
                  f"MD5_OTHER_PATH={len(md5_other_path)}")
    save_json("crossmatch_summary.json", out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
