"""Fetch and unwrap Collapse release-repo metadata (live).

Source: CollapseLauncher-ReleaseRepo ``metadata/v3/stable/config_master.json``
and ``config_GIGlb.json``. Unwrapping follows
``CollapseLauncher/Classes/Helper/Metadata/DataCooker.cs:41-83,94-185``.

The 2026 metadata shape is:

* ``config_master.json`` — public fields ``Key`` (base64 Collapse-magic blob
  holding a 1024-bit PKCS#1 DER private key) and ``BitSize`` (128).
* ``config_GIGlb.json`` — plain JSON. Sensitive-ish fields are base64
  Collapse-magic blobs (flags ``0x101`` = Brotli + encrypted), RSA-encrypted
  with the master key in ``BitSize``-byte blocks, then Brotli-compressed.

Outputs (in ``samples/``):
  * ``metadata_summary.json`` — redacted: dispatcher URL templates,
    ProtoDispatchKey seed, key fingerprints/sizes. Master RSA XML is NOT stored.
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from datetime import datetime, timezone

from common import get, make_session, save_json, sha256_hex
from keys import metadata_key, unwrap_magic

BASE = (
    "https://raw.githubusercontent.com/CollapseLauncher/"
    "CollapseLauncher-ReleaseRepo/main/metadata/v3/stable"
)
MASTER_URL = f"{BASE}/config_master.json"
GIGLB_URL = f"{BASE}/config_GIGlb.json"

# Fields worth keeping in the redacted summary (all public launcher metadata).
SUMMARY_FIELDS = [
    "ProfileName",
    "GameChannel",
    "ZoneName",
    "GameType",
    "GameExecutableName",
    "GameDispatchURL",
    "ProtoDispatchKey",
    "DispatcherKeyBitLength",
    "DispatcherKey",
    "ChannelID",
    "SubChannelID",
    "LauncherGetGameURL",
    "LauncherResourceURL",
    "LauncherResourceChunksURL",
    "LauncherType",
]


def b64decode(s: str) -> bytes:
    return base64.b64decode(s + "=" * (-len(s) % 4))


def unwrap_field(value: str, key, bit_size: int) -> str:
    return unwrap_magic(b64decode(value), key, bit_size).decode("utf-8")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--offline-summary", help="print an existing summary instead of fetching")
    args = ap.parse_args()
    if args.offline_summary:
        print(open(args.offline_summary, encoding="utf-8").read())
        return 0

    s = make_session()
    mr = get(s, MASTER_URL)
    gr = get(s, GIGLB_URL)
    print(f"[master] {mr.status_code} {mr.headers.get('content-type')} {len(mr.content)}B {MASTER_URL}")
    print(f"[giglb ] {gr.status_code} {gr.headers.get('content-type')} {len(gr.content)}B {GIGLB_URL}")
    mr.raise_for_status()
    gr.raise_for_status()
    master = mr.json()
    meta = gr.json()

    bit_size = int(master["BitSize"])
    master_key = metadata_key(master)
    raw_key = master["Key"]
    key_info = {
        "bit_size_bytes": bit_size,
        "master_public_size_bits": master_key.size_in_bits(),
        "master_key_blob_sha256": sha256_hex(
            b64decode(raw_key) if isinstance(raw_key, str) else bytes(raw_key)
        ),
        "master_key_blob_len": len(b64decode(raw_key)),
    }

    unwrapped: dict[str, str] = {}
    for name in SUMMARY_FIELDS:
        value = meta.get(name)
        if not isinstance(value, str):
            continue
        if value.startswith("Q29sbGFwc2U"):  # base64("Collapse")
            unwrapped[name] = unwrap_field(value, master_key, bit_size)
        else:
            unwrapped[name] = value

    # Confirm the live DispatcherKey equals the GenshinSlimmer copy.
    local_xml = open(
        __import__("pathlib").Path(__file__).resolve().parent / "dispatch_rsa.xml",
        encoding="utf-8",
    ).read()
    key_match = unwrapped.get("DispatcherKey", "").strip() == local_xml.strip()

    # Redact the dispatcher private key: store only its SHA-256 and length.
    dispatcher_xml = unwrapped.get("DispatcherKey")
    if dispatcher_xml:
        unwrapped["DispatcherKey"] = (
            f"<redacted: {len(dispatcher_xml)} chars, sha256={sha256_hex(dispatcher_xml.encode())}>"
        )

    summary = {
        "fetched_at_utc": datetime.now(timezone.utc).isoformat(),
        "sources": [MASTER_URL, GIGLB_URL],
        "master": key_info,
        "live_dispatcher_key_matches_genshinslimmer_copy": key_match,
        "fields": unwrapped,
        "all_field_names": sorted(meta.keys()),
    }
    p = save_json("metadata_summary.json", summary)
    print(f"[saved ] {p}")
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
