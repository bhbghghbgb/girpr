# girpr — Genshin Impact low-disk repair patcher

A CLI tool that brings a possibly-corrupt, possibly-any-older-version Genshin Impact install
straight to the current live version while using as little extra disk as possible.
Unattended by design: args in, exit code + logs + `SUMMARY` line out, no UI.

Approach: **Starward's Repair-in-Chunk-mode as the patching core** (version-agnostic,
per-file `{path}_tmp` streaming repair with verified local-chunk reuse and atomic promote),
**plus Collapse's extra-file purge** as a pre/post phase. See `docs/01-findings-and-verdict.md`
for the full comparison, `docs/02-repair-chunk-spec.md` for the language-agnostic spec, and
`docs/03-rust-implementation-plan.md` for the Rust module plan.

## Requirements

- Rust 1.80+ (`cargo build`)
- Network access to HoYoPlay + Sophon CDNs
- The game directory (read/write); only `max_file_size` free bytes beyond the game are needed
  with `--io-threads 1` (peak transient ≈ `io_threads × largest file`)

## Build

```powershell
cargo build --release
```

## Usage

```powershell
# Preview what would change (writes nothing)
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-after --dry-run

# Verify only (writes nothing); exit 4 if anything is damaged
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --check-only

# Repair in place, purge unknown extra files afterwards
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-after --io-threads 4
```

### Options

| Flag | Default | Meaning |
|---|---|---|
| `--game-path <DIR>` | (required) | Game install directory (contains `config.ini`, `*_Data`, …) |
| `--biz <BIZ>` | (required) | `hk4e_cn` \| `hk4e_global` \| `hk4e_bilibili` |
| `--audio <LANG>`… | keep current, else `en-us` | Audio languages to keep (repeatable): `zh-cn`, `en-us`, `ja-jp`, `ko-kr`, or `none` for game-only (no audio). If omitted, the audio scan file is read; if that is missing/unreadable, falls back to `en-us` so automation gets a launchable game. An explicit selection (including `none`) overwrites the scan file |
| `--io-threads <N>` | `4` | Concurrent **files** (chunks within a file are sequential → HDD-friendly). SSD: 4–8, HDD: 1–2 |
| `--purge-after` | off | After patching, delete every file not in the live manifest (Collapse files-cleanup parity) |
| `--purge-before` | off | Same files-cleanup as `--purge-after`, but run *before* patching (frees space for the repair itself) |
| `--check-only` | off | Verify size+MD5 of every file and report; writes nothing |
| `--dry-run` | off | Log actions without writing anything |
| `--json-summary` | off | Emit the begin `REPORT` and final `SUMMARY` stdout lines as JSON |
| `--log-level <LVL>` | `info` | `error`, `warn`, `info`, `debug`, `trace` (`RUST_LOG` also honored) |

### Exit codes (automation-friendly)

| Code | Meaning |
|---|---|
| `0` | Success (check-only: everything intact) |
| `1` | Usage / config error (bad args, unknown audio lang, `--audio none` mixed with langs, `--io-threads < 1`, unusable `--game-path`, game in legacy FILE mode) |
| `2` | Metadata / network error (HoYoPlay/Sophon unreachable, non-zero retcode, bad manifest, checksum mismatch) |
| `3` | Write / verify error — two mechanisms, same code: counted per-file failures (`Ok(summary, 3)` with `SUMMARY … failed>0`) vs fatal write failures (`Err(RunFailure::write)` with `FATAL` only, no `SUMMARY`): a file failed final MD5 after retries, promote/rename failed, `config.ini` / audio-scan write failed, files-cleanup failed |
| `4` | Check-only found damage |

Each fatal error is classified at the site that raises it (`run` → `RunFailure`), so the
right code exits even for post-phase write failures — code `3` is not limited to
the per-file repair loop. Per-file failures instead continue across files, skip the
post-phase (no deprecated/audio/purge-after/`config.ini` bump, so the version is not
marked latest while damaged), and return `Ok(summary, 3)` so automation still gets the
machine-readable `SUMMARY` with `failed` counts.

Progress and diagnostics go to **stderr** (structured `tracing` logs, level from
`--log-level` plus `RUST_LOG`) and to a `TRACE`-level log file
`<exe-dir>/logs/girpr_<timestamp>.log` beside the binary (see `src/main.rs`; created
on startup, appended). After the
server metadata calls the tool writes a one-time
`REPORT local_version=<v|none> latest_version=<v> biz=… exe=… download_mode=… branch=… package_id=… build_id=… audio_langs=… diff_tags=…`
line to **stdout** (also mirrored to the log) so a parser sees both the on-disk
version (`local_version`, from `config.ini`) and the "will be updated to"
version (`latest_version`, from `getGameBranches`), plus metadata that does **not**
come from config — the rest of the fields are sourced from the HoYoPlay/Sophon
APIs; `audio_langs` is the effective set the run will keep. The final
`SUMMARY total=… skipped=… repaired=… failed=… download_bytes=… deleted_extra_bytes=… exit=…`
line also goes to **stdout** for easy parsing **and is mirrored to the log**, so a
stderr-only capture still keeps it. During long phases a
`PROGRESS done=<done>/<total> skipped=… repaired=… failed=… download_bytes=…`
line is emitted to **stdout every 10s (also mirrored to the log)** — from a
background reporter during repair, and inline during `--check-only`
(`skipped` = intact files so far, `failed` = bad files so far). With
`--json-summary`, the `REPORT` and `SUMMARY` lines are emitted as JSON objects
instead.

### Logging & correlation

Each file task logs inside a `file{seq,total,task,path}` span (`task` = tokio async-task id,
stable across worker-thread hops), so `grep 'path=<file>'` groups its lifecycle:
`check start` (expected size/md5) → outcome with actuals → per-chunk trace
(reuse-hit vs download, byte counts) → `after: tmp ready` → `repaired` with chunk stats.
API calls log the full request URL + retcode at debug, full JSON bodies at trace.
The same events also go to the `<exe-dir>/logs/` file above (always `TRACE`), so
`--log-level` only affects the stderr view.

## How it keeps disk usage low

1. **Files-cleanup (Collapse parity)**: with `--purge-before` / `--purge-after`,
   every file not in the live manifest is deleted — temps (`*_tmp`, `*.hdiff`,
   `chunk/`, `ldiff/`, `staging/`), orphans, unselected audio, `ScreenShot/`,
   logs. Only `config.ini` + manifest files + `audio_lang_*` /
   `Audio_*_pkg_version` are kept. The two flags run the same cleanup; only
   the timing differs (before frees space for the repair itself).
2. **Skip intact files**: size + full MD5 check first — 0 bytes downloaded for healthy files.
3. **Chunk-level repair**: only missing/corrupt chunks are fetched; unchanged chunks are copied
   from verified local slices (hash-gated, with network fallback).
4. **Per-file temp + atomic move**: transient cost is one `_tmp` beside the file being repaired —
   no whole-game duplicate, no retained blob stores, no zip staging.
5. **Post-phase**: deprecated files, audio cache→res move, and (with
   `--purge-after`) the files-cleanup; `config.ini` is bumped to the latest version.

v1 expected-set gap: the cleanup compares against `{latest Sophon manifest
paths}` only — no dispatcher persistent union, no SDK/WPF/plugin zips (game
launches without them).

## Project layout

```text
src/main.rs    logging init, exit codes, SUMMARY output, orchestration
src/lib.rs     CLI definition (clap Args), module root
src/biz.rs     biz/channel/launcher/game-id mapping
src/hyp.rs     HoYoPlay client (getGameConfigs/getGameBranches/getBuild/getDeprecated)
src/sophon.rs  Chunk-manifest protobuf, fetch+verify+parse, manifest filtering
src/repair.rs  Work-list build, per-file repair, files-cleanup purge
src/util.rs    MD5 helpers, config.ini read/write
docs/          findings & verdict, language-agnostic spec, Rust plan
```

## Limitations (v1)

Chunk-repair only: no hdiff fast-update, no 7z legacy path, no predownload, no SDK/WPF/plugin
downloads, no dispatcher persistent revisions, no speed limiter. Reruns are safe (idempotent:
intact files skipped, partial `_tmp` resumed by length and re-verified by final MD5).

## V1 simplifications vs Starward's chunk path (deliberate)

The patching core ports Starward's repair-in-chunk-mode, with three deliberate
simplifications. They trade peak RAM / re-download bytes for lower disk use and
a smaller implementation. Do not "fix" them without reading this first.

- **S1 — Whole-blob buffering.** Manifests and chunks are fetched and
  zstd-decoded fully in memory (`decode_all`), and manifests are never cached
  inside `game_dir`. Starward instead streams chunks through a
  `Pipe + DecompressionStream` pipeline
  ([GameInstallHelper.cs](https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L374-L412))
  and caches manifests under an app cache dir
  ([GamePackageService.cs](https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L822-L864)).
  Pro: no retained `chunk/` blob store, no in-game manifest cache, simpler code.
  Tradeoff: higher peak RAM per active file (one chunk + pipe buffers ×
  `io_threads`); very large manifests/chunks could pressure memory — stream
  them if that ever bites.
- **S2 — Same-file-only chunk reuse.** A chunk is reused locally only when the
  same path in the local build carries the same `(uncompressed_md5,
  uncompressed_size)` slice. Starward maps each chunk to *any* local file via
  `OriginalFileFullPath / OriginalFileOffset` cross-file dedup
  ([GameInstallFile.cs](https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallFile.cs#L100-L131),
  [GameInstallHelper.cs](https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L347-L360)).
  Pro: reuse map is per-path and tiny; covers ~all Genshin wins (same-file
  chunk stability). Tradeoff: moved/renamed content re-downloads instead of
  being sourced from another local file.
  Future note (cross-file reuse, not implemented): if chunk reuse is ever
  extended to Starward-style any-file dedup, `--purge-before` would destroy
  those reuse sources — it deletes old-only files against the live manifest,
  so a moved `A(old) → B(new)` would lose `A` before `B` could copy from it
  (correctness still holds via download fallback, but the bytes are
  re-downloaded). `--purge-before` stays destructive
  by design — disk-critical runs accept the re-download cost.
- **S3 — Resume-by-length with final-MD5 gate.** An existing `_tmp` prefix is
  kept based on length alone (`cur_len >= offset + size` skips the chunk);
  only the final whole-file MD5 decides promotion. Same as Starward's
  `fs.Length < chunk.Offset + chunk.UncompressedSize` check
  ([GameInstallHelper.cs](https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GameInstallHelper.cs#L342-L343)).
  Pro: cheap resume, no per-chunk manifest of completed ranges. Tradeoff: a
  corrupt prefix is only caught at the end (wasted work, then retry from
  network via the hash-gated reuse fallback).
