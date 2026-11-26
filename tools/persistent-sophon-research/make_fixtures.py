"""Generate the small SYNTHETIC fixtures used by the offline Rust tests.

Nothing here touches production keys or live endpoints. The RSA-2048 key is a
test-only key generated once and committed as ``samples/fixtures/test_key.pem``;
all other outputs are deterministic functions of that key and the fixed tables
below, so re-running this script reproduces byte-identical fixtures.

Outputs (all under ``samples/fixtures/``):
  * ``test_key.pem``           — fixed test-only RSA-2048 private key (PKCS#1)
  * ``dispatch_response.json`` — mock ``query_cur_region`` body: base64 of
                                 PKCS#1 v1.5-encrypted synthetic protobuf
  * ``res.ndjson``, ``data.ndjson``, ``silence.ndjson`` — tiny manifests
  * ``hotfix_a.blk``, ``hotfix_b.pck`` — tiny full-file blobs matching rows
  * ``fixtures.json``          — seed, key fingerprint, MD5/size of every file

Run with: ``.venv/Scripts/python.exe make_fixtures.py``
"""
from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path

from Crypto.Cipher import PKCS1_v1_5
from Crypto.PublicKey import RSA

from common import SAMPLES, parse_fields

FIX = SAMPLES / "fixtures"
KEY_PATH = FIX / "test_key.pem"

# Fixed, test-only constants (never production material).
SEED = "0011223344556677"
CHANNEL = "OSRELWin"
GAME_VERSION = "9.9_live"
QUERY_VERSION = "OSRELWin9.9.0"
CODENAME = "hk4e_global"
RES_REVISION = 99000001
DATA_REVISION = 99000002
SILENCE_REVISION = 99000003
RES_HASH = "aabbccddeeff"
DATA_HASH = "112233445566"
SILENCE_HASH = "ffeeddccbbaa"
RES_BASE = "https://mock.girpr.test/client_game_res/test"
DATA_BASE = "https://mock.girpr.test/client_design_data/test"

MASK = (1 << 64) - 1
P1 = 0x9E3779B185EBCA87
P2 = 0xC2B2AE3D27D4EB4F
P3 = 0x165667B19E3779F9
P4 = 0x85EBCA77C2B2AE63
P5 = 0x27D4EB2F165667C5


def rotl(x: int, n: int) -> int:
    return ((x << n) | (x >> (64 - n))) & MASK


def xxh64(data: bytes, seed: int = 0) -> int:
    """Minimal dependency-free XXH64 (validated against reference vectors)."""
    def u64(buf: bytes) -> int:
        return int.from_bytes(buf, "little")

    def round_(acc: int, val: int) -> int:
        acc = (acc + val * P2) & MASK
        acc = rotl(acc, 31)
        return (acc * P1) & MASK

    def merge(acc: int, val: int) -> int:
        acc ^= round_(0, val)
        return (acc * P1 + P4) & MASK

    ln = len(data)
    i = 0
    if ln >= 32:
        v1 = (seed + P1 + P2) & MASK
        v2 = (seed + P2) & MASK
        v3 = seed & MASK
        v4 = (seed - P1) & MASK
        while i + 32 <= ln:
            v1 = round_(v1, u64(data[i : i + 8]))
            i += 8
            v2 = round_(v2, u64(data[i : i + 8]))
            i += 8
            v3 = round_(v3, u64(data[i : i + 8]))
            i += 8
            v4 = round_(v4, u64(data[i : i + 8]))
            i += 8
        h = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & MASK
        for v in (v1, v2, v3, v4):
            h = merge(h, v)
    else:
        h = (seed + P5) & MASK
    h = (h + ln) & MASK
    while i + 8 <= ln:
        h ^= round_(0, u64(data[i : i + 8]))
        h = (rotl(h, 27) * P1 + P4) & MASK
        i += 8
    if i + 4 <= ln:
        h ^= (int.from_bytes(data[i : i + 4], "little") * P1) & MASK
        h = (rotl(h, 23) * P2 + P3) & MASK
        i += 4
    while i < ln:
        h ^= (data[i] * P5) & MASK
        h = (rotl(h, 11) * P1) & MASK
        i += 1
    h ^= h >> 33
    h = (h * P2) & MASK
    h ^= h >> 29
    h = (h * P3) & MASK
    h ^= h >> 32
    return h


def varint(n: int) -> bytes:
    out = bytearray()
    while n >= 0x80:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n)
    return bytes(out)


def fv(num: int, val: int) -> bytes:
    return varint(num << 3) + varint(val)


def fb(num: int, data: bytes) -> bytes:
    return varint((num << 3) | 2) + varint(len(data)) + data


def ndjson_line(remote: str, body: bytes, extra: dict | None = None) -> bytes:
    obj = {
        "remoteName": remote,
        "md5": hashlib.md5(body).hexdigest(),
        "hash": f"{xxh64(body):016x}",
        "fileSize": len(body),
    }
    if extra:
        obj.update(extra)
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode() + b"\r\n"


def descriptor(name: str, body: bytes) -> bytes:
    return ndjson_line(
        name, body, {"isPatch": True} if name == "res_versions_external" else None
    ).rstrip(b"\r\n")


def load_or_create_key() -> RSA.RsaKey:
    if KEY_PATH.exists():
        return RSA.import_key(KEY_PATH.read_bytes())
    # One-time generation. The committed file is the fixed test key thereafter.
    key = RSA.generate(2048)
    KEY_PATH.write_bytes(key.export_key(format="PEM", pkcs=1))
    return key


def build_plaintext(res_desc: bytes, data_desc: bytes, sil_desc: bytes) -> bytes:
    rv = fv(1, RES_REVISION) + fb(3, res_desc) + fb(5, RES_HASH.encode()) + fb(6, GAME_VERSION.encode())
    props = b"".join(
        [
            fb(1, b"127.0.0.1"),
            fv(2, 22101),
            fb(7, b"TEST"),
            fb(8, RES_BASE.encode()),
            fb(9, DATA_BASE.encode()),
            fb(12, GAME_VERSION.encode()),
            fv(14, DATA_REVISION),
            fv(18, SILENCE_REVISION),
            fb(19, data_desc),
            fb(20, sil_desc),
            fb(22, rv),
            fb(26, DATA_HASH.encode()),
            fb(27, SILENCE_HASH.encode()),
            fb(36, CODENAME.encode()),
            fb(37, b"test-ipv6"),  # extra field: generic dump must tolerate it
        ]
    )
    return fb(3, props) + fb(11, bytes(range(16))) + fb(14, b"dGVzdA==")


def main() -> int:
    FIX.mkdir(parents=True, exist_ok=True)
    key = load_or_create_key()

    # Known-answer check for the pure-Python XXH64.
    assert xxh64(b"") == 0xEF46DB3751D8E999, hex(xxh64(b""))
    assert xxh64(b"a") == 0xD24EC4F1A98C6E5B, hex(xxh64(b"a"))

    blob_a = bytes(range(64))
    blob_b = bytes((i * 7 + 3) & 0xFF for i in range(48))
    (FIX / "hotfix_a.blk").write_bytes(blob_a)
    (FIX / "hotfix_b.pck").write_bytes(blob_b)

    # A full dispatcher manifest contains only NDJSON rows. Its descriptor
    # (embedded in the protobuf) describes the complete file bytes.
    res_ndjson = b"".join(
        [
            ndjson_line("blocks/00/11111111.blk", blob_a, {"isPatch": True}),
            ndjson_line("English(US)/VO_TEST.pck", blob_b, {"category": "Audio"}),
        ]
    )
    # Blank line in the middle: the parser must skip it (Collapse throws).
    data_ndjson = (
        ndjson_line("blocks/00/22222222.blk", blob_b)
        + b"\n"
        + ndjson_line("hardware_model_config.json", blob_b, {"localName": "config.json"})
    )
    sil_ndjson = ndjson_line("blocks/00/33333333.blk", blob_a)

    for name, body in (
        ("res", res_ndjson),
        ("data", data_ndjson),
        ("silence", sil_ndjson),
    ):
        (FIX / f"{name}.ndjson").write_bytes(body)

    res_desc = descriptor("res_versions_external", res_ndjson)
    data_desc = descriptor("data_versions", data_ndjson)
    sil_desc = descriptor("silence_data_versions", sil_ndjson)
    proto = build_plaintext(res_desc, data_desc, sil_desc)
    pub = key.publickey()
    cipher = PKCS1_v1_5.new(pub)
    enc = b"".join(cipher.encrypt(proto[i : i + 245]) for i in range(0, len(proto), 245))
    response = {
        "content": base64.b64encode(enc).decode(),
        "sign": base64.b64encode(bytes(256)).decode(),
    }
    (FIX / "dispatch_response.json").write_text(
        json.dumps(response, indent=2) + "\n", encoding="utf-8"
    )

    # Round-trip check: decrypt like dispatch.py and confirm the field shape.
    dec = PKCS1_v1_5.new(key)
    back = b"".join(
        dec.decrypt(enc[i : i + 256], None) for i in range(0, len(enc), 256)
    )
    assert back == proto
    root = parse_fields(back)
    assert 3 in root and 11 in root and 14 in root

    def digest(name: str) -> dict:
        b = (FIX / name).read_bytes()
        return {"file": name, "bytes": len(b), "md5": hashlib.md5(b).hexdigest()}

    fixture_doc = {
        "description": "Synthetic, test-only fixtures. No production keys or endpoints.",
        "seed": SEED,
        "channel": CHANNEL,
        "query_version": QUERY_VERSION,
        "codename": CODENAME,
        "test_key": {
            "file": "test_key.pem",
            "bits": key.size_in_bits(),
            "sha256_public_der": hashlib.sha256(key.publickey().export_key("DER")).hexdigest(),
        },
        "dispatch_response": {
            "file": "dispatch_response.json",
            "json_keys": sorted(response),
            "ciphertext_bytes": len(enc),
            "rsa_blocks": len(enc) // 256,
            "plaintext_bytes": len(proto),
            "plaintext_sha256": hashlib.sha256(proto).hexdigest(),
        },
        "revisions": {
            "res": [RES_REVISION, RES_HASH],
            "data": [DATA_REVISION, DATA_HASH],
            "silence": [SILENCE_REVISION, SILENCE_HASH],
        },
        "manifests": {n: digest(f"{n}.ndjson") for n in ("res", "data", "silence")},
        "hotfix_blobs": {n: digest(n) for n in ("hotfix_a.blk", "hotfix_b.pck")},
    }
    (FIX / "fixtures.json").write_text(
        json.dumps(fixture_doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(json.dumps(fixture_doc, indent=2, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
