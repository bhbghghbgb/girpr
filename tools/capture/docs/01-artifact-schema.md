# Artifact schema — `gircap` bundle

Byte-level layout and every field of every machine-readable file.

**Every document below carries `"schema_version": 1`.** A reader should reject an
unknown `schema_version` rather than guess. JSON is UTF-8, LF, 2-space indent,
keys in the order written here. `*.ndjson` is newline-delimited, one object per
line, no enclosing array, so it streams.

**Compression rule.** A document is gzipped (`*.json.gz`) only when its
uncompressed body exceeds `--gzip-threshold` (default 1 MiB). The decision is
recorded in `MANIFEST.json → files[].encoding`, so a reader never has to guess.
Today exactly one document trips it: the full decoded manifest. Every gzipped
document ships a plain-text sidecar (`*.tsv`) so an agent can `grep` it without
decompressing.

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
│   └── requests.ndjson
├── manifests/<tag>/
│   ├── <field>.manifest.zst                  # raw blob, byte-exact
│   ├── <field>.manifest.meta.json
│   ├── <field>.manifest.json.gz              # full decode (or .json if small)
│   └── <field>.paths.tsv                     # always plain; path \t size \t md5 \t nchunks
├── diff/
│   ├── paths.diff.json
│   ├── paths.tsv                             # path \t state \t old_size \t new_size \t cost
│   ├── candidates.json
│   └── reuse_pool.json
├── game/                                     # ★ the fixture
│   ├── config.ini
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

`capture_id` is `<class>` or `<class>+<n>` (e.g. `partial_same_offset`,
`partial_moved_offset+2`). It appears in `patchmaps/` and in `MANIFEST.json`.

Nothing under `chunks/` or `manifests/` is ever deleted by the tool. Any
temporary file it creates is written into `tmp/` and moved into place, and is
removed only on success — `verify/checksums.json` proves nothing is missing.

---

## 2. `MANIFEST.json` — index

```jsonc
{
  "schema_version": 1,
  "tool":   { "name": "gircap", "version": "0.1.0" },
  "capture": {
    "id": "20261014T101500Z",
    "captured_at": "2026-10-14T10:15:00Z",
    "host_os": "windows",
    "tool_argv": ["capture", "--biz", "hk4e_global", …],
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
  "selection_pool": {                    // matching_fields scored and diffed
    "fields": ["game", "en-us"],
    "manifest_entries": { "game": 2744, "en-us": 177 },
    "note": "--audio widens this pool; captures may come from any listed field"
  },
  "totals": {
    "files_captured": 8,
    "chunks_blobs": 41,
    "bytes_downloaded": 96041377,
    "bytes_on_disk": 98812044
  },
  "matrix": [                            // one row per required class, in admission order
    { "class": "identical", "status": "captured", "capture_id": "identical",
      "path": "GenshinImpact_Data/Resources/browser_assets", "matching_field": "game",
      "also_satisfies": [] },
    { "class": "partial_same_offset", "status": "captured", "capture_id": "partial_same_offset",
      "path": "GenshinImpact_Data/Plugins/HoYoChannel.dll", "matching_field": "game",
      "also_satisfies": ["partial_same_offset"] },
    { "class": "partial_size_changed", "status": "covered_by",
      "covered_by": "partial_same_offset",
      "reason": "cheapest candidate already captured for a cheaper class" },
    { "class": "full_reuse_reordered", "status": "unavailable",
      "reason": "no path in any selected field has reused==nchunks && md5 differs",
      "candidates_considered": 2839 }
  ],
  "files": [                             // every file under the artifact root
    { "path": "game/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": 2340264, "md5": "1e21…", "sha256": "…", "encoding": "identity" }
  ],
  "totals_by_set": { "materialize_old": 27, "materialize_new": 31, "patch_fetch": 14 }
}
```

`matrix[].status` ∈ `captured` | `covered_by` | `over_budget` | `unavailable`.
A class is never simply absent (assertion A17).

---

## 3. `api/*.json`

Byte-exact response bodies, unmodified — not re-serialized. A reader must be
able to `diff` a captured file against a fresh request. `api/requests.ndjson`
records how they were obtained.

## 4. `api/requests.ndjson`

One object per HTTP request, in issue order.

```jsonc
{ "seq": 3, "ts": "2026-10-14T10:15:02.117Z", "kind": "manifest",   // api|manifest|chunk|probe
  "method": "GET", "url": "https://…/manifest_671e…",
  "range": null,                        // literal Range header sent, if any
  "status": 200, "final_url": "https://…", "redirected": false,
  "elapsed_ms": 418,
  "req_headers":  { "user-agent": "gircap/0.1.0", "accept-encoding": "gzip" },
  "resp_headers": { "content-type": "application/octet-stream", "content-length": "9007187",
                    "etag": "\"313EA8…\"", "last-modified": "…", "accept-ranges": "bytes",
                    "content-md5": "MT6ox…", "server": "AliyunOSS", "x-cache": "Hit from cloudfront",
                    "via": "1.1 c4fb…cloudfront.net (CloudFront)", "x-amz-cf-pop": "SGN50-P1",
                    "age": "1712968" },
  "bytes_in": 9007187,
  "stored_as": "manifests/7.1.0/game.manifest.zst",
  "attempts": 1 }
```

Header keys are lowercased; values are strings exactly as received. `kind` lets a
reader ask "what did the manifest fetch cost?" without string-matching URLs.

## 5. `api/tag_probe.json`

Which tags the API actually serves, with no blob downloads.

```jsonc
{ "schema_version": 1,
  "requested": ["7.1.0", "7.0.0", "6.7.0", "6.6.0", "5.5.0"],
  "results": [
    { "tag": "6.6.0", "in_diff_tags": false, "http": 200, "retcode": 0,
      "build_id": "L7CJe4FP1z71", "tag_echoed": "6.6.0", "served": true },
    { "tag": "0.0.1", "in_diff_tags": false, "http": 200, "retcode": -202,
      "build_id": null, "tag_echoed": null, "served": false }
  ],
  "summary": { "requested": 5, "served": 5, "not_in_diff_tags_but_served": 2 } }
```

`served` is the load-bearing fact: `diff_tags` is a patch-support list, not a
retention list (spec §3.4).

---

## 6. `manifests/<tag>/<field>.manifest.meta.json`

Everything known about one archive blob.

```jsonc
{ "schema_version": 1,
  "tag": "7.1.0", "build_id": "sMXGW2ll3Fuu", "matching_field": "game",
  "category_id": "10016", "category_name": "game",
  "manifest": { "id": "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d",
                "checksum": "8d4dfb34d2ee2cf64aae45a9b1ecf58d",
                "compressed_size": 9007187, "uncompressed_size": 16827424 },
  "declared": {                                  // fields the API sends that most clients drop
    "stats": { "file_count": 2744, "chunk_count": 113610,
               "compressed_size": 128310073410, "uncompressed_size": 131981846376 },
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
               "structure": { "duplicate_paths": 0, "empty_paths": 0,
                              "backslash_paths": 0, "non_ascii_paths": 0,
                              "unsorted_chunk_offsets": 0, "non_gapless_files": 0,
                              "size_mismatch_files": 0 } }
}
```

`verified.id_prefix_verified` is **always false** and is expected to be. The
16-hex prefix is not a hash of any observable input (spec §3.7); the field
exists so the artifact states that finding rather than implying the prefix went
unchecked.

## 7. `manifests/<tag>/<field>.manifest.json.gz` — full decode

The whole protobuf, verbatim, so no consumer has to re-download to inspect it.

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

`unknown_field6` is the wire's tag 6. Its meaning is unknown; it is preserved
because the upstream schema carries it and dropping it would make this dump a
lossy transform. *(measured)* it is stable across the two builds for the same
logical chunk even when content differs, which is exactly why it must not be
guessed at.

## 8. `manifests/<tag>/<field>.paths.tsv`

Always plain text, tab-separated, header line, `\n` endings:

```
path	size	md5	nchunks	is_folder
GenshinImpact_Data/Plugins/HoYoChannel.dll	2340264	d992a789…	3	false
```

This is the sidecar that makes the gzipped JSON greppable without decompression.

---

## 9. `diff/paths.diff.json`

```jsonc
{ "schema_version": 1,
  "old_tag": "7.0.0", "new_tag": "7.1.0",
  "fields": ["game", "en-us"],
  "counts": { "common": 2662, "identical": 1592, "changed": 1070,
              "added": 82, "removed": 11, "folders": 0 },
  "byte_counts": { "added": 954110883, "removed": 215743102 },
  "added":   ["GenshinImpact_Data/Plugins/libHttpClient.GDK.dll", …],
  "removed": ["GenshinImpact_Data/…/00664172.blk", …],
  "by_field": { "game": { "common": 2662, "identical": 1592, "added": 82, "removed": 11 },
                "en-us": { "common": 0, "identical": 0, "added": 0, "removed": 0 } } }
```

Counts at the top level are aggregated over `fields`; `by_field` is the
breakdown, and every `added`/`removed` entry is prefixed with its field so a
path from two pools cannot collide. The `game` row is the **measured** one for
7.0.0→7.1.0; the `en-us` row is a placeholder — the value depends on which
audio languages `--audio` selected.

`diff/paths.tsv` is the flat per-path form (`path \t state \t old_size \t
new_size \t reused \t moved \t patch_bytes \t total_cost`), sorted by `path`.

## 10. `diff/candidates.json`

The scoring table, for every common path (not just the winners) unless
`--max-candidate-rows` trims it. Lets a later run ask "why did it pick that?"

```jsonc
{ "schema_version": 1,
  "columns": ["path","old_size","new_size","nchunks","reused","moved","same_md5",
              "old_bytes","new_bytes","patch_bytes","total_cost","classes"],
  "rows": [
    { "path": "…/HoYoChannel.dll", "old_size": 2340264, "new_size": 2340264,
      "nchunks": 3, "reused": 1, "moved": 0, "same_md5": false,
      "old_bytes": 913027, "new_bytes": 913040, "patch_bytes": 274426,
      "total_cost": 1826067, "classes": ["partial_same_offset"], "rank": 1 }
  ] }
```

Column-oriented so a reader can sort without re-parsing objects.

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
when several files or both builds reference it.

```jsonc
{ "schema_version": 1,
  "blobs": [
    { "chunk_id": "f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b",
      "stored_path": "chunks/blobs/f5dc3ca1f3a0ba3d_291a7719….zst",
      "bytes": 638614, "container": "zstd",
      "md5_compressed": "79f1d31986dae0e09f05fcad6320205d",
      "md5_decompressed": "291a7719a4a23664d37825a41fad3c5b",
      "sha256_compressed": "…",
      "uncompressed_size": 1626019,
      "unknown_field6": 5814299429820432384,
      "verified": { "md5_compressed_matches": true, "md5_decompressed_matches": true,
                    "size_matches": true },
      "referenced_by": [
        { "tag": "7.0.0", "capture_id": "partial_same_offset",
          "path": "…/HoYoChannel.dll", "role": "materialize_old", "offset": 690719 },
        { "tag": "7.1.0", "capture_id": "partial_same_offset",
          "path": "…/HoYoChannel.dll", "role": "materialize_new", "offset": 690719 }
      ],
      "sets": ["materialize_old", "materialize_new"] }
  ],
  "sets": {
    "materialize_old": [ … chunk ids … ], "materialize_new": [ … ],
    "patch_fetch":     [ … ]
  } }
```

`role` ∈ `materialize_old` | `materialize_new` | `patch_fetch`. `patch_fetch` is
always a subset of `materialize_new` for the same file (assertion A12).

---

## 13. `patchmaps/<capture_id>.json` — the core

Per target chunk: where its bytes come from.

```jsonc
{ "schema_version": 1,
  "capture_id": "partial_same_offset",
  "path": "GenshinImpact_Data/Plugins/HoYoChannel.dll",
  "matching_field": "game",
  "classes": ["partial_same_offset"],
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
      "uncompressed_md5": "de1cfe69…", "compressed_md5": "06ba73bb…" },
    { "i": 1,
      "decision": "REUSE",
      "target_range":  { "start": 690719,  "end": 2316738 },
      "source_range":  { "start": 690719,  "end": 2316738, "file_tag": "7.0.0" },
      "chunk_id":      "f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b",
      "url":           "https://…/chunks/…/sMXGW2ll3Fuu/f5dc3ca1…",
      "uncompressed_md5": "291a7719…",
      "note": "byte-identical to the old chunk at the same offset" },
    { "i": 2,
      "decision": "FETCH",
      "target_range":  { "start": 2316738, "end": 2340264 },
      "source_range":  null,
      "chunk_id":      "340ca86ee8ac9be0_87434f1bc3431f2c443dc5311d543308",
      "url":           "https://…", "compressed_size": 14631, "uncompressed_size": 23526,
      "uncompressed_md5": "87434f1b…" }
  ],
  "totals": { "chunks": 3, "reuse": 1, "fetch": 2, "moved": 0,
              "download_bytes": 274426, "materialize_old_bytes": 913027,
              "materialize_new_bytes": 913040 },
  "expectations": {
    "result_path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
    "result_md5": "d992a789…",
    "download_bytes": 274426,
    "must_not_fetch": ["f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b"]
  }
}
```

`expectations.must_not_fetch` is the negative assertion: a run that downloads
that chunk has a broken reuse path, and it is nameable from the artifact alone.

`source_range` is the local-file byte range the main tool would read and MD5-verify
before copying. `moved` in `totals` counts entries where `source_range.start !=
target_range.start`.

For `removed` captures there is no `new`, no `plan`, and no `expectations`; for
`added` there is no `old`, and every `plan` entry is `FETCH` with
`source_range: null`.

---

## 14. `game/` and `expected/`

Plain bytes at their real relative paths — no wrappers, no metadata. `game/`
contains old-version files only; `expected/` contains new-version files only.
`game/` additionally holds the tool-read config: `config.ini` and the audio scan
file at `getGameConfigs.audio_pkg_scan_dir`.

Both trees are covered by `MANIFEST.json → files[]` and
`verify/checksums.json`. A consumer can `diff -r` `game/` against `expected/`
and every differing file is exactly one that needs patching.

> **Consuming `game/` with the main tool is blocked** on `girpr` gaining path
> inclusion/exclusion. Run against this directory as-is, the main tool builds its
> plan from the whole live manifest and would fetch the entire game. See
> [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle--and-one-external-blocker)
> for the intended invocation and why the fix belongs in `girpr`, not here.

## 15. `verify/checksums.json`

```jsonc
{ "schema_version": 1,
  "algorithm": "sha256",
  "entries": [
    { "path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": 2340264,
      "md5": "d992a78915ebc0cc31edb9c6d5250c47",
      "sha256": "…", "container": "identity" }
  ],
  "file_count": 96, "total_bytes": 98812044 }
```

Sorted by `path`. Covers every regular file under the artifact root except
`verify/checksums.json` itself.

## 16. `verify/selfcheck.json`

```jsonc
{ "schema_version": 1,
  "result": "pass",                                  // pass | fail | incomplete
  "counts": { "total": 71, "passed": 71, "failed": 0 },
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

One row per assertion **instance**, not per assertion kind — 71 chunks produce 71
A6 rows. `ok:false` rows are never dropped; a failing capture leaves them in
place so the failure is inspectable rather than summarized away.

## 17. `README.md`

A rendering of `MANIFEST.json` and `selfcheck.json`: versions, the matrix table
with paths and sizes, the totals, and how to re-verify (`gircap verify`). It is
generated, carries no facts of its own, and is safe to delete.