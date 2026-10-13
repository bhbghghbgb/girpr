# `gircap` — live-artifact capture for `girpr` testing

**Status:** spec (design agreed, nothing built yet)
**Scope:** `tools/capture` only. This document and its siblings describe *this*
tool; they say nothing about `girpr`'s behaviour beyond what is needed to know
what a correct artifact looks like.

---

## 1. What this is

`gircap` fetches from the **live** HoYoPlay + Sophon APIs and writes a
**self-contained artifact bundle** containing:

1. a real `game_dir` fixture holding **one real file per test-matrix variant**,
   byte-exactly as it exists at the *old* version;
2. **every API response and manifest** the main tool would consult to reason
   about that file;
3. **every chunk blob** needed to materialize the old file, the current file,
   and the patch between them — retained, never deleted;
4. **ground truth** for the patch result, assembled by `gircap` itself.

It exists so that an agent (or a test) can later run the main tool against
`game/` and assert, offline and by hash, that it produced `expected/` and
fetched exactly the chunks `patch_fetch` says it must fetch.

It is a **capture tool**. It never patches, never repairs, never invokes the
main tool, and never imports its code.

---

## 2. Independence (non-negotiable)

| Rule | Enforcement |
|---|---|
| No dependency on the `girpr` crate | `Cargo.toml` has no `girpr` path/registry dep; a CI check rejects the string |
| Never invokes `girpr` or its tests | no `Command` against the `girpr` binary anywhere in the tree |
| No shared contracts | own flag set, own exit codes, own report format. The README is the only shared vocabulary (version strings) |
| No shared fixtures | `gircap` builds its own mock server for its own tests |

This mirrors `tools/sync`: *"Separate from the `girpr` binary; shares no flags,
no exit-code contract, no code."*

---

## 3. Live API surface used

All of the following were **measured** against `hk4e_global` on 2026-10-14.
Anything in this section marked *(measured)* is an observation, not an
assumption, and `gircap` records it in the artifact as evidence.

### 3.1 HoYoPlay (JSON, cheap — always captured verbatim)

| Endpoint | URL | Node |
|---|---|---|
| `getGameConfigs` | `https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3` | `data.launch_configs[]` |
| `getGameBranches` | `…/getGameBranches?launcher_id=…&language=en-us&game_ids[]=…` | `data.game_branches[]` |
| `getGameDeprecatedFileConfigs` | `…/getGameDeprecatedFileConfigs?launcher_id=…&language=en-us&game_ids[]=…&channel=1&sub_channel=0` | `data.deprecated_file_configs[]` |
| `getGamePackages` *(legacy, evidence only)* | `…/getGamePackages?launcher_id=…&language=en-us&game_ids[]=…` | `data.game_packages[]` |

Wrapper is `{"retcode":0,"message":"OK","data":{…}}`. `channel`/`sub_channel`
are sent **only** on `getGameDeprecatedFileConfigs`.

### 3.2 Sophon `getBuild` (JSON, cheap — always captured)

```
GET https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild
      ?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x[&tag=<version>]
```

`package_id` / `password` / `branch` come from `getGameBranches → main`, never
hardcoded.

### 3.3 Manifest + chunk blobs (octet-stream, zstd)

```
manifest: {manifest_download.url_prefix}/{manifest.id}
chunk:    {chunk_download.url_prefix}/{chunk.id}
```

### 3.4 Version pair *(measured, 2026-10-14)*

```
getGameBranches → main.tag       = "7.1.0"
                  main.diff_tags  = ["7.0.0", "6.7.0"]
```

**`getBuild?tag=` serves far more than `diff_tags`.** *(measured)*

| tag | result |
|---|---|
| *(none)* | ✅ `build_id sMXGW2ll3Fuu` (7.1.0) |
| `7.0.0` (in `diff_tags`) | ✅ `3XnPviMfoILi` |
| `6.7.0` (in `diff_tags`) | ✅ `E2zvp18oyzw6` |
| `6.6.0` (**not** in `diff_tags`) | ✅ `L7CJe4FP1z71` |
| `5.5.0` (**not** in `diff_tags`) | ✅ `wUcNY2SeSiQE` |

`diff_tags` is a **patch-support** list, not a retention list. A full old-version
file is therefore reconstructible **today** without the legacy zip path — the
concern raised at scoping is resolved and the legacy API is irrelevant to the
capture. `getGamePackages` is captured anyway, purely as dated evidence:

```
main.major.version = "5.5.0"   main.major.game_pkgs = 8   patches = ["5.4.0","5.3.0"]
```
— frozen at 2025-03-14, ~2 major versions stale.

**Pair resolution:** `--new-tag` defaults to `main.tag`; `--old-tag` defaults to
`diff_tags[0]` ("current minus one"). Any tag the API serves is accepted; an
unserveable tag is exit `2`, never a silent fallback.

### 3.5 Manifest wire format

```proto
message SophonChunkManifest { repeated SophonChunkFile chuncks = 1; }  // spelling is upstream's
message SophonChunkFile {
  string file = 1;  repeated SophonChunk chunks = 2;
  bool is_folder = 3;  int64 size = 4;  string md5 = 5;
}
message SophonChunk {
  string id = 1;  string uncompressed_md5 = 2;
  int64 offset = 3;  int64 compressed_size = 4;  int64 uncompressed_size = 5;
  int64 unknown = 6;  string compressed_md5 = 7;
}
```

### 3.6 Manifest structure *(measured, 7.0.0 and 7.1.0)*

These hold for both `game` manifests and are recorded as assertions, not
assumptions — if a future version violates one, the capture still works and
`selfcheck.json` says which property broke:

- 0 `is_folder` entries, 0 empty paths, 0 backslash paths, 0 duplicate paths, 0 non-ASCII paths
- chunk offsets strictly ascending, gapless from 0, `Σ uncompressed_size == size` for every file
- largest chunk: **3 689 476 B** compressed (bounds the main tool's whole-chunk buffering)

### 3.7 Hashes and containers *(measured)*

| Fact | Evidence |
|---|---|
| chunk blobs are zstd | `compression: 1`; magic `28 b5 2f fd` |
| `md5(raw chunk blob)` == `compressed_md5` == OSS `ETag` == OSS `Content-MD5` | verified |
| `md5(decompressed chunk)` == `uncompressed_md5` | verified |
| `md5(decompressed manifest)` == `manifest.checksum` | verified |
| `manifest.checksum` == the **32-hex suffix** of `manifest.id` | verified |
| the **16-hex prefix** of `manifest.id` is **opaque** | *not* xxh64 of the blob — see below |
| chunk URLs are namespaced per `build_id` | same chunk id under the other build's prefix → **404** |
| `Range:` is honoured | `Range: bytes=10-99` → `206` + `Content-Range: 10-99/259795` |
| CDN | `Server: AliyunOSS`, `Via: …cloudfront.net`, `x-cache: Hit from cloudfront`, `accept-ranges: bytes`, `etag`, `last-modified`, `content-md5` |

**The 16-hex prefix is not a hash of anything we can see.** *(measured)* it is
not xxh64 of the compressed blob, the decompressed blob, the checksum, the id,
the URL, or `build_id + id`. Collapse's cache validator
(`Hi3Helper.Sophon/Helper/Extension.cs:TryReadFromCached`) assumes it is an
xxh64, so for Genshin that validation can never pass and the cache is re-fetched
every time. `gircap` records the prefix verbatim, asserts **only** the 32-hex
suffix, and labels the prefix `opaque` in the artifact so a later reader does not
repeat the mistake.

---

## 4. What gets captured

### 4.1 Matrix, not a single file

`gircap` captures **one file per test-matrix variant** by default. Each variant
targets a *distinct* decision the main tool makes. `--per-class N` raises the
count; there are no per-variant flags.

| class | predicate over the old/new manifest entries | the branch it exercises | *(measured)* cheapest candidate, 7.0.0→7.1.0 | cost |
|---|---|---|---|---|
| `identical` | `old.md5 == new.md5` | skip: size+MD5 match, 0 bytes | `GenshinImpact_Data/Resources/browser_assets` | 54 B |
| `zero_reuse_single` | `reused == 0`, `nchunks == 1` | single download into empty `_tmp` | `…/AssetBundles/data_revision` | 42 B |
| `zero_reuse_multi` | `reused == 0`, `nchunks > 1` | N sequential downloads, no reuse | `GenshinImpact_Data/Resources/unity default resources` | 634 KB |
| `partial_same_offset` | `0 < reused < nchunks`, `moved == 0`, `size equal` | S2 reuse happy path | `…/Plugins/HoYoChannel.dll` | 1.83 MB |
| `partial_moved_offset` | `0 < reused < nchunks`, `moved > 0` | S2 reuse where the source offset ≠ target offset | `…/blocks/00/09801749.blk` | 22.7 MB |
| `partial_size_changed` | `0 < reused < nchunks`, `moved == 0`, `size differs` | oversize-`_tmp` truncate + length gate | `…/blocks/00/00981562.blk` | 21.7 MB |
| `added` | path only in the new manifest | "missing local file" branch | `…/Plugins/libHttpClient.GDK.dll` | 114 KB |
| `removed` | path only in the old manifest | not planned; `--purge-after` target | `…/blocks/00/00664172.blk` | 1.73 MB |
| `full_reuse_reordered` | `reused == nchunks`, `md5 differs` | every chunk reused, layout rearranged | **none in this pair** — recorded `unavailable` | — |

*(measured)* population over the 2 662 common paths: `identical` 1 592,
`partial_same_offset` 29, `partial_moved_offset` 430, `partial_size_changed`
447, `zero_reuse_single` 120, `zero_reuse_multi` 491, `added` 82, `removed` 11.

**Selection pool = `game` ∪ the selected audio fields.** Each Sophon manifest is
a `matching_field` pool, and every pool is scored and diffed, not just `game`:
*(measured)* each audio manifest carries 177 further file entries for ~1 MB
compressed. Widening `--audio` therefore widens the candidate corpus, which is
the backstop when a class has no `game`-field candidate in a given version pair.
Ranking prefers `game` on a cost tie (`field` asc after `total_cost`, so the
choice stays deterministic), and every capture records which `matching_field` it
came from. Audio files are real game files and are materialised exactly like
`game` ones when selected — the only difference is that they are, on average,
~80 MB each, so cost ranking matters there.

Predicates overlap in general (a file can satisfy several). The admission step
is **greedy by cost with an exclusion set**:

```
for class in REQUIRED_ORDER:
    if any already-captured file satisfies class:  record covered_by=<path>; continue
    pick the cheapest candidate satisfying class that is not already captured
    if none:                                      record status="unavailable" + reason
```

Every captured file is tagged with **all** classes it satisfies, so
`candidates.json` + `MANIFEST.json` always answer "which variant does this file
cover, and why".

### 4.2 Chunk retention — three named sets

For a captured path `P` and version pair `(O, N)`:

| set | definition | retained on disk |
|---|---|---|
| `materialize_old` | every chunk of `O`'s entry for `P` | ✅ |
| `materialize_new` | every chunk of `N`'s entry for `P` | ✅ |
| `patch_fetch` | `materialize_new` minus chunks with a `(uncompressed_md5, uncompressed_size)` match in `materialize_old` | ✅ (subset of the above) |

**`materialize_old ∪ materialize_new` is closed.** Nothing in the artifact may
depend on a file or blob that was not retained. This is asserted during capture
and re-provable offline by `gircap materialize --offline`, which rebuilds both
files from `chunks/` with no network and checks both manifest MD5s.

Blobs are stored **content-addressed by chunk id**, once, at
`chunks/blobs/<chunk-id>.zst`, with `chunks/index.json` mapping every
`(tag, file, chunk)` triple to its blob, its URL, and its membership in each of
the three sets. A chunk reused by two captured files — or identical across the
two builds — is stored once and referenced twice.

Corollaries that must hold and are asserted:

- `patch_fetch ⊆ materialize_new` — every byte the patch needs is already retained
- `reused` chunks of `N` are byte-identical to their `O` counterpart (same md5+size ⇒ same content)
- for `identical`, `materialize_old == materialize_new` in content; the blob is stored once
- for `removed`, only `materialize_old` exists and there is **no** `expected/`
- for `added`, only `materialize_new` exists and `game/` has no such file

### 4.3 `game/` — the fixture

Exactly three kinds of entry, nothing else, so the directory can be handed to
the main tool untouched:

```
game/
  config.ini                                              # game_version = OLD tag
  GenshinImpact_Data/Persistent/audio_lang_14             # tool-read config
  GenshinImpact_Data/<… the captured variant files …>     # OLD-version bytes
```

`config.ini` mirrors the main tool's own writer, so its surgical rewrite is a
no-op except for `game_version`:

```ini
[General]
channel=1
cps=hyp_hoyoverse
game_biz=hk4e_global
game_version=7.0.0
sdk_version=
sub_channel=0
```

The audio scan file holds display names, one per line, in the order the game
writes them: `Chinese` / `English(US)` / `Japanese` / `Korean`. `gircap` writes
`English(US)` by default; `--audio <code>…` changes it, and `--audio none`
writes an empty file. Its path comes from `getGameConfigs.audio_pkg_scan_dir` —
never hardcoded.

`--audio` has a **second** effect beyond the fixture file: each selected
language adds a manifest to the capture and to the selection pool (§4.1). The
default of `en-us` is therefore both a launchable-install default *and* a
deliberate widening of the candidate corpus — a second audio language costs
~1.2 MB of manifest and adds 177 more file entries to score. It exists as a
backstop for the case where the `game`-field pool yields no candidate for a
required class in a given version pair.

### 4.4 `expected/` — ground truth

`expected/<same rel path>` holding the **new**-version bytes, assembled by
`gircap` from retained chunks and verified against the new manifest's whole-file
MD5. Absent for `removed`.

---

## 5. Scoring

For every path present in both manifests:

```
old_index     : (uncompressed_md5, uncompressed_size) -> [old_offset, …]   # old manifest only
reused        : |{ c in N.chunks : c.key in old_index }|
moved         : |{ c in N.chunks : c.key in old_index and c.offset ∉ old_index[c.key] }|
patch_bytes   : Σ c.compressed_size for c in N.chunks if c.key ∉ old_index
old_bytes     : Σ c.compressed_size for c in O.chunks        # to materialize the input
new_bytes     : Σ c.compressed_size for c in N.chunks        # to materialize the output
total_cost    : old_bytes + new_bytes                       # ← the ranking key
```

`total_cost` — **not** `patch_bytes` — is the ranking key, because the old file
must be materialized in full for the fixture to be real. On 7.0.0→7.1.0 that
puts `HoYoChannel.dll` at 1.83 MB rather than its 274 KB patch size.

Tie-break order: `total_cost`, then `matching_field` asc (`game` before the
audio fields), then `moved` desc, then `reused` desc, then `path` asc. Fully
deterministic; re-running on the same version pair and `--audio` picks the same
files.

### Budget

`--max-download-bytes N` is a hard ceiling on `Σ total_cost` over admitted
files. Admission is **smallest-first**: files are sorted by `total_cost` asc and
taken until the budget is exhausted; the rest are recorded as
`status="over_budget"`. A single file that cannot fit is reported, not silently
swallowed — and the run still exits `0` with `selfcheck` green, because a
deliberate exclusion is not a failure.

---

## 6. Invariants asserted in `verify/selfcheck.json`

Each row is `{assertion, subject, expected, actual, ok}`.

| # | assertion |
|---|---|
| A1 | every API response captured has `retcode == 0` |
| A2 | `getBuild(new).data.tag == --new-tag`; `getBuild(old).data.tag == --old-tag` |
| A3 | manifest blob `md5(decompressed) == manifest.checksum` |
| A4 | `manifest.checksum == id 32-hex suffix`; `id 16-hex prefix` recorded as `opaque` |
| A5 | decoded manifest `file_count`/`chunk_count`/`uncompressed_size` reconcile with `getBuild.stats` |
| A6 | per chunk: `len(raw blob) == compressed_size`; `md5(raw) == compressed_md5`; `md5(zstd(raw)) == uncompressed_md5`; `len(decoded) == uncompressed_size` |
| A7 | `assemble(materialize_old)` has `md5 == old entry.md5` |
| A8 | `assemble(materialize_new)` has `md5 == new entry.md5` |
| A9 | `assemble(REUSE-from-old ∪ patch_fetch)` byte-equals `assemble(materialize_new)` |
| A10 | every reused chunk's old-file slice MD5 == `uncompressed_md5` |
| A11 | chunk offsets gapless from 0, ascending, `Σ uncompressed_size == size` |
| A12 | `patch_fetch` disjoint from the reuse set, and `⊆ materialize_new` |
| A13 | `game/config.ini` re-parses with `game_version == --old-tag` |
| A14 | `game/` contains no path outside `expected/`'s manifest set + the tool-read config files |
| A15 | `diff/` counts equal the recomputed set differences |
| A16 | `checksums.json` covers every file written under the artifact root, and re-hashing matches |
| A17 | each required matrix class is `captured` \| `covered_by` \| `unavailable` — never absent |

**`gircap capture` exits non-zero if any of A1–A14 fails.**

---

## 7. CLI

```
gircap capture
  --biz hk4e_cn|hk4e_global|hk4e_bilibili     (default hk4e_global)
  --out <DIR>                                 (default ./artifacts)
  [--new-tag latest|<tag>]                    (default: getGameBranches main.tag)
  [--old-tag <tag>]                           (default: main.diff_tags[0])
  [--per-class N]                             (default 1 — files per matrix class)
  [--max-download-bytes N]                    (default: unlimited)
  [--require-class <id>…]                     (fail if unmet; default: all required)
  [--files <rel-path>…]                       (bypass selection; still scored + recorded)
  [--audio <code>…|none]                      (default en-us; sets the scan file AND
                                               the selection pool — see §4.1/§4.3)
  [--manifest-detail full|paths-only]         (default full)
  [--gzip-threshold <bytes>]                  (default 1048576)
  [--probe-tags <tag>…]                       (default: diff_tags ∪ {old,new}; JSON only)
  [--no-chunk-blobs]                          (metadata only; breaks A7–A10 by design)
  [--jobs N] [--retries N] [--timeout S] [--log-level LV] [--log-file P]
  [--output text|json]

gircap verify      --artifact <DIR>            # re-run A3–A17 from stored bytes, no network
gircap materialize --artifact <DIR> [--out DIR] # rebuild old/new files from chunks/, no network
gircap list        --root <DIR>               # index every capture under a root
```

### Exit codes

| code | meaning |
|---|---|
| `0` | artifact complete and every asserted invariant green |
| `1` | usage / configuration error |
| `2` | network or API failure (incl. an unservable `--old-tag`) |
| `3` | an assertion failed — the artifact is **not** trustworthy |
| `4` | a `--require-class` could not be satisfied |

### Output

stdout is a pure data plane: one `SUMMARY` line, or with `--output json` a
single JSON object `{event:"summary", …}`. All narration goes to stderr via
`tracing` and, with `--log-file`, to a JSON-lines log. The only `println!` in the
crate is the summary emitter.

---

## 8. Consuming the bundle — and one external blocker

The bundle is built to be handed to the main tool. Two of the three consumption
paths work today; the third is blocked on a `girpr` feature that does not exist
yet.

| what you want to do | needs | status |
|---|---|---|
| inspect / assert from the metadata alone | nothing — `MANIFEST.json`, `patchmaps/`, `verify/` | ✅ works |
| rebuild old/new files offline | `gircap materialize` | ✅ works |
| **run `girpr --game-dir <artifact>/game` and check it patches to the new version** | **`girpr` path inclusion/exclusion** | ❌ **blocked** |

### 8.1 The blocker

`game/` holds a handful of files. Run the main tool against it as-is and the
main tool does what it is specified to do: build the plan from the **whole live
manifest** and fetch every file in it. A run meant to repair one 2.3 MB DLL would
pull ~130 GB, and a `--purge-after` run would also delete every file in the
bundle that the live manifest happens not to contain.

That is correct behaviour for the main tool and is not something `gircap` should
work around. The fix belongs in `girpr`: a repeatable path include/exclude
(glob, rooted at the game dir) that narrows both the repair work list and the
files-cleanup keep-set. With it, the intended invocation is roughly:

```
girpr --game-path <artifact>/game --biz hk4e_global
      --include "GenshinImpact_Data/Plugins/HoYoChannel.dll"
      --include "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk"
      …
```

and `SUMMARY download_bytes` can then be compared against each capture's
`patchmaps/<id>.json → totals.download_bytes`.

**Until that flag exists, the end-to-end check is a manual step and is recorded
as such.** `gircap` does not depend on `girpr`, does not invoke it, and does not
shim the missing flag; `docs/02-plan.md` M11 runs the check by hand and marks it
blocked rather than quietly skipping it.

### 8.2 What the artifact provides so that check is cheap when it unblocks

- `expected/` gives the exact bytes a correct run must produce, so the
  comparison is `md5`, not eyeballing.
- `patchmaps/<id>.json → expectations` gives `download_bytes` (must equal),
  `result_md5` (must equal) and **`must_not_fetch`** (must not be requested) —
  the last one names the chunk ids a working reuse path skips, so a broken reuse
  path is a specific, nameable failure rather than a byte-count mismatch.
- `game/config.ini` already reads the old version, so the run takes the
  `getBuild(old)` reuse path without extra flags.
- `verify/` makes the fixture's own integrity a precondition, so a bundle that
  fails the check can be ruled out as the cause.

---

## 9. Decisions taken

| decision | value | where |
|---|---|---|
| workspace membership | **workspace member** (`members = [".", "tools/sync", "tools/capture"]`), matching `girsync` | plan M0 |
| artifact root | **`artifacts/` in the repo, gitignored** — bundles are large and regenerable | plan M0 |
| `--audio` default | **`en-us`** — launchable default, and it widens the selection pool | §4.3 |
| `--per-class` default | **1** | §4.1 |
| per-class variants | **no individual flags** — the matrix is the interface | §4.1 |
| commit discipline | **one commit per milestone, no milestone lands uncommitted** | plan §3 |

---

## 10. Out of scope

- No patching, no hdiff, no 7z, no predownload, no purge simulation
- No `getGameScanInfo`, SDK, WPF, or plugin enumeration
- No cross-build *cross-file* chunk reuse modelling beyond **measuring and
  recording** it — *(measured)* 98 910 new chunks have an MD5 present somewhere
  in the old manifest, but only **2 files** have a chunk reusable *only* from a
  different old file (1 776 356 B forgone by same-file-only reuse). That number
  is recorded in `diff/reuse_pool.json` because it quantifies a deliberate
  simplification, not because `gircap` acts on it.
- Audio manifests are captured for the selection pool and can supply a capture
  when the `game` pool has no candidate for a class, but no audio *bulk* beyond
  the selected files is ever downloaded