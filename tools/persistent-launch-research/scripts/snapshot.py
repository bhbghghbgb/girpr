#!/usr/bin/env python3
"""Recursively snapshot a directory tree (sequential IO, one disk at a time).

Writes one JSON object with metadata for every file: rel path (POSIX separators),
size, mtime (unix seconds), DOS attributes, optional MD5 for files up to
--hash-max-size bytes. Hashing is sequential. A full-tree hash of a 144 GB
game is not done by default; use --hash-max-size large only when needed.

Usage:
  python snapshot.py ROOT OUT.jsonl [--hash-max-size BYTES] [--only-hash PATH ...]
"""
import argparse
import hashlib
import json
import os
import stat
import sys


def attr_string(st):
    a = st.st_file_attributes if hasattr(st, "st_file_attributes") else 0
    names = []
    for bit, name in ((0x1, "R"), (0x2, "H"), (0x4, "S"), (0x20, "A")):
        if a & bit:
            names.append(name)
    return "".join(names) or "-"


def md5_of(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(4 << 20)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("out")
    ap.add_argument("--hash-max-size", type=int, default=0,
                    help="hash files with size <= N bytes (0 = no hashing)")
    ap.add_argument("--only-hash", nargs="*", default=[],
                    help="extra rel paths to hash regardless of size")
    args = ap.parse_args()

    root = os.path.abspath(args.root)
    only = {p.replace("\\", "/").lower() for p in args.only_hash}
    count = 0
    total = 0
    with open(args.out, "w", encoding="utf-8", newline="\n") as out:
        for dirpath, _dirnames, filenames in os.walk(root):
            for name in filenames:
                full = os.path.join(dirpath, name)
                rel = os.path.relpath(full, root).replace("\\", "/")
                try:
                    st = os.stat(full, follow_symlinks=False)
                except OSError as e:
                    print(f"stat failed: {rel}: {e}", file=sys.stderr)
                    continue
                rec = {
                    "rel": rel,
                    "size": st.st_size,
                    "mtime": int(st.st_mtime),
                    "attr": attr_string(st),
                }
                if (0 < st.st_size <= args.hash_max_size) or (rel.lower() in only):
                    try:
                        rec["md5"] = md5_of(full)
                    except OSError as e:
                        rec["md5_error"] = str(e)
                out.write(json.dumps(rec, separators=(",", ":")) + "\n")
                count += 1
                total += st.st_size
    print(f"snapshot: {count} files, {total} bytes -> {args.out}")


if __name__ == "__main__":
    main()
