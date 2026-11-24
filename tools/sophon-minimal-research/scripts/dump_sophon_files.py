#!/usr/bin/env python3
"""Dump the EXACT file set advertised by the live Sophon getBuild manifests.

Wraps the re-runnable helpers in tools/persistent-sophon-research (same live
snapshot methodology as docs/08 §4.5) and writes the full, unabridged file list
to generated/ (gitignored). Also prints a targeted check for the *_streaming /
metadata files the minimal-install question hinges on.

Only MANIFESTS are downloaded (a few MB), never game bytes.

Usage (from anywhere):
    python scripts/dump_sophon_files.py            # current build (7.2.0)
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPR = HERE.parent.parent / "persistent-sophon-research"
sys.path.insert(0, str(SPR))

from common import SAMPLES, get, make_session  # noqa: E402
from crossmatch import parse_sophon_files  # noqa: E402
from probe import build_url  # noqa: E402

OUT = HERE.parent / "generated"
OUT.mkdir(exist_ok=True)

# Files the question asks about, expressed as Sophon manifest paths.
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
    "config.ini",
    "pkg_version",
    "GenshinImpact.exe",
    "GenshinImpact_Data/StreamingAssets/Persistent/plugindelayed.config",
]


def decompress(body: bytes) -> bytes:
    import gzip
    import zstandard
    if body[:2] == b"\x1f\x8b":
        return gzip.decompress(body)
    if body[:4] == b"\x28\xb5\x2f\xfd":
        return zstandard.ZstdDecompressor().decompress(body)
    return body


def main() -> int:
    s = make_session()
    probe = json.loads((SAMPLES / "probe_summary.json").read_text(encoding="utf-8"))
    branch = probe["branch_packages"][0]["main"]
    print(f"[branch] tag={branch.get('tag')} package_id={branch.get('package_id')}")

    url = build_url(branch)
    r = get(s, url, timeout=60)
    print(f"[getBuild] HTTP {r.status_code} {url}")
    build = r.json()["data"]

    all_files: list[dict] = []
    manifests_meta: list[dict] = []
    for m in build.get("manifests", []):
        mid = m["manifest"]["id"]
        purl = m["manifest_download"]["url_prefix"].rstrip("/") + "/" + mid
        r = get(s, purl, timeout=120)
        body = decompress(r.content)
        files = parse_sophon_files(body)
        field = m.get("matching_field")
        manifests_meta.append({
            "field": field, "id": mid, "checksum": m["manifest"].get("checksum"),
            "file_count": len(files), "url": purl,
        })
        for f in files:
            all_files.append({"field": field, **f})
        print(f"[manifest] {field}: {len(files)} files")

    # Full dump (gitignored).
    tsv = OUT / "sophon_files_720.tsv"
    with tsv.open("w", encoding="utf-8", newline="\n") as fh:
        fh.write("field\tfile\tsize\tmd5\tchunks\n")
        for f in all_files:
            fh.write(f"{f['field']}\t{f['file']}\t{f['size']}\t{f['md5']}\t{f['chunks']}\n")

    summary = {
        "build_tag": branch.get("tag"),
        "build_id": build.get("build_id"),
        "manifests": manifests_meta,
        "total_manifest_file_entries": len(all_files),
        "dump": str(tsv.relative_to(HERE.parent)),
    }
    (OUT / "sophon_files_summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")

    # Targeted presence check.
    paths = {f["file"].replace("\\", "/") for f in all_files}
    print("\n=== TARGET FILE PRESENCE IN SOPHON ===")
    for t in TARGETS:
        print(f"  {'YES' if t in paths else 'NO ':3}  {t}")

    # Any *_streaming / *revision / svc_catalog / ctable anywhere?
    print("\n=== ALL paths containing streaming/revision/svc_catalog/ctable/pkg_version/config.ini ===")
    for f in sorted(paths):
        low = f.lower()
        if any(k in low for k in ("streaming", "revision", "svc_catalog", "ctable", "pkg_version", "config.ini")):
            print("  ", f)
    return 0


if __name__ == "__main__":
    sys.exit(main())
