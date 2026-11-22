# Repair spec - Rust implementation plan - `girpr` v1 (GI low-disk repair patcher)

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
Cargo.toml            # runtime: clap, chrono, tokio, reqwest, serde(+json), tracing(+subscriber, appender), anyhow, walkdir, futures-util, tempfile, plus the girpr-* path crates below; prost/zstd and the loopback mock are dev-only; no regex dependency
src/main.rs           # shim: Args::try_parse (arg errors -> exit 1) -> logging::init_tracing -> repair::run -> SUMMARY -> exit
src/lib.rs            # module map + re-exports
src/cli.rs            # clap Args, audio-lang normalize/validate, Args::into_ctx -> RunCtx
src/config.rs         # RunCtx (validated run config) + RunFailure (typed exit code) + RepairMode
src/logging.rs        # console layer (--log-level/RUST_LOG) + file layer (--log-file-level), over the shared policy
crates/log/           # the shared log policy: log file path, log levels, env vars (shared by girpr and the three dev tools)
crates/tempdir/       # the shared scratch-directory policy: a name nothing collides with, a tree removed with the test (also used by tools/)
crates/test-fixtures/ # facade-free offline HYP/Sophon mock + fixture builders for the root integration tests, reached through thin shims under tests/common/
crates/test-support/  # facade-coupled run glue (RunCtx construction, pipeline driver) shared by the root integration tests, reached through the thin tests/common/ shim
tools/sync/cache/    # girsync's redb hash cache, its postcard wire codec and digest vocabulary; consumed directly by girsync and sled2redb
tools/sync/leaves/    # girsync's acyclic rw limiter, include/exclude filter, link classifier and hardlink facts, consumed directly by girsync so a high-level edit leaves them cached
tools/sync/test-support/ # girsync-coupled option/cache/planner fixtures shared by girsync and sled2redb; keeps crates/tempdir/ girsync-free
app/report/src/    # Summary/Meter/StoppedBy counters + record data plane, REPORT / PROGRESS / SUMMARY emission, phase timer (plan_record lives in the scope reporter)
src/biz.rs            # re-exports girpr_model::region::Region as Biz; ids/channels/base URLs live in crates/model/
app/fetch/src/hyp.rs        # HYP client: getGameConfigs/getGameBranches/getBuild/getDeprecated
app/fetch/src/sophon/      # prost chunk-manifest structs, manifest fetch+verify+parse, the streamed chunk fetch, filtering, local chunk map
app/util-path/        # server-path containment, atomic publish, shared junction/symlink fixtures
app/util-hash/        # md5 helpers and the one-read file scan
app/util-ini/         # config.ini read/write, ignore/blacklist files
app/util-counter/     # the saturating atomic byte counter
src/repair/mod.rs     # phase coordinator + exit-code contract: run()
src/repair/phases/metadata.rs # local config.ini version, ignore/blacklist files, server metadata (step 1-2)
src/repair/phases/manifest.rs # select+fetch latest manifests, build the local chunk-reuse map (step 3)
app/repair-plan/src/plan.rs # RepairPlan / PlannedFile, sorted+deduped+blacklist-filtered work list, chunk layout (step 4)
src/repair/purge.rs          # Collapse files-cleanup: plan_purge, apply_purge, keep_set, classify_purge_path (step 5, 7)
src/repair/scope/     # the path scope and the byte ceilings: filter, limits, report (step 4, ceilings); ledger.rs is a thin alias for girpr-report's Meter/StoppedBy
src/repair/phases/check.rs   # --check-only size+MD5 verification loop (step 6.5)
src/repair/file/mod.rs       # per-file repair: the in-place tiers, the temp mode, reuse vs download (step 6)
src/repair/reuse.rs   # the salvage order: which reuse source survives, in which direction (salvage_order_refs in production; salvage_order is test-only)
src/repair/scan.rs    # the --scan pass: the plan's exact download estimate and damaged-chunk count
app/lock/          # the single-writer lock (outside the install)
app/audio/            # audio scan-file read/write + effective selection (acyclic leaf)
src/repair/phases/post.rs    # deprecated delete, audio cache->res move, purge-after, config.ini bump (step 7)
tests/repair_offline.rs   # offline integration/e2e against a local mock server
tests/cli_contract.rs     # binary-level argument handling and exit codes
tests/artifact_bridge.rs  # the bundle-backed end-to-end suite (docs/06)
tests/common/mod.rs       # thin shims over the shared test libraries below
crates/test-fixtures/     # facade-free offline mock, artifact replay server and deterministic fixtures
crates/test-support/      # facade-coupled run glue (RunCtx construction, pipeline driver)
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr|none>]...   (repeatable; default: keep current else en-us; `none` = explicit game-only, overwrites scan file, cannot mix)
      [--jobs <N>]                            (concurrent FILES; a modest default; chunks sequential/file; `--io-threads` is a deprecated alias)
      [--purge-after] [--purge-before]          (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--repair-mode <auto|inplace|temp|discard>]  (how each file is written; default auto)
      [--max-ram-bytes <N>]                    (cap the RAM the in-place repair may stage)
      [--scan]                                 (read the install once, for exact plan figures)
      [--log-level error|warn|info|debug|trace] [--log-file-level error|warn|info|debug|trace] [--output text|json] [--log-file <FILE>] [--no-log-file]
```

The path scope, the byte ceilings, `--fail-fast`, `--purge-only`,
`--print-plan`/`--plan-out` and the endpoint overrides are
[`docs/04`](04-scope-limitation-spec.md)'s tier; the README's flag table is
the whole list.

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args — clap-level
included, so the code remaps clap's default 2 — unusable game path, non-chunk
`default_download_mode`), 2 metadata/network, 3 write/verify (counted per-file repair
failures via `Ok(summary, 3)` with `SUMMARY`, plus fatal config.ini / audio-scan write,
files-cleanup via `Err(RunFailure::write)` with `FATAL` only), 4 check-only found damage,
5 a requested enforcement ceiling bound the run (`SUMMARY stopped=…`, `limited=1`). `--help`
is the only other 0. Fatal
errors carry their class via `config::RunFailure` at the raise site; per-file verify
failures continue and skip the post-phase instead. Automation: `tracing` diagnostics
on stderr; the begin `REPORT` line (versions + API fields), `PROGRESS` lines, and
the final `SUMMARY key=value` (NDJSON with `--output json`) go to stdout and are mirrored to the log.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version | `util::read_game_version` (1) | Starward |
| Server metadata (`game_config`/`game_branch`/`chunk_build` latest + optional local) | `repair::phases::metadata::fetch` + `hyp::HypClient` (2) | Starward |
| Effective audio + scan-file write | `girpr_audio::{resolve_effective_audio,write_scan_file}` (1), aliased as `audio` in `src/repair/mod.rs` | Starward |
| Begin `REPORT` | `report::report_record` + `report::emit` (2.5) | Starward |
| Manifest fetch/verify/parse | `repair::phases::manifest::fetch_latest` → `sophon::fetch_manifest` → zstd decode (streamed into a buffer the API's `uncompressed_size` sizes) → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `girpr_repair_plan::build_plan` (4) + `repair::phases::manifest::local_reuse_map` → `sophon::build_local_chunk_map` keyed by path, chunk md5+size (3) | Starward |
| Exact plan figures (`--scan`) | `repair::scan::scan_plan` — one read per planned file, parallel over `jobs`, giving the download estimate and the damaged-chunk count (4) | — |
| Deprecated-file list (fetched in the plan phase, per-launcher `channel`/`sub_channel`) | `hyp::HypClient::deprecated_files` → `hyp_url(api, with_channel)` (2.6) | Starward |
| Files-cleanup before | `repair::purge::plan_purge` → `apply_purge` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Single-writer lock | `girpr_lock::Lock::acquire` — an OS advisory lock on `{temp}/girpr-{key:016x}.lock`, keyed by an FNV-1a hash of the canonical game dir; a live lock refuses the run with exit 1 (5, taken in the execute phase) | — |
| Per-file repair | `repair::file::repair_all` — one scan (whole-file MD5 + per-chunk verdicts) → stage the bytes that must move → write only the wrong chunks: staged copy, else `GET chunk_prefix/id` → zstd decode **streamed straight into the file**, digested on the way past → trim → final md5 (6) | Starward's chunk repair; the in-place write is what the launcher does too |
| Check-only | `repair::phases::check::verify_plan` (6.5) | Starward |
| Post deletes/config | `repair::phases::post::run` — deprecated delete + audio cache→res + `util::write_config_ini` (7) | Collapse-handling (section-preserving `config.ini` bump; `game_biz` force is a Starward carryover) |
| Files-cleanup after | `repair::purge::plan_purge` → `apply_purge` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` + the legacy in-install `girpr.lock` + the cleaned blacklist) |
| Summary | `report::summary_record` + `report::emit`, emitted by `main` (8) | — |

Concurrency: a bounded spawn/join window over files (`repair::file::join_file_tasks`, at most
`jobs` tasks in flight); a single `reqwest::Client` with a fixed
connection-pool cap (see `app/fetch/src/hyp.rs`; not scaled by `jobs`); per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
with fixed-size buffers. Retries: 5× with jittered exponential backoff on manifest/chunk/file ops.

## 4. Testing

`cargo nextest run` runs its test layers, all offline by construction:

- **Unit tests** (`#[cfg(test)] mod tests` next to the code): config.ini parse/bump,
  audio-lang mapping and flag validation, API URL/query building (per-launcher
  `channel`/`sub_channel`, `getBuild` tag encoding), local chunk-map build, manifest
  filter, work-list build/dedup, expected-set/purge classification, audio scan file,
  REPORT/PROGRESS/SUMMARY formatting, HYP wrapper and int-or-string parsing.
- **Integration tests** (`tests/repair_offline.rs`): a dedicated `#[tokio::test]` per
  behavior, each running the full `repair::run` pipeline against a `MockServer`
  on `127.0.0.1:<ephemeral>` (`crates/test-fixtures`, reached through the thin
  `tests/common/mod.rs` shim) injected through
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
  `#[ignore]`d, and it drives the real binary over a captured bundle — a `--print-plan`
  pre-pass plus one real run per capture, the repair captures asking for
  `--repair-mode inplace` explicitly. It asserts the patched bytes, the exact fetched-chunk
  set and the reported download bytes against the bundle's ground truth; the other write
  modes are tested on crafted fixtures, not here.
- **The streaming invariant** (`app/fetch/src/sophon/stream.rs`): an always-on unit test
  (`the_streamed_decode_writes_in_pieces_not_one_buffer`) asserts that a decoded
  chunk reaches the sink in more than one write, every byte arrives, and no single
  write holds the whole body. It fails if streaming is replaced by a buffered
  decode. It replaces a former `#[ignore]`d peak-heap measurement that printed a
  peak-live-heap ratio but asserted nothing.

## 5. Logging & correlation

`tracing` to stderr (`--log-level`, with `RUST_LOG` adding to
it) plus a file log at `<exe-dir>/logs/girpr_<timestamp>.log` (`--log-file-level`).
Both layers are built in `src/logging.rs` over the shared
policy in `crates/log/` — the same policy the three dev tools (`gircap`, `girsync`,
`sled2redb`) follow. The two
levels are separate: `--log-level` only affects stderr, `--log-file-level` only
the file. A log file that cannot be opened degrades to a warning and a sink,
never a failure. Where the file goes, in order: `--no-log-file` (none at all),
then `--log-file`, then `GIRPR_LOG_FILE`, then the timestamped default. The
default itself is a release build's only: a debug build writes no file unless
one was asked for.
- `hyp` (debug): every HYP/Sophon JSON call with full request URL, retcode,
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

| | `inplace` (`auto` picks this when the target is writable) | `temp` (`--repair-mode temp`) |
|---|---|---|
| the target file | is written where it lies: the wrong bytes are replaced in place | is **never modified**: the repair is assembled in `{path}_tmp` and swapped in with an atomic rename |
| an interrupted run | leaves a file that fails the same check; the next run re-verifies and continues | leaves the target exactly as it was, plus a partial `*_tmp` the next run resumes by length |
| a final-MD5 mismatch | redoes only the chunks the per-chunk verdicts name | deletes the temp and restarts the file |
| the resume artefact | none: the scan finds the chunks that are already right, so a corrupt prefix cannot be inherited | the `*_tmp` prefix, kept and continued |
| peak extra disk | `jobs × largest *growth*` — 0 for a same-size repair | `jobs × largest file` |
| `stopped=--max-download-bytes`/`stopped=--max-write-bytes` | the next run re-verifies and continues | the next run resumes the `*_tmp` prefix |

The rows are two ways to spend disk, not two rungs of a safety ladder. Both gate the file on
the same whole-file MD5, and `config.ini` advances only when every planned file verified.

**The RAM a repair holds.** A downloaded chunk is **streamed**: the compressed body is
decoded as it arrives and the decoded bytes are written into the target as they are
produced, so the peak per active chunk is one read window plus the decoder's buffer,
not the chunk — a scale-free invariant in `app/fetch/src/sophon/stream.rs` fails if a buffered
decode ever writes the whole body in one piece. The
only RAM that scales with the file is the in-place tier's staging, and that is what
`--max-ram-bytes` bounds. A manifest is streamed too, into a buffer the API's
`uncompressed_size` sizes; its *decoded* bytes have to be resident for `prost` to parse
them, which is the one term the decode cannot remove.

**What is deliberately not implemented yet,** with the reason:

| item | why not now |
|---|---|
| the interval-graph optimum for the chunk order | the RAM tier removes the ordering constraint entirely wherever the payload fits, and the two-direction rule already captures a uniform shift — the dominant real shape — for the rest. It is a pure function (`repair::reuse::salvage_order_refs`, with test-only `salvage_order`) to swap, and the writer does not know which tier produced the order |
| a speculative single-pass capture (capture every candidate source range during the scan, instead of a second pass over the bad ones only) | saves a read pass when the plan-time estimate fits the budget; costs RAM for sources that turn out not to be needed. The exact two-pass scheme is simpler to reason about, and the read is mostly served by the page cache |
| cross-file salvage (a chunk from any local file, not just the same path) | mutually exclusive with an in-place repair absent a file-level order: repairing a donor destroys a donee's source. The reuse map is same-file-only, so the conflict is latent — but it must be settled before anyone adds it |
| reclaim-on-demand (delete files mid-run to make room) | the in-place repair removes most of the pressure; it collides with `--max-delete-bytes`' "before anything is deleted" promise and deserves its own flag and its own doc |
| an incremental protobuf parse of the manifest | the manifest's *decoded* bytes (the measured game manifest for a release build) must be resident for `prost` to parse them — the remaining term is protobuf, not zstd. The wire form is a sequence of `0x0A <varint len> <bytes>`, so each entry could be parsed on its own as the bytes arrive, but that needs the *read* side of the decoder (an async→sync bridge) and a small wire reader that must agree with `prost` on field order, unknown fields and framing. A separate piece of work with its own risk profile |
| hard-link dedup of byte-identical manifest paths | permanent space rather than transient, NTFS-only (exFAT, where GI installs often live, has no hard links), and it interacts with the in-place write: rewriting one link rewrites both |
| a compressed chunk cache on disk | strictly more disk, retained on purpose — the opposite of what this tool is for |

