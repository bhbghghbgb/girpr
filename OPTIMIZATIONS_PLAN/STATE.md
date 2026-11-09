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
| F05 | RC-02 review | `src/repair/phases/post.rs` | `copy_publish_with` keeps its own `PUBLISH_TMP_SUFFIX` append; adopt `util::path::temp_sibling`. | completed |
| F06 | RC-02 review | `src/repair/scope/report.rs` | Temp-then-rename replaces a symlink at the target and resets the target's mode; document the trade. | completed |
| F07 | T02 implementer | `src/hyp.rs` | Negative sizes are still accepted and clamped to zero; decide whether to reject them explicitly. | completed |
| F08 | T03 implementer | `src/repair/scope/ledger.rs` | `Summary::release_ram_bytes` still uses unchecked `fetch_sub`; consider the same saturation guard. | completed |
| F09 | T05 implementer | `src/repair/phases/post.rs` | The copy-fallback caller path is only unit-tested through the injected helper, not a real failed rename. | completed |
| F10 | T05 implementer | `src/repair/phases/post.rs` | The fixed temp sibling name would collide if two moves target the same path concurrently. | dropped (one run at a time under the lock makes the collision unreachable) |
| F11 | T06 implementer | `src/repair/scope/report.rs` | Untested `file_name() == None` fallback; cleanup warns when a directory occupies the temp path. | completed |
| F12 | T07 implementer | `src/repair/purge.rs` | A purge test hard-codes the `.girpr_tmp` spelling instead of using the shared name helper. | completed |
| F13 | RC-03 review | `src/repair/phases/metadata.rs` | `join!` waits out a sibling's retry schedule after an authoritative failure; cancel it once its error is settled. | completed |
| F14 | RC-03 review | `src/repair/phases/metadata.rs` | Spawn-join failures are mapped ad hoc (ignore/blacklist vs audio/version exit classes). | completed |
| F15 | RC-03 review | `src/repair/phases/manifest.rs` | Bounded-fetch reindex plumbing and the concurrency log line are duplicated for latest and local. | completed |
| F16 | RC-03 review | `src/repair/phases/manifest.rs` | A fast failure still waits for slower siblings; reorder incrementally and cancel the rest. | completed |
| F17 | T08 implementer | `src/repair/phases/metadata.rs` | Blocking local reads are dropped unawaited on early return; drain them for stricter lifetime hygiene. | completed |
| F18 | T09 implementer | `tests/common` | The mock models a single latest manifest; multi-manifest integration coverage needs fixture work. | completed |
| F19 | RC-04 review | `src/repair/phases/check.rs` | `--check-only` emits progress only after the whole verify pass; a ticker should keep it live. | completed |
| F20 | RC-04 review | `src/repair/phases/post.rs`, `src/repair/purge.rs` | `delete_deprecated` and `apply_purge` still run synchronous filesystem loops on async workers. | completed |
| F21 | RC-04 review | `src/repair/phases/check.rs` | A panicking check task becomes a damaged-file verdict instead of failing the run, unlike purging. | completed |
| F22 | RC-04 review | `src/main.rs`, `src/cli.rs` | A huge `--io-threads` now fails runtime construction rather than clamping; validate the flag. | completed |
| F23 | T10 implementer | `src/repair/phases/check.rs` | The damaged-chunk total from check is only logged, not carried on `Summary`. | completed |
| F24 | RC-05 review | `src/util/md5.rs` | `scan_reader`/`Scan.file_md5` return hex that callers re-parse; carry `Md5` instead. | completed |
| F25 | RC-05 review | `src/repair/file/temp.rs` tests | The rollback resume-point test sets `tmp_len` by hand; the caller's update is not exercised end-to-end. | completed |
| F26 | RC-05 review | `src/repair/file/temp.rs` | The sink's seek-failure refund path has no test seam. | completed |
| F27 | RC-05 review | `src/repair/file/temp.rs` | The source handle stays open across `promote_tmp`; drop it first to avoid the platform assumption. | completed |
| F28 | T13 implementer | `src/repair/file/temp.rs` | `try_reuse_slice`'s doc still says it "Returns `false`"; the return type is `ChunkOutcome`. | completed |
| F29 | T13 implementer | `src/repair/file/inplace.rs` | The in-place path still resolves reuse sources per chunk. | completed |
| F30 | T14 implementer | `src/repair/file/temp.rs` | `try_reuse_slice` seeks `out` to the chunk offset even on the download path, now redundant. | completed |
| F31 | T14 implementer | `src/repair/file/temp.rs` | The sink remains unbuffered; buffering needs explicit restart/digest flush handling. | dropped (buffering needs restart/digest flush semantics for no measured win) |
| F32 | RC-06 review | `tests/repair_machinery.rs` | No end-to-end test mutates a file between plan and repair to prove the fresh-scan fallback. | completed |
| F33 | RC-06 review | `src/repair/file/inplace.rs` | The reuse debug log's `age_secs` is the file's mtime age, not the scan snapshot's age. | completed |
| F34 | RC-06 review | `src/repair/scan.rs`, `src/repair/file/inplace.rs` | Coarse-mtime filesystems can evade the exact length+mtime guard; document or harden. | completed |
| F35 | RC-07 review | `src/repair/lock.rs` | `Drop` unlocks then deletes, a narrow Unix race; leave the file or re-verify after acquire. | completed |
| F36 | RC-07 review | `src/repair/lock.rs` | The temp lock is opened by name and truncated; a pre-created symlink could truncate a user file. | completed |
| F37 | RC-07 review | `src/repair/lock.rs` | `key_for` hashes `to_string_lossy()`; distinct non-UTF-8 paths can collide. | completed |
| F38 | RC-07 review | `src/repair/purge.rs`, `docs/01-03` | Comments and docs still describe the in-install `girpr.lock` and keep-set wording. | completed |
| F39 | RC-07 review | `README.md`, `Cargo.toml` | README claims Rust 1.85+, but `File::try_lock` needs 1.89; no `rust-version` pin. | completed |
| F40 | RC-07 review | `src/cli.rs` | `excessive_jobs_warning` says workers can fall below file tasks, but the runtime is sized to `max(parallelism, jobs+1)`. | completed |
| F41 | RC-07 review | `src/main.rs`, `src/repair/**`, `tools/sync`, `docs/02-06` | Stale `--io-threads` prose remains after the `--jobs` rename. | completed |
| F42 | RC-07 review | `src/lib.rs` | `test_support` re-exports `stream_chunk_to` beyond the task's list and duplicates root items; document the seam. | completed |
| F43 | T18 implementer | `src/report/mod.rs` | `Summary` public fields have types (`Meter`, `SummaryFlags`, `Ceiling`) no longer nameable outside the crate. | completed |
| F44 | T19 implementer | `src/cli.rs` | On hosts with parallelism below 4, the default `--jobs 4` warns on every run. | completed |
| F45 | T17 implementer | `src/repair/lock.rs` | On Windows the exclusive lock hides the body, so refusals usually cannot name the holder pid. | completed |
| F46 | RC-08 review | `src/repair/purge.rs`, `src/repair/phases/post.rs` | `--purge-before --purge-after --dry-run` lists the same cleanup candidates twice. | completed |
| F47 | RC-08 review | `src/repair/phases/post.rs` | A dry run that trips the post-repair ceiling logs error-level `limit exceeded` yet exits 0. | completed |
| F48 | RC-08 review | `src/report/mod.rs` | `emit_progress`/`PROGRESS_INTERVAL_SECS` are no longer re-exported; restore if that is API. | completed |
| F49 | RC-08 review | `src/logging.rs` tests | A flush test asserts it is the only setter of the global sink; fragile if another test installs tracing. | completed |
| F50 | T20 implementer | `src/repair/phases/post.rs` | A dry run now does a second purge walk plus the audio walk, adding I/O and network work. | dropped (the extra walks are the intended cost of exercising the post-phase) |
| F51 | T20 implementer | `src/repair/phases/post.rs` | A post-phase purge re-plan failure in a dry run now maps to exit 3. | completed |
| F52 | T21 implementer | `src/repair/phases/check.rs` | Check-only counters are folded after `collect`, so the ticker may report `done=0` mid-phase. | completed |
| F53 | T21 implementer | `tests/cli_contract.rs` | No integration assertion that a real binary run emits exactly one `SUMMARY`. | completed |
| F54 | T22 implementer | `src/logging.rs` | The flush handle is a `OnceLock` for the first sink only; a second `init_tracing` is unflushable. | completed |
| F55 | T22 implementer | `src/logging.rs` | The flush-failure stderr branch has no test (stderr capture is process-level). | completed |
| F56 | RC-10 review | `src/repair/scan.rs` | `ScanReport` carries no plan identity; index alignment silently mismatches a different plan. | completed |
| F57 | RC-10 review | `src/repair/phases/plan.rs` | `FileId` derived `Debug` prints `FileId(0)` rather than the index as the task asked. | completed |
| F58 | T27 implementer | `src/repair/phases/plan.rs`, `src/repair/scan.rs` | `RepairPlan::files` and `ScanReport::files` stay public, so callers can bypass the id accessors. | completed |
| F59 | T27 implementer | `src/repair/scope/report.rs` | `DiskPlan::for_id` returns a default for an out-of-range id; `PlanDocument` uses raw indices. | completed |
| F60 | RC-11 review | `src/util/atomic.rs` | Module docs link a `pub(crate)` `temp_sibling`, which may warn under `cargo doc`. | completed |
| F61 | RC-11 review | `src/util/ini.rs` | `write_config_ini` now creates a missing game directory through the helper where it previously failed. | completed |
| F62 | RC-11 review | `src/retry.rs` | Jitter is salted by process id and attempt only, so concurrent files share the same spread. | completed |
| F63 | RC-11 review | `src/repair/file/mod.rs` | A comment still budgets 5 × (15 s + 10 s); with the final sleep removed it is ~100 s. | completed |
| F64 | RC-11 review | `src/hyp.rs` | No call-site test that `get_node`/`get_direct` retry a retryable error and stop on a fatal one. | completed |
| F65 | T28 implementer | `src/util/ini.rs` tests | No config-specific "no temp left" assertion after the helper extraction. | completed |
| F66 | T28 implementer | `src/util/atomic.rs` | Error context is artifact-neutral; artifact-specific labels would need a parameter. | completed |
| F67 | RC-12 review | `src/repair/phases/post.rs` | `move_audio_cache` still strips the cache prefix manually; it could use `rel_from_base`. | completed |
| F68 | RC-12 review | `src/util/path.rs` | `rel_from_base` and `join_rel` each inline the lossy `/` conversion; one private helper would be literal. | completed |
| F69 | RC-12 review | `src/repair/scan.rs` | The helper tolerates length mismatch; a `debug_assert` at the two call sites would still be useful. | completed |
| F70 | RC-12 review | `src/repair/file/inplace.rs` | `source_ok_by_range` is the inverse mapping; cross-reference or pin it near the shared remap. | completed |
| F71 | T30 implementer | `src/util/path.rs` | `join_rel` assumes an already-clean `dir_rel` with no trailing separator. | completed |
| F72 | T30 implementer | `src/util/mod.rs` | The `path` module summary does not mention the two new rel helpers. | completed |
| F73 | RC-13 review | `src/repair/mod.rs`, `README.md`, `docs/02` | `--print-plan --dry-run` still fetches the deprecated list, contradicting the documented contract. | completed |
| F74 | RC-13 review | `src/report/emit.rs` | Public docs link to the private `write_line`, leaving a rustdoc warning. | completed |
| F75 | T33 implementer | `tools/sync` | The sibling crate has its own unrelated `Record::event` implementation. | dropped (unrelated sibling-crate implementation) |
| F76 | T33 implementer | `src/util/ini.rs` | No pure `render_config_ini` helper; future tests observing rendering without writing would need one. | dropped (helper for a test that does not exist yet) |
| F77 | RC-14 review | `src/repair/mod.rs` | `publish_plan`'s doc describes a disk-figures computation that moved into `build_and_assert_plan`. | skipped (doc already matches) |
| F78 | RC-14 review | `src/repair/mod.rs`, `src/repair/file/inplace.rs` | Four new `too_many_arguments` allows are parameter bags for T35 to absorb. | dropped (superseded by F82 (current five-site inventory)) |
| F79 | RC-14 review | `src/repair/file/inplace.rs`, `src/repair/mod.rs` | Trim/tier decisions and plan-step guards have only end-to-end coverage. | completed |
| F80 | T34 implementer | `src/repair/mod.rs` | `build_and_assert_plan` is still ~150 lines and could split further. | completed |
| F81 | T34 implementer | `src/retry.rs` | Clippy `enum_variant_names` warning predates the work and is still present. | completed |
| F82 | RC-15 review | `src/repair/mod.rs`, `src/repair/file/mod.rs`, `src/repair/file/inplace.rs` | Five `too_many_arguments` allows remain outside T35's four sites. | completed |
| F83 | RC-15 review | `src/report/emit.rs` | `ReportProvenance` exposes every field publicly; a constructor or narrower visibility would harden it. | completed |
| F84 | T35 implementer | `src/sophon/stream.rs` | The streaming path joins the URL per attempt; carrying the joined URL would avoid the allocation. | dropped (negligible allocation on a network-bound path) |
| F85 | RC-15 review | `src/config.rs`, `tests/repair_offline.rs` | `PartialEq<i32> for RunExit` is a numeric escape hatch; convert test assertions to variants and drop it. | completed |
| F86 | RC-15 review | `src/repair/phases/check.rs`, `src/main.rs` | Prose still describes exit codes as bare numbers. | completed |
| F87 | T36 implementer | `src/config.rs` | `From<RunFailure> for RunExit` is exercised only by tests; no production caller yet. | completed |
| F88 | T37 implementer | `tools/capture` | The tool still depends on `tokio` with `full`; scoped out of the build batch. | completed |
| F89 | T37 implementer | `Cargo.toml` | `reqwest` unifies tokio's `net`/`fs`/`io-util`/`sync` into any build; reqwest's own surface, not removable here. | dropped (superseded by F106) |
| F90 | T38 implementer | `src/util/ini.rs` | The old pattern's `\s*` could match a newline; the hand-written line scan intentionally does not, and nothing depends on it. | dropped (intentional difference; nothing depends on the old behavior) |
| F91 | T38 implementer | `src/util/ini.rs` | `read_game_version` and `parse_ini` share the "first `=`" rule without a common helper. | completed |
| F92 | T39 implementer | `Cargo.toml` | `futures-util` defaults pull `futures-macro` and `slab`; a `std`-only feature set could shrink the graph. | completed |
| F93 | T39 implementer | `src/sophon/stream.rs` | The `Stream` bound names `futures-core` directly; `futures_util::stream::Stream` would be the same trait. | completed |
| F94 | T40 implementer | repo | No CI exists; a future CI job must run `cargo check -p girsync -p gircap` or a workspace job. | dropped (repository has no CI system) |
| F95 | T40 implementer | `README.md` | The nextest note hard-codes a test count ("362 tests") that is now 470 and drifts; de-number it. | skipped (no count remains) |
| F96 | T41 implementer | `Cargo.toml` | Release builds strip symbols; crash triage may want `strip = "debuginfo"` or a separate profile. | dropped (crash-triage symbol needs are hypothetical) |
| F97 | T42 implementer | `clippy.toml` | Revisit `allow-unwrap-in-tests`/`allow-expect-in-tests` to replace the code-level test allows. | completed |
| F98 | T43 implementer | `Cargo.toml` | `md-5`'s optional `asm`/`md5-asm` backend may beat the portable path on x86; measure separately. | dropped (speculative; the portable upgrade already passed the gate) |
| F99 | T43 implementer | `tools/sync`, `tools/capture` | Both tools still depend on `md5 0.7`; a separate migration could retire it from the lockfile. | completed |
| F100 | T43 implementer | `src/repair/scan.rs` | The scan clones a context per range when resolving verdicts; unchanged here. | dropped (small allocation outside a hot loop) |
| F101 | RC-09 review | `src/repair/scope/ledger.rs` | `add_saturating` loads then `fetch_add`, so additions within `n` of `u64::MAX` could still wrap. | completed |
| F102 | RC-09 review | `src/repair/mod.rs` | `PurgeKeys::new` folds the expected set on the async runtime before `spawn_blocking`. | completed |
| F103 | RC-09 review | `src/repair/purge.rs` | `mark_kept_ancestors` allocates one `PathBuf` per newly marked ancestor. | completed |
| F104 | RC-09 review | `src/util/path.rs` | The T26 skip rests on a one-off measurement with no retained harness to re-check on other installs. | dropped (the decision is recorded; a retained harness for a skipped change is not warranted) |
| F105 | RC-09 review | `src/logging.rs`, docs | Buffering delays external `tail -f` by up to 64 KiB; worth stating if log streaming is supported. | completed |
| F106 | RC-16 review | `Cargo.toml` | The tokio comment promises a saving `reqwest` already spends; reword to the real one (dropping `process`/`signal`, dev-only `test-util`). | completed |
| F107 | RC-16 review | tests/benchmarks | The MD5 throughput claim has no committed reproduction; add a small `#[ignore]`d benchmark or script. | completed |
| F108 | RC-16 review | `tests/common/mod.rs`, `tests/artifact_bridge/server.rs`, `tests/peak_heap.rs` | The same `md5_hex` loop is duplicated three times; one shared helper would do. | completed |
| F109 | RC-16 review | `tools/sync`, `tools/capture` | The dev tools stay outside the lint policy; consider a tools-only lint check when next touched. | completed |
| F110 | T44 implementer | `tests/repair_offline.rs`, `tests/common/mod.rs` | Manifest ordering is pinned only at module level; a binary-level case needs a fixture that fails one selected manifest. | completed |
| F111 | RC-17 review | `tests/repair_machinery.rs` | No end-to-end stale-verdict case that mutates a file between the plan scan and the repair. | dropped (superseded by F32 (same end-to-end case)) |
| F112 | RC-17 review | `src/repair/phases/post.rs` | Partial-copy tests stop at the helper; the post-phase outcome/config.ini unbumped claim is not asserted. | completed |
| F113 | RC-17 review | `tests/repair_offline.rs` | The duration process test always passes `--scan`; absence of `phase=scan` on an unscanned run is unasserted. | completed |
| F114 | T45 implementer | `src/phase.rs` | `phase=plan` excludes the scan and `publish_plan`; purge execution is not timed at all. | completed |
| F115 | T45 implementer | `src/phase.rs` | `PhaseTimer` logs on drop during an error path, reporting time-to-failure as if the phase completed. | completed |
| F116 | RC-18 review | `tests/cli_contract.rs` | `check_golden` allows `PROGRESS` between `REPORT` and `PLAN`, wider than the documented window. | completed |
| F117 | RC-18 review | `README.md` | The data-plane section omits the one-record-per-event rule and that `PLAN` precedes `SUMMARY`. | completed |
| F118 | RC-18 review | `tests/repair_offline.rs` | Module docs still describe the pre-split case layout ("first ten cases", "three cases run the real binary"). | completed |
| F119 | T46 implementer | `tests/cli_contract.rs` | Only the JSON plane is pinned; the text plane relies on unit tests. | completed |
| F120 | T47 implementer | `tests/repair_offline.rs`, `tests/cli_contract.rs` | The `cli_contract` comment still points at `repair_offline.rs` for ceilings, now in `repair_limits.rs`. | completed |
| F121 | HYP-01 review | `docs/07-panic-audit.md` | Still describes `num_from_string` as `u64 as i64` wrapping above `i64::MAX`; the visitor uses `i64::try_from`. | completed |
| F122 | HYP-01 review | `src/hyp.rs` | `ManifestFile.compressed_size` is deserialized but never read; its doc implies an in-crate clamp that does not exist. | completed |
| F123 | PATHKEY-01 review | `tools/sync` | No end-to-end case exercises a map-key site (`diff_maps`, `check_mixed_case_keys`, planner pairing) with a non-ASCII pair. | completed |
| F124 | PATHKEY-01 review | `src/repair/scope/filter.rs`, `src/util/path.rs` | The docs imply the glob fold is a different width; `glob`'s fold is also ASCII-only, so the real distinction is predicate-vs-key plus Windows-only folding. | completed |
| F125 | PATHKEY-01 review | `tools/sync/src/hash/set.rs` | `parse_hash_list` still uses Unicode `to_lowercase`, so `bla\u{212A}e3` folds onto `blake3`. | completed |
| F126 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The wrap test's failure on the old shape is probabilistic; a test-only pause seam would make it deterministic. | completed |
| F127 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The wrap test spawns ~16k threads (1-3 s); trim rounds if CI time matters. | completed |
| F128 | LEDGER-01 review | `src/repair/scope/ledger.rs` | The unlimited charge/settle hot path is now a CAS loop; revisit only if profiling shows counter contention (e.g. sharding). | dropped (no profile shows counter contention; speculative) |
| F129 | RETRY-01 review | `src/retry.rs` | The jitter doc's "distinct by construction" holds only until the salt times the multiplier wraps (~6.9e9), not merely the counter. | completed |
| F130 | RETRY-01 review | `src/retry.rs` | No explicit test that `jitter` is injective over a fixed attempt's caller range; the invariant is implicit. | completed |
| F131 | METADATA-01 review | `src/repair/phases/metadata.rs` | The class table builds failures via `class().failure(...)`, so `join_failure` bypassing `class()` would go unnoticed. | completed |
| F132 | METADATA-01 review | `src/repair/phases/metadata.rs` | The enum `label()` strings duplicate the literals in `fetch`; a shared constant would pin them. | completed |
| F133 | METADATA-01 review | `src/repair/phases/metadata.rs` | No fetch-level test that a branch error settling first is retained when config succeeds. | completed |
| F134 | METADATA-01 review | `src/repair/phases/metadata.rs` | The enum doc implies one exit class per read, but inner read errors differ from join errors. | completed |
| F135 | METADATA-01 review | `tests/repair_offline.rs` | The non-chunk mode test asserts the branch request was issued, which depends on polling order under cancellation. | completed |
| F136 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | The mock `serve` spawns listener/per-connection tasks with no shutdown, relying on runtime teardown. | completed |
| F137 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | `OrderedResults` is only unit-tested via manual `push`, not driven through an out-of-order stream. | completed |
| F138 | MANIFEST-01 review | `src/repair/phases/manifest.rs` | `local_reuse_map` still awaits every fetch after a failure; early exit would need a different policy. | dropped (speculative future policy; shared fan-out already supports it) |
| F139 | JOBS-01 review | `tests/cli_contract.rs`, `src/cli.rs` | No boundary case through the `--io-threads` alias; only `--jobs` is exercised at the ceiling. | completed |
| F140 | JOBS-01 review | `src/cli.rs` | `--help` no longer prints clap's `[default: 4]` because the field is `Option<usize>`; the prose still states it. | completed |
| F141 | JOBS-01 review | `src/cli.rs` | `MAX_JOBS = 1024` still asks for 1025 worker threads; a lower ceiling may be worth revisiting if runtime construction is the real risk. | completed |
| F142 | LOGGING-01 review | `src/logging.rs` | `FILE_SINKS` is append-only for the process lifetime; repeated installs retain every sink and handle. | completed |
| F143 | LOGGING-01 review | `src/logging.rs` | No test drives `install_sink` through `flush_log`; only `flush_sinks` is pinned. | completed |
| F144 | LOGGING-01 review | `src/logging.rs` | Only the first sink's flush error is reported; later failures are silent by design. | completed |
| F145 | LOCK-01 review | `src/repair/lock.rs` | `may_write_body` still accepts a hard link to an empty/`pid=` regular file, and a symlink swapped between open and metadata defeats the check. | completed |
| F146 | LOCK-01 review | `src/repair/lock.rs` tests | Lock files now persist, so tests leave `girpr-*.lock` files and a dangling symlink in the temp directory. | completed |
| F147 | LOCK-01 review | `src/repair/lock.rs` | The Unix-only lock cases never ran on this host; they need a Linux/macOS run. | blocked (needs a Linux/macOS host) |
| F148 | INI-01 review | `src/util/ini.rs` | The refusal says "does not exist" even when the path exists as a non-directory. | completed |
| F149 | INI-01 review | `tests/` | No CLI-level case for the game dir vanishing between `prepare_install` and the post-phase bump. | completed |
| F150 | INI-01 review | `src/util/ini.rs` | `split_first_eq` is a thin wrapper over `split_once`; keep it only while the shared rule earns its name. | completed |
| F151 | BUILD-01 review | `Cargo.toml` (members) | Only `girpr` carries `rust-version`; `tools/*` and `crates/*` have none, so the workspace MSRV error names redb. | completed |
| F152 | BUILD-01 review | `src/repair/scan.rs` | The MD5 benchmark covers whole-file hashing but not the dominant `scan_file`/`scan_reader` read path. | completed |
| F153 | BUILD-01 review | `src/sophon/stream.rs` | Two adjacent `use futures_util::...` lines could be one brace import. | completed |
| F154 | BUILD-TOOLS-01 review | `tools/capture/Cargo.toml` | The realized tokio saving (`parking_lot`, `process`, `signal`, `fs`, `io-std`) could be stated in the tools' build docs. | completed |
| F155 | BUILD-TOOLS-01 review | `tools/sync/src/hash/stream.rs` | The digest trait comes from `sha2::Digest`; importing `md5::Digest as _` would make the source explicit. | completed |
| F156 | TEST-HELPERS-01 review | `tests/common/mod.rs` | Including the whole fixture module in `artifact_server`/`peak_heap` slightly increases test build time. | completed |
| F157 | TEST-HELPERS-01 review | `tests/peak_heap.rs`, `tests/artifact_bridge.rs` | The explicit `#[path = "common/mod.rs"]` on `mod common;` is redundant. | completed |
| F158 | LINT-01 review | `src/lib.rs`, `clippy.toml` | A root `clippy.toml` with the two test options would let `src/lib.rs` drop its `cfg_attr(test)` allow; integration targets still need theirs. | completed |
| F159 | LINT-01 review | docs | The `cedb038` commit message still claims the clippy test options are wholly ignored; the README is now accurate. | dropped (history-only; README is now accurate and rewriting is forbidden) |
| F160 | LINT-01 review | `tools/sync/src/commands/sync/apply/phases.rs` | Non-test `expect`s remain in the tools, blocking any future `unwrap_used`/`expect_used` opt-in. | completed |
| F161 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` tests | The removed-seek test infers positioning from the cursor's initial value; a sink wrapper would pin it directly. | completed |
| F162 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | The seek-refund test drives the helper; no end-to-end `write_downloaded_chunk` case with an unseekable sink. | completed |
| F163 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | A copy-path seek failure drops the cached source handle instead of restoring it, unlike the other fallbacks. | completed |
| F164 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | The `drop(src_handle)` Windows share-mode rationale is defensive rather than required for std's default share mode. | completed |
| F165 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | Five tests still spell `.girpr_tmp` literally instead of deriving it from `temp_sibling`. | completed |
| F166 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | The fallback test could assert source-present/target-absent inside the injected copy, not only after. | completed |
| F167 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | `copy_publish_with`'s no-file-name refusal has no direct test. | completed |
| F168 | TEMP-PUBLISH-01 review | `src/repair/scope/report.rs` | The `#[cfg(unix)]` symlink test and Unix non-UTF-8 arm never ran on this host; they need a Unix run. | blocked (needs a Linux/macOS host) |
| F169 | INPLACE-01 review | `src/repair/file/inplace.rs`, `src/repair/scan.rs` | An intact scan verdict on a coarse-mtime filesystem can still be accepted after a same-tick rewrite, so the run reports the file skipped and leaves it unrepaired until a later run. | completed |
| F170 | INPLACE-01 review | `src/repair/file/inplace.rs` | The natural evasion window is narrower than the forced test suggests; the recorded mtime must share the coarse tick with the post-scan rewrite. | completed |
| F171 | INPLACE-01 review | `src/repair/file/inplace.rs` | `trim_is_free` uses `is_none_or`, so an overflowing `r + s` would read as inside the target if the offset type ever widens. | completed |
| F172 | INPLACE-01 review | `src/repair/file/inplace.rs` | `reusable_verdict` stats the path instead of the open handle, so a replacement between open and stat is not detected. | completed |
| F173 | CHECK-01 review | `src/repair/phases/check.rs` | The live-progress test observes the fold by a bounded `yield_now` spin; a oneshot/watch would remove the timing assumption. | completed |
| F174 | CHECK-01 review | `src/report/summary.rs` | `check_damaged_chunks` is 0 both for "no damage" and "never measured"; an `Option` accessor would disambiguate. | completed |
| F175 | CHECK-01 review | `src/repair/phases/check.rs` | After a panic, in-flight blocking checks keep reading until they finish, lengthening the post-failure tail. | completed |
| F176 | CHECK-01 review | `src/repair/mod.rs` | The `run_check_only` comment about a dropped clone predates the panic path and is imprecise. | completed |
| F177 | REPORT-API-01 review | `src/hyp.rs`, `src/repair/phases/metadata.rs` | `GameConfig::game`/`ChunkBuild::tag` are unread; validate them against the requested biz/tag in a hardening batch. | completed |
| F178 | REPORT-API-01 review | `src/hyp.rs` | `ChunkBuild::tag`'s comment says the run compares the build id, but it only reports it; the `ManifestFile` size doc also overstates. | completed |
| F179 | REPORT-API-01 review | `README.md` | The module map still names `src/report.rs`, `emit_progress` and `spawn_progress_reporter`. | completed |
| F180 | PHASE-01 review | `src/phase.rs` | The no-double-count doc omits that the debug per-file `duration_ms` line nests inside `repair`. | completed |
| F181 | PHASE-01 review | `src/phase.rs` | The `plan` interval also contains the purge-plan walk, and the `REPORT` emission is an unnamed untimed gap. | completed |
| F182 | TEST-MANIFEST-01 review | `tests/repair_offline.rs` | The multi-manifest binary test does not cover the `ignore` set (`res_category_dir`). | completed |
| F183 | TEST-MANIFEST-01 review | `tests/common/mod.rs` | `ExtraManifestSpec::new` ties `category_id` to `matching_field`, so an ignored-by-category fixture cannot be expressed. | completed |
| F184 | TEST-MANIFEST-01 review | `tests/common/mod.rs` | Extra manifests' chunks are registered even when `served = false`; a future "absent manifest" case needs that adjusted. | completed |
| F185 | ATOMIC-01 review | `src/report/emit.rs` | The rustdoc links the private `write_line`, producing the sole `cargo doc` warning. | completed |
| F186 | ATOMIC-01 review | `src/repair/phases/post.rs` | The same private `temp_sibling` doc link pattern; silent now because private docs are not built. | completed |
| F187 | ATOMIC-01 review | `src/util/path.rs` | `temp_sibling` has no direct unit test pinning the `file_name() == None` fallback name. | completed |
| F188 | PLANID-01 review | `src/repair/phases/plan.rs` | `RepairPlan::latest` stays public and mutable; a read-only accessor would prevent accidental fingerprint invalidation. | completed |
| F189 | PLANID-01 review | `src/repair/scope/report.rs` | `PlanDocument::new`'s `expect` on the disk table is an invariant; a fallible signature or `debug_assert` would protect future callers. | completed |
| F190 | PLANID-01 review | `src/repair/scan.rs` | `matches_plan` relies on a 64-bit non-cryptographic hash; revisit if a scan report ever crosses a process boundary. | completed |
| F191 | PLANID-01 review | tests | A few tests use the crate-private `RepairPlan::files()` where `iter()` would express intent better. | completed |
| F192 | EXIT-01 review | `src/main.rs` | `parse_args`'s doc says it maps failures to `RunExit::Usage`, but the body still exits with literal `0`/`1`. | completed |
| F193 | EXIT-01 review | `src/main.rs` | `exit_code` takes `i32`, forcing callers to call `.code()` first; it could take `RunExit`. | completed |
| F194 | EXIT-01 review | `src/config.rs` | `RunExit`'s doc claims one integer conversion at the boundary, but the tracing line also calls `.code()`. | completed |
| F195 | EXIT-01 review | tests | A few test names and messages still mention exit numbers rather than variant names. | completed |
| F196 | DRYRUN-01 review | `tests/repair_purge.rs`, `tests/repair_limits.rs` | Dry-run dedup and the dry post-ceiling have only unit coverage; end-to-end assertions would pin the log/`SUMMARY` contract. | completed |
| F197 | DRYRUN-01 review | `src/repair/phases/post.rs` | When every after candidate was already listed, the after pass's "bytes would be freed" line is skipped entirely. | completed |
| F198 | DRYRUN-01 review | `src/repair/mod.rs` | `wants_deprecated`'s `(ctx.dry_run \|\| !readonly)` term is redundant now. | completed |
| F199 | PURGE-PERF-01 review | `src/repair/purge.rs`, `src/repair/phases/post.rs` | `deleted_bytes`/`would_be` use `+`; `saturating_add` would make the panic branches provably unreachable. | completed |
| F200 | PURGE-PERF-01 review | `src/repair/phases/post.rs` | `spawn_delete_deprecated` clones the whole deprecated `Vec<String>`; an `Arc<[String]>` would avoid it. | completed |
| F201 | PURGE-PERF-01 review | `src/repair/phases/post.rs` | `move_audio_cache` still uses raw `spawn_blocking`, losing its log lines under a scoped subscriber. | completed |
| F202 | PURGE-PERF-01 review | `src/repair/purge.rs` | `mark_kept_ancestors`'s per-ancestor `PathBuf` could be avoided with an index set, at the cost of coupling to walk order. | completed |
| F203 | MD5-TYPE-01 review | `src/util/mod.rs`, `README.md` | Docs still say `util::md5_file` resolves at the module root and name it in the repair path, but only `util::md5::md5_file` exists now. | completed |
| F204 | PATHHELPERS-01 review | `src/repair/phases/post.rs` | `move_audio_cache_with` now strips the cache prefix twice; a helper returning both key and destination path would avoid it. | completed |
| F205 | PATHHELPERS-01 review | `src/repair/phases/post.rs` | The dry-run log for nested cache paths now prints the `/` rel instead of the platform spelling. | completed |
| F206 | PATHHELPERS-01 review | `tests/repair_post.rs` | No end-to-end test moves a non-UTF-8-named audio cache file and asserts the destination name bytes. | completed |
| F207 | REFACTOR-01 review | `src/repair/mod.rs` | `estimate_download` both computes and mutates `summary` flags; separating the flag writes would make it read as pure. | completed |
| F208 | EXIT-02 review | `src/main.rs` | `clamp_code` is now unreachable via `exit_code`; remove it or document it as defense against a future discriminant change. | completed |
| F209 | DRYRUN-02 review | `src/repair/purge.rs` | `apply_purge` still emits `0 bytes would be freed` for a dry pass with no candidates; now documented, suppression would be a behavior change. | completed |
| F210 | DRYRUN-02 review | `tests/repair_limits.rs` | The ceiling test cannot independently distinguish the up-front check from the post-repair re-check. | completed |
| F211 | DRYRUN-02 review | `src/repair/phases/post.rs` | A real `--purge-after --plan-out <inside install>` deletes the plan document it just wrote; document or exclude it from the keep set. | completed |
| F212 | PURGE-PERF-02 review | `src/report/summary.rs` | `deleted_extra_bytes.fetch_add` still uses plain atomic addition; a saturating variant would remove the last wrap path. | completed |
| F213 | PURGE-PERF-02 review | `src/hyp.rs` | `fetch_deprecated` still returns `Vec<String>`; returning `Arc<[String]>` would remove the one remaining conversion copy. | completed |
| F214 | TEST-REGRESSION-01 review | `tests/repair_machinery.rs` | The fresh-scan test relies on a same-size rewrite changing mtime; coarse filesystems could make it flake. | completed |
| F215 | TEST-REGRESSION-01 review | `tests/process.rs` | The unscanned-run assertion is appended to the duration-field test; its own test would keep that one focused. | completed |
| F216 | TEST-OUTPUT-01 review | `tests/cli_contract.rs` | The JSON-plane tests still use only `assert_golden`; reuse `assert_one_terminal_summary` there too. | skipped (golden already pins it) |
| F217 | TEST-OUTPUT-01 review | `tests/cli_contract.rs`, `src/report/record.rs` | `text_records` splits on whitespace, so a future value containing a space would break parsing. | completed |
| F218 | TEST-OUTPUT-01 review | `tests/cli_contract.rs` | Text-plane coverage misses `--check-only` and `--plan-out`, which stay JSON-only at integration level. | completed |
| F219 | INPLACE-02 review | `tests/repair_machinery.rs` | The fresh-scan fixture's mtime is fresh, so the new age rule forces the re-read; the mtime-differs branch is no longer exercised. | completed |
| F220 | INPLACE-02 review | `src/repair/file/inplace.rs` | The residual-race doc names only a store that never advances mtime; coarser-than-2 s stores and backdating writers also evade it. | completed |
| F221 | INPLACE-02 review | `src/repair/file/inplace.rs` | The age rule assumes a forward-moving wall clock; a backwards jump is unhandled and worth a note. | completed |
| F222 | CHECK-02 review | `src/repair/phases/check.rs` | The live-progress test blocks indefinitely if a fold is lost; a bounded `recv_timeout` would fail clearly. | completed |
| F223 | CHECK-02 review | `src/report/summary.rs` | The raw `check_damaged_chunks` field is still `pub`; the accessor doc discourages direct use, but the field could be private. | completed |
| F224 | LOCK-02 review | `tests/repair_offline.rs`, CLI tests | Integration runs still persist `girpr-*.lock` files (123 per full suite) in the temp dir; needs test hygiene. | completed |
| F225 | LOCK-02 review | `src/repair/lock.rs` | An `nlink == 0` file reports "has 0 names (hard links)"; "has no remaining names" would read better. | completed |
| F226 | LOCK-02 review | `tools/sync/Cargo.toml` | `redb 4.3.0` requires Rust 1.90, so `cargo check --workspace` fails on the declared MSRV 1.89. | completed |
| F227 | BUILD-02 review | `tests/scripts/io-profile/iot-bench` | Still pins `redb = "=4.3.0"` and claims it matches the workspace; the comment is false and it will not build on 1.89. | completed |
| F228 | BUILD-02 review | `src/util/md5.rs` | `scan_throughput` prints `dest_ok`/`source_ok` but never asserts them, so a scan regression still prints a plausible number. | completed |
| F229 | BUILD-02 review | repository | The declared per-member MSRVs are unenforced; no CI job builds a member with its `rust-version`. | completed |
| F230 | BUILD-TOOLS-02 review | `tools/capture/Cargo.toml` | `tokio` is requested without `default-features = false`, so its empty `default` feature is enabled. | completed |
| F231 | BUILD-TOOLS-02 review | `tools/capture/Cargo.toml` | The manifest comment says nothing uses `net`/`sync`, while reqwest brings them in; it could mirror the docs' precision. | completed |
| F232 | TEST-HELPERS-02 review | `tests/common/digest.rs` | The module doc says "shared by the test fixtures", but fixture-free targets include it directly. | completed |
| F233 | INI-02 review | `src/util/ini.rs` | `game_dir_refusal` uses `Path::exists()`, which follows symlinks, so a dangling symlink reads as "does not exist". | completed |
| F234 | INI-02 review | `src/util/ini.rs` | The ENOTDIR branch treats any non-`NotFound` error under a non-directory path as "is not a directory", possibly masking parent permission errors. | completed |
| F235 | LOGGING-02 review | `src/logging.rs` | `FILE_SINKS` could be `Mutex<Option<FileSink>>` so the one-sink invariant lives in the type. | completed |
| F236 | LOGGING-02 review | `src/logging.rs` | No test covers the rejected-install path (a second `init_tracing` does not register a handle). | completed |
| F237 | LOGGING-02 review | `src/logging.rs` | `flush_log`'s doc still describes visiting "more than one sink", which it can no longer reach. | completed |
| F238 | JOBS-02 review | `tests/cli_contract.rs` | The help test searches all of stdout for `[default: 4]`; scoping it to the `--jobs` block would avoid a false pass. | completed |
| F239 | JOBS-02 review | `src/cli.rs` | The `[default: 4]` help line is hand-maintained apart from `DEFAULT_JOBS`. | completed |
| F240 | JOBS-02 review | `tests/cli_contract.rs` | The binary-level boundary pair is alias-only; a `--jobs` pair would mirror it at process level. | completed |
| F241 | MANIFEST-02 review | `src/repair/phases/manifest.rs` | The out-of-order stream test asserts only `is_err()` for the first result; a message check would pin its identity. | completed |
| F242 | MANIFEST-02 review | `src/repair/phases/manifest.rs` | Live HTTP-stream construction and `manifest_concurrency` bounding are not exercised by the `from_stream` seam. | completed |
| F243 | MANIFEST-02 review | `src/repair/phases/manifest.rs` | Finished connection tasks are reaped only on a new accept, so entries can briefly accumulate. | completed |
| F244 | METADATA-02 review | `src/repair/phases/metadata.rs` | `LocalRead::ALL` is a fixed four-element array, so a new variant would not break the "covers every local read" test. | completed |
| F245 | METADATA-02 review | `tests/common/mod.rs` | `config_after_branches` relies on one `Notify` permit per branch response; a retried config call could hang. | completed |
| F246 | METADATA-02 review | `tests/repair_offline.rs` | The non-chunk test's branch failure is retryable `500`; a terminal branch failure under a refused mode is not covered. | completed |
| F247 | TEMP-WRITER-02 review | `src/repair/file/temp.rs` | An end-to-end unseekable-sink test would need `DownloadedChunk`/`ChunkFileSink` generic over `W: Write + Seek`. | completed |
| F248 | TEMP-WRITER-02 review | `src/repair/file/temp.rs` | `SeekRecording` guards only the missing-source branch; the no-candidate and digest-mismatch fallbacks could be pinned too. | completed |
| F249 | TEMP-PUBLISH-02 implementer | `src/util/path.rs` | `temp_sibling` on a path with no file name returns one still containing a trailing `..` before resolution; the doc could say so. | completed |
| F250 | TEMP-PUBLISH-02 implementer | `src/repair/phases/post.rs` | The no-file-name guard is unreachable through `move_audio_cache_with`; the direct unit test is its only coverage. | completed |
| F251 | SYNC-01 review | `tools/sync/src/commands/sync/plan/pairs.rs` | `shared_pairs` has no non-ASCII unit case of its own; a planner regression is only caught indirectly by the e2e test. | completed |
| F252 | SYNC-01 review | `tools/sync/src/hash/set.rs` | The hash test targets an unknown name; no non-ASCII lookalike reaches an accepted algorithm today. Revisit if a name containing `k` is added. | completed |
| F253 | HYP-02 review | `tests/common/mod.rs` | The local build fixture tag is hardcoded as `9.9.8-test`; deriving it from the requested local version would keep reuse tests honest. | completed |
| F254 | HYP-02 review | `src/repair/phases/metadata.rs` | The local-build mismatch warning wording is not pinned by a test; only the resulting `local_build = None` is. | completed |
| F255 | HYP-02 review | `src/repair/phases/metadata.rs` | Biz mismatch is checked before the download-mode refusal, so a wrong-biz non-chunk response reports metadata; precedence is undocumented. | completed |
| F256 | PATHHELPERS-02 review | `src/util/path.rs` tests | No explicit prefix-but-not-component case (`GameX/a.pck` against `Game`) pins the component-boundary contract. | completed |
| F257 | PATHHELPERS-02 review | `tests/repair_post.rs` | The non-UTF-8 move test goes through in-process `girpr::run`; a process-spawning variant would also cover CLI-layer rewriting. | completed |
| F258 | TEST-MANIFEST-02 review | `tests/common/mod.rs` | The unserved-manifest test uses distinct chunk ids, so it cannot catch an unserved manifest overwriting a served manifest's chunk with a shared id. | completed |
| F259 | TEST-MANIFEST-02 review | `tests/repair_offline.rs` | The "both manifests are still advertised" assertion reads the fixture list, not the mock's `getBuild` JSON. | completed |
| F260 | LINT-02 review | `tools/capture/src/select/mod.rs` | `enumerate()` over `REQUIRED_ORDER` assumes its index equals the `ALL` index; a reorder would silently pair classes with the wrong candidates. | completed |
| F261 | LINT-02 review | `tools/capture/src/http/log.rs` | `to_ndjson`'s doc describes only the happy path; the `io::Result` error case is unmentioned. | completed |
| F262 | DOCS-USER-01 review | `README.md` | Per-file counts ("74 cases", "25 cases") drift like the removed total; soften them. | completed |
| F263 | DOCS-USER-01 review | `src/report/emit.rs`, `src/main.rs` | `summary_record` ("ending every run") and the `main` comment ("every return path ends with exactly one") overstate the fatal path. | completed |
| F264 | DOCS-USER-01 review | `tests/artifact_bridge.rs` | `base_args` uses the deprecated `--io-threads` spelling; switch to `--jobs` unless intentional alias coverage. | completed |
| F265 | DOCS-COMMENTS-01 review | `README.md` | The module map still lists `src/sophon.rs` although the code is `src/sophon/`. | completed |
| F266 | DOCS-COMMENTS-01 review | `src/repair/scope/filter.rs`, `docs/04-scope-limitation-spec.md` | `util::contained_rel` is named, but it is only `util::path::contained_rel`. | completed |
| F267 | DOCS-COMMENTS-01 review | `docs/07-panic-audit.md` | Table line ranges are stale after the module splits. | completed |
| F268 | DOCS-COMMENTS-01 review | `README.md` | `tests/repair_offline.rs` is described as "one `#[tokio::test]` per behavior"; it also has a plain `#[test]`. | skipped (already accurate) |
| F269 | REFACTOR-02 review | `src/repair/mod.rs` | `DownloadEstimate` mixes byte figures with four summary-shaped values; a `PlanSummaryValues` split would keep it about bytes. | completed |
| F270 | REFACTOR-02 review | `src/repair/mod.rs` | `reported_bytes`'s doc names `--max-download-bytes`, while the guard is `ctx.limits.download_bytes`; another limit source would stale it. | completed |
| F271 | EXIT-03 review | `src/config.rs` | `RunExit` uses implicit discriminants; `#[repr(u8)]` would make the boundary conversion infallible. | completed |
| F272 | EXIT-03 review | `src/config.rs`, `src/main.rs` | The six exit variants are listed in three separate tests; a shared exhaustive source would prevent drift. | completed |
| F273 | EXIT-03 review | `src/main.rs` | `parse_args` bypasses `exit_code` and exits with the raw `i32`; it would not fail loudly if `RunExit` outgrew `u8`. | completed |
| F274 | EXIT-03 review | `src/config.rs` | The `u8` range guarantee is runtime-only; a `const` assertion over the discriminants would catch a bad change at compile time. | completed |
| F275 | DRYRUN-03 review | `tests/repair_purge.rs` | Add a companion assertion that an in-install `--plan-out` survives an exclusive `--purge-before` run. | completed |
| F276 | DRYRUN-03 review | `src/repair/mod.rs`, `src/repair/phases/post.rs` | The `--purge-before`/`--purge-after` plan-out asymmetry is now documented; revisit whether `--purge-before` should plan after publishing the document. | completed |
| F277 | PURGE-PERF-03 review | `src/repair/scope/ledger.rs`, `src/report/summary.rs` | The saturating CAS idiom is duplicated; a shared helper would remove it. | completed |
| F278 | PURGE-PERF-03 review | `src/repair/mod.rs` | `fetch_deprecated`'s collect still pays the `Vec + Arc` double allocation; an exact-size buffer would avoid it. | completed |
| F279 | TEST-REGRESSION-02 review | `tests/repair_machinery.rs` | The aged/fresh stamps are absolute epochs; on a host clock before 2023 the age-eligibility would silently not hold. | completed |
| F280 | TEST-REGRESSION-02 review | `tests/process.rs` | The new unscanned-phase test duplicates the fixture-write setup of the duration test; a shared local helper would reduce drift. | completed |
| F281 | TEST-OUTPUT-02 review | `tests/cli_contract.rs` | `text_records` treats any key-shaped token inside a value as a new field, so a value containing `word=…` would misparse. | completed |
| F282 | TEST-OUTPUT-02 review | `tests/cli_contract.rs` | `text_records` drops values, so the space test can only assert keys; a value-bearing parser would allow asserting the reconstructed `exe` value. | completed |
| F283 | TEST-OUTPUT-02 review | `tests/cli_contract.rs` | The still-rejected "token before the first field" path has no explicit negative test. | completed |
| F284 | TEST-OUTPUT-02 review | `tests/cli_contract.rs` | The text-plane plan-out test checks `repair.len() == 2` but not that the entries are the fixture's two rels. | completed |
| F285 | INPLACE-03 review | `tests/repair_machinery.rs` | The new age-branch test derives its stamp as `SystemTime::now() + 1h`; a fixed far-future constant would match the file's style and remove wall-clock dependence. | completed |
| F286 | CHECK-03 review | `src/repair/phases/check.rs` | The live-progress test's gate wait (`while !(slow_started && fast_done)`) is still an unbounded `Condvar` wait. | completed |
| F287 | CHECK-03 review | `src/report/summary.rs` | The now-`pub(super)` counter's doc could note that writes go through `publish_check_damaged_chunks`. | completed |
| F288 | PATHHELPERS-03 review | `tests/repair_post.rs` | The binary test's doc implies the name can be rewritten while building the context, but it is only discovered in the post-phase walk; the added value is CLI/RunCtx wiring. | completed |
| F289 | PATHHELPERS-03 review | `tests/repair_post.rs` | The skip-on-write-failure pattern reports a pass without exercising assertions on hosts that reject such names; consider an opt-in strict mode or CI note. | completed |
| F290 | TEST-MANIFEST-03 review | `tests/repair_offline.rs` | The `getBuild` expected ids are fixture-derived; fetching the advertised bodies (`200` for `kept`, `404` for `gone`) would tie advertisement to servability. | completed |
| F291 | TEST-MANIFEST-03 review | `tests/repair_offline.rs` | `gone_id`/`kept_id` are taken by positional index; lookup by `matching_field` would be less brittle. | completed |
| F292 | TEMP-WRITER-03 review | `src/repair/file/temp.rs` tests | The unseekable test cannot distinguish "reserved then refunded" from "never reserved" (`live() == 0` both ways); a ledger spy would pin reserve-before-seek. | completed |
| F293 | TEMP-WRITER-03 review | `src/repair/file/temp.rs` | `ChunkFileSink::new`'s doc still says it holds "the only `&mut File`" though it is now generic over `W`. | completed |
| F294 | TEMP-WRITER-03 review | `src/repair/file/temp.rs` tests | `SeekRecording.writes` counts bytes, not write calls; `bytes` would read better. | completed |
| F295 | TEMP-PUBLISH-03 review | `src/util/path.rs` tests | If the no-file-name `temp_sibling` fallback should ever be rejected, the helper doc and both shape assertions need updating together. | skipped (already covered by 40b23a5, 1e96ab6, 86c6fc1, 02b346d; no change needed) |
| F296 | TEMP-PUBLISH-03 review | `src/repair/phases/post.rs` | The no-file-name guard is production-reachable only if the walk later yields symlinks/directories; add a production-path test then. | skipped (already covered by 40b23a5, 1e96ab6, 86c6fc1, 02b346d; no change needed) |
| F297 | DOCS-USER-02 review | `tests/artifact_bridge.rs` | The six bridge tests are `#[ignore]`d and were not run; run them manually with `--run-ignored only` before a repair-spec release. | completed |
| F298 | DOCS-USER-02 review | `tests/process.rs`, `tests/repair_*.rs` | Many integration tests still pass `--io-threads` where `--jobs` would do; a cleanup could modernize them while keeping one alias case. | completed |
| F299 | DOCS-COMMENTS-02 review | `docs/07-panic-audit.md` | The audit predates some local-invariant `expect` sites, e.g. `PlanTotals::new` in `src/repair/scope/report.rs`. | completed |
| F300 | DOCS-COMMENTS-02 review | `docs/07-panic-audit.md` | The method section's file list omits `src/repair/audio.rs` and `src/repair/lock.rs`. | completed |
| F301 | EXIT-04 review | `src/config.rs` | `RunExit::ALL`'s completeness is not compile-enforced; a macro defining enum, discriminants and `ALL` from one list would be airtight. | completed |
| F302 | EXIT-04 review | `src/main.rs` | `every_documented_exit_code_fits_in_a_process_status` is now near-tautological; drop it or reduce it to a note. | completed |
| F303 | EXIT-04 review | `src/config.rs` | `RunExit::ALL` is new public API surface; `pub(crate)` would keep the published surface smaller if only tests use it. | completed |
| F304 | DRYRUN-04 review | `tests/repair_purge.rs` | The purge-before plan-out test only asserts the document is non-empty; parsing it and checking it describes the same repair plan would be stronger. | todo |
| F305 | PURGE-PERF-04 review | `src/repair/mod.rs` | `fetch_deprecated`'s normalization/drop-with-warning paths have no focused test; a backslash or absolute deprecated name would pin the `retain_mut` semantics. | completed |
| F306 | PURGE-PERF-04 review | `src/repair/mod.rs` | The "one Arc allocation" claim relies on std's in-place collect specialization; confirm with an allocator counter if it matters. | completed |
| F307 | PURGE-PERF-04 review | `src/util/counter.rs` | The shared test pause barrier is thread-local and process-wide; a test pausing then hitting a different counter on the same thread would pause unexpectedly. | completed |
| F308 | HYP-03 review | `tests/*.rs` | Several test files still hardcode `9.9.9-test`/`1.0.0-long-gone` rather than `LATEST_TAG`/a constant; a test-hygiene sweep would reduce drift. | completed |
| F309 | HYP-03 review | `src/repair/phases/metadata.rs` tests | `logs_of` captures only the current thread; if `fetch` ever emits from a `spawn_blocking` worker, the helper would silently miss it. | completed |
| F310 | METADATA-03 review | `tests/common/mod.rs` | The mock can only fail a branch call with a non-JSON body; a JSON `retcode` failure mode is needed to test retryable-vs-terminal branch behavior. | completed |
| F311 | METADATA-03 review | `tests/common/mod.rs` | `branch_failure_status` is behaviourally inert for retry classification; fold it into a single failure-body knob. | completed |
| F312 | MANIFEST-03 review | `src/repair/phases/manifest.rs` tests | `wait_until_idle`/`wait_for` use `notify_waiters()` with a pre-check, which could miss a wakeup on a multi-thread runtime; `Notified::enable()` would harden it. | completed |
| F313 | MANIFEST-03 review | `src/repair/phases/manifest.rs` tests | The bound table hardcodes `(1,1)`/`(2,2)` instead of calling `manifest_concurrency(jobs)`; compute it to stay in lockstep. | completed |
| F314 | MANIFEST-03 review | `src/repair/phases/manifest.rs` tests | A gated connection aborted mid-wait skips `gate.leave()`, so `current` would not return to zero if a future test waited for idle after dropping a gated server. | completed |
| F315 | JOBS-03 review | `tests/cli_contract.rs` | `help_entry` matches a flag by prefix, so `--jobs` would also match a future `--jobs-*`; require a following space/newline. | completed |
| F316 | JOBS-03 review | `tests/cli_contract.rs` | The synthetic slicing test only covers the first-entry branch; add a case with a preceding option to cover the `\n`-marker branch. | completed |
| F317 | JOBS-03 review | `tests/cli_contract.rs` | `help_entry` hardcodes clap's six-space indent; a comment or derived-indent check would document that coupling. | completed |
| F318 | LOGGING-03 review | `src/logging.rs` | `FILE_SINKS` is plural but now holds a single `Option`; rename to `FILE_SINK`. | completed |
| F319 | LOGGING-03 review | `src/logging.rs` tests | The rejected-install test writes into the sink directly and does not assert the install-warning text; emitting through the subscriber and pinning the WARN would be stronger. | completed |
| F320 | LOCK-03 review | `tests/artifact_bridge.rs` | The bridge test duplicates `remove_run_lock` locally; consolidating with `common::remove_run_lock_for_args` would remove the last copy. | completed |
| F321 | LOCK-03 review | `tests/common/mod.rs` | `remove_run_lock` acquires-and-deletes even for read-only runs that never took a lock, briefly creating a file; skip when the mode cannot lock. | completed |
| F322 | LOCK-03 review | `tests/common/mod.rs` | The vanished-install fallback assumes the caller's parent is canonical; a junction/relative-spelled install would diverge and leak the real file. | completed |
| F323 | INI-03 review | `src/util/ini.rs` | In the `NotFound` arm, an inspection-side error is discarded and the original `NotFound` surfaced; a cleanup could surface the inspection error instead. | completed |
| F324 | INI-03 review | `src/util/ini.rs` | Windows junctions/reparse points are not `is_symlink()`, so a dangling junction reads as `is not a directory`. | completed |
| F325 | INI-03 review | `src/util/ini.rs` tests | `try_symlink_dir` treats every creation error as "cannot test"; tightening it to the known privilege error would avoid a false skip. | completed |
| F326 | BUILD-03 review | `.github/workflows/msrv.yml` | The workflow triggers on every push/PR with no path filter, caching, concurrency group, or `permissions`, installing up to three toolchains each run. | completed |
| F327 | BUILD-03 review | `tests/scripts/check-msrv.ps1` | The script checks default targets only; `--all-targets` would guard test-only code against raising a member's floor. | completed |
| F328 | BUILD-03 review | `tests/scripts/io-profile/iot-bench` | The standalone bench is outside the workspace and no gate compiles it, so its pin could drift again; a CI `cargo check --locked` would catch it. | completed |
| F329 | BUILD-TOOLS-03 review | `Cargo.toml` | The root `girpr` manifest also requests `tokio` without `default-features = false` (same empty `default`); align for consistency. | completed |
| F330 | BUILD-TOOLS-03 review | `tools/capture/Cargo.toml` | "This list is the whole request" is true of the manifest, but reqwest-tree crates still activate tokio's empty `default` in the resolved graph; clarify if it misleads. | completed |
| F331 | EXIT-05 review | `src/main.rs` | The binary's hand-spelled six-status list must be updated manually if a seventh `RunExit` variant is added. | completed |
| F332 | PURGE-PERF-05 review | `src/util/counter.rs` tests | Add a focused regression test that pauses one cell and charges a different cell on the same thread, pinning the scoping directly. | completed |
| F333 | PURGE-PERF-05 review | `src/repair/mod.rs` tests | The normalization test calls the helper directly; an end-to-end `fetch_deprecated` test over a stub client would also pin that the fetch path invokes it. | completed |
| F334 | BUILD-04 review | `.github/workflows/msrv.yml` | The `iot-bench` job uses default stable, so a 1.89-specific bench regression would go unnoticed; pinning 1.89 would close that gap. | completed |
| F335 | BUILD-04 review | `tests/scripts/io-profile/README.md` | "buildable with the workspace toolchain" is loose given the job uses default stable; say "current stable" explicitly. | completed |
| F336 | BUILD-04 review | `.github/workflows/msrv.yml` | The path filter means docs/config-only PRs produce no `msrv` check; confirm branch protection compatibility or add an always-run companion. | completed |
| F337 | BUILD-TOOLS-04 review | `Cargo.toml` | The root `[dev-dependencies]` `tokio` entry also omits `default-features = false`; align for symmetry with the normal dependency. | completed |
| F338 | BUILD-TOOLS-04 review | `tools/capture/Cargo.toml` | The comment says nothing uses `tokio::fs`, but reqwest's `stream` feature compiles it; a tweak could avoid the same source-vs-graph misreading. | completed |
| F339 | BUILD-TOOLS-04 review | `Cargo.toml` | The root `tokio` comment could also state that the reqwest tree still enables the empty `default` in the resolved graph. | completed |
| F340 | LOCK-04 review | `tests/common/run_lock.rs` | `mode_takes_lock`/`args_take_lock` lack direct tests; nothing asserts the mirror accepts `--purge-only` and rejects the three read-only flags. | completed |
| F341 | LOCK-04 review | `tests/common/run_lock.rs` | `#![allow(dead_code)]` is module-wide; a targeted allowance would keep unexpected dead code visible. | completed |
| F342 | LOCK-04 review | `tests/common/run_lock.rs` | `lock_key_path` retains the caller's spelling for vanished components; harmless on case-folding Windows, risky on a case-sensitive host. | completed |
| F343 | LOCK-04 review | `tests/common/run_lock.rs` | A mutating run that fails before execution still never took the lock, yet cleanup acquires-and-deletes the path, briefly creating a file. | completed |
| F344 | LOCK-04 review | `tests/common/run_lock.rs` | `game_path_arg` only recognizes `--game-path <value>`; an `=`-form caller would silently skip cleanup. | completed |
| F345 | INI-04 review | `src/util/ini.rs` | No call-site regression test asserts the `inspect game dir <path>` context/kind; a seam would make that combination testable. | completed |
| F346 | INI-04 review | `src/util/ini.rs` tests | The junction classification is documented from std behavior but not exercised; a Windows-only `mklink /J` test could pin the dangling-junction message. | completed |
| F347 | INI-04 review | `src/util/ini.rs` tests | On non-Unix/non-Windows targets `try_symlink_dir` now panics rather than skipping; consider skipping on `Unsupported`. | completed |
| F348 | INI-04 review | `src/util/ini.rs` | The `Err` doc says "the caller must surface it", but the `NotADirectory` caller keeps its original error; clarify. | completed |
| F349 | METADATA-04 review | `tests/common/mod.rs` | `BranchFailure::Terminal { retcode }` does not validate that `retcode` is terminal; a debug assertion or restricted constructor would self-check the knob. | completed |
| F350 | METADATA-04 review | `tests/repair_offline.rs` | `a_terminal_branch_failure_is_metadata_under_chunk_mode` now exercises the non-JSON fatal-parse body; its name/comment could be aligned. | completed |
| F351 | MANIFEST-04 review | `tests/repair_limits.rs` | `a_404_manifest_fails_the_run_without_waiting_for_it` saw one transient ~5 s run under load; a stability pass is warranted if it recurs. | skipped (no change needed: no avoidable wait; no second gated-origin consumer) |
| F352 | MANIFEST-04 review | `src/repair/phases/manifest.rs` tests | The `InFlight`/`TestServer` coordination helpers could be shared with other phase test modules if similar gated-origin patterns appear. | skipped (no change needed: no avoidable wait; no second gated-origin consumer) |
| F353 | TEST-REGRESSION-03 review | `src/repair/file/inplace.rs` tests | Unit tests still use absolute `UNIX_EPOCH + 1_700_000_000` stamps and share the pre-2023 host-clock sensitivity. | completed |
| F354 | TEST-REGRESSION-03 review | `tests/process.rs` | Other process tests still repeat the standard-install setup; adopting `start_standard_install` would reduce duplication further. | completed |
| F355 | TEST-REGRESSION-03 review | `tests/process.rs` | `the_publish_and_pre_purge_steps_are_timed` still open-codes the mock + temp dir + fixture write and could use `start_standard_install`. | completed |
| F356 | TEST-REGRESSION-03 review | `tests/repair_machinery.rs` | `stamp_ago` panics if the host clock is within 48 h of the platform minimum representable `SystemTime`; clamping or a documented floor would harden it. | completed |
| F357 | TEST-OUTPUT-03 review | `tests/cli_contract.rs` | The bare text grammar is ambiguous for a value containing a known key token followed by `=`; production has no escaping to resolve it. | completed |
| F358 | TEST-OUTPUT-03 review | `tests/cli_contract.rs` | `scan_bytes` is a literal special case apart from `PLAN_KEYS`; a named constant would keep the vocabulary symmetric. | completed |
| F359 | TEST-OUTPUT-03 review | `tests/cli_contract.rs` | The test's event/key schema duplicates the production schema; a shared exported constant would avoid drift. | skipped (no production schema manifest exists; sharing would need a new public API) |
| F360 | INPLACE-04 review | `tests/repair_machinery.rs` | The future stamp uses unchecked `SystemTime + Duration`; a `stamp_ahead` helper mirroring `stamp_ago` would make the pair symmetric and the failure explicit. | completed |
| F361 | CHECK-04 review | `src/report/summary.rs` | "no store reaches this field without the flag" is true of production paths but a test stores directly; say "no production store" for strict precision. | completed |
| F362 | CHECK-04 review | `src/repair/phases/check.rs` | The test repeats `Duration::from_secs(10)` across the gate wait and `recv_timeout`; a module-level constant would keep them in step. | completed |
| F363 | CHECK-04 review | `src/repair/phases/check.rs` | The injected check closure still has an unbounded `while !state.release_slow { cv.wait(state) }` inside its spawned thread. | completed |
| F364 | PATHHELPERS-04 review | `tests/repair_post.rs`, config | nextest's default profile captures passing-test output, so the `SKIP precondition:` line is invisible under the plain gate; a per-test `success-output` override or a README `--no-capture` note would help. | completed |
| F365 | PATHHELPERS-04 review | `tests/repair_post.rs` | The skip branch is unexercised on both supported CI filesystems; an injected-write seam would cover both branches deterministically. | completed |
| F366 | PATHHELPERS-04 review | `README.md` | The switch is presence-based, not value-based, so `GIRPR_TEST_REQUIRE_NON_UTF8=0` still forces failure; clarify "any value". | completed |
| F367 | PATHHELPERS-04 review | `src/util/path.rs` | Pre-existing silent skips remain elsewhere (e.g. the junction test); the same explicit/opt-in treatment could apply. | completed |
| F368 | DOCS-USER-03 review | `tests/artifact_bridge.rs`, `docs/06-artifact-bridge-spec.md` | The cost rationale still says the suite is worth running "when something has gone wrong", understating the pre-release occasion. | completed |
| F369 | DOCS-USER-03 review | `tests/peak_heap.rs` | Line 12 still writes the flag as `--io-threads 8` in prose; `--jobs` would be consistent. | completed |
| F370 | DOCS-USER-03 review | `tests/artifact_bridge.rs` | The `gircap verify` precondition is skipped unless `gircap` is pre-built; a release run should build it so the precondition executes. | completed |
| F371 | DOCS-USER-03 review | `tests/cli_contract.rs` | Several process-level cases still pass `--io-threads` beyond the dedicated alias tests; incidental ones could become `--jobs`. | completed |
| F372 | DOCS-COMMENTS-03 review | `docs/07-panic-audit.md` | Method step 1 names `.unwrap()`/`.expect()`/`[]` but its updated command also greps `panic!`/`unreachable!` and slices; the prose should match. | completed |
| F373 | DOCS-COMMENTS-03 review | `docs/07-panic-audit.md` | The new `audio.rs`/`lock.rs` note continues the numbered-list sentence after its period and orders modules after `purge`; cosmetic. | completed |
| F374 | DOCS-COMMENTS-03 review | `docs/07-panic-audit.md` | The table now mixes panic sites with non-panicking clamps (`max_chunk_bytes`); a framing sentence would clarify. | completed |
| F375 | DOCS-COMMENTS-03 review | `docs/07-panic-audit.md` | The audit still refers to a "§6"/"§6.1" section that does not exist in this file. | completed |
| F376 | DOCS-COMMENTS-03 review | `src/repair/phases/manifest.rs` | `OrderedResults::push`'s `slots[index]` is a local-invariant direct index not named explicitly in the new row. | completed |
| F377 | HYP-04 review | `src/repair/phases/metadata.rs` | `logs_of`'s doc conflates thread-locality with parallelism; cite `set_default` being thread-local (and the current-thread runtime). | completed |
| F378 | HYP-04 review | `src/repair/mod.rs` | `spawn_blocking_traced`'s doc should note that dropping the handle detaches rather than cancels the task. | completed |
| F379 | HYP-04 review | `src/repair/mod.rs` | No test pins the eager-spawn property the refactor exists for; a narrow test would guard the overlap contract. | completed |
| F380 | HYP-04 review | `src/repair/phases/scan.rs`, `check.rs`, `mod.rs` | Other raw `spawn_blocking` sites do not propagate a scoped dispatcher; they would need `spawn_blocking_traced` if per-file log assertions are added. | completed |
| F381 | HYP-04 review | `tests/common/mod.rs` | `logs_of` and the metadata test's `logs_of` duplicate the same capture implementation; a shared test-support helper could remove it. | skipped (lib/integration boundary; needs a new public async helper) |
| F382 | TEMP-WRITER-04 review | `src/repair/file/temp.rs` tests | `RefusingSeek` names its `Arc<Summary>` `ledger` while the rest of the file says summary/download meter; rename for consistency. | completed |
| F383 | TEMP-WRITER-04 review | `src/repair/file/temp.rs` tests | `SeekRecording.bytes` is only asserted to be 0; a case that writes through the recorder would verify the byte count. | completed |
| F384 | TEMP-WRITER-04 review | `src/repair/file/temp.rs` tests | `RefusingSeek`'s `Option<Arc<Summary>>` + `Default` exist for one call site; a required parameter would simplify it. | completed |
| F385 | TEMP-WRITER-04 review | `src/repair/file/temp.rs` | `settle_seek_failure`'s doc justifies itself as a "test seam" in production code; reframe around the refund invariant. | completed |
| F386 | TEMP-WRITER-04 review | `src/repair/file/temp.rs` | `ChunkFileSink`'s doc cites `copy_within` as the reuse copy, but temp mode uses `try_reuse_slice`. | completed |
| F387 | TEMP-PUBLISH-04 review | `src/repair/scope/report.rs` | `atomic::publish` accepts no-file-name targets and relies on the rename failing; an early refusal would give a clearer error. | completed |
| F388 | TEMP-PUBLISH-04 review | `src/util/path.rs` tests | The `temp_sibling` test asserts the unresolved `Audio/res/../.girpr_tmp` spelling while the doc says it lands at `Audio/.girpr_tmp`; pin the resolved location. | completed |
| F389 | TEMP-PUBLISH-04 review | `src/repair/phases/post.rs` | If the audio-cache walk ever follows links or admits directories, the no-file-name guard becomes production-reachable and needs a production-path test. | completed |
| F390 | TEST-MANIFEST-04 review | `src/lib.rs`, `src/sophon/stream.rs` | `join_url` is private, so the test duplicates its join formula; exposing it through `test_support` would pin production's join and prevent drift. | completed |
| F391 | TEST-MANIFEST-04 review | `tests/repair_offline.rs` | Other tests still take extra-manifest ids positionally (`several_latest_manifests_are_selected_in_build_order` and two more). | completed |
| F392 | TEST-MANIFEST-04 review | `tests/repair_offline.rs` | Production's `join_url` ignores the advertised `url_suffix`, so a non-empty suffix remains untested; may be intentional. | skipped (unsupported/ambiguous suffix semantics; documented contract keeps prefix-only join) |
| F393 | TEST-MANIFEST-04 review | `tests/repair_offline.rs` | The new assertions check status only, not that the `kept` `200` body equals the fixture bytes; optional hardening. | completed |
| F394 | EXIT-06 review | `src/main.rs` tests | The macro's `fn(RunExit)` annotation and `RunExit::` paths hard-wire one type; parameterize the type path if a second enum needs the guard. | completed |
| F395 | EXIT-06 review | `src/config.rs`, `src/main.rs` tests | The status list now exists twice (`RunExit::ALL` and the bin-test macro); a public read-only accessor would remove it but widens API. | completed |
| F396 | PURGE-PERF-06 review | `src/util/counter.rs` tests | The cross-cell gate test would hang rather than fail on a broken gate; charging on a helper thread with a bounded `recv_timeout` would fail CI cleanly. | completed |
| F397 | PURGE-PERF-06 review | `src/util/counter.rs` tests | The resume test uses `Barrier::new(1)`, so the "pause" half is vacuous; a 2-party barrier released from a helper thread would pin suspension too. | completed |
| F398 | PURGE-PERF-06 review | `src/repair/mod.rs` tests | The new `serve_body` helper duplicates the loopback-stub idiom in `metadata`, `manifest`, and `hyp` tests; a shared `#[cfg(test)]` fixture would remove it. | completed |
| F399 | TEST-REGRESSION-04 review | `src/repair/file/inplace.rs` tests, `tests/repair_machinery.rs` | `stamp_ago` docs say it saturates at the Unix epoch, but a pre-epoch clock returns `now`; align doc and behavior or guard pre-epoch. | completed |
| F400 | TEST-REGRESSION-04 review | `src/repair/file/inplace.rs` tests | `coarse_tick_of` still `expect`s a post-epoch clock and does epoch math; it is the last epoch-dependent helper. | completed |
| F401 | TEST-OUTPUT-04 review | `tests/repair_scan.rs` | The emitted `scan_bytes=` value is still spelled literally; a shared spelling could be used if a test-support location exists. | todo |
| F402 | TEST-OUTPUT-04 review | `README.md` | The bare-text limitation is documented only beside the test parser; the `--output` guidance could point users to JSON when values may contain `key=` tokens. | todo |
| F403 | TEST-OUTPUT-04 review | `tests/cli_contract.rs` | If production later exposes a key/schema manifest, the golden key lists could be derived from it instead of duplicated. | todo |
| F404 | CHECK-05 review | `src/repair/phases/check.rs` tests | On expiry the injected closure returns `CheckOutcome::clean()` so counters still read 2; a distinct outcome or comment would read clearer. | todo |
| F405 | CHECK-05 review | `src/repair/phases/check.rs` tests | Three waits each get a full `GATE_WAIT` from their own start; one absolute test deadline shared by all would make failure ordering unambiguous. | todo |
| F406 | CHECK-05 review | `src/repair/phases/check.rs` tests | The shared 10 s constant conflates "how long the test waits" with "how long release may take"; splitting the two is a cleanup if touched again. | todo |
| F407 | PATHHELPERS-05 review | `tests/repair_post.rs`, `src/util/path.rs` tests | The injected `*_skips_without_the_switch` tests print a `SKIP precondition:` line though nothing was skipped; distinguish the simulated refusal. | todo |
| F408 | PATHHELPERS-05 review | `tests/artifact_bridge.rs` | The `gircap`-absent skip still relies on `--no-capture`; a matching `success-output` override could surface it under a plain run. | todo |
| F409 | DOCS-USER-04 review | `tools/capture/README.md` | Its bridge run recipe (`--test-threads=1` variant) omits the `cargo build -p gircap` precondition step. | todo |
| F410 | DOCS-USER-04 review | `tests/artifact_bridge.rs` | The `#[ignore]` reason strings name only the nextest command; they could mention the verifier build so the listing is self-sufficient. | todo |
| F411 | DOCS-USER-04 review | `docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge.rs` | Cost wording disagrees on profile ("well under a minute in release" vs "minutes of wall-clock" for debug); state the profile explicitly. | todo |
| F412 | DOCS-COMMENTS-04 review | `docs/07-panic-audit.md` | The region list no longer contains `audio.rs`/`lock.rs` while the next paragraph says they were swept the same way; fold them in or reword the list. | todo |
| F413 | DOCS-COMMENTS-04 review | `docs/07-panic-audit.md` | Name `saturating_*`/`checked_*` and `+=`/`-=` explicitly beside "arithmetic" so the prose mirrors the sweep command token for token. | todo |
| F414 | HYP-05 review | `tests/common/mod.rs` | `logs_of` still carries the old "thread-local and cannot race another test" wording; apply the corrected `set_default` + current-thread-runtime phrasing. | todo |
| F415 | HYP-05 review | `tests/common/mod.rs`, `src/repair/phases/metadata.rs` | The async `logs_of` duplication remains; consolidating it needs a public async helper, so leave unless that API is wanted. | todo |
| F416 | METADATA-05 review | `tests/repair_offline.rs` | After the rename, the temp-dir label `branch_terminal_metadata` and the `expect_err` message still say "terminal" for a fatal non-JSON parse. | todo |
| F417 | METADATA-05 review | `tests/common/mod.rs` | `is_terminal_retcode` duplicates the production predicate; exposing one classifier in `test_support` would prevent drift. | todo |
| F418 | METADATA-05 review | `tests/repair_offline.rs` | The `-1001` fixture construction was converted in the rename commit rather than the validator commit; history nit only. | todo |
| F419 | MANIFEST-05 review | `tests/repair_scan.rs` | `an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate` took 11.0 s in a full-suite run; may merit a look if it recurs. | todo |
| F420 | MANIFEST-05 review | `src/repair/phases/metadata.rs`, `src/sophon/stream.rs`, `src/hyp.rs`, `tests/common` | Plain test HTTP servers remain duplicated; consolidation is worthwhile only once a second gated-origin user exists. | todo |
| F421 | JOBS-04 review | `tests/cli_contract.rs` | `help_entry`'s synthetic fixtures spell the six-space indent literally instead of using `HELP_OPTION_INDENT`. | todo |
| F422 | JOBS-04 review | `tests/cli_contract.rs` | `help_entry` cannot find an option rendered with a short-alias prefix (`-j, --jobs`); it would panic rather than return a wrong slice. | todo |
| F423 | JOBS-04 review | `tests/cli_contract.rs` | The boundary set omits `\r`; harmless for clap's `\n` output but would break on a Windows-console capture. | todo |
| F424 | LOGGING-04 review | `src/logging.rs` tests | `SINK_REGISTRY_TEST_LOCK`'s doc should note the reliance on no other test installing a global subscriber first (nextest process-per-test). | todo |
| F425 | LOGGING-04 review | `src/logging.rs` tests | The warning assertion pins the prefix but not the upstream `SetGlobalDefaultError` reason (`already been set`); optional. | todo |
| F426 | LOCK-05 review | `tests/common/run_lock.rs` | The CLI rejects a repeated `--game-path`; `game_path_arg` could return `None` for duplicates so cleanup never targets a path that never ran. | todo |
| F427 | LOCK-05 review | `tests/common/run_lock.rs` | The `#[cfg(test)]` module in the `#[path]`-shared file runs in every including binary; a dedicated module/bin would cut duplicated executions. | todo |
| F428 | LOCK-05 review | `tests/common/run_lock.rs` | `lock_key_path` mis-keys an empty `--game-path` (anchors at cwd while production keys `""`); note it if empty paths ever matter. | todo |
| F429 | INI-05 review | `src/util/ini.rs` tests | The junction test's `remove_dir` runs only after every assertion; a cleanup guard would tidy failure paths that leave a dangling junction. | todo |
| F430 | INI-05 review | `src/util/ini.rs` tests | The dangling-symlink test leaves its link for `TempDir` cleanup while the junction test removes its own; share one reparse-point cleanup strategy. | todo |
| F431 | INI-05 review | `src/util/ini.rs` tests | `try_junction_dir` passes paths as argv to `cmd /C`; a single pre-quoted command line would harden unusual temp paths. | todo |
| F432 | INI-05 review | `src/util/ini.rs` tests | The call-site test drives the private helper directly, so a refactor could bypass it; no practical alternative exists. | todo |
| F433 | INI-05 review | `src/util/ini.rs` | `symlink_skip_reason` treats every `Unsupported` as a host skip; intended, but the set can broaden as std maps more codes there. | todo |
| F434 | BUILD-05 review | `.github/workflows/msrv.yml`, `tests/scripts/io-profile/iot-bench/Cargo.toml` | `1.89` is hardcoded in the workflow and README while `check-msrv.ps1` derives versions; the bench pin could lag when MSRV moves. | todo |
| F435 | BUILD-05 review | `.github/workflows/msrv.yml`, `tests/scripts/check-msrv.ps1` | The `iot-bench` install step reimplements the script's toolchain install; factor out only if more toolchain jobs appear. | todo |
| F436 | BUILD-05 review | `.github/workflows/msrv.yml` | rust-cache hashes every runner toolchain, so the bench cache key churns as images add toolchains; optional narrower keying. | todo |
| F437 | BUILD-TOOLS-05 review | `tools/capture/Cargo.toml` | The comment's "so the saving is what this list does not ask for" clause sits after listing transitive features and could read more clearly. | todo |
| F438 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` | `ChunkFileSink` also serves the in-place download, so its reuse-copy comparison names only the temp path; mention `copy_within` charging likewise. | todo |
| F439 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` tests | `write_calls == 2` relies on the first regular-file read returning exactly `COPY_BUF`; a read-counting source wrapper would remove the OS-read dependence. | todo |
| F440 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` tests | The ~12-line `ReuseCall` construction repeats across roughly nine tests; a small builder helper would reduce it. | todo |
| F441 | TEMP-PUBLISH-05 review | `src/util/atomic.rs` tests | The `!tmp.exists()` check uses the unresolved `missing/../.girpr_tmp`, so it is redundant beside the parent-dir assertion; assert the resolved fallback. | todo |
| F442 | TEMP-PUBLISH-05 review | `src/util/path.rs` | `temp_sibling`'s doc names only `copy_publish_with` as the refusing caller; `atomic::publish` now refuses too. | todo |
| F443 | TEMP-PUBLISH-05 review | `src/util/atomic.rs` | `Path::file_name()` is `Some("dir")` for `dir/.`, so the new guard misses that spelling; the rename still fails and cleans up. | todo |
| F444 | TEST-MANIFEST-05 review | `src/sophon/stream.rs`, `tools/capture` | `join_url` (`{prefix}/{id}`) conflicts with the capture tool's `{prefix}/{id}{suffix}`; decide the contract and test a non-empty suffix. | todo |
| F445 | TEST-MANIFEST-05 review | `src/lib.rs` | The `test_support` doc groups `join_url` under helpers the offline server and peak-heap probe need, but the new consumer is `tests/repair_offline.rs`. | todo |
| F446 | TEST-MANIFEST-05 review | `tests/repair_offline.rs` | `extra_manifest` is local; if another test binary describes extra latest manifests, move it into `tests/common`. | todo |
| F447 | PURGE-PERF-07 review | `src/sophon/stream.rs` tests | `serve_status` shares the accept/count/one-shot-reply skeleton with `test_http::serve_bodies`; a status-aware parameter would remove a third copy. | todo |
| F448 | PURGE-PERF-07 review | `src/util/counter.rs` tests | Both pause tests repeat `const RECEIVE_WAIT: Duration = Duration::from_secs(10)`; a module-level constant would keep the single hang guard in one place. | todo |
| F449 | PURGE-PERF-07 review | `src/util/counter.rs` tests | On assertion failure the helper thread stays parked at the two-party barrier until process exit; worth a release path if the suite ever joins threads in-process. | todo |
| F450 | PURGE-PERF-07 review | `src/lib.rs` | `mod test_http;` sits after the alphabetical module list; cosmetic placement fix. | todo |

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
| BUILD-TOOLS-01 | F88, F99 | done | Wave A; dev-tool dependency narrowing; medium; 737c91e, 27726ef |
| TEST-HELPERS-01 | F108 | done | Wave A; one shared `md5_hex` test helper; low; dd373fd |
| LINT-01 | F97, F109 | done | Wave A, after BUILD-TOOLS-01; central test-lint policy; low; 594f37a, a7ea85f, cedb038, c36bf08 |
| TEMP-WRITER-01 | F25, F26, F27, F28, F30 | done | Wave B; temp writer rollback, refunds, docs, seek; medium; a98d906, 5e55c8f, 470cad9, beaf64d, 6fc928a |
| TEMP-PUBLISH-01 | F05, F06, F09, F12 | done | Wave B; temp-sibling publish of audio and plan files; medium; 8485d11, 3f232f3, 1a6e4e6, 2fb2e21 |
| INPLACE-01 | F29, F33, F34, F70, F79 | done | Wave B; in-place reuse accuracy and guards; medium; e7b963d, 7dd2316, 4f49e07, 0c3cea4, 13d5335 |
| CHECK-01 | F19, F21, F23, F52 | done | Wave B; check-phase progress, totals, panic verdict; high; 3468022, d919207 |
| REPORT-API-01 | F42, F43, F48, F83 | done | Wave B; report type visibility and construction; medium; 1cd5039, 3080e19, e23a64f |
| PHASE-01 | F114, F115 | done | Wave B; phase-timer coverage and error-path logging; low; fa568fd, 3539316 |
| TEST-MANIFEST-01 | F18, F110 | done | Wave B, after MANIFEST-01; multi-manifest fixtures and binary ordering; medium; a45f32b, 25b95b2 |
| ATOMIC-01 | F11, F60, F65, F66 | done | Wave C, after INI-01 and TEMP-PUBLISH-01; atomic-writer edges and docs; low; 1711f38, a673fb7, a90d7c7 |
| PLANID-01 | F56, F57, F58, F59 | done | Wave C, after INPLACE-01 and ATOMIC-01; plan identity; high; ce142b2, 8f322d3, 7632211, ed935e0, 38514bd |
| EXIT-01 | F85, F86, F87 | done | Wave C, after CHECK-01; named exit-code API and prose; medium; 2df6b66, ae55a2d, 05f0995 |
| DRYRUN-01 | F46, F47, F51, F73 | done | Wave C, after TEMP-PUBLISH-01 and PHASE-01; dry-run post contract; high; f3eb86c, 5f6d49e, 28c25cc |
| PURGE-PERF-01 | F20, F102, F103 | done | Wave C, after TEMP-PUBLISH-01; move purge work off async workers; high; fb3482d, f2356a8, f8df8b7 |
| MD5-TYPE-01 | F24 | done | Wave D, after PLANID-01; carry `Md5` through scan results; medium; f7a5a1e |
| PATHHELPERS-01 | F67, F68, F69, F71, F187 | done | Wave D, after INPLACE-01, PLANID-01, DRYRUN-01, PURGE-PERF-01; low; 44ce5ed, 9bfbf9b, 40b23a5, 684b89a |
| REFACTOR-01 | F80, F82 | done | Wave D, after PHASE-01, INPLACE-01, PURGE-PERF-01; medium; c21fa15, 231e6bd |
| PLANID-02 | F188, F189, F190, F191 | done | Wave D; plan API guardrails and fingerprint robustness; low; 32467fb, fac16a9, 012e06d, 334dc1f |
| EXIT-02 | F192, F193, F194, F195 | done | Wave D; exit-code boundary polish and prose; low; 764fbf9, 2c94a60, ad2cbb1, ff4539a |
| EXIT-03 | F208 | done | Wave E; clamp helper reachability; low; 7d883da, 3602837 |
| EXIT-04 | F271, F272, F273, F274 | done | Wave E; structural exit-code width and single source of variants; low; 3c50fde |
| EXIT-05 | F301, F302, F303 | done | Wave E; ALL completeness, tautological test and visibility; low; 09f5730, d283472, 27813fc |
| EXIT-06 | F331 | done | Wave E; binary exit-status list maintenance note; low; e51ef4f |
| EXIT-07 | F394, F395 | done | Wave E; macro generality and duplicated status list; low; 3ffe754 |
| PURGE-PERF-07 | F396, F397, F398 | done | Wave E; counter-test failure modes and shared stub; low; 3b5ec92, 45a6136 |
| TEST-REGRESSION-05 | F399, F400 | done | Wave E; remaining epoch assumptions in test helpers; low; c2c4cd9, 54b10bc, 07be299 |
| PURGE-PERF-06 | F332, F333 | done | Wave E; counter scoping and fetch-path coverage; low; af319a5, c581925 |
| DRYRUN-02 | F196, F197, F198 | done | Wave D; dry-run end-to-end coverage and cleanup-line parity; low; b98cfa7, 8c10743, ca30361, a220f91, 55f8843 |
| DRYRUN-03 | F209, F210, F211 | done | Wave E; dry-run zero-byte line, ceiling-stage distinction and plan-out purge hazard; low; af47aa8, 7ebfcc6, cbae708, 61f7bfe, b71f0ed |
| DRYRUN-04 | F275, F276 | done | Wave E; purge-before plan-out coverage and asymmetry decision; low; 8023145, 28c1716 |
| PURGE-PERF-02 | F199, F200, F201, F202 | done | Wave D; purge arithmetic hardening and blocking-helper consistency; low; 3cb5771, 62b05e2, fe8d45e |
| PURGE-PERF-03 | F212, F213 | done | Wave E; last wrap path and deprecated-list conversion; low; 3506fdd, f6f6e4f |
| PURGE-PERF-04 | F277, F278 | done | Wave E; shared saturating-add helper and deprecated collect allocation; low; 803dcbf, 12a4941 |
| PURGE-PERF-05 | F305, F306, F307 | done | Wave E; deprecated normalization test and counter-barrier notes; low; 4af6344, c6871df, a59c949, a76c901 |
| TEST-REGRESSION-02 | F214, F215 | done | Wave E; coarse-mtime flake and test focus; low; 967f670, e4a1ce0 |
| TEST-REGRESSION-03 | F279, F280 | done | Wave E; host-clock-independent stamps and shared phase-test setup; low; ea1dc1f, ee7d6c7 |
| TEST-REGRESSION-04 | F353, F354, F355, F356, F360 | done | Wave E; remaining absolute stamps and process-test setup reuse; low; 016c716, ed4a829, 93f5aed |
| TEST-OUTPUT-02 | F216, F217, F218 | done | Wave E; JSON-plane summary assertion, text-value guard and extra text paths; low; 4c0ef53, babff10 |
| TEST-OUTPUT-03 | F281, F282, F283, F284 | done | Wave E; value-aware text parser and stronger plan-out assertions; low; 1fcd7d9, d3c096f |
| TEST-OUTPUT-04 | F357, F358, F359 | done | Wave E; text-grammar ambiguity and shared schema vocabulary; low; f38bf2b, e5d1a3d |
| TEST-OUTPUT-05 | F401, F402, F403 | todo | Wave E; remaining text-plane vocabulary notes; low |
| CHECK-06 | F404, F405, F406 | todo | Wave E; gate-test failure reporting and deadline unification; low |
| PATHHELPERS-06 | F407, F408 | todo | Wave E; simulated-skip wording and bridge skip visibility; low |
| DOCS-USER-05 | F409, F410, F411 | todo | Wave E; bridge recipe alignment and profile wording; low |
| DOCS-COMMENTS-05 | F412, F413 | todo | Wave E; panic-audit region list and sweep vocabulary; low |
| HYP-06 | F414, F415 | todo | Wave E; shared log-helper wording and duplication decision; low |
| METADATA-06 | F416, F417, F418 | todo | Wave E; branch-failure wording and shared terminal predicate; low |
| MANIFEST-06 | F419, F420 | todo | Wave E; slow md5-gate test and server duplication; low |
| JOBS-05 | F421, F422, F423 | todo | Wave E; help_entry fixture constants and boundary edges; low |
| LOGGING-05 | F424, F425 | todo | Wave E; global-subscriber test preconditions; low |
| LOCK-06 | F426, F427, F428 | todo | Wave E; duplicate game-path, shared-module test volume, empty key; low |
| INI-06 | F429, F430, F431, F432, F433 | todo | Wave E; junction/symlink test cleanup and platform broadenings; low |
| BUILD-06 | F434, F435, F436 | todo | Wave E; MSRV pin derivation and cache keying; low |
| BUILD-TOOLS-06 | F437 | todo | Wave E; capture comment wording; low |
| TEMP-WRITER-06 | F438, F439, F440 | todo | Wave E; dual-mode sink doc, read wrapper, reuse builder; low |
| TEMP-PUBLISH-06 | F441, F442, F443 | todo | Wave E; publish guard spelling and doc sync; low |
| TEST-MANIFEST-06 | F444, F445, F446 | todo | Wave E; suffix contract, seam doc, helper placement; low |
| PURGE-PERF-08 | F447, F448, F449, F450 | todo | Wave E; stub status variant, shared hang-guard constant, thread release, module order; low |
| INPLACE-03 | F219, F220, F221 | done | Wave E; guard-test coverage and residual-race docs; low; 5452da5, c5ff283, 55e56cc |
| INPLACE-04 | F285 | done | Wave E; fixed far-future stamp in the age-branch test; low; 9a38ddb |
| CHECK-03 | F222, F223 | done | Wave E; test timeout and tally field visibility; low; 852fb0a, ccec3a3 |
| CHECK-04 | F286, F287 | done | Wave E; bounded gate wait and counter doc; low; 39df01f, 0cb49b2 |
| CHECK-05 | F361, F362, F363 | done | Wave E; doc precision and remaining bounded waits; low; c76def0, 7d8c6d7 |
| PATHHELPERS-02 | F204, F205, F206 | done | Wave D; audio-move path reuse and non-UTF-8 coverage; low; e17247f, e30f620, e3128e5 |
| PATHHELPERS-03 | F256, F257 | done | Wave E; component-boundary case and CLI-layer non-UTF-8 coverage; low; a8fa982, 7625d90 |
| PATHHELPERS-04 | F288, F289 | done | Wave E; test doc accuracy and strict non-UTF-8 skip mode; low; 43e3eba, d87959e |
| PATHHELPERS-05 | F364, F365, F366, F367 | done | Wave E; skip visibility, injected-write seam and remaining silent skips; low; 8fdfce9, 63b3740, de58699 |
| REFACTOR-02 | F207 | done | Wave E; make the download estimate read as pure; low; 0e79367 |
| REFACTOR-03 | F269, F270 | done | Wave E; estimate type split and limit-source doc; low; 880fd58 |
| TEST-REGRESSION-01 | F32, F112, F113, F126, F127, F130 | done | Wave D, after INPLACE-01, DRYRUN-01, TEMP-PUBLISH-01; mutation, partial-copy and timing regressions; low; cadadd0, 0c61ffd, 11e27b1, 9a430aa, b053632 |
| TEST-OUTPUT-01 | F53, F116, F119 | done | Wave D, after CHECK-01 and PHASE-01; output-plane golden tightening; medium; 93d71d5, ccdfe50 |
| DOCS-USER-01 | F38, F41, F95, F105, F117 | done | Wave E, after LOCK-01, JOBS-01, LOGGING-01, TEST-OUTPUT-01; low; e8ae20c, 3a0d207, b4b5c48, 3fc46c6, 1ddbb61 |
| DOCS-USER-02 | F262, F263, F264 | done | Wave E; drifting counts, fatal-path wording and alias spelling; low; aba6aea, ef16ae7, 38047d4, b768c22 |
| DOCS-USER-03 | F297, F298 | done | Wave E; ignored bridge run and scan-test flag modernization; low; 3c38d19, 15b37c7, 974e5f3 |
| DOCS-USER-04 | F368, F369, F370, F371 | done | Wave E; bridge rationale, prose flags and release precondition; low; a78ad3b, 18306c1, 0b8d354 |
| DOCS-COMMENTS-01 | F63, F72, F74, F77, F118, F120, F121, F122, F124, F129, F178, F179, F180, F181, F185, F186, F203 | done | Wave E, after REFACTOR-01 and PATHHELPERS-01; stale comments, module summaries, rustdoc links, and doc precision; low; 2fa0805, 07dafc0, c7080d1, d00d77b, 03e3924, 8b2ebc5, 14b1118, 394e281, 7e0f8ca, 7ac798d |
| DOCS-COMMENTS-02 | F265, F266, F267, F268 | done | Wave E; remaining module-map, re-export and line-range staleness; low; ff2e70a, 011ce1f, 3e35cc2, 6a2f2d8 |
| DOCS-COMMENTS-03 | F299, F300 | done | Wave E; audit local-invariant coverage and file list; low; d7ce878 |
| DOCS-COMMENTS-04 | F372, F373, F374, F375, F376 | done | Wave E; audit method/table consistency and missing section refs; low; e236e0e, 354d74c, edfbd7a |
| HYP-02 | F177 | done | Wave D; validate the hyp response identity against the request; medium; 9e7d77d |
| HYP-03 | F253, F254, F255 | done | Wave E; local fixture tag, warning wording and mismatch precedence; low; ec3e7aa, 64e5c56, a9e8598 |
| HYP-04 | F308, F309 | done | Wave E; test tag constants and multi-thread log capture; low; 5cf9315, d1bdced, 4922eec |
| HYP-05 | F377, F378, F379, F380, F381 | done | Wave E; traced-spawn docs/tests and log-helper consolidation; low; 2eba4c5, 0d9bbe0, 2e1b83e, ccf01a6 |
| SYNC-01 | F123, F125 | done | Wave A; girsync map-key call-site coverage and ASCII hash-name folding; low; a467632, 8daccfb |
| SYNC-02 | F251, F252 | done | Wave E; planner-side non-ASCII pairing and hash-name revisit note; low; fcf94f1, 39d3e99, 2049582 |
| METADATA-02 | F131, F132, F133, F134, F135 | done | Wave D; metadata classification tests, label constants and doc precision; low; d07d6d5, 37f4c8f, 3864bb9, 490dd06, 89763ec, 38e3f93 |
| METADATA-03 | F244, F245, F246 | done | Wave E; variant exhaustiveness, notify safety and terminal branch failure; low; 60d20de, 021568e, 4170fc9, c8c6194 |
| METADATA-04 | F310, F311 | done | Wave E; mock retryable branch failure mode; low; 636aa70 |
| METADATA-05 | F349, F350 | done | Wave E; branch-failure knob validation and test naming; low; 912a57e, d7d206f |
| MANIFEST-02 | F136, F137 | done | Wave D; manifest mock shutdown and out-of-order stream coverage; low; 6abf7c9, 538e0ad |
| MANIFEST-03 | F241, F242, F243 | done | Wave E; first-error message, live-stream coverage and task reaping; low; 4441005, 205980a, de13f61 |
| MANIFEST-04 | F312, F313, F314 | done | Wave E; test-server wakeup and bound-table hardening; low; 300900d, 6a886b6 |
| MANIFEST-05 | F351, F352 | skipped | Wave E; 404-manifest stability pass and shared coordination helpers; low |
| JOBS-02 | F139, F140, F141 | done | Wave D; alias boundary case, help default annotation and ceiling review; low; 3f602a7, 1749bb9, 6f34e2f |
| JOBS-03 | F238, F239, F240 | done | Wave E; help assertion scope, default literal and binary parity; low; bcdb196, f3491e8 |
| JOBS-04 | F315, F316, F317 | done | Wave E; help-entry slicing hardening; low; 388365c, 5447a8e, 190a061, 5e111b2 |
| LOGGING-02 | F142, F143, F144 | done | Wave D; sink registry lifetime, install-to-flush coverage and per-sink errors; low; 04d8474, f6be025 |
| LOGGING-03 | F235, F236, F237 | done | Wave E; sink registry typing, rejected-install test and flush doc; low; 81d3ef5, 3433b1c |
| LOGGING-04 | F318, F319 | done | Wave E; sink naming and rejected-install test strength; low; 9b87298, a2ea354, 281878b |
| LOCK-02 | F145, F146, F147 | done | Wave D; lock handle-identity hardening, test litter and Unix-only coverage; medium; f60e44f, 0cb311e; F147 blocked on a Unix host |
| LOCK-03 | F224, F225 | done | Wave E; integration lock litter and nlink wording; low; abf0516, ae143c3 |
| LOCK-04 | F320, F321, F322 | done | Wave E; lock-cleanup helper consolidation and edge notes; low; aa0c802, b59ecd1, e4fcba8 |
| LOCK-05 | F340, F341, F342, F343, F344 | done | Wave E; lock-policy mirror tests and cleanup edge cases; low; 78b827c, 3a64ea9, a3690de, 7a0a7a4, 6dc6a34, 145fcc0 |
| INI-02 | F148, F149, F150 | done | Wave D; ini refusal wording, external-race coverage and helper scope; low; a21ba8e, 4e01a1d, e8829e4 |
| INI-03 | F233, F234 | done | Wave E; dangling-symlink and masked-permission wording; low; 133d1d0, eb9c78f, 285ff2d |
| INI-04 | F323, F324, F325 | done | Wave E; inspection-error surfacing, junction wording and symlink-skip precision; low; 86cc49b, a495877 |
| INI-05 | F345, F346, F347, F348 | done | Wave E; inspection context test and junction/skip docs; low; b163784, 1def4fb, 458f0e6, e522f0a, b0e85df |
| BUILD-02 | F151, F152, F153, F226 | done | Wave D; per-member MSRV, scan-path benchmark and import tidy; low; cb45eb6, 854c371, 66fd07a, db9cb80 |
| BUILD-TOOLS-02 | F154, F155 | done | Wave D; tools build-doc note and explicit digest trait import; low; 02ff8aa, 62b9fc3, 0ebb636 |
| BUILD-03 | F227, F228, F229 | done | Wave E; iot-bench pin, benchmark honesty and MSRV enforcement; low; c13c7e7, 6332e10, 21400a1 |
| BUILD-04 | F326, F327, F328 | done | Wave E; CI cost and MSRV/bench gate coverage; low; dd79ec1, fa5d90a, 315a356 |
| BUILD-05 | F334, F335, F336 | done | Wave E; bench toolchain pin, README wording and required-check coverage; low; 71587e5, 54f1e72 |
| BUILD-TOOLS-03 | F230, F231 | done | Wave E; tokio default features and manifest comment precision; low; a30bdaf |
| BUILD-TOOLS-04 | F329, F330 | done | Wave E; root tokio default features and comment nuance; low; 1effa98, 2681403 |
| BUILD-TOOLS-05 | F337, F338, F339 | done | Wave E; tokio dev-dependency and comment precision; low; 9ab8ef8, 8a577d0 |
| TEST-HELPERS-02 | F156, F157 | done | Wave D; shared-module build cost and redundant `#[path]`; low; 2266a18, acdf2af |
| TEST-HELPERS-03 | F232 | done | Wave E; digest helper doc wording; low; fac7264 |
| TEST-MANIFEST-02 | F182, F183, F184 | done | Wave D; multi-manifest fixture filter and served-absent cases; low; 08e2a6f, 1087905 |
| TEST-MANIFEST-03 | F258, F259 | done | Wave E; shared chunk-id and direct advertisement assertions; low; c611c7d, 41a7b28 |
| TEST-MANIFEST-04 | F290, F291 | done | Wave E; advertisement-to-servability and field-based id lookup; low; 3bb9805, 2c288c6 |
| TEST-MANIFEST-05 | F390, F391, F392, F393 | done | Wave E; shared join helper and remaining manifest-test hardening; low; 72ad074, 1a1a574, 5fe0036 |
| LINT-02 | F158, F160 | done | Wave D; clippy.toml for unit-test allows and tools expect cleanup; low; 9de1581, 1ce9966, cb410fa |
| LINT-03 | F260, F261 | done | Wave E; select-order coupling and to_ndjson doc; low; 0758f03, 5e0aae5, fe464ad |
| TEMP-WRITER-02 | F161, F162, F163, F164 | done | Wave D; temp writer positioning seam, end-to-end refund and comment accuracy; low; 412739a, c0721b3, 686c4b5, 049009b |
| TEMP-WRITER-03 | F247, F248 | done | Wave E; sink genericity and broader cursor pinning; low; 953db0b, 563a178 |
| TEMP-WRITER-04 | F292, F293, F294 | done | Wave E; reservation-order pin, generic doc and counter name; low; c156617, 740f5d0, 076bbfc |
| TEMP-WRITER-05 | F382, F383, F384, F385, F386 | done | Wave E; recorder naming and doc precision; low; 7a3b573, b3011d4, fe59130, 5ef5c89 |
| TEMP-PUBLISH-02 | F165, F166, F167, F168 | done | Wave D; publish test naming, ordering assertion, refusal test and Unix run; low; f0a42f0, bae3465, 86c6fc1; F168 blocked on a Unix host |
| TEMP-PUBLISH-03 | F249, F250 | done | Wave E; temp-sibling doc edge and guard reachability note; low; 1e96ab6, 02b346d |
| TEMP-PUBLISH-04 | F295, F296 | skipped | Wave E; fallback-decision and future walk-reachability notes; low |
| TEMP-PUBLISH-05 | F387, F388, F389 | done | Wave E; no-file-name publish refusal and temp-sibling resolution pin; low; 1697a35, 0d58148, f11563c |
| INPLACE-02 | F169, F170, F171, F172 | done | Wave D; coarse-mtime skip-path hardening and guard robustness; medium; 7744afb, 9e232ad, bb03835 |
| CHECK-02 | F173, F174, F175, F176 | done | Wave D; check-progress signal, tally ambiguity, cancellation and comment; low; ab15eae, 3895704, f682292, f46cf6d, ff8a17a |

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
