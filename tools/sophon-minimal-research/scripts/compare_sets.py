#!/usr/bin/env python3
"""Compare the exact Sophon file set (7.2.0) against the torrent/F: base copy.

Also hashes the target *_streaming / metadata files on F: (sequential, one disk)
and compares their MD5 with the Sophon manifest MD5, proving whether a Sophon
repair would restore byte-identical verification inputs.

Inputs (both gitignored generated artifacts):
  generated/sophon_files_720.tsv
  ../persistent-launch-research/snapshots/F-fresh/F-snapshot.jsonl   (torrent copy)
"""
from __future__ import annotations

import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SOPHON_TSV = ROOT / "generated" / "sophon_files_720.tsv"
F_SNAP = (ROOT.parent / "persistent-launch-research" / "snapshots"
          / "F-fresh" / "F-snapshot.jsonl")

TARGETS = [
    "GenshinImpact_Data/StreamingAssets/res_versions_streaming",
    "GenshinImpact_Data/StreamingAssets/data_versions_streaming",
    "GenshinImpact_Data/StreamingAssets/silence_versions_streaming",
    "GenshinImpact_Data/StreamingAssets/VideoAssets/video_versions_streaming",
    "GenshinImpact_Data/StreamingAssets/AssetBundles/res_revision",
    "GenshinImpact_Data/StreamingAssets/AssetBundles/data_revision",
    "GenshinImpact_Data/StreamingAssets/AssetBundles/silence_revision",
    "GenshinImpact_Data/StreamingAssets/ctable_streaming.dat",
    "GenshinImpact_Data/StreamingAssets/svc_catalog",
    "pkg_version",
    "GenshinImpact.exe",
]
F_GAME = Path(r"F:\Genshin Impact game")


def md5(path: Path) -> str:
    h = hashlib.md5()
    with path.open("rb") as f:
        while True:
            b = f.read(4 << 20)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def main() -> int:
    sophon: dict[str, dict] = {}
    for ln in SOPHON_TSV.read_text(encoding="utf-8").splitlines()[1:]:
        field, file, size, md5v, chunks = ln.split("\t")
        sophon.setdefault(file.replace("\\", "/").lower(), {
            "file": file, "field": field, "size": int(size), "md5": md5v,
        })
    torrent: dict[str, dict] = {}
    for ln in F_SNAP.read_text(encoding="utf-8").splitlines():
        rec = json.loads(ln)
        torrent[rec["rel"].replace("\\", "/").lower()] = rec

    ss, ts = set(sophon), set(torrent)
    both = ss & ts
    sophon_only = ss - ts
    torrent_only = ts - ss
    print(f"sophon unique paths : {len(ss)}")
    print(f"torrent unique paths: {len(ts)}")
    print(f"both                : {len(both)}")
    print(f"sophon-only         : {len(sophon_only)}")
    print(f"torrent-only        : {len(torrent_only)}")

    # Categorize torrent-only by first two path components.
    cat = Counter()
    for p in torrent_only:
        parts = p.split("/")
        key = "/".join(parts[:2]) if len(parts) > 1 else parts[0]
        cat[key] += 1
    print("\n=== TORRENT-ONLY (in torrent, not in Sophon) by prefix ===")
    for k, n in cat.most_common():
        print(f"  {n:5}  {k}")

    print("\n=== SOPHON-ONLY (in Sophon, not in torrent) ===")
    for p in sorted(sophon_only):
        print("  ", sophon[p]["file"], f"[{sophon[p]['field']}]")

    # Size the two sets using manifest sizes / snapshot sizes.
    print(f"\nsophon manifest sum bytes : {sum(v['size'] for v in sophon.values()):,}")
    print(f"torrent sum bytes         : {sum(v['size'] for v in torrent.values()):,}")
    print(f"torrent-only bytes        : {sum(torrent[p]['size'] for p in torrent_only):,}")

    # Target MD5 check: Sophon manifest vs F: file on disk (torrent, unchanged base).
    print("\n=== TARGET MD5: Sophon manifest vs F: (torrent base) ===")
    target_results = []
    for t in TARGETS:
        key = t.lower()
        s = sophon.get(key)
        local = F_GAME / t
        local_md5 = md5(local) if local.exists() else None
        match = bool(s and local_md5 and s["md5"].lower() == local_md5)
        target_results.append({
            "target": t,
            "in_sophon": bool(s),
            "sophon_md5": s["md5"] if s else None,
            "sophon_size": s["size"] if s else None,
            "f_md5": local_md5,
            "f_exists": local.exists(),
            "match": match,
        })
        print(f"  {'MATCH' if match else 'DIFF ':5} sophon={s['md5'] if s else '-':32} "
              f"F={local_md5 or '-':32} {t}")

    summary = {
        "sophon_unique_paths": len(ss),
        "torrent_unique_paths": len(ts),
        "both": len(both),
        "sophon_only": len(sophon_only),
        "torrent_only": len(torrent_only),
        "torrent_only_by_prefix": dict(cat.most_common()),
        "sophon_only_paths": [sophon[p]["file"] for p in sorted(sophon_only)],
        "sophon_manifest_sum_bytes": sum(v["size"] for v in sophon.values()),
        "torrent_sum_bytes": sum(v["size"] for v in torrent.values()),
        "torrent_only_bytes": sum(torrent[p]["size"] for p in torrent_only),
        "target_md5_check": target_results,
    }
    out = ROOT / "generated" / "compare_summary.json"
    out.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    main()
