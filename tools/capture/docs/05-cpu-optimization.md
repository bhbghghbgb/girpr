# CPU optimisation report — `gircap capture`

**Date:** 2026-10-30  
**Tree:** `125f39e`  
**Scope:** `tools/capture` (`gircap`), capture path only. Report only — no source
changes were made for this investigation.  
**Bundle measured:** `artifacts/hk4e_global/7.0.0__7.1.0/20261029T124809.314540Z`,
replayed over loopback so the run remains live-shaped without touching the
network.

---

## 1. Executive summary

The default capture is CPU-bound by two things that are not part of the
artifact contract:

1. **The full decoded manifest JSON.** It is a convenience transform of the raw
   `.manifest.zst` archive, not independent evidence.
2. **Gzip.** It only compresses that decoded JSON. It saves disk, which is not
   the objective, and it costs substantially more CPU than it saves anything
   else.

On the measured bundle:

| mode | capture CPU | capture wall | bundle on disk | files |
|---|---:|---:|---:|---:|
| default (`--manifest-detail full`, gzip at 1 MiB) | **6.8–7.4 s** | 6.8–8.9 s | 97.7 MiB | 80 |
| full decode, no gzip | **3.45–3.55 s** | 4.5–4.9 s | 167.5 MiB | 80 |
| `--manifest-detail paths-only` | **2.36–2.38 s** | 2.0–2.3 s | 76.0 MiB | 76 |

`gircap verify` follows the same shape:

| artifact | verify CPU |
|---|---:|
| reference full/gzip bundle | 1.44–1.47 s |
| generated full/gzip bundle | 1.47–1.48 s |
| generated full/no-gzip bundle | 1.23 s |
| generated paths-only bundle | 0.84–0.86 s |

**Recommendation:** make `paths-only` the default, and make full decoded JSON an
explicit opt-in that is written **plain**, not gzipped. That takes the default
capture from about **7 s to about 2.4 s CPU** — roughly a 3× reduction — while
keeping every file `girpr`'s `artifact_bridge` actually reads. The same change
also removes about **0.6 s** from the bridge's `gircap verify` precondition.

The rest of the code opportunities are real but second-order: eliminating the
duplicate JSON serialisation pass, and removing a few full-file copies in the
patch assembly. The close-out's second full-tree hash is deliberate (A16) and
should **not** be weakened.

---

## 2. Method

### 2.1 What was measured

CPU time, not wall time. Wall time on this machine is dominated by filesystem
and antivirus behaviour and moved by seconds between otherwise identical runs;
`Process.TotalProcessorTime` was stable.

Each capture was run as a real release binary:

```text
gircap capture
  --hyp-base    http://127.0.0.1:65488/hyp
  --sophon-base http://127.0.0.1:65488/sophon
  --old-tag 7.0.0 --new-tag 7.1.0
  --out <fresh scratch root>
  --no-log-file --output json
```

A small Python server replayed the reference bundle's `api/`, manifest and blob
responses on `127.0.0.1:65488`, preserving the manifest-id → `(tag, field)` and
chunk-id mappings. Server CPU is not included in the figures. The server and
bundle are local, so the benchmark measures the tool rather than the network.

### 2.2 Variants

| variant | extra flags |
|---|---|
| full/default | none (`--manifest-detail full`, `--gzip-threshold 1048576`) |
| full/no-gzip | `--gzip-threshold 1000000000000` |
| paths-only | `--manifest-detail paths-only` |

Two or three repetitions were run per variant. The ranges above cover the
spread. `--jobs 1` and `--jobs 4` did not materially change total CPU; the
numbers combine them.

### 2.3 Stage sampling

A stderr-line sampler took `TotalProcessorTime` snapshots at the existing
`tracing` milestones. Stage boundaries are approximate because some work has no
intervening log line — notably, the manifest tree write and the chunk fetch both
fall between `manifest decoded` and `chunk store done`. That combined stage is
labelled as such below.

Representative run:

| phase | full + gzip | full, no gzip | paths-only |
|---|---:|---:|---:|
| resolve, probe, manifest decode, diff, selection, plan | 0.98 | 0.80 | 0.77 |
| **manifest/tree write + chunk fetch/measure** | **3.83** | **0.67** | **0.34** |
| fixture assembly | 0.53 | 0.55 | 0.50 |
| close-out + A16 | 1.94 | 1.45 | 0.69 |
| process overhead | 0.08 | 0.08 | 0.08 |
| **total** | **7.36** | **3.55** | **2.38** |

The combined write/fetch row is the important one:

- paths-only: the row is almost entirely the 25 chunk fetches and their
  measurement, about 0.34 s;
- full/no-gzip: add about 0.33 s for generating and writing 96 MB of decoded
  JSON;
- full/gzip: add about **3.2 s** for the two serialisation crossings plus
  deflate. The close-out then spends another **0.5 s** decompressing the four
  `.json.gz` documents while hashing them.

So gzip alone costs about **3.7 s per capture** on this data, plus about
**0.24 s** in `verify`.

---

## 3. Where the bytes and CPU go

The four full manifests dominate both the write and the close-out:

| component | paths-only | full/gzip | full/no-gzip |
|---|---:|---:|---:|
| `manifests/` on disk | 20.7 MB | 43.4 MB | 116.6 MB |
| `chunks/` | 16.6 MB | 16.6 MB | 16.6 MB |
| `game/` | 20.9 MB | 20.9 MB | 20.9 MB |
| `expected/` | 19.6 MB | 19.6 MB | 19.6 MB |
| `patchmaps/` + `api/` + `verify/` | ~0.12 MB | ~0.12 MB | ~0.12 MB |

The raw manifest archives are only **19.96 MB** total. The full decode adds
**95.9 MB** of plain JSON; gzip shrinks that to **22.7 MB**. In other words:

- gzip saves **73 MB on disk**;
- it costs **~3.7 s CPU per capture** and **~0.24 s per verify**;
- it makes the decode harder to inspect offline, not easier.

That is the wrong trade for a CPU-time objective.

The close-out costs more even without gzip because every byte of the bundle is
hashed twice by design: `Checksums::scan`/`scan_reusing` builds
`verify/checksums.json`, then `Checksums::a16` independently re-reads and
re-hashes the tree. That is A16's whole value — it is an assertion, not a
tautology — so the second pass is not waste to remove. The waste is giving it
96 MB of redundant JSON to hash in the first place.

---

## 4. Opportunities, ranked

### O1. Default to `paths-only` — largest win, no bridge loss

**Measured saving:** 4.4–5.0 s capture CPU; 0.6 s verify CPU.

`--manifest-detail paths-only` already exists and is already a working bundle
mode. It drops exactly four documents — the four full manifest decodes — and
keeps the archive, `.meta.json`, `.paths.tsv`, `chunks/`, `game/`, `expected/`,
`patchmaps/`, `api/`, `MANIFEST.json` and `verify/`. The generated paths-only
bundle verifies cleanly: 63/63 checks, 25 blobs, 6 captures rebuilt. The bridge
reads none of the dropped files.

Why this is safe on the "state of the API" rule:

- the **raw `.manifest.zst` archive is the exact state**, byte-for-byte what the
  CDN served;
- the `.meta.json` sidecar records stored path, encoding, compressed digests,
  decompressed size and reconciliation data;
- `paths.tsv` is the path-level view: path, size, md5, chunk count, folder flag;
- the full decode contains no additional source-of-truth bytes. It is a
  JSON rendering of the same protobuf, with `unknown_field6` passed through.
  Anything nested and non-path-level is still recoverable from the archive.

The full decode is therefore high-CPU, duplicated information, with convenience
rather than validation value. It is the right thing to make opt-in. Even with
gzip switched off, the full decode still costs about **1.2 s per capture** and
**0.4 s per verify** over paths-only, because both close-out passes have 96 MB
more to hash.

**Contract after this change**

- default: raw archive + meta + paths TSV, no full decode;
- `--manifest-detail full`: still available for a person debugging a manifest;
- `MANIFEST.json → files[]` and `verify/checksums.json` simply have four fewer
  entries; no schema change.

This also makes `--gzip-threshold` irrelevant to a normal capture, because the
full decode was the only document large enough to trip it.

**Source points:** `src/cli/mod.rs:177` (`default_value_t = ManifestDetail::Full`),
`src/manifest/dump.rs:129` (the only `Full` branch),
`src/manifest/dump.rs:171` (`full_decode_written`).

---

### O2. If full detail is kept, stop gzipping it

**Measured saving:** ~3.8 s capture CPU; ~0.24 s verify CPU.

If the project decides the full decode must remain in the default bundle for
offline learning, the next-best change is to stop compressing it. The full
decode is already a convenience document; making it plain costs 73 MB more disk
and removes the largest CPU cost in the tool. It also makes the file directly
readable, which is the stated purpose of keeping it.

Two implementation options:

1. **Minimal:** set `DEFAULT_GZIP_THRESHOLD` to `u64::MAX` (or remove the gzip
   branch for manifests). Keeps the CLI flag and the recorded value, but no
   normal capture gzips.
2. **Cleaner:** make full mode always write `.json`; if gzip is still wanted as
   a diagnostic, make it an explicit opt-in flag rather than a threshold that
   silently changes a file extension.

Option 2 is preferable because a file name that depends on content size is a
small but real source of surprise. The current rule is documented and tested,
so this is a contract decision, not a bug fix.

**Source points:** `src/cli/mod.rs:121`
(`DEFAULT_GZIP_THRESHOLD = 1_048_576`), `src/manifest/dump.rs:137–150`,
`src/emit/mod.rs:481`.

---

### O3. Remove the duplicate serialisation pass

**Measured saving:** ~0.1–0.2 s, full-detail mode only.

The full branch serialises the manifest twice:

1. `json_len(&full, ...)` walks the whole decode into a counting writer just to
   learn the byte length and choose `.json` vs `.json.gz`;
2. `write_json_gz(..., &full, gzipped)` walks it again to write the file.

That is pure duplication. It is small compared with gzip, but it is avoidable
without changing one byte:

- add a `write_json` helper that serialises through a counting writer into the
  already-known `.json` path and returns the byte count;
- use that count for `files[].uncompressed_bytes` and `meta`;
- keep the old `json_len` pre-pass only on the explicit gzip path, where the
  extension genuinely depends on the length.

If full mode becomes plain by default (O2), this removes the pre-pass entirely
for normal use.

**Source points:** `src/manifest/dump.rs:137`, `src/emit/mod.rs:444` and `:481`.

---

### O4. Reduce patch-flow copies and the third file buffer

**Measured saving:** ~0.05–0.15 s; assembly is 0.50–0.55 s total.

Assembly is now the largest remaining non-I/O phase on a paths-only capture
(about 0.5 s, or roughly 20% of the total). The patch flow is doing more copying
than it needs to:

- a REUSE candidate does `slice.to_vec()` and then
  `out.extend_from_slice(&body)` — two copies of the same slice;
- a FETCH does `decoded().into_owned()` and then copies that into `out`;
- a third full-file buffer (`out`) exists only so `assemble` can compare it with
  `new.bytes` and compute its MD5.

A lower-copy shape:

1. write REUSE bytes directly from `old.bytes[src_range]` into the target range
   of `out`, with no intermediate `Vec`;
2. for FETCH, borrow the `Cow` and copy directly into `out` (no `into_owned`);
3. compare each produced chunk against the corresponding range of `new.bytes`
   and update an incremental MD5 as the chunk is produced. That preserves A9's
   byte equality and the patch map's `md5`, without a third whole-file buffer.

The independence of A9 is preserved: the comparison is still against the
separately assembled `new.bytes`, chunk by chunk, not against `out` itself.

This is a modest win, but it is real, and it is the right cleanup if the
assembly remains a hot path after O1. Do not combine it with a decoded-chunk
cache: the bundle is only 25 blobs / 16.6 MB here, the docs already measured
zstd decode as small next to filesystem work, and capture deliberately avoids
holding the fixture resident. Caching would trade memory for very little CPU.

**Source points:** `src/assemble/assembly.rs:305–315` (`slice.to_vec()`),
`:356` (`into_owned()`), `:374` (`extend_from_slice`), `:400` (`md5_hex(&out)`),
`:244` (A9 byte comparison), `src/assemble/source.rs:147`.

---

### O5. Do not "optimise" A16 by reusing hashes

A16 is the second independent full-tree hash. Reusing the first pass's digests
would make it compare a value with itself and would remove the only check that
catches a stale or incomplete `checksums.json`. It is also the gate that makes
`gircap verify` trustworthy as the bridge precondition.

The CPU lever is **fewer bytes in the tree** (O1), not a weaker A16. If the
project ever needs to cut close-out CPU further, the correct questions are
"which documents are redundant?" and "which files can be excluded by contract?",
not "can the second hash share a buffer?".

**Source points:** `src/emit/closeout.rs:158`, `:211`, `:232`;
`src/emit/index.rs:340`, `:390`; `docs/03-design-notes.md` §2.

---

### O6. Micro-optimisation: `paths_tsv`'s lookup map

`paths_tsv` sorts a `Vec<&str>` of paths and then looks each one up in a
`HashMap<&str, &SophonChunkFile>` built from the same entries. That is 2,744
hash inserts and lookups per game manifest for no reason: sort the entries
directly.

```rust
let mut rows: Vec<&SophonChunkFile> = entries.iter().collect();
rows.sort_unstable_by_key(|e| e.file.as_str());
for e in rows { /* render e directly */ }
```

This is a correctness-neutral simplification and saves allocations. Its CPU
effect is below the noise floor (<0.01 s), so it should not be sold as a
performance fix; it is listed only because it removes a data structure that
exists solely to undo the sort.

**Source:** `src/manifest/dump.rs:226–252`.

---

## 5. What the bridge actually requires

The runtime reader in `tests/artifact_bridge` and the server in
`tests/artifact_bridge/server.rs` need:

| artifact component | why the bridge needs it | kept by paths-only? |
|---|---|---|
| `MANIFEST.json` | `biz`, `matrix[]`; the capture is resolved through the matrix | yes |
| `patchmaps/<id>.json` | plan decisions, fetch/reuse sets, old/new sizes, `result_md5` | yes |
| `api/getBuild.<tag>.json` | manifest ids and the `url_prefix` values the server rewrites | yes |
| `api/getGameConfigs.json`, `getGameBranches.json`, `getGameDeprecatedFileConfigs.json` | HYP route replay | yes |
| `api/error_shapes.json` | recorded `404` / `-202` shapes for negative replay | yes |
| `manifests/<tag>/<field>.manifest.zst` | the exact bytes served for each manifest id | yes |
| `chunks/blobs/<id>.zst` | chunk response bodies | yes |
| `chunks/index.json` | materialisation and `verify` closure | yes |
| `game/` | old fixture and scratch-copy source | yes |
| `expected/` | ground truth; `expectations.result_path` names it | yes |
| `verify/checksums.json`, `verify/selfcheck.json` | `gircap verify` precondition (63/63 checks) | yes |
| `api/requests.ndjson` | not read by the bridge, but records the API conversation | yes |
| full `*.manifest.json[.gz]` | **not read by the bridge at all** | **no** |

Two nuances:

- The bridge's runtime assertions use `result_md5`, not the bytes under
  `expected/`, but `expected/` is still the bundle's ground truth and the
  `result_path` contract. It is inexpensive compared with the decoded JSON and
  should stay.
- `gircap verify` is a hard precondition (`docs/06-artifact-bridge-spec.md` §3),
  so `verify/checksums.json` and `verify/selfcheck.json` are not optional even
  though the server does not read their contents directly.

The minimum is therefore **the raw manifest archive + API body + chunk blobs +
fixture**, not a decoded JSON copy. Removing the full decode does not weaken the
bridge; it removes a fourth representation of the same manifest.

---

## 6. Recommended target contract

1. **`gircap capture` defaults to `--manifest-detail paths-only`.**
   - `manifests/<tag>/<field>.manifest.zst` — exact API state.
   - `.manifest.meta.json` — stored path, encoding, digests, reconciliation.
   - `.paths.tsv` — greppable path-level index.
   - No `.manifest.json` / `.manifest.json.gz` by default.

2. **`--manifest-detail full` remains available for diagnostics and writes plain
   `.json`.** Gzip is not the default and, if retained at all, is an explicit
   opt-in. Disk is not the objective; offline readability is.

3. **`--gzip-threshold` stays accepted for compatibility and is recorded, but
   does not fire on a normal capture.** If the flag is eventually removed, that
   is a separate CLI/schema decision.

4. **Do not remove or shrink the raw API/manifest/chunk records.** They are the
   "state of the API" half of the bundle and the bridge's actual input.

5. **Keep `game/`, `expected/`, patchmaps, matrix and verify documents.** They
   are the fixture/ground-truth half and the precondition.

Expected default result on this bundle: **~2.4 s CPU, 76 MiB, 76 files**, versus
**~7 s CPU, 98 MiB, 80 files** today. Full diagnostic mode becomes ~3.5 s and
167 MiB instead of ~7 s and 98 MiB — larger on disk, but much cheaper and more
readable, which matches the stated objective.

---

## 7. Risks, behaviour changes, validation

### Risks

| risk | assessment | mitigation |
|---|---|---|
| A consumer directly reads `manifests/*/game.manifest.json[.gz]` | not the in-repo bridge; possible external reader | keep `--manifest-detail full`; document the opt-in |
| Default bundle loses nested chunk fields from the plain JSON | raw archive and meta remain exact; paths TSV covers the common path-level view | decode the archive offline, or request full mode |
| Full mode becomes larger on disk | +73 MB for the measured fixture | full mode is diagnostic; default remains 76 MiB |
| `MANIFEST.json` / checksums / README counts change | expected and correct for a different file set | regenerate one reference bundle; update pinned expectations |
| A16 or close-out accidentally weakened while chasing CPU | high consequence: `verify` becomes less trustworthy | do not share the two hash passes; keep `tests/bundle_diff.rs`, schema pins and A16 tests |
| Assembly streaming comparison changes A9 semantics | medium | keep `new.bytes` as the comparison target; add a test that a one-byte patch-flow difference still fails A9 |

### Validation steps

1. `cargo build --release -p gircap`.
2. Capture a paths-only bundle into scratch:
   `--manifest-detail paths-only --no-log-file --output json`.
3. `gircap verify --artifact <scratch> --no-log-file --output json` → expect
   63/63 checks and exit 0.
4. Run `cargo test -p girpr --test artifact_bridge` against that bundle (using
   the suite's normal bundle selection, or the same scratch directory copied
   into the discovery root). The bridge must pass without any full manifest
   decode.
5. Run `cargo test -p gircap` (or nextest) after changing the default, and
   `tests/capture_layout.rs` / `tests/bundle_diff.rs` for the new file set.
6. Keep one full-detail capture as a regression check: it must still verify, and
   its `MANIFEST.json → files[]` must list the four `.json` documents.
7. If gzip remains as an opt-in, verify one gzipped full bundle too; do not let
   both encodings rot.

### What not to do

- Do not drop the raw `.manifest.zst` archive or recompress it; it is the exact
  API state and the bridge's route target.
- Do not drop `paths.tsv` under paths-only; it is the only path-level document
  left when the full decode is absent.
- Do not remove `expected/` merely because the runtime assertion uses
  `result_md5`; it is the offline ground truth and the `result_path` target.
- Do not remove A16, replace it with a hash reuse, or stop hashing a file just
  because it was written by the tool.
- Do not optimise by lowering `--jobs`; CPU is flat across the job counts
  measured here, and wall time is filesystem-bound.

---

## 8. Suggested implementation order

| order | change | expected effect | effort |
|---|---|---|---:|
| 1 | default `ManifestDetail::PathsOnly`; update CLI tests/docs | −4.4–5.0 s capture, −0.6 s verify | small |
| 2 | full mode writes plain `.json`; gzip opt-in only | −~0.24 s verify, −3.8 s capture when full is explicitly used | small–medium |
| 3 | `write_json` returning byte count; drop the pre-pass on the plain path | −0.1–0.2 s full mode | small |
| 4 | direct-copy / streaming patch flow | −0.05–0.15 s | medium |
| 5 | `paths_tsv` sort-by-reference cleanup | below noise | tiny |

Step 1 captures nearly all of the available default-CPU win and is the only
change needed to hit the ~2.4 s target. Steps 2–3 clean up the opt-in mode;
step 4 is worthwhile only after the default path is settled. Step 5 is a
readability cleanup, not a benchmark item.

---

## 9. Reproduction notes

- Reference verify:
  ```text
  .\target\release\gircap.exe verify ^
    --artifact artifacts\hk4e_global\7.0.0__7.1.0\20261029T124809.314540Z ^
    --no-log-file --output json
  ```
- Capture variants used the replay server at
  `E:\Symlinks\Temp\gircap_bench\replay.py`, with the reference bundle as its
  read-only source.
- CPU was read from `System.Diagnostics.Process.TotalProcessorTime`; stage
  snapshots were taken from the existing `tracing` milestones. The sampler's
  write/fetch row is deliberately combined because there is no log line between
  `emit::tree::write` and `chunk store done`.
- Bundle sizes are sums of regular files under the artifact root. `MiB` means
  1,048,576 bytes; the raw byte totals are 102,421,534 (full/gzip),
  175,620,641 (full/no-gzip) and 79,718,431 (paths-only).
- No repository source or artifact was changed for this report. Scratch bundles
  and benchmark scripts live only under `E:\Symlinks\Temp\gircap_bench`.
