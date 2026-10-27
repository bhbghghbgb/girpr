# `gircap` — live-artifact capture for `girpr` testing

**Scope:** `tools/capture` only. This document and its siblings describe *this*
tool; they say nothing about `girpr`'s behaviour beyond what is needed to know
what a correct artifact looks like.
**Status:** implemented. What follows is specification, not a to-do list — where
this document and the code disagree, the code is right and this document is a
bug.

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
| No shared contracts, except the log policy | own exit codes, own report format, own flag set apart from the log flags (`--log-level`, `--log-file-level`, `--log-file`, `--no-log-file`) and their environment variables. The README is the only other shared vocabulary (version strings). The scratch-directory policy below is a test concern: it shares no flag, no exit code and no output of any program |
| No shared fixtures | `gircap` builds its own mock server for its own tests |
| Two shared *leaf* crates: the log policy and the scratch-directory policy | `girpr-log` (`crates/log/`): where the log file goes, what each sink's level is, which env var answers for which flag. `girpr-tempdir` (`crates/tempdir/`): how a test's scratch directory is named and when it is removed. Neither holds any logic of any program, and no test of one program depends on another's |

The intent is **do not couple**. Two tools that must stay independent should not
end up depending on each other in order to prove one of them works, and the
artifact is the whole interface between them: a bundle on disk, read by whoever
wants it. None of the above is mechanically enforced — nothing here checks it —
so it is a rule the tree has to keep, not a guarantee a build makes.

**The log policy is the one thing about a *run* that is shared, deliberately.** A
user who has learned `--log-file` for one of the three programs should not have
to learn it again for the next, and a log file that lands somewhere different
per tool is a log file nobody finds. The shared crate is a leaf: it depends on
`chrono` (for the timestamp in a file name) and on `tracing-subscriber` (for
the filter), on no program at all, and it cannot make one program's test depend
on another's. What stays independent is everything that could couple the
*meaning* of a run: the exit codes, the report format, the fixtures, and every
flag that is not about logging.

**The scratch-directory policy is shared for the same reason, and is a
dev-dependency.** A scratch directory is a thing for a test, not for a program:
the shared crate is how a test's tree is *named* (so two runs never share one)
and *removed* (so a failed case leaves nothing behind). Sharing it means the
three suites cannot drift apart on the one thing where a drift is invisible
until it eats another test's tree. Nothing that ships links against it, and it
depends on no program at all.

This mirrors `tools/sync`: *"Separate from the `girpr` binary; shares no flags,
no exit-code contract, no code."* — with the same exceptions, the log policy
and the scratch-directory policy, on the same grounds.

---

## 3. Live API surface used

All of the following were **measured** against `hk4e_global`. Anything in this
section marked *(measured)* is an observation, not an assumption, and `gircap`
records it in the artifact as evidence.

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

### 3.3a The two route families disagree about failure *(measured)*

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
that follows from `Key`), an explicit `error` flag, and a `retryable` flag
**derived from the tool's own retry policy** — so the policy is readable from the
artifact and not only from the fetcher. See
[`01-artifact-schema.md` §5a](01-artifact-schema.md).

### 3.4 Version pair *(measured)*

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
file is therefore reconstructible **today** without the legacy zip path, so the
legacy API is irrelevant to the capture. `getGamePackages` is captured anyway,
purely as dated evidence:

```
main.major.version = "5.5.0"   main.major.game_pkgs = 8   patches = ["5.4.0","5.3.0"]
```
— frozen in 2025, ~2 major versions stale.

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

The four sizes are `i64` **straight off the wire** and are treated as untrusted
everywhere: negative values are clamped to zero and arithmetic on them is
checked, so a manifest declaring an absurd size becomes a recorded finding rather
than an abort.

### 3.6 Manifest structure *(measured, 7.0.0 and 7.1.0)*

These hold for both `game` manifests and are recorded as assertions, not
assumptions — if a future version violates one, the capture still works and
`selfcheck.json` says which property broke:

- 0 `is_folder` entries, 0 empty paths, 0 backslash paths, 0 duplicate paths, 0 non-ASCII paths
- chunk offsets strictly ascending, gapless from 0, `Σ uncompressed_size == size` for every file
- largest chunk: **3 689 476 B** compressed (bounds the main tool's whole-chunk buffering)

A violation of any of these is **counted, never fatal**: the capture still
happens, the property lands in `meta.json → decoded.structure`, and A11 goes red
on the specific file that broke.

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

#### The case space: what the patch is, and what a class can force

A class is a statement about the **patch**: what a consumer has to download to
produce the new file, and what the file becomes. It is not a statement about
how a repair tool gets there.

The repair spec (2026-10-20, merge `f9e6f6a`) makes the repair **in place** by
default, in two tiers, with a trim that is sometimes eager and sometimes
deferred. Per file, it decides nine things.

| # | decision | what it depends on | tested where |
|---|---|---|---|
| D1 | **skip** — the file is already correct | the whole-file MD5 | `girpr`'s `tests/repair_machinery.rs` |
| D2 | **what happens to each chunk**: *kept* (already correct where it belongs), *copied* (its bytes live elsewhere in the file), *fetched* (no usable local source) | the old/new chunk layouts | `girpr`'s `tests/repair_machinery.rs` |
| D3 | **the tier** — RAM-staged (the moving bytes fit the budget) or ordered (they do not) | the payload size vs `--max-ram-bytes` | `girpr`'s `tests/repair_machinery.rs` |
| D4 | **the direction** — ascending or descending, whichever keeps more reuse | the *sign* of the moves | `girpr`'s `tests/repair_machinery.rs` |
| D5 | **the trim** — eager (the dead tail is released before the first write) or deferred (the tail waits for the end) | the old size vs the new size, and **where the sources are** | `girpr`'s `tests/repair_machinery.rs` |
| D6 | **the growth** — the file on disk is shorter than the target, so the in-place path has to extend it | the two sizes | `girpr`'s `tests/repair_machinery.rs` |
| D7 | **the mode** — in place / temp / discard, and the fallback when the target cannot be opened for writing | a flag, and the filesystem | `girpr`'s `tests/repair_machinery.rs` |
| D8 | **the resume** — in place re-verifies and continues; temp resumes by length | an interrupted run | `girpr`'s `tests/repair_machinery.rs` |
| D9 | **the zero-fill** of a hole in the chunk coverage | a manifest that does not tile its file | `girpr`'s `tests/repair_machinery.rs` |

**None of those decisions is shared with any other repair tool.** They are the
repair tool's own, and they are tested where they belong — on crafted
fixtures, where a 192-byte input states each of them exactly and a broken
branch fails the case that names it.

**So the class set is over the patch space, and only the patch space.** A class
is a statement about what the patch is, and the six classes are the patch
space's boundary conditions: **nothing** fetched, **everything** fetched, a
**subset** fetched with the reused part already in place, a **subset** fetched
with the reused part moved, a **new** file and a **deleted** file.

#### The classes

| class | predicate over the old/new manifest entries | the patch it covers | *(measured)* cheapest candidate, 7.0.0→7.1.0 | cost |
|---|---|---|---|---|
| `identical` | `old.md5 == new.md5` | nothing to fetch: the file is already right | `GenshinImpact_Data/Resources/browser_assets` | 54 B |
| `no_reuse` | `same_md5 == false`, `reused == 0`, `nchunks > 1` | everything fetched: the whole file is overwritten | `GenshinImpact_Data/Resources/unity default resources` | 634 KB |
| `reuse_in_place` | `same_md5 == false`, `0 < reused < nchunks`, `moved == 0` | a subset fetched: the reused chunks are already correct where they belong | `GenshinImpact_Data/Plugins/HoYoChannel.dll` | 1.83 MB |
| `reuse_moved` | `same_md5 == false`, `moved > 0` | a subset fetched: the reused chunks live elsewhere in the file and have to be copied | `…/StreamingAssets/AssetBundles/blocks/00/09801749.blk` | 22.7 MB |
| `added` | path only in the new manifest | the file is created whole | `…/Plugins/libHttpClient.GDK.dll` | 114 KB |
| `removed` | path only in the old manifest | the file is purged | `…/StreamingAssets/AssetBundles/blocks/00/00664172.blk` | 1.73 MB |

**The two reuse classes are exclusive, and the class set is a partition.** A
file that reuses *and* moves is the `reuse_moved` one, and a file that reuses
without moving is the `reuse_in_place` one — so one capture is one shape, and
the bundle is one file per shape.

**The `same_md5 == false` carve-out is what makes the partition true.** Every
content class requires it: a file whose whole-file MD5s agree is the **skip**
branch — the repair makes no content decision about it at all, so it cannot be
a statement about how the content changed. The live pool holds one such path
(`StreamingAssets/AudioAssets/Music4.pck`): the chunking was re-tiled, so four
chunks "move", yet the file's content is byte-identical. Without the carve-out
it would satisfy a content class as well as `identical`, and the class set would
not be a partition. **A reader implementing the table above must include it**,
or `Music4.pck` lands in `reuse_moved`.

**A one-chunk file with nothing reusable is not a class.** It is a degenerate
patch, and `added` already covers "the file is created whole".

*(measured)* population over the pool the capture selects (`game` ∪ `en-us`,
2 932 paths): `identical` 1 733, `no_reuse` 491, `reuse_in_place` 29,
`reuse_moved` 463, `added` 84, `removed` 11.

**The admission order is a decision.** The cheap shapes come first, so a small
budget buys the boundaries before the expensive ones:

```
identical, no_reuse, reuse_in_place, reuse_moved, added, removed
```

#### What the reference bundle exercises

*(measured)* against the reference bundle. The counts are the patch's own
vocabulary: how many chunks the new side has, how many of them the old file
already holds (**reused**), how many of the reused ones live elsewhere in the
file (**moved**), and how many have to come off the network (**fetched**).

| capture | chunks | reused | moved | fetched | download bytes | the two sizes |
|---|---|---|---|---|---|---|
| `identical` | 1 | 1 | 0 | 0 | 0 | 14 → 14 (the same) |
| `no_reuse` | 2 | 0 | 0 | 2 | 336 852 | 4 871 912 → 5 064 836 (**grows**) |
| `reuse_in_place` | 3 | 1 | 0 | 2 | 274 426 | 2 340 264 → 2 340 264 (the same) |
| `reuse_moved` | 10 | 8 | 4 | 2 | 1 564 894 | 11 914 387 → 11 912 007 (**shrinks**) |
| `added` | 1 | 0 | 0 | 1 | 114 357 | — → 249 256 (created) |
| `removed` | — | — | — | — | — | 1 731 133 → — (purged) |

**Every patch shape is here once.** `no_reuse` fetches the whole file;
`reuse_in_place` fetches a part of it and leaves the rest where it belongs;
`reuse_moved` fetches a part of it and copies the rest from elsewhere in the
file; `identical` fetches nothing; `added` creates a file; `removed` deletes
one.

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

#### The candidates, and what each class costs

*(measured)* over the pool the capture selects (`game` ∪ `en-us`, 2 932 paths).
The ranking key is `gircap`'s own: `total_cost` = the compressed bytes of the old
and the new file, because the old file has to be materialized in full for the
fixture to be real.

| class | candidates | cheapest candidate | `total_cost` | on disk |
|---|---|---|---|---|
| `identical` | 1 733 | `…/Resources/browser_assets` | 54 | 28 B |
| `no_reuse` | 491 | `…/Resources/unity default resources` | 633 588 | 9.9 MB |
| `reuse_in_place` | 29 | `…/Plugins/HoYoChannel.dll` | 1 826 067 | 4.7 MB |
| `reuse_moved` | 463 | `…/blocks/00/09801749.blk` | 22 662 068 | 23.8 MB |
| `added` | 84 | `…/Plugins/libHttpClient.GDK.dll` | 114 357 | 249 KB |
| `removed` | 11 | `…/blocks/00/00664172.blk` | 1 731 133 | 1.7 MB |

**`reuse_moved` is the expensive one.** Its cheapest candidate is 22.7 MB of
compressed bytes and 23.8 MB of fixture, so it is the last capture admitted
before the one-sided shapes. `gircap` has the mechanism for a capture that
cannot afford one: `--max-download-bytes` refuses a class it cannot pay for
and the matrix records it as `over_budget` with its reason. The reference bundle
is captured **without** a ceiling, so every class is funded; a capture with a
ceiling records the truth instead of pretending.

#### What the bundle cannot cover

Two things are out of the bundle's reach, and both are said here rather than
left to be discovered.

**The corruption-repair input state.** The new repair spec opens with two
regimes, and the first one is the common case: the file is the right size and
most chunks are intact, so a chunk's reuse source is at *its own offset* and the
bad chunk is simply overwritten where it lies. In a real corruption repair the
install is at the **current** version and the *bytes* are damaged; the bundle
cannot produce that state, because `game/` is always materialized from the
**old** manifest, so a chunk that is reused at its own offset is already
correct by construction.

What the bundle *does* cover is the **behaviour** that regime produces: the
payload is 0, the wrong chunks are written in place and the good ones are kept
(`reuse_in_place`). What it does not cover is the **input** — the case where the
local map has an entry for the damaged chunk and the source check is what
rejects it. A second fixture flavour, a "damaged current install" capture, is a
real feature and not a flag: the class model would need a second dimension and
the manifest diff would have to be scored against the damage rather than
against the old version. It is its own change, with its own cost.

**The malformed-manifest cases.** A hole in the chunk coverage, or an overlap,
cannot come from a real capture: the server's manifests tile their files. They
are only reachable with a synthetic manifest, which is a **bridge** fixture
rather than a bundle case.

Predicates overlap in general (a file can satisfy several). The admission step
is **greedy by cost with an exclusion set**:

```
for class in REQUIRED_ORDER:
    if any file an earlier class already wants satisfies class:
        record capture_id=<that capture's id>               # the capture that answers it
        record covered_by=<that capture's path>; continue    # and no second file
    pick the cheapest candidate satisfying class that is not already captured
    if none:                                      record status="unavailable" + reason
```

The second rule is what stops the bundle paying twice for one shape: a class
whose cheapest candidate an earlier class already took is **covered by** that
capture. The one edge case: a `covered_by` row whose covering capture was
refused by the budget becomes `over_budget`, because a class that cannot have
the covering capture cannot have a dearer one either.

Every captured file is tagged with **all** classes it satisfies, so
`candidates.json` + `MANIFEST.json` always answer "which variant does this file
cover, and why".

`--require-class` is validated and parsed **before any socket opens**: a typo is
a usage error at `exit 1` rather than a failure discovered after two manifests,
a diff and a full selection have been paid for. `--per-class` is floored to `1`
in the resolved plan rather than at each consumer, and `--jobs`, `--retries` and
`--timeout` are range-checked in the same place.

### 4.1a What `api/` carries, including the failures

`api/` holds the six response bodies **byte-exact**, the request log, and two
generated documents that are this tool's own measurements rather than the API's
words:

| document | what it is |
|---|---|
| `tag_probe.json` | which tags the API will actually serve |
| `error_shapes.json` | what the API does with a request it will not serve |

The second exists because **a bundle otherwise contains no not-found evidence at
all** — every request a capture makes succeeds, so a recorded bundle says nothing
about what a missing object looks like. A tool that replays a bundle would have
to invent that, and its "the API 404s things" would be unfounded.

The two route families disagree about failure, and the document records both
(measured against `hk4e_global`):

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
length that follows from `Key`; the JSON envelopes have none).

### 4.2 Chunk retention — three named sets

For a captured path `P` and version pair `(O, N)`:

| set | definition | retained on disk |
|---|---|---|
| `materialize_old` | every chunk of `O`'s entry for `P` | ✅ |
| `materialize_new` | every chunk of `N`'s entry for `P` | ✅ |
| `patch_fetch` | `materialize_new` minus chunks with a `(uncompressed_md5, uncompressed_size)` match in `materialize_old` | ✅ (subset of the above) |

**`materialize_old ∪ materialize_new` is closed.** Nothing in the artifact may
depend on a file or blob that was not retained. This is asserted during capture
and re-provable offline by `gircap materialize`, which rebuilds both files from
`chunks/` with no network and checks both manifest MD5s. There is no `--offline`
flag: `materialize` never opens a socket, so the flag would have asserted
something it cannot affect. (`gircap verify --artifact` is the read-only half and
is likewise offline by construction.)

Blobs are stored **content-addressed by chunk id**, once, at
`chunks/blobs/<chunk-id>.zst`, with `chunks/index.json` mapping every
`(tag, file, chunk)` triple to its blob, its URL, and its membership in each of
the three sets. A chunk reused by two captured files — or identical across the
two builds — is stored once and referenced twice. Each blob also records every
`(tag, build_id)` that declares it, newest first, so the old CDN is only asked if
the new one has gone; two builds declaring one id with *different* content is
recorded as a conflict rather than silently resolved.

**A blob that was fetched is kept even when it will not decode.** The store
streams every fetched body to disk, digests it on the way past and records the
row. If the bytes are not a zstd frame the row carries `decode_error` with the
real reason and the two decode-dependent `verified` keys false, instead of being
discarded. A truncated or mis-served response is the evidence this tool exists
to keep.

**A chunk that never arrived is a row too, and it says why.** A refused source, a
failed write or a panicked task leaves `stored_path` null and carries the fetch
failure in `decode_error`; the run counts that chunk **once** among the failed
fetches, so the summary and the index cannot disagree about how many chunks are
missing.

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

The prelude is written unconditionally, including under `--no-chunk-blobs`:
`config.ini` and the audio scan file come from `getGameConfigs` and `--audio`,
not from a chunk, so a metadata-only bundle still has a `game/` for A13 to read.

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
files. One comparator owns that chain, because a tie-break that drifts between
two spellings is a silent change of which file is admitted.

### Budget

`--max-download-bytes N` is a hard ceiling on `Σ total_cost` over admitted
files. Admission is **smallest-first**: files are sorted by `total_cost` asc and
taken until the budget is exhausted; the rest are recorded as
`status="over_budget"`. A single file that cannot fit is reported, not silently
swallowed — and because the run's own assertions decide the exit code, a
deliberate exclusion is a recorded status, not a failure.

The same flag also caps the chunk store's **projected wire cost** before a blob
is fetched (`Σ compressed_size` over the distinct chunks the plan would request).
That second ceiling is about bytes crossing the network, so it does not apply to
`--no-chunk-blobs`: the run fetches nothing, and refusing it would price a store
that is never written. The projection is still recorded.

---

## 6. Invariants asserted in `verify/selfcheck.json`

Each row is `{id, assertion, subject, expected, actual, ok}`.

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
| A17 | each required matrix class is `captured` \| `covered_by` \| `over_budget` \| `unavailable` - never absent |
| A18 | the bytes rebuilt from `chunks/` are byte-for-byte the bytes `game/` and `expected/` hold |
| A19 | each two-sided captured file differs between `game/` and `expected/` exactly when its class says it should |
| A20 | a patchmap's `totals` and `expectations` are the arithmetic of its own `plan` |
| A21 | the bundle covers both sides of the reuse boundary: at least one capture with nothing reusable and at least one with something |
| A22 | the bundle covers both sides of the moved boundary: at least one capture whose reused chunks are in place and at least one whose reused chunks moved |
| A23 | the bundle covers both sides of the size boundary: at least one capture whose file grows and at least one whose file shrinks |

There are **twenty-three** ids: seventeen from this spec (A1–A17), A19 and the
three coverage assertions (A21–A23) from a capture, and A18 and A20 from
`verify` only.

### A21, A22 and A23 — the coverage the class set exists to buy

The three boundaries of the patch space, and each one has two sides. A bundle
that happened to contain a file with nothing reusable would say nothing about
the other side of that boundary, and a reader would have to take the class names
on trust — which is exactly the trust this tool exists to replace with a
measurement. The three assertions read the patchmaps' own arithmetic — the
plan's counts and the two sides' sizes — and say whether the bundle covers
both sides of each boundary.

**An absent side is no file, not a file of zero bytes.** The old side of an
`added` capture is *no file* and the new side of a `removed` capture is *no
file*; neither is a file of zero bytes, and the difference matters: a file that
is **created** has grown and a file that is **deleted** has shrunk, whether it
is a hundred bytes or none at all. What changed is the file, not the byte
count. A file present on both sides grows or shrinks when its two sizes say
so.

**A capture with no plan is on neither side of the reuse boundary.** It has no
target, so it has nothing to reuse and nothing to fetch, and counting it as
"nothing reusable" would be a false claim about a capture that says nothing at
all.

**They are fatal in a capture and a finding in a `verify`,** and the difference
is the point. A capture's contract is "a bundle that covers the patch space", so
a capture that does not cover it fails its own run and names the assertion. A
bundle's *coverage* is a design property, not an integrity one: a narrow bundle
(a tight `--max-download-bytes`) or a hand-built one is still internally
consistent, so `verify` reports the shortfall as a **finding** and the bundle
still verifies. `verify`'s job is to prove the bundle still says what it said —
and "this bundle does not cover both sides of the reuse boundary" is a thing it
said.

**Which ids a `capture` records, and which fail the run.** `capture` records
**A1–A17 and A19** — A18 and A20 are statements about the bundle rather than
about the run, so no capture records them and a reader who sees either id in a
capture's `selfcheck.json` has been handed a `verify` report. **Every** recorded
id is fatal: a failure in any of them ends the run with exit `3`, because a
failure in A15–A17 (the documents this tool just wrote) means the bundle is
internally inconsistent, which is as untrustworthy as a wrong digest. A row that
was **skipped** rather than failed is a different verdict and exits `0` with
`result: "incomplete"`: a `--no-chunk-blobs` run cannot prove A6–A12, A19 or
A21–A23,
records those ids as skipped, and says so rather than claiming a pass. A caller
that cares must read `result`, not just the exit code.

**A17 is about the matrix being *complete*.** Every class has a row, and every
row says what happened to it — `captured`, `covered_by` (another capture answers
it), `over_budget` (a candidate existed and this run's budget would not pay for
it) or `unavailable` (no path in the pool satisfies it). None of those four is a
failure of the bundle: a matrix that records `over_budget` is *more* honest than
one that fails. The refusal is `--require-class`'s job, and a class the caller
named and did not get is exit `4`.

### A18 — and what `verify` does *not* re-derive

None of the seventeen asks whether `game/` is what the retained blobs say it is.
That is the bundle's central claim and the reason `materialize` exists, so it
gets an id of its own. It cannot be recorded by a capture, because it is a
statement about the bundle rather than about the run.

### A19 — the converse of A14

A14 asks one direction: is `game/` free of paths the tool under test would build
a plan against? It cannot see the other failure — a capture admitted for
`reuse_in_place` whose two variant files came out byte-identical. That
bundle would satisfy A14, satisfy A7/A8 (each side matches its own declared MD5),
and be useless, because the fixture would hold no evidence of the patch it was
captured for.

A19 is found by comparing the two trees. It skips one-sided captures (`added`,
`removed`) — their file is absent from one tree *by construction*, which is the
point of the class — and it is skipped under `--no-chunk-blobs` for the same
reason A6 is: with no assembled sides the check would pass vacuously, and a green
row about nothing is the exact failure mode this crate keeps having to fix.

### A20 — a patchmap must add up to itself

It re-sums `compressed_size` over the plan's `FETCH` entries, recounts
`reuse`/`fetch`/`moved`, and checks that `expectations.download_bytes` is that
same figure. `verify`-only, like A18. It exists because a patchmap can carry
`"plan": null`, `"totals": null` and `"expectations": { … "download_bytes": 0 }`
for an `added` capture and still be green — nothing else looks. §13 of the schema
records the rule that prevents it; A20 is what stops the rule being quietly
dropped.

### What `verify` re-derives, and what it does not

`gircap verify` re-derives **nine** of the twenty-one — A6, A7, A8, A11, A13,
A16, A17, A18 and A20. It does not re-derive A1–A5, A9, A10, A12, A14, A15, A19
or A21–A23: the first **eleven** are statements about the API bodies, the
manifests and the diff, and re-running them inside the bundle would be re-running
the capture's own algorithm against the capture's own output, which can only ever
agree; the last three are the bundle's **coverage**, which `verify` reports as
a finding rather than as a failed check (see A21 above).

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

### `--require-class` defaults to **nothing**

An empty `--require-class` is **not** "every required class". The implementation
requires nothing unless a class is named, so a capture whose matrix leaves a
class `unavailable` still exits `0`; the unavailable row is the finding, not the
exit code. That is a deliberate reversal of "all required classes", and the live
pair is why: requiring all six by default would make *every* run of a pair
that lacks one of them exit `4`. `unavailable` is a fact about the manifests;
`over_budget` is a fact about this run's flags; only a class the caller named
is fatal. A caller that needs the stronger rule asks for it:

```
gircap capture … --require-class identical --require-class added
```

and gets exit `4` when one of them is unmet.

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
  [--require-class <id>…]                     (fail if unmet; default: NONE)
  [--files <rel-path>…]                       (bypass selection; still scored + recorded)
  [--audio <code>…|none]                      (default en-us; sets the scan file AND
                                               the selection pool — see §4.1/§4.3)
  [--manifest-detail full|paths-only]         (default full)
  [--gzip-threshold <bytes>]                  (default 1048576)
  [--probe-tags <tag>…]                       (default: diff_tags ∪ {old,new}; JSON only)
  [--no-chunk-blobs]                          (metadata only; declares A6–A12,
                                             A19 and A21–A23 skipped)
  [--hyp-base <URL>]                          (testing seam; default: the
                                             production base for --biz)
  [--sophon-base <URL>]                       (testing seam; see --hyp-base)
  [--max-response-bytes <bytes>]              (default 67108864; one response
                                             body's ceiling)
  [--jobs N] [--retries N] [--timeout S] [--log-level LV] [--log-file-level LV] [--log-file P]
  [--no-log-file] [--output text|json]

gircap verify      --artifact <DIR>            # re-derive A6,A7,A8,A11,A13,A16,A17,A18,A20, no network
gircap materialize --artifact <DIR> [--out DIR] # rebuild old/new files from chunks/, no network
gircap list        --root <DIR>               # index every capture under a root
```

### Exit codes

| code | meaning |
|---|---|
| `0` | the run completed. **Read `result`**: `pass` means every asserted invariant is green, `incomplete` means rows were *skipped* (a `--no-chunk-blobs` bundle) rather than proven |
| `1` | usage / configuration error |
| `2` | network or API failure (incl. an unservable `--old-tag`, and a writer `io::Error`) |
| `3` | an assertion failed — the artifact is **not** trustworthy |
| `4` | a `--require-class` could not be satisfied |

Codes `3` and `4` are the two that leave a tree behind, and they are the two the
failure summary marks `artifact_trustworthy=false`. Exit `0` with `result:
incomplete` is not a pass: a `--no-chunk-blobs` capture provably cannot establish
A6–A12, A19 or A21–A23, so it exits `0` and records them as skipped; a harness
that treats "exited 0" as "every claim in this bundle is proven" will be wrong
about exactly the bundles that prove least.

**A class the budget refused is not a failure, and that is a change worth
stating.** `--max-download-bytes` can refuse a class the capture cannot afford;
the matrix records it as `over_budget` with its reason, and the run exits `0`.
A17 accepts that row, because a matrix that says "a candidate existed and this
run's budget would not pay for it" is *more* honest than one that fails — and
the information is not lost: the row's reason, the `SUMMARY` line's
`captures_over_budget=` and the `budget` block in `diff/matrix.json` all carry
it. The guard rail is `--require-class`: naming a class makes it fatal, and an
unmet one is exit `4`. Before this, A17 failed on `over_budget` and the run
exited `3`, which made a capture that had dropped most of its classes look
untrustworthy rather than incomplete. The row is a fact about the run; the
exit code is a verdict on the bundle.

### Output

stdout is a pure data plane: **records, one per line, the terminal one last.**
With `--output text` a record is `LABEL k=v k=v …`; with `--output json` it is one
object per line, NDJSON, no enclosing array. All narration goes to stderr via
`tracing` and to a JSON-lines log file. The only `println!` in the crate is the
record emitter.

**A release build always writes a log file; a debug build writes one only when
it is asked to.** A release build's default is
`<exe-dir>/logs/gircap_<timestamp>.log`; `--log-file` and `GIRCAP_LOG_FILE`
name one otherwise, in that order, and `--no-log-file` turns the file off in
any build. Its verbosity is `--log-file-level` (`debug` by
default), the console's is `--log-level` (`info` by default). `RUST_LOG` *adds*
to the console — it can turn up other crates, but it cannot change this
program's own level, because a flag is what the person running the tool typed.
The policy is the workspace's and lives in `crates/log/`, shared with the
repair binary and `girsync` — see §2.

**The console filter is scoped to `gircap`.** It was the bare level before the
policy was shared, so `reqwest`'s and `hyper`'s own `info` lines reached
stderr; they do not any more, and the log file is where a reader now looks for
them. The scope is what makes `--log-level` mean *this program's* verbosity
rather than the verbosity of everything it links.

**The last record is always the terminal one.** `list` writes one `bundle` record
per bundle it found and then its terminal record, so a machine can read every
per-bundle fact (path, tags, capture id, patchmap count, blob count,
`selfcheck`, `complete`) from stdout instead of parsing stderr narration, and can
read the stream until the end rather than expecting one line and parsing it.

A **failing** `capture` emits **two** records: first the run's own summary —
carrying the numbers it had already measured, `stage=artifact`, and
`exit_code=0` — and then the failure summary, carrying `result: "error"`,
`exit_code=3` and `artifact_trustworthy=false`. `stage` is what tells the two
apart: the failure record does not carry it. A harness that reads the *first*
terminal-looking line of a failing run will read `exit_code=0` off a run that
exited `3`, so it must read the **last** record; a harness that wants the run's
numbers rather than the verdict matches on `stage=`.

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
tell a reader nothing the label does not. Within one object the keys are **names**:
read them by name, never by position. Values are percent-escaped and the field
separator is a space, so a harness can `split(' ')` without quoting rules.

A directory under the root that does not open as a bundle is **not** a record.
It stays stderr-only: there is no bundle to describe, so a record would have to
carry the failure instead of the facts.

**The summary still counts the ones that are bundles.** A directory that is not
a bundle at all — a half-written capture — is stepped over, and a directory
that *is* a bundle this build refuses (an older schema, which is a permanent
condition with a known fix) is named on stderr with the reason and counted in
the summary's `bundles_unreadable`. An operator whose inventory quietly omits a
bundle has an inventory that is wrong.

---

## 8. Consuming the bundle with `girpr`

The bundle is built to be handed to the main tool.

| what you want to do | needs | status |
|---|---|---|
| inspect / assert from the metadata alone | nothing — `MANIFEST.json`, `patchmaps/`, `verify/` | ✅ works |
| rebuild old/new files offline | `gircap materialize` | ✅ works |
| **run `girpr --game-path <copy-of-game>` and check it patches to the new version** | **`girpr` path inclusion/exclusion** | ✅ **shipped** — `girpr --include <GLOB>` |

`girpr`'s README is the authority on what its flags guarantee — in particular
that a filtered-out path is treated as nonexistent and so is invisible to the
files-cleanup as well as to the repair, and that a scoped run does not bump
`config.ini`.

### 8.1 Why the scope is needed, and what it changed

`game/` holds a handful of files. Run the main tool against it unscoped and the
main tool does what it is specified to do: build the plan from the **whole live
manifest** and fetch every file in it. A run meant to repair one 2.3 MB DLL would
pull ~130 GB, and a `--purge-after` run would also delete every file in the
bundle that the live manifest happens not to contain.

That is correct behaviour for the main tool and not something `gircap` should
work around — which is why the fix belongs in `girpr` rather than shimmed here.
The invocation is:

```
girpr --game-path <copy-of-game> --biz hk4e_global
      --include "GenshinImpact_Data/Plugins/HoYoChannel.dll"
      …
```

and `SUMMARY download_bytes` can be compared against each capture's
`patchmaps/<id>.json → totals.download_bytes`.

**`gircap` does not depend on `girpr`.** It does not invoke it, does not read
its flags, and does not shim them; §2's independence rule holds and the grep over
`tools/capture/src/` returns doc comments only. The end-to-end check therefore
lives on the `girpr` side (`girpr`'s `tests/artifact_bridge.rs`), is `#[ignore]`d
because it needs a bundle and takes minutes, and is run by hand.

### 8.1.1 The procedure, and the numbers it must produce

Recorded so that whoever runs it does not have to reconstruct it, and so that a
result can be checked rather than believed. Every figure below is checked against
the bundle's **own** patchmap — nothing here is a constant to hard-code.

1. **Copy `game/` to scratch.** `girpr` writes into the game directory, so point
   `--game-path` at the copy and never at the bundle: that is what keeps a
   cleanup flag from turning into a way to destroy the evidence. Run every capture
   against the same copy — each touches exactly one path, so the runs accumulate
   and the reuse map stays anchored to the old version. Afterwards
   `gircap verify --artifact <bundle>` must still exit `0`.

2. **Pre-flight for free, before spending anything.** `--print-plan` does not
   repair, purge or bump, and it is not an offline mode — it still fetches the
   API — but it costs no payload:

   ```
   PLAN files=1 est_download_bytes=274426 purge_files=0 purge_bytes=0 scoped=1
   ```

   `est_download_bytes` is **`totals.download_bytes`**. `files=1` proves the glob
   selected the intended path and nothing else, and `purge_files=0` is the scope
   holding over the cleanup.

   `--check-only` is the other free check, and it **is expected to report
   damage**: under `--include <a capture path>` the run verifies only that one
   path against the *new* manifest while the copy still holds the *old* bytes,
   so the four repairable captures come back damaged and exit **`4`**. Only
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
   makes `download_bytes` attributable; a single run over all six sums to the
   same total but cannot be compared per capture. `removed` is the exception and
   takes `--purge-only` instead of `--max-files 1` (§8.1.2).

4. **Assert the cross-tool claim**, per capture — the three things, and the
   run's preconditions:

   | # | assertion | read from |
   |---|---|---|
   | 1 | the patched file's bytes are `expectations.result_md5`, and its length is `new.size` | the file on disk |
   | 2 | the chunks asked for are exactly the `plan[]` entries whose decision is `FETCH`, and **none** of the `REUSE` ones was asked for | **the replay server's own request log** — see below |
   | 3 | `SUMMARY download_bytes` equals `patchmaps/<id>.json → totals.download_bytes` | `SUMMARY` |
   | — | *precondition:* exit `0`, `local_manifest=ok`, `scoped=1`, `config_bumped=0` | `SUMMARY` |
   | — | for `removed`: the path is gone, and `deleted_extra_bytes` equals `old.size` | `SUMMARY` + the tree diff |

   **The normative source for assertion 2 is the server's.** The bridge runs a
   shim that records what it was asked for, and that record is the primary
   answer: it is the only one that sees every request, including any that no
   client-side log line describes.

   The procedure reads assertion 2 out of a log file at `--log-level trace` as
   the fallback for a reader with no shim — name one with `--log-file <path>`,
   which is written in any build. Its own caveat
   is the reason the server log is better: **a reused chunk is logged too**, as
   `reuse slice md5 ok -> copy chunk=<id>`, so a naive substring search for the id
   reports a violation on a perfectly correct reuse. Match on the *download*
   record, never on the bare id: `chunk downloaded chunk=<id>` and
   `no local reuse candidate -> download chunk=<id>`.

   `local_manifest=ok` is the precondition's teeth. Without it a run whose local
   manifest failed to load still exits `0` with a plausible byte count, having
   reused nothing; the key is emitted only when a local version was known and the
   lookup was attempted, so it separates a real reuse from a full re-fetch.

   **"Nothing outside the scope moved" is `girpr`'s own claim, not this
   bundle's**, and it is asserted where the tool is: on a crafted multi-file
   manifest, in `girpr`'s `tests/repair_machinery.rs`.

5. **Do not pass `--purge-after` / `--purge-before` for a repair run anyway.**
   There is nothing to delete under a one-path scope, and a run that deletes
   something is a run whose evidence you then have to reason about. Passing them
   is not *unsafe* — `plan::purge::classify_purge_path` tests the scope first and
   returns `Keep` for anything not visible, so a scoped cleanup cannot reach the
   rest of the fixture — but `purge_files=0` in the `PLAN` line above is the
   direct evidence that the cleanup had nothing to do.

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

Only the `--include` glob is shared with the other five; `--purge-only` is the
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

Expected per-capture figures, for the reference bundle. Compare each against the
artifact's own patchmap rather than against this table:

| capture | class | `download_bytes` | reuse / fetch / moved | `must_not_fetch` |
|---|---|---|---|---|
| `identical` | `identical` | 0 | 1 / 0 / 0 | 1 |
| `no_reuse` | `no_reuse` | 336 852 | 0 / 2 / 0 | 0 |
| `reuse_in_place` | `reuse_in_place` | 274 426 | 1 / 2 / 0 | 1 |
| `reuse_moved` | `reuse_moved` | 1 564 894 | 8 / 2 / 4 | 8 |
| `added` | `added` | **114 357** | 0 / 1 / 0 | 0 |
| `removed` | `removed` | **0** *(cleanup run; `deleted_extra_bytes=1 731 133`)* | *(no plan)* | 0 |

All six exit `0`. **2 290 529 B in total**, which is the sum of the patchmaps'
figures to the byte. Ten reused chunks across the three captures that declare
them, none of them downloaded. Only the six captures' own paths changed on the
scratch copy — the `added` file appeared, the `removed` file disappeared, the
`identical` one was left alone, and the other three were modified in place — so
the scope held across all six runs. The bundle itself was never a working tree
and still verifies.

**One capture, one class.** The class set is a partition on the content, so a
file answers exactly one class and the bundle holds six captures for six
classes.

`added` and `removed` are the two one-sided captures and they are not shaped like
the rest, so read them with §13.1 of the schema:

- **`added` has a `plan` and `totals` after all.** Every entry is `FETCH`, and
  `download_bytes` is **114 357** — the one chunk's compressed size. The figure
  is real, not a defaulted zero; see §13.2 of the schema.
- **`removed` has no `plan` and no `totals`** — there is no new side to patch to,
  so the reuse / fetch / moved cells above are empty rather than `0 / 0 / 0`.
  Its `download_bytes` is `0`, and that is a true value: a removal downloads
  nothing. The run that produces it is a cleanup run, and the size it reports is
  `deleted_extra_bytes` = `old.size`; see §8.1.2.

The four that carry a total of their own sum to **2 176 172 B**; adding `added`
makes it **2 290 529 B**.

### 8.2 What the artifact provides so that the check is cheap to run

- `expected/` gives the exact bytes a correct run must produce, so the
  comparison is `md5`, not eyeballing.
- `patchmaps/<id>.json → expectations` gives `download_bytes` (must equal),
  `result_md5` (must equal) and **`must_not_fetch`** (must not be requested) —
  the last one names the chunk ids a working reuse path skips, so a broken reuse
  path is a specific, nameable failure rather than a byte-count mismatch.
- `patchmaps/<id>.json → plan[]` says, per chunk, whether the bytes are already
  in the old file (`REUSE`) or have to come off the network (`FETCH`). The set
  of chunks a run asked for is exactly the `FETCH` set, so a run that fetched
  more than the patch says, or less, is nameable.
- `game/config.ini` already reads the old version, so the run takes the
  `getBuild(old)` reuse path without extra flags.
- `verify/` makes the fixture's own integrity a precondition, so a bundle that
  fails the check can be ruled out as the cause.

---

## 9. Decisions taken

| decision | value | where |
|---|---|---|
| workspace membership | **workspace member** (`members = [".", "tools/sync", "tools/capture"]`), matching `girsync` | [`02-plan.md`](02-plan.md) §0 |
| artifact root | **`artifacts/` in the repo, gitignored** — bundles are large and regenerable | [`02-plan.md`](02-plan.md) §0 |
| `--audio` default | **`en-us`** — launchable default, and it widens the selection pool | §4.3 |
| `--per-class` default | **1** | §4.1 |
| per-class variants | **no individual flags** — the matrix is the interface | §4.1 |
| `schema_version` | **stays `2`** unless the bundle's byte layout changes incompatibly | [`01-artifact-schema.md`](01-artifact-schema.md) |
| commit discipline | **one commit per change, each with its own gate result in the message** | [`02-plan.md`](02-plan.md) §0 |

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
