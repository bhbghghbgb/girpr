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
| F32 | RC-06 review | `tests/repair_machinery.rs` | No end-to-end test mutates a file between plan and repair to prove the fresh-scan fallback. | todo |
| F33 | RC-06 review | `src/repair/file/inplace.rs` | The reuse debug log's `age_secs` is the file's mtime age, not the scan snapshot's age. | completed |
| F34 | RC-06 review | `src/repair/scan.rs`, `src/repair/file/inplace.rs` | Coarse-mtime filesystems can evade the exact length+mtime guard; document or harden. | completed |
| F35 | RC-07 review | `src/repair/lock.rs` | `Drop` unlocks then deletes, a narrow Unix race; leave the file or re-verify after acquire. | completed |
| F36 | RC-07 review | `src/repair/lock.rs` | The temp lock is opened by name and truncated; a pre-created symlink could truncate a user file. | completed |
| F37 | RC-07 review | `src/repair/lock.rs` | `key_for` hashes `to_string_lossy()`; distinct non-UTF-8 paths can collide. | completed |
| F38 | RC-07 review | `src/repair/purge.rs`, `docs/01-03` | Comments and docs still describe the in-install `girpr.lock` and keep-set wording. | todo |
| F39 | RC-07 review | `README.md`, `Cargo.toml` | README claims Rust 1.85+, but `File::try_lock` needs 1.89; no `rust-version` pin. | completed |
| F40 | RC-07 review | `src/cli.rs` | `excessive_jobs_warning` says workers can fall below file tasks, but the runtime is sized to `max(parallelism, jobs+1)`. | completed |
| F41 | RC-07 review | `src/main.rs`, `src/repair/**`, `tools/sync`, `docs/02-06` | Stale `--io-threads` prose remains after the `--jobs` rename. | todo |
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
| F53 | T21 implementer | `tests/cli_contract.rs` | No integration assertion that a real binary run emits exactly one `SUMMARY`. | todo |
| F54 | T22 implementer | `src/logging.rs` | The flush handle is a `OnceLock` for the first sink only; a second `init_tracing` is unflushable. | completed |
| F55 | T22 implementer | `src/logging.rs` | The flush-failure stderr branch has no test (stderr capture is process-level). | completed |
| F56 | RC-10 review | `src/repair/scan.rs` | `ScanReport` carries no plan identity; index alignment silently mismatches a different plan. | completed |
| F57 | RC-10 review | `src/repair/phases/plan.rs` | `FileId` derived `Debug` prints `FileId(0)` rather than the index as the task asked. | completed |
| F58 | T27 implementer | `src/repair/phases/plan.rs`, `src/repair/scan.rs` | `RepairPlan::files` and `ScanReport::files` stay public, so callers can bypass the id accessors. | completed |
| F59 | T27 implementer | `src/repair/scope/report.rs` | `DiskPlan::for_id` returns a default for an out-of-range id; `PlanDocument` uses raw indices. | completed |
| F60 | RC-11 review | `src/util/atomic.rs` | Module docs link a `pub(crate)` `temp_sibling`, which may warn under `cargo doc`. | completed |
| F61 | RC-11 review | `src/util/ini.rs` | `write_config_ini` now creates a missing game directory through the helper where it previously failed. | completed |
| F62 | RC-11 review | `src/retry.rs` | Jitter is salted by process id and attempt only, so concurrent files share the same spread. | completed |
| F63 | RC-11 review | `src/repair/file/mod.rs` | A comment still budgets 5 × (15 s + 10 s); with the final sleep removed it is ~100 s. | todo |
| F64 | RC-11 review | `src/hyp.rs` | No call-site test that `get_node`/`get_direct` retry a retryable error and stop on a fatal one. | completed |
| F65 | T28 implementer | `src/util/ini.rs` tests | No config-specific "no temp left" assertion after the helper extraction. | completed |
| F66 | T28 implementer | `src/util/atomic.rs` | Error context is artifact-neutral; artifact-specific labels would need a parameter. | completed |
| F67 | RC-12 review | `src/repair/phases/post.rs` | `move_audio_cache` still strips the cache prefix manually; it could use `rel_from_base`. | completed |
| F68 | RC-12 review | `src/util/path.rs` | `rel_from_base` and `join_rel` each inline the lossy `/` conversion; one private helper would be literal. | completed |
| F69 | RC-12 review | `src/repair/scan.rs` | The helper tolerates length mismatch; a `debug_assert` at the two call sites would still be useful. | completed |
| F70 | RC-12 review | `src/repair/file/inplace.rs` | `source_ok_by_range` is the inverse mapping; cross-reference or pin it near the shared remap. | completed |
| F71 | T30 implementer | `src/util/path.rs` | `join_rel` assumes an already-clean `dir_rel` with no trailing separator. | completed |
| F72 | T30 implementer | `src/util/mod.rs` | The `path` module summary does not mention the two new rel helpers. | todo |
| F73 | RC-13 review | `src/repair/mod.rs`, `README.md`, `docs/02` | `--print-plan --dry-run` still fetches the deprecated list, contradicting the documented contract. | completed |
| F74 | RC-13 review | `src/report/emit.rs` | Public docs link to the private `write_line`, leaving a rustdoc warning. | todo |
| F75 | T33 implementer | `tools/sync` | The sibling crate has its own unrelated `Record::event` implementation. | dropped (unrelated sibling-crate implementation) |
| F76 | T33 implementer | `src/util/ini.rs` | No pure `render_config_ini` helper; future tests observing rendering without writing would need one. | dropped (helper for a test that does not exist yet) |
| F77 | RC-14 review | `src/repair/mod.rs` | `publish_plan`'s doc describes a disk-figures computation that moved into `build_and_assert_plan`. | todo |
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
| F95 | T40 implementer | `README.md` | The nextest note hard-codes a test count ("362 tests") that is now 470 and drifts; de-number it. | todo |
| F96 | T41 implementer | `Cargo.toml` | Release builds strip symbols; crash triage may want `strip = "debuginfo"` or a separate profile. | dropped (crash-triage symbol needs are hypothetical) |
| F97 | T42 implementer | `clippy.toml` | Revisit `allow-unwrap-in-tests`/`allow-expect-in-tests` to replace the code-level test allows. | completed |
| F98 | T43 implementer | `Cargo.toml` | `md-5`'s optional `asm`/`md5-asm` backend may beat the portable path on x86; measure separately. | dropped (speculative; the portable upgrade already passed the gate) |
| F99 | T43 implementer | `tools/sync`, `tools/capture` | Both tools still depend on `md5 0.7`; a separate migration could retire it from the lockfile. | completed |
| F100 | T43 implementer | `src/repair/scan.rs` | The scan clones a context per range when resolving verdicts; unchanged here. | dropped (small allocation outside a hot loop) |
| F101 | RC-09 review | `src/repair/scope/ledger.rs` | `add_saturating` loads then `fetch_add`, so additions within `n` of `u64::MAX` could still wrap. | completed |
| F102 | RC-09 review | `src/repair/mod.rs` | `PurgeKeys::new` folds the expected set on the async runtime before `spawn_blocking`. | completed |
| F103 | RC-09 review | `src/repair/purge.rs` | `mark_kept_ancestors` allocates one `PathBuf` per newly marked ancestor. | completed |
| F104 | RC-09 review | `src/util/path.rs` | The T26 skip rests on a one-off measurement with no retained harness to re-check on other installs. | dropped (the decision is recorded; a retained harness for a skipped change is not warranted) |
| F105 | RC-09 review | `src/logging.rs`, docs | Buffering delays external `tail -f` by up to 64 KiB; worth stating if log streaming is supported. | todo |
| F106 | RC-16 review | `Cargo.toml` | The tokio comment promises a saving `reqwest` already spends; reword to the real one (dropping `process`/`signal`, dev-only `test-util`). | completed |
| F107 | RC-16 review | tests/benchmarks | The MD5 throughput claim has no committed reproduction; add a small `#[ignore]`d benchmark or script. | completed |
| F108 | RC-16 review | `tests/common/mod.rs`, `tests/artifact_bridge/server.rs`, `tests/peak_heap.rs` | The same `md5_hex` loop is duplicated three times; one shared helper would do. | completed |
| F109 | RC-16 review | `tools/sync`, `tools/capture` | The dev tools stay outside the lint policy; consider a tools-only lint check when next touched. | completed |
| F110 | T44 implementer | `tests/repair_offline.rs`, `tests/common/mod.rs` | Manifest ordering is pinned only at module level; a binary-level case needs a fixture that fails one selected manifest. | completed |
| F111 | RC-17 review | `tests/repair_machinery.rs` | No end-to-end stale-verdict case that mutates a file between the plan scan and the repair. | dropped (superseded by F32 (same end-to-end case)) |
| F112 | RC-17 review | `src/repair/phases/post.rs` | Partial-copy tests stop at the helper; the post-phase outcome/config.ini unbumped claim is not asserted. | todo |
| F113 | RC-17 review | `tests/repair_offline.rs` | The duration process test always passes `--scan`; absence of `phase=scan` on an unscanned run is unasserted. | todo |
| F114 | T45 implementer | `src/phase.rs` | `phase=plan` excludes the scan and `publish_plan`; purge execution is not timed at all. | completed |
| F115 | T45 implementer | `src/phase.rs` | `PhaseTimer` logs on drop during an error path, reporting time-to-failure as if the phase completed. | completed |
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
| F154 | BUILD-TOOLS-01 review | `tools/capture/Cargo.toml` | The realized tokio saving (`parking_lot`, `process`, `signal`, `fs`, `io-std`) could be stated in the tools' build docs. | todo |
| F155 | BUILD-TOOLS-01 review | `tools/sync/src/hash/stream.rs` | The digest trait comes from `sha2::Digest`; importing `md5::Digest as _` would make the source explicit. | todo |
| F156 | TEST-HELPERS-01 review | `tests/common/mod.rs` | Including the whole fixture module in `artifact_server`/`peak_heap` slightly increases test build time. | todo |
| F157 | TEST-HELPERS-01 review | `tests/peak_heap.rs`, `tests/artifact_bridge.rs` | The explicit `#[path = "common/mod.rs"]` on `mod common;` is redundant. | todo |
| F158 | LINT-01 review | `src/lib.rs`, `clippy.toml` | A root `clippy.toml` with the two test options would let `src/lib.rs` drop its `cfg_attr(test)` allow; integration targets still need theirs. | todo |
| F159 | LINT-01 review | docs | The `cedb038` commit message still claims the clippy test options are wholly ignored; the README is now accurate. | dropped (history-only; README is now accurate and rewriting is forbidden) |
| F160 | LINT-01 review | `tools/sync/src/commands/sync/apply/phases.rs` | Non-test `expect`s remain in the tools, blocking any future `unwrap_used`/`expect_used` opt-in. | todo |
| F161 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` tests | The removed-seek test infers positioning from the cursor's initial value; a sink wrapper would pin it directly. | todo |
| F162 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | The seek-refund test drives the helper; no end-to-end `write_downloaded_chunk` case with an unseekable sink. | todo |
| F163 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | A copy-path seek failure drops the cached source handle instead of restoring it, unlike the other fallbacks. | todo |
| F164 | TEMP-WRITER-01 review | `src/repair/file/temp.rs` | The `drop(src_handle)` Windows share-mode rationale is defensive rather than required for std's default share mode. | todo |
| F165 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | Five tests still spell `.girpr_tmp` literally instead of deriving it from `temp_sibling`. | todo |
| F166 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | The fallback test could assert source-present/target-absent inside the injected copy, not only after. | todo |
| F167 | TEMP-PUBLISH-01 review | `src/repair/phases/post.rs` | `copy_publish_with`'s no-file-name refusal has no direct test. | todo |
| F168 | TEMP-PUBLISH-01 review | `src/repair/scope/report.rs` | The `#[cfg(unix)]` symlink test and Unix non-UTF-8 arm never ran on this host; they need a Unix run. | todo |
| F169 | INPLACE-01 review | `src/repair/file/inplace.rs`, `src/repair/scan.rs` | An intact scan verdict on a coarse-mtime filesystem can still be accepted after a same-tick rewrite, so the run reports the file skipped and leaves it unrepaired until a later run. | todo |
| F170 | INPLACE-01 review | `src/repair/file/inplace.rs` | The natural evasion window is narrower than the forced test suggests; the recorded mtime must share the coarse tick with the post-scan rewrite. | todo |
| F171 | INPLACE-01 review | `src/repair/file/inplace.rs` | `trim_is_free` uses `is_none_or`, so an overflowing `r + s` would read as inside the target if the offset type ever widens. | todo |
| F172 | INPLACE-01 review | `src/repair/file/inplace.rs` | `reusable_verdict` stats the path instead of the open handle, so a replacement between open and stat is not detected. | todo |
| F173 | CHECK-01 review | `src/repair/phases/check.rs` | The live-progress test observes the fold by a bounded `yield_now` spin; a oneshot/watch would remove the timing assumption. | todo |
| F174 | CHECK-01 review | `src/report/summary.rs` | `check_damaged_chunks` is 0 both for "no damage" and "never measured"; an `Option` accessor would disambiguate. | todo |
| F175 | CHECK-01 review | `src/repair/phases/check.rs` | After a panic, in-flight blocking checks keep reading until they finish, lengthening the post-failure tail. | todo |
| F176 | CHECK-01 review | `src/repair/mod.rs` | The `run_check_only` comment about a dropped clone predates the panic path and is imprecise. | todo |
| F177 | REPORT-API-01 review | `src/hyp.rs`, `src/repair/phases/metadata.rs` | `GameConfig::game`/`ChunkBuild::tag` are unread; validate them against the requested biz/tag in a hardening batch. | todo |
| F178 | REPORT-API-01 review | `src/hyp.rs` | `ChunkBuild::tag`'s comment says the run compares the build id, but it only reports it; the `ManifestFile` size doc also overstates. | todo |
| F179 | REPORT-API-01 review | `README.md` | The module map still names `src/report.rs`, `emit_progress` and `spawn_progress_reporter`. | todo |
| F180 | PHASE-01 review | `src/phase.rs` | The no-double-count doc omits that the debug per-file `duration_ms` line nests inside `repair`. | todo |
| F181 | PHASE-01 review | `src/phase.rs` | The `plan` interval also contains the purge-plan walk, and the `REPORT` emission is an unnamed untimed gap. | todo |
| F182 | TEST-MANIFEST-01 review | `tests/repair_offline.rs` | The multi-manifest binary test does not cover the `ignore` set (`res_category_dir`). | todo |
| F183 | TEST-MANIFEST-01 review | `tests/common/mod.rs` | `ExtraManifestSpec::new` ties `category_id` to `matching_field`, so an ignored-by-category fixture cannot be expressed. | todo |
| F184 | TEST-MANIFEST-01 review | `tests/common/mod.rs` | Extra manifests' chunks are registered even when `served = false`; a future "absent manifest" case needs that adjusted. | todo |
| F185 | ATOMIC-01 review | `src/report/emit.rs` | The rustdoc links the private `write_line`, producing the sole `cargo doc` warning. | todo |
| F186 | ATOMIC-01 review | `src/repair/phases/post.rs` | The same private `temp_sibling` doc link pattern; silent now because private docs are not built. | todo |
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
| F203 | MD5-TYPE-01 review | `src/util/mod.rs`, `README.md` | Docs still say `util::md5_file` resolves at the module root and name it in the repair path, but only `util::md5::md5_file` exists now. | todo |
| F204 | PATHHELPERS-01 review | `src/repair/phases/post.rs` | `move_audio_cache_with` now strips the cache prefix twice; a helper returning both key and destination path would avoid it. | todo |
| F205 | PATHHELPERS-01 review | `src/repair/phases/post.rs` | The dry-run log for nested cache paths now prints the `/` rel instead of the platform spelling. | todo |
| F206 | PATHHELPERS-01 review | `tests/repair_post.rs` | No end-to-end test moves a non-UTF-8-named audio cache file and asserts the destination name bytes. | todo |
| F207 | REFACTOR-01 review | `src/repair/mod.rs` | `estimate_download` both computes and mutates `summary` flags; separating the flag writes would make it read as pure. | todo |
| F208 | EXIT-02 review | `src/main.rs` | `clamp_code` is now unreachable via `exit_code`; remove it or document it as defense against a future discriminant change. | todo |
| F209 | DRYRUN-02 review | `src/repair/purge.rs` | `apply_purge` still emits `0 bytes would be freed` for a dry pass with no candidates; now documented, suppression would be a behavior change. | todo |
| F210 | DRYRUN-02 review | `tests/repair_limits.rs` | The ceiling test cannot independently distinguish the up-front check from the post-repair re-check. | todo |
| F211 | DRYRUN-02 review | `src/repair/phases/post.rs` | A real `--purge-after --plan-out <inside install>` deletes the plan document it just wrote; document or exclude it from the keep set. | todo |
| F212 | PURGE-PERF-02 review | `src/report/summary.rs` | `deleted_extra_bytes.fetch_add` still uses plain atomic addition; a saturating variant would remove the last wrap path. | todo |
| F213 | PURGE-PERF-02 review | `src/hyp.rs` | `fetch_deprecated` still returns `Vec<String>`; returning `Arc<[String]>` would remove the one remaining conversion copy. | todo |

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
| EXIT-03 | F208 | todo | Wave E; clamp helper reachability; low |
| DRYRUN-02 | F196, F197, F198 | done | Wave D; dry-run end-to-end coverage and cleanup-line parity; low; b98cfa7, 8c10743, ca30361, a220f91, 55f8843 |
| DRYRUN-03 | F209, F210, F211 | todo | Wave E; dry-run zero-byte line, ceiling-stage distinction and plan-out purge hazard; low |
| PURGE-PERF-02 | F199, F200, F201, F202 | done | Wave D; purge arithmetic hardening and blocking-helper consistency; low; 3cb5771, 62b05e2, fe8d45e |
| PURGE-PERF-03 | F212, F213 | todo | Wave E; last wrap path and deprecated-list conversion; low |
| PATHHELPERS-02 | F204, F205, F206 | todo | Wave D; audio-move path reuse and non-UTF-8 coverage; low |
| REFACTOR-02 | F207 | todo | Wave E; make the download estimate read as pure; low |
| TEST-REGRESSION-01 | F32, F112, F113, F126, F127, F130 | in_progress | Wave D, after INPLACE-01, DRYRUN-01, TEMP-PUBLISH-01; mutation, partial-copy and timing regressions; low |
| TEST-OUTPUT-01 | F53, F116, F119 | todo | Wave D, after CHECK-01 and PHASE-01; output-plane golden tightening; medium |
| DOCS-USER-01 | F38, F41, F95, F105, F117 | todo | Wave E, after LOCK-01, JOBS-01, LOGGING-01, TEST-OUTPUT-01; low |
| DOCS-COMMENTS-01 | F63, F72, F74, F77, F118, F120, F121, F122, F124, F129, F178, F179, F180, F181, F185, F186, F203 | todo | Wave E, after REFACTOR-01 and PATHHELPERS-01; stale comments, module summaries, rustdoc links, and doc precision; low |
| HYP-02 | F177 | todo | Wave D; validate the hyp response identity against the request; medium |
| SYNC-01 | F123, F125 | todo | Wave A; girsync map-key call-site coverage and ASCII hash-name folding; low |
| METADATA-02 | F131, F132, F133, F134, F135 | todo | Wave D; metadata classification tests, label constants and doc precision; low |
| MANIFEST-02 | F136, F137 | todo | Wave D; manifest mock shutdown and out-of-order stream coverage; low |
| JOBS-02 | F139, F140, F141 | todo | Wave D; alias boundary case, help default annotation and ceiling review; low |
| LOGGING-02 | F142, F143, F144 | todo | Wave D; sink registry lifetime, install-to-flush coverage and per-sink errors; low |
| LOCK-02 | F145, F146, F147 | todo | Wave D; lock handle-identity hardening, test litter and Unix-only coverage; medium |
| INI-02 | F148, F149, F150 | todo | Wave D; ini refusal wording, external-race coverage and helper scope; low |
| BUILD-02 | F151, F152, F153 | todo | Wave D; per-member MSRV, scan-path benchmark and import tidy; low |
| BUILD-TOOLS-02 | F154, F155 | todo | Wave D; tools build-doc note and explicit digest trait import; low |
| TEST-HELPERS-02 | F156, F157 | todo | Wave D; shared-module build cost and redundant `#[path]`; low |
| TEST-MANIFEST-02 | F182, F183, F184 | todo | Wave D; multi-manifest fixture filter and served-absent cases; low |
| LINT-02 | F158, F160 | todo | Wave D; clippy.toml for unit-test allows and tools expect cleanup; low |
| TEMP-WRITER-02 | F161, F162, F163, F164 | todo | Wave D; temp writer positioning seam, end-to-end refund and comment accuracy; low |
| TEMP-PUBLISH-02 | F165, F166, F167, F168 | todo | Wave D; publish test naming, ordering assertion, refusal test and Unix run; low |
| INPLACE-02 | F169, F170, F171, F172 | todo | Wave D; coarse-mtime skip-path hardening and guard robustness; medium |
| CHECK-02 | F173, F174, F175, F176 | todo | Wave D; check-progress signal, tally ambiguity, cancellation and comment; low |

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
