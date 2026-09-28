# Repair-by-Chunk Spec (language agnostic) — Genshin low-disk patcher

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
1. Parse `game_dir/config.ini` for `game_version=` (last match wins); absent/unparseable → `none`.
2. Read `res_category_dir` ignore list if game config names it (see step 2): file of JSON lines
   `{"category":"<matching_field>","is_delete":true}` → ignore set.
  3. Determine effective audio langs: explicit flag wins, including `--audio none`
   (explicit empty = game-only, scan file overwritten with an empty list); else read
   audio scan file (`AudioPackageScanDir`, e.g. list of `Chinese|English(US)|Japanese|Korean`
   → map to `zh-cn|en-us|ja-jp|ko-kr`); else default `{en-us}` (girpr-specific default for
   automation so the game launches — Starward/ Collapse use game-only when the scan file
   is missing, and `none` opts into that behavior explicitly).
   Write the scan file back when an explicit selection was given (including `none`),
   except in `--dry-run` / `--check-only` (no writes) or when the scan dir is unconfigured.

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
   the calls above — API-sourced, not from config). With `--json-summary` this is
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
3. Zstd-decompress entire blob → compute MD5 of **decompressed** bytes → must equal `checksum`
   (case-insensitive); mismatch → retry whole fetch up to 5× with linear backoff → still mismatch → abort (exit 2).
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
(HDD-friendly; also bounds peak disk to `io_threads × largest_file`). Retries: 5× per file.

Per file `F`:
1. `if exists(game_dir/F.path) and len==F.size and md5(file)==F.md5` → **skip** (log
   `check: md5 match -> skip`, count as verified, no download). Else proceed.
2. Ensure parent dir. Open `game_dir/F.path_tmp` (`{path}_tmp`) with `OpenOrCreate|ReadWrite`.
   Let `have = len(tmp)`. If `have > F.size` → truncate to `F.size` (corrupt resume). Work only on
   chunks with `offset+uncompressed_size > have` (already-complete prefix is kept; it is re-verified
   by the final whole-file MD5).
3. For each pending chunk `C` in offset order, seek `tmp` to `C.offset`:
   a. If `C.reuse` set and `exists(reuse_path)` with expected length: read
      `reuse_path[reuse_offset .. +uncompressed_size]`, MD5 it; if equals `C.uncompressed_md5` →
      copy bytes to `tmp` at `C.offset`, continue (no network). Mismatch → fall through to (b).
   b. `GET {chunk_prefix}/{C.id}` → response bytes are **zstd-compressed** chunk payload.
      v1 (S1 simplification, see README): buffer the whole chunk, `decode_all`, then verify
      `uncompressed_md5` immediately — mismatch fails the attempt and retries the whole file
      (up to 5×). Only then write the bytes at `C.offset`. A streaming
      `Pipe + DecompressionStream` pipeline (8–512 KiB buffer) may replace this later to
      lower peak RAM; the final whole-file MD5 still gates promotion.
   c. Any chunk failure → retry whole file (up to 5); resume keeps `tmp` prefix.
 4. Close `tmp`. `if len(tmp)==F.size and md5(tmp)==F.md5` → atomic `rename(tmp → final)` (overwrite),
   log the structured `repaired` event (`chunks_total`/`chunks_reused`/`chunks_downloaded`/
   `chunks_resumed`, `download_bytes`, `final_md5`). Else delete `tmp`, log error, count the file as
   failed and continue remaining files; after all files, return `Ok(summary, 3)` (skips the post-phase
   so `config.ini` is not bumped while damaged) — not `Err(RunFailure::write)`.
   Never modify the original in place; never leave `tmp` behind on success.
5. `--check-only`: perform only step 1 for all files and report; write nothing.

Why this is "repair": no step assumes the old bytes are intact — reused slices are hash-gated and
the final whole-file MD5 gates promotion. Any-version → latest works because the plan is derived
from the latest manifest alone (local manifest is a pure optimization).

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
 3. Rewrite `config.ini` (Starward `SetGameConfigIniAsync` parity,
   `GameInstallService.cs:788-849`): Starward copies every line except the
   `[General]` header into an INI stream, forces the keys it owns, and
   re-serializes one `[General]` block. Reproduced here:
   - unknown keys pass through; keys from other `[sections]` survive flattened
     into `[General]` as `Section:Key` (how .NET's INI provider surfaces them),
   - comment (`;` / `#`) and blank lines are dropped, because the provider never
     emits them back,
   - key order is deterministic (file order, then the forced block) instead of
     .NET's dictionary order; the result is still valid and idempotent,
   - force `game_version=<latest>`, `game_biz`, `channel/sub_channel/cps` per biz:
     cn `1/1/hyp_mihoyo`, global `1/0/hyp_hoyoverse`, bili `14/0/hyp_mihoyo`), and
     `sdk_version=` — always empty in v1: Starward writes the channel SDK version or
     `""` for the same key, and v1 does no SDK fetch. TODO(sdk_version): fetch the
     channel SDK and write the real version if a channel ever requires it. Create with `[General]`
     header if missing.

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
(skips intact files, resumes partial `*_tmp`).

## 3. Disk-usage argument (why minimal)

- Pre/post files-cleanup bounds "before/after" to exactly the live file set + `config.ini`.
- During: transient per active file = `≤ 1 × file size` (`_tmp` grows beside the old file) plus pipe
  buffers; no `chunk_collapse/` blob store, no `ldiff/` store, no zip staging, no in-game manifest cache.
  Peak ≈ `io_threads × largest_file`. With `io_threads=1` the tool needs only
  `max_file_size` free bytes beyond the game (vs 2× game for zip-based installers).
- Network-optimal without extra disk: unchanged files skipped (0 bytes), changed files fetch only
  missing chunks (dedup via local manifest), reused slices copied locally (no download, no extra file).

## 4. Failure modes

| Failure | Handling |
|---|---|
| Manifest MD5 mismatch | retry whole fetch up to 5×, then abort exit 2 (never patch from bad metadata) |
| Chunk download corrupt (final MD5 fail) | delete `_tmp`, retry file ×5, then fail file exit 3 |
| Reused slice mismatch | treat as cache miss → download that chunk |
| Unknown local version | `local=null` → all chunks download-on-demand; still correct |
| Missing server diff support (-202) | abort (Genshin is chunk-only; no legacy path in v1) |
| Interrupted run | `_tmp` resume by length + final MD5; leftover `*_tmp` purged only via `--purge-before`/`--purge-after` |

## 5. What was deliberately excluded (and when to add)

- hdiff/patch-diff updates (faster downloads, needs intact source + `ldiff/` space) — add as
  `--update-fast` later; repair remains the fallback.
- 7z/legacy package path, predownload, SDK/WPF/plugin zips, dispatcher persistent files,
  `ctable.dat`, quota dialogs, hardlinks, speed limiter — launcher conveniences; game launches
  without them. Add `--with-sdk/--with-wpf` only if a channel's launch check proves otherwise.
