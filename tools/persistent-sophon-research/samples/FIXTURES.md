# Synthetic fixtures for offline tests

Small, fully synthetic stand-ins for the live dispatcher, the three persistent
manifests and a couple of full-file hotfix blobs. They are the source for the
mock routes added in commits 4–9 of `docs/09`. **No production key, ciphertext,
manifest or game byte is used here.**

Regenerate (byte-for-byte deterministic after the first run) with:

```powershell
cd tools/persistent-sophon-research
.venv/Scripts/python.exe make_fixtures.py
```

`make_fixtures.py` generates `test_key.pem` once with a local RNG and commits it
as fixed test data; every other output is a pure function of that key and the
fixed constants in the script. Re-running reuses the committed key, so all
digests below are stable. The script self-checks its pure-Python XXH64 against
the reference vectors (`""` → `ef46db3751d8e999`, `"a"` → `d24ec4f1a98c6e5b`)
and round-trips the ciphertext through `dispatch.py`'s decrypt shape.

## Files (`samples/fixtures/`)

| file | purpose | bytes | MD5 |
|---|---|---:|---|
| `test_key.pem` | fixed **test-only** RSA-2048 private key, PKCS#1 PEM | 1674 | see below |
| `dispatch_response.json` | mock `query_cur_region` body, `{"content","sign"}` | — | — |
| `res.ndjson` | tiny res manifest (2 hotfix rows) | 279 | `ce5fe1b283fa2b97929eabb86452429f` |
| `data.ndjson` | tiny data manifest (2 rows, one blank line between) | 275 | `bbc05fb62bbde346e56368bf77b122d3` |
| `silence.ndjson` | tiny silence manifest (1 row) | 122 | `d8d8b615534bc1c66c7bb042d1b4603c` |
| `hotfix_a.blk` | 64-byte full-file blob for `blocks/00/11111111.blk` | 64 | `b2d3f56bc197fd985d5965079b5e7148` |
| `hotfix_b.pck` | 48-byte full-file blob for `English(US)/VO_TEST.pck` and `blocks/00/22222222.blk` | 48 | `5117f4269ca5456f8dd0e47ddd3e0832` |
| `fixtures.json` | machine-readable seed / fingerprint / all digests | — | — |

The private key's sha256 public-DER fingerprint is
`e0fcb3321f5dfd7a6e5d208699782e44c40da897b6ee6813c3d81ff8bae303c0`.
It exists only to make the mock decryption testable; it is not and never was a
game or launcher key. Never substitute a production key here.

## Fixed values (`fixtures.json` is authoritative)

* seed `0011223344556677`; channel `OSRELWin`; query version `OSRELWin9.9.0`;
  codename `hk4e_global`.
* revisions: res `99000001`/`aabbccddeeff`, data `99000002`/`112233445566`,
  silence `99000003`/`ffeeddccbbaa`.
* repo bases: `https://mock.girpr.test/client_game_res/test` and
  `https://mock.girpr.test/client_design_data/test` — mock routes rewrite this
  authority at bind time, exactly as Sophon prefixes are rewritten today.
* descriptor body sha256 `67e5716939f845156f87815b6a3a0880f4e8823b460037405840d2e8a01e9431`
  (633 bytes); ciphertext 768 bytes = 3 × 256-byte RSA blocks; JSON keys exactly
  `content` and `sign` (the `sign` is 256 zero bytes).

## Wire shape

A minimal protobuf is built by `make_fixtures.py`: root field `3` (properties)
carries fields `1`, `2`, `7`, `8`, `9`, `12`, `14`, `18`, `19`, `20`, `22`, `26`,
`27`, `36`, and a deliberately unknown field `37` for the generic field-dump
test. Root also carries opaque fields `11` and `14`. Field `22` is
`RepoResVersionProperties` with `1` (res revision), `3` (res descriptor NDJSON),
`5` (res hash) and `6` (version). Fields `19`/`20` are the data/silence
descriptors. Each descriptor's `md5` and `fileSize` describe the complete
corresponding `.ndjson` file (verified by the generator), so a mock manifest
route can serve it and the product's MD5 gate will pass.

The 633-byte protobuf is split into 245-byte chunks and each chunk is encrypted
with PKCS#1 v1.5 **public-key encryption** using the test key, mirroring
`dispatch.py`'s 256-byte-block private-key decryption. `sign` is dummy.

## Intentional test hooks

* `data.ndjson` has a blank line between its two rows — the parser must skip
  it (Collapse's own parser throws on it).
* `res.ndjson` row `blocks/00/11111111.blk` carries `isPatch: true` and a
  `hash` (XXH64); `English(US)/VO_TEST.pck` carries `category` (extra key);
  `data.ndjson`'s second row carries `localName` (extra key).
* The two blobs' MD5s match rows in `res.ndjson`, so a mock hotfix CDN route can
  serve them for a successful digest-gate test, a truncated body for the
  failure case, and a 404 on the primary URL to exercise alternate fallback.
