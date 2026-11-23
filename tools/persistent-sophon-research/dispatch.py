"""Query the LIVE Genshin dispatcher (``query_cur_region``) and decode it.

Reuses seed/URL template from ``samples/metadata_summary.json`` (produced by
``fetch_metadata.py``) and the public RSA key in ``dispatch_rsa.xml``.

Produces in ``samples/``:
  * ``dispatch_summary.json`` — HTTP facts, field numbers/values (all fields,
    including ones the local .proto omits), derived CDN URLs, manifest stats.
  * ``dispatch_fields.txt`` — full generic protobuf field dump (root + nested).
  * ``res_versions_external.head.ndjson`` / ``data_versions.head.ndjson`` /
    ``silence_versions.head.ndjson`` — first 40 lines only (NOT full blobs).
  * ``persistent_paths.sample.txt`` — first 300 ``remoteName`` values with
    their md5/hash, for cross-matching against Sophon manifests.
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

from Crypto.Cipher import PKCS1_v1_5

from common import (
    SAMPLES,
    as_str,
    dump_fields,
    first,
    get,
    make_session,
    parse_fields,
    save_json,
    sha256_hex,
)
from keys import dispatcher_key

DEFAULT_URL_TEMPLATE = (
    "https://{0}.yuanshen.com/query_cur_region?version={1}"
    "&lang=1&platform=3&binary=1&time=260&channel_id=1&sub_channel_id=1"
    "&account_type=1&dispatchSeed={2}&key_id=5"
)
HOSTS = ["osusadispatch", "oseurodispatch", "osasiadispatch"]
VERSIONS = ["OSRELWin7.2.0"]


def load_metadata() -> tuple[str, str]:
    p = SAMPLES / "metadata_summary.json"
    if p.exists():
        fields = json.loads(p.read_text(encoding="utf-8"))["fields"]
        return fields.get("GameDispatchURL", DEFAULT_URL_TEMPLATE), fields.get(
            "ProtoDispatchKey", "018bc9560abc707f"
        )
    return DEFAULT_URL_TEMPLATE, "018bc9560abc707f"


def decrypt_content(content_b64: str, key) -> bytes:
    enc = base64.b64decode(content_b64 + "=" * (-len(content_b64) % 4))
    if len(enc) % 256:
        raise ValueError(f"ciphertext {len(enc)}B is not 256-byte aligned")
    cipher = PKCS1_v1_5.new(key)
    out = bytearray()
    for off in range(0, len(enc), 256):
        pt = cipher.decrypt(enc[off : off + 256], None)
        if pt is None:
            raise ValueError(f"RSA block {off // 256} failed")
        out += pt
    return bytes(out)


def parse_manifest(text: str | None, name: str) -> dict:
    if not text:
        return {"present": False, "lines": 0, "sample": []}
    lines = [ln for ln in text.splitlines() if ln.strip()]
    sample = []
    for ln in lines[:40]:
        try:
            obj = json.loads(ln)
        except json.JSONDecodeError:
            sample.append({"raw": ln[:300]})
            continue
        if isinstance(obj, dict):
            sample.append(
                {
                    k: obj.get(k)
                    for k in ("remoteName", "md5", "hash", "fileSize", "isPatch", "path")
                    if k in obj
                }
            )
        else:
            sample.append({"raw": str(obj)[:300]})
    (SAMPLES / f"{name}.head.ndjson").write_text("\n".join(lines[:40]) + "\n", encoding="utf-8")
    return {
        "present": True,
        "lines": len(lines),
        "sample": sample,
        "sha256_of_full_text": sha256_hex(text.encode("utf-8")),
        "keys_seen": sorted({k for ln in lines[:200] for k in _keys(ln)}),
    }


def _keys(line: str) -> list[str]:
    try:
        o = json.loads(line)
        return list(o.keys()) if isinstance(o, dict) else []
    except Exception:
        return []


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default=None, help="e.g. OSRELWin7.2.0")
    ap.add_argument("--host", default=None, help="e.g. osusadispatch")
    ap.add_argument("--seed", default=None)
    args = ap.parse_args()

    tmpl, seed = load_metadata()
    seed = args.seed or seed
    versions = [args.version] if args.version else VERSIONS
    hosts = [args.host] if args.host else HOSTS
    key = dispatcher_key()

    resp_data = None
    used = {}
    s = make_session()
    for ver in versions:
        for host in hosts:
            url = tmpl.format(host, ver, seed)
            try:
                r = get(s, url, timeout=15)
            except Exception as e:
                print(f"[query] {host} {ver}: transport error {e}")
                continue
            ctype = r.headers.get("content-type")
            print(f"[query] {host} {ver}: HTTP {r.status_code} {ctype} {len(r.content)}B")
            if r.status_code != 200:
                continue
            try:
                j = r.json()
            except Exception:
                print(f"         non-JSON body: {r.text[:200]!r}")
                continue
            if set(j.keys()) != {"content", "sign"} or not j.get("content"):
                print(f"         unexpected JSON keys {sorted(j.keys())}")
                continue
            enc = base64.b64decode(j["content"] + "=" * (-len(j["content"]) % 4))
            if len(enc) < 512 or len(enc) % 256:
                print(f"         unusable ciphertext ({len(enc)}B) — stale seed?")
                continue
            resp_data = (r, j, url)
            used = {"host": host, "version": ver, "seed": seed, "url": url}
            break
        if resp_data:
            break

    if resp_data is None:
        print("No usable dispatcher response. See README troubleshooting.")
        return 2
    r, j, url = resp_data
    print(f"[decrypt] {len(base64.b64decode(j['content']))}B ciphertext")
    proto = decrypt_content(j["content"], key)
    print(f"[decrypt] {len(proto)}B protobuf, sha256={sha256_hex(proto)[:16]}")

    root = parse_fields(proto)
    dump = ["=== ROOT (all fields) ===", dump_fields(root), ""]
    props_buf = first(root, 3)
    props = parse_fields(props_buf) if props_buf else {}
    dump += ["=== FIELD 3 GatewayProperties (all fields) ===", dump_fields(props), ""]
    rv_buf = first(props, 22)
    rv = parse_fields(rv_buf) if rv_buf else {}
    dump += ["=== FIELD 22 RepoResVersionProperties (all fields) ===", dump_fields(rv), ""]
    (SAMPLES / "dispatch_fields.txt").write_text("\n".join(dump), encoding="utf-8")

    def pstr(fields, n):
        return as_str(first(fields, n))

    def pint(fields, n):
        v = first(fields, n)
        return v if isinstance(v, int) else None

    res_num = pint(rv, 1)
    res_hash = pstr(rv, 5)
    data_num = pint(props, 14)
    data_hash = pstr(props, 26)
    sil_num = pint(props, 18)
    sil_hash = pstr(props, 27)
    res_base = pstr(props, 8)
    data_base = pstr(props, 9)

    res_manifest = parse_manifest(pstr(rv, 3), "res_versions_external")
    data_manifest = parse_manifest(pstr(props, 19), "data_versions")
    sil_manifest = parse_manifest(pstr(props, 20), "silence_data_versions")

    derived = {}
    if res_base and res_num and res_hash:
        base = f"{res_base.rstrip('/')}/output_{res_num}_{res_hash}/client"
        derived["client_game_res_url"] = base
        derived["res_versions_external_url"] = f"{base}/StandaloneWindows64/res_versions_external"
        derived["base_revision_url"] = f"{base}/StandaloneWindows64/base_revision"
    if data_base and data_num and data_hash:
        base = f"{data_base.rstrip('/')}/output_{data_num}_{data_hash}/client/General"
        derived["client_design_data_url"] = base
        derived["data_versions_url"] = f"{base}/AssetBundles/data_versions"

    # Save a trim set of hotfix-looking remoteNames for cross-matching later.
    persistent_paths = []
    for manifest in (res_manifest, data_manifest, sil_manifest):
        for entry in manifest.get("sample", []):
            if "remoteName" in entry:
                persistent_paths.append(entry)
    (SAMPLES / "persistent_paths.sample.txt").write_text(
        "\n".join(
            f"{e.get('remoteName')}\t{e.get('md5')}\t{e.get('hash')}" for e in persistent_paths
        ),
        encoding="utf-8",
    )

    summary = {
        "queried_at_utc": datetime.now(timezone.utc).isoformat(),
        "used": used,
        "http": {
            "status": r.status_code,
            "content_type": r.headers.get("content-type"),
            "response_bytes": len(r.content),
            "content_b64_bytes": len(j["content"]),
            "sign_b64_bytes": len(j.get("sign", "")),
            "json_keys": sorted(j.keys()),
        },
        "protobuf": {
            "plaintext_bytes": len(proto),
            "plaintext_sha256": sha256_hex(proto),
            "root_field_numbers": sorted(root),
            "properties_field_numbers": sorted(props),
            "resver_field_numbers": sorted(rv),
            "root_unknown_11_12": [first(root, 11).hex()[:200] if first(root, 11) else None,
                                   first(root, 12).hex()[:200] if first(root, 12) else None],
            "properties_38_47": {str(n): (as_str(first(props, n)) or
                                          (first(props, n).hex()[:200] if isinstance(first(props, n), bytes) else first(props, n)))
                                 for n in range(38, 48) if n in props},
        },
        "values": {
            "res_revision": res_num,
            "res_hash": res_hash,
            "data_revision": data_num,
            "data_hash": data_hash,
            "silence_revision": sil_num,
            "silence_hash": sil_hash,
            "repo_res_version_url": res_base,
            "repo_design_data_url": data_base,
            "game_version_from_props": pstr(props, 12),
            "server_name": pstr(props, 7),
        },
        "derived_urls": derived,
        "manifests": {
            "res_versions_external": res_manifest,
            "data_versions": data_manifest,
            "silence_data_versions": sil_manifest,
        },
    }
    p = save_json("dispatch_summary.json", summary)
    print(f"[saved ] {p}")
    print(json.dumps(summary["values"], indent=2))
    print(json.dumps(derived, indent=2))
    print(json.dumps({k: v["lines"] for k, v in summary["manifests"].items()}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
