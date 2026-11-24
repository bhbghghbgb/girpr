#!/usr/bin/env python3
"""Compare two snapshot.py JSONL files and report added/removed/changed files.

Usage: python diff_snap.py BEFORE.jsonl AFTER.jsonl
"""
import json
import sys


def load(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            out[r["rel"]] = r
    return out


def main():
    before, after = load(sys.argv[1]), load(sys.argv[2])
    added = sorted(set(after) - set(before))
    removed = sorted(set(before) - set(after))
    changed = []
    for rel in sorted(set(before) & set(after)):
        a, b = before[rel], after[rel]
        diffs = [k for k in ("size", "mtime", "attr", "md5")
                 if a.get(k) != b.get(k)]
        if diffs:
            changed.append((rel, diffs, a, b))

    print(f"=== ADDED ({len(added)}) ===")
    for rel in added:
        r = after[rel]
        print(f"+ {rel}\t{r['size']}\t{r['attr']}")
    print(f"=== REMOVED ({len(removed)}) ===")
    for rel in removed:
        r = before[rel]
        print(f"- {rel}\t{r['size']}\t{r['attr']}")
    print(f"=== CHANGED ({len(changed)}) ===")
    for rel, diffs, a, b in changed:
        detail = ", ".join(f"{k}: {a.get(k)!r} -> {b.get(k)!r}" for k in diffs)
        print(f"* {rel}\t{detail}")


if __name__ == "__main__":
    main()
