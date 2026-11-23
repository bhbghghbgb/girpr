"""Probe the Sophon *patch* endpoint (``getPatchBuild``) for Persistent/hotfix data.

``getPatchBuild`` is a POST (Starward: HoYoPlayClient.cs:594-612). It advertises
version-to-version patch manifests, separate from the ``getBuild`` chunk
manifests. We enumerate every advertised patch manifest and search its raw
decompressed bytes for:

  * the literal strings ``Persistent`` / ``hotfix`` / ``external``;
  * any of the 376 dispatcher hotfix MD5 hex strings (bytes absent from every
    ``getBuild`` manifest).

A hit would be direct evidence the server ships hotfix bytes via Sophon.
"""
from __future__ import annotations

import gzip
import json
import sys

import zstandard

from common import SAMPLES, get, make_session, save_json

PATCH_URL = (
    "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getPatchBuild"
    "?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x"
)


def decompress(body: bytes) -> bytes:
    if body[:2] == b"\x1f\x8b":
        return gzip.decompress(body)
    if body[:4] == b"\x28\xb5\x2f\xfd":
        return zstandard.ZstdDecompressor().decompress(body)
    return body


def main() -> int:
    s = make_session()
    hc = json.loads((SAMPLES / "hotfix_check_summary.json").read_text(encoding="utf-8"))
    hotfix_md5s = [r["md5"].encode() for r in hc["md5_absent_from_all_sophon_builds"]]
    print(f"[hotfix md5 pool] {len(hotfix_md5s)}")

    r = s.post(PATCH_URL, timeout=60)
    print(f"[getPatchBuild] HTTP {r.status_code} {r.headers.get('content-type')} {len(r.content)}B")
    r.raise_for_status()
    data = r.json()["data"]
    rec = {"url": PATCH_URL, "tag": data.get("tag"), "build_id": data.get("build_id"),
           "patch_id": data.get("patch_id"), "manifests": [], "raw_head": r.text[:400]}
    for m in data.get("manifests", []):
        mid = m["manifest"]["id"]
        purl = m["manifest_download"]["url_prefix"].rstrip("/") + "/" + mid
        r = get(s, purl, timeout=120)
        entry = {"field": m.get("matching_field"), "id": mid, "url": purl,
                 "status": r.status_code, "content_type": r.headers.get("content-type"),
                 "compressed_bytes": len(r.content)}
        if r.status_code == 200:
            body = decompress(r.content)
            entry["decompressed_bytes"] = len(body)
            low = body.lower()
            entry["string_hits"] = {k: low.count(k.encode()) for k in
                                     ("persistent", "hotfix", "external", "ctable", "blocks/")}
            hits = [md.decode() for md in hotfix_md5s if md in low]
            entry["hotfix_md5_hits"] = hits[:30]
            entry["hotfix_md5_hit_count"] = len(hits)
        rec["manifests"].append(entry)
        print(f"[patch manifest] {entry['field']} HTTP {entry['status']} "
              f"{entry.get('decompressed_bytes')}B "
              f"strings={entry.get('string_hits')} hotfix_md5_hits={entry.get('hotfix_md5_hit_count')}")
    save_json("patch_probe_summary.json", rec)
    return 0


if __name__ == "__main__":
    sys.exit(main())
