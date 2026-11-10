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
| F401 | TEST-OUTPUT-04 review | `tests/repair_scan.rs` | The emitted `scan_bytes=` value is still spelled literally; a shared spelling could be used if a test-support location exists. | skipped (no shared key location in scope; PLAN_SCAN_BYTES_KEY is private to cli_contract) |
| F402 | TEST-OUTPUT-04 review | `README.md` | The bare-text limitation is documented only beside the test parser; the `--output` guidance could point users to JSON when values may contain `key=` tokens. | completed |
| F403 | TEST-OUTPUT-04 review | `tests/cli_contract.rs` | If production later exposes a key/schema manifest, the golden key lists could be derived from it instead of duplicated. | completed |
| F404 | CHECK-05 review | `src/repair/phases/check.rs` tests | On expiry the injected closure returns `CheckOutcome::clean()` so counters still read 2; a distinct outcome or comment would read clearer. | completed |
| F405 | CHECK-05 review | `src/repair/phases/check.rs` tests | Three waits each get a full `GATE_WAIT` from their own start; one absolute test deadline shared by all would make failure ordering unambiguous. | completed |
| F406 | CHECK-05 review | `src/repair/phases/check.rs` tests | The shared 10 s constant conflates "how long the test waits" with "how long release may take"; splitting the two is a cleanup if touched again. | skipped (only one real wait duration; the release wait is the same absolute deadline) |
| F407 | PATHHELPERS-05 review | `tests/repair_post.rs`, `src/util/path.rs` tests | The injected `*_skips_without_the_switch` tests print a `SKIP precondition:` line though nothing was skipped; distinguish the simulated refusal. | completed |
| F408 | PATHHELPERS-05 review | `tests/artifact_bridge.rs` | The `gircap`-absent skip still relies on `--no-capture`; a matching `success-output` override could surface it under a plain run. | completed |
| F409 | DOCS-USER-04 review | `tools/capture/README.md` | Its bridge run recipe (`--test-threads=1` variant) omits the `cargo build -p gircap` precondition step. | completed |
| F410 | DOCS-USER-04 review | `tests/artifact_bridge.rs` | The `#[ignore]` reason strings name only the nextest command; they could mention the verifier build so the listing is self-sufficient. | completed |
| F411 | DOCS-USER-04 review | `docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge.rs` | Cost wording disagrees on profile ("well under a minute in release" vs "minutes of wall-clock" for debug); state the profile explicitly. | completed |
| F412 | DOCS-COMMENTS-04 review | `docs/07-panic-audit.md` | The region list no longer contains `audio.rs`/`lock.rs` while the next paragraph says they were swept the same way; fold them in or reword the list. | completed |
| F413 | DOCS-COMMENTS-04 review | `docs/07-panic-audit.md` | Name `saturating_*`/`checked_*` and `+=`/`-=` explicitly beside "arithmetic" so the prose mirrors the sweep command token for token. | completed |
| F414 | HYP-05 review | `tests/common/mod.rs` | `logs_of` still carries the old "thread-local and cannot race another test" wording; apply the corrected `set_default` + current-thread-runtime phrasing. | completed |
| F415 | HYP-05 review | `tests/common/mod.rs`, `src/repair/phases/metadata.rs` | The async `logs_of` duplication remains; consolidating it needs a public async helper, so leave unless that API is wanted. | skipped (needs a public async test helper; no non-public seam covers it) |
| F416 | METADATA-05 review | `tests/repair_offline.rs` | After the rename, the temp-dir label `branch_terminal_metadata` and the `expect_err` message still say "terminal" for a fatal non-JSON parse. | completed |
| F417 | METADATA-05 review | `tests/common/mod.rs` | `is_terminal_retcode` duplicates the production predicate; exposing one classifier in `test_support` would prevent drift. | completed |
| F418 | METADATA-05 review | `tests/repair_offline.rs` | The `-1001` fixture construction was converted in the rename commit rather than the validator commit; history nit only. | skipped (history-only; the fixture is already distinguished by its constructor and doc) |
| F419 | MANIFEST-05 review | `tests/repair_scan.rs` | `an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate` took 11.0 s in a full-suite run; may merit a look if it recurs. | completed |
| F420 | MANIFEST-05 review | `src/repair/phases/metadata.rs`, `src/sophon/stream.rs`, `src/hyp.rs`, `tests/common` | Plain test HTTP servers remain duplicated; consolidation is worthwhile only once a second gated-origin user exists. | skipped (no second gated-origin user; the three plain helpers differ in contract) |
| F421 | JOBS-04 review | `tests/cli_contract.rs` | `help_entry`'s synthetic fixtures spell the six-space indent literally instead of using `HELP_OPTION_INDENT`. | completed |
| F422 | JOBS-04 review | `tests/cli_contract.rs` | `help_entry` cannot find an option rendered with a short-alias prefix (`-j, --jobs`); it would panic rather than return a wrong slice. | completed |
| F423 | JOBS-04 review | `tests/cli_contract.rs` | The boundary set omits `\r`; harmless for clap's `\n` output but would break on a Windows-console capture. | completed |
| F424 | LOGGING-04 review | `src/logging.rs` tests | `SINK_REGISTRY_TEST_LOCK`'s doc should note the reliance on no other test installing a global subscriber first (nextest process-per-test). | completed |
| F425 | LOGGING-04 review | `src/logging.rs` tests | The warning assertion pins the prefix but not the upstream `SetGlobalDefaultError` reason (`already been set`); optional. | completed |
| F426 | LOCK-05 review | `tests/common/run_lock.rs` | The CLI rejects a repeated `--game-path`; `game_path_arg` could return `None` for duplicates so cleanup never targets a path that never ran. | completed |
| F427 | LOCK-05 review | `tests/common/run_lock.rs` | The `#[cfg(test)]` module in the `#[path]`-shared file runs in every including binary; a dedicated module/bin would cut duplicated executions. | completed |
| F428 | LOCK-05 review | `tests/common/run_lock.rs` | `lock_key_path` mis-keys an empty `--game-path` (anchors at cwd while production keys `""`); note it if empty paths ever matter. | completed |
| F429 | INI-05 review | `src/util/ini.rs` tests | The junction test's `remove_dir` runs only after every assertion; a cleanup guard would tidy failure paths that leave a dangling junction. | completed |
| F430 | INI-05 review | `src/util/ini.rs` tests | The dangling-symlink test leaves its link for `TempDir` cleanup while the junction test removes its own; share one reparse-point cleanup strategy. | completed |
| F431 | INI-05 review | `src/util/ini.rs` tests | `try_junction_dir` passes paths as argv to `cmd /C`; a single pre-quoted command line would harden unusual temp paths. | completed |
| F432 | INI-05 review | `src/util/ini.rs` tests | The call-site test drives the private helper directly, so a refactor could bypass it; no practical alternative exists. | completed |
| F433 | INI-05 review | `src/util/ini.rs` | `symlink_skip_reason` treats every `Unsupported` as a host skip; intended, but the set can broaden as std maps more codes there. | completed |
| F434 | BUILD-05 review | `.github/workflows/msrv.yml`, `tests/scripts/io-profile/iot-bench/Cargo.toml` | `1.89` is hardcoded in the workflow and README while `check-msrv.ps1` derives versions; the bench pin could lag when MSRV moves. | completed |
| F435 | BUILD-05 review | `.github/workflows/msrv.yml`, `tests/scripts/check-msrv.ps1` | The `iot-bench` install step reimplements the script's toolchain install; factor out only if more toolchain jobs appear. | skipped (only two install sites; sharing needs an interface for one line) |
| F436 | BUILD-05 review | `.github/workflows/msrv.yml` | rust-cache hashes every runner toolchain, so the bench cache key churns as images add toolchains; optional narrower keying. | skipped (rust-cache has no toolchain-exclusion input; raw actions/cache would be worse) |
| F437 | BUILD-TOOLS-05 review | `tools/capture/Cargo.toml` | The comment's "so the saving is what this list does not ask for" clause sits after listing transitive features and could read more clearly. | completed |
| F438 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` | `ChunkFileSink` also serves the in-place download, so its reuse-copy comparison names only the temp path; mention `copy_within` charging likewise. | completed |
| F439 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` tests | `write_calls == 2` relies on the first regular-file read returning exactly `COPY_BUF`; a read-counting source wrapper would remove the OS-read dependence. | completed |
| F440 | TEMP-WRITER-05 review | `src/repair/file/temp.rs` tests | The ~12-line `ReuseCall` construction repeats across roughly nine tests; a small builder helper would reduce it. | completed |
| F441 | TEMP-PUBLISH-05 review | `src/util/atomic.rs` tests | The `!tmp.exists()` check uses the unresolved `missing/../.girpr_tmp`, so it is redundant beside the parent-dir assertion; assert the resolved fallback. | completed |
| F442 | TEMP-PUBLISH-05 review | `src/util/path.rs` | `temp_sibling`'s doc names only `copy_publish_with` as the refusing caller; `atomic::publish` now refuses too. | completed |
| F443 | TEMP-PUBLISH-05 review | `src/util/atomic.rs` | `Path::file_name()` is `Some("dir")` for `dir/.`, so the new guard misses that spelling; the rename still fails and cleans up. | completed |
| F444 | TEST-MANIFEST-05 review | `src/sophon/stream.rs`, `tools/capture` | `join_url` (`{prefix}/{id}`) conflicts with the capture tool's `{prefix}/{id}{suffix}`; decide the contract and test a non-empty suffix. | skipped (documented contract is prefix-only; references agree; suffix semantics unmeasurable) |
| F445 | TEST-MANIFEST-05 review | `src/lib.rs` | The `test_support` doc groups `join_url` under helpers the offline server and peak-heap probe need, but the new consumer is `tests/repair_offline.rs`. | completed |
| F446 | TEST-MANIFEST-05 review | `tests/repair_offline.rs` | `extra_manifest` is local; if another test binary describes extra latest manifests, move it into `tests/common`. | skipped (only one test binary describes extra latest manifests) |
| F447 | PURGE-PERF-07 review | `src/sophon/stream.rs` tests | `serve_status` shares the accept/count/one-shot-reply skeleton with `test_http::serve_bodies`; a status-aware parameter would remove a third copy. | completed |
| F448 | PURGE-PERF-07 review | `src/util/counter.rs` tests | Both pause tests repeat `const RECEIVE_WAIT: Duration = Duration::from_secs(10)`; a module-level constant would keep the single hang guard in one place. | completed |
| F449 | PURGE-PERF-07 review | `src/util/counter.rs` tests | On assertion failure the helper thread stays parked at the two-party barrier until process exit; worth a release path if the suite ever joins threads in-process. | completed |
| F450 | PURGE-PERF-07 review | `src/lib.rs` | `mod test_http;` sits after the alphabetical module list; cosmetic placement fix. | completed |
| F451 | TEST-REGRESSION-05 review | `src/repair/file/inplace.rs` tests, `tests/repair_machinery.rs` | On a pre-epoch host `stamp_ago` returns `now`, so fixtures silently lose their "older than one coarse tick" property and the guarded-reuse path is not exercised. | completed |
| F452 | TEST-REGRESSION-05 review | `src/repair/file/inplace.rs` tests, `tests/repair_machinery.rs` | `stamp_ago_from`/`stamp_ago` and `coarse_tick_of` are duplicated between the unit-test module and the integration test; a shared test-support module would remove it. | completed |
| F453 | TEST-OUTPUT-05 review | `tests/common/mod.rs`, `tests/cli_contract.rs`, `tests/repair_scan.rs` | Move `PLAN_SCAN_BYTES_KEY` into `tests/common` so `repair_scan.rs` shares the spelling instead of writing `scan_bytes=` literally. | completed |
| F454 | TEST-OUTPUT-05 review | `README.md` | The `--output` flag table row is now very long; a short subsection could carry the caveat and keep the table readable. | completed |
| F455 | CHECK-06 review | `src/repair/phases/check.rs` tests | `before`'s doc says expiry is kept separate so a wait records it, but only the release wait distinguishes it; the two harness waits collapse it via `unwrap_or_default()`. | completed |
| F456 | CHECK-06 review | `src/repair/phases/check.rs` tests | Expiry is reported through `CheckFailure::Read`, so the log reads "CHECK fail: unreadable"; harmless for a test double, but a dedicated marker would read better. | completed |
| F457 | CHECK-06 review | `src/repair/phases/check.rs` tests | The shared deadline means each wait's headroom is "time left" rather than a fresh 10 s; worth a comment if the fixture grows. | completed |
| F458 | PATHHELPERS-06 review | `tests/artifact_bridge.rs` | The module doc and `#[ignore]` messages still tell readers to pass `--no-capture`, which the new override makes unnecessary. | completed |
| F459 | PATHHELPERS-06 review | `.config/nextest.toml` | The adjacent comment saying the run command carries `--no-capture` only to make the line visible is now stale; update with the docs. | completed |
| F460 | PATHHELPERS-06 review | `tests/repair_post.rs`, `src/util/path.rs` tests | `JunctionRefusal` and `NameRefusal` express the same two-state idea in two test modules; a shared helper could deduplicate them. | completed |
| F461 | DOCS-USER-05 review | `README.md`, `docs/06-artifact-bridge-spec.md`, `tools/capture/README.md`, `tests/artifact_bridge.rs` | The four bridge-run recipes differ cosmetically (`--test-threads=1`, `GIRPR_ARTIFACT_BUNDLE` spelled out in only two); unify them. | completed |
| F462 | DOCS-USER-05 review | `tools/capture/README.md` | The reference bundle label `20261021T141336Z` no longer matches the locally present bundle; the example can drift. | completed |
| F463 | DOCS-USER-05 review | `docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge.rs` | The 139 s cost figure is host-qualified but the command/profile that produced it is not recorded; state the exact command. | completed |
| F464 | DOCS-COMMENTS-05 review | `docs/07-panic-audit.md` | The sweep regex matches only `as usize|as u64|as i64` while the prose says "`as` casts" generally; narrow the wording or extend the command. | completed |
| F465 | HYP-06 review | `tests/common/mod.rs` | `logs_of`'s correctness depends on the default current-thread runtime; a future multi-thread `#[tokio::test]` caller would silently lose non-`spawn_blocking_traced` task lines. | completed |
| F466 | MANIFEST-06 follow-up | `src/repair/file/mod.rs`, `src/retry.rs` | A deterministic whole-file digest mismatch is retried 5× (~10 s of backoff) before reporting failure; it could be classified terminal or skipped when the plan has no download work. | completed |
| F467 | MANIFEST-06 follow-up | `src/repair/phases/manifest.rs`, `tests/artifact_bridge/server.rs`, `tests/common` | If a shared gated-origin helper is ever attempted, `manifest.rs::TestServer` and `artifact_bridge/server.rs::Origins` are the real candidates, not the plain helpers. | skipped (gated-origin helper does not exist; a note about a future one would be noise) |
| F468 | MANIFEST-06 review | `tests/repair_scan.rs` | Exit 3 is asserted without pinning that the md5 gate (not another write failure) produced it; asserting the final-digest message would harden it. | completed |
| F469 | JOBS-05 review | `tests/cli_contract.rs` | No test pins the panic message's "both shapes" wording; a `#[should_panic(expected = ...)]` test would lock it in. | completed |
| F470 | JOBS-05 review | `tests/cli_contract.rs` | No fixture combines CRLF with the short-alias shape; the two paths are tested only separately. | completed |
| F471 | JOBS-05 review | `tests/cli_contract.rs` | The four-space description suffix is still a literal in each `format!`; a `HELP_DESCRIPTION_INDENT` constant would remove the last layout literals. | completed |
| F472 | LOGGING-05 review | `src/logging.rs` tests | The lock doc's "only because nextest runs each test in its own process" is stronger than needed while no second `try_init` exists; a future subscriber-installing unit test must take the lock. | completed |
| F473 | LOGGING-05 review | `src/logging.rs` tests | `already been set` is tied to `tracing-core`'s wording; testing the typed `SetGlobalDefaultError` rejection would survive a dependency bump. | completed |
| F474 | LOCK-06 review | `tests/common/run_lock.rs` | `game_path_arg` still returns `Some` for a trailing `--game-path` with no value or a flag-looking value; clap rejects both, so the doc's "no value" claim is broader than the code. | completed |
| F475 | LOCK-06 review | `tests/run_lock.rs` | The module doc says the shared file is included "with `#[path]` by every binary", but `tests/common/mod.rs` includes it as a plain child module. | completed |
| F476 | INI-06 review | `src/util/ini.rs` tests | The junction test treats any `try_junction_dir` failure as a host skip, unlike the narrower `symlink_skip_reason`; a broken `cmd`/`mklink` environment would silently skip. | completed |
| F477 | INI-06 review | `src/util/ini.rs` tests | `LinkGuard::remove` disarms before attempting removal, so a failed success-path removal is never retried in `Drop`; swapping the order would retry cleanly. | completed |
| F478 | BUILD-06 review | `.github/workflows/msrv.yml` | A failed `cargo metadata` call surfaces as "root package does not declare rust-version"; an explicit `$LASTEXITCODE` check would report the real cause. | completed |
| F479 | BUILD-06 review | `README.md`, `tests/scripts/check-msrv.ps1` | The README and the script header still spell out `1.89`/`1.88`/`1.85`, which will drift when the MSRV moves. | completed |
| F480 | BUILD-06 review | `.github/workflows/msrv.yml`, `tests/scripts/check-msrv.ps1` | The duplicated `rustup toolchain install` is accepted for now; if a third install site appears, extract a shared installer helper. | skipped (install sites: 2, threshold not met) |
| F481 | TEMP-WRITER-06 review | `src/repair/file/temp.rs` tests | The read-error handle-drop path is pinned only at helper level; an end-to-end `try_reuse_slice` test would need a reader seam over the `File` handle. | completed |
| F482 | TEMP-WRITER-06 review | `src/repair/file/temp.rs` tests | `ExactReads` and `FailingReads` are local doubles; more copy-path tests may warrant a shared reader fixture. | completed |
| F483 | TEMP-WRITER-06 follow-up | `src/hyp.rs` | A pre-existing broken intra-doc link (`crate::sophon::retryable`) makes `cargo doc -p girpr --no-deps` fail under `-D warnings`. | completed |
| F484 | TEMP-PUBLISH-06 review | `src/repair/phases/post.rs` | `copy_publish_with` still guards only with `file_name()`, so it would accept a `dir/.` target; share `ends_in_a_file_name` if that writer becomes reachable. | skipped (writer is not reachable with a `dir/.` target) |
| F485 | TEMP-PUBLISH-06 review | `src/util/atomic.rs` | Windows targets ending in a dot or space (e.g. `config.`) are accepted and alias another name; `contained_rel` already refuses those, the atomic writer does not. | completed |
| F486 | TEMP-PUBLISH-06 review | `src/util/path.rs` | The doc says `atomic::publish` refuses "a target with no file name"; it also refuses directory spellings, so the wording could be broadened. | completed |
| F487 | TEST-MANIFEST-06 review | `tools/capture/src/manifest/mod.rs` | `CapturedManifest::url` and `chunk_url` append `url_suffix` without the new "unmeasured conservative reading" note. | completed |
| F488 | TEST-MANIFEST-06 review | `tools/capture` | Starward's unused `GameSophonManifestUrl.BuildUrl` reads `url_suffix` as a query string; informational unless reference parity is revisited. | skipped (external reference has no callers; no repo change) |
| F489 | MANIFEST-07 review | `src/repair/file/inplace.rs` | The overlap guard is conservative: duplicate identical chunks count as overlapping and stay retryable; a post-redo rescan could recover the terminal classification at the cost of one extra read on the failure path. | completed |
| F490 | MANIFEST-07 review | `tests/repair_scan.rs` | No end-to-end fixture with overlapping chunk offsets proves a retry eventually reaches the manifest digest; the current test pins the guard only. | completed |
| F491 | TEST-MANIFEST-07 review | `tools/capture/src/hyp/types.rs` | `Download::url_suffix` points two hops at the caveat through `archive_url`; a direct link is awkward because `join` is `pub(crate)`. | skipped (the two-hop chain is correct; no public target for a direct link) |
| F492 | TEST-MANIFEST-07 review | `tools/capture/src/hyp/types.rs` | The shared caveat now covers the chunk join but names only the manifest-fetch paths of the reference clients. | skipped (accurate while every measured `url_suffix` is empty) |
| F493 | PURGE-PERF-08 review | `src/util/counter.rs` tests | `ReleaseOnPanic` has a narrow race: if the guard drops after `Barrier::wait` releases the helper but before it records `passed`, the waiter can park until exit. | completed |
| F494 | PURGE-PERF-08 review | `src/repair/mod.rs` | A private 10 s `RECEIVE_WAIT` remains there; it is a candidate for the same single-constant hoist. | skipped (different operation from the hoisted counter constant; one caller, no shared constant to join) |
| F495 | TEST-REGRESSION-06 review | `src/lib.rs` | The new helper pins cover an ordinary and a pre-epoch clock, but not a clock shortly *after* the epoch crossing backward, which was the removed test's other shape. | completed |
| F496 | TEST-REGRESSION-06 review | `tests/repair_machinery.rs` | `age_mtime` encodes "skip" as a `bool`; a wrapper type would stop a future call site from ignoring it. | completed |
| F497 | TEST-OUTPUT-06 review | `README.md` | The `--output` row says "Stdout only; diagnostics stay on stderr" without mentioning that the log file carries the same rendered lines. | skipped (pre-existing gap, outside this batch's row; no test covers it) |
| F498 | TEST-OUTPUT-06 review | `tests/common/mod.rs` | The shared constant's doc hard-codes the emitting function name, which a future rename would leave stale. | skipped (prose-only nit; the emitter is stable and covered by cli_contract) |
| F499 | CHECK-07 review | `src/repair/phases/check.rs` tests | The new `CHECK fail: test double:` branch only runs on a genuine 10 s stall and has no test; feeding a `TestGateExpired` outcome through the warn site would pin it. | completed |
| F500 | CHECK-07 review | `src/repair/phases/check.rs` tests | The two harness waits repeat `before(deadline).unwrap_or_default()`; a named "remaining or zero" helper would make the zero-timeout intent explicit. | completed |
| F501 | PATHHELPERS-07 review | `README.md`, `docs/06-artifact-bridge-spec.md`, `tools/capture/README.md` | The other bridge recipes still append `--no-capture`, now disagreeing with the module doc; align them and mention the per-case override. | completed |
| F502 | PATHHELPERS-07 review | `tests/artifact_bridge.rs` | Dropping `--no-capture` also drops its serial execution; the bundle/temp isolation has not been confirmed under parallel runs. | completed |
| F503 | DOCS-USER-06 review | `docs/06-artifact-bridge-spec.md` | §4.6 keys the manifest cache by `(bundle, tag, field)` while the test server keys it by the archive `PathBuf`; equivalent, but the document names a key the code does not use. | completed |
| F504 | DOCS-USER-06 review | `docs/06-artifact-bridge-spec.md`, `README.md` | `GIRPR_ARTIFACT_ROOT` is honoured by `artifacts_root()` but is absent from the documented resolution order. | completed |
| F505 | DOCS-USER-06 review | `README.md`, `docs/06-artifact-bridge-spec.md` | "63 s for the six tests" is run wall time, not the sum of case durations; the wording could say "wall". | completed |
| F506 | HYP-07 review | `src/repair/phases/metadata.rs` | The private `logs_of` copy still has the older doc and no runtime-flavor guard; bring it in line with the shared helper. | completed |
| F507 | JOBS-06 review | `tests/cli_contract.rs` | `HELP_DESCRIPTION_INDENT` is documented as `HELP_OPTION_INDENT` + 4 but neither derived nor asserted, so the two can desync silently. | completed |
| F508 | LOGGING-06 review | `src/logging.rs` tests | The full-line assertion couples to `TryInitError` forwarding the inner `Display` verbatim; no stable public accessor compares `source()`. | skipped (no stable public API exists to assert the typed source) |
| F509 | LOGGING-06 review | `src/logging.rs` tests | `set_global_default(NoSubscriber::default())` builds a `Dispatch` only to elicit the error; a comment could note that. | skipped (purely cosmetic; the construct is idiomatic for eliciting the error) |
| F510 | LOCK-07 review | `tests/common/run_lock.rs` | The helpers tokenize rather than mirror clap's option arity, so `--include --game-path /x` can still be read as a game path. | completed |
| F511 | LOCK-07 review | `tests/run_lock.rs` | The CLI test proves a lone `-` parses but does not assert the parsed `game_path` equals `-`. | completed |
| F512 | INI-07 implementer | `src/util/path.rs` | `junction_refusal` treats any `mklink` failure as a host refusal and skips unless `GIRPR_TEST_REQUIRE_JUNCTION` is set; the same broken `cmd`/argument failure could skip silently. | completed |
| F513 | INI-07 implementer | `src/util/ini.rs` tests | The shared `link_creation_denied` comment still names `CreateSymbolicLinkW` specifically, though it now also covers junctions. | completed |
| F514 | INI-07 review | `src/util/ini.rs` | `try_junction_dir` treats any `symlink_metadata` error as "missing", so a permission or transient error is reported as a missing link instead of its own cause. | completed |
| F515 | INI-07 review | `src/util/ini.rs` | The raw-1314 branch of `junction_skip_reason` is effectively unreachable for junction creation, since `mklink` failures arrive as `Error::other`; document it or key on the failure signature. | completed |
| F516 | INI-07 review | `src/util/ini.rs` tests | The guard-retry test exercises a plain directory rather than a reparse point, so it does not cover link-type-specific removal. | skipped (deliberately host-independent; the existing link tests cover real removal) |
| F517 | BUILD-07 implementer | `tests/scripts/check-msrv.ps1` | The script already checks `$LASTEXITCODE` but reports a generic "cargo metadata failed"; the message could name the command and exit code like the workflow now does. | completed |
| F518 | BUILD-07 implementer | `Cargo.toml`, `tools/sync/Cargo.toml`, `tests/scripts/io-profile/iot-bench/Cargo.toml` | Comments still name `1.89`/`1.90` when documenting the redb 4.1 pin. | skipped (accurate today; they document the dependency pin, and only need a revisit if the MSRV moves) |
| F519 | BUILD-07 implementer | `tests/scripts/io-profile/iot-bench/Cargo.toml` | `iot-bench` is a standalone workspace with no `rust-version`; the MSRV workflow intentionally reads the root manifest. | skipped (intentional; the bench has no floor of its own) |
| F520 | BUILD-07 review | `README.md` | "The root package's `rust-version` ... for the whole workspace" is true only while no member exceeds the root's 1.89; a member floor above the root would make it stale. | completed |
| F521 | BUILD-08 implementer | `tests/scripts/check-msrv.ps1` | Other failures (toolchain install, member check) still omit exit codes, so they report less detail than the metadata failure now does. | completed |
| F522 | BUILD-08 implementer | `tests/scripts/check-msrv.ps1`, `.github/workflows/msrv.yml` | The same `cargo metadata` command line and message now live in both files; a flag change must update both. | skipped (inherent to the YAML/PowerShell split; the duplication is two short lines) |
| F523 | BUILD-10 implementer | `README.md` | The testing section names a concrete host toolchain (`clippy 0.1.99 / rustc 1.99.1`). | skipped (it records the measurement environment for the allowances, so it is intentionally concrete) |
| F524 | BUILD-10 review | `README.md` | "The root's is today's high-water mark" reads as uniquely highest, while `tools/sync` is tied at 1.89. | completed |
| F525 | BUILD-10 review | `tools/sync/Cargo.toml` | The pin comment says "supports the workspace's Rust 1.89", while the README now anchors the pin to `girsync`'s own minimum. | completed |
| F526 | BUILD-11 implementer | `tests/scripts/check-msrv.ps1` | The script relies on the host's `$PSNativeCommandUseErrorActionPreference`; if it is `$true`, native failures throw before the `$LASTEXITCODE` branches and the tailored messages are lost. | completed |
| F527 | BUILD-11 implementer | `tests/scripts/check-msrv.ps1` | The `rustup run ... rustc --version` probe treats any failure as a missing toolchain (e.g. a corrupt one) and then tries to install. | completed |
| F528 | BUILD-13 implementer | `tools/sync/Cargo.toml` | Line 5 still justifies the crate's `rust-version` with "redb 4.1 needs 1.89". | skipped (accurate and complementary to the reworded pin comment; no conflict) |
| F529 | BUILD-13 implementer | `README.md` | The root's requirement is attributed to `File::try_lock`/`File::unlock`, matching the root manifest comment; both need the same edit if that API's MSRV changes. | skipped (cross-file coupling is inherent; both places agree today) |
| F530 | BUILD-13 review | `README.md` | "pins `redb` to its 4.1 line" reads as the whole 4.1 series while the manifest pins exactly `=4.1.0`. | completed |
| F531 | BUILD-13 review | `Cargo.toml` | The root comment explains the 1.89 floor without noting `girsync` ties at 1.89. | completed |
| F532 | BUILD-13 review | `README.md`, `tools/sync/Cargo.toml` | The MSRV prose literals duplicate manifest and registry facts, and `check-msrv.ps1` derives from `cargo metadata` so it cannot catch prose drift. | skipped (the literals are deliberate and named by the surrounding prose; a metadata-driven check would not cover prose either) |
| F533 | TEMP-WRITER-07 implementer | `src/repair/file/temp.rs` | The copy-stage `ReuseCopy::ReadFailed` handle drop can only be tested end-to-end behind a reader/handle generic threaded through `ReuseCall`, `try_reuse_slice`, and the caller. | skipped (would change production types in three places; the helper-level pin is retained) |
| F534 | TEMP-WRITER-07 implementer | `src/lib.rs` | `terminal_retcode` is exposed only through `#[doc(hidden)] test_support` yet documented as public; a real public API would need `SophonNotFound` exported to restore the link. | skipped (conditional on making it a real public API) |
| F535 | TEMP-WRITER-07 review | `src/repair/file/temp.rs` tests | The new read-error test asserts sink emptiness only through `SeekRecording`, while sibling tests also check the on-disk length after a failed path. | completed |
| F536 | TEMP-WRITER-07 review | `src/hyp.rs` | The reword dropped the pointer to the blob retry policy; linking `crate::sophon::stream::retryable` would restore it. | skipped (`sophon` is a private module, so that path is not publicly linkable and rustdoc rejects it) |
| F537 | TEMP-PUBLISH-07 implementer | `src/util/atomic.rs` | `atomic::publish` checks only the target's final component for a Windows-trailing dot or space, while `contained_rel` checks every component. | completed |
| F538 | TEMP-PUBLISH-07 implementer | `src/repair/phases/post.rs` | `copy_publish_with` does not share the trailing dot/space rule, so on Unix a game file literally named `foo.` would still be copied to `res/foo.`. | completed |
| F539 | MANIFEST-08 implementer | `src/repair/file/inplace.rs` | The overlap failure path reads the file twice after the redo (final digest, then the rescan, which also digests); one `scan_reader` could answer both. | completed |
| F540 | MANIFEST-08 implementer | `src/repair/file/inplace.rs` | `verify_in_place` digests and then scans on mismatch; the scan is failure-only and predates the redo change, but one pass could serve both answers. | completed |
| F541 | MANIFEST-08 review | `src/repair/file/inplace.rs` | `redo_mismatch_is_terminal` rescans the whole file through `scan_reader`; only the overlapping ranges need re-checking. | completed |
| F542 | MANIFEST-08 review | `src/repair/file/inplace.rs` | `scan.dest_ok.iter().all(...)` assumes one verdict per chunk; a defensive length check like `verify_in_place`'s `.get(i)` would fail closed. | completed |
| F543 | PURGE-PERF-09 implementer | `src/util/counter.rs` | The test-only `Gate` is deliberately two-party (`arrived == 2`); N-party reuse would need generalizing. | skipped (conditional; no N-party fixture exists) |
| F544 | PURGE-PERF-09 implementer | `src/util/counter.rs` tests | The new regression test's simulated panic text reaches captured test output. | skipped (harmless; visible only in verbose runs) |
| F545 | PURGE-PERF-09 review | `src/util/counter.rs` | The panic-release path with a party actually parked at the gate (`arrived == 1`, `open == false`) is not directly exercised; current tests release an already-open gate. | completed |
| F546 | PURGE-PERF-09 review | `src/util/counter.rs` | `Gate::release`'s doc says repeating it is "a no-op", but it increments the diagnostic `releases` counter. | completed |
| F547 | PURGE-PERF-09 review | `src/util/counter.rs` | `release` uses `lock().unwrap()`; `unwrap_or_else(PoisonError::into_inner)` would make the drop immune to a poisoned mutex. | skipped (poisoning is unreachable through this code; robustness only) |
| F548 | TEST-REGRESSION-07 implementer | `src/lib.rs` | The test name `an_ago_stamp_keeps_its_age_on_either_side_of_the_epoch` does not mention the backward crossing now pinned. | completed |
| F549 | TEST-REGRESSION-07 implementer | `src/lib.rs` | The pre-epoch pin builds `UNIX_EPOCH - Duration::from_secs(60)` with unchecked subtraction, which would panic on a host without pre-epoch time. | completed |
| F550 | TEST-REGRESSION-07 implementer | `OPTIMIZATIONS_PLAN` gates | Positional nextest filters like `girpr lib` or `girpr machinery` select zero tests; `--lib` / `--test <name>` are the working spellings. | skipped (harness/nextest filter behaviour, not repo code) |
| F551 | TEST-REGRESSION-07 review | `tests/repair_machinery.rs` | `MtimeAge` is local; if another test file needs the same named skip outcome it should live beside the shared clock helpers. | skipped (conditional; only one test file uses it today) |
| F552 | CHECK-08 implementer | `src/repair/phases/check.rs` tests | The other warn-site arms (`PathEscaped`, `Unreadable`, `Read`, `Absent`) are only indirect; one injected-outcome test could pin all six arms. | completed |
| F553 | CHECK-08 implementer | `src/repair/phases/check.rs` tests | `remaining_or_zero` could also serve the release wait if its `Some`/`None` branch were refactored. | skipped (would change the release-wait shape) |
| F554 | CHECK-08 review | `src/repair/phases/check.rs` tests | `check_warnings` slices from `CHECK fail`, stripping the `WARN` level, so a regression from `warn!` to `info!` would still pass. | completed |
| F555 | CHECK-08 review | `src/repair/phases/check.rs` tests | `remaining_or_zero`'s doc restates `before`'s and can drift from it. | completed |
| F556 | PATHHELPERS-08 implementer | `README.md`, `docs/06-artifact-bridge-spec.md` | The default parallel bridge run (69.4 s here) is not faster than the recorded serial 63 s on this 4-core host; a timing claim should say which mode it measured. | skipped (the docs say "roughly a minute", which both measurements support) |
| F557 | PATHHELPERS-08 implementer | `tests/artifact_bridge.rs` | Parallel safety is stated in prose only; asserting that scratch paths differ would test the tempdir crate rather than the bridge. | skipped (the property belongs to the tempdir crate) |
| F558 | PATHHELPERS-08 implementer | `.config/nextest.toml` | The bridge override matches six test names without a binary qualifier, so a same-named test elsewhere would inherit `success-output = "immediate"`. | completed |
| F559 | PATHHELPERS-08 review | `tests/artifact_bridge.rs` | The module doc explains the no-flag run and the override but not the `-E 'test(...)'` narrowing the recipes now mention. | completed |
| F560 | PATHHELPERS-08 review | `README.md` | The top bridge command block has no cross-reference to the detailed note further down, so the two copies can drift. | completed |
| F561 | PATHHELPERS-08 review | `docs/06-artifact-bridge-spec.md` | The note cites §6.2 for both the scratch tree and the listeners, but the scratch tree is §6.3. | completed |
| F562 | DOCS-USER-07 implementer | `tests/artifact_bridge.rs` | The module docs still say "63 s for the six tests" and show a resolution order without `GIRPR_ARTIFACT_ROOT`, mirroring the old wording. | completed |
| F563 | DOCS-USER-07 implementer | `tests/artifact_bridge/server.rs` | The §4.6 comment says "eight per-capture bridges"/"eight runs" while the suite has six tests, and its size wording (~40 MB of ~130 MB) disagrees with the measured 97.7 MiB bundle and 41.4 MiB of manifests. | completed |
| F564 | DOCS-USER-07 review | `tests/artifact_bridge/server.rs` | Manifests are described as "~40 MB of a ~130 MB bundle" and "~320 MB across eight runs"; the bundle is 97.7 MiB with 41.4 MiB under `manifests/`, and there are six tests. | skipped (duplicate of F563, which now carries these measurements) |
| F565 | DOCS-USER-07 review | `docs/06-artifact-bridge-spec.md` | §3.2 step 4 says "otherwise a directory scan" though the scan runs only when `gircap list` is unavailable or fails, and step 5's `--out <root>` text covers the zero-bundle failure while the multiple-bundle failure points at `GIRPR_ARTIFACT_BUNDLE`. | completed |
| F566 | DOCS-USER-08 implementer | `tests/artifact_bridge/server.rs` | The comments say "a `getBuild` body declares five manifests and the bundle stores two", while §4.6 says "four manifests per run"; both can be true but read as a contradiction. | completed |
| F567 | DOCS-USER-08 implementer | `tests/artifact_bridge/server.rs`, `docs/06-artifact-bridge-spec.md` | The cache holds only the served `.manifest.zst` archives (~19.0 MiB), not the whole 41.4 MiB `manifests/` tree both texts cite for scale. | completed |
| F568 | DOCS-USER-08 implementer | `tests/artifact_bridge.rs` | The `#[ignore]` reason mentions the `.config/nextest.toml` success-output override. | skipped (the override exists and was exercised in the parallel bridge run) |
| F569 | DOCS-USER-08 review | `tests/artifact_bridge/server.rs` | The cache is a per-process `static`, and nextest gives each test its own process, so "shared by all of them — including across the six per-capture bridges" only holds in a single-process run. | completed |
| F570 | DOCS-USER-08 review | `tests/artifact_bridge/server.rs` | "every capture pays the disk read once instead of six times" mixes a per-capture subject with a suite-wide count. | completed |
| F571 | DOCS-USER-08 review | `tests/artifact_bridge/server.rs` | "the suite's six runs" counts test cases, but each case is a pre-pass plus a real run (twelve `girpr` invocations). | completed |
| F572 | HYP-08 implementer | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | The async `logs_of` now exists twice; consolidating needs a public async test helper. | skipped (would add public API surface for a private test helper) |
| F573 | HYP-08 review | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | "the future ... may be polled on worker threads" is over-broad: a multi-thread `#[tokio::test]` still polls the awaited body on the calling thread; only spawned tasks migrate. | completed |
| F574 | HYP-08 review | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | `spawn_blocking_traced` captures the dispatcher at spawn time, so on a multi-thread runtime the spawn site may already lack the subscriber; the "blocking lines are captured too" claim is scoped to correct usage. | completed |
| F575 | HYP-08 review | `src/repair/test_support.rs` | An async `logs_of` could live behind `cfg(test)`, but it would be unreachable from integration tests and still duplicated. | skipped (only worth revisiting if a third copy appears) |
| F576 | JOBS-07 implementer | `tests/cli_contract.rs` | `HELP_SHORT_OPTION_INDENT` likewise has no automated tie to clap's actual layout; only fixture-based tests exercise it. | completed |
| F577 | JOBS-07 implementer | `tests/cli_contract.rs` | No test asserts that a real `--help` description line begins at `HELP_DESCRIPTION_INDENT`; output is only checked through `help_entry`. | completed |
| F578 | LOCK-08 implementer | `tests/common/run_lock.rs` | The mirror does not model `--` or bare positionals; the helper could still name a game path seen before them. | skipped (the CLI has no positionals and every caller passes flags plus values only; lock removal is a no-op when wrong) |
| F579 | LOCK-08 implementer | `tests/common/run_lock.rs` | `args_take_lock` still tokenizes without arity, so invalid argv gets an arbitrary (no-op) answer. | skipped (its documented reason holds for every argv clap accepts) |
| F580 | LOCK-08 implementer | `tests/run_lock.rs` | A future value-taking flag using `allow_hyphen_values` or a short value form would need the table and rule revisited; the drift test covers arity and aliases only. | skipped (the drift test fails first if arity changes) |
| F581 | LOCK-08 review | `tests/common/run_lock.rs` | Invalid inline values (`--jobs=`, `--max-files=abc`) are not validated, so the helper can return a path for a list clap refuses. | skipped (clap refuses the argv, so the run never starts and there is no lock to remove) |
| F582 | LOCK-08 review | `tests/common/run_lock.rs` | `--` end-of-options and flag-conflict shapes are not mirrored. | skipped (same reachability argument as F578) |
| F583 | INI-08 implementer | `src/util/ini.rs`, `src/util/path.rs` | `try_junction_dir`, `junction_skip_reason` and the 1314 check now exist twice, in two `#[cfg(test)]` modules; a shared `util`-local test helper would remove the duplication without widening the `lib.rs` test seam. | completed |
| F584 | INI-08 implementer | `src/util/path.rs` | The path helper's doc omits the `%VAR%` caveat the ini helper documents: `cmd` expands percent pairs even inside quotes. | completed |
| F585 | INI-08 implementer | `src/util/ini.rs` | `link_creation_denied`'s Unix branch documentation is symlink-specific, though no Unix link variant shares it today. | skipped (conditional; only the symlink classifier compiles on Unix) |
| F586 | INI-08 review | `src/util/path.rs` | The exit-0-but-link-missing branch has no direct test; only its `ErrorKind::Other` class is covered through the non-zero-exit case, so a seam over `Output`/`ExitStatus` would pin it. | completed |
| F587 | INI-09 implementer | `src/util/ini.rs`, `src/util/path.rs` | `link_creation_denied` exists only in `ini.rs`; `path.rs` inlines the same 1314 test. | skipped (duplicate of F583, which covers sharing the 1314 check) |
| F588 | INI-09 implementer | `src/util/path.rs`, `src/util/ini.rs` | `junction_inspection_error` collapses the original error to `Error::other`, so its kind and raw code cannot be recovered. | skipped (deliberate; classification is fail-closed by contract) |
| F589 | INI-09 implementer | `src/util/path.rs`, `src/util/ini.rs` | The `%VAR%` expansion hazard for fixture paths is enforced only by a comment, not by rejecting `%` in the paths passed to `try_junction_dir`. | completed |
| F590 | INI-09 review | `src/util/ini.rs`, `src/util/path.rs` | `junction_skip_reason`'s doc says the raw-code arm "belongs to the spawn path", but spawning `cmd` never needs `SeCreateSymbolicLinkPrivilege`; "defensive" is the accurate framing. | completed |
| F591 | INI-09 review | `src/util/ini.rs`, `src/util/path.rs` | `try_junction_dir` verifies only that something exists at the link path, not that it is a reparse point, so a mis-created directory would be reported later with a less direct message. | completed |
| F592 | INI-10 implementer | `src/util/ini.rs` | `link_creation_denied`'s doc says it classifies "a directory symlink or a junction", but junction classification now lives in the shared module. | todo |
| F593 | INI-10 implementer | `.config/nextest.toml` | `config_bump_refuses_a_dangling_junction_game_dir` is not in the `success-output = "immediate"` override, so a future host skip there would be invisible under the default gate. | todo |
| F594 | INI-10 implementer | `src/util/ini.rs` | The ini junction test prints its skip manually while the path test routes through `skip_fixture_refusal`; the two styles still differ. | todo |
| F595 | INI-10 review | `src/util/ini.rs` | The ini junction test ignores `GIRPR_TEST_REQUIRE_JUNCTION` while the path test honours it, so ini's junction case can silently skip under the require switch. | todo |
| F596 | INI-10 review | `src/util/junction_support.rs` | The inner `mod tests` relies on the module's `#[cfg(all(test, windows))]` gate instead of carrying `#[cfg(test)]` like the rest of the crate. | skipped (redundant under the module gate; not a behaviour difference) |
| F597 | INI-10 review | `src/util/mod.rs` | Unix compilation of the `#[cfg(all(test, windows))]` gate was not exercised on this host. | skipped (gating is structurally sound; a Unix CI run confirms it) |
| F598 | BUILD-14 review | `tests/scripts/check-msrv.ps1` | The install-failure throw drops rustup's installer output, so mirror/network errors are silent. | todo |
| F599 | BUILD-14 review | `tests/scripts/check-msrv.ps1` | The absence regex matches any toolchain name, not the requested `$ver`, so a bad `rust-version` still triggers an install attempt. | todo |
| F600 | BUILD-14 review | `tests/scripts/check-msrv.ps1` | The script has no automated test; its branches were verified with throwaway `PATH` shims only. | todo |
| F601 | BUILD-16 review | `tools/sync/Cargo.toml` | The pin comment says "redb 4.1 needs 1.89", series wording next to the exact `=4.1.0` pin. | todo |
| F602 | BUILD-16 review | `README.md` | Registry-derived prose ("4.2 and later need 1.90") will drift with redb releases, and `check-msrv.ps1` reads only member `rust-version`s. | skipped (same as F532; prose literals are deliberate and not metadata-derivable) |
| F603 | TEMP-WRITER-08 implementer | `src/repair/file/temp.rs` tests | `a_reuse_copy_tells_a_read_error_from_a_short_slice` checks the copy outcome but not `out.bytes`/on-disk length after its `ReadFailed` and `Truncated` cases; the short-slice case writes a partial piece. | todo |
| F604 | TEMP-WRITER-08 implementer | `src/repair/file/temp.rs` tests | `a_missing_reuse_source_falls_back_to_download` asserts the recorder's `seeks`/`bytes` only, without the on-disk length its siblings use. | todo |
| F605 | TEMP-WRITER-08 review | `src/repair/file/temp.rs` tests | The new assertion message calls the on-disk length "in the sink" while the siblings phrase it as bytes landing in the sink. | skipped (cosmetic; all three assert the same exact length and the message states its intent) |
| F606 | TEMP-PUBLISH-08 implementer | `src/repair/phases/post.rs` | `move_audio_cache_with` tries `std::fs::rename` before the copy fallback, so on Unix a cache file named `foo.` can still be published as `res/foo.` without reaching the new refusal. | todo |
| F607 | TEMP-PUBLISH-08 implementer | `src/repair/phases/post.rs` | The copy alias refusal message omits the path; the caller's warn line supplies both paths. | skipped (informational only; the caller names both paths) |
| F608 | TEMP-PUBLISH-08 review | `src/repair/phases/post.rs` tests | The copy test's "no destination directory" assertion is vacuous because `copy_publish_with` never creates directories; an integration test through `move_audio_cache_with` would cover the real sequence. | todo |
| F609 | TEMP-PUBLISH-08 review | `src/util/path.rs`, `src/util/atomic.rs` | Windows verbatim targets (`\\?\…`) do not strip dot/space, so the lexical rule can refuse a spelling verbatim would accept. | skipped (conditional; no caller produces verbatim publish targets) |
| F610 | MANIFEST-09 implementer | `src/repair/file/inplace.rs` tests | No end-to-end test drives a real post-redo terminal mismatch through `verify_and_retry`; it would need a zstd origin whose redo still mismatches. | skipped (conditional; the unit test already covers the terminal decision) |
| F611 | MANIFEST-09 implementer | `src/repair/file/inplace.rs` | The verify success path hashes chunk ranges whose verdicts are unused, which is inherent to deciding the outcome in one pass. | skipped (inherent to the one-pass design; only worth revisiting if a profile shows it) |
| F612 | MANIFEST-09 review | `src/repair/file/inplace.rs` | The first verify now hashes the chunk ranges as well as the file (roughly twice the MD5 work per repaired file) to save one read on the rare mismatch path; the ignored `md5_throughput`/`scan_throughput` benchmarks could size it. | todo |
| F613 | MANIFEST-09 review | `src/repair/file/inplace.rs` | The "no overlap implies terminal" invariant now lives in the caller asking for no ranges, so a future caller handing the predicate a scan built from an empty range list for an overlapping layout would wrongly get `true`. | todo |
| F614 | MANIFEST-09 review | `src/repair/file/inplace.rs` tests | The post-redo tail is still covered only at unit level, never through `verify_and_retry`. | skipped (duplicate of F610; it would need a download mock for the redo) |
| F615 | MANIFEST-10 implementer | `src/repair/file/inplace.rs` | The post-redo pass could be narrowed further to the chunks intersecting a rewritten chunk (the `still_bad` list), a strict subset of the contested set; `read_after_redo` does not receive `still_bad` today. | todo |
| F616 | MANIFEST-10 implementer | `src/repair/file/inplace.rs` | `overlapping_ranges` re-derives the contested offsets from `Layout::overlaps` and allocates a `Vec` per call. | skipped (cost is negligible; once per attempt) |
| F617 | MANIFEST-10 implementer | `src/repair/file/inplace.rs` | The narrowing leans on `PlannedFile::layout()` skipping negative-offset chunks, so a hand-built plan with a negative offset would leave that chunk unchecked. | skipped (defence in depth; `validate_chunk_ranges` refuses such a plan twice) |
| F618 | MANIFEST-10 implementer | `src/repair/file/inplace.rs` | The terminal predicate reads exactly `expected` verdicts, so a scan returning more verdicts than asked for would go unnoticed. | todo |
| F619 | MANIFEST-10 review | `src/repair/file/inplace.rs` tests | The invariant test pins one fixed layout; a generated sweep over layouts would pin the narrowed-set property far more broadly. | todo |
| F620 | MANIFEST-10 review | `src/repair/phases/plan.rs` | The narrowing's soundness rests on `Layout::overlaps` naming both sides of every overlap; a unit test in `plan.rs` stating that property would make the dependency explicit. | todo |
| F621 | MANIFEST-10 review | `src/repair/file/inplace.rs` | `overlapping_ranges` recomputes `file.layout()` although `patch_chunks` already computed it. | skipped (negligible next to the read; not worth the churn) |
| F622 | PURGE-PERF-11 implementer | `src/util/counter.rs` | `Gate::arrive_and_wait` opens the gate only on the exact transition `arrived == 2`; a third arrival while the gate is closed would never open it. | skipped (unreachable today; arrivals increase by one, so the count passes through 2) |
| F623 | PURGE-PERF-11 implementer | `src/util/counter.rs` | `wait_for_arrivals` takes a count but has one caller, and `is_open` is derivable from the same lock as `arrival_count`; the helpers could fold into one snapshot accessor. | skipped (conditional; one caller today) |
| F624 | PURGE-PERF-11 implementer | `src/util/counter.rs` | `Gate::release` calls `notify_all` even when the gate was already open. | skipped (harmless with the current one-waiter shape) |
| F625 | PURGE-PERF-11 implementer | `src/util/counter.rs` | `ReleaseOnPanic` records no reason for the release, so a diagnostic consumer sees the count but not the cause. | skipped (conditional; no consumer needs the cause today) |
| F626 | PURGE-PERF-11 review | `src/util/counter.rs` tests | The new test pins "the party is parked" only indirectly through `release_count == 1`, which turned out racy under a gate that no longer blocks; a direct precondition check (`done_rx.try_recv()` is `Err` right after the parked-state assertions) would pin it deterministically. | todo |
| F627 | PURGE-PERF-11 review | `src/util/counter.rs` | The release doc says a repeat release "leaves the gate's state unchanged" although `GateState::releases` is incremented; naming the open/arrival state would be literally exact. | skipped (cosmetic; the next sentence discloses the count) |
| F628 | PURGE-PERF-11 review | `src/util/counter.rs` tests | No test pins that the gate actually blocks an unreleased party; with the wait loop disabled, three existing tests still pass. | todo |
| F629 | TEST-REGRESSION-08 implementer | `src/repair/file/inplace.rs` tests | The local `ago()` helper calls `stamp_ago(age).expect(...)`, so that unit test fails loudly instead of skipping on a host that cannot represent the age. | todo |
| F630 | TEST-REGRESSION-08 implementer | `src/lib.rs` | The ordinary-clock assertion's `now - day` is the only remaining unchecked subtraction in the clock tests. | todo |
| F631 | TEST-REGRESSION-08 implementer | `src/util/ini.rs` tests | Two fixtures still print their own `SKIP precondition:` lines instead of using the shared `skip_fixture_refusal` helper. | completed |
| F632 | TEST-REGRESSION-08 implementer | `README.md`, `.config/nextest.toml` | The skip-switch bullet and the `success-output` override only cover the two existing switches; a skip line from an unlisted test is invisible under the plain gate. | todo |
| F633 | TEST-REGRESSION-08 review | `.config/nextest.toml` | The renamed epoch test has no `success-output = "immediate"` override, so its skip line would be invisible under the plain gate, unlike every other skip-printing test. | todo |
| F634 | TEST-REGRESSION-08 review | `src/lib.rs`, `README.md`, `.config/nextest.toml` | The new pre-epoch skip has no presence-based switch, so a run that means to prove the backward crossing cannot insist on it. | todo |
| F635 | TEST-REGRESSION-08 review | `src/lib.rs` | The pre-epoch reference value is now the same expression the helper uses, so the assertion no longer pins "the platform's unchecked subtraction does not panic"; a short comment would stop a reader mistaking it for a tautology. | todo |
| F636 | CHECK-09 implementer | `src/repair/phases/check.rs` tests | `parallel_checks_match_one_job_and_stay_in_plan_order` still asserts the md5/size lines only by substring, and the new exact test now subsumes it. | todo |
| F637 | CHECK-09 implementer | `src/repair/phases/check.rs` | `CheckOutcome`/`CheckFailure` are not `Clone`, so the test helper rebuilds outcomes per rel instead of holding a shared table. | todo |
| F638 | CHECK-09 implementer | `src/repair/phases/check.rs`, `src/util/` | `check_one` still has no deterministic path to `PathEscaped`/`Unreadable`/`Read`; an injectable seam at `util::file_len`/`util::md5::md5_file_digest` could make those shapes reachable from `check_one`, not only by outcome injection. | todo |
| F639 | CHECK-09 implementer | `src/repair/phases/check.rs` tests | `check_warnings` strips the timestamp/level/target prefix, so no test can assert a full raw log line including the `WARN` level and the module target. | skipped (duplicate of F554, which CHECK-11 already covers) |
| F640 | CHECK-09 review | `src/repair/phases/check.rs` tests | The rel label `scanned_md5.dat` carries a chunk count while the run passes `scan = None`, so the label suggests the scanned digest path although the shape is injected directly. | todo |
| F641 | CHECK-09 review | `tools/capture/docs/00-spec.md` | The capture tool's spec shows an abbreviated log shape (no `file=` field) and cites the stale path `src/repair/check.rs`; the runtime prints `file=` first and lives in `src/repair/phases/check.rs`. | todo |
| F642 | CHECK-11 implementer | `src/repair/phases/check.rs` tests | The extractor drops a line whose prefix holds no parseable level, so a subscriber that stopped printing the level would shrink the list rather than fail loudly; a fallible extractor would make that explicit. | todo |
| F643 | CHECK-11 implementer | `src/repair/phases/check.rs` tests | The phase's other log line (`info!("check-only: total=… bad=…")`) is still asserted by content only, not by level. | todo |
| F644 | CHECK-11 implementer | `src/repair/phases/check.rs` tests | `tracing::Level`'s `FromStr` also accepts the numeric spellings 1-5 and is case-insensitive, so a prefix token "1" would parse as `ERROR`. | skipped (conditional; the subscriber's prefix has no such token today) |
| F645 | CHECK-11 implementer | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | Two more copies of the async log-capture implementation duplicate `src/repair/test_support.rs`. | skipped (duplicate of F572/HYP-09; consolidating needs a public async test helper) |
| F646 | CHECK-11 review | `src/repair/phases/check.rs` tests | The extractor scans the whole prefix for the first level-looking token, and `Level::from_str` also accepts bare numbers ("2" is `WARN`), so the parse relies on the level preceding the target rather than on its position. | todo |
| F647 | CHECK-11 review | `src/repair/phases/check.rs` tests | The extractor assumes one log record per line; a multi-line message would be split and its continuation dropped, and that assumption is undocumented. | todo |
| F648 | PATHHELPERS-10 implementer | `.config/nextest.toml` | The sibling override block names three host-skip tests without a binary qualifier, and those names are far more collision-prone than the bridge ones. | todo |
| F649 | PATHHELPERS-10 implementer | `README.md`, `docs/06-artifact-bridge-spec.md`, `tools/capture/README.md` | `cargo nextest list` silently lists nothing for `#[ignore]`d tests without `--run-ignored all`, so the documented filter-narrowing hint should say so. | todo |
| F650 | PATHHELPERS-10 implementer | `tests/` across `girpr`, `gircap`, `girsync` | Duplicate test leaf names already exist across three binaries, which is the mechanism that makes the unqualified override risky. | skipped (informational; qualifying the filter removes the consequence) |
| F651 | PATHHELPERS-10 review | `.config/nextest.toml` | The sibling override block still names three host-skip tests by bare `test(=...)` without a binary qualifier. | skipped (duplicate of F648, which PATHHELPERS-12 covers) |
| F652 | PATHHELPERS-10 review | `.config/nextest.toml` | `config_bump_refuses_a_dangling_junction_game_dir` is not covered by either `success-output = "immediate"` override, so a future host skip there would be invisible under the plain gate. | skipped (duplicate of F593, which INI-15 covers) |
| F653 | PATHHELPERS-11 implementer | `docs/06-artifact-bridge-spec.md` | §5's driver table intro cites §6.3 for the per-capture listeners, whose rationale is §6.2. | todo |
| F654 | PATHHELPERS-11 implementer | `tools/capture/README.md` | The bridge command there lacks the `cargo build -p gircap` line both the README and the spec carry, so a reader following only the capture tool's README gets a `gircap verify` skip instead of the precondition. | todo |
| F655 | PATHHELPERS-11 implementer | `README.md`, `tests/artifact_bridge.rs` | The "run in parallel" wording could be read as intra-process threads, although nextest runs each case in its own process. | skipped (noting only; the wording is correct as written) |
| F656 | PATHHELPERS-11 review | `README.md`, `tools/capture/README.md` | Those recipes describe the scratch tree and listeners without section numbers, unlike the spec and module doc which now cite §6.3 and §6.2. | todo |
| F657 | PATHHELPERS-11 review | `README.md`, `docs/06-artifact-bridge-spec.md`, `tools/capture/README.md` | The recipes show the filter only as a fragment next to the `--run-ignored only` command, so a reader who types just the fragment selects zero tests because nextest's default filter excludes the ignored cases. | todo |
| F658 | DOCS-USER-10 implementer | `tests/artifact_bridge.rs` | The module docs still summarise the order as "gircap list --root <root>, then a directory scan" without saying the scan is a fallback for an unavailable or failing listing. | todo |
| F659 | DOCS-USER-10 implementer | `docs/06-artifact-bridge-spec.md` | §3.2 step 3 says `gircap list` "reports one `bundle` record per bundle on stdout", naming only the text form although the bridge also accepts the JSON form's `"event":"bundle"`. | todo |
| F660 | DOCS-USER-10 implementer | `tests/artifact_bridge.rs` | `scan_for_bundles` is fixed-depth while `gircap`'s own consistency test scans recursively, so a bundle outside the fixed layout would be missed by the bridge's scan. | skipped (conditional; only bundles outside the documented layout are affected) |
| F661 | DOCS-USER-10 implementer | `tests/artifact_bridge.rs` | `gircap list` can exit non-zero (for example an unreadable root), in which case the bridge silently falls back to the scan and never surfaces the reason. | skipped (the fallback is the documented behaviour; surfacing it would change the bridge's contract) |
| F662 | DOCS-USER-10 review | `tests/artifact_bridge.rs` | `scan_for_bundles` has no comment stating the fixed-depth layout the spec now names, although the recursive listing can find a root one level deeper. | todo |
| F663 | DOCS-USER-11 implementer | `tests/artifact_bridge.rs`, `README.md` | Those two still say "~20 MB of real bundle bytes" where §3.1 now says "~19 MiB", so one figure appears in two unit systems. | completed |
| F664 | DOCS-USER-11 implementer | `tools/capture/README.md`, `tools/capture/docs/03-design-notes.md`, `tools/capture/src/manifest/fetch.rs` | They still quote the four archives as "~20 MB" and `manifests/` as "≈41 MiB" without the served-archives split §4.6 now draws. | todo |
| F665 | DOCS-USER-11 implementer | `tests/artifact_bridge/server.rs` | The cache comment says the cache is shared across the six per-capture bridges and that every capture pays the read once, which holds for `cargo test` but not for nextest's one-process-per-test. | skipped (duplicate of F569 and F570, which DOCS-USER-13 covers) |
| F666 | DOCS-USER-11 review | `tools/capture/README.md` | Its directory breakdown does not reproduce on this bundle: `chunks/` is 15.83 MiB (says ≈17 MiB) and `expected/` is 18.66 MiB (says ≈20 MiB). | todo |
| F667 | DOCS-USER-11 review | `docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge/server.rs` | "Over a hundred megabytes across the suite's six runs" is loose: the suite runs six cases, each a pre-pass plus a real run, so the real figure is ~228 MiB; §4.6 and §6.2 carry the same "the suite's runs" wording. | todo |
| F668 | DOCS-USER-11 review | `docs/06-artifact-bridge-spec.md`, `tests/artifact_bridge/server.rs` | The sidecars are named `.meta.json`/`.paths.tsv` in the new text while the files are `<field>.manifest.meta.json`/`<field>.paths.tsv`. | skipped (consistent with the repo's existing `manifests/*/*.meta.json` shorthand) |
| F669 | DOCS-USER-13 implementer | `tests/artifact_bridge/server.rs` | `tests/artifact_server.rs` compiles the same server module, so it holds a second, independent cache; the comment's "one static for the whole process" does not spell out that the two binaries never share one. | todo |
| F670 | DOCS-USER-13 review | `docs/06-artifact-bridge-spec.md` | §4.6 and §6.2 still describe the cache in the old terms ("over a hundred megabytes", "shared by every listener") without the runner distinction the code comment now carries. | todo |
| F671 | DOCS-USER-13 review | `tools/capture/docs/00-spec.md` | It says "the scope held across all six runs", the same six-cases-versus-twelve-runs slip in a different document. | todo |
| F672 | DOCS-USER-13 review | `docs/06-artifact-bridge-spec.md` | A sharper figure than "over a hundred megabytes" would match the twelve-run count the batch introduced. | skipped (duplicate of F667, which DOCS-USER-18 covers) |
| F673 | HYP-10 implementer | `src/repair/mod.rs` | Nothing documents at `spawn_blocking_traced`'s definition that it must be called from the thread carrying the dispatcher; a future refactor into a spawned task would silently lose its log lines. | todo |
| F674 | HYP-10 implementer | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | The two async `logs_of` copies are still duplicated. | skipped (duplicate of F572 and F645; consolidating needs a public async test helper) |
| F675 | HYP-10 implementer | `src/repair/scan.rs`, `src/repair/phases/check.rs`, `src/repair/mod.rs` | Raw `tokio::task::spawn_blocking` calls produce lines no scoped subscriber can capture. | skipped (conditional; only matters if a test ever needs those lines) |
| F676 | HYP-10 review | `src/repair/phases/metadata.rs`, `tests/common/mod.rs` | Neither `logs_of` doc states the converse of the new claim: under a current-thread runtime a `tokio::spawn`ed task runs on the same thread and therefore keeps the subscriber. | todo |
| F677 | JOBS-08 implementer | `tests/cli_contract.rs` | `-h` (short help) renders a different layout, with descriptions beside the flags, and nothing in the suite pins it. | todo |
| F678 | JOBS-08 implementer | `tests/cli_contract.rs` | The new tests sample one option line and one description line; a sweep over every entry would pin each column. | todo |
| F679 | JOBS-08 implementer | `tests/cli_contract.rs` | `option_body` requires the short alias to be exactly one char followed by ", ", so a clap change to the separator would silently stop matching rather than fail loudly. | todo |
| F680 | JOBS-08 review | `tests/cli_contract.rs` | The `real_help()` doc comment says "they all read the same capture", which is true under `cargo test` but not under nextest's process-per-test. | todo |
| F681 | JOBS-08 review | `tests/cli_contract.rs` | The description test pins the wording "Game install directory", so rewording `--game-path`'s doc comment would break a layout test. | todo |
| F682 | JOBS-08 review | `tests/cli_contract.rs` | `help_entry_finds_the_help_option_behind_its_short_alias` spawns its own `--help` instead of reusing `real_help()`, so the file can spawn the binary twice per process. | todo |
| F683 | INI-12 implementer | `src/util/junction_support.rs` | `junction_inspection_error`'s `PermissionDenied` branch ("could not be inspected") still has no direct end-to-end drive; the seam could inject the inspection error. | completed |
| F684 | INI-12 implementer | `src/util/ini.rs` | `util::ini` uses the same shared helper but not the new seam, so its equivalent branch is still only covered indirectly. | completed |
| F685 | INI-12 review | `src/util/path.rs`, `src/util/ini.rs`, `src/util/junction_support.rs` | The two callers classify differently: path uses a local `JunctionFixture`/`classify_junction_fixture` while ini still uses an inline `match`, which is the drift the shared module exists to prevent. | completed |
| F686 | INI-12 review | `src/util/path.rs` | The containment case's panic arm is still only driven indirectly; driving it directly needs the skip/panic decision lifted into a shared helper. | todo |
| F687 | INI-12 review | `OPTIMIZATIONS_PLAN/GUIDE.md` | The commit-shape convention asks for an `opt(<task-id>):` prefix while another rule forbids a task id in commit messages. | skipped (the no-task-id rule wins; the commit message satisfies it) |
| F688 | INI-13 implementer | `src/util/junction_support.rs` | `reject_percent_path` renders components lossily, which matches what `cmd` receives; the coupling between the check and the command deserves a one-line note in the doc. | todo |
| F689 | INI-13 implementer | `src/util/junction_support.rs` | A host whose temp path contains a percent can no longer run the junction fixtures at all; a future relaxation could allow a percent that provably cannot form a pair. | skipped (conditional; failing loudly with the offending path named is the intended trade) |
| F690 | INI-13 review | `tests/` | Nothing pins cmd's per-argument pairing, so the corrected doc claim rests on measurement rather than a test; a probe could keep it honest. | todo |
| F691 | INI-13 review | `src/util/junction_support.rs` | `junction_command` spells `Result<String, std::io::Error>` where the file uses `std::io::Result<String>`, and an assertion message claims more than the flag proves. | todo |
| F692 | INI-14 implementer | `src/util/junction_support.rs` | `tools/sync` notes that `FileType::is_symlink` was false for a junction on some toolchains, so the new reparse-point check relies on std's name-surrogate reparse-tag test alone and could refuse a good fixture on an older toolchain. | todo |
| F693 | INI-14 implementer | `src/util/junction_support.rs` | `junction_inspection_error`'s `PermissionDenied` branch still has no direct end-to-end drive. | skipped (duplicate of F683, which INI-17 covers) |
| F694 | INI-14 review | `src/util/junction_support.rs` | The guard proves "is a name-surrogate reparse point", not "is a junction": a symbolic link at the link path would also pass, and std exposes no reparse tag publicly. | todo |
| F695 | INI-15 implementer | `src/util/path.rs`, `src/util/junction_support.rs` | `path.rs` keeps private copies of `require_junction()` and `skip_junction()`; the junction fixture support could have one home in the shared module. | todo |
| F696 | INI-15 implementer | `.config/nextest.toml` | The ini symlink case's host skip is still invisible under the plain gate; it is in no `success-output` override even though it can skip on a host without symlink privilege. | todo |
| F697 | INI-15 implementer | `src/util/ini.rs`, `src/util/path.rs` | The injected-refusal probes print "simulated refusal" to stderr but are covered by no `success-output` override. | skipped (consistent with the path module's equivalents; the output is diagnostic only) |
| F698 | INI-15 review | `src/util/path.rs`, `src/util/ini.rs`, `src/util/junction_support.rs` | Both test modules keep a private `skip_junction` wrapper with identical arguments; the wrapper could live in the shared module once. | todo |
| F699 | INI-15 review | `src/lib.rs` | `skip_fixture_refusal`'s panic message says "this host cannot create …" even for `FixtureRefusal::Injected`, where no host was consulted; the printed skip line distinguishes the two but the panic does not. | todo |
| F700 | INI-15 review | `.config/nextest.toml` | The first override (`test(=a_non_utf8_audio_cache_name_moves_*)`) is still unqualified by package/binary. | skipped (duplicate of F648, which PATHHELPERS-12 covers) |
| F701 | INI-16 implementer | `src/util/ini.rs` | `link_creation_denied` and `symlink_skip_reason` are Unix-relevant but live beside the extracted junction classifier; a symlink home for them would need a different module or a widened gate. | todo |
| F702 | INI-16 implementer | `tools/sync/tests/symlink.rs` | That crate has its own `skip_if_no_link!` macro with a different skip-line format; the girpr test-support seam is not reachable from girsync's integration tests. | skipped (separate crate; a shared seam would need its own design) |
| F703 | INI-16 implementer | `src/util/ini.rs` | The shared helper's panic message reads "this host cannot create …" even for `FixtureRefusal::Injected`. | skipped (duplicate of F699, which TEST-REGRESSION-12 covers) |
| F704 | INI-16 implementer | `src/lib.rs` | `skip_pre_epoch` prints its own `SKIP precondition:` line, the last self-printed one in the crate; the shared fixture helper does not fit a clock subject. | todo |
| F705 | INI-16 review | `src/util/path.rs`, `src/repair/scope/report.rs` | The `#[cfg(unix)]` symlink fixtures `unwrap()` the symlink creation, so on a Unix host that refuses symlinks they panic instead of skipping; they need their own skip reason and a `GIRPR_TEST_REQUIRE_SYMLINK` path. | todo |
| F706 | INI-16 review | `src/util/ini.rs` | The Unix `link_creation_denied` treats `EPERM`/`EACCES` as a host refusal, but the `Unsupported`-means-skip rule leans on std's platform mapping, which could widen and let a broken environment skip. | todo |
| F707 | INI-17 implementer | `src/util/junction_support.rs` | The classification's `HostRefused` arm is reachable only from a real spawn-level refusal, so unlike `Broken` it cannot be driven through the seam; the callers' injected-refusal cases cover the skip path separately. | todo |
| F708 | INI-17 review | `src/util/ini.rs`, `src/util/path.rs` | The new broken-fixture cases re-implement the caller's own `match classify_junction_fixture(..)` instead of driving the caller's decision code, so a caller that flipped the `Broken` arm to skip would still leave them green; lift the decision into one shared helper (the goal F686 also names). | todo |
| F709 | INI-17 review | `src/util/junction_support.rs` | `junction_skip_reason` is still `pub(super)` although no sibling module imports it any more; it could be a private `fn`. | todo |

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
| TEST-OUTPUT-05 | F401, F402, F403 | done | Wave E; remaining text-plane vocabulary notes; low; 9eaefe7, f74ec88, 73374f5 |
| CHECK-06 | F404, F405, F406 | done | Wave E; gate-test failure reporting and deadline unification; low; 3dc4247 |
| PATHHELPERS-06 | F407, F408 | done | Wave E; simulated-skip wording and bridge skip visibility; low; 9488a01, 2edf394 |
| DOCS-USER-05 | F409, F410, F411 | done | Wave E; bridge recipe alignment and profile wording; low; 1df38da, 002e9c7, ea33eda |
| DOCS-COMMENTS-05 | F412, F413 | done | Wave E; panic-audit region list and sweep vocabulary; low; f1921c2, 1be83dc |
| HYP-06 | F414, F415 | done | Wave E; shared log-helper wording and duplication decision; low; 8be8980 |
| METADATA-06 | F416, F417, F418 | done | Wave E; branch-failure wording and shared terminal predicate; low; 211a332, ebb59d8 |
| MANIFEST-06 | F419, F420 | done | Wave E; slow md5-gate test and server duplication; low; ea8b9f0 |
| JOBS-05 | F421, F422, F423 | done | Wave E; help_entry fixture constants and boundary edges; low; 976e65e |
| LOGGING-05 | F424, F425 | done | Wave E; global-subscriber test preconditions; low; 9376cc6, 922053c |
| LOCK-06 | F426, F427, F428 | done | Wave E; duplicate game-path, shared-module test volume, empty key; low; 27e28b8, 7b690c9 |
| INI-06 | F429, F430, F431, F432, F433 | done | Wave E; junction/symlink test cleanup and platform broadenings; low; f5135f1, 9426a7e, c4cf9bf, b4c6098 |
| BUILD-06 | F434, F435, F436 | done | Wave E; MSRV pin derivation and cache keying; low; 0f466e0 |
| BUILD-TOOLS-06 | F437 | done | Wave E; capture comment wording; low; 0fd58ab |
| TEMP-WRITER-06 | F438, F439, F440 | done | Wave E; dual-mode sink doc, read wrapper, reuse builder; low; 683e227, 63004a5, 53cf1be, d20aeb6 |
| TEMP-PUBLISH-06 | F441, F442, F443 | done | Wave E; publish guard spelling and doc sync; low; 5e8075b, 9546194, b0d5c3c, 002dffa |
| TEST-MANIFEST-06 | F444, F445, F446 | done | Wave E; suffix contract, seam doc, helper placement; low; 0dd1bbe, a9f4555, 9c1c400 |
| PURGE-PERF-08 | F447, F448, F449, F450 | done | Wave E; stub status variant, shared hang-guard constant, thread release, module order; low; 798adc5, 952b7f3, 6c4dd61, 29db786 |
| TEST-REGRESSION-06 | F451, F452 | done | Wave E; pre-epoch fixture ageing and shared stamp helpers; low; 562af2a, 90fb7c5, 9d256ab, 82b8942, c668aa6 |
| TEST-OUTPUT-06 | F453, F454 | done | Wave E; shared scan-bytes key and README layout; low; 7e8c04d, a7d9834 |
| CHECK-07 | F455, F456, F457 | done | Wave E; gate-test expiry wording and deadline comment; low; df3f4c4, 5701bd8, 1a0fb73 |
| PATHHELPERS-07 | F458, F459, F460 | done | Wave E; bridge flag docs, stale config comment, refusal dedup; low; ccaa79e, 5680206 |
| DOCS-USER-06 | F461, F462, F463 | done | Wave E; bridge recipe unification and cost provenance; low; 3a83504, 81d5f4b, 4157326 |
| DOCS-COMMENTS-06 | F464 | done | Wave E; sweep cast wording; low; da468d2 |
| HYP-07 | F465 | done | Wave E; multi-thread runtime caveat for the log helper; low; 74db398 |
| MANIFEST-07 | F466, F467, F468 | done | Wave E; digest-retry classification, gated-origin candidates, gate-message pin; low; 6192197, 7001ec7, f2966cc |
| JOBS-06 | F469, F470, F471 | done | Wave E; help_entry panic pin, CRLF short-alias fixture, indent constant; low; d3af585, d7c4b28, c989e28 |
| LOGGING-06 | F472, F473 | done | Wave E; lock doc scope and typed rejection pin; low; e3a3cf3, b79ef56 |
| LOCK-07 | F474, F475 | done | Wave E; run-lock cleanup hardening and module doc; low; 1d525d5, e5cde0b, 07c92d9 |
| INI-07 | F476, F477 | done | Wave E; junction skip classification and guard removal order; low; 8483c89, d3dba86 |
| BUILD-07 | F478, F479, F480 | done | Wave E; metadata failure message, MSRV literals, installer helper threshold; low; a6d34b0, 3b29885, e34a8b2 |
| TEMP-WRITER-07 | F481, F482, F483 | done | Wave E; end-to-end read-error seam, reader fixture, broken doc link; low; 8fab9f8, a60f278 |
| TEMP-PUBLISH-07 | F484, F485, F486 | done | Wave E; shared spelling guard, Windows aliases, doc wording; low; 3766e53, 6ac80bd |
| TEST-MANIFEST-07 | F487 | done | Wave E; capture suffix note consistency; low; bf0ad72 |
| MANIFEST-08 | F489, F490 | done | Wave E; overlap-guard precision and end-to-end overlapping fixture; low; ff1dcef, ff2e6b7, cd7afbd |
| TEST-MANIFEST-08 | F491, F492 | done | Wave E; suffix-note cross-reference and client-coverage wording; low |
| PURGE-PERF-09 | F493, F494 | done | Wave E; release-guard race and the remaining wait constant; low; ae55ba3 |
| TEST-REGRESSION-07 | F495, F496 | done | Wave E; post-epoch pin and skip-result type; low; f6da824, 5556cc2 |
| TEST-OUTPUT-07 | F497, F498 | done | Wave E; output-row log-file mention and constant doc; low |
| CHECK-08 | F499, F500 | done | Wave E; test-double warn pin and a named remaining-time helper; low; 753f063, c90d1c0 |
| PATHHELPERS-08 | F501, F502 | done | Wave E; remaining bridge recipes and parallel-run isolation; low; 5c8839a |
| DOCS-USER-07 | F503, F504, F505 | done | Wave E; cache-key wording, artifact-root variable, wall-time wording; low; 2bc0239, 2b3d76b, ae20679, dcc5874 |
| HYP-08 | F506 | done | Wave E; align the metadata log helper with the shared one; low; f53e096 |
| JOBS-07 | F507 | done | Wave E; assert the help indent relationship; low; 81cd638 |
| LOGGING-07 | F508, F509 | done | Wave E; typed-rejection coupling and dispatch-construction note; low |
| LOCK-08 | F510, F511 | done | Wave E; clap arity mirroring and lone-dash assertion; low; c93137b, f3a507e |
| INI-08 | F512, F513 | done | Wave E; path junction refusal classification and shared comment; low; 2cd7789, a411cc3 |
| INI-09 | F514, F515 | done | Wave E; junction metadata error cause and unreachable skip branch; low; cc7ba1f, b2634eb |
| INI-10 | F583, F584 | done | Wave E; shared junction test helper and `%VAR%` caveat; low; 1a822ca |
| INI-11 | F585 | done | Wave E; Unix link-denial wording; low |
| INI-12 | F586 | done | Wave E; pin the link-missing junction branch; low; 8712cf3 |
| INI-13 | F589 | done | Wave E; guard the percent-expansion hazard; low; 7021c14, 51eee2c |
| INI-14 | F590, F591 | done | Wave E; 1314 wording and reparse-point verification; low; e44667e, 88da409 |
| INI-15 | F592, F593, F594, F595 | done | Wave E; ini junction skip visibility and link-denial doc; low; bbd537a, e2ab3b4, 2a15ed1, 430566d |
| BUILD-20 | F598, F599 | todo | Wave E; install-failure output and version-scoped absence test; low |
| BUILD-21 | F600 | todo | Wave E; shim harness for the MSRV script; low |
| BUILD-22 | F601 | todo | Wave E; exact redb version in the pin comment; low |
| TEMP-WRITER-10 | F603, F604 | todo | Wave E; on-disk assertions for the remaining fallback paths; low |
| TEMP-WRITER-11 | F605 | done | Wave E; assertion-message wording; low |
| TEMP-PUBLISH-09 | F606, F607, F608 | todo | Wave E; guard the rename target against alias spellings and cover it end to end; low |
| BUILD-08 | F517 | done | Wave E; name the command and exit code in the MSRV script failure; low; d4bc74f |
| BUILD-09 | F518, F519 | done | Wave E; MSRV pin comments and standalone bench floor; low |
| BUILD-10 | F520 | done | Wave E; make the README workspace floor robust to a member above the root; low; 7c87314 |
| BUILD-11 | F521 | done | Wave E; exit codes for the remaining MSRV script failures; low; 24ec32c |
| BUILD-12 | F523 | done | Wave E; concrete toolchain in the testing section; low |
| BUILD-13 | F524, F525 | done | Wave E; high-water-mark wording and pin-comment consistency; low; 7b28bdb, bd94e50 |
| BUILD-14 | F526, F527 | done | Wave E; native-command preference and probe failure classification; low; 3927ccf, 67606f6 |
| BUILD-15 | F528, F529 | done | Wave E; pin justification and MSRV attribution coupling; low |
| BUILD-16 | F530, F531 | done | Wave E; 4.1.0 pin wording and root MSRV tie; low; 279628e, d5935af |
| BUILD-17 | F532 | done | Wave E; MSRV prose literal duplication; low |
| BUILD-18 | F533, F534 | done | Wave E; copy-stage read-error seam and doc-hidden export note; low |
| TEMP-WRITER-08 | F535 | done | Wave E; on-disk length after the failed read path; low; 3e38b4c |
| TEMP-WRITER-09 | F536 | done | Wave E; retry-policy pointer in the rewording; low |
| TEMP-PUBLISH-08 | F537, F538 | done | Wave E; alias-shaped directories and copy-path trailing dot/space; low; 5ce79d6, 451a1f4, 4727eb2 |
| MANIFEST-09 | F539, F540 | done | Wave E; single-pass digest and range scan on failure paths; low; 1481a45, 98b58de |
| MANIFEST-10 | F541, F542 | done | Wave E; overlap-only rescan and verdict-length guard; low; a5b0d6f, df5b20f, 6b22854 |
| MANIFEST-11 | F612 | todo | Wave E; measure the one-pass hashing trade; low |
| MANIFEST-12 | F613, F618 | todo | Wave E; keep the terminal rule's no-overlap invariant with the predicate; low |
| MANIFEST-13 | F615 | todo | Wave E; narrow the post-redo pass to the rewritten chunks; low |
| MANIFEST-14 | F619, F620 | todo | Wave E; pin the overlap property from both sides; low |
| PURGE-PERF-13 | F622, F623, F624, F625 | done | Wave E; gate helper limits and diagnostics; low |
| PURGE-PERF-14 | F626, F628 | todo | Wave E; pin the gate's blocking contract directly; low |
| TEST-REGRESSION-11 | F629, F630, F635 | todo | Wave E; panic-free clock fixtures; low |
| TEST-REGRESSION-12 | F699, F704 | todo | Wave E; skip reporting wording in the shared helper; low |
| INI-16 | F631 | done | Wave E; use the shared skip helper in the ini fixtures; low; 3cff9b3 |
| INI-17 | F683, F684 | done | Wave E; drive the uninspectable-link branch for both callers; low; 3a22365 |
| INI-18 | F686, F695, F698, F701, F707, F708 | todo | Wave E; one classification shape and a directly driven panic arm; low |
| INI-23 | F709 | todo | Wave E; narrow the shared classifier's unused visibility; low |
| INI-19 | F688 | todo | Wave E; note the lossy-render coupling in the percent guard; low |
| INI-20 | F690, F691 | todo | Wave E; pin cmd's pairing and tidy the percent guard; low |
| INI-21 | F692, F694 | todo | Wave E; reparse-point check across toolchains; low |
| INI-22 | F705, F706 | todo | Wave E; Unix symlink fixture skips and classifier framing; low |
| DOCS-USER-14 | F632, F633, F634 | todo | Wave E; document and surface the skip switches; low |
| CHECK-12 | F636, F637, F640 | todo | Wave E; exact parallel assertions and cloneable outcomes; low |
| CHECK-13 | F638 | todo | Wave E; reachable failure shapes from check_one; low |
| DOCS-USER-15 | F641 | todo | Wave E; capture spec log shape and path; low |
| CHECK-14 | F642, F643, F646, F647 | todo | Wave E; fallible level extraction and the info line's level; low |
| PATHHELPERS-12 | F648, F649, F657, F696 | todo | Wave E; qualify the sibling override and document the ignored-test filter; low |
| PATHHELPERS-13 | F654 | todo | Wave E; capture README bridge command completeness; low |
| DOCS-USER-16 | F653, F656 | todo | Wave E; spec section citation for the listeners; low |
| DOCS-USER-17 | F658, F659, F662 | todo | Wave E; bundle-discovery wording in the module doc and the spec; low |
| DOCS-USER-18 | F664, F666, F667, F671 | todo | Wave E; harmonise the bundle-size figures; low |
| DOCS-USER-19 | F669, F670 | todo | Wave E; cache scope wording in the spec and the comment; low |
| HYP-12 | F673, F676 | todo | Wave E; document the spawn_blocking_traced contract; low |
| PURGE-PERF-10 | F543, F544 | done | Wave E; gate arity and captured panic text; low |
| PURGE-PERF-11 | F545, F546 | done | Wave E; release-wakes-a-parked-party test and doc wording; low; 41b80d5, b0c9947 |
| PURGE-PERF-12 | F547 | done | Wave E; poisoned-mutex robustness; low |
| TEST-REGRESSION-08 | F548, F549 | done | Wave E; pin name and checked pre-epoch construction; low; b7bbfbf, 30f4b7f |
| TEST-REGRESSION-09 | F550 | done | Wave E; nextest gate spellings; low |
| TEST-REGRESSION-10 | F551 | done | Wave E; shared skip-outcome type; low |
| CHECK-09 | F552 | done | Wave E; pin the remaining check warn arms; low; 63bd985 |
| CHECK-10 | F553 | done | Wave E; release-wait helper reuse; low |
| CHECK-11 | F554, F555 | done | Wave E; pin the warn level and trim the helper doc; low; 3f7f1e0, e0bf1fe |
| PATHHELPERS-09 | F556, F557 | done | Wave E; timing-mode wording and tempdir prose; low |
| PATHHELPERS-10 | F558 | done | Wave E; qualify the bridge override filter; low; 4fdce0e |
| PATHHELPERS-11 | F559, F560, F561 | done | Wave E; module-doc filter, README cross-reference, section citation; low; 38cd738, 9c1a9a3, d17517c |
| DOCS-USER-08 | F563 | done | Wave E; bridge module docs and server comment accuracy; low; df90dfc |
| DOCS-USER-09 | F564 | done | Wave E; duplicate of F563; low |
| DOCS-USER-10 | F565 | done | Wave E; bundle-search failure wording; low; 236cffa, f6a3777 |
| DOCS-USER-11 | F566, F567 | done | Wave E; manifest-count wording and served-archive scale; low; 0fb4cc4, 503b823, cd6ae9b |
| DOCS-USER-12 | F568 | done | Wave E; ignore-reason override reference; low |
| DOCS-USER-13 | F569, F570, F571 | done | Wave E; cache comment scope and count wording; low; 2a932cc |
| HYP-09 | F572 | done | Wave E; duplicated async log helper; low |
| HYP-10 | F573, F574 | done | Wave E; runtime-flavor doc scoping in both log helpers; low; 9f68fd5 |
| HYP-11 | F575 | done | Wave E; async helper placement; low |
| JOBS-08 | F576, F577 | done | Wave E; real help output indent pins; low; 43d336b, 2c0f279 |
| JOBS-09 | F677, F678, F679, F680, F681, F682 | todo | Wave E; short-help layout and full help sweep; low |
| LOCK-09 | F578, F579, F580 | done | Wave E; argv terminator, lock-scan arity, hyphen values; low |
| LOCK-10 | F581, F582 | done | Wave E; inline-value validation and conflict shapes; low |
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
