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

| Rule | How it shows up |
|---|---|
| No dependency on the `girpr` crate | `Cargo.toml` declares no `girpr` path or registry dep |
| Never invokes `girpr` or its tests | no `Command` against the `girpr` binary anywhere in the tree |
| No shared contracts | own flag set, own exit codes, own report format. The README is the only shared vocabulary (version strings) |
| No shared fixtures | `gircap` builds its own mock server for its own tests |

The intent is **do not couple**. Two tools that must stay independent should not
end up depending on each other in order to prove one of them works, and the
artifact is the whole interface between them: a bundle on disk, read by whoever
wants it. None of the above is mechanically enforced — nothing here checks it —
so it is a rule the tree has to keep, not a guarantee a build makes.

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

### 3.3a The two route families disagree about failure *(measured, 2026-10-16)*

This is recorded rather than assumed because both projects' retry policies are
written against it, and because a capture that never sees a failure cannot
record one.

| family | a request it will not serve | the answer |
|---|---|---|
| blob (`autopatchhk.yuanshen.com`, `Server: AliyunOSS`) | bogus / short id, unknown `build_id`, a valid id under the other build's prefix, a completely unknown path | **`404`** + `application/xml` `<Error><Code>NoSuchKey</Code>…`. **Never** a `200` carrying a code |
| JSON (`sg-hyp-api`, `sg-downloader-api`) | `getBuild?tag=<unknown>` | `200` + `{"data":null,"message":"not found","retcode":-202}` |
| | `getBuild` with a bad `package_id` | `200` + `retcode -201` (`invalid params`) |
| | `getGameConfigs` with a bad `game_ids[]` | `200` + `retcode -1001` (`invalid params`) |
| | `getGameBranches` with a bad `game_ids[]` | `200` + **`retcode 0`** and an empty `game_branches` — **not an error** |

That last row is the one that gets mistaken. It is a *success with nothing in
it*: the launcher answered correctly, there are no branches for that game. Any
consumer that treats "empty node" as "retry me" turns a real answer into a retry
loop.

`gircap` records all seven shapes in **`api/error_shapes.json`** on every capture,
with the volatile fields named (`RequestId`, `HostId`, `Key`, and the body length
that follows from `Key`) and `retryable: false` on each — so the policy is
readable from the artifact and not only from the fetcher. See
[`01-artifact-schema.md` §5a](01-artifact-schema.md).

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
  int64 unknown_field6 = 6;  string compressed_md5 = 7;
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
| chunk URLs are namespaced per `build_id` | the same chunk id under the other build's prefix is a **`404`** — see §3.3a, and `api/error_shapes.json → blob_wrong_build` |
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

### 4.1a What `api/` carries, including the failures

`api/` holds the six response bodies **byte-exact**, the request log, and two
generated documents that are this tool's own measurements rather than the API's
words:

| document | what it is |
|---|---|
| `tag_probe.json` | which tags the API will actually serve |
| `error_shapes.json` | what the API does with a request it will not serve |

The second exists because **a bundle otherwise contains no not-found evidence
at all** — every request a capture makes succeeds, so a recorded bundle says
nothing about what a missing object looks like. A tool that replays a bundle
would have to invent that, and its "the API 404s things" would be unfounded.

The two route families disagree about failure, and the document records both
(measured 2026-10-16, `hk4e_global`):

| family | missing object | measured answer |
|---|---|---|
| blob (`autopatchhk.yuanshen.com`, `Server: AliyunOSS`) | manifest, chunk, unknown `build_id`, **a valid id under the other build's prefix** | `404` + `application/xml` `NoSuchKey` — never a `200` with a code |
| JSON (`sg-hyp-api`, `sg-downloader-api`) | bad `package_id` / `game_ids[]`, unknown tag | `200`, failure in `retcode`: `-201`, `-1001`, `-202` |

And one answer in that table is **not a failure**: `getGameBranches` with an
unknown `game_ids[]` returns `{"retcode":0,…,"data":{"game_branches":[]}}`. It is
recorded with `error: false` so a later reader cannot infer a failure from the
shape of the rows around it.

**A consumer must not synthesize its own failure shapes.** Replay `status`,
`content-type` and `body` from `api/error_shapes.json`, substituting only the
fields that entry marks volatile (`RequestId`, `HostId`, `Key` and the body
length that follows from `Key`; the JSON envelopes have none). A bundle
predating the file has no such document — skip those assertions and say so
rather than inventing the behaviour.

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
| A17 | each required matrix class is `captured` \| `covered_by` \| `unavailable` - never absent |

**`gircap capture` exits non-zero if any of A1-A14 fails.**

### A18, added by M9 — and what `verify` does *not* re-derive

None of the seventeen asks whether `game/` is what the retained blobs say it is.
That is the bundle's central claim and the reason `materialize` exists, so M9
gives it an id of its own:

| A18 | the bytes rebuilt from `chunks/` are byte-for-byte the bytes `game/` and `expected/` hold |

A18 is a `verify`-only row. The capture cannot record it, because it is a
statement about the bundle rather than about the run, so it never appears in a
capture's `selfcheck.json`.

### A19, added by M11 — the converse of A14

A14 asks one direction: is `game/` free of paths the tool under test would build
a plan against? It cannot see the other failure — a capture admitted for
`partial_same_offset` whose two variant files came out byte-identical. That
bundle would satisfy A14, satisfy A7/A8 (each side matches its own declared
MD5), and be useless, because the fixture would hold no evidence of the patch it
was captured for.

| A19 | each two-sided captured file differs between `game/` and `expected/` exactly when its class says it should |

A19 is found by comparing the two trees by hand, which is why it exists: M11's
step 3 does exactly that comparison and nothing in the suite asserted it. It
skips one-sided captures (`added`, `removed`) — their file is absent from one
tree *by construction*, which is the point of the class.

### A20, added by M12 — a patchmap must add up to itself

| A20 | a patchmap's `totals` and `expectations` are the arithmetic of its own `plan` |

`verify`-only, like A18: it is a statement about the bundle, so no capture records
it. It re-sums `compressed_size` over the plan's `FETCH` entries, recounts
`reuse`/`fetch`/`moved`, and checks that `expectations.download_bytes` is that same
figure.

A20 exists because the bundle that motivated it carried `"plan": null`,
`"totals": null` and `"expectations": { … "download_bytes": 0 }` for its `added`
capture, and `verify` passed it with 83 green rows — because nothing looked. The
one-chunk file in that bundle costs 114 357 B to repair and the artifact said
zero. §1 of the artifact schema records the rule that now prevents it; A20 is what
stops the rule being quietly dropped.

It is re-derivable offline like the other seven because it reads only
`patchmaps/*.json`, which is why it belongs in the derived set rather than the
covered-by-recording one.

So there are **twenty** ids: seventeen from this spec, A18 and A20 from `verify`
only, A19 from a capture.

`gircap verify` re-derives **nine** of them — A6, A7, A8, A11, A13, A16, A17,
A18 and A20. It does not re-derive A1-A5, A9, A10, A12, A14 or A15: those
are statements about the API bodies, the manifests and the diff, and re-running
them inside the bundle would be re-running the capture's own algorithm against
the capture's own output, which can only ever agree.

The nine are split in two, and `SUMMARY` names both as `checks_derived=`:

| set | ids | what it means |
|---|---|---|
| `REDERIVED` | A6, A7, A8, A11, A13, A16, A17 | re-derived **and** written down by the capture, so `verify` can cross-check them against the recording (`agrees_with_recording=`) |
| `DERIVED_UNRECORDED` | A18, A20 | re-derived from the bundle's own documents, but no capture records them — there is nothing to agree with |

Keeping them apart is what stops the summary claiming a cross-check that cannot
happen. `checks_covered_by_recording=` names the complement.

One asymmetry worth stating: **A6 is re-derived against `chunks/index.json`, not
against the manifests.** The four manifest-declared checks are already recorded in
the index; re-deriving them would need the manifest, and a verifier that reaches
for the manifest to check the manifest's own blob is checking the wrong thing.
What `verify` re-derives is the part the index is responsible for — that the file
at `stored_path` is the bytes the index describes. A blob the index lists with no
path (which is what `--no-chunk-blobs` produces) is **red, not skipped**: that is
the finding that the bundle cannot be materialized.

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

gircap verify      --artifact <DIR>            # re-derive A6,A7,A8,A11,A13,A16,A17,A18,A20, no network
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

stdout is a pure data plane: **records, one per line, summary last.** With
`--output text` a record is `LABEL k=v k=v …`; with `--output json` it is one
object per line, NDJSON, no enclosing array. All narration goes to stderr via
`tracing` and, with `--log-file`, to a JSON-lines log. The only `println!` in the
crate is the record emitter.

Every run ends with exactly one `SUMMARY` — the terminal record, carrying the
result and the exit code. A command may emit records before it: `list` writes
one `BUNDLE` per bundle it found and then the `SUMMARY`, so a machine can read
every per-bundle fact (path, tags, capture id, patchmap count, blob count,
`selfcheck`, `complete`) from stdout instead of parsing stderr narration, and
can read the stream until the summary rather than expecting one line and
parsing it.

```
$ gircap list --root artifacts --output json
{"event":"bundle","tool":"gircap","version":"0.1.0","path":"…/20261015T092615Z","id":"20261015T092615Z","tags":"7.0.0 -> 7.1.0","patchmaps":8,"blobs":38,"selfcheck":"pass","complete":true}
{"event":"bundle", … ,"id":"20261015T111345Z", …}
{"event":"list","tool":"gircap","version":"0.1.0","result":"ok","root":"artifacts","bundles":2, …,"exit_code":0}
```

`event` is the discriminator and **always leads**, so a consumer can dispatch
mid-stream. Its value is the record's command for the terminal record
(`capture` / `verify` / `list` / `materialize`) and the label lowercased
otherwise (`BUNDLE` → `bundle`) — a `BUNDLE` record saying `event: "list"` would
tell a reader nothing the label does not. Within one object the keys are
**names**: read them by name, never by position.

A directory under the root that does not open as a bundle is **not** a record.
It stays stderr-only, as it always was: there is no bundle to describe, so a
record would have to carry the failure instead of the facts.

---

## 8. Consuming the bundle with `girpr`

The bundle is built to be handed to the main tool.

| what you want to do | needs | status |
|---|---|---|
| inspect / assert from the metadata alone | nothing — `MANIFEST.json`, `patchmaps/`, `verify/` | ✅ works |
| rebuild old/new files offline | `gircap materialize` | ✅ works |
| **run `girpr --game-path <copy-of-game>` and check it patches to the new version** | **`girpr` path inclusion/exclusion** | ✅ **shipped** — `girpr --include <GLOB>` |

> **Update: the blocker is resolved, and the check has been run.** The path
> include/exclude asked for below shipped as `girpr --include` / `--exclude`,
> alongside `--max-files` / `--max-download-bytes` / `--max-write-bytes` /
> `--max-delete-bytes` (exit `5` on a ceiling, never trimming the work list),
> `--print-plan`, `--plan-out`, `--purge-only` and `--fail-fast`. `girpr`'s
> README is the authority on what the flags guarantee — in particular that a
> filtered-out path is treated as nonexistent and so is invisible to the
> files-cleanup as well as to the repair, and that a scoped run does not bump
> `config.ini`.
>
> §8.1's body, §8.1.1 and §8.1.2 were **revised by the capture side** once the
> flag existed, because the procedure was `gircap`'s to own and the reason it gave
> for avoiding `--purge-after` had become false. All three were then run: every
> capture repaired against a scratch copy, every figure matching the patchmap to
> the byte, and the results are in §8.1.1.

### 8.1 Why the scope was needed, and what it changed

`game/` holds a handful of files. Run the main tool against it unscoped and the
main tool does what it is specified to do: build the plan from the **whole live
manifest** and fetch every file in it. A run meant to repair one 2.3 MB DLL would
pull ~130 GB, and a `--purge-after` run would also delete every file in the
bundle that the live manifest happens not to contain.

That is correct behaviour for the main tool and was never something `gircap`
should work around — which is why the fix was asked for in `girpr` rather than
shimmed here. The intended invocation is now a real one:

```
girpr --game-path <copy-of-game> --biz hk4e_global
      --include "GenshinImpact_Data/Plugins/HoYoChannel.dll"
      …
```

and `SUMMARY download_bytes` can be compared against each capture's
`patchmaps/<id>.json → totals.download_bytes`.

**`gircap` still does not depend on `girpr`.** It does not invoke it, does not
read its flags, and does not shim them; §2's independence rule is unchanged and
the grep over `tools/capture/src/` still returns doc comments only. The
end-to-end check remains a manual step, run by hand against a scratch copy, for
the reason §8.1.2 gives.

### 8.1.1 The procedure, and the numbers it must produce

Recorded so that whoever runs it does not have to reconstruct it, and so that a
result can be checked rather than believed. **This is the procedure that was run
against this bundle, and the results below are transcribed from that run, not from
the docs.** Against `artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z`.

1. **Copy `game/` to scratch.** `girpr` writes into the game directory, so point
   `--game-path` at the copy and never at the bundle: that is what keeps a
   cleanup flag from turning into a way to destroy the evidence. Run every capture
   against the same copy — each touches exactly one path, so the runs accumulate
   and the reuse map stays anchored to the old version. Afterwards
   `gircap verify --artifact <bundle>` must still exit `0`; it did, 90/90.

2. **Pre-flight for free, before spending anything.** `--print-plan` does not
   repair, purge or bump, and it is not an offline mode — it still fetches the
   API — but it costs no payload:

   ```
   PLAN files=1 est_download_bytes=274426 purge_files=0 purge_bytes=0 scoped=1
   ```

   `est_download_bytes` is **`totals.download_bytes`** — 274 426 is exactly the
   `partial_same_offset` figure in this bundle's patchmap. `files=1` proves the
   glob selected the intended path and nothing else, and `purge_files=0` is the
   scope holding over the cleanup.

   `--check-only` is the other free check, and it **is expected to report
   damage**: under `--include <a capture path>` the run verifies only that one
   path against the *new* manifest while the copy still holds the *old* bytes,
   so the seven repairable captures come back damaged and exit **`4`**. Only
   `identical` is clean and exits `0`, because its bytes already match. A `0`
   here would mean the check looked at nothing.

   What it names is what makes it useful:

   ```
   CHECK fail: md5 expect_md5=d992a789… actual_md5=1e210ff1…
   ```

   this capture's `new.md5` and `old.md5`. Those two come from `tracing::warn!`
   in `src/repair/check.rs`, so they reach **stderr and the log file only, never
   stdout** — on the data plane the only signal is `exit 4`. A mismatch means
   the fixture and the tool disagree about the file before a single byte moves.

3. **Repair, once per capture**, with `--include` naming exactly that capture's
   path, plus `--max-files 1`, `--io-threads 1` and `--fail-fast`. `--max-files 1`
   is the cheap answer to `*` crossing `/` — and this fixture contains
   `GenshinImpact_Data/Resources/unity default resources`, a path with a space in
   it, so the glob is worth quoting and bounding. One run per capture is what
   makes `download_bytes` attributable; a single run over all eight sums to the
   same total but cannot be compared per capture. `removed` is the exception and
   takes `--purge-only` instead of `--max-files 1` (§8.1.2).

4. **Assert five things**, per capture:

   | # | assertion | read from |
   |---|---|---|
   | 1 | `SUMMARY download_bytes` equals `patchmaps/<id>.json → totals.download_bytes` | `SUMMARY` |
   | 2 | the patched file's MD5 equals `expectations.result_md5` | the file on disk |
   | 3 | no id in `expectations.must_not_fetch` was requested | **the replay server's own request log** — see below |
   | 4 | exit `0`, `config_bumped=0`, `local_manifest=ok`, every path outside the scope untouched | `SUMMARY` + a before/after tree diff |
   | 5 | for `removed`: the path is gone, and `deleted_extra_bytes` equals `old.size` | `SUMMARY` + the tree diff |

   **The normative source for assertion 3 is the server's.** The bridge runs a
   shim that records what it was asked for, and that record is the primary
   answer: it is the only one that sees every request, including any that no
   client-side log line describes.

   The procedure below reads assertion 3 out of `<exe-dir>/logs/girpr_<ts>.log`
   at `--log-level trace` instead, and that is the fallback for a reader with no
   shim. Its own caveat is the reason the server log is better: **a reused chunk
   is logged too**, as `reuse slice md5 ok -> copy chunk=<id>`, so a naive
   substring search for the id reports a violation on a perfectly correct reuse —
   which is how a first attempt at this check failed, on 19 chunks that were never
   downloaded. Match on the *download* record, never on the bare id:
   `chunk downloaded chunk=<id>` and
   `no local reuse candidate -> download chunk=<id>`.

   `local_manifest=ok` is assertion 4's teeth. Without it a run whose local
   manifest failed to load still exits `0` with a plausible byte count, having
   reused nothing; the key is emitted only when a local version was known and the
   lookup was attempted, so it separates a real reuse from a full re-fetch.
   Assertion 4's tree diff is the only thing that proves the scope held across
   the whole filesystem rather than just the repair plan.

5. **Do not pass `--purge-after` / `--purge-before` for a repair run anyway.**
   There is nothing to delete under a one-path scope, and a run that deletes
   something is a run whose evidence you then have to reason about. Passing them
   is not *unsafe* — `plan::purge::classify_purge_path` tests the scope first and
   returns `Keep` for anything not visible, so a scoped cleanup cannot reach the
   rest of the fixture — but `purge_files=0` in the `PLAN` line above is the
   direct evidence that the cleanup had nothing to do.

**Result, all eight, exit `0`:**

| capture | `download_bytes` | expected | md5 | `must_not_fetch` |
|---|---|---|---|---|
| `identical` | 0 | 0 | ok | 1 chunk reused, 0 downloaded |
| `zero_reuse_single` | 21 | 21 | ok | — |
| `zero_reuse_multi` | 336 852 | 336 852 | ok | — |
| `partial_same_offset` | 274 426 | 274 426 | ok | 1 chunk reused, 0 downloaded |
| `partial_moved_offset` | 1 564 894 | 1 564 894 | ok | 8 chunks reused, 0 downloaded |
| `partial_size_changed` | 197 192 | 197 192 | ok | 9 chunks reused, 0 downloaded |
| `added` | 114 357 | 114 357 | ok | — |
| `removed` | 0 | 0 | *(gone)* | — |

**2 487 742 B spent in total**, which is the sum of the patchmaps' figures to the
byte. 19 reused chunks across the four captures that declare them, none of them
downloaded. Only the eight captures' own paths changed on the scratch copy — the
`added` file appeared, the `removed` file disappeared, and the other seven were
modified in place — so the scope held across all eight runs. The bundle itself
was never a working tree and still verifies 90/90.

### 8.1.2 `removed` needs a different run, and a different assertion

A `removed` path is absent from the new manifest by construction, therefore
absent from every repair plan. The only work that can touch it is the
files-cleanup, so it gets a **cleanup-only run** scoped to that one path — add
`--purge-only` — and its assertion is that the path is **gone afterwards** and
that nothing was downloaded. Not a byte total, because there is no patch to cost.

```
girpr --game-path <copy-of-game> --biz hk4e_global --purge-only
      --include "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/00664172.blk"
→ SUMMARY total=0 … download_bytes=0 deleted_extra_bytes=1731133 exit=0
      scoped=1 config_bumped=0 plan_files=0 local_manifest=ok
```

`plan_files=0` is correct and is not a failure: there is nothing to repair. The
two figures worth checking are `deleted_extra_bytes=1731133`, which is the
patchmap's `old.size`, and `download_bytes=0`. `expectations.absent_after: true`
says exactly this, so the artifact's contract and the run's output agree.

Only the `--include` glob is shared with the other seven; `--purge-only` is the
one flag that makes this capture different, and leaving it off would make the run
exit non-zero for the same reason a repair run would.

### 8.1.3 Do not pass `--audio` under a scope

A consumer reaching for `--audio en-us` — the natural reflex, and the flag this
capture was itself made with — is **refused**, not silently honoured. Verified by
hand against this bundle:

```
girpr --game-path <copy-of-game> --biz hk4e_global
      --include "GenshinImpact_Data/Plugins/HoYoChannel.dll" --audio en-us
→ exit 1: --audio writes the scan file at 'GenshinImpact_Data/Persistent/audio_lang_14',
  which is outside the active path scope; include it or drop one of the two flags
```

That path is this bundle's own audio scan file, which holds `English(US)`. The
refusal is right: an explicit `--audio` mutates a path the scope has declared
nonexistent, and the scan file is the one file a consumer is most likely to
invalidate by accident. Omitting the flag is also what makes the run a no-op here —
the run autodetects from the scan file and resolves to `en-us`, which is exactly
the state the bundle was captured in.

Three details worth knowing rather than rediscovering:

- **The escape hatch is to include the scan file.** Adding
  `--include "GenshinImpact_Data/Persistent/audio_lang_14"` makes the same run
  exit `0`. It is never hardcoded in `girpr`; the path comes from
  `getGameConfigs`. Including it does not add it to the repair plan — `files=1`
  either way — so this is a legitimate alternative if a run really does need to
  set audio.
- **The refusal is skipped on a read-only run.** `ctx.readonly()` is
  `dry_run || check_only`, and the guard is behind it, so `--check-only` or
  `--dry-run` can carry `--audio` under a scope without complaint. That is not
  inconsistency: neither writes the scan file, so there is no conflict to refuse.
- **The path is `/`-separated and case-insensitive**, like every other glob, and
  `*` also crosses `/` — which is why step 3 passes `--max-files 1`.

Expected per-capture `download_bytes` for that bundle:

| capture | class | `download_bytes` | reuse / fetch / moved | `must_not_fetch` |
|---|---|---|---|---|
| `identical` | `identical` | 0 | 1 / 0 / 0 | 1 |
| `zero_reuse_single` | `zero_reuse_single` | 21 | 0 / 1 / 0 | 0 |
| `zero_reuse_multi` | `zero_reuse_multi` | 336 852 | 0 / 2 / 0 | 0 |
| `partial_same_offset` | `partial_same_offset` | 274 426 | 1 / 2 / 0 | 1 |
| `partial_moved_offset` | `partial_moved_offset` | 1 564 894 | 8 / 2 / 4 | 8 |
| `partial_size_changed` | `partial_size_changed` | 197 192 | 9 / 1 / 0 | 9 |
| `added` | `added` | **114 357** | 0 / 1 / 0 | 0 |
| `removed` | `removed` | **0** *(cleanup run; `deleted_extra_bytes=1 731 133`)* | *(no plan)* | 0 |

`added` and `removed` are the two one-sided captures and they are not shaped like
the rest, so read them with §13.1 of the schema:

- **`added` has a `plan` and `totals` after all.** Every entry is `FETCH`, and
  `download_bytes` is **114 357** — the one chunk's compressed size. An earlier
  build emitted `plan: null`, `totals: null` and `download_bytes: 0` here, which
  was not "unrecorded" but *false*; see §13.2 of the schema.
- **`removed` has no `plan` and no `totals`** — there is no new side to patch to,
  so the reuse / fetch / moved cells above are empty rather than `0 / 0 / 0`.
  Its `download_bytes` is `0`, and that is a true value: a removal downloads
  nothing. The run that produces it is a cleanup run, and the size it reports is
  `deleted_extra_bytes` = `old.size`; see §8.1.2.

The six that carry `totals` sum to **2 373 385 B**; adding `added` makes it
**2 487 742 B**, which is exactly what the eight runs spent.

### 8.2 What the artifact provides so that the check is cheap to run

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