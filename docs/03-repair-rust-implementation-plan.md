# Repair spec - Rust implementation plan - `girpr` v1 (Genshin low-disk repair patcher)

Implements `docs/02-repair-chunk-spec.md`. Findings:
`docs/01-repair-findings-and-verdict.md`.

The `repair-` prefix on all three documents is load-bearing: this file, the spec
and the findings are one series describing the whole-install repair patcher.
[`docs/04-scope-limitation-spec.md`](04-scope-limitation-spec.md) is a separate
tier and does not amend any of the three.

## 1. Crate layout (single binary, std + tokio)

The binary is a thin shim; everything else is in the library. `src/repair/` is
one command split into phases — the coordinator (`mod.rs`) owns phase order and
the exit-code contract, the submodules own the work.

```text
Cargo.toml            # clap, tokio, reqwest, serde, prost, md5, zstd, tracing, anyhow, walkdir, regex
src/main.rs           # shim: Args::try_parse (arg errors -> exit 1) -> logging::init_tracing -> repair::run -> SUMMARY -> exit
src/lib.rs            # module map + re-exports
src/cli.rs            # clap Args, audio-lang normalize/validate, Args::into_ctx -> RunCtx
src/config.rs         # RunCtx (validated run config) + RunFailure (typed exit code) + RepairMode
src/logging.rs        # console layer (--log-level/RUST_LOG) + file layer (--log-file-level), over the shared policy
crates/log/           # the shared log policy: log file path, log levels, env vars (also used by tools/)
crates/tempdir/       # the shared scratch-directory policy: a name nothing collides with, a tree removed with the test (also used by tools/)
src/report.rs         # Summary counters + REPORT / PROGRESS / SUMMARY formatting and emission
src/biz.rs            # biz/channel/launcher/game-id mapping + config.ini channel values
src/hyp.rs            # HoYoPlay client: getGameConfigs/getGameBranches/getBuild/getDeprecated
src/sophon.rs         # prost chunk-manifest structs, manifest fetch+verify+parse, the streamed chunk fetch, filtering, local chunk map
src/util.rs           # md5 helpers, the one-read file scan, config.ini read/write, ignore/blacklist files, rel-path normalization
src/repair/mod.rs     # phase coordinator + exit-code contract: run()
src/repair/metadata.rs# local config.ini version, ignore/blacklist files, server metadata (step 1-2)
src/repair/manifest.rs# select+fetch latest manifests, build the local chunk-reuse map (step 3)
src/repair/plan/mod.rs# RepairPlan / PlannedFile, sorted+deduped+blacklist-filtered work list, chunk layout (step 4)
src/repair/plan/purge.rs # Collapse files-cleanup: plan_purge, apply_purge, keep_set, classify_purge_path (step 5, 7)
src/repair/scope/     # the path scope and the byte ceilings: filter, ledger, limits, report (step 4, ceilings)
src/repair/check.rs   # --check-only size+MD5 verification loop (step 6.5)
src/repair/file.rs    # per-file repair: the in-place tiers, the temp mode, reuse vs download (step 6)
src/repair/reuse.rs   # the salvage order: which reuse source survives, in which direction
src/repair/scan.rs    # the --scan pass: the plan's exact download estimate and damaged-chunk count
src/repair/lock.rs    # the single-writer lock (girpr.lock)
src/repair/post.rs    # deprecated delete, audio cache->res move, purge-after, config.ini bump (step 7)
src/repair/audio.rs   # audio scan-file read/write + effective selection
tests/repair_offline.rs   # offline integration/e2e against a local mock server
tests/cli_contract.rs     # binary-level argument handling and exit codes
tests/artifact_bridge.rs  # the bundle-backed end-to-end suite (docs/06)
tests/common/mod.rs       # mock HoYoPlay+Sophon server and the deterministic fixtures
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr|none>]...   (repeatable; default: keep current else en-us; `none` = explicit game-only, overwrites scan file, cannot mix)
      [--io-threads <N, default 4>]            (concurrent FILES; chunks sequential/file)
      [--purge-after] [--purge-before]          (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--repair-mode <auto|inplace|temp|discard>]  (how each file is written; default auto)
      [--max-ram-bytes <N>]                    (cap the RAM the in-place repair may stage)
      [--scan]                                 (read the install once, for exact plan figures)
      [--log-level error|warn|info|debug|trace] [--log-file-level error|warn|info|debug|trace] [--output text|json] [--log-file <FILE>]
```

The path scope, the byte ceilings, `--print-plan`/`--plan-out` and the endpoint
overrides are [`docs/04`](04-scope-limitation-spec.md)'s tier; the README's flag table is
the whole list.

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args — clap-level
included, so the code remaps clap's default 2 — unusable game path, non-chunk
`default_download_mode`), 2 metadata/network, 3 write/verify (counted per-file repair
failures via `Ok(summary, 3)` with `SUMMARY`, plus fatal config.ini / audio-scan write,
files-cleanup via `Err(RunFailure::write)` with `FATAL` only), 4 check-only found damage. `--help`
is the only other 0. Fatal
errors carry their class via `config::RunFailure` at the raise site; per-file verify
failures continue and skip the post-phase instead. Automation: `tracing` diagnostics
on stderr; the begin `REPORT` line (versions + API fields), `PROGRESS` lines, and
the final `SUMMARY key=value` (NDJSON with `--output json`) go to stdout and are mirrored to the log.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version | `util::read_game_version` (1) | Starward |
| Server metadata (`game_config`/`game_branch`/`chunk_build` latest + optional local) | `repair::metadata::fetch` + `hyp::HypClient` (2) | Starward |
| Effective audio + scan-file write | `repair::audio::{resolve_effective_audio,write_scan_file}` (1) | Starward |
| Begin `REPORT` | `report::format_report_line` (2.5) | Starward |
| Manifest fetch/verify/parse | `repair::manifest::fetch_latest` → `sophon::fetch_manifest` → zstd decode (streamed into a buffer the API's `uncompressed_size` sizes) → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `repair::plan::build_plan` + `sophon::build_local_chunk_map` keyed by path, chunk md5+size (4) | Starward |
| Exact plan figures (`--scan`) | `repair::scan::scan_plan` — one read per planned file, parallel over `io_threads`, giving the download estimate and the damaged-chunk count (4) | — |
| Deprecated-file list (lazy, per-launcher `channel`/`sub_channel`) | `hyp::HypClient::deprecated_files` → `hyp_url(api, with_channel)` (2.6) | Starward |
| Files-cleanup before | `repair::plan::purge::plan_purge` → `apply_purge` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Single-writer lock | `repair::lock::Lock::acquire` — `{game_dir}/girpr.lock`, created with `create_new`; a live lock refuses the run with exit 1 (6) | — |
| Per-file repair | `repair::file::repair_all` — one scan (whole-file MD5 + per-chunk verdicts) → stage the bytes that must move → write only the wrong chunks: staged copy, else `GET chunk_prefix/id` → zstd decode **streamed straight into the file**, digested on the way past → trim → final md5 (6) | Starward's chunk repair; the in-place write is what the launcher does too |
| Check-only | `repair::check::verify_plan` (6.5) | Starward |
| Post deletes/config | `repair::post::run` — deprecated delete + audio cache→res + `util::write_config_ini` (7) | Collapse-handling (section-preserving `config.ini` bump; `game_biz` force is a Starward carryover) |
| Files-cleanup after | `repair::plan::purge::plan_purge` → `apply_purge` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` only) |
| Summary | `report::format_summary_line`, emitted by `main` (8) | — |

Concurrency: `tokio::Semaphore(io_threads)` over files; one `reqwest::Client` with fixed
`pool_max_idle_per_host(16)` (see `src/hyp.rs`; not scaled by `io_threads`); per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
(512 KiB bufs). Retries: 5× linear backoff on manifest/chunk/file ops.

## 4. Testing

`cargo nextest run` runs two layers, both offline by construction:

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
  call, the local-slice reuse and the temp mode's resume-by-length (asserting the
  exact chunk download set) and its rollback of a chunk that fails its digest, the
  manifest's checksum gate, the in-place path's two tiers and the mode
  resolution, the single-writer lock, and the audio scan file.
  `--dry-run`/`--check-only` are covered here, so no live-game CI dependency exists.
- **Binary-level tests** (`tests/cli_contract.rs`): run the built binary
  (`CARGO_BIN_EXE_girpr`) to pin the process exit codes — every argument error is
  `1`, never clap's `2`, and `--help` is `0`. No game directory is touched.
- **The bundle-backed suite** (`tests/artifact_bridge.rs`, [`docs/06`](06-artifact-bridge-spec.md)):
  `#[ignore]`d, and it drives the real binary over a captured bundle — each
  capture twice, once per write mode, asserting that the two produce the same
  file and that the in-place one writes less.
- **The measurement** (`tests/peak_heap.rs`): `#[ignore]`d, and it prints the
  chunk path's **peak live heap** — the streamed write against the buffered
  decode — so the RAM claim is re-derivable in a minute instead of asserted in
  a document.

## 5. Logging & correlation

`tracing` to stderr (`--log-level`, `info` by default, with `RUST_LOG` adding to
it) plus a file log at `<exe-dir>/logs/girpr_<timestamp>.log` (`--log-file-level`,
`debug` by default). Both layers are built in `src/logging.rs` over the shared
policy in `crates/log/` — the same policy the two dev tools follow. The two
levels are separate: `--log-level` only affects stderr, `--log-file-level` only
the file. A log file that cannot be opened degrades to a warning and a sink,
never a failure. Where the file goes, in order: `--no-log-file` (none at all),
then `--log-file`, then `GIRPR_LOG_FILE`, then the timestamped default. The
default itself is a release build's only: a debug build writes no file unless
one was asked for.
- `hyp` (debug): every HoYoPlay/Sophon JSON call with full request URL, retcode,
  node/attempt; (trace) full JSON response bodies (small metadata only, never chunk binaries).
- `sophon` (debug): manifest GET, compressed/decompressed byte counts, expected vs actual
  decompressed MD5, parsed file/chunk counts; (trace) per-chunk download byte counts.
- `repair`: every file task runs inside a `file{seq,total,task,path}` span (`task` = tokio
  async-task id, stable across worker-thread hops; first event notes the picking worker thread),
  so `grep 'path=<file>'` groups its whole lifecycle: `check start` (expect size/md5/chunks) →
  `check: missing|size mismatch|md5 mismatch (actual)|md5 match -> skip` → the scan's verdicts
  (`scanned`) → per-chunk trace (`chunk already complete|no local reuse candidate|reuse slice md5
  ok|slice mismatch -> download|chunk decoded`) → `repaired` (chunks total/reused/downloaded/kept,
  bytes, final md5). The temp mode adds `after: tmp ready for promote` before the rename.
  Check-only failures log `expect_*` vs `actual_*` at warn level.

## 6. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, no dispatcher
persistent revisions, inside-`game_dir` manifest cache never written, `chunk/` reuse dir not retained.

**The two write modes, and what each one is for:**

| | `inplace` (default) | `temp` (`--repair-mode temp`) |
|---|---|---|
| the target file | is written where it lies: the wrong bytes are replaced in place | is **never modified**: the repair is assembled in `{path}_tmp` and swapped in with an atomic rename |
| an interrupted run | leaves a file that fails the same check; the next run re-verifies and continues | leaves the target exactly as it was, plus a partial `*_tmp` the next run resumes by length |
| a final-MD5 mismatch | redoes only the chunks the per-chunk verdicts name | deletes the temp and restarts the file |
| the resume artefact | none: the scan finds the chunks that are already right, so a corrupt prefix cannot be inherited | the `*_tmp` prefix, kept and continued |
| peak extra disk | `io_threads × largest *growth*` — 0 for a same-size repair | `io_threads × largest file` |
| `stopped=--max-download-bytes`/`stopped=--max-write-bytes` | the next run re-verifies and continues | the next run resumes the `*_tmp` prefix |

The rows are two ways to spend disk, not two rungs of a safety ladder. Both gate the file on
the same whole-file MD5, and `config.ini` advances only when every planned file verified.

**The RAM a repair holds.** A downloaded chunk is **streamed**: the compressed body is
decoded as it arrives and the decoded bytes are written into the target as they are
produced, so the peak per active chunk is one read window plus the decoder's buffer
(**33 684 B measured** on the 2 162 302 B chunk — `tests/peak_heap.rs`), not the chunk. The
only RAM that scales with the file is the in-place tier's staging, and that is what
`--max-ram-bytes` bounds. A manifest is streamed too, into a buffer the API's
`uncompressed_size` sizes; its *decoded* bytes have to be resident for `prost` to parse
them, which is the one term the decode cannot remove.

**What is deliberately not implemented yet,** with the reason:

| item | why not now |
|---|---|
| the interval-graph optimum for the chunk order | the RAM tier removes the ordering constraint entirely wherever the payload fits, and the two-direction rule already captures a uniform shift — the dominant real shape — for the rest. It is one pure function (`repair::reuse::salvage_order`) to swap, and the writer does not know which tier produced the order |
| a speculative single-pass capture (capture every candidate source range during the scan, instead of a second pass over the bad ones only) | saves one read pass when the plan-time estimate fits the budget; costs RAM for sources that turn out not to be needed. The exact two-pass scheme is simpler to reason about, and the read is mostly served by the page cache |
| cross-file salvage (a chunk from any local file, not just the same path) | mutually exclusive with an in-place repair absent a file-level order: repairing a donor destroys a donee's source. The reuse map is same-file-only, so the conflict is latent — but it must be settled before anyone adds it |
| reclaim-on-demand (delete files mid-run to make room) | the in-place repair removes most of the pressure; it collides with `--max-delete-bytes`' "before anything is deleted" promise and deserves its own flag and its own doc |
| an incremental protobuf parse of the manifest | the manifest's *decoded* bytes (16.8 MB for the `game` manifest of build 7.1.0) must be resident for `prost` to parse them — the remaining term is protobuf, not zstd. The wire form is a sequence of `0x0A <varint len> <bytes>`, so each entry could be parsed on its own as the bytes arrive, but that needs the *read* side of the decoder (an async→sync bridge) and a ~60-line wire reader that must agree with `prost` on field order, unknown fields and framing. A separate piece of work with its own risk profile |
| hard-link dedup of byte-identical manifest paths | permanent space rather than transient, NTFS-only (exFAT, where Genshin installs often live, has no hard links), and it interacts with the in-place write: rewriting one link rewrites both |
| a compressed chunk cache on disk | strictly more disk, retained on purpose — the opposite of what this tool is for |

