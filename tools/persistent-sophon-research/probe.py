"""Live HYP + Sophon probes for hk4e_global.

Queries (all read-only GETs):
  * getGameConfigs / getGameBranches / getGameDeprecatedFileConfigs (HYP)
  * getBuild for every branch/package returned (Sophon)
Then walks all chunks manifests advertised by getBuild and checks whether any
manifest path overlaps the dispatcher's Persistent/hotfix names.

URL/body shapes follow ``app/fetch/src/hyp.rs`` and ``crates/model/src/region.rs``:
  HYP    https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/<endpoint>?launcher_id=...&language=en-us&game_ids[]=...
  Sophon https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild?branch=...&package_id=...&password=...
"""
from __future__ import annotations

import argparse
import gzip
import io
import json
import sys
import urllib.parse
from pathlib import Path

import zstandard

from common import SAMPLES, get, make_session, save_json

HYP = "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api"
SOPHON = "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api"
LAUNCHER_ID = "VYTpXlbWo8"  # hk4e_global
GAME_ID = "gopR6Cufr3"

HOTFIX_MARKERS = (
    "persistent", "hotfix", "external", "patch", "ctable", "blocks/", ".blk",
    ".pck", ".usm", ".cuepoint", "svc_catalog", "res_versions",
)


def hyp_url(endpoint: str, with_game: bool = True, channel: bool = False) -> str:
    q = {"launcher_id": LAUNCHER_ID, "language": "en-us"}
    if with_game:
        q["game_ids[]"] = GAME_ID
        # requests will percent-encode [] properly.
    url = f"{HYP}/{endpoint}?{urllib.parse.urlencode(q, safe='[]')}"
    if channel:
        url += "&channel=1&sub_channel=0"
    return url


def get_json(session, url: str, label: str) -> dict | None:
    r = get(session, url, timeout=60)
    ctype = r.headers.get("content-type")
    print(f"[{label}] HTTP {r.status_code} {ctype} {len(r.content)}B {url}")
    if r.status_code != 200:
        print(f"         {r.text[:300]}")
        return None
    try:
        return r.json()
    except Exception:
        print(f"         non-JSON: {r.text[:300]}")
        return None


def build_url(pkg: dict, tag: str | None = None) -> str:
    q = urllib.parse.urlencode(
        {
            "branch": pkg.get("branch", "main"),
            "package_id": pkg.get("package_id", ""),
            "password": pkg.get("password", ""),
        }
    )
    url = f"{SOPHON}/getBuild?{q}"
    if tag:
        url += "&tag=" + urllib.parse.quote(tag)
    return url


def decode_manifest(url: str, content: bytes, size: int) -> bytes:
    """Return decompressed manifest protobuf bytes (sizes from getBuild)."""
    if content[:2] == b"\x1f\x8b":
        return gzip.decompress(content)
    if content[:4] == b"\x28\xb5\x2f\xfd":
        return zstandard.ZstdDecompressor().decompress(content)
    return content


def parse_sophon_files(proto: bytes) -> list[dict]:
    """Minimal wire parser for SophonChunkManifest (field 1 = repeated file)."""
    from common import parse_fields

    root = parse_fields(proto)
    files = []
    for wt, buf in root.get(1, []):
        if wt != 2:
            continue
        f = parse_fields(buf)
        def s(n):
            v = f.get(n, [(0, b"")])[0][1]
            return v.decode("utf-8", "replace") if isinstance(v, bytes) else str(v)
        def i(n):
            v = f.get(n, [(0, 0)])[0][1]
            return v if isinstance(v, int) else 0
        files.append({"file": s(1), "size": i(4), "md5": s(5), "chunks": len(f.get(2, []))})
    return files


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--download-manifests", action="store_true",
                    help="download and parse every advertised Sophon manifest")
    ap.add_argument("--max-manifests", type=int, default=12)
    args = ap.parse_args()

    s = make_session()
    out: dict = {"queried_at_utc": __import__("datetime").datetime.now(__import__("datetime").timezone.utc).isoformat()}

    for endpoint, key, with_game, channel in (
        ("getGameConfigs", "configs", True, False),
        ("getGameBranches", "branches", True, False),
        ("getGameDeprecatedFileConfigs", "deprecated", True, True),
    ):
        url = hyp_url(endpoint, with_game, channel)
        data = get_json(s, url, endpoint)
        out[key] = {"url": url, "data": data}
        if data is not None:
            save_json(f"hyp_{endpoint}.json", data)

    branches = (out["branches"]["data"] or {}).get("data", {}).get("game_branches", [])
    out["branch_packages"] = branches
    print(f"\n[branches] {len(branches)} package(s)")
    for b in branches:
        print("  ", {k: b.get(k) for k in ("main", "pre_download")})

    builds = []
    if args.download_manifests:
        for b in branches:
            pkg = b.get("main") or {}
            if not pkg:
                continue
            for tag in (None,):  # latest only; local/older build probed separately
                url = build_url(pkg, tag)
                data = get_json(s, url, f"getBuild[{pkg.get('tag')}]")
                if not data or data.get("retcode") != 0:
                    continue
                d = data.get("data", {})
                rec = {"url": url, "branch": pkg.get("branch"), "tag": pkg.get("tag"),
                       "build_id": d.get("build_id"), "package_id": pkg.get("package_id")}
                rec["fields"] = sorted(d.keys())
                # Recursively find manifest descriptors.
                manifests = []
                def walk(o, path=""):
                    if isinstance(o, dict):
                        if "manifest" in o and isinstance(o["manifest"], dict):
                            m = o["manifest"]
                            manifests.append({
                                "path": path,
                                "id": m.get("id"),
                                "checksum": m.get("checksum"),
                                "uncompressed_size": m.get("uncompressed_size"),
                                "compressed_size": m.get("compressed_size"),
                                "url_prefix": (o.get("manifest_download") or {}).get("url_prefix"),
                                "matching_field": o.get("matching_field"),
                            })
                        for k, v in o.items():
                            walk(v, f"{path}.{k}")
                    elif isinstance(o, list):
                        for i, v in enumerate(o):
                            walk(v, f"{path}[{i}]")
                walk(d)
                rec["manifests"] = manifests
                builds.append(rec)
                print(f"  [getBuild] tag={pkg.get('tag')} build={d.get('build_id')} manifests={len(manifests)}")
                for m in manifests[:20]:
                    print("     ", m.get("matching_field"), m.get("id"), (m.get("url_prefix") or "")[:80])
        out["builds"] = builds

        # Cross-match manifest paths against the dispatcher's persistent names.
        names = []
        p = SAMPLES / "res_versions_external.names.tsv"
        if p.exists():
            names = [ln.split("\t")[0] for ln in p.read_text(encoding="utf-8").splitlines() if ln]
        name_set = set(n.replace("\\", "/").lower() for n in names)
        hotfix_hits = []
        path_samples = []
        count = 0
        for b in builds:
            for m in b.get("manifests", []):
                if count >= args.max_manifests:
                    break
                prefix, mid = m.get("url_prefix"), m.get("id")
                if not prefix or not mid:
                    continue
                url = prefix.rstrip("/") + "/" + mid
                count += 1
                r = get(s, url, timeout=90)
                print(f"[manifest] HTTP {r.status_code} {r.headers.get('content-type')} {len(r.content)}B {m.get('matching_field')} {url[:120]}")
                if r.status_code != 200:
                    continue
                try:
                    proto = decode_manifest(url, r.content, m.get("uncompressed_size") or 0)
                except Exception as e:
                    print(f"           decode failed: {e}")
                    continue
                files = parse_sophon_files(proto)
                samples = [f for f in files[:15]]
                path_samples.append({"manifest_id": mid, "field": m.get("matching_field"),
                                     "file_count": len(files), "samples": samples})
                hits = []
                for f in files:
                    fp = f["file"].replace("\\", "/").lower()
                    if fp in name_set or any(mk in fp for mk in HOTFIX_MARKERS):
                        hits.append(f)
                if hits:
                    hotfix_hits.append({"manifest_id": mid, "field": m.get("matching_field"),
                                        "hits": hits[:100]})
                    print(f"           *** {len(hits)} hotfix/persistent-like paths, e.g. {[h['file'] for h in hits[:5]]}")
        out["manifest_path_samples"] = path_samples
        out["hotfix_hits"] = hotfix_hits
        save_json("sophon_probe_summary.json", out)

    save_json("probe_summary.json", out)
    print("\n[done]")
    return 0


if __name__ == "__main__":
    sys.exit(main())
