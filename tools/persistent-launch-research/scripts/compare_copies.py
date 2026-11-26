#!/usr/bin/env python3
"""Compare two snapshot.py JSONL inventories from two game copies.

Usage: python compare_copies.py A.jsonl B.jsonl
"""
import json
import sys


def load(p):
    d = {}
    with open(p, encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            d[r["rel"]] = r
    return d


def main():
    a, b = load(sys.argv[1]), load(sys.argv[2])
    extra_b = sorted(set(b) - set(a))
    extra_a = sorted(set(a) - set(b))
    print(f"=== ONLY IN B ({len(extra_b)}) ===")
    for k in extra_b:
        r = b[k]
        print(f"  {r['size']:>12} {r['attr']:>3} {k}")
    print(f"=== ONLY IN A ({len(extra_a)}) ===")
    for k in extra_a[:40]:
        r = a[k]
        print(f"  {r['size']:>12} {r['attr']:>3} {k}")
    if len(extra_a) > 40:
        print(f"  ... and {len(extra_a) - 40} more")
    print("=== COMMON WITH DIFFERENT MD5/SIZE ===")
    n = 0
    for k in sorted(set(a) & set(b)):
        x, y = a[k], b[k]
        if x.get("md5") != y.get("md5") or x["size"] != y["size"]:
            print(f"  {k}: A={x.get('md5')}/{x['size']} B={y.get('md5')}/{y['size']}")
            n += 1
    print(f"  total differing: {n}")


if __name__ == "__main__":
    main()
