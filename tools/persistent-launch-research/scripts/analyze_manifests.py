#!/usr/bin/env python3
"""Compare streaming manifests vs persisted manifests and report differences.

Usage: python analyze_manifests.py FRESH_GAME_DIR SYNCED_GAME_DIR
"""
import json
import os
import sys


def rows(path):
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                out.append(json.loads(line))
    return out


def summarize(name, rs):
    print(f"\n=== {name}: {len(rs)} rows ===")
    keys = set()
    for r in rs:
        keys.update(r.keys())
    print("keys:", sorted(keys))
    print("sample:", json.dumps(rs[0], ensure_ascii=False) if rs else None)


def main():
    fresh, synced = sys.argv[1], sys.argv[2]
    fsa = os.path.join(fresh, "GenshinImpact_Data", "StreamingAssets")
    ep = os.path.join(synced, "GenshinImpact_Data", "Persistent")
    esa = os.path.join(synced, "GenshinImpact_Data", "StreamingAssets")

    res_s = rows(os.path.join(fsa, "res_versions_streaming"))
    data_s = rows(os.path.join(fsa, "data_versions_streaming"))
    silence_s = rows(os.path.join(fsa, "silence_versions_streaming"))
    video_s = rows(os.path.join(fsa, "VideoAssets", "video_versions_streaming"))
    summarize("FRESH res_versions_streaming", res_s)
    summarize("FRESH data_versions_streaming", data_s)
    summarize("FRESH silence_versions_streaming", silence_s)
    summarize("FRESH video_versions_streaming", video_s)

    res_p = rows(os.path.join(ep, "res_versions_persist"))
    data_p = rows(os.path.join(ep, "data_versions_persist"))
    silence_p = rows(os.path.join(ep, "silence_data_versions_persist"))
    summarize("SYNCED res_versions_persist", res_p)
    summarize("SYNCED data_versions_persist", data_p)
    summarize("SYNCED silence_data_versions_persist", silence_p)

    # Compare data streaming vs persist (same byte length, different md5)
    a = {r.get("remoteName"): r for r in data_s}
    b = {r.get("remoteName"): r for r in data_p}
    diff = [k for k in a if a[k] != b.get(k)]
    print(f"\ndata: streaming-only={len(set(a)-set(b))} persist-only={len(set(b)-set(a))} value-diff={len(diff)}")
    for k in diff[:10]:
        print("  data diff", k, "\n    S", a[k], "\n    P", b.get(k))

    # Which persisted hotfix files actually exist on disk?
    exist = missing = 0
    for r in res_p + data_p + silence_p:
        name = r.get("remoteName", "")
        # apply the suffix -> folder mapping used by the game / Collapse
        if name.endswith(".pck"):
            folder = "AudioAssets"
        elif ("blocks" in name and name.endswith(".blk")) or name.endswith("svc_catalog"):
            folder = "AssetBundles"
        elif name.endswith(".usm") or name.endswith(".cuepoint"):
            folder = "VideoAssets"
        else:
            folder = ""
        rel = os.path.join(folder, name) if folder else name
        path = os.path.join(ep, rel.replace("/", os.sep))
        if os.path.exists(path):
            exist += 1
        else:
            missing += 1
    print(f"\npersist rows -> on disk: exist={exist} missing={missing}")


if __name__ == "__main__":
    main()
