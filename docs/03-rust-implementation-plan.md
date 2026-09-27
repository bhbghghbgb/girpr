# Rust implementation plan — `girpr` v1 (Genshin low-disk repair patcher)

Spec: `docs/02-repair-chunk-spec.md`. Findings: `docs/01-findings-and-verdict.md`.

## 1. Crate layout (workspace root = `girpr` binary; std + tokio)

```text
Cargo.toml            # clap, tokio, reqwest, serde, prost, md5, zstd, tracing, anyhow, walkdir, regex
src/main.rs           # logging init (stderr + per-run log file), arg validation, exit codes, SUMMARY output
src/lib.rs            # module root, CLI definition (clap Args), audio-lang normalize
src/biz.rs            # biz/channel/launcher/game-id mapping + config.ini channel values
src/hyp.rs            # HoYoPlay client: getGameConfigs/getGameBranches/getBuild/getDeprecated
src/sophon.rs         # prost chunk-manifest structs, manifest fetch+verify+parse, filtering
src/plan.rs           # work list: RepairPlan/PlannedFile + build_plan (dedup, blacklist)
src/error.rs          # RunFailure — typed error carrying the process exit code
src/report.rs         # Summary counters, REPORT/PROGRESS line formats, background progress reporter
src/util.rs           # md5 helpers, path normalization, config.ini read/write
src/repair/mod.rs     # pipeline orchestration (run) + RepairCtx
src/repair/file.rs    # per-file chunk repair + bounded file-parallelism driver
src/repair/check.rs   # --check-only verification pass
src/repair/purge.rs   # Collapse-parity files-cleanup, keep-set, purge classification
src/repair/audio.rs   # audio-scan read/write + audio cache->res move
tools/sync/           # dev-only `girsync` mirror tool — separate crate, same workspace
```

`src/repair/` is the only nested module: the pipeline is one `run()` in `mod.rs`, with the
per-file, check-only, files-cleanup and audio phases as submodules. Retry/backoff lives next to
the calls it protects (`hyp::get_node`/`get_direct`, `sophon::fetch_manifest`/`fetch_chunk_bytes`,
`repair::file`'s 5-attempt loop) — there is no shared retry helper. Atomic promote happens in
`repair::file` (`rename(tmp, final)`), not in `util`.

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <LANG>]...                  (repeatable; default: keep current selection)
      [--io-threads <N, default 4>]        (concurrent FILES; chunks sequential/file)
      [--purge-after] [--purge-before]     (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]            (verify / print actions, write nothing)
      [--log-level error|warn|info|debug|trace] [--json-summary]
```

`--audio` takes one value per flag (`--audio en-us --audio ja-jp`) and accepts the launcher
aliases too (`Chinese`, `English(US)`, `Japanese`, `Korean`); an unknown value exits `1`.

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args, unusable game
path in write modes, legacy FILE mode), 2 metadata/network, 3 write/verify (per-file repair,
config.ini / audio-scan write, files-cleanup), 4 check-only found damage. Errors
carry their class via `error::RunFailure` at the raise site. Automation: structured
`tracing` logs on stderr (plus a per-run `logs/girpr_<ts>.log` that always gets trace);
`PROGRESS` every 10s, the begin `REPORT` line (versions + API fields) and the final
`SUMMARY key=value` (or JSON with `--json-summary`) on stdout — each also mirrored to the log.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version + audio | `util::read_game_version`, `repair::audio::{read_current_audio, write_audio_scan}` (1) | Starward |
| Up to 5 metadata calls | `hyp::HypClient::{game_config, game_branch, chunk_build, deprecated_files}` + `report::format_report_line` begin-report (2) | Starward |
| Manifest fetch/verify/parse | `sophon::select_manifests` → `sophon::fetch_manifest(http, meta)` → zstd decode → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `plan::build_plan(per_manifest, blacklist, latest)` (dedup + blacklist) and `sophon::build_local_chunk_map(local_manifests)` → `HashMap<rel, (uncompressed_md5, uncompressed_size, offset)>` (4) | Starward |
| Files-cleanup before | `repair::purge::collapse_purge_extra(game_dir, plan, keep_set, dry_run)` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Per-file repair | `repair::file::repair_all_files` (semaphore over files) → `repair_one_file` (skip check) → `repair_attempt` (open `_tmp` → sequential chunks: slice-reuse md5-gated else `GET chunk_prefix/id` → zstd decode → write at offset → final md5 → rename) (6) | Starward |
| Post deletes/config | deprecated delete + `repair::audio::move_audio_cache` + `util::write_config_ini` (7) | Starward-handling |
| Files-cleanup after | `repair::purge::collapse_purge_extra(game_dir, plan, keep_set, false)` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` only) |
| Summary | `main.rs` prints `SUMMARY` from `report::Summary` (8) | — |

Chunk reuse is resolved at repair time (not precomputed per file): the local-chunk map gives the
candidate `(md5, size, offset)` for the same path, and `repair_attempt` re-hashes the actual
slice before copying — a mismatch falls back to download. See spec Step 4 and README S2.

Concurrency: `tokio::Semaphore(io_threads)` over files; one `reqwest::Client` with
`pool_max_idle_per_host(16)`; per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
(512 KiB bufs). Retries: 5× linear backoff on manifest/chunk/file ops.

## 4. Testing

`cargo test -p girpr`: 19 pure unit tests, no network and no game-dir fixtures beyond
`std::env::temp_dir()` sandboxes — `config.ini` parse/bump, audio-lang mapping, local chunk-map
build, manifest filter, expected-set/purge classification (incl. a real `collapse_purge_extra`
run), `REPORT`/`PROGRESS` formatting, protobuf round-trip, HoYoPlay wrapper/int coercion.
Network paths are not covered by automated tests; exercise them manually with
`--dry-run` / `--check-only` against a real install.

## 5. Logging & correlation

`tracing` to stderr at `--log-level` (uses `RUST_LOG` if set) **and** to a per-run file
`logs/girpr_<YYYY-MM-DD_HH-MM-SS.mmm>.log` next to the binary, which always receives `trace`
(ansi off) so an unattended failure can be replayed at full verbosity.
- `hyp` (debug): every HoYoPlay/Sophon JSON call with full request URL, retcode,
  node/attempt; (trace) full JSON response bodies (small metadata only, never chunk binaries).
- `sophon` (debug): manifest GET, compressed/decompressed byte counts, expected vs actual
  decompressed MD5, parsed file/chunk counts; (trace) per-chunk download byte counts.
- `repair::file`: every file task runs inside a `file{seq,total,task,path}` span (`task` = tokio
  async-task id, stable across worker-thread hops; first event notes the picking worker thread),
  so `grep 'path=<file>'` groups its whole lifecycle: `check start` (expect size/md5/chunks) →
  `check: missing|size mismatch|md5 mismatch (actual)|md5 match -> skip` → per-chunk trace
  (`already complete|no reuse candidate|reuse slice ok|slice mismatch -> download|decoded bytes`) →
  `after: tmp ready` → `repaired` (chunks total/reused/downloaded/resumed, bytes, final md5).
- `repair::check`: failures log `expect_*` vs `actual_*` at warn level (`CHECK fail: …`).
- `report`: `REPORT` (once, from `repair::run`), `PROGRESS` (every `PROGRESS_INTERVAL_SECS` = 10s)
  and the final `SUMMARY` (from `main.rs`) are printed to stdout *and* emitted at info to the log,
  so a stdout-only or a stderr/log-only capture each keeps the automation contract.

## 6. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, no dispatcher
persistent revisions, inside-`game_dir` manifest cache never written, `chunk/` reuse dir not retained.
