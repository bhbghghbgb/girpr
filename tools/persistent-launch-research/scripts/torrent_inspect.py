#!/usr/bin/env python3
"""Parse a .torrent (bencode, stdlib only) and list its files with sizes.

Usage: python torrent_inspect.py FILE.torrent
"""
import sys


def bdecode(data, i=0):
    c = data[i:i + 1]
    if c == b"i":
        j = data.index(b"e", i)
        return int(data[i + 1:j]), j + 1
    if c == b"l":
        i += 1
        out = []
        while data[i:i + 1] != b"e":
            v, i = bdecode(data, i)
            out.append(v)
        return out, i + 1
    if c == b"d":
        i += 1
        out = {}
        while data[i:i + 1] != b"e":
            k, i = bdecode(data, i)
            v, i = bdecode(data, i)
            out[k] = v
        return out, i + 1
    j = data.index(b":", i)
    n = int(data[i:j])
    s = data[j + 1:j + 1 + n]
    return s, j + 1 + n


def main():
    data = open(sys.argv[1], "rb").read()
    meta, _ = bdecode(data)
    info = meta[b"info"]
    name = info[b"name"].decode("utf-8", "replace")
    print(f"torrent name: {name}")
    print(f"piece length: {info[b'piece length']}")
    print(f"pieces: {len(info[b'pieces']) // 20}")
    print(f"announce: {meta.get(b'announce', b'').decode('utf-8', 'replace')}")
    files = info.get(b"files")
    if files is None:
        print(f"single file: {name} ({info[b'length']} bytes)")
        return
    total = 0
    rows = []
    for f in files:
        path = "/".join(p.decode("utf-8", "replace") for p in f[b"path"])
        rows.append((path, f[b"length"]))
        total += f[b"length"]
    print(f"file count: {len(rows)}, total {total} bytes ({total / 2**30:.2f} GiB)")
    for path, length in rows:
        print(f"{length:>15}  {path}")


if __name__ == "__main__":
    main()
