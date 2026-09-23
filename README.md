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
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-extra --dry-run

# Verify only (writes nothing); exit 4 if anything is damaged
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --check-only

# Repair in place, purge unknown extra files afterwards
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-extra --io-threads 4
```

### Options

| Flag | Default | Meaning |
|---|---|---|
| `--game-path <DIR>` | (required) | Game install directory (contains `config.ini`, `*_Data`, …) |
| `--biz <BIZ>` | (required) | `hk4e_cn` \| `hk4e_global` \| `hk4e_bilibili` |
| `--audio <LANG>`… | keep current | Audio languages to keep (repeatable): `zh-cn`, `en-us`, `ja-jp`, `ko-kr` |
| `--io-threads <N>` | `4` | Concurrent **files** (chunks within a file are sequential → HDD-friendly). SSD: 4–8, HDD: 1–2 |
| `--purge-extra` | off | After patching, delete files not in the live manifest (Collapse parity) |
| `--purge-before` | off | Also purge extra files *before* patching (frees space for the repair itself) |
| `--check-only` | off | Verify size+MD5 of every file and report; writes nothing |
| `--dry-run` | off | Log actions without writing anything |
| `--json-summary` | off | Emit the final summary as JSON instead of `KEY=value` |
| `--log-level <LVL>` | `info` | `error`, `warn`, `info`, `debug`, `trace` (`RUST_LOG` also honored) |

### Exit codes (automation-friendly)

| Code | Meaning |
|---|---|
| `0` | Success (check-only: everything intact) |
| `1` | Usage / config error (bad args, unknown audio lang, …) |
| `2` | Metadata / network error (HoYoPlay/Sophon unreachable, bad manifest) |
| `3` | Verify / write error (a file failed final MD5 after retries) |
| `4` | Check-only found damage |

Progress and diagnostics go to **stderr** (structured `tracing` logs); the final
`SUMMARY total=… skipped=… repaired=… failed=… download_bytes=… deleted_extra_bytes=… freed_temp_bytes=… exit=…`
line goes to **stdout** for easy parsing **and is mirrored to the log**, so a
stderr-only capture still keeps it. During long phases a
`PROGRESS done=<done>/<total> skipped=… repaired=… failed=… download_bytes=…`
line is emitted to **stdout every 10s (also mirrored to the log)** — from a
background reporter during repair, and inline during `--check-only`
(`skipped` = intact files so far, `failed` = bad files so far).

### Logging & correlation

Each file task logs inside a `file{seq,total,task,path}` span (`task` = tokio async-task id,
stable across worker-thread hops), so `grep 'path=<file>'` groups its lifecycle:
`check start` (expected size/md5) → outcome with actuals → per-chunk trace
(reuse-hit vs download, byte counts) → `after: tmp ready` → `repaired` with chunk stats.
API calls log the full request URL (no redaction — the HoYoPlay/Sophon API is public)
+ retcode at debug, full JSON bodies at trace.

## How it keeps disk usage low

1. **Pre-clean**: deletes `**/*_tmp`, `**/*.hdiff`, `chunk/`, `ldiff/`, `staging/` before starting.
2. **Skip intact files**: size + full MD5 check first — 0 bytes downloaded for healthy files.
3. **Chunk-level repair**: only missing/corrupt chunks are fetched; unchanged chunks are copied
   from verified local slices (hash-gated, with network fallback).
4. **Per-file temp + atomic move**: transient cost is one `_tmp` beside the file being repaired —
   no whole-game duplicate, no retained blob stores, no zip staging.
5. **Post-clean**: deprecated files, temps, and (with `--purge-extra`) any file not in the live
   manifest are removed; `config.ini` is bumped to the latest version.

## Project layout

```text
src/main.rs    CLI, logging init, exit codes, SUMMARY output
src/lib.rs     CLI definition, biz/channel/launcher/game-id mapping
src/hyp.rs     HoYoPlay client (getGameConfigs/getGameBranches/getBuild/getDeprecated)
src/sophon.rs  Chunk-manifest protobuf, fetch+verify+parse, manifest filtering
src/repair.rs  Work-list build, per-file repair, cleanup, extra-file purge
src/util.rs    MD5 helpers, temp sweep, config.ini read/write
docs/          findings & verdict, language-agnostic spec, Rust plan
```

## Limitations (v1)

Chunk-repair only: no hdiff fast-update, no 7z legacy path, no predownload, no SDK/WPF/plugin
downloads, no dispatcher persistent revisions, no speed limiter. Reruns are safe (idempotent:
intact files skipped, partial `_tmp` resumed by length and re-verified by final MD5).
