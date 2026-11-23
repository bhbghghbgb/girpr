"""Shared helpers for the persistent/Sophon live probes.

All network calls go through :func:`make_session` so the exact User-Agent is
pinned and reproducible. The protobuf parser here is deliberately *generic*:
the local GenshinGateway.proto omits fields 11/12/14 and 38-47, and one of the
research questions is exactly what those unknown fields contain.

Provenance: the wire parser mirrors ``Parse-ProtoFields`` in
``reference-projects/GenshinSlimmer/GenshinSlimmer.ps1:860-884``.
"""
from __future__ import annotations

import base64
import binascii
import json
import re
from collections import defaultdict
from pathlib import Path
from typing import Any

import requests

HERE = Path(__file__).resolve().parent
SAMPLES = HERE / "samples"
SAMPLES.mkdir(exist_ok=True)

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
)


def make_session(timeout: int = 20) -> requests.Session:
    s = requests.Session()
    s.headers.update(
        {
            "User-Agent": UA,
            "Accept": "application/json, text/plain, */*",
        }
    )
    s.request_timeout = timeout  # type: ignore[attr-defined]
    return s


def get(session: requests.Session, url: str, **kwargs: Any) -> requests.Response:
    kwargs.setdefault("timeout", getattr(session, "request_timeout", 20))
    return session.get(url, **kwargs)


# ---------------------------------------------------------------------------
# Protobuf wire parser
# ---------------------------------------------------------------------------
def read_varint(buf: bytes, pos: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while pos < len(buf):
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if b < 0x80:
            return result, pos
        shift += 7
        if shift > 70:
            break
    raise ValueError("bad varint")


def parse_fields(buf: bytes) -> dict[int, list[tuple[int, Any]]]:
    """Return ``{field_number: [(wire_type, value), ...]}`` preserving repeats."""
    fields: dict[int, list[tuple[int, Any]]] = defaultdict(list)
    pos = 0
    while pos < len(buf):
        try:
            tag, pos = read_varint(buf, pos)
        except ValueError:
            break
        fn, wt = tag >> 3, tag & 7
        if fn == 0:
            break
        if wt == 0:
            val, pos = read_varint(buf, pos)
        elif wt == 1:
            val = buf[pos : pos + 8]
            pos += 8
        elif wt == 2:
            ln, pos = read_varint(buf, pos)
            if ln < 0 or pos + ln > len(buf):
                break
            val = buf[pos : pos + ln]
            pos += ln
        elif wt == 5:
            val = buf[pos : pos + 4]
            pos += 4
        elif wt == 3:  # start group -> skip until matching end group
            depth = 1
            while depth and pos < len(buf):
                t, pos = read_varint(buf, pos)
                if (t & 7) == 3:
                    depth += 1
                elif (t & 7) == 4:
                    depth -= 1
            val = b"<group>"
        elif wt == 4:
            break
        else:
            break
        fields[fn].append((wt, val))
    return dict(fields)


PRINTABLE_RE = re.compile(rb"^[\x09\x0a\x0d\x20-\x7e]+$")


def describe_value(wt: int, val: Any, max_len: int = 400) -> str:
    if wt == 0:
        return f"varint={val}"
    if wt in (1, 5):
        return f"hex={val.hex()}"
    if wt == 2:
        if PRINTABLE_RE.match(val) and len(val) <= max_len:
            return f"str={val.decode('ascii')!r}"
        return f"bytes[{len(val)}]={val[:max_len].hex()}{'...' if len(val) > max_len else ''}"
    return f"wt{wt}={val!r}"


def dump_fields(fields: dict[int, list[tuple[int, Any]]], indent: str = "") -> str:
    lines = []
    for fn in sorted(fields):
        for wt, val in fields[fn]:
            lines.append(f"{indent}{fn:>3} [{wt}] {describe_value(wt, val)}")
    return "\n".join(lines)


def first(fields: dict[int, list[tuple[int, Any]]], fn: int) -> Any | None:
    entries = fields.get(fn)
    return entries[0][1] if entries else None


def as_str(val: Any | None) -> str | None:
    if val is None:
        return None
    if isinstance(val, bytes):
        return val.decode("utf-8", "replace")
    return str(val)


def save_json(name: str, obj: Any) -> Path:
    p = SAMPLES / name
    p.write_text(json.dumps(obj, indent=2, ensure_ascii=False), encoding="utf-8")
    return p


def b64(s: str) -> bytes:
    return base64.b64decode(s + "=" * (-len(s) % 4))


def sha256_hex(data: bytes) -> str:
    import hashlib

    return hashlib.sha256(data).hexdigest()
