# Artifact schema — `gircap` bundle

Byte-level layout and every field of every machine-readable file.

**Every document below carries `"schema_version": 1`.** A reader should reject an
unknown `schema_version` rather than guess. JSON is UTF-8, LF, 2-space indent,
keys in the order written here. `*.ndjson` is newline-delimited, one object per
line, no enclosing array, so it streams.

**Required, not additive.** `verify` and `materialize` refuse a bundle that is
missing any document below. That is a deliberate choice: a bundle is a *fixture*
other work depends on, so a field that some bundles carry and others do not is a
trap for whoever reads it next. Where a field is genuinely optional within a
document, it is marked below and is optional *everywhere*.

**Compression rule.** A document is gzipped (`*.json.gz`) only when its
uncompressed body exceeds `--gzip-threshold` (default 1 MiB). The decision is
recorded in `MANIFEST.json → files[].encoding`, so a reader never has to guess.
In a real capture exactly the four full decoded `game` and `en-us` manifests trip
it; every gzipped document ships a plain-text sidecar (`*.tsv`) so an agent can
`grep` it without decompressing.

The worked values throughout are from the reference capture of `hk4e_global`
7.0.0 → 7.1.0: 324 files, 10 captures, 259 chunk blobs, 277 request rows, 354
selfcheck rows, `result: "pass"`.

---

## 1. Tree

```
artifacts/<biz>/<old_tag>__<new_tag>/<capture_id>/
├── MANIFEST.json
├── README.md
├── api/
│   ├── getGameConfigs.json
│   ├── getGameBranches.json
│   ├── getGameDeprecatedFileConfigs.json
│   ├── getGamePackages.json                  # legacy; captured as staleness evidence
│   ├── getBuild.<new_tag>.json
│   ├── getBuild.<old_tag>.json
│   ├── tag_probe.json
│   ├── error_shapes.json
│   └── requests.ndjson
├── manifests/<tag>/
│   ├── <field>.manifest.zst                  # raw blob, byte-exact
│   ├── <field>.manifest.meta.json
│   ├── <field>.manifest.json.gz              # full decode (or .json if small)
│   └── <field>.paths.tsv                     # always plain; path \t size \t md5 \t nchunks \t is_folder
├── diff/
│   ├── paths.diff.json
│   ├── matrix.json                            # one row per matrix class
│   ├── paths.tsv                             # field \t path \t state \t old_size \t new_size \t reused \t moved \t patch_bytes \t total_cost
│   ├── candidates.json
│   └── reuse_pool.json
├── game/                                     # ★ the fixture
│   ├── config.ini
│   ├── GenshinImpact_Data/Persistent/audio_lang_14
│   └── <capture paths, OLD-version bytes>
├── expected/                                 # ★ ground truth
│   └── <capture paths, NEW-version bytes>
├── chunks/
│   ├── blobs/<chunk_id>.zst                  # one per distinct chunk id
│   └── index.json
├── patchmaps/
│   └── <capture_id>.json
└── verify/
    ├── checksums.json
    └── selfcheck.json
```

`capture_id` is `<class>` or `<class>+<n>` (e.g. `kept_in_place`,
`moved_ascending+2`). It appears in `patchmaps/` and in `MANIFEST.json`.

Nothing under `chunks/` or `manifests/` is ever deleted by the tool.

**Atomicity is not uniform, and this is deliberate.** Chunk blobs are the only
artifacts written through a `tmp/` file that is then renamed into place, and a
blob's `stored_path` points at a name that does not exist until the rename
happens. That step is what lets `verify` trust a blob: a crash mid-write would
otherwise leave a truncated file at the final path, which nothing could tell from
a good one. The metadata documents, the assembled `game/` and `expected/` files
and `manifests/*` are written with a plain create-and-write: each is small enough
that a torn write is a small, reproducible failure, and routing them through the
same dance would only hide *which* step failed. A reader must therefore not
assume any document in a bundle was renamed into position.

---

## 2. `MANIFEST.json` — index

```jsonc
{
  "schema_version": 1,
  "tool":   { "name": "gircap", "version": "0.1.0" },
  "capture": {
    "id": "20261020T154810Z",
    "captured_at": "2026-10-20T15:48:10Z",
    "host_os": "windows",
    "tool_argv": ["target\\release\\gircap.exe", "capture", "--biz", "hk4e_global", …],
    "complete": true            // false only if interrupted; selfcheck then explains why
  },
  "biz":      { "id": "hk4e_global", "launcher_id": "VYTpXlbWo8", "game_id": "gopR6Cufr3",
                "channel": "1", "sub_channel": "0", "cps": "hyp_hoyoverse",
                "hyp_base": "…", "sophon_base": "…" },
  "versions": {
    "new": { "tag": "7.1.0", "build_id": "sMXGW2ll3Fuu", "source": "main.tag" },
    "old": { "tag": "7.0.0", "build_id": "3XnPviMfoILi", "source": "diff_tags[0]" },
    "diff_tags": ["7.0.0", "6.7.0"]
  },
  "field": "game",                       // matching_field the captures were drawn from
  "audio": ["en-us"],                    // codes written into game/…/audio_lang_14
  "gzip_threshold": 1048576,             // --gzip-threshold this run used
  "selection_pool": {                    // matching_fields scored and diffed
    "fields": ["game", "en-us"],
    "manifest_entries": { "game": 2744, "en-us": 177 },
    "note": "--audio widens this pool; captures may come from any listed field"
  },
  "totals": {
    "files_captured": 10,
    "chunks_blobs": 259,
    "bytes_downloaded": 250418224,
    "bytes_on_disk": 802823771
  },
  "matrix": [                            // one row per class, in admission order
    { "class": "identical", "status": "captured", "capture_id": "identical",
      "path": "GenshinImpact_Data/Resources/browser_assets", "matching_field": "game",
      "candidates_considered": 1733 },
    { "class": "trim_deferred", "status": "covered_by",
      "capture_id": "moved_ascending",
      "covered_by": "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk",
      "also_satisfies": ["trim_deferred"],
      "reason": "the cheapest candidate is captured for another class",
      "candidates_considered": 282 }
  ],
  "files": [                             // every file under the artifact root
    { "path": "game/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": 2340264, "md5": "1e21…", "sha256": "…", "encoding": "identity" }
  ],
  "totals_by_set": { "materialize_old": 29, "materialize_new": 28, "patch_fetch": 9 }
}
```

`matrix[].status` ∈ `captured` | `covered_by` | `over_budget` | `unavailable`.
A class is never simply absent (assertion A17). All four states are acceptable
to A17: the assertion is about the matrix being *complete* — every class has a
row and every row says what happened to it — and `over_budget` ("a candidate
existed and this run's budget would not pay for it") is a complete answer. The
refusal is `--require-class`'s job. The row's optional keys
(`capture_id`, `path`, `matching_field`, `covered_by`, `also_satisfies`,
`reason`, `candidates_considered`) are omitted rather than written `null`.

`files[]` lists every file that existed **before the close-out began** — the
fixture, the blobs, the manifests, the diff, the patchmaps and `api/`. The four
documents the close-out itself writes are absent, because the list is taken
before they are written: `MANIFEST.json` (it is being written), `README.md` and
`verify/selfcheck.json` (both render the manifest), and `verify/checksums.json`
(its own). A document that hashes itself has no fixed point, and here the
dependency runs both ways.

`verify/checksums.json` is the complete inventory: `file_count: 323` for a
324-file bundle, and it covers all four.

`totals.bytes_on_disk` sums `files[].bytes`, so for a gzipped document it counts
the **decompressed** length — the same rule as §15. It is therefore larger than
what `du` reports (≈803 MB against ≈730 MB on disk for the reference bundle);
that is the correct answer, because it is the number a reader would need to hold
the document in memory.

`gzip_threshold` is **required**, like every other field in this document. It
used to default on read, so a bundle with no recorded threshold would read back
as having used the flag's default — a claim about a run nobody wrote down, on the
one field whose purpose is to say why one document is gzipped and the next is
not. A field that defaults to a constant is worse than one that is required,
because the default is indistinguishable from the real value.

---

## 3. `api/*.json`

Byte-exact response bodies, unmodified — not re-serialized. A reader must be
able to `diff` a captured file against a fresh request. `api/requests.ndjson`
records how they were obtained.

The two exceptions are generated, and both are this tool's own measurements
rather than the API's words: `tag_probe.json` (§5) and `error_shapes.json` (§5a).

## 4. `api/requests.ndjson`

One object per HTTP request, in `seq` order. In the reference capture: 277 rows —
6 `api`, 4 `manifest`, 259 `chunk`, 8 `probe`.

```jsonc
{ "seq": 1, "ts": "2026-10-17T12:33:35.971Z", "kind": "api",     // api|manifest|chunk|probe
  "method": "GET", "url": "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs?…",
  "range": null,                        // literal Range header sent, if any
  "status": 200, "final_url": "https://…", "redirected": false,
  "elapsed_ms": 338,
  "req_headers":  { "accept": "*/*", "accept-encoding": "identity",
                    "user-agent": "UnityPlayer/2019.4.40f1 (UnityWebRequest/1.0, libcurl/7.80.0-DEV)" },
  "resp_headers": { "connection": "keep-alive", "content-length": "1529",
                    "content-type": "application/json",
                    "date": "Sat, 17 Oct 2026 12:33:29 GMT", "vary": "Accept-Encoding",
                    "via": "1.1 …cloudfront.net (CloudFront)",
                    "x-cache": "Miss from cloudfront", "x-amz-cf-pop": "HKG61-P1", … },
  "bytes_in": 1529,
  "stored_as": "api/getGameConfigs.json",
  "attempts": 1 }
```

Header keys are lowercased; values are strings exactly as received. `kind` lets a
reader ask "what did the manifest fetch cost?" without string-matching URLs.

`kind` is a closed set — `api`, `manifest`, `chunk`, `probe` — and two of the
probes' paths share `probe` while differing in retry behaviour. A `getBuild?tag=`
tag probe goes through the retry policy and records a real `attempts`; the
negative probes behind `api/error_shapes.json` are sent **once** and always record
`attempts: 1`. On a healthy capture every probe row shows `1`, so the row alone
does not say which path produced it; the URL does. The set is deliberately not
widened to distinguish them: `kind` is a published vocabulary and a reader
matching on `probe` must not be broken by it.

`req_headers` is what the tool **sent**, not what it wishes it had sent, and the
record and the client are built from one list so they cannot disagree.
`accept-encoding` is `identity` because that is the honest value: the HTTP client
has no decompression support enabled, and naming `gzip` on the wire would make the
CDN answer with a compressed body the tool could not decode — every manifest
digest in the bundle would become a digest of compressed JSON.

`bytes_in` is the number of body bytes the run actually transferred, **on every
row including the non-2xx ones**. It is the only way to price a request without
re-downloading it, so a throttled or errored attempt is counted like any other.

`stored_as` is the path, relative to the artifact root, the body was written to,
or `null` when it was not kept. Per kind:

| kind | `stored_as` |
|---|---|
| `api` | `api/<endpoint>.json` — the body was written |
| `manifest` | `manifests/<tag>/<field>.manifest.zst` |
| `chunk` | `chunks/blobs/<chunk_id>.zst` |
| `probe` | `null` — a probe response is deliberately not stored, and that is what makes it a probe |

## 5. `api/tag_probe.json`

Which tags the API actually serves, with no blob downloads.

```jsonc
{ "schema_version": 1,
  "requested": ["7.1.0", "7.0.0", "6.7.0"],
  "results": [
    { "tag": "7.0.0", "in_diff_tags": true, "http": 200, "retcode": 0,
      "build_id": "3XnPviMfoILi", "tag_echoed": "7.0.0", "served": true },
    { "tag": "0.0.1", "in_diff_tags": false, "http": 200, "retcode": -202,
      "build_id": null, "tag_echoed": null, "served": false }
  ],
  "summary": { "requested": 3, "served": 3, "not_in_diff_tags_but_served": 1 } }
```

`served` is the load-bearing fact: `diff_tags` is a patch-support list, not a
retention list (spec §3.4), so a capture may pin a pair the API still serves but
`diff_tags` no longer names.

---

## 5a. `api/error_shapes.json`

What the API does with a request it will **not** serve, with no blob downloads.
Seven shapes on every capture.

The two route families disagree about what failure looks like, and a consumer
that replays a bundle has to reproduce both. Without this document it would have
to invent them, and its claim that "the API 404s things" would be unfounded —
a bundle recorded only successes contains no not-found evidence at all.

```jsonc
{ "schema_version": 1,
  "shapes": [
    { "id": "chunk_missing", "family": "blob",
      "request": "a chunk id under the new build's prefix that the build does not have",
      "url": "https://autopatchhk.yuanshen.com/…/chunks/cxhpq4g4rgg0/sMXGW2ll3Fuu/000…",
      "status": 404, "content_type": "application/xml", "retcode": null,
      "error": true,
      "body": "<?xml version=\"1.0\" …><Error><Code>NoSuchKey</Code>…</Error>",
      "volatile_fields": ["RequestId", "HostId", "Key", "body length (follows Key)"],
      "retryable": false },
    { "id": "getgameconfigs_bad_game_id", "family": "json",
      "request": "getGameConfigs with a game_ids[] the API does not know",
      "url": "https://sg-hyp-api.hoyoverse.com/…/getGameConfigs?…",
      "status": 200, "content_type": "application/json", "retcode": -1001,
      "error": true,
      "body": "{\"data\":null,\"message\":\"invalid params\",\"retcode\":-1001}",
      "volatile_fields": [], "retryable": false },
    { "id": "getgamebranches_bad_game_id", "family": "json",
      "request": "getGameBranches with a game_ids[] the API does not know …",
      "status": 200, "content_type": "application/json", "retcode": 0,
      "error": false,
      "body": "{\"data\":{\"game_branches\":[]},\"message\":\"OK\",\"retcode\":0}",
      "volatile_fields": [], "retryable": false }
  ] }
```

| field | meaning |
|---|---|
| `id` | stable; **assert on this**, never on `url`, which carries a build id and changes every capture |
| `family` | `blob` (404) or `json` (200 + `retcode`) |
| `status` / `content_type` / `retcode` | exactly as received; `retcode` is `null` for a blob |
| `error` | whether the origin treats this as a failure — see below |
| `body` | byte-exact |
| `volatile_fields` | names that change per request, so the body **cannot** be replayed byte-for-byte |
| `retryable` | **derived from the tool's own retry policy**: "would the fetcher have retried this status?" |

The seven ids are `blob_missing`, `chunk_missing`, `blob_wrong_build`,
`getbuild_unknown_tag`, `getbuild_bad_package`, `getgameconfigs_bad_game_id`,
`getgamebranches_bad_game_id`.

`retryable` states the **policy**, not what this run did: the negative probes go
out through `Fetcher::observe`, which never retries, so a `true` here means "the
fetcher would have retried this" and not "this was retried". On a healthy
capture every row is `false`, because every recorded shape answers `200` or
`404` and neither is in the retry set — a `429` or `503` is what separates a
stated policy from a constant.

**`error: false` is not a bug.** `getGameBranches` answers an unknown game id
with `retcode 0` and an empty `game_branches`. That is a **success with nothing
in it**, not a failure, and it is recorded here precisely so a reader cannot
infer a failure from the shape of the other rows.

**Replaying.** `girpr` calls `.error_for_status()` and never reads the body, so
**`status` and `content_type` are the contract**; the body is fidelity, for a
human reading a replay server's log. For the blob rows the `volatile_fields`
mean a replayer must synthesize `RequestId`, `HostId` and `Key` — and therefore
the body length — and nothing else. The JSON rows have no volatile fields, so
they replay exactly.

**The document is required.** `verify` and `materialize` refuse a bundle without
it, because a bundle recorded only successes contains no not-found evidence at
all: a consumer that wanted to replay one would have to invent what a `404` looks
like, and its "the API 404s things" would be unfounded. Making the document
optional would have meant "this bundle can tell you how the API fails" is true of
some bundles and not others, with nothing in the bundle to say which.

It is read back through the same type that wrote it, so a document that is
present and unusable fails at open, by name, rather than at some later consumer.

---

## 6. `manifests/<tag>/<field>.manifest.meta.json`

Everything known about one archive blob.

```jsonc
{ "schema_version": 1,
  "tag": "7.1.0", "build_id": "sMXGW2ll3Fuu", "matching_field": "game",
  "category_id": "10016", "category_name": "…",   // the API's own name for the category
  "manifest": { "id": "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d",
                "checksum": "8d4dfb34d2ee2cf64aae45a9b1ecf58d",
                "compressed_size": 9007187, "uncompressed_size": 16827424 },
  "declared": {                                  // fields the API sends that most clients drop
    "stats": { "file_count": 2744, "chunk_count": 113610,
               "compressed_size": 128310073410, "uncompressed_size": 131981846376 },
    "deduplicated_stats": { "file_count": 2744, "chunk_count": 113441,
                            "compressed_size": 128174445744,
                            "uncompressed_size": 131776619976 },
    "chunk_download":   { "url_prefix": "…", "url_suffix": "", "compression": 1,
                          "encryption": 0, "password": "" },
    "manifest_download":{ "url_prefix": "…", "url_suffix": "", "compression": 1,
                          "encryption": 0, "password": "" } },
  "id_parts": { "prefix_16hex": "671e1a92a6cf53ff", "prefix_semantics": "opaque",
                "suffix_32hex": "8d4dfb34d2ee2cf64aae45a9b1ecf58d",
                "suffix_equals_checksum": true },
  "stored": {
    "path": "manifests/7.1.0/game.manifest.zst",
    "bytes": 9007187, "container": "zstd",
    "md5_compressed": "313ea8c56f184ed57c778e442d4fc039",
    "sha256_compressed": "…",
    "md5_decompressed": "8d4dfb34d2ee2cf64aae45a9b1ecf58d",
    "bytes_decompressed": 16827424, "encoding": "identity"
  },
  "verified": { "decompressed_md5_matches_checksum": true,
                "checksum_matches_id_suffix": true,
                "id_prefix_verified": false },
  "decoded": { "entries": 2744, "files": 2744, "folders": 0,
               "total_chunks": 113610, "max_chunk_compressed_bytes": 3689476,
               "total_file_bytes": 131981846376,
               "structure": { "duplicate_paths": 0, "empty_paths": 0,
                              "backslash_paths": 0, "non_ascii_paths": 0,
                              "unsorted_chunk_offsets": 0, "non_gapless_files": 0,
                              "size_mismatch_files": 0 } },
  "full_decode_written": true
}
```

`verified.id_prefix_verified` is **always false** and is expected to be. The
16-hex prefix is not a hash of any observable input (spec §3.7); the field exists
so the artifact states that finding rather than implying the prefix went
unchecked.

`decoded.structure` counts the six structural properties of spec §3.6. They are
recorded, never fatal: a violation still produces a capture, and A11 goes red on
the file that broke.

`deduplicated_stats` is the API's own second, de-duplicated view of the same
numbers, kept because it is the field a client would otherwise drop; `gircap`
asserts nothing against it.

## 7. `manifests/<tag>/<field>.manifest.json.gz` — full decode

The whole protobuf, verbatim, so no consumer has to re-download to inspect it.
`full_decode_written` in `meta.json` says whether this document was produced
(`false` under `--manifest-detail paths-only`).

```jsonc
{
  "schema_version": 1, "tag": "7.1.0", "build_id": "sMXGW2ll3Fuu",
  "matching_field": "game", "entries": [
    { "file": "GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "is_folder": false, "size": 2340264, "md5": "d992a789…",
      "chunks": [
        { "id": "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3",
          "uncompressed_md5": "de1cfe69…", "offset": 0,
          "compressed_size": 259795, "uncompressed_size": 690719,
          "unknown_field6": 8640275792690937856,          // opaque, preserved verbatim
          "compressed_md5": "06ba73bb…" } ]
    }
  ]
}
```

The document is written **streamed through the gzip encoder**, not built as one
string and compressed afterwards, because the full `game` decode is the largest
thing the tool holds. The compressed bytes are identical either way, which is
why `verify/checksums.json` records the *decompressed* digest and a `.gz`
document's identity does not depend on how it was produced.

`unknown_field6` is the wire's tag 6. Its meaning is unknown; it is preserved
because the upstream schema carries it and dropping it would make this dump a
lossy transform. *(measured)* it is stable across the two builds for the same
logical chunk even when content differs, which is exactly why it must not be
guessed at.

## 8. `manifests/<tag>/<field>.paths.tsv`

Always plain text, tab-separated, header line, `\n` endings:

```
path	size	md5	nchunks	is_folder
GenshinImpact.exe	444260776	f69da59bdc366df20bd2b598700e9370	325	false
```

This is the sidecar that makes the gzipped JSON greppable without decompression.

---

## 9. `diff/paths.diff.json`

```jsonc
{ "schema_version": 1,
  "old_tag": "7.0.0", "new_tag": "7.1.0",
  "fields": ["game", "en-us"],
  "counts": { "common": 2837, "identical": 1733, "changed": 1104,
              "added": 84, "removed": 11, "folders": 0 },
  "byte_counts": { "added": 6172743354, "removed": 263761287, "patch": 14173762284 },
  "added":   ["game:GenshinImpact_Data/Plugins/libHttpClient.GDK.dll", …],
  "removed": ["game:GenshinImpact_Data/…/00664172.blk", …],
  "by_field": { "game": { "common": 2662, "identical": 1592, "changed": 1070,
                          "added": 82, "removed": 11 },
                "en-us": { "common": 175, "identical": 141, "changed": 34,
                           "added": 2, "removed": 0 } },
  "new_chunks_reusable_at_all": 110850 }
```

Counts at the top level are aggregated over `fields`; `by_field` is the
breakdown, and every `added`/`removed` entry is prefixed with its field so a path
from two pools cannot collide. The `game` row is the **measured** one for
7.0.0→7.1.0; the `en-us` row depends on which audio languages `--audio` selected.

`new_chunks_reusable_at_all` is how many of the new manifest's chunks have an MD5
present **anywhere** in the old manifest — the number the same-file-only reuse
simplification in §10 of the spec gives up (see `reuse_pool.json`).

`diff/paths.tsv` is the flat per-path form and leads with the `field`, because the
same relative path can appear in more than one pool:

```
field	path	state	old_size	new_size	reused	moved	patch_bytes	total_cost
game	GenshinImpact.exe	changed	430973352	444260776	0	0	179241379	353260299
```

## 10. `diff/candidates.json`

The scoring table, for **every** common path, not just the winners — 2 837 rows
for the reference pool. Lets a later run ask "why did it pick that?"

```jsonc
{ "schema_version": 1,
  "columns": ["path","matching_field","old_size","new_size","nchunks","reused","moved",
              "moved_up","moved_down","old_max_end","same_md5","old_bytes","new_bytes",
              "patch_bytes","total_cost","classes","rank"],
  "rows": [
    { "path": "…/HoYoChannel.dll", "matching_field": "game",
      "old_size": 2340264, "new_size": 2340264,
      "nchunks": 3, "reused": 1, "moved": 0, "moved_up": 0, "moved_down": 0,
      "old_max_end": 2340264, "same_md5": false,
      "old_bytes": 913027, "new_bytes": 913040, "patch_bytes": 274426,
      "total_cost": 1826067, "classes": ["kept_in_place"], "rank": 1 }
  ] }
```

Column-oriented so a reader can sort without re-parsing objects.

`matching_field` names the pool the path was scored from (`game`, or an audio
code). It is a row column because the same relative path can appear in more than
one pool, and without it a reader summing `total_cost` across rows would double
count. `rank` is 1-based, in `total_cost` order.

`moved_up` and `moved_down` split `moved` by **direction**: `moved_up` counts
the chunks whose source is *before* their destination (the bytes move later,
and the descending write order keeps them alive), `moved_down` counts the ones
whose source is *after* it (the bytes move earlier, and the ascending order
keeps them alive). `moved` is their sum. The split is what makes the write
direction a *measurement* rather than something a reader has to re-derive.

`old_max_end` is the largest end (`source offset + size`) over the file's reuse
sources. It is the **trim test's input**: the repair releases the dead tail
before the first write only when no source reaches past the target's end, and
the tool under test asks that same question of that same number
(`repair::file::trim_is_free`).

## 11. `diff/reuse_pool.json`

Quantifies same-file-only reuse. Recorded because it bounds a deliberate
simplification in the main tool; `gircap` never acts on it.

```jsonc
{ "schema_version": 1,
  "new_chunks_total": 113610,
  "new_chunks_md5_present_in_old_manifest": 98910,
  "files_with_a_cross_file_only_chunk": 2,
  "cross_file_only_chunks": 2,
  "cross_file_only_bytes_forgone": 1776356,
  "note": "same-file-only reuse is a deliberate simplification; this quantifies its cost" }
```

---

## 12. `chunks/index.json` and `chunks/blobs/`

One blob per distinct `chunk_id`, byte-exact (zstd as served), stored once even
when several files or both builds reference it. 259 blobs for the reference
capture.

```jsonc
{ "schema_version": 1,
  "blobs": [
    { "chunk_id": "096e224c175242e1_60215d50041d92c36dab0fa5dd7609b4",
      "stored_path": "chunks/blobs/096e224c175242e1_60215d50041d92c36dab0fa5dd7609b4.zst",
      "bytes": 519810, "container": "zstd",
      "md5_compressed": "a6b9a872edc05505899c7c60bcdbf406",
      "md5_decompressed": "60215d50041d92c36dab0fa5dd7609b4",
      "sha256_compressed": "5694eef1b3f2104f98ca509168f90608a16b7a4db8e0229d4463b32d01f346cf",
      "uncompressed_size": 519785,
      "unknown_field6": 4233562806722494464,
      "verified": { "raw_size_matches": true, "md5_compressed_matches": true,
                    "md5_decompressed_matches": true, "decoded_size_matches": true },
      "decode_error": null,
      "referenced_by": [
        { "tag": "7.0.0", "build_id": "3XnPviMfoILi",
          "capture_id": "moved_ascending",
          "path": "…/blocks/00/09801749.blk", "role": "materialize_old", "offset": 5502333 }
      ],
      "sets": ["materialize_old"] }
  ],
  "sets": {
    "materialize_old": [ … chunk ids … ], "materialize_new": [ … ],
    "patch_fetch":     [ … ]
  } }
```

`role` ∈ `materialize_old` | `materialize_new` | `patch_fetch`. `patch_fetch` is
always a subset of `materialize_new` for the same file (assertion A12).

### `verified` — four keys, not three

`verified` carries the **four** A6 checks, and the names are precise:

| key | claim |
|---|---|
| `raw_size_matches` | `len(raw) == compressed_size` — the blob arrived whole |
| `md5_compressed_matches` | `md5(raw) == compressed_md5` — it arrived *intact*, not merely complete |
| `md5_decompressed_matches` | `md5(zstd(raw)) == uncompressed_md5` — it is the chunk the manifest named |
| `decoded_size_matches` | `len(decoded) == uncompressed_size` |

There is **no `size_matches` key**; the two size claims are named separately
because they are separate facts about different byte counts (the frame's length
as served, and the length after decoding).

### `decode_error` — a fetched blob is evidence, not a failure

`decode_error` is `null` whenever the blob measured cleanly, and otherwise
carries the real reason: the byte count, the first magic bytes actually seen, and
the compression the build declared. **The blob is still written and still listed.**
A truncated or mis-served response is the evidence this tool exists to keep, so it
is recorded with its bytes and a truthful `decode_error` rather than discarded and
backfilled with a "the fetch failed" story — with the two decode-dependent
`verified` keys `false` and the two that need no decode still decided.

It is `Option<String>`, so a reader must treat it as optional: a blob that was
never fetched at all has a null `stored_path`, a null-ish row body and no
`verified` verdict.

A `stored_path` that is `null` has three causes that cannot all be described the
same way — a source that refused, a write that failed, and a fetch task that
panicked mid-measure — and the row records which.

### `build_id` — part of the chunk's address

A chunk's URL is `…/chunks/<series>/<build_id>/<16hex>_<32hex>`, so the build id
is **where the bytes live**, not a label on them. The same chunk id under the
other build's prefix answers `404` with an `application/xml` `NoSuchKey` body
(measured against `autopatchhk.yuanshen.com`, `Server: AliyunOSS`) — see
[`00-spec.md` §3.7](00-spec.md). `tag` does not determine `build_id`: two tags can
map to different build ids over time, so a consumer that has to build a chunk URL,
or decide whether an observed request was namespaced correctly, cannot derive it
from the tag.

`referenced_by` is the **only** thing `gircap materialize` consults: the chunk
list of a side is rebuilt from `referenced_by`'s `offset`s and each blob's
`uncompressed_size`, and the retained manifests are not read at all. That is
deliberate — a rebuild that consulted the manifests would only prove they are
still readable, whereas rebuilding from the closure set proves the thing the
bundle actually claims, which is that the blobs alone are enough. A bundle whose
manifests survived but whose index is incomplete fails here, and should.

---

## 13. `patchmaps/<capture_id>.json` — the core

Per target chunk: where its bytes come from.

```jsonc
{ "schema_version": 1,
  "capture_id": "kept_in_place",
  "path": "GenshinImpact_Data/Plugins/HoYoChannel.dll",
  "matching_field": "game",
  "classes": ["kept_in_place"],
  "old": { "tag": "7.0.0", "size": 2340264, "md5": "1e210ff1…", "nchunks": 3 },
  "new": { "tag": "7.1.0", "size": 2340264, "md5": "d992a789…", "nchunks": 3 },
  "plan": [
    { "i": 0,
      "decision": "FETCH",
      "target_range":  { "start": 0,       "end": 690719 },
      "source_range":  null,
      "chunk_id":      "ddb31a9fd85346c4_de1cfe6927dacbc58d2d55a0b45ea0f3",
      "url":           "https://…/chunks/…/sMXGW2ll3Fuu/ddb31a9f…",
      "compressed_size": 259795, "uncompressed_size": 690719,
      "uncompressed_md5": "de1cfe69…", "note": null },
    { "i": 1,
      "decision": "REUSE",
      "target_range":  { "start": 690719,  "end": 2316738 },
      "source_range":  { "start": 690719,  "end": 2316738, "file_tag": "7.0.0" },
      "chunk_id":      "f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b",
      "url":           null,
      "compressed_size": 638614, "uncompressed_size": 1626019,
      "uncompressed_md5": "291a7719…",
      "note": "byte-identical to the old chunk at the same offset" },
    { "i": 2,
      "decision": "FETCH",
      "target_range":  { "start": 2316738, "end": 2340264 },
      "source_range":  null,
      "chunk_id":      "340ca86ee8ac9be0_87434f1bc3431f2c443dc5311d543308",
      "url":           "https://…/chunks/…/sMXGW2ll3Fuu/340ca86e…",
      "compressed_size": 14631, "uncompressed_size": 23526,
      "uncompressed_md5": "87434f1b…", "note": null }
  ],
  "totals": { "chunks": 3, "reuse": 1, "fetch": 2, "moved": 0,
              "download_bytes": 274426,
              "materialize_old_bytes": 2340264, "materialize_new_bytes": 2340264 },
  "expectations": {
    "result_path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
    "result_md5": "d992a789…",
    "absent_after": false,
    "download_bytes": 274426,
    "must_not_fetch": ["f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b"],
    "summary": {
      "damaged_chunks": 2, "inplace": 1, "temp": 0, "kept_bytes": 1626019,
      "ram_bytes_ordered": 0, "ram_bytes_staged": 0, "write_bytes": 714245
    }
  },
  "repair": {
    "kept": 1, "moved": 0, "fetch": 2,
    "kept_bytes": 1626019, "payload_bytes": 0, "write_bytes": 714245,
    "direction": "n/a", "keeps_ascending": 0, "keeps_descending": 0,
    "trim": "none", "growth_bytes": 0, "mode": "inplace"
  }
}
```

`expectations.must_not_fetch` is the negative assertion: a run that downloads
that chunk has a broken reuse path, and it is nameable from the artifact alone.

### 13.0 The `repair` block — what the in-place path would do

**What the repair does with this file, computed by the capture** so a consumer
does not have to work it out — and so the artifact and the tool cannot disagree
about it. Every number comes from the plan and the old side's bytes.

| field | what it says |
|---|---|
| `kept` | the chunks that are already correct where they belong: **kept**, not written |
| `moved` | the chunks whose bytes live elsewhere in the file: copied from a source |
| `fetch` | the chunks with no usable local source: downloaded |
| `kept_bytes` | the bytes that did not have to be written |
| `payload_bytes` | the RAM the **staged** tier needs: the bytes that have to move |
| `write_bytes` | the bytes the in-place run writes |
| `direction` | the write order the **ordered** tier would pick: `ascending`, `descending` or `n/a` |
| `keeps_ascending` / `keeps_descending` | how many sources each order would salvage |
| `trim` | when the dead tail is released: `eager`, `deferred` or `none` |
| `growth_bytes` | how much the file on disk has to grow |
| `mode` | the mode the default run takes: `inplace` |

The three counts are the repair's own vocabulary — **kept**, **copied**,
**fetched** — and they are not the same as the plan's `REUSE`/`FETCH` split: a
chunk whose source sits at its own offset is a `REUSE` in the plan and a
**kept** chunk here, because the repair leaves it alone.

The direction is the ordered tier's rule, the one
`repair::reuse::survives` applies: a source survives the ascending order when
it sits at or after its destination, the descending one when it sits at or
before it, and a source that reaches past the end of the file counts for
neither. A tie goes to ascending, as it does in the tool — but a tie that
keeps **nothing** is reported as `n/a`, because there is no order to name: six
of the ten reference captures have nothing to order at all.

`trim` is `eager` when the file on disk is longer than the target and **no
reuse source reaches past the target's end** — the tail is dead weight and is
released before the first write — and `deferred` when one does, because a
source the trim would delete is a source that is gone. `none` means the file
does not shrink.

The block is **null** for a capture with no new side: there is nothing to
repair. The writer emits the key either way, so a reader never has to tell
"absent" from "not written" — and §13.1's own `removed` example uses the same
convention for `"new": null, "plan": null, "totals": null`.

### 13.0a The `expectations.summary` block — the run's counters, per flag

**Every counter the repair reports is conditional on a flag** — the mode, the
RAM budget, the scan, the write ceiling — so the expectations are stated per
flag rather than as one flat set of numbers.

| key | the flag that produces it | what it says |
|---|---|---|
| `damaged_chunks` | `--scan` | the chunks that are wrong where they belong |
| `inplace` | `--repair-mode inplace` | the files the run repaired in place |
| `temp` | `--repair-mode inplace` | the files the run repaired through the temp path — none |
| `kept_bytes` | `--repair-mode inplace` | the bytes that did not have to be written |
| `ram_bytes_ordered` | `--max-ram-bytes 0` | the RAM the run reserved — zero, because the ordered tier stages nothing |
| `ram_bytes_staged` | `--max-ram-bytes` big enough for the staged tier | the RAM the run reserved |

**The two RAM rows are the capture's way of stating one key.** `girpr` reports a
single `ram_bytes` in its `SUMMARY`, and what it holds depends on the budget it
was given: zero under `--max-ram-bytes 0`, because the ordered tier stages
nothing, and the payload's size under a budget big enough for the staged tier.
So a consumer compares `ram_bytes_ordered` against the run that had no budget
and `ram_bytes_staged` against the run that had one — and the two rows are
what let it tell those two runs apart.
| `write_bytes` | `--max-write-bytes`, in place | the bytes the run wrote |

A file that is **skipped** — already correct, so the repair never runs —
changes every counter: `inplace` is `0` and `kept_bytes` and `write_bytes` are
`0` too. That is the `identical` class, and it is why the expectations are
about a *run* rather than about the file.

The block is **null** for a capture with nothing to repair — the same
convention as the `repair` block above.

`source_range` is the local-file byte range the main tool would read and MD5-verify
before copying. `moved` in `totals` counts entries where `source_range.start !=
target_range.start`.

A REUSE carries `url: null` — the bytes are already local, so there is nothing to
fetch — and a FETCH carries `note: null` for the same reason: each field is
present and says nothing only where it genuinely has nothing to say.
`materialize_old_bytes` and `materialize_new_bytes` are the **whole-file** sizes
each side assembles to, not the compressed chunk sums; `download_bytes` is the
only figure here that is priced per chunk.

A REUSE is decided *after* the slice has been verified by MD5, so a failed REUSE
becomes a FETCH. That downgrade is recorded on the entry (`note`) and in the
outcome (`rewrites`): a silent downgrade is indistinguishable from one that was
never a candidate, and the two demand opposite debugging.

### 13.1 The one-sided classes

`old` and `new` are the two sides; `plan`, `totals` and `expectations` describe
producing the **new** one. That makes the one-sided cases asymmetric, and both are
spelled out here because getting them wrong makes the artifact lie.

**`added`** — no `old`, but `plan`, `totals` and `expectations` are all present.
Every `plan` entry is `FETCH` with `source_range: null` and a `note` saying why,
because there is nothing to slice from. `totals` reads `reuse: 0`,
`fetch: nchunks`, `moved: 0`, `materialize_old_bytes: 0`, and `download_bytes`
is the whole file's compressed size — the file has to be fetched in full. The
writer **refuses** to emit a capture that has a new side and no plan.

**`removed`** — no `new`, and therefore no `plan` and no `totals`: there is no
target to plan for and nothing to cost. `expectations` **is** present, in its own
shape:

```jsonc
{ "capture_id": "removed",
  "old": { "tag": "7.0.0", "size": 1731133, "md5": "6c220a33…", "nchunks": 2 },
  "new": null, "plan": null, "totals": null,
  "expectations": {
    "result_path": null,
    "result_md5":   null,
    "absent_after": true,
    "download_bytes": 0,
    "must_not_fetch": [],
    "summary": null
  },
  "repair": null }
```

`download_bytes: 0` here is a **true** value, not a default: a removal downloads
nothing. It is written down rather than left `null` precisely because the number
is known and a consumer should be able to compare against it.

### 13.2 `download_bytes` is always a number

`expectations.download_bytes` is the sum of `compressed_size` over the plan's
`FETCH` entries — what a correct patch costs. It is **never** null and never
defaulted. This is stated as a rule because getting it wrong is not a cosmetic
bug: `null` where a figure exists reads as *unrecorded* and `0` reads as a claim.
An `added` capture whose file actually costs 114 357 B to download, written as
`plan: null` / `totals: null` / `download_bytes: 0`, makes a consumer that
asserts `SUMMARY download_bytes == expectations.download_bytes` compare 114 357
against 0 and call a perfectly correct run a failure. Assertion A20 exists to stop
the rule being quietly dropped.

Absent data has to be absent. Where a figure genuinely does not exist, the field
that would hold it is `null` — which is why `result_path` and `result_md5` can be
null while `download_bytes` cannot.

`expectations` is required on every capture. A patchmap a consumer cannot assert
anything about is the defect, not a legitimate shape; that is what makes `removed`
testable rather than merely recorded.

---

## 14. `game/` and `expected/`

Plain bytes at their real relative paths — no wrappers, no metadata. `game/`
contains old-version files only; `expected/` contains new-version files only.
`game/` additionally holds the tool-read config: `config.ini` and the audio scan
file at `getGameConfigs.audio_pkg_scan_dir`.

Both trees are covered by `MANIFEST.json → files[]` and
`verify/checksums.json`. A consumer can `diff -r` `game/` against `expected/`
and every differing file is exactly one that needs patching — except `identical`,
whose two sides are deliberately byte-identical.

> **Consuming `game/` with the main tool needs `girpr --include`.** Run against
> this directory unscoped, the main tool builds its plan from the whole live
> manifest and would fetch the entire game. Point it at a **copy** of `game/` and
> name one capture per run with `--include` (`removed` takes
> `--purge-only --include` instead). See
> [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle-with-girpr) for the
> procedure and the per-capture figures.

## 15. `verify/checksums.json`

```jsonc
{ "schema_version": 1,
  "algorithm": "sha256",
  "entries": [
    { "path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": 2340264,
      "md5": "d992a78915ebc0cc31b86d10b05c14e4f",
      "sha256": "…", "encoding": "identity" },
    { "path": "manifests/7.1.0/game.manifest.json.gz",
      "bytes": 40996047,
      "md5": "8c718562f39f91f7e399b02de591f3bd",
      "sha256": "…", "encoding": "gzip" }
  ],
  "file_count": 96, "total_bytes": 208307799 }
```

This is the **complete** inventory of the bundle: every file except
`checksums.json` itself. `file_count` is therefore one less than the number of
files on disk, and a bundle whose count disagrees with its tree fails A16.

**`encoding: gzip` means the digest is of the _decompressed_ body**, and `bytes`
is the decompressed length too. That is the only way a reader can check the
document rather than the container, and it means a digest does not change if the
file is recompressed or produced by a different compressor. It also means
verifying a `.gz` entry means decompressing it first — which is what `gircap
verify` does, and why A16 would fail if the rule and the implementation
disagreed.

A16 is deliberately **two independent passes**: one that writes the digests and
one that re-reads the tree and compares. A single pass would make the assertion a
tautology. Because `checksums.json` must cover `selfcheck.json` — and `README.md`,
which renders it — and `selfcheck.json` must carry the A16 row, the close-out
writes those documents twice and the second scan is the one recorded. Two passes
converge exactly: the second `checksums.json` is scanned after the final
`selfcheck.json` and `README.md`, and neither of those sizes depends on a digest,
so a third pass would find nothing new.

**A16's row states a file count and not a byte total.** That is forced, not
chosen: the row lives inside `selfcheck.json`, which is one of the files
`total_bytes` sums, so the last version of that number to be written is the one in
`checksums.json` — and any total in the row describes an earlier, smaller tree.
The count is safe, because no file appears or disappears between the two passes.

## 16. `verify/selfcheck.json`

```jsonc
{ "schema_version": 1,
  "result": "pass",                                  // pass | fail | incomplete
  "counts": { "total": 354, "passed": 354, "failed": 0 },
  "assertions": [
    { "id": "A6", "assertion": "chunk decompressed md5 matches",
      "subject": "chunks/blobs/340ca86e….zst",
      "expected": "87434f1bc3431f2c443dc5311d543308",
      "actual":   "87434f1bc3431f2c443dc5311d543308",
      "ok": true }
  ],
  "matrix": [ { "class": "identical", "status": "captured", "ok": true }, … ]
}
```

One row per assertion **instance**, not per assertion kind — 259 stored chunks
produce 259 A6 rows, so `counts.total` is much larger than the number of ids. In
the reference capture it is 354: A1 6, A2 2, A3 4, A4 8, A6 259, A7 9, A8 9,
A9 8, A10 6, A11 18, A12 9, A13 1, A14 1, A15 8, A16 1, A17 1, A19 1,
A21 1, A22 1, A23 1.
`ok:false` rows are never dropped; a failing capture leaves them in place so the
failure is inspectable rather than summarized away.

`assertion` carries the assertion's wording, so a reader looking at a failed
bundle does not have to open another file to find out what A11 means. The
wording table is pinned by a test: `id` alone would pass if an assertion were
silently narrowed, and "every reused chunk's old-file slice MD5 matches" and
"every reused chunk's MD5 matches" are both `A10`.

A skipped row is recorded too, and is **not** a failure: `ok` is absent and the
row is listed under `skipped`. A `--no-chunk-blobs` capture skips A6–A12, A19 and
A21–A23, reports `result: "incomplete"`, and exits `0`.

A capture's `selfcheck.json` never contains an `A18` **or** an `A20` row: both
are statements about the bundle rather than about the run, so only
`gircap verify` can assert them. A reader who sees either id in this file has
been handed a `verify` report, not a capture's own account of itself.

## 17. `README.md`

A rendering of `MANIFEST.json` and `selfcheck.json`: versions, the matrix table
with paths and sizes, the totals, any failed assertion, and how to re-verify.
It is generated from the **final** `selfcheck.json` — after the A16 row is
recorded — so its counts agree with the document it renders, and it carries no
facts of its own. It is safe to delete.
