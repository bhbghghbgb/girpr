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
`audio_langs ⊆ {zh-cn,en-us,ja-jp,ko-kr}` (default: keep currently selected, see §3),
`io_threads ≥ 1` (default 4; HDD → 1–2, SSD → 4–8), flags `--purge-extra`, `--check-only`, `--dry-run`.
Outputs: game at latest version; `config.ini:game_version=<latest>`; exit `0` ok,
`1` usage/config error, `2` network/metadata error, `3` verify/write error,
`4` check-only found damage; machine-readable final
summary line (counts + bytes) plus structured logs (per-file start/skip/repair/fail with reason).

## 2. Procedure

### Step 0 — Pre-clean (reclaim space first)
1. If `game_dir` missing → create (fresh install path; local version = none).
2. **Starward-handling temp sweep** (extended: Starward's `ClearDeprecatedFiles`
   `GameInstallService.cs:722-777` only sweeps post-task when `PredownloadVersion is null`; we also
   sweep pre-task): delete inside `game_dir`: `**/*_tmp`, `**/*.hdiff`, dirs `chunk/`, `ldiff/`,
   `staging/`. Log bytes/entries freed to `freed_temp_bytes` (NOT `deleted_extra_bytes`).
   Never touch `ScreenShot/`, `config.ini`, `audio_lang_*`.
3. **Collapse-handling purge-before** (optional `--purge-before`, repair-time `CheckRedundantFiles`
   parity `Collapse/.../RepairManagement/Genshin/Check.cs:26-67,232-319`): run the §Step-6C purge
   now to free space for the repair itself. Counts to `deleted_extra_bytes`; `--dry-run` only logs.

### Step 1 — Read local state
1. Parse `game_dir/config.ini` for `game_version=` (last match wins); absent/unparseable → `none`.
2. Read `res_category_dir` ignore list if game config names it (see step 2): file of JSON lines
   `{"category":"<matching_field>","is_delete":true}` → ignore set.
3. Determine effective audio langs: explicit flag wins; else read audio scan file
   (`AudioPackageScanDir`, e.g. list of `Chinese|English(US)|Japanese|Korean` → map to
   `zh-cn|en-us|ja-jp|ko-kr`); else default `{en-us}` + any already-present audio manifest files' langs.
   Write the scan file back when an explicit selection was given.

### Step 2 — Fetch server metadata (up to 5 calls: 4 required + local best-effort)
1. `GET getGameConfigs?launcher_id=&language=&game_ids[]=` → pick entry for game id. Keep:
   `audio_scan_dir, audio_cache_dir, audio_res_dir, res_category_dir, blacklist_dir (+enable flag),
   default_download_mode`. Assert chunk-capable (Genshin is); abort otherwise.
2. `GET getGameBranches?launcher_id=&language=&game_ids[]=` → pick entry for game id →
   `main{package_id,branch,password,tag,diff_tags[]}`. `latest = main.tag`.
3. `GET getBuild?branch=&package_id=&password=` (no `tag`) → latest chunk build
   `{build_id,tag,manifests[]}`. If server returns retcode -202 → abort (Genshin must be chunk mode).
4. If local version known: `GET getBuild?...&tag={local}` → local chunk build, best-effort
   (failure → proceed with `local=null`; costs only dedup, not correctness).
5. `GET getGameDeprecatedFileConfigs?...&channel=&sub_channel=` → deprecated file names list.

Each HoYoPlay response is `{"retcode":0,"message":"...","data":{"<node>":...}}`; `retcode != 0` → error.

### Step 3 — Fetch + verify + parse manifests (latest [+ local])
For each wanted manifest in latest build (filter: drop `matching_field ∈ ignore_set`; drop every
`*-??` audio field except selected audio langs; keep `game` + selected):
1. `manifest{id,checksum,compressed_size}` + `manifest_download{url_prefix}`, `chunk_download{url_prefix}`.
2. Download `GET {manifest_prefix}/{id}` (single GET, retry ×5 linear backoff).
3. Zstd-decompress entire blob → compute MD5 of **decompressed** bytes → must equal `checksum`
   (case-insensitive); mismatch → retry whole fetch up to 5× with linear backoff → still mismatch → abort (exit 2).
4. Protobuf-decode `SophonChunkManifest{chunks: [{file, chunks:[{id,uncompressed_md5,offset,
   compressed_size,uncompressed_size,compressed_md5}], is_folder, size, md5}]}`.
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

### Step 5 — Repair files (bounded parallelism, resume-safe, idempotent)
Concurrency: semaphore of `io_threads` over **files**; chunks within a file strictly sequential
(HDD-friendly; also bounds peak disk to `io_threads × largest_file`). Retries: 5× per file.

Per file `F`:
1. `if exists(game_dir/F.path) and len==F.size and md5(file)==F.md5` → **skip** (log `skip`, count
   as verified, no download). Else proceed.
2. Ensure parent dir. Open `game_dir/F.path_tmp` (`{path}_tmp`) with `OpenOrCreate|ReadWrite`.
   Let `have = len(tmp)`. If `have > F.size` → truncate to `F.size` (corrupt resume). Work only on
   chunks with `offset+uncompressed_size > have` (already-complete prefix is kept; it is re-verified
   by the final whole-file MD5).
3. For each pending chunk `C` in offset order, seek `tmp` to `C.offset`:
   a. If `C.reuse` set and `exists(reuse_path)` with expected length: read
      `reuse_path[reuse_offset .. +uncompressed_size]`, MD5 it; if equals `C.uncompressed_md5` →
      copy bytes to `tmp` at `C.offset`, continue (no network). Mismatch → fall through to (b).
   b. `GET {chunk_prefix}/{C.id}` → response bytes are **zstd-compressed** chunk payload.
      Stream-decompress while writing at `C.offset` (do not buffer whole file; 8–512 KiB pipe buffer).
      Optional: MD5 the decompressed bytes and compare to `C.uncompressed_md5` immediately
      (early retry); mandatory: rely on step 4's whole-file check.
   c. Any chunk failure → retry whole file (up to 5); resume keeps `tmp` prefix.
4. Close `tmp`. `if len(tmp)==F.size and md5(tmp)==F.md5` → atomic `rename(tmp → final)` (overwrite),
   log `repaired {downloaded_bytes}`. Else delete `tmp`, log error, mark file failed (exit 3).
   Never modify the original in place; never leave `tmp` behind on success.
5. `--check-only`: perform only step 1 for all files and report; write nothing.

Why this is "repair": no step assumes the old bytes are intact — reused slices are hash-gated and
the final whole-file MD5 gates promotion. Any-version → latest works because the plan is derived
from the latest manifest alone (local manifest is a pure optimization).

### Step 6 — Post-phase (deletes + config)
Split by provenance; keep the two byte counters separate.

**6S — Starward-handling** (`GameInstallService.cs:ClearDeprecatedFiles` L722-777 + audio/config):
1. Delete each `deprecated_files[]` path if present (joined under `game_dir`, files only).
   Re-run the Step-0 temp sweep (catches interrupted runs).
2. If audio cache dir ≠ res dir and both configured: move `cache/**/*` → `res/<relative>` (overwrite),
   then remove emptied cache tree (prevents stranded duplicates; Starward parity
   `GameInstallService.cs:427-444,643-660`).
3. Rewrite `config.ini`: preserve all existing keys except force
   `game_version=<latest>` (+ `game_biz`, `channel/sub_channel/cps` per biz:
   cn `1/1/hyp_mihoyo`, global `1/0/hyp_hoyoverse`, bili `14/0/hyp_mihoyo`). Create with `[General]`
   header if missing.

**6C — Collapse-handling purge-extra** (`GenshinInstall.GetUnusedFileInfoList`
`Collapse/.../InstallManagement/Genshin/GenshinInstall.cs:176-263` parity, v1 scope: no
SDK/WPF/dispatcher union — see `docs/01 §3`):
1. Build expected set = `{latest manifest paths}`
   ∪ server-config metadata files (see Step 2 — all live inside `game_dir` but are NEVER in the
   chunk manifests, so without this the purge would delete them):
   `{config.ini, exe per game config, blacklist file, res_category file, audio scan file,
   audio_lang_14 + Audio_<entry>_pkg_version per selected lang (Collapse `GenshinInstall.cs:228-247`
   pattern `^Audio_<entry>_pkg_version$`)}`.
2. Enumerate `game_dir/**/*` (files only); **skip without comparing** (sweep owns them):
   `*_tmp`, `*.hdiff`, anything under `chunk/`, `ldiff/`, `staging/`.
   Delete anything else not in expected set **except** user-data allowlist both launchers never touch:
   `ScreenShot/**`, `log*/**` (tight prefix — must NOT be bare `starts_with("log")`, which matches
   e.g. `login.dat`), `audio_lang_*`, `config.ini`. `--dry-run` only logs candidates + bytes.
   Counts to `deleted_extra_bytes`. Run before Step 5 too when `--purge-before` is set.

### Step 7 — Report
Log + stdout summary: `{latest, files_total, files_skipped, files_repaired, files_failed,
download_bytes, deleted_extra_bytes, freed_temp_bytes}`. Exit code per §1. Rerun is safe
(skips intact files, resumes partial `*_tmp`).

## 3. Disk-usage argument (why minimal)

- Pre/post purge + temp sweep bounds "before/after" to exactly the live file set + tiny metadata.
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
| Interrupted run | `_tmp` resume by length + final MD5; temp sweep on next start |

## 5. What was deliberately excluded (and when to add)

- hdiff/patch-diff updates (faster downloads, needs intact source + `ldiff/` space) — add as
  `--update-fast` later; repair remains the fallback.
- 7z/legacy package path, predownload, SDK/WPF/plugin zips, dispatcher persistent files,
  `ctable.dat`, quota dialogs, hardlinks, speed limiter — launcher conveniences; game launches
  without them. Add `--with-sdk/--with-wpf` only if a channel's launch check proves otherwise.
