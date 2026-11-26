#!/usr/bin/env python3
"""Check E:'s *_persist manifests against the files actually on disk.

Usage: python verify_persist_manifest.py SNAPSHOT.jsonl SYNCED_GAME_DIR
"""
import hashlib
import json
import os
import sys


def rows(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


def folder(name):
    if name.endswith(".pck"):
        return "AudioAssets"
    if ("blocks" in name and name.endswith(".blk")) or name.endswith("svc_catalog"):
        return "AssetBundles"
    if name.endswith(".usm") or name.endswith(".cuepoint"):
        return "VideoAssets"
    return ""


def main():
    snap = {}
    with open(sys.argv[1], encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            snap[r["rel"]] = r
    ep = os.path.join(sys.argv[2], "GenshinImpact_Data", "Persistent")
    manifests = [("res_versions_persist", "res"),
                 ("data_versions_persist", "data"),
                 ("silence_data_versions_persist", "silence")]
    for file, label in manifests:
        rs = rows(os.path.join(ep, file))
        ok = bad = missing = nohash = 0
        examples = []
        for r in rs:
            name = r["remoteName"]
            fd = folder(name)
            rel = f"GenshinImpact_Data/Persistent/{fd + '/' if fd else ''}{name}"
            rec = snap.get(rel)
            if rec is None:
                missing += 1
                continue
            if "md5" not in rec:
                nohash += 1
                continue
            if rec["md5"].lower() == r["md5"].lower() and rec["size"] == r["fileSize"]:
                ok += 1
            else:
                bad += 1
                if len(examples) < 5:
                    examples.append((rel, rec["md5"], rec["size"], r["md5"], r["fileSize"]))
        print(f"{file}: rows={len(rs)} md5+size OK={ok} MISMATCH={bad} "
              f"missing_from_Persistent={missing} not_hashed={nohash}")
        for e in examples:
            print("   MISMATCH", e)


if __name__ == "__main__":
    main()
