# Repair spec - Repair-by-Chunk (language agnostic) - Genshin low-disk patcher

Goal: bring a possibly-corrupt, possibly-any-older-version Genshin install to the current live
version with minimal extra disk usage, unattended (args + exit code + logs).

Non-goals (v1): hdiff/patch-diff updates, 7z package installs, predownload, speed limit, SDK/WPF
downloads, dispatcher persistent revisions, quota dialogs, hardlinks, UI/progress bars.

## 0. Constants (Genshin)

- HoYoPlay hosts: CN `https://hyp-api.mihoyo.com`, global `https://sg-hyp-api.hoyoverse.com`,
  path prefix `/hyp/hyp-connect/api/`. Query always `?launcher_id={L}&language=en-us`.
  Launcher IDs: CN `jGHBHlcOq1`, global `VYTpXlbWo8`, bili `umfgRO5gh5`.
  Game IDs: `hk4e_cn=1Z8W5NHUQb`, `hk4e_global=gopR6Cufr3`, `hk4e_bilibili=T2S0Gz4Dr2`.
- Sophon hosts: CN `https://downloader-api.mihoyo.com/downloader/sophon_chunk/api/`,
  global `https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/`.
- Audio matching fields: `zh-cn, en-us, ja-jp, ko-kr` (also `mini-*` variants, treated as audio).
  Base field is `game`.
- Hashes are lowercase-hex **MD5**; every file and every chunk carries `(size, md5)`.
  Chunks additionally carry `(offset, compressed_size, uncompressed_size, compressed_md5)`.

## 1. Inputs / outputs

Inputs: `game_dir`, `biz ∈ {hk4e_cn, hk4e_global, hk4e_bilibili}`,
`audio_langs ⊆ {zh-cn,en-us,ja-jp,ko-kr}` or explicit empty (`--audio none` = game-only;
default when omitted: keep currently selected, else `en-us`, see Step 1). `--audio` values are
matched case-insensitively, `_` is accepted for `-`, and the display names `chinese` /
`english` / `english(us)` / `japanese` / `korean` are aliases; anything else is a usage error
(exit 1), as is mixing `none` with a language. Unknown flags, missing/duplicate-incompatible
values and `io_threads = 0` are also exit 1 (clap-level errors included); `--help` is exit 0.
`io_threads ≥ 1` (default 4; HDD → 1–2, SSD → 4–8), flags `--purge-before`, `--purge-after`,
`--check-only`, `--dry-run`.
Outputs: game at latest version; `config.ini:game_version=<latest>`; exit `0` ok,
`1` usage/config error, `2` metadata/network error, `3` write/verify error,
`4` check-only found damage; a begin `REPORT` line (versions + API-sourced
fields, Step 2.5) and a machine-readable final summary line (counts + bytes),
both mirrored to the log, plus structured logs (per-file start/skip/repair/fail
with reason). Fatal errors carry their class at the raise site (`config::RunFailure`), so the
exit code matches the class even for post-phase write failures. Per-file verify
failures are the exception: they continue across files and return `Ok(summary, 3)`
(counted, with `SUMMARY … failed>0`) instead of `Err(RunFailure::write)` (fatal,
`FATAL` only, no `SUMMARY`).

## 2. Procedure

### Step 0 — Prepare `game_dir`
1. If `game_dir` missing → create (fresh install path; local version = none).
   The files-cleanup is NOT run here: its expected set needs the latest manifest
   paths (Steps 3–4), so `--purge-before` executes as Step 5.

### Step 1 — Read local state
1. Parse `game_dir/config.ini` for `game_version=` (last match wins); a missing
   file or no match → `none`. A file that exists but cannot be read is an error
   (exit 1), not a fresh install: treating it as one would let the post-phase
   overwrite the version record.
2. Read `res_category_dir` ignore list if game config names it (see step 2): file of JSON lines
   `{"category":"<matching_field>","is_delete":true}` → ignore set. An absent file is an
   empty set; an unreadable one is an error. Same rule for the blacklist file.
  3. Determine effective audio langs: explicit flag wins, including `--audio none`
   (explicit empty = game-only, scan file overwritten with an empty list); else read
   audio scan file (`AudioPackageScanDir`, e.g. list of `Chinese|English(US)|Japanese|Korean`
   → map to `zh-cn|en-us|ja-jp|ko-kr`). A **missing** scan file defaults to `{en-us}`
   automation so the game launches — Starward/Collapse use game-only when the scan file
   is missing, and `none` opts into that behavior explicitly).
   A **readable empty** file is the game-only selection written by `--audio none` and is
   kept as-is. An **unreadable** file is an error, not a default. A non-empty file
   whose lines name none of the four display names is also an error (exit 1) naming
   the file and the first unrecognized line — an empty result would read as game-only
   and hide the selected language from the purge keep set. An unrecognized line beside
   a recognized one is warned and ignored; a leading UTF-8 BOM is stripped before
   parsing. A file whose only content is a BOM is non-empty and is refused; genuinely
   blank lines are game-only.
   Write the scan file back when an explicit selection was given (including `none`),
   except in `--dry-run` / `--check-only` / `--print-plan` (no writes) or when the scan
   dir is unconfigured. The write happens in the execution phase, after every refusal
   in the plan half, so a refused run leaves the install untouched. It is atomic
   (temp + rename in the same directory).

### Step 2 — Fetch server metadata
1. `GET getGameConfigs?launcher_id=&language=&game_ids[]=` → pick entry for game id. Keep:
   `audio_scan_dir, audio_cache_dir, audio_res_dir, res_category_dir, blacklist_dir (+enable flag),
   default_download_mode`. Assert chunk-capable: `default_download_mode` must be exactly
   `DOWNLOAD_MODE_CHUNK` (the only value this tool can patch — `DOWNLOAD_MODE_FILE` and
   `DOWNLOAD_MODE_LDIFF` need the 7z/hdiff paths v1 does not implement). Anything else, including
   an empty/missing value, aborts with exit 1 rather than failing later at `getBuild`.
2. `GET getGameBranches?launcher_id=&language=&game_ids[]=` → pick entry for game id →
   `main{package_id,branch,password,tag,diff_tags[]}`. `latest = main.tag`.
3. `GET getBuild?branch=&package_id=&password=` (no `tag`) → latest chunk build
   `{build_id,tag,manifests[]}`. If server returns retcode -202 → abort (Genshin must be chunk mode).
4. If local version known: `GET getBuild?...&tag={local}` → local chunk build, best-effort
   (failure → proceed with `local=null`; costs only dedup, not correctness).
5. Emit the begin `REPORT` line to stdout (mirrored to the log), so a parser sees
   the on-disk version and the target version before any repair starts:
   `REPORT local_version=<v|none> latest_version=<latest> biz=… exe=… download_mode=… branch=… package_id=… build_id=… audio_langs=… diff_tags=…`
   (`local_version` and the effective `audio_langs` from Step 1; the rest from
   the calls above — API-sourced, not from config). Under `--output json` this is
   a JSON object instead.
 6. `GET getGameDeprecatedFileConfigs?...&channel=&sub_channel=` → deprecated file names
   list. This is the lazy call: fetched best-effort during the post-phase (Step 8), never in
   `--check-only`/`--dry-run`; a failure only warns. `channel`/`sub_channel` are the
   **per-launcher** pair (cn `1/1`, global `1/0`, bili `14/0`, i.e. `LauncherConfig.Channel` /
   `.SubChannel`) — the list is filtered by them, so sending the global pair for cn/bilibili
   returns the wrong set. It is the only one of the three APIs that carries them.

Each HoYoPlay response is `{"retcode":0,"message":"...","data":{"<node>":...}}`; `retcode != 0` → error.

### Step 3 — Fetch + verify + parse manifests (latest [+ local])
For each wanted manifest in latest build (filter: drop `matching_field ∈ ignore_set`; drop every
`*-??` audio field except selected audio langs; keep `game` + selected):
1. `manifest{id,checksum,compressed_size}` + `manifest_download{url_prefix}`, `chunk_download{url_prefix}`.
2. Download `GET {manifest_prefix}/{id}` (single GET, retry ×5 linear backoff).
3. The blob is **streamed**: it is zstd-decoded as it arrives into a buffer sized by the
   API's `uncompressed_size`, so the compressed body is never resident and the decode never
   has to grow the buffer → compute MD5 of **decompressed** bytes → must equal `checksum`
   (case-insensitive); mismatch → retry whole fetch up to 5× with linear backoff → still
   mismatch → abort (exit 2).

   The reported size is a **hint, not a promise**: it is a raw number out of the API's JSON
   and nothing has checked it, so it only ever sizes the buffer (clamped). A blob bigger than
   the hint grows the buffer; a wrong number cannot ask for an impossible allocation.
 4. Protobuf-decode `SophonChunkManifest{chunks: [{file, chunks:[{id,uncompressed_md5,offset,
   compressed_size,uncompressed_size,unknown:int64=6,compressed_md5=7}], is_folder, size, md5}]}`
   (keep opaque field 6; `compressed_md5` is tag 7).
   Drop `is_folder` entries from the work list (create dirs on demand instead).
5. Same for the local build if present (used only for chunk reuse, §4). Cache manifests outside
   `game_dir` (e.g. system temp keyed by manifest id) or keep in memory; never store in `game_dir`.

### Step 4 — Build work list
For each latest file `F{path,size,md5,chunks[]}`:
- `local_F` = same `path` in local build manifest (may be absent/different).
- Build `chunk_plan[]`: for each chunk `C{offset,uncompressed_size,uncompressed_md5}`:
  `reuse = (local_F contains chunk with same (uncompressed_md5, uncompressed_size))`
  → record `(reuse_path=game_dir/local_F.path, reuse_offset)`, else `reuse=null`.
  (Cross-file dedup by md5 is allowed but optional; same-file covers ~all Genshin wins.)
- If blacklist enabled and `game_dir/{blacklist_dir}` exists (JSON-lines `{fileName}`),
  drop listed paths from the work list.

### Step 5 — Files-cleanup before (reclaim space before repairing)
Optional `--purge-before` (Collapse `GenshinInstall.GetUnusedFileInfoList` parity
`Collapse/.../InstallManagement/Genshin/GenshinInstall.cs:177-263`): run the
§Step-7 cleanup *now* — after the plan exists, before any repair — to free space
for the repair itself. Same file set as `--purge-after`; only the timing differs.
Counts to `deleted_extra_bytes`; `--dry-run` only logs; skipped entirely in
`--check-only` (verification must not delete).

### Step 6 — Repair files (bounded parallelism, resume-safe, idempotent)
Concurrency: semaphore of `io_threads` over **files**; chunks within a file strictly sequential
(HDD-friendly). Retries: 5× per file.

Two ways to write a file, chosen per file by `--repair-mode` (default `auto`):

- **in place** (the default): the target is opened `read+write` and only the bytes that are
  wrong are written, at the offsets they belong at. No `_tmp` file is created, and the peak
  extra disk is the file's **growth** rather than its size.
- **temp**: the repair is assembled in `{path}_tmp` and renamed over the target, so the
  **target file is never modified** and the swap is atomic — a repair that fails leaves the
  target exactly as it was. The peak extra disk is the whole file. It is the mode to pick
  when the target must not be touched, and the way a file is written when its target cannot
  be opened for writing.

**Two regimes.** The in-place path is not a defragmentation, and the difference decides the
design. There is no staging area anywhere in it — a reuse source and its destination are
both inside the same file, and the copy between them uses the existing 512 KiB `COPY_BUF`
and nothing else.

- **Corruption repair** — the file is the right size and most chunks are intact. A chunk's
  reuse source is at *its own offset* (`r_i == o_i`), so the bad chunk is simply overwritten
  where it lies. **No bytes move.** There is no ordering constraint and nothing to stage:
  this is a patch, not a relocation.
- **Version upgrade** — the file's content shifted, so a chunk's source is elsewhere and its
  bytes have to physically move from `r_i` to `o_i`. *This* is the defrag-shaped case.

The second regime is the classic "relocate blocks inside a file without a staging area"
problem, and it has exactly two answers: **order the relocations** so no source is
overwritten before it is read, or **stage the moving bytes in RAM**. The first is free and
forces the writes into one direction (a backwards-seek pattern); the second costs RAM and
buys everything else. Which one a file gets is decided per file, so the in-place path is
not one algorithm but two tiers:

| tier | when | extra disk | RAM | reuse | writes | direction |
|---|---|---|---|---|---|---|
| **RAM-staged** | the moving payload fits `--max-ram-bytes` | `0` | the payload | 100 % | only the wrong chunks | ascending |
| **ordered** | the payload does not fit | `0` | `0` | one direction | the wrong chunks | ascending *or* descending |

Both tiers produce **the same file**; they differ in how much reuse they keep and in which
direction they write.

Per file `F`:
1. **Scan** — one read of `game_dir/F.path` that answers everything: the whole-file MD5 (the
   skip check), which chunks are already correct where they belong, and which reuse sources
   are intact. `if len==F.size and md5==F.md5` → **skip** (log `check: md5 match -> skip`,
   count as verified, no download). Else proceed.
2. **Stage** — the bytes of the chunks that have to *move* inside the file are read into RAM
   (bounded by `--max-ram-bytes`), so the write order cannot destroy them. A chunk needs
   staging only if its reuse source is *elsewhere* in the file and the chunk is actually
   wrong: a source at the chunk's own offset is the bad chunk itself, so it is not a source
   at all.

   | regime | wrong chunks | sources elsewhere? | payload |
   |---|---|---|---|
   | corruption repair | a few | no — normally `r_i == o_i`, and that source *is* the bad chunk | **0** |
   | version upgrade | most | yes | ~`S` |
   | mixed re-flow | some | some | the sum over those chunks |

   So the common repair case stages **nothing**. The budget is **run-level**, reserved per
   file the way `--max-write-bytes` reserves, because `io_threads` files are in flight at
   once. A file that cannot get its reservation does not fail: it is written in the ordered
   direction instead, which produces the same file with less reuse.
3. **Patch** — the wrong chunks are written, ascending in the RAM-staged tier and in the
   salvage direction in the ordered tier: from the staged bytes when there are
   any, from a download otherwise. A download is `GET {chunk_prefix}/{C.id}` → response bytes
   are **zstd-compressed** chunk payload → the body is **streamed**: it is decoded as it
   arrives and the decoded bytes are written at `C.offset` as they are produced, digested on
   the way past, so the chunk is never resident as a whole. `uncompressed_size` is enforced
   as the bytes go, and `uncompressed_md5` is verified when the stream ends — mismatch fails
   the attempt and retries the whole file (up to 5×).
4. **Trim** — the file is cut to `F.size`: a shorter file grows, a longer one loses its dead
   tail. A hole in the chunk coverage (a range no chunk claims) is zero-filled.
5. **Verify** — one more read: `if md5==F.md5` the file is good, and the structured `repaired`
   event is logged (`chunks_total`/`chunks_reused`/`chunks_downloaded`/`chunks_kept`,
   `download_bytes`, `final_md5`). Else the per-chunk verdicts name the culprit and only
   those are redone; if they still do not match, the file is counted as failed and the run
   continues — after all files, `Ok(summary, 3)` (skips the post-phase so `config.ini` is
   not bumped while damaged) — not `Err(RunFailure::write)`.
6. A target that cannot be opened for writing is written the temp way, loudly: write
   `{path}_tmp` (kept, not truncated), work only on chunks with
   `offset+uncompressed_size > have` (the already-complete prefix is kept; the final
   whole-file MD5 gates the rename), then `if len(tmp)==F.size and md5(tmp)==F.md5` → atomic
   `rename(tmp → final)` (overwrite), else delete `tmp`, log error, count the file as failed
   and continue.
7. `--check-only`: perform only step 1 for all files and report; write nothing.

**The ordered tier's rule.** Write chunk `i`'s destination `W_i = [o_i, o_i+s_i)` from its
reuse source `R_i = [r_i, r_i+s_i)` in the same file. The `W`s are disjoint and, after the
zero-fill, tile `[0, size)`. So:

- **ascending order** (smallest `o` first): before chunk `i` the written region is
  `[0, o_i)`. `R_i` survives ⟺ `r_i ≥ o_i` (and `r_i+s_i` is inside the file).
- **descending order**: before chunk `i` the written region is `(o_i+s_i, size)`. `R_i`
  survives ⟺ `r_i ≤ o_i` (and `r_i+s_i` is inside the file).

Neither condition mentions any other chunk, so both are **exactly computable at plan
time** — no simulation, no heuristic. A pure function picks whichever direction preserves
more reuse and marks the surviving chunks; the rest download. `r_i == o_i` counts for both.
If the layout has holes, the zero-fill happens *after* the ordered loop, so during the loop
the written region is a subset of the one the rule assumes and both conditions stay
conservative — never wrong, occasionally pessimistic.

A uniform upward shift of a file's content (everything moved later) is the descending case;
a downward shift is the ascending case. A file whose chunking re-flowed has sources in both
directions and keeps the larger half.

Illustration — content shifted one chunk to the right:

```
on disk (old):      [0,4)=AAAA  [4,8)=BBBB  [8,12)=CCCC  [12,16)=DDDD
target  (manifest): [0,4)=0000  [4,8)=AAAA  [8,12)=BBBB  [16-)=CCCC
reuse sources:        none        r=0        r=4          r=8

descending: CCCC@12 <- @8   ✓  r=8  <= o=12
            BBBB@8  <- @4   ✓  r=4  <= o=8
            AAAA@4  <- @0   ✓  r=0  <= o=4
            0000@0  <- --      no source -> download          3/4 salvaged
ascending:  AAAA@4  <- @0   x  r=0  <  o=4  (already overwritten by 0000)
            BBBB@8  <- @4   x
            CCCC@12 <- @8   x                                        0/4
```

**The eager trim.** `trim_is_free(file)` = "no reuse candidate has `r + s > size`". That is
the whole test: an entry in the local reuse map past the target's end can only ever be a
source, and only if the file shrinks a lot. When it is true the tail is dead weight and
`set_len` releases it before a single byte is written. When it is false and `O − S` is
large, the trim is deferred to the end (the file grows during the repair instead) and the
reason is logged.

**How the two modes resume.** In place there is no artefact to resume from and none is
needed: the scan of step 1 finds the chunks that are already correct where they belong, so
the next run rewrites only the rest — and a corrupt prefix is not something it can inherit.
The temp mode resumes by **length**: the completed prefix of `{path}_tmp` is kept and the
next run continues it. A chunk that fails its digest is **rolled back**: the `_tmp` is
truncated to that chunk's offset, so the resume point stays honest — the bytes before it
were each digest-checked when they were written, and everything from it on is gone. The
whole-file MD5 is the gate in both, and the version in `config.ini` is advanced only by a
run in which every planned file verified.

**The invariant.** Every byte written lands in a region that was **already wrong at the
moment of the write**, and the whole-file MD5 decides whether the file is good.

A *downloaded* chunk cannot be hash-gated before it lands: the digest of a decoded stream is
only known when the stream ends, and the stream ends after the last byte has been written.
What replaces the gate is checkable in the code rather than hopeful:

- the in-place path writes only to the chunks the scan found wrong, and to the ones the
  whole-file MD5 still finds wrong after a re-do — so the bytes it lands replace bytes that
  were already bad;
- a downloaded chunk's bytes come from the network, not from the file, so it consumes no
  source region: the salvage rule is not *weakened*, it just loses a candidate when a download
  fails its digest — the region it overwrote may have been another chunk's source, and the
  next scan re-checks every source, so the cost is a download, never a wrong byte;
- the in-place path has **no rollback**, and does not need one: it has no resume artefact, and
  the next attempt's scan finds the chunk wrong and rewrites it. The temp mode is the other
  way round — it resumes by length, so a chunk that failed its digest is rolled back to its
  offset;
- the temp path is safe by construction: the `_tmp` is promoted only after the whole-file
  MD5 passes, and the rollback above keeps its *resume* honest;
- a staged source's MD5 is still checked before it is staged, so that path keeps the
  stronger property it always had.

Why this is "repair": no step assumes the old bytes are intact — a staged source is
hash-gated, and the final whole-file MD5 gates the file. Any-version → latest works
because the plan is derived from the latest manifest alone (local manifest is a pure
optimization).

### Step 7 — Post-phase (deletes + config)

**Starward-handling remainder** (`GameInstallService.cs:ClearDeprecatedFiles` + audio/config):
1. Delete each `deprecated_files[]` path if present (joined under `game_dir`, files only).
   Bytes deleted here count toward the single `deleted_extra_bytes` counter even when
   neither purge flag is set.
   (Redundant when the purge below is enabled — deprecated files are not in the
   live manifest — but keeps them cleaned without any purge flag.)
2. If audio cache dir ≠ res dir and both configured: move `cache/**/*` → `res/<relative>` (overwrite)
   (Starward parity `GameInstallService.cs:427-445,643-661`). Only files are moved;
   leftover empty cache dirs are removed by the files-cleanup's emptied-dir sweep
   whenever either `--purge-before` or `--purge-after` runs (the sweep lives inside
    `repair::purge::purge_extra` and is skipped only in `--dry-run`).
 3. Rewrite `config.ini` (Collapse `GameVersionBase` parity:
    section-preserving surgical update, never a wholesale rewrite):
    every `[section]` survives in first-seen order (duplicates merged, names
    case-insensitive); only keys inside `[General]` (case-insensitive) are
    touched. Force `game_version=<latest>`, `game_biz`,
    `channel/sub_channel/cps` per biz (cn `1/1/hyp_mihoyo`, global
    `1/0/hyp_hoyoverse`, bili `14/0/hyp_mihoyo`; `game_biz` is a deliberate
    Starward carryover — Collapse never writes it, but game/launcher interop
    expects it). `sdk_version` is preserved when present, created empty when
    missing (Collapse only holds the empty default; the real SDK lives in
    `plugin_sdk_version`, never touched). `uapc`/`wpf_version`/`predownload`/
    `plugin_*` are pure passthrough, never created. Comments and blank lines
    are dropped, keys within a section are sorted alphabetically, blank line
    after each section. Create `[General]` if missing.

**Files-cleanup** (optional `--purge-after`, same function as Step 5 —
`GenshinInstall.GetUnusedFileInfoList` parity, v1 scope: expected set is
`{latest Sophon manifest paths}` only, no SDK/WPF/dispatcher union):
1. Build expected set = `{latest manifest paths}` ∪ `{config.ini}`.
   `config.ini` is the ONLY metadata keep: it lives inside `game_dir` but never
   appears in chunk manifests. The exe needs no keep (the game flags unknown
   exes in the folder, so the tool binary must live elsewhere). `audio_lang_*`
   + `Audio_*_pkg_version` are kept by filename pattern (Collapse
   `GenshinInstall.cs:232-241`); everything else not in the manifest purges —
   including temps (`*_tmp`, `*.hdiff`, `chunk/`, `ldiff/`, `staging/`,
   `*.diff`, `*deletefiles*`), unselected audio, `ScreenShot/`, logs, server
   bookkeeping files.
2. Enumerate `game_dir/**/*` (files only); delete anything not in the expected
   set, then remove emptied dirs (dir sweep runs whenever a purge runs with writes
   enabled, i.e. both `--purge-before` and `--purge-after`; skipped in `--dry-run`).
   `--dry-run` only logs candidates + bytes and leaves `deleted_extra_bytes` at 0 by design.
   Counts to the single `deleted_extra_bytes` counter (shared with deprecated-file
   deletes above). `--purge-before` runs
   this same cleanup at Step 5. Manifests are never cached inside `game_dir`; the only
   in-`game_dir` transient is the atomic `config.ini.girpr_tmp` used by the `config.ini` bump.

### Step 8 — Report
Log + stdout summary: `{files_total, files_skipped, files_repaired, files_failed,
download_bytes, deleted_extra_bytes}` (numeric-only; the versions were already
reported at Step 2.5). Exit code per §1. Rerun is safe
(skips intact files, and re-verifies the rest: the in-place path finds the chunks that
are already right where they belong, the temp path resumes the `*_tmp` prefix).

## 3. Disk-usage argument (why minimal)

- Pre/post files-cleanup bounds "before/after" to exactly the live file set + `config.ini`.
- During: transient per active file = the file's **growth** under the in-place path
  (`≤ 1 × file size` under `--repair-mode temp`, where the `_tmp` grows beside the old
  file) plus pipe buffers; no `chunk_collapse/` blob store, no `ldiff/` store, no zip
  staging, no in-game manifest cache. Peak ≈ `io_threads × largest growth`, which is 0
  for a same-size repair. Under `--repair-mode temp` the peak is
  `io_threads × largest_file`, and with `io_threads=1` the tool needs only
  `max_file_size` free bytes beyond the game (vs 2× game for zip-based installers).
- Network-optimal without extra disk: unchanged files skipped (0 bytes), changed files fetch only
  missing chunks (dedup via local manifest), reused slices copied locally (no download, no extra file).

The per-mode model, with `S` = target size, `O` = current on-disk size (0 if absent) and
`G` = growth:

| mode | bytes written per repaired file | peak extra free space | if the run is interrupted |
|---|---|---|---|
| `inplace` (default) | only the chunks that are wrong *or moved* | `max(0, S − min(O, S))` | a mixed file, re-verified next run |
| `temp` | `S` — the whole file | `S`, so `jobs × S_max` across a run | the old file untouched + a partial `*_tmp` |
| `discard` | `S` | `0` | a partial file |

The rows are two ways to spend disk, not two rungs of a safety ladder: `inplace` spends the
least and writes the least, and `temp` spends the whole file in exchange for the target
never being modified. For a corrupt-but-right-sized file — the common repair case —
`inplace` needs **zero** extra space and writes only the corrupt bytes. For a file that
grows by `Δ`, `inplace` needs `Δ`, not `S`. For a fresh install nothing is saved because
nothing is on disk to save.

**The bytes the repair did not have to touch.** `SUMMARY kept_bytes` is the run's disk
claim in one number: the bytes that were already correct where they belonged and did not
have to be written. It is the complement of the **settled** writes — the bytes that landed
in the install from a source that passed its gate — and the settled writes are the
*file's*, accumulated across its attempts: a chunk an abandoned attempt wrote is in the
install, so the next attempt re-verifies the file, finds that chunk correct, and does
not write it again. A counter derived from the *last* attempt's writes would claim those
bytes were kept, and the run would report touching fewer bytes than it had.

## 4. Failure modes

| Failure | Handling |
|---|---|
| Manifest MD5 mismatch | retry whole fetch up to 5×, then abort exit 2 (never patch from bad metadata) |
| Chunk download corrupt (chunk MD5 fail) | the attempt fails and the file is retried (×5): the in-place path re-scans, finds the chunk wrong and rewrites it; the temp path rolls the chunk back to its offset and re-downloads it, keeping the good prefix. The file is then gated by the whole-file MD5, and a file that still fails is counted and the run exits 3 |
| Whole-file MD5 mismatch | the temp path deletes `_tmp` and retries the file ×5; the in-place path redoes only the chunks the verdicts name; then fail file exit 3 |
| Reused slice mismatch | treat as cache miss → download that chunk |
| Unknown local version | `local=null` → all chunks download-on-demand; still correct |
| Missing server diff support (-202) | abort (Genshin is chunk-only; no legacy path in v1) |
| Interrupted run | the in-place path re-verifies and continues (the chunks that are already right stay right); the temp path resumes by length; a leftover `*_tmp` is purged only via `--purge-before`/`--purge-after` |
| A manifest whose chunks do not tile the file | validated at plan time; the hole is zero-filled and logged |
| Power loss mid-repair | the whole-file MD5 fails on the next run, which re-verifies and continues |
| The volume fills while a file grows | the file fails, is counted, retried and reported; `--repair-mode temp` is the mode for a volume with no room for the growth |
| A read-only attribute on a game file | cleared before the write, the way the launcher does it |
| Two `girpr` runs against one install | the single-writer lock refuses the second one (exit 1) by name and by pid |

## 5. What was deliberately excluded (and when to add)

- hdiff/patch-diff updates (faster downloads, needs intact source + `ldiff/` space) — add as
  `--update-fast` later; repair remains the fallback.
- 7z/legacy package path, predownload, SDK/WPF/plugin zips, dispatcher persistent files,
  `ctable.dat`, quota dialogs, hardlinks, speed limiter — launcher conveniences; game launches
  without them. Add `--with-sdk/--with-wpf` only if a channel's launch check proves otherwise.
