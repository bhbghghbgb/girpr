"""Fetch and unwrap Collapse release-repo metadata (live).

Source: CollapseLauncher-ReleaseRepo ``metadata/v3/stable/config_master.json``
and the per-game preset configs under the same directory. Unwrapping follows
``CollapseLauncher/Classes/Helper/Metadata/DataCooker.cs:41-83,94-185``.

The 2026 metadata shape is:

* ``config_master.json`` — public fields ``Key`` (base64 Collapse-magic blob
  holding a 1024-bit PKCS#1 DER private key) and ``BitSize`` (128).
* the preset config — plain JSON. Sensitive-ish fields are base64
  Collapse-magic blobs (flags ``0x101`` = Brotli + encrypted), RSA-encrypted
  with the master key in ``BitSize``-byte blocks, then Brotli-compressed.

The per-biz config file is **resolved from the release/preset table**
(``metadata/v3/stable/stamp.json``), not guessed. The table maps
``(GameName="Genshin Impact", GameRegion)`` to ``MetadataPath``:

  hk4e_global     -> Global         -> config_GIGlb.json
  hk4e_cn         -> Mainland China -> config_GICN.json
  hk4e_bilibili   -> Bilibili       -> config_GICNBilibili.json

Outputs (in ``samples/``):
  * ``metadata_summary.json`` — redacted: per-biz dispatcher URL templates,
    ProtoDispatchKey seed, DispatcherKey fingerprint, region id, key sizes.
    Master RSA XML and the dispatcher private key are NOT stored here.
  * ``raw/dispatcher_keys/<biz>.xml`` — the unwrapped dispatcher private key,
    kept out of git by ``.gitignore`` so the live probes can decrypt in a
    second process without committing production key material.
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from datetime import datetime, timezone

from common import get, make_session, save_json, sha256_hex
from keys import dispatcher_key_path, metadata_key, unwrap_magic

BASE = (
    "https://raw.githubusercontent.com/CollapseLauncher/"
    "CollapseLauncher-ReleaseRepo/main/metadata/v3/stable"
)
MASTER_URL = f"{BASE}/config_master.json"
STAMP_URL = f"{BASE}/stamp.json"

BIZ_REGION = {
    "hk4e_global": "Global",
    "hk4e_cn": "Mainland China",
    "hk4e_bilibili": "Bilibili",
}
BIZ_ORDER = ["hk4e_global", "hk4e_cn", "hk4e_bilibili"]

# Fields worth keeping in the redacted summary (all public launcher metadata).
SUMMARY_FIELDS = [
    "ProfileName",
    "GameChannel",
    "ZoneName",
    "ZoneFullname",
    "GameType",
    "GameExecutableName",
    "GameBizName",
    "GameDispatchURL",
    "ProtoDispatchKey",
    "DispatcherKeyBitLength",
    "DispatcherKey",
    "ChannelID",
    "SubChannelID",
    "LauncherGetGameURL",
    "LauncherResourceURL",
    "LauncherType",
]


def b64decode(s: str) -> bytes:
    return base64.b64decode(s + "=" * (-len(s) % 4))


def unwrap_value(value, key, bit_size: int):
    if isinstance(value, str) and value.startswith("Q29sbGFwc2U"):  # b64("Collapse")
        return unwrap_magic(b64decode(value), key, bit_size).decode("utf-8")
    return value  # plain JSON scalar (int/bool/str) — already unwrapped


def resolve_config_paths(stamp: list[dict]) -> dict[str, str]:
    """Resolve per-biz preset file names from the release/preset table."""
    out: dict[str, str] = {}
    for biz, region in BIZ_REGION.items():
        matches = [
            e["MetadataPath"]
            for e in stamp
            if e.get("GameName") == "Genshin Impact"
            and e.get("GameRegion") == region
            and e.get("MetadataType") == "PresetConfigV2"
        ]
        if len(matches) != 1:
            raise RuntimeError(
                f"preset table has {len(matches)} GI / {region} entries: {matches}"
            )
        out[biz] = matches[0]
    return out


def collapse_region_id(fields: dict) -> tuple[int, str]:
    """Collapse ``PresetConfig.cs:815-823`` + ``DispatchHelper.cs:173-194``.

    Global has no fixed tuple in the preset: ``GetRegServerNameID`` falls back to
    the registry-selected server (USA = 0), which the live probe pinned to
    ``osusadispatch``. CN Main and Bili are fixed by (SubChannelID, ChannelID).
    """
    ch, sub = fields.get("ChannelID"), fields.get("SubChannelID")
    if sub == 1 and ch == 1:
        return 4, "preset (SubChannelID=1, ChannelID=1) -> CN Main"
    if sub == 0 and ch == 14:
        return 5, "preset (SubChannelID=0, ChannelID=14) -> CN Bilibili"
    return 0, "registry default (USA) pinned to osusadispatch by live probe"


def load_existing_summary():
    from common import SAMPLES

    p = SAMPLES / "metadata_summary.json"
    if p.exists():
        return json.loads(p.read_text(encoding="utf-8"))
    return {}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--biz",
        choices=BIZ_ORDER,
        action="append",
        help="biz to unwrap (repeatable); omit to fetch all three",
    )
    ap.add_argument("--offline-summary", help="print an existing summary instead of fetching")
    args = ap.parse_args()
    if args.offline_summary:
        print(open(args.offline_summary, encoding="utf-8").read())
        return 0

    biz_list = args.biz or BIZ_ORDER
    s = make_session()
    mr = get(s, MASTER_URL)
    sr = get(s, STAMP_URL)
    print(f"[master] {mr.status_code} {mr.headers.get('content-type')} {len(mr.content)}B {MASTER_URL}")
    print(f"[stamp ] {sr.status_code} {sr.headers.get('content-type')} {len(sr.content)}B {STAMP_URL}")
    mr.raise_for_status()
    sr.raise_for_status()
    master = mr.json()
    config_paths = resolve_config_paths(sr.json())
    print("[preset ] resolved from stamp.json: " + ", ".join(f"{b}={config_paths[b]}" for b in biz_list))

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

    summary = load_existing_summary()
    summary.setdefault("fetched_at_utc", datetime.now(timezone.utc).isoformat())
    summary["fetched_at_utc"] = datetime.now(timezone.utc).isoformat()
    summary["sources"] = sorted({MASTER_URL, STAMP_URL} | set(summary.get("sources", [])))
    summary["master"] = key_info
    summary.setdefault("biz", {})

    for biz in biz_list:
        url = f"{BASE}/{config_paths[biz]}"
        r = get(s, url)
        print(f"[{biz:14}] {r.status_code} {r.headers.get('content-type')} {len(r.content)}B {url}")
        r.raise_for_status()
        meta = r.json()

        unwrapped = {}
        for name in SUMMARY_FIELDS:
            if name in meta:
                unwrapped[name] = unwrap_value(meta[name], master_key, bit_size)

        # Persist the raw dispatcher key out of tree so dispatch.py can decrypt
        # in its own process. The summary below keeps only a fingerprint.
        dispatcher_xml = unwrapped.get("DispatcherKey")
        if isinstance(dispatcher_xml, str) and dispatcher_xml.strip():
            key_path = dispatcher_key_path(biz)
            key_path.parent.mkdir(parents=True, exist_ok=True)
            key_path.write_text(dispatcher_xml, encoding="utf-8")
            key_match = dispatcher_xml.strip() == (
                (key_path.parent.parent.parent / "dispatch_rsa.xml")
                .read_text(encoding="utf-8")
                .strip()
            )
            unwrapped["DispatcherKey"] = (
                f"<redacted: {len(dispatcher_xml)} chars, "
                f"sha256={sha256_hex(dispatcher_xml.encode())}>"
            )
        else:
            key_match = False

        region_id, region_reason = collapse_region_id(unwrapped)
        summary["biz"][biz] = {
            "config_file": config_paths[biz],
            "config_url": url,
            "region_id": region_id,
            "region_id_source": region_reason,
            "channel_id": unwrapped.get("ChannelID"),
            "sub_channel_id": unwrapped.get("SubChannelID"),
            "live_dispatcher_key_matches_genshinslimmer_copy": key_match,
            "fields": unwrapped,
            "all_field_names": sorted(meta.keys()),
        }
        print(
            f"[{biz:14}] seed={unwrapped.get('ProtoDispatchKey')} "
            f"bits={unwrapped.get('DispatcherKeyBitLength')} region={region_id}"
        )

    # Legacy top-level view (global) keeps existing scripts/README examples working.
    g = summary["biz"].get("hk4e_global")
    if g:
        summary["fields"] = g["fields"]
        summary["all_field_names"] = g["all_field_names"]
        summary["live_dispatcher_key_matches_genshinslimmer_copy"] = g[
            "live_dispatcher_key_matches_genshinslimmer_copy"
        ]

    p = save_json("metadata_summary.json", summary)
    print(f"[saved ] {p}")
    for biz in biz_list:
        f = summary["biz"][biz]["fields"]
        compact = {
            k: f.get(k)
            for k in (
                "GameDispatchURL",
                "ProtoDispatchKey",
                "DispatcherKeyBitLength",
                "DispatcherKey",
                "ChannelID",
                "SubChannelID",
            )
        }
        print(biz, json.dumps(compact, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
