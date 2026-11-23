"""Live CDN probes for Sophon-vs-HTTP addressing under dispatcher-derived paths.

Tests, with exact HTTP status/content-type recorded:

  1. a REAL chunk id from the live 7.2.0 ``game`` Sophon manifest, at its
     advertised ``chunk_download.url_prefix`` (control: should be 200);
  2. the same chunk id under dispatcher-derived CDN paths (``output_<rev>_<hash>``)
     — are Sophon chunks also served there?
  3. a manifest id under the dispatcher-derived path instead of
     ``client_app/sophon/manifests/...``;
  4. the plain persistent hotfix file URL, with and without Sophon-style query
     params, and an HTTP Range request (does the full-file path support ranges?).

Nothing here needs auth; every request is a read-only GET/HEAD.
"""
from __future__ import annotations

import gzip
import json
import sys
import urllib.parse

import zstandard

from common import SAMPLES, get, make_session, parse_fields, save_json


def read_varint(buf, pos):
    result = shift = 0
    while pos < len(buf):
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if b < 0x80:
            return result, pos
        shift += 7
    raise ValueError


def first_str(fields, n):
    v = fields.get(n, [(0, b"")])[0][1]
    return v.decode("utf-8", "replace") if isinstance(v, bytes) else str(v)


def parse_files_with_chunks(proto: bytes) -> list[dict]:
    root = parse_fields(proto)
    out = []
    for wt, buf in root.get(1, []):
        if wt != 2:
            continue
        f = parse_fields(buf)
        chunks = []
        for cwt, cbuf in f.get(2, []):
            if cwt != 2:
                continue
            c = parse_fields(cbuf)
            chunks.append({
                "id": first_str(c, 1),
                "uncompressed_md5": first_str(c, 2),
                "offset": c.get(3, [(0, 0)])[0][1],
                "compressed_size": c.get(4, [(0, 0)])[0][1],
                "uncompressed_size": c.get(5, [(0, 0)])[0][1],
            })
        out.append({"file": first_str(f, 1), "size": f.get(4, [(0, 0)])[0][1],
                    "md5": first_str(f, 5), "chunks": chunks})
    return out


def probe(s, url: str, label: str, headers=None, method="GET", save_body=None) -> dict:
    try:
        r = get(s, url, timeout=40, headers=headers or {}) if method == "GET" else s.head(url, timeout=40)
    except Exception as e:
        print(f"[{label}] TRANSPORT ERROR {e}")
        return {"label": label, "url": url, "error": str(e)}
    rec = {"label": label, "url": url, "method": method,
           "status": r.status_code, "content_type": r.headers.get("content-type"),
           "content_length": r.headers.get("content-length"),
           "accept_ranges": r.headers.get("accept-ranges"),
           "body_bytes": len(r.content)}
    print(f"[{label}] HTTP {r.status_code} {rec['content_type']} len={rec['content_length']} "
          f"ranges={rec['accept_ranges']} body={len(r.content)}B")
    if save_body and r.status_code == 200:
        (SAMPLES / save_body).write_bytes(r.content)
    if r.status_code != 200 and r.content:
        rec["body_head"] = r.text[:200]
    return rec


def main() -> int:
    s = make_session()
    d = json.loads((SAMPLES / "dispatch_summary.json").read_text(encoding="utf-8"))
    derived = d["derived_urls"]
    probe_data = json.loads((SAMPLES / "probe_summary.json").read_text(encoding="utf-8"))
    m = probe_data["builds"][0]["manifests"][0]
    manifest_url = m["url_prefix"] + "/" + m["id"]

    r = get(s, manifest_url, timeout=120)
    body = r.content
    if body[:2] == b"\x1f\x8b":
        body = gzip.decompress(body)
    elif body[:4] == b"\x28\xb5\x2f\xfd":
        body = zstandard.ZstdDecompressor().decompress(body)
    files = parse_files_with_chunks(body)
    with_chunks = [f for f in files if f["chunks"]]
    f0 = with_chunks[0]
    c0 = f0["chunks"][0]
    print(f"[control file] {f0['file']} md5={f0['md5']} chunks={len(f0['chunks'])}")
    print(f"[control chunk] id={c0['id']} offset={c0['offset']} csize={c0['compressed_size']}")

    # Real chunk prefix from getBuild.
    build = get(s, probe_data["builds"][0]["url"], timeout=60).json()["data"]
    entry = build["manifests"][0]
    chunk_prefix = entry["chunk_download"]["url_prefix"]
    chunk_url = f"{chunk_prefix}/{c0['id']}"

    results = [probe(s, chunk_url, "control: real chunk at advertised prefix")]
    game_base = derived["client_game_res_url"]

    guesses = [
        (f"{game_base}/sophon/chunks/{c0['id']}", "chunk under dispatcher game_res base"),
        (f"{game_base}/StandaloneWindows64/sophon/chunks/{c0['id']}", "chunk under StandaloneWindows64"),
        (f"{game_base}/client_app/sophon/chunks/{c0['id']}", "chunk under client_app path"),
        (f"{game_base}/sophon/manifests/{m['id']}", "manifest under dispatcher game_res base"),
        (f"{game_base}/StandaloneWindows64/{m['id']}", "manifest under StandaloneWindows64"),
    ]
    for url, label in guesses:
        results.append(probe(s, url, label))

    # Pick a genuine hotfix file from the cross-match (md5 absent from all Sophon).
    hc = json.loads((SAMPLES / "hotfix_check_summary.json").read_text(encoding="utf-8"))
    hotfix = hc["md5_absent_from_all_sophon_builds"][0]
    print(f"[hotfix] {hotfix['remoteName']} md5={hotfix['md5']} size={hotfix['fileSize']}")
    # Mirror Collapse's GetRelativePathByRemoteName (Fetch.Persistent.cs:239-278):
    # blocks*.blk -> AssetBundles/, .pck -> AudioAssets/, .usm -> VideoAssets/.
    rn = hotfix["remoteName"]
    if rn.startswith("blocks") or rn.endswith("svc_catalog"):
        rel = "AssetBundles"
    elif rn.lower().endswith(".pck"):
        rel = "AudioAssets"
    elif rn.lower().endswith((".usm", ".cuepoint")):
        rel = "VideoAssets"
    else:
        rel = ""
    hot_url = f"{derived['client_game_res_url']}/StandaloneWindows64/{rel}/{rn}".replace("//", "/").replace("https:/", "https://")
    results.append(probe(s, hot_url, "hotfix plain full-file URL (correct AssetBundles path)"))
    # Verify the bytes are the exact hotfix, and that no Sophon-style param helps.
    import hashlib
    r = get(s, hot_url, timeout=60)
    if r.status_code == 200:
        print(f"[hotfix verify] md5={hashlib.md5(r.content).hexdigest()} "
              f"expected={hotfix['md5']} bytes={len(r.content)}")
    results.append(probe(s, hot_url + "?chunk=1", "hotfix URL + ?chunk=1"))
    results.append(probe(s, hot_url + "?sophon=1", "hotfix URL + ?sophon=1"))
    results.append(probe(s, hot_url, "hotfix Range 0-15", headers={"Range": "bytes=0-15"}))

    save_json("cdn_probe_summary.json", {"control": {
        "file": f0["file"], "chunk": c0, "chunk_prefix": chunk_prefix,
        "manifest_url": manifest_url,
    }, "results": results, "guessed_hotfix": {**hotfix, "url": hot_url, "relative_dir": rel}})
    return 0


if __name__ == "__main__":
    sys.exit(main())
