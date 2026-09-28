# Rust implementation plan — `girpr` v1 (Genshin low-disk repair patcher)

Spec: `docs/02-repair-chunk-spec.md`. Findings: `docs/01-findings-and-verdict.md`.

## 1. Crate layout (single binary, std + tokio)

The binary is a thin shim; everything else is in the library. `src/repair/` is
one command split into phases — the coordinator (`mod.rs`) owns phase order and
the exit-code contract, the submodules own the work.

```text
Cargo.toml            # clap, tokio, reqwest, serde, prost, md5, zstd, tracing, anyhow, walkdir, regex
src/main.rs           # shim: Args::try_parse (arg errors -> exit 1) -> logging::init_tracing -> repair::run -> SUMMARY -> exit
src/lib.rs            # module map + re-exports
src/cli.rs            # clap Args, audio-lang normalize/validate, Args::into_ctx -> RunCtx
src/config.rs         # RunCtx (validated run config) + RunFailure (typed exit code)
src/logging.rs        # console layer (--log-level/RUST_LOG) + always-TRACE file layer
src/report.rs         # Summary counters + REPORT / PROGRESS / SUMMARY formatting and emission
src/biz.rs            # biz/channel/launcher/game-id mapping + config.ini channel values
src/hyp.rs            # HoYoPlay client: getGameConfigs/getGameBranches/getBuild/getDeprecated
src/sophon.rs         # prost chunk-manifest structs, manifest fetch+verify+parse, filtering, local chunk map
src/repair/mod.rs     # phase coordinator + exit-code contract: run()
src/repair/metadata.rs# local config.ini version, ignore/blacklist files, server metadata (step 1-2)
src/repair/manifest.rs# select+fetch latest manifests, build the local chunk-reuse map (step 3)
src/repair/plan.rs    # RepairPlan / PlannedFile, sorted+deduped+blacklist-filtered work list (step 4)
src/repair/purge.rs   # Collapse files-cleanup: purge_extra, keep_set, classify_purge_path (step 5, 7)
src/repair/check.rs   # --check-only size+MD5 verification loop (step 6.5)
src/repair/file.rs    # per-file repair: skip check, _tmp resume, slice reuse, download, promote (step 6)
src/repair/post.rs    # deprecated delete, audio cache->res move, purge-after, config.ini bump (step 7)
src/repair/audio.rs   # audio scan-file read/write + effective selection
src/util.rs           # md5 helpers, file length, config.ini read/write, ignore/blacklist files, rel-path normalization
tests/repair_offline.rs   # offline integration/e2e against a local mock server
tests/cli_contract.rs     # binary-level argument handling and exit codes
tests/common/mod.rs       # mock HoYoPlay+Sophon server and the deterministic fixture
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr|none>]...   (repeatable; default: keep current else en-us; `none` = explicit game-only, overwrites scan file, cannot mix)
      [--io-threads <N, default 4>]            (concurrent FILES; chunks sequential/file)
      [--purge-after] [--purge-before]          (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--log-level error|warn|info|debug|trace] [--json-summary]
```

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args — clap-level
included, so the code remaps clap's default 2 — unusable game path, non-chunk
`default_download_mode`), 2 metadata/network, 3 write/verify (counted per-file repair
failures via `Ok(summary, 3)` with `SUMMARY`, plus fatal config.ini / audio-scan write,
files-cleanup via `Err(RunFailure::write)` with `FATAL` only), 4 check-only found damage. `--help`
is the only other 0. Fatal
errors carry their class via `config::RunFailure` at the raise site; per-file verify
failures continue and skip the post-phase instead. Automation: `tracing` diagnostics
on stderr; the begin `REPORT` line (versions + API fields), `PROGRESS` lines, and
the final `SUMMARY key=value` (or JSON with `--json-summary`) go to stdout and are mirrored to the log.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version | `util::read_game_version` (1) | Starward |
| Server metadata (`game_config`/`game_branch`/`chunk_build` latest + optional local) | `repair::metadata::fetch` + `hyp::HypClient` (2) | Starward |
| Effective audio + scan-file write | `repair::audio::{resolve_effective_audio,write_scan_file}` (1) | Starward |
| Begin `REPORT` | `report::format_report_line` (2.5) | Starward |
| Manifest fetch/verify/parse | `repair::manifest::fetch_latest` → `sophon::fetch_manifest` → zstd decode → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `repair::plan::build_plan` + `sophon::build_local_chunk_map` keyed by path, chunk md5+size (4) | Starward |
| Deprecated-file list (lazy, per-launcher `channel`/`sub_channel`) | `hyp::HypClient::deprecated_files` → `hyp_url(api, with_channel)` (2.6) | Starward |
| Files-cleanup before | `repair::purge::purge_extra(game_dir, plan, keep_set(), dry_run)` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Per-file repair | `repair::file::repair_all` — skip check → open `_tmp` → sequential chunks: slice-reuse (md5-gated) else `GET chunk_prefix/id` → zstd decode → write at offset → final md5 → rename (6) | Starward |
| Check-only | `repair::check::verify_plan` (6.5) | Starward |
| Post deletes/config | `repair::post::run` — deprecated delete + audio cache→res + `util::write_config_ini` (7) | Collapse-handling (section-preserving `config.ini` bump; `game_biz` force is a Starward carryover) |
| Files-cleanup after | `repair::purge::purge_extra(game_dir, plan, keep_set(), false)` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` only) |
| Summary | `report::format_summary_line`, emitted by `main` (8) | — |

Concurrency: `tokio::Semaphore(io_threads)` over files; one `reqwest::Client` with fixed
`pool_max_idle_per_host(16)` (see `src/hyp.rs`; not scaled by `io_threads`); per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
(512 KiB bufs). Retries: 5× linear backoff on manifest/chunk/file ops.

## 4. Testing

`cargo test` runs two layers, both offline by construction:

- **Unit tests** (`#[cfg(test)] mod tests` next to the code): config.ini parse/bump,
  audio-lang mapping and flag validation, API URL/query building (per-launcher
  `channel`/`sub_channel`, `getBuild` tag encoding), local chunk-map build, manifest
  filter, work-list build/dedup, expected-set/purge classification, audio scan file,
  REPORT/PROGRESS/SUMMARY formatting, HoYoPlay wrapper and int-or-string parsing.
- **Integration tests** (`tests/repair_offline.rs`): one `#[tokio::test]` per
  behavior, each running the full `repair::run` pipeline against a `MockServer`
  on `127.0.0.1:<ephemeral>` (`tests/common/mod.rs`) injected through
  `RunCtx::hyp_base_override` / `sophon_base_override`. Covers repair, check-only
  clean/dirty, dry-run, both purge timings, deprecated deletes, the non-chunk
  `default_download_mode` refusal, the per-launcher channel on the deprecated
  call, S2 slice reuse and S3 `_tmp` resume (asserting the exact chunk download
  set), and the audio scan file.
  `--dry-run`/`--check-only` are covered here, so no live-game CI dependency exists.
- **Binary-level tests** (`tests/cli_contract.rs`): run the built binary
  (`CARGO_BIN_EXE_girpr`) to pin the process exit codes — every argument error is
  `1`, never clap's `2`, and `--help` is `0`. No game directory is touched.

## 5. Logging & correlation

`tracing` to stderr (`--log-level` + `RUST_LOG`) plus a `TRACE`-level file log at
`<exe-dir>/logs/girpr_<timestamp>.log` (both layers built in `src/logging.rs`;
`--log-level` only affects stderr). A log file that cannot be opened degrades to a
warning and a sink, never a failure.
- `hyp` (debug): every HoYoPlay/Sophon JSON call with full request URL, retcode,
  node/attempt; (trace) full JSON response bodies (small metadata only, never chunk binaries).
- `sophon` (debug): manifest GET, compressed/decompressed byte counts, expected vs actual
  decompressed MD5, parsed file/chunk counts; (trace) per-chunk download byte counts.
- `repair`: every file task runs inside a `file{seq,total,task,path}` span (`task` = tokio
  async-task id, stable across worker-thread hops; first event notes the picking worker thread),
  so `grep 'path=<file>'` groups its whole lifecycle: `check start` (expect size/md5/chunks) →
  `check: missing|size mismatch|md5 mismatch (actual)|md5 match -> skip` → per-chunk trace
  (`chunk already complete|no local reuse candidate|reuse slice md5 ok|slice mismatch -> download|chunk decoded`) →
  `after: tmp ready for promote` → `repaired` (chunks total/reused/downloaded/resumed, bytes, final md5).
  Check-only failures log `expect_*` vs `actual_*` at warn level.

## 6. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, no dispatcher
persistent revisions, inside-`game_dir` manifest cache never written, `chunk/` reuse dir not retained.
