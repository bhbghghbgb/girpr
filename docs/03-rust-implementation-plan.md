# Rust implementation plan — `girpr` v1 (Genshin low-disk repair patcher)

Spec: `docs/02-repair-chunk-spec.md`. Findings: `docs/01-findings-and-verdict.md`.

## 1. Crate layout (single binary, std + tokio)

```text
Cargo.toml            # clap, tokio, reqwest, serde, prost, md5, zstd, tracing, anyhow, walkdir, regex
src/main.rs           # logging init, exit codes, REPORT/SUMMARY output, orchestration
src/lib.rs            # CLI definition (clap Args), audio-lang normalize, module root
src/biz.rs            # biz/channel/launcher/game-id mapping + config.ini channel values
src/hyp.rs            # HoYoPlay client: getGameConfigs/getGameBranches/getBuild/getDeprecated
src/sophon.rs         # prost chunk-manifest structs, manifest fetch+verify+parse, filtering
src/repair.rs         # work-list build, per-file repair, files-cleanup, config.ini write
src/util.rs           # md5 helpers, retry, atomic rename, config.ini read/write
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr>]...   (repeatable; default: keep current selection)
      [--io-threads <N, default 4>]            (concurrent FILES; chunks sequential/file)
      [--purge-after] [--purge-before]          (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--log-level error|warn|info|debug|trace] [--json-summary]
```

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args, unusable game
path, legacy FILE mode), 2 metadata/network, 3 write/verify (per-file repair,
config.ini / audio-scan write, files-cleanup), 4 check-only found damage. Errors
carry their class via `repair::RunFailure` at the raise site. Automation: all
progress on stderr (tracing); a begin `REPORT` line (versions + API fields) and
the final `SUMMARY key=value` (or JSON with `--json-summary`) on stdout.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version + audio | `repair::read_local_version`, audio scan file read/write (1) | Starward |
| Up to 5 metadata calls | `hyp::get_game_config/branches/build/deprecated` + `repair::format_report_line` begin-report (2) | Starward |
| Manifest fetch/verify/parse | `sophon::fetch_manifest(manifest_dl, meta)` → zstd decode → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `repair::build_plan(latest, local)` keyed by path, chunk md5+size (4) | Starward |
| Files-cleanup before | `repair::collapse_purge_extra(game_dir, plan)` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Per-file repair | `repair::repair_file()` — skip check → open `_tmp` → sequential chunks: slice-reuse (md5-gated) else `GET chunk_prefix/id` → zstd stream → write at offset → final md5 → rename (6) | Starward |
| Post deletes/config | deprecated delete + audio cache→res + `write_config_ini` (7) | Starward-handling |
| Files-cleanup after | `repair::collapse_purge_extra(game_dir, plan)` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` only) |
| Summary | (8) | — |

Concurrency: `tokio::Semaphore(io_threads)` over files; one `reqwest::Client` with
~`io_threads*4` pool; per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
(512 KiB bufs). Retries: 5× linear backoff on manifest/chunk/file ops.

## 4. Testing

`cargo test`: pure unit tests — config.ini parse/bump, audio-lang mapping, local chunk-map
build, manifest filter, expected-set/purge classification, REPORT/SUMMARY formatting.
Network paths covered by `--dry-run`/`--check-only` against a fixture manifest (no live-game CI dependency).

## 5. Logging & correlation

`tracing` to stderr; `--log-level debug|trace` (uses `RUST_LOG` if set).
- `hyp` (debug): every HoYoPlay/Sophon JSON call with full request URL, retcode,
  node/attempt; (trace) full JSON response bodies (small metadata only, never chunk binaries).
- `sophon` (debug): manifest GET, compressed/decompressed byte counts, expected vs actual
  decompressed MD5, parsed file/chunk counts; (trace) per-chunk download byte counts.
- `repair`: every file task runs inside a `file{seq,total,task,path}` span (`task` = tokio
  async-task id, stable across worker-thread hops; first event notes the picking worker thread),
  so `grep 'path=<file>'` groups its whole lifecycle: `check start` (expect size/md5/chunks) →
  `check: missing|size mismatch|md5 mismatch (actual)|md5 match -> skip` → per-chunk trace
  (`already complete|no reuse candidate|reuse slice ok|slice mismatch -> download|decoded bytes`) →
  `after: tmp ready` → `repaired` (chunks total/reused/downloaded/resumed, bytes, final md5).
  Check-only failures log `expect_*` vs `actual_*` at warn level.

## 6. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, no dispatcher
persistent revisions, inside-`game_dir` manifest cache never written, `chunk/` reuse dir not retained.
