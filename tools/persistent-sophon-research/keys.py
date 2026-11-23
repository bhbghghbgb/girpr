"""RSA key material used by the live dispatcher probes.

Two keys are involved:

1. ``dispatch_rsa.xml`` — the 2048-bit private key for ``query_cur_region``
   (``key_id=5``), copied verbatim from
   ``reference-projects/GenshinSlimmer/GenshinSlimmer.ps1:902``. It is public
   launcher/client material, not a girpr secret. Collapse's live metadata also
   serves the same key in its ``DispatcherKey`` field (1679-byte XML), which
   ``fetch_metadata.py`` verifies. See README for provenance/licensing notes.
2. The Collapse *metadata master key*, fetched at runtime from
   ``config_master.json``. It is a 128-byte (RSA-1024) PKCS#1 DER key wrapped in
   a Collapse-magic Brotli blob. It is never written to disk.
"""
from __future__ import annotations

import base64
import struct
import xml.etree.ElementTree as ET
from pathlib import Path

import brotli
from Crypto.PublicKey import RSA

HERE = Path(__file__).resolve().parent
COLLAPSE_SIG = 7310310183885631299


def rsa_from_xml(xml: str) -> RSA.RsaKey:
    root = ET.fromstring(xml)
    vals = {c.tag: base64.b64decode(c.text or "") for c in root}

    def i(name: str) -> int:
        return int.from_bytes(vals[name], "big")

    n, e = i("Modulus"), i("Exponent")
    d = i("D") if "D" in vals else None
    p = i("P") if "P" in vals else None
    q = i("Q") if "Q" in vals else None
    # .NET XML's InverseQ is q^-1 mod p (iqmp); PyCryptodome's 7th component is
    # u = p^-1 mod q. Supplying iqmp there fails validation, so let pycryptodome
    # derive dp/dq/u from (n, e, d, p, q) instead.
    if d and p and q and p * q == n:
        return RSA.construct((n, e, d, p, q))
    return RSA.construct((n, e, d))


def dispatcher_key() -> RSA.RsaKey:
    return rsa_from_xml((HERE / "dispatch_rsa.xml").read_text(encoding="utf-8"))


def is_magic(raw: bytes) -> bool:
    return len(raw) >= 32 and struct.unpack_from("<q", raw, 0)[0] == COLLAPSE_SIG


def magic_info(raw: bytes) -> tuple[int, int, int, int]:
    flags = struct.unpack_from("<q", raw, 8)[0]
    comp_size, decomp_size = struct.unpack_from("<qq", raw, 16)
    return flags & 0xFF, (flags >> 8) & 0xFF, comp_size, decomp_size


def unwrap_magic(raw: bytes, key: RSA.RsaKey | None = None, bit_size: int = 0) -> bytes:
    """Decrypt (optionally) and decompress a Collapse-magic blob's payload."""
    assert is_magic(raw), "not a Collapse-magic blob"
    comp_type, encrypted, comp_size, decomp_size = magic_info(raw)
    payload = raw[32:]
    if encrypted:
        from Crypto.Cipher import PKCS1_v1_5

        assert key is not None and bit_size > 0
        cipher = PKCS1_v1_5.new(key)
        chunks = []
        for off in range(0, len(payload), bit_size):
            pt = cipher.decrypt(payload[off : off + bit_size], None)
            if pt is None:
                raise RuntimeError("RSA decrypt failed in Collapse-magic blob")
            chunks.append(pt)
        payload = b"".join(chunks)
    if len(payload) != comp_size:
        raise RuntimeError(f"payload {len(payload)}B != compSize {comp_size}B")
    if comp_type == 0:
        out = payload[:decomp_size]
    elif comp_type == 1:
        out = brotli.decompress(payload)
    elif comp_type == 2:
        import zstandard

        out = zstandard.ZstdDecompressor().decompress(payload)
    else:
        raise RuntimeError(f"unsupported compression {comp_type}")
    if len(out) != decomp_size:
        raise RuntimeError(f"decompressed {len(out)}B != {decomp_size}B")
    return out


def metadata_key(master: dict) -> RSA.RsaKey:
    """Build the Collapse metadata master key from config_master.json's ``Key``."""
    raw = master["Key"]
    if isinstance(raw, str):
        raw = base64.b64decode(raw + "=" * (-len(raw) % 4))
    if is_magic(raw):
        der = unwrap_magic(raw)
    else:
        der = raw
    return RSA.import_key(der)
