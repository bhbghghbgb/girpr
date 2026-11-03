# Checklist state

Committed state for the temporary optimization work. The orchestrator owns this
file; implementers and reviewers must not edit it. Every change to this file is
its own commit (`chore(plan): ...`), separate from code commits.

Status vocabulary for tasks: `todo`, `in_progress`, `pending_review`,
`completed`, `reverted`, `blocked`, `skipped`. Review batches: `pending`,
`in_progress`, `done`. See `ORCHESTRATION.md` for transitions.

## Tasks

| ID | Title | Status | Commits | Batch |
|---|---|---|---|---|
| T01 | C1 - closed stdout is not a panic | completed | 37473e0, 587af78 | RC-01 |
| T02 | C5 - reject oversized JSON numbers | completed | 0bf3646, 7f7190e | RC-01 |
| T03 | C6 - harden meter subtraction | completed | 76e7499, f029dfc, 262dcdc | RC-01 |
| T04 | C7 - use a Windows path key, not Unicode lowercase | completed | 176561b, 670c45e | RC-01 |
| T05 | C3 - never publish a partial audio target | completed | 5ba1466, 084bb1e | RC-02 |
| T06 | C4 - write `--plan-out` atomically | completed | 3b9fcce, 3675334 | RC-02 |
| T07 | C8 - one temp-file naming rule | completed | 6739ad8, d47582f, 78477e4 | RC-02 |
| T08 | P1 - overlap metadata calls | completed | eb8fe99, a88c94f | RC-03 |
| T09 | P2 - fetch manifests concurrently | completed | 8244a36, ebbbb14, 8a32496 | RC-03 |
| T10 | P4 - move blocking phases off workers; parallel check | completed | 3b65c99, f7e4a13, 021d810, 90f62de | RC-04 |
| T11 | P5 - size the runtime for `--jobs` | completed | 63ceede, 35d55fe | RC-04 |
| T12 | P6 - verify with one whole-file digest | completed | fcdcfe1, b099683, c8c5d3f | RC-05 |
| T13 | P7 - reuse the temp source handle | completed | 65d1915, 8e8303a, 024d535 | RC-05 |
| T14 | P8 - remove the per-piece sink seek | completed | 8920f51, 62095c4, 9921c0d | RC-05 |
| T15 | P10 - share the local chunk map | completed | 6d5b5b4, eb705ce | RC-05 |
| T16 | P3 - consume `--scan` verdicts in repair | completed | 2ea0747, df034d0 | RC-06 |
| T17 | C2 + C9 - one lock for every mutating run | completed | 8800aad, 14cffba, cc204d9, d698d8c | RC-07 |
| T18 | A1 - narrow the public surface | completed | c0dd7de, 9007c90 | RC-07 |
| T19 | A2 - make `--jobs` the primary flag | completed | fa79ffd, c1c875a, 14bef05 | RC-07 |
| T20 | A3 - exercise the post-phase in dry-run | completed | d32c395, bb1368c, c4285e7, f221aad, f0fa925 | RC-08 |
| T21 | A4 - one emitter and one progress driver | completed | 0948590, b8d5068, c56681c | RC-08 |
| T22 | A5 - return an exit code and flush logs | completed | dc8f7cd, 54ec0ee, e197cd3 | RC-08 |
| T23 | P9 - buffer the file log sink | completed | 16e2902, 11acd42 | RC-09 |
| T24 | P11 - remove purge/scope per-pass allocations | completed | debba64, 68242fe, a9d5b20, f7d2165 | RC-09 |
| T25 | P13 - fast-path unlimited meters | completed | af8e5bf, e799311 | RC-09 |
| T26 | P12 - resolve `PathRoot` leaves safely (decision gate) | skipped | | RC-09 |
| T27 | D1 - per-file facts live on the file | completed | 3576cf4, 07c556d, 52158e8, 1449a7f | RC-10 |
| T28 | D3 - one atomic-write helper | completed | 0ba2419, 29396b3, 1f2707b | RC-11 |
| T29 | D2 - one retry/backoff scaffold | completed | fea2efe, c665a40, 526f78c, 01776f9, a41eda4 | RC-11 |
| T30 | D4 - one rel-path spelling helper | completed | 4ee251b, 48d4499, fb85754 | RC-12 |
| T31 | D5 - one per-range to per-chunk remap | completed | 9090504, d81083e | RC-12 |
| T32 | D7 - reconcile documentation with the code | completed | b92f90d, 8e3f127 | RC-13 |
| T33 | D6 - remove test-only production API | completed | d47cd56, da2b5a2, bad95ca | RC-13 |
| T34 | D8 - split the functions doing too much | completed | 005fa5f, 4fb3095, ab3e2c8, c4c6abb, 9999749, 3a1eb18 | RC-14 |
| T35 | D9 - replace parameter bags with structs | completed | 09c40dc, 4660afd, 6fd9670, 63961bc | RC-15 |
| T36 | D10 - name every exit code | completed | 66b37a6, 487b7ad, bc12be9 | RC-15 |
| T37 | B1 - trim the tokio feature set | completed | 075f6d2 | RC-16 |
| T38 | B2 - remove the `regex` dependency | completed | 4567a8f, a244131 | RC-16 |
| T39 | B3 - use `futures-util` directly | completed | 4c64d0e | RC-16 |
| T40 | B4 - build the product by default | completed | b4ee7e9, 73370de | RC-16 |
| T41 | B5 - tune the release profile | completed | 53cd8ef | RC-16 |
| T42 | B6 - add a workspace lint policy | completed | 21e49a7, 50faff7 | RC-16 |
| T43 | B7 - evaluate the md-5 upgrade (conditional) | completed | 8078f2b, 07c6ab2, a2b2d04 | RC-16 |
| T44 | T1 - regression tests for the fixed failure paths | completed | 9894d6e | RC-17 |
| T45 | T2 - log phase and per-file durations | completed | b4b0d84, 1d95c1a, 7c54827 | RC-17 |
| T46 | T4 - pin the output-contract ordering | completed | 1bb253d, b58c967 | RC-18 |
| T47 | T3 - split the large integration test | completed | 10e8849, 9fb6863 | RC-18 |

## Review batches

| Batch | Tasks | Status | Review commits | Notes |
|---|---|---|---|---|
| RC-01 | T01-T04 | done | 262dcdc | T03 fixed: deprecated atomic API in `adjust_live` |
| RC-02 | T05-T07 | done | | All three completed without fixes |
| RC-03 | T08-T09 | done | | Both completed without fixes |
| RC-04 | T10-T11 | done | | Both completed without fixes |
| RC-05 | T12-T15 | done | 9921c0d | T14 fixed: reservation settled on chunk seek failure |
| RC-06 | T16 | done | | Completed without fixes |
| RC-07 | T17-T19 | done | | All three completed without fixes |
| RC-08 | T20-T22 | done | f0fa925 | T20 fixed: docs' deprecated-list dry-run contract |
| RC-09 | T23-T26 | done | | T23-T25 completed without fixes; T26 skip confirmed |
| RC-10 | T27 | done | | Completed without fixes |
| RC-11 | T28-T29 | done | | Both completed without fixes |
| RC-12 | T30-T31 | done | | Both completed without fixes |
| RC-13 | T32-T33 | done | 8e3f127 | T32 fixed: legacy-vs-active lock in settings docs |
| RC-14 | T34 | done | | Completed without fixes |
| RC-15 | T35-T36 | done | | Both completed without fixes |
| RC-16 | T37-T43 | done | | All completed without fixes; T43 gate passed (+26%) |
| RC-17 | T44-T45 | done | | Both completed without fixes |
| RC-18 | T46-T47 | done | | Both completed without fixes |

## Rework

Tasks the reviewer reverted, with the reason and the suggested correction. Pick
these before the follow-up phase.

| ID | Original task | Reason | Status |
|---|---|---|---|
| | | | |

## Follow-ups

Non-blocking items recorded by reviewers or implementers. Do not implement
until the main tasks and rework are done.

| ID | Source | Area | Description | Status |
|---|---|---|---|---|
| F01 | RC-01 review | `src/hyp.rs` | `num_from_string` string branches discard the `ParseIntError` detail; keep the parse reason for diagnosability. | completed |
| F02 | RC-01 review | `src/report/emit.rs` | Non-`BrokenPipe` stdout errors warn per call with no latch; revisit only if log flooding is observed. | dropped (no observed flooding; latch is speculative) |
| F03 | RC-01 review | `tools/sync` | Sibling crate still folds path keys with `str::to_lowercase` (same Unicode-fold class of issue). | completed |
| F04 | RC-01 review | `src/repair/scope/filter.rs` | Glob case-insensitive matching differs from `path_key`; cross-reference the two docs. | completed |
| F05 | RC-02 review | `src/repair/phases/post.rs` | `copy_publish_with` keeps its own `PUBLISH_TMP_SUFFIX` append; adopt `util::path::temp_sibling`. | todo |
| F06 | RC-02 review | `src/repair/scope/report.rs` | Temp-then-rename replaces a symlink at the target and resets the target's mode; document the trade. | todo |
| F07 | T02 implementer | `src/hyp.rs` | Negative sizes are still accepted and clamped to zero; decide whether to reject them explicitly. | completed |
| F08 | T03 implementer | `src/repair/scope/ledger.rs` | `Summary::release_ram_bytes` still uses unchecked `fetch_sub`; consider the same saturation guard. | completed |
| F09 | T05 implementer | `src/repair/phases/post.rs` | The copy-fallback caller path is only unit-tested through the injected helper, not a real failed rename. | todo |
| F10 | T05 implementer | `src/repair/phases/post.rs` | The fixed temp sibling name would collide if two moves target the same path concurrently. | dropped (one run at a time under the lock makes the collision unreachable) |
| F11 | T06 implementer | `src/repair/scope/report.rs` | Untested `file_name() == None` fallback; cleanup warns when a directory occupies the temp path. | todo |
| F12 | T07 implementer | `src/repair/purge.rs` | A purge test hard-codes the `.girpr_tmp` spelling instead of using the shared name helper. | todo |
| F13 | RC-03 review | `src/repair/phases/metadata.rs` | `join!` waits out a sibling's retry schedule after an authoritative failure; cancel it once its error is settled. | completed |
| F14 | RC-03 review | `src/repair/phases/metadata.rs` | Spawn-join failures are mapped ad hoc (ignore/blacklist vs audio/version exit classes). | completed |
| F15 | RC-03 review | `src/repair/phases/manifest.rs` | Bounded-fetch reindex plumbing and the concurrency log line are duplicated for latest and local. | completed |
| F16 | RC-03 review | `src/repair/phases/manifest.rs` | A fast failure still waits for slower siblings; reorder incrementally and cancel the rest. | completed |
| F17 | T08 implementer | `src/repair/phases/metadata.rs` | Blocking local reads are dropped unawaited on early return; drain them for stricter lifetime hygiene. | completed |
| F18 | T09 implementer | `tests/common` | The mock models a single latest manifest; multi-manifest integration coverage needs fixture work. | todo |
| F19 | RC-04 review | `src/repair/phases/check.rs` | `--check-only` emits progress only after the whole verify pass; a ticker should keep it live. | todo |
| F20 | RC-04 review | `src/repair/phases/post.rs`, `src/repair/purge.rs` | `delete_deprecated` and `apply_purge` still run synchronous filesystem loops on async workers. | todo |
| F21 | RC-04 review | `src/repair/phases/check.rs` | A panicking check task becomes a damaged-file verdict instead of failing the run, unlike purging. | todo |
| F22 | RC-04 review | `src/main.rs`, `src/cli.rs` | A huge `--io-threads` now fails runtime construction rather than clamping; validate the flag. | completed |
| F23 | T10 implementer | `src/repair/phases/check.rs` | The damaged-chunk total from check is only logged, not carried on `Summary`. | todo |
| F24 | RC-05 review | `src/util/md5.rs` | `scan_reader`/`Scan.file_md5` return hex that callers re-parse; carry `Md5` instead. | todo |
| F25 | RC-05 review | `src/repair/file/temp.rs` tests | The rollback resume-point test sets `tmp_len` by hand; the caller's update is not exercised end-to-end. | todo |
| F26 | RC-05 review | `src/repair/file/temp.rs` | The sink's seek-failure refund path has no test seam. | todo |
| F27 | RC-05 review | `src/repair/file/temp.rs` | The source handle stays open across `promote_tmp`; drop it first to avoid the platform assumption. | todo |
| F28 | T13 implementer | `src/repair/file/temp.rs` | `try_reuse_slice`'s doc still says it "Returns `false`"; the return type is `ChunkOutcome`. | todo |
| F29 | T13 implementer | `src/repair/file/inplace.rs` | The in-place path still resolves reuse sources per chunk. | todo |
| F30 | T14 implementer | `src/repair/file/temp.rs` | `try_reuse_slice` seeks `out` to the chunk offset even on the download path, now redundant. | todo |
| F31 | T14 implementer | `src/repair/file/temp.rs` | The sink remains unbuffered; buffering needs explicit restart/digest flush handling. | dropped (buffering needs restart/digest flush semantics for no measured win) |
| F32 | RC-06 review | `tests/repair_machinery.rs` | No end-to-end test mutates a file between plan and repair to prove the fresh-scan fallback. | todo |
| F33 | RC-06 review | `src/repair/file/inplace.rs` | The reuse debug log's `age_secs` is the file's mtime age, not the scan snapshot's age. | todo |
| F34 | RC-06 review | `src/repair/scan.rs`, `src/repair/file/inplace.rs` | Coarse-mtime filesystems can evade the exact length+mtime guard; document or harden. | todo |
| F35 | RC-07 review | `src/repair/lock.rs` | `Drop` unlocks then deletes, a narrow Unix race; leave the file or re-verify after acquire. | completed |
| F36 | RC-07 review | `src/repair/lock.rs` | The temp lock is opened by name and truncated; a pre-created symlink could truncate a user file. | completed |
| F37 | RC-07 review | `src/repair/lock.rs` | `key_for` hashes `to_string_lossy()`; distinct non-UTF-8 paths can collide. | completed |
| F38 | RC-07 review | `src/repair/purge.rs`, `docs/01-03` | Comments and docs still describe the in-install `girpr.lock` and keep-set wording. | todo |
| F39 | RC-07 review | `README.md`, `Cargo.toml` | README claims Rust 1.85+, but `File::try_lock` needs 1.89; no `rust-version` pin. | completed |
| F40 | RC-07 review | `src/cli.rs` | `excessive_jobs_warning` says workers can fall below file tasks, but the runtime is sized to `max(parallelism, jobs+1)`. | completed |
| F41 | RC-07 review | `src/main.rs`, `src/repair/**`, `tools/sync`, `docs/02-06` | Stale `--io-threads` prose remains after the `--jobs` rename. | todo |
| F42 | RC-07 review | `src/lib.rs` | `test_support` re-exports `stream_chunk_to` beyond the task's list and duplicates root items; document the seam. | todo |
| F43 | T18 implementer | `src/report/mod.rs` | `Summary` public fields have types (`Meter`, `SummaryFlags`, `Ceiling`) no longer nameable outside the crate. | todo |
| F44 | T19 implementer | `src/cli.rs` | On hosts with parallelism below 4, the default `--jobs 4` warns on every run. | completed |
| F45 | T17 implementer | `src/repair/lock.rs` | On Windows the exclusive lock hides the body, so refusals usually cannot name the holder pid. | completed |
| F46 | RC-08 review | `src/repair/purge.rs`, `src/repair/phases/post.rs` | `--purge-before --purge-after --dry-run` lists the same cleanup candidates twice. | todo |
| F47 | RC-08 review | `src/repair/phases/post.rs` | A dry run that trips the post-repair ceiling logs error-level `limit exceeded` yet exits 0. | todo |
| F48 | RC-08 review | `src/report/mod.rs` | `emit_progress`/`PROGRESS_INTERVAL_SECS` are no longer re-exported; restore if that is API. | todo |
| F49 | RC-08 review | `src/logging.rs` tests | A flush test asserts it is the only setter of the global sink; fragile if another test installs tracing. | completed |
| F50 | T20 implementer | `src/repair/phases/post.rs` | A dry run now does a second purge walk plus the audio walk, adding I/O and network work. | dropped (the extra walks are the intended cost of exercising the post-phase) |
| F51 | T20 implementer | `src/repair/phases/post.rs` | A post-phase purge re-plan failure in a dry run now maps to exit 3. | todo |
| F52 | T21 implementer | `src/repair/phases/check.rs` | Check-only counters are folded after `collect`, so the ticker may report `done=0` mid-phase. | todo |
| F53 | T21 implementer | `tests/cli_contract.rs` | No integration assertion that a real binary run emits exactly one `SUMMARY`. | todo |
| F54 | T22 implementer | `src/logging.rs` | The flush handle is a `OnceLock` for the first sink only; a second `init_tracing` is unflushable. | completed |
| F55 | T22 implementer | `src/logging.rs` | The flush-failure stderr branch has no test (stderr capture is process-level). | completed |
| F56 | RC-10 review | `src/repair/scan.rs` | `ScanReport` carries no plan identity; index alignment silently mismatches a different plan. | todo |
| F57 | RC-10 review | `src/repair/phases/plan.rs` | `FileId` derived `Debug` prints `FileId(0)` rather than the index as the task asked. | todo |
| F58 | T27 implementer | `src/repair/phases/plan.rs`, `src/repair/scan.rs` | `RepairPlan::files` and `ScanReport::files` stay public, so callers can bypass the id accessors. | todo |
| F59 | T27 implementer | `src/repair/scope/report.rs` | `DiskPlan::for_id` returns a default for an out-of-range id; `PlanDocument` uses raw indices. | todo |
| F60 | RC-11 review | `src/util/atomic.rs` | Module docs link a `pub(crate)` `temp_sibling`, which may warn under `cargo doc`. | todo |
| F61 | RC-11 review | `src/util/ini.rs` | `write_config_ini` now creates a missing game directory through the helper where it previously failed. | completed |
| F62 | RC-11 review | `src/retry.rs` | Jitter is salted by process id and attempt only, so concurrent files share the same spread. | completed |
| F63 | RC-11 review | `src/repair/file/mod.rs` | A comment still budgets 5 × (15 s + 10 s); with the final sleep removed it is ~100 s. | todo |
| F64 | RC-11 review | `src/hyp.rs` | No call-site test that `get_node`/`get_direct` retry a retryable error and stop on a fatal one. | completed |
| F65 | T28 implementer | `src/util/ini.rs` tests | No config-specific "no temp left" assertion after the helper extraction. | todo |
| F66 | T28 implementer | `src/util/atomic.rs` | Error context is artifact-neutral; artifact-specific labels would need a parameter. | todo |
| F67 | RC-12 review | `src/repair/phases/post.rs` | `move_audio_cache` still strips the cache prefix manually; it could use `rel_from_base`. | todo |
| F68 | RC-12 review | `src/util/path.rs` | `rel_from_base` and `join_rel` each inline the lossy `/` conversion; one private helper would be literal. | todo |
| F69 | RC-12 review | `src/repair/scan.rs` | The helper tolerates length mismatch; a `debug_assert` at the two call sites would still be useful. | todo |
| F70 | RC-12 review | `src/repair/file/inplace.rs` | `source_ok_by_range` is the inverse mapping; cross-reference or pin it near the shared remap. | todo |
| F71 | T30 implementer | `src/util/path.rs` | `join_rel` assumes an already-clean `dir_rel` with no trailing separator. | todo |
| F72 | T30 implementer | `src/util/mod.rs` | The `path` module summary does not mention the two new rel helpers. | todo |
| F73 | RC-13 review | `src/repair/mod.rs`, `README.md`, `docs/02` | `--print-plan --dry-run` still fetches the deprecated list, contradicting the documented contract. | todo |
| F74 | RC-13 review | `src/report/emit.rs` | Public docs link to the private `write_line`, leaving a rustdoc warning. | todo |
| F75 | T33 implementer | `tools/sync` | The sibling crate has its own unrelated `Record::event` implementation. | dropped (unrelated sibling-crate implementation) |
| F76 | T33 implementer | `src/util/ini.rs` | No pure `render_config_ini` helper; future tests observing rendering without writing would need one. | dropped (helper for a test that does not exist yet) |
| F77 | RC-14 review | `src/repair/mod.rs` | `publish_plan`'s doc describes a disk-figures computation that moved into `build_and_assert_plan`. | todo |
| F78 | RC-14 review | `src/repair/mod.rs`, `src/repair/file/inplace.rs` | Four new `too_many_arguments` allows are parameter bags for T35 to absorb. | dropped (superseded by F82 (current five-site inventory)) |
| F79 | RC-14 review | `src/repair/file/inplace.rs`, `src/repair/mod.rs` | Trim/tier decisions and plan-step guards have only end-to-end coverage. | todo |
| F80 | T34 implementer | `src/repair/mod.rs` | `build_and_assert_plan` is still ~150 lines and could split further. | todo |
| F81 | T34 implementer | `src/retry.rs` | Clippy `enum_variant_names` warning predates the work and is still present. | completed |
| F82 | RC-15 review | `src/repair/mod.rs`, `src/repair/file/mod.rs`, `src/repair/file/inplace.rs` | Five `too_many_arguments` allows remain outside T35's four sites. | todo |
| F83 | RC-15 review | `src/report/emit.rs` | `ReportProvenance` exposes every field publicly; a constructor or narrower visibility would harden it. | todo |
| F84 | T35 implementer | `src/sophon/stream.rs` | The streaming path joins the URL per attempt; carrying the joined URL would avoid the allocation. | dropped (negligible allocation on a network-bound path) |
| F85 | RC-15 review | `src/config.rs`, `tests/repair_offline.rs` | `PartialEq<i32> for RunExit` is a numeric escape hatch; convert test assertions to variants and drop it. | todo |
| F86 | RC-15 review | `src/repair/phases/check.rs`, `src/main.rs` | Prose still describes exit codes as bare numbers. | todo |
| F87 | T36 implementer | `src/config.rs` | `From<RunFailure> for RunExit` is exercised only by tests; no production caller yet. | todo |
| F88 | T37 implementer | `tools/capture` | The tool still depends on `tokio` with `full`; scoped out of the build batch. | todo |
| F89 | T37 implementer | `Cargo.toml` | `reqwest` unifies tokio's `net`/`fs`/`io-util`/`sync` into any build; reqwest's own surface, not removable here. | dropped (superseded by F106) |
| F90 | T38 implementer | `src/util/ini.rs` | The old pattern's `\s*` could match a newline; the hand-written line scan intentionally does not, and nothing depends on it. | dropped (intentional difference; nothing depends on the old behavior) |
| F91 | T38 implementer | `src/util/ini.rs` | `read_game_version` and `parse_ini` share the "first `=`" rule without a common helper. | completed |
| F92 | T39 implementer | `Cargo.toml` | `futures-util` defaults pull `futures-macro` and `slab`; a `std`-only feature set could shrink the graph. | completed |
| F93 | T39 implementer | `src/sophon/stream.rs` | The `Stream` bound names `futures-core` directly; `futures_util::stream::Stream` would be the same trait. | completed |
| F94 | T40 implementer | repo | No CI exists; a future CI job must run `cargo check -p girsync -p gircap` or a workspace job. | dropped (repository has no CI system) |
| F95 | T40 implementer | `README.md` | The nextest note hard-codes a test count ("362 tests") that is now 470 and drifts; de-number it. | todo |
| F96 | T41 implementer | `Cargo.toml` | Release builds strip symbols; crash triage may want `strip = "debuginfo"` or a separate profile. | dropped (crash-triage symbol needs are hypothetical) |
| F97 | T42 implementer | `clippy.toml` | Revisit `allow-unwrap-in-tests`/`allow-expect-in-tests` to replace the code-level test allows. | todo |
| F98 | T43 implementer | `Cargo.toml` | `md-5`'s optional `asm`/`md5-asm` backend may beat the portable path on x86; measure separately. | dropped (speculative; the portable upgrade already passed the gate) |
| F99 | T43 implementer | `tools/sync`, `tools/capture` | Both tools still depend on `md5 0.7`; a separate migration could retire it from the lockfile. | todo |
| F100 | T43 implementer | `src/repair/scan.rs` | The scan clones a context per range when resolving verdicts; unchanged here. | dropped (small allocation outside a hot loop) |
| F101 | RC-09 review | `src/repair/scope/ledger.rs` | `add_saturating` loads then `fetch_add`, so additions within `n` of `u64::MAX` could still wrap. | completed |
| F102 | RC-09 review | `src/repair/mod.rs` | `PurgeKeys::new` folds the expected set on the async runtime before `spawn_blocking`. | todo |
| F103 | RC-09 review | `src/repair/purge.rs` | `mark_kept_ancestors` allocates one `PathBuf` per newly marked ancestor. | todo |
| F104 | RC-09 review | `src/util/path.rs` | The T26 skip rests on a one-off measurement with no retained harness to re-check on other installs. | dropped (the decision is recorded; a retained harness for a skipped change is not warranted) |
| F105 | RC-09 review | `src/logging.rs`, docs | Buffering delays external `tail -f` by up to 64 KiB; worth stating if log streaming is supported. | todo |
| F106 | RC-16 review | `Cargo.toml` | The tokio comment promises a saving `reqwest` already spends; reword to the real one (dropping `process`/`signal`, dev-only `test-util`). | completed |
| F107 | RC-16 review | tests/benchmarks | The MD5 throughput claim has no committed reproduction; add a small `#[ignore]`d benchmark or script. | completed |
| F108 | RC-16 review | `tests/common/mod.rs`, `tests/artifact_bridge/server.rs`, `tests/peak_heap.rs` | The same `md5_hex` loop is duplicated three times; one shared helper would do. | todo |
| F109 | RC-16 review | `tools/sync`, `tools/capture` | The dev tools stay outside the lint policy; consider a tools-only lint check when next touched. | todo |
| F110 | T44 implementer | `tests/repair_offline.rs`, `tests/common/mod.rs` | Manifest ordering is pinned only at module level; a binary-level case needs a fixture that fails one selected manifest. | todo |
| F111 | RC-17 review | `tests/repair_machinery.rs` | No end-to-end stale-verdict case that mutates a file between the plan scan and the repair. | dropped (superseded by F32 (same end-to-end case)) |
| F112 | RC-17 review | `src/repair/phases/post.rs` | Partial-copy tests stop at the helper; the post-phase outcome/config.ini unbumped claim is not asserted. | todo |
| F113 | RC-17 review | `tests/repair_offline.rs` | The duration process test always passes `--scan`; absence of `phase=scan` on an unscanned run is unasserted. | todo |
| F114 | T45 implementer | `src/phase.rs` | `phase=plan` excludes the scan and `publish_plan`; purge execution is not timed at all. | todo |
| F115 | T45 implementer | `src/phase.rs` | `PhaseTimer` logs on drop during an error path, reporting time-to-failure as if the phase completed. | todo |
| F116 | RC-18 review | `tests/cli_contract.rs` | `check_golden` allows `PROGRESS` between `REPORT` and `PLAN`, wider than the documented window. | todo |
| F117 | RC-18 review | `README.md` | The data-plane section omits the one-record-per-event rule and that `PLAN` precedes `SUMMARY`. | todo |
| F118 | RC-18 review | `tests/repair_offline.rs` | Module docs still describe the pre-split case layout ("first ten cases", "three cases run the real binary"). | todo |
| F119 | T46 implementer | `tests/cli_contract.rs` | Only the JSON plane is pinned; the text plane relies on unit tests. | todo |
| F120 | T47 implementer | `tests/repair_offline.rs`, `tests/cli_contract.rs` | The `cli_contract` comment still points at `repair_offline.rs` for ceilings, now in `repair_limits.rs`. | todo |
| F121 | HYP-01 review | `docs/07-panic-audit.md` | Still describes `num_from_string` as `u64 as i64` wrapping above `i64::MAX`; the visitor uses `i64::try_from`. | todo |
| F122 | HYP-01 review | `src/hyp.rs` | `ManifestFile.compressed_size` is deserialized but never read; its doc implies an in-crate clamp that does not exist. | todo |
| F123 | PATHKEY-01 review | `tools/sync` | No end-to-end case exercises a map-key site (`diff_maps`, `check_mixed_case_keys`, planner pairing) with a non-ASCII pair. | todo |
| F124 | PATHKEY-01 review | `src/repair/scope/filter.rs`, `src/util/path.rs` | The docs imply the glob fold is a different width; `glob`'s fold is also ASCII-only, so the real distinction is predicate-vs-key plus Windows-only folding. | todo |
| F125 | PATHKEY-01 review | `tools/sync/src/hash/set.rs` | `parse_hash_list` still uses Unicode `to_lowercase`, so `bla\u{212A}e3` folds onto `blake3`. | todo |
| F126 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The wrap test's failure on the old shape is probabilistic; a test-only pause seam would make it deterministic. | todo |
| F127 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The wrap test spawns ~16k threads (1-3 s); trim rounds if CI time matters. | todo |
| F128 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The unlimited charge/settle hot path is now a CAS loop; revisit only if profiling shows counter contention (e.g. sharding). | dropped (no profile shows counter contention; speculative) |
| F129 | RETRY-01 review | `src/retry.rs` | The jitter doc's "distinct by construction" holds only until the salt times the multiplier wraps (~6.9e9), not merely the counter. | todo |
| F130 | RETRY-01 review | `src/retry.rs` | No explicit test that `jitter` is injective over a fixed attempt's caller range; the invariant is implicit. | todo |
| F131 | METADATA-01 review | `src/repair/phases/metadata.rs` | The class table builds failures via `class().failure(...)`, so `join_failure` bypassing `class()` would go unnoticed. | todo |
| F132 | METADATA-01 review | `src/repair/phases/metadata.rs` | The enum `label()` strings duplicate the literals in `fetch`; a shared constant would pin them. | todo |
| F133 | METADATA-01 review | `src/repair/phases/metadata.rs` | No fetch-level test that a branch error settling first is retained when config succeeds. | todo |
| F134 | METADATA-01 review | `src/repair/phases/metadata.rs` | The enum doc implies one exit class per read, but inner read errors differ from join errors. | todo |
| F135 | METADATA-01 review | `tests/repair_offline.rs` | The non-chunk mode test asserts the branch request was issued, which depends on polling order under cancellation. | todo |
| F136 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | The mock `serve` spawns listener/per-connection tasks with no shutdown, relying on runtime teardown. | todo |
| F137 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | `OrderedResults` is only unit-tested via manual `push`, not driven through an out-of-order stream. | todo |
| F138 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | `local_reuse_map` still awaits every fetch after a failure; early exit would need a different policy. | dropped (speculative future policy; shared fan-out already supports it) |
| F139 | JOBS-01 review | `tests/cli_contract.rs`, `src/cli.rs` | No boundary case through the `--io-threads` alias; only `--jobs` is exercised at the ceiling. | todo |
| F140 | JOBS-01 review | `src/cli.rs` | `--help` no longer prints clap's `[default: 4]` because the field is `Option<usize>`; the prose still states it. | todo |
| F141 | JOBS-01 review | `src/cli.rs` | `MAX_JOBS = 1024` still asks for 1025 worker threads; a lower ceiling may be worth revisiting if runtime construction is the real risk. | todo |
| F142 | LOGGING-01 review | `src/logging.rs` | `FILE_SINKS` is append-only for the process lifetime; repeated installs retain every sink and handle. | todo |
| F143 | LOGGING-01 review | `src/logging.rs` | No test drives `install_sink` through `flush_log`; only `flush_sinks` is pinned. | todo |
| F144 | LOGGING-01 review | `src/logging.rs` | Only the first sink's flush error is reported; later failures are silent by design. | todo |
| F145 | LOCK-01 review | `src/repair/lock.rs` | `may_write_body` still accepts a hard link to an empty/`pid=` regular file, and a symlink swapped between open and metadata defeats the check. | todo |
| F146 | LOCK-01 review | `src/repair/lock.rs` tests | Lock files now persist, so tests leave `girpr-*.lock` files and a dangling symlink in the temp directory. | todo |
| F147 | LOCK-01 review | `src/repair/lock.rs` | The Unix-only lock cases never ran on this host; they need a Linux/macOS run. | todo |
| F148 | INI-01 review | `src/util/ini.rs` | The refusal says "does not exist" even when the path exists as a non-directory. | todo |
| F149 | INI-01 review | `tests/` | No CLI-level case for the game dir vanishing between `prepare_install` and the post-phase bump. | todo |
| F150 | INI-01 review | `src/util/ini.rs` | `split_first_eq` is a thin wrapper over `split_once`; keep it only while the shared rule earns its name. | todo |
| F151 | BUILD-01 review | `Cargo.toml` (members) | Only `girpr` carries `rust-version`; `tools/*` and `crates/*` have none, so the workspace MSRV error names redb. | todo |
| F152 | BUILD-01 review | `src/repair/scan.rs` | The MD5 benchmark covers whole-file hashing but not the dominant `scan_file`/`scan_reader` read path. | todo |
| F153 | BUILD-01 review | `src/sophon/stream.rs` | Two adjacent `use futures_util::...` lines could be one brace import. | todo |

## Follow-up batches

Filled in by the follow-up batching session, then executed batch by batch.

| Batch | Follow-ups | Status | Notes |
|---|---|---|---|
| HYP-01 | F01, F07, F64 | done | Wave A; hyp number parsing and retry seams; medium; 87db654, 3337ea1, eaf0b2c |
| PATHKEY-01 | F03, F04 | done | Wave A; path-key folding outside the lock; medium; 2956bec, 0708ab5 |
| LEDGER-01 | F08, F101 | done | Wave A; meter saturation on add and release; medium; 42eea14, 0a2c5f5 |
| RETRY-01 | F62, F81 | done | Wave A; retry jitter spread and clippy variant names; low; 24c2a99, f4589fb, ef9daf3 |
| METADATA-01 | F13, F14, F17 | done | Wave A; metadata join cancellation and error mapping; medium; b29da6c, 7714bea, bbaeda1, dc93735 |
| MANIFEST-01 | F15, F16 | done | Wave A; manifest fetch reindex dedup and cancellation; medium; 68402c6, 9ca2194, 309ff77 |
| JOBS-01 | F22, F40, F44 | done | Wave A; `--jobs`/`--io-threads` validation and warnings; medium; 7c5dd1c, b989167, e7842a2, a759a51, 80120f7 |
| LOGGING-01 | F49, F54, F55 | done | Wave A; log flush lifecycle and tests; low; c42e863, 3e426bc |
| LOCK-01 | F35, F36, F37, F45 | done | Wave A; lock safety, key hashing, holder diagnostics; high; 6cc5d97, 8083753, 2c02ef5, 9642547, a653c9c, a093aa4, 12d4201 |
| INI-01 | F61, F91 | done | Wave A; config-ini write contract and first-`=` helper; medium; 66e0f30, 0916dc2 |
| BUILD-01 | F39, F92, F93, F106, F107 | done | Wave A; dependency truth, toolchain pin, MD5 benchmark; low; 279c241, 7465081, c97827f, 8d38c6b, bf504df, f2bd9a6 |
| BUILD-TOOLS-01 | F88, F99 | in_progress | Wave A; dev-tool dependency narrowing; medium |
| TEST-HELPERS-01 | F108 | todo | Wave A; one shared `md5_hex` test helper; low |
| LINT-01 | F97, F109 | todo | Wave A, after BUILD-TOOLS-01; central test-lint policy; low |
| TEMP-WRITER-01 | F25, F26, F27, F28, F30 | todo | Wave B; temp writer rollback, refunds, docs, seek; medium |
| TEMP-PUBLISH-01 | F05, F06, F09, F12 | todo | Wave B; temp-sibling publish of audio and plan files; medium |
| INPLACE-01 | F29, F33, F34, F70, F79 | todo | Wave B; in-place reuse accuracy and guards; medium |
| CHECK-01 | F19, F21, F23, F52 | todo | Wave B; check-phase progress, totals, panic verdict; high |
| REPORT-API-01 | F42, F43, F48, F83 | todo | Wave B; report type visibility and construction; medium |
| PHASE-01 | F114, F115 | todo | Wave B; phase-timer coverage and error-path logging; low |
| TEST-MANIFEST-01 | F18, F110 | todo | Wave B, after MANIFEST-01; multi-manifest fixtures and binary ordering; medium |
| ATOMIC-01 | F11, F60, F65, F66 | todo | Wave C, after INI-01 and TEMP-PUBLISH-01; atomic-writer edges and docs; low |
| PLANID-01 | F56, F57, F58, F59 | todo | Wave C, after INPLACE-01 and ATOMIC-01; plan identity; high |
| EXIT-01 | F85, F86, F87 | todo | Wave C, after CHECK-01; named exit-code API and prose; medium |
| DRYRUN-01 | F46, F47, F51, F73 | todo | Wave C, after TEMP-PUBLISH-01 and PHASE-01; dry-run post contract; high |
| PURGE-PERF-01 | F20, F102, F103 | todo | Wave C, after TEMP-PUBLISH-01; move purge work off async workers; high |
| MD5-TYPE-01 | F24 | todo | Wave D, after PLANID-01; carry `Md5` through scan results; medium |
| PATHHELPERS-01 | F67, F68, F69, F71 | todo | Wave D, after INPLACE-01, PLANID-01, DRYRUN-01, PURGE-PERF-01; low |
| REFACTOR-01 | F80, F82 | todo | Wave D, after PHASE-01, INPLACE-01, PURGE-PERF-01; medium |
| TEST-REGRESSION-01 | F32, F112, F113, F126, F127, F130 | todo | Wave D, after INPLACE-01, DRYRUN-01, TEMP-PUBLISH-01; mutation, partial-copy and timing regressions; low |
| TEST-OUTPUT-01 | F53, F116, F119 | todo | Wave D, after CHECK-01 and PHASE-01; output-plane golden tightening; medium |
| DOCS-USER-01 | F38, F41, F95, F105, F117 | todo | Wave E, after LOCK-01, JOBS-01, LOGGING-01, TEST-OUTPUT-01; low |
| DOCS-COMMENTS-01 | F63, F72, F74, F77, F118, F120, F121, F122, F124, F129 | todo | Wave E, after REFACTOR-01 and PATHHELPERS-01; stale comments, module summaries, rustdoc links, and doc precision; low |
| SYNC-01 | F123, F125 | todo | Wave A; girsync map-key call-site coverage and ASCII hash-name folding; low |
| METADATA-02 | F131, F132, F133, F134, F135 | todo | Wave D; metadata classification tests, label constants and doc precision; low |
| MANIFEST-02 | F136, F137 | todo | Wave D; manifest mock shutdown and out-of-order stream coverage; low |
| JOBS-02 | F139, F140, F141 | todo | Wave D; alias boundary case, help default annotation and ceiling review; low |
| LOGGING-02 | F142, F143, F144 | todo | Wave D; sink registry lifetime, install-to-flush coverage and per-sink errors; low |
| LOCK-02 | F145, F146, F147 | todo | Wave D; lock handle-identity hardening, test litter and Unix-only coverage; medium |
| INI-02 | F148, F149, F150 | todo | Wave D; ini refusal wording, external-race coverage and helper scope; low |
| BUILD-02 | F151, F152, F153 | todo | Wave D; per-member MSRV, scan-path benchmark and import tidy; low |

Merge-conflict warnings: `src/repair/phases/post.rs` is the hottest file
(TEMP-PUBLISH-01, DRYRUN-01, PURGE-PERF-01, PATHHELPERS-01) and
`src/repair/mod.rs` is next (PHASE-01, DRYRUN-01, PURGE-PERF-01, REFACTOR-01);
never run two batches touching either file concurrently. `src/repair/scan.rs`
serializes INPLACE-01, PLANID-01, MD5-TYPE-01 and PATHHELPERS-01.

Dropped follow-ups are marked in the table above with the reason.

## Notes

- `INDEX.md` defines the task map, dependencies, and review checkpoints.
- `completed` means the task and its review batch are done.
- `reverted` means the original work was undone and a rework row was added.
- `skipped` means the task was stale or no longer applicable; record why.
- Task ordering is the report's overall ordering; do not reorder the table.
- T26 skipped with evidence: path resolution measured ~1.5% of wall clock warm
  (~1.9% cold) on a 10k-file install, below the 2-3% gate, and a parent-prefix
  cache cannot remove the leaf `canonicalize`, so the containment window would
  buy ~0.7% end-to-end. No code change.
