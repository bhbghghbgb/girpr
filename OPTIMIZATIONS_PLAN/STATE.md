# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or a historical change log in this file. Record context, decisions, status and
commit ranges (the functional review handoff data) only. The report is a
snapshot: line numbers, counts and even whole recommendations may be stale —
implementers always validate against current code before editing.

## Current position

* Next action: wait for the T21 implementing session to finish, then apply its
  result. T21 completes CP10's tasks, so after recording it, run the CP10 review
  phase (`ORCHESTRATION.md` §4).
* Active phase: implementation. CP1–CP9 `done`; T20 `pending_review`, T21 `in_progress` (last CP10 task).
* Active checkpoint: none yet (CP10 `open`). Active batch: none.
* Blocked on user: no. (The user supplies the session-spawning mechanism.)
* Scheduled after all tasks are terminal and all checkpoints `done` — and BEFORE
  any follow-up batching/implementation — is a user-directed flaky-test
  investigation (investigate → fix → review sessions). See its section below.

## Status vocabulary

* Tasks: `todo` → `in_progress` → `pending_review` → `completed` /
  `completed (fixed)` / `reverted`; or `todo`/`in_progress` → `skipped`.
* Checkpoints: `open` → `in_progress` → `done`.
* Follow-ups: `proposed` → `scheduled` → `in_progress` → `pending_review` →
  `completed` / `removed`. Follow-up batches reuse the per-task / checkpoint
  loop and its separate state commits (`ORCHESTRATION.md` §5).

## Tasks

Seeded order follows the report's suggested sequencing (§L810–L824): P0 low-risk
first, then P1 narrow production seams, then P2 wide high-payoff seams.

| ID | Title | Findings | Status | Priority | CP | Notes |
|---|---|---|---|---|---|---|
| T01 | `retry.rs` injectivity sampling → coprimality proof | C1 | completed | P0 | CP1 | CP1 review: completed. |
| T02 | Root-unit low-risk merges: `plan.rs` layout + `clock_support` | C3, C4 | completed | P0 | CP1 | CP1 review: completed. |
| T03 | sled2redb low-risk merges: stale-file fixture + destination/temp case | D3, D4 | completed | P0 | CP1 | CP1 review: completed. D3 fixture saving did not materialize (see context). |
| T04 | Small-crates table folds: `crates/log` + `crates/tempdir` | D5, D6, D7 | completed | P0 | CP1 | CP1 review: completed. Protected tests intact. |
| T05 | Capture: reduce full captures 3→1 (Ceiling + GzipThreshold) | B1, B5 | completed | P0 | CP2 | CP2 review: completed. |
| T06 | Capture: device-name flag refusal → config unit test | B2 | completed | P0 | CP2 | CP2 review: completed. |
| T07 | Capture: downscale `m7_assembly` megabyte bodies | B3 | completed | P0 | CP2 | CP2 review: completed. |
| T08 | Sync smaller findings: helper meta-tests + fixture trims | E6 | completed | P0 | CP2 | CP2 review: completed; scan.rs harness-pin skip confirmed. |
| T09 | `repair_machinery.rs` in-process `Summary` downshift | A1 | completed | P1 | CP3 | CP3 review: completed. |
| T10 | `repair_plan.rs` audio-plan partial downshift | A3 | completed | P1 | CP3 | CP3 review: completed (mock-server estimate note confirmed). |
| T11 | Inject the MD5 buffer; shrink 513 KiB files | C2 | completed | P1 | CP4 | CP4 review: completed. |
| T12 | Pure `DiskPlan::from_figures` seam | C5 | completed | P1 | CP4 | CP4 review: completed (DiskFigure deviation validated). |
| T13 | Capture: merge mock / no-network full captures | B4 | completed | P1 | CP5 | CP5 review: completed; row-pin deviation confirmed. |
| T14 | `repair_purge.rs` plan-out + purge-only merges | A7 | skipped | P1 | CP5 | CP5 review: skipped (deliberate plan-out skip confirmed); purge-only trio merged in b3cf81bf (12→10 tests), acceptance criteria met. |
| T15 | Parameterize the sync heartbeat period | E4 | completed | P1 | CP6 | CP6 review: completed; guardrail did not bind. |
| T16 | Sync injectable record sink + `run_captured` | E1 | completed | P2 | CP7 | CP7 review: completed. |
| T17 | Sync scoped subscriber + injectable console writer | E2 | completed | P2 | CP8 | CP8 review: completed; tuple deviation and both fallbacks confirmed. |
| T18 | Sync injected `TestHooks` for ambient env hooks | E5 | completed | P2 | CP8 | CP8 review: completed; env fallback + heartbeat fold confirmed. |
| T19 | Richer `CompareOutcome`, delete duplicated pipelines | E3 | completed | P2 | CP9 | CP9 review: completed; ~301 net lines removed, harnesses only. |
| T20 | `repair_scan.rs` estimate/mode downshift + merge | A2 | pending_review | P2 | CP10 | Implemented in 5deab7fb; spawns 16→4, tests 15→14; process pins retained. |
| T21 | `peak_heap.rs` measurement → scale-free invariant | A5 | in_progress | P2 | CP10 | Invariant must not pass vacuously; may drop workspace `unsafe` exemption. |
| T22 | Expose `Args` + conversion record from sled2redb lib | D1 | todo | P2 | CP11 | Unblocks D8 (follow-up F02). Keep one spawn for the flag→record→exit-0 path. |
| T23 | Split `convert_into` at the sled-read / redb-write boundary | D2 | todo | P2 | CP11 | Keep one real sled DB for round-trip, one bad-row DB for error path. |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T04 | done | P0 root unit + small crates + sled2redb low-risk — all completed, no fixes/reverts |
| CP2 | T05–T08 | done | P0 capture + sync small findings — all completed, no fixes/reverts |
| CP3 | T09–T10 | done | `repair_machinery` + `repair_plan` downshifts — both completed, no fixes/reverts |
| CP4 | T11–T12 | done | root-unit production seams (MD5, DiskPlan) — both completed, no fixes/reverts |
| CP5 | T13–T14 | done | capture mock merge + `repair_purge` merges — T13 completed, T14 skipped (confirmed), no fixes/reverts |
| CP6 | T15 | done | sync heartbeat parameterization — completed, no fixes/reverts |
| CP7 | T16 | done | sync record sink + `run_captured` (largest single change) — completed, no fixes/reverts |
| CP8 | T17–T18 | done | sync logging + `TestHooks` seams — both completed, no fixes/reverts |
| CP9 | T19 | done | sync `CompareOutcome` pipeline dedup — completed, no fixes/reverts |
| CP10 | T20–T21 | open | `repair_scan` downshift + `peak_heap` invariant |
| CP11 | T22–T23 | open | sled2redb production seams |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep each range until its checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | `5b5a10cb..934eadcb` | completed |
| T02 | `c6140196..ca643fec` | completed |
| T03 | `1bfa735f..cb9226fb` | completed |
| T04 | `0ea6802e..ccbef0ad` | completed |
| T05 | `e5cf89dc..4a097a31` | completed |
| T06 | `ab96ebab..6b7dfdde` | completed |
| T07 | `c9dc8307..e6aefb87` | completed |
| T08 | `8160cef5..c8ed2fef` | completed |
| T09 | `4c74ef45..0e4bf161` | completed |
| T10 | `78746658..7da360d3` | completed |
| T11 | `6117c5ff..787c35f0` | completed |
| T12 | `ae909714..08ca4e98` | completed |
| T13 | `2758abff..bda7e0c5` | completed |
| T14 | `86b465f3..b3cf81bf` | skipped (confirmed) |
| T15 | `7c787656..a2f21205` | completed |
| T16 | `0c22b736..3bd03d22` | completed |
| T17 | `069ca97a..6f488b0b` | completed |
| T18 | `4c7625a6..f1800244` | completed |
| T19 | `416b480b..416aeda3` | completed |
| T20 | `5d58b228..5deab7fb` | — (pending CP10 review) |
| T21–T23 | — (not started) | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a batching review session has decided the batches (`ORCHESTRATION.md`
§5). The first three are seeded from the report.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | **B6:** in-process capture seam — call `run::capture/verify/materialize/list` directly and assert on `Outcome` records; removing the TCP mock needs an injected HTTP transport for `Fetcher` (report §B6 L318–L330). | report | proposed | Largest capture lever, wide/risky; B1–B5 (T05/T06/T07/T13) capture most benefit first. |
| F02 | ~~**D8:** render the real `sled2redb::Args::command()` for the help pin instead of the hand-built `clap::Command` (report §D8 L583–L591).~~ | report | removed | CP5 reviewer removed it: zero test-count/spawn/fixture effect, pure maintenance; avoid an implement+review cycle for no scale benefit. |
| F03 | Table-ize the ~15 near-identical fixture/expected-record tests in `tools/sync/tests/lazy/compare.rs` (report §E3 L717–L719). | report | proposed | Priority scale item: unblocked by T19's shared seam (416aeda3), ~14 tests — CP9 reviewer recommends next lever. |
| F04 | ~~Widen T01's no-u64-wrap assertion scope if jitter's test coverage is extended beyond attempts `1..=5`; the proof currently asserts the bound only for that range.~~ | T01 implementer | removed | CP2 reviewer removed it: non-actionable conditional watch item; adds coverage rather than reducing scale, contingent on an unplanned jitter-test extension. |
| F05 | Fold `plan.rs::a_file_with_no_chunks_and_no_size_is_tiled` into T02's merged table (redundant with its `"empty file"` row). | T02 implementer | proposed | Non-blocking; outside T02's named cluster, left for later cleanup. |
| F06 | ~~Optional D6 hygiene: downshift the `Targets` `Layer`/`Arc<Mutex<Vec>>` harness in `filter.rs:36–68` to `EnvFilter::enabled` with constructed metadata (~30 lines).~~ | T04 implementer | removed | CP1 reviewer removed it: no test-count effect and riskier than the fold; avoid a future implement+review cycle for zero scale benefit. |
| F07 | ~~Simplify away the now-near-trivial `BundleVariant` enum and its `key()`/`extra()` methods (only `Default` remains after T05) — e.g. an argument-less `shared_bundle()`.~~ | T05 implementer | removed | CP2 reviewer removed it: pure enum hygiene, zero test-count effect (same rationale as F06). |
| F08 | ~~Metadata-only assertion path for T07's retained measured test: compute totals from declared chunk sizes via a pure seam instead of materializing ~4.7 MB of bodies.~~ | T07 implementer | removed | CP2 reviewer removed it: needs a production seam, marginal remaining MBs, and contradicts T07's "keep one measured ground-truth pin" criterion. |
| F09 | ~~Share the chunk-length constants between `m7_assembly` and `m6_chunk_store` (different fixture structure, carries md5 strings).~~ | T07 implementer | removed | CP2 reviewer removed it: zero test-count effect, low value. |
| F10 | ~~Make `Rig::logs` a lazily-initialized `OnceLock<TempDir>` so the 17 fully-converted T09 rigs stop allocating an unused log directory.~~ | T09 implementer | removed | CP3 reviewer removed it: zero test-count/spawn effect, pure hygiene (same rationale as F06/F07/F09). |
| F11 | Downshift the staged and by-mode sub-runs in `the_three_tiers_produce_the_same_file` (pure `ram_bytes` counters; only the shortage sub-run needs the log) — ~2 more spawns. | T09 implementer | proposed | Real spawn reduction; CP3 reviewer to decide. |
| F12 | Merge the two `--fail-fast` malformed-manifest cases into a `Lies` table, and merge `the_three_modes_produce_the_same_bytes` with `no_mode_leaves_a_temp_file_behind` into a three-mode loop asserting bytes + leftovers. | T09 implementer | proposed | Real test-count reduction; CP3 reviewer to decide. |
| F13 | Share one `MockServer` across all three cases of T10's audio-plan test (start before the loop, stop after) to cut 2 listener/task starts. | T10 implementer | proposed | Real but small scale reduction, low risk; outside T10's stated steps. |
| F17 | Capture the on-disk shared bundle with `--jobs 1` so request `seq` is deterministic, letting T13's row-pin test read from it too (→ one fewer capture) — but it serializes the shared bundle read by ~20 offline tests. | T13 implementer | proposed | Trades fixture-build speed for one fewer test; reviewer decision only if fixture time acceptable. |
| F18 | A6 (out of T14 scope): `tests/process.rs:281–330` duplicates the library-level removed-path case at `repair_purge.rs:257–307`; its comment claims a process-only distinction that does not exist (asserts `code == 0`). Could be removed or reduced to a one-line flag smoke. | T14 implementer | proposed | Separate finding, explicitly out of T14's guardrails. |
| F19 | ~~Expose the walk's heartbeat emission count on a library result so the "fires exactly once per period" cadence can be asserted in-process rather than only via log rendering.~~ | T15 implementer | removed | CP6 reviewer removed it: coverage-adder, zero scale reduction. |
| F20 | ~~Use a T17 scoped subscriber to pin the `scan progress: walk` / `effective progress: phase *` event names and fields in-process.~~ | T15 implementer | removed | CP6 reviewer removed it: coverage-adder, depends on T17, zero scale reduction. |
| F21 | ~~Downshift `output.rs:626` (`the_scan_counts_the_entries_the_walk_saw`) — another spawn-based test whose counters already live on `ScanStats` — the same way as T15.~~ | T15 implementer | removed | CP8 reviewer removed it as stale: T17 already downshifted that test in `6f488b0b`. |
| F22 | ~~Extract `init_tracing`'s `log file:`/`log start` announcements into a shared helper so T17's remaining RUST_LOG smoke can be downshifted in-process.~~ | T17 implementer | removed | CP8 reviewer removed it: the smoke pins the real `init_tracing` install path; a shared helper would make it assert a re-implementation (coverage loss) for −1 spawn. |
| F23 | If the scheduled post-task flaky-test investigation resolves the WARN-capture flake, downshift the 2 `stat_trust/flag_surface.rs` spawns. | T17 implementer | proposed | Reframed by CP8 reviewer: the flake-investigation half is subsumed by the user-directed post-task investigation; retain only the downstream −2 spawn action, gated on that investigation's outcome. |
| F24 | Downshift T18's remaining log-file-ordering spawn with T17's scoped-subscriber helper and `--log-level trace` now that commit limits are injectable. | T18 implementer | proposed | Real −1 spawn, needs log-ordering capture; CP8 reviewer to decide. |
| F28 | Pin the exact stdout line for T20's "PLAN printed with a wrong digest" via a second binary spawn (currently asserted via plan-out document content). | T20 implementer | proposed | Would ADD a spawn, contrary to the one-binary-run-per-test guardrail; CP10 reviewer likely to prune. |
| F29 | Production seam exposing `summary.flags.plan_download_bytes` unconditionally so T20's remaining binary PLAN-line run can fully downshift. | T20 implementer | proposed | Real −1 spawn but needs a production seam (field currently only set with a --max-download-bytes ceiling); CP10 reviewer to decide. |
| F26 | ~~Production single-side seam (`scan_stat_only → plan_one_side → resolve_folder`) to dedup `tests/helpers.rs::phased` (~28-line copy), which T19 could not serve.~~ | T19 implementer | removed | CP9 reviewer removed it: ~100-line production seam for one ~28-line helper, zero test-count/spawn effect; `phased` intentionally spells out phases for per-phase assertions. |
| F27 | ~~Have `compare_detailed` return its `EnteredSpan` for the caller to hold while rendering, restoring exact span fidelity for `end`/`report` events.~~ | T19 implementer | removed | CP9 reviewer removed it: log-only, zero test-scale effect, no test pins span membership. |
| F25 | ~~Fold `TestCacheBackend` into `TestHooks` to fully unify the seam.~~ | T18 implementer | removed | CP8 reviewer removed it: zero scale benefit, churns roughly 30 hand-built `CommonOpts` literals. |
| F15 | ~~Replace T11's per-call 512 KiB `Vec` allocations in the production `scan_reader`/`md5_reader_digest` wrappers with a caller-owned or pooled buffer.~~ | T11 implementer | removed | CP4 reviewer removed it: production perf change, zero test-scale benefit. |
| F16 | ~~Add a pure `DiskFigure::Unresolved`/`Unreadable` figure test for stronger T12 coverage of the warning path.~~ | T12 implementer | removed | CP4 reviewer removed it: adds coverage only, zero scale reduction. |
| F14 | ~~Pin T10's in-process plan to its expected 2 `repair` entries for a stronger "plan reported" assertion.~~ | T10 implementer | removed | CP3 reviewer removed it: adds coverage, not scale reduction (same rationale as F04). |

## Post-task flaky-test investigation (user-directed)

Scheduled by the user after T17 reported flaky tests under plain `cargo test`
(stable under `cargo nextest`) that were already present in its given baseline;
whether this was introduced during this orchestration session or predates it is
unknown. This investigation MUST run after all tasks are terminal and all
checkpoints are `done`, but BEFORE any follow-up batching or implementation —
an explicit override of the default §5 completion order.

* Status: scheduled (not started).
* Scope: determine whether the flakiness originated during this orchestration
  session or before it, and determine whether and how it can be fixed.
* Sessions (one at a time, in this order): (1) **investigate** — establish origin
  and root cause, report findings only, no fix; (2) **fix** — implement a fix if
  the investigation warrants one; (3) **review** — review the fix. Give each
  session pointers and use separate `STATE.md` commits for status changes, as
  with the task loop.
* Evidence seed: T17's `6f488b0b` scoped-subscriber work; the scoped WARN
  assertion was intermittently flaky under threaded `cargo test` but stable
  under `cargo nextest`, so the stat_trust fold was reverted (see context).

## Follow-up batches

After all tasks and checkpoints are done, a batching review session decides how
to batch the surviving follow-ups (and may add/remove). Record the result here;
implement one batch at a time with one implementing session then one review
session, using the same separate state commits. Statuses: `open` → `in_progress`
→ `pending_review` → `done`.

| Batch | Follow-ups | Status | Commit range | Verdict | Scope / pointers | Notes |
|---|---|---|---|---|---|---|
| — | none yet | — | — | — | — | Batching review decides after tasks complete. |

## Known constraints from the report (persistent)

From the report §"Constraints to preserve" (L826–L841) and "Assumptions to
validate" (L843–L851):

* `README.md:446–450` documents a `--release -p girpr-log` gate run **solely**
  for `the_default_file_is_written_in_a_release_build_and_not_in_a_debug_one`;
  keep that test named and intact — D5 (T04) must not fold it away.
* The two `no_run` doctests mirrored by tests in `crates/log`/`crates/tempdir`
  exist because nextest cannot run doctests — keep (relevant to T04).
* The `#[ignore]` tests are deliberate hand-run measurements/pre-release checks
  — keep unless T21 (A5) is implemented (it removes one). They are skipped by
  the default gate, so removing them would not reduce the running count.
* `tests/zz_probe*.rs` are not Cargo test targets; `tests/scripts/io-profile/*`
  are not part of any gate and their iteration counts are measurement
  resolution — leave both alone.
* tools/sync's grouped suites (`lazy/`, `hash_mode/`, `stat_trust/`, `why/`,
  `rw_threads/`, `sync_plan/`) test genuinely different mechanisms — merge their
  harnesses (T19), not their cases.
* "Leave alone": §A L193–L200, §B L332–L340, §C L461–L466, §D L593–L607 —
  no new justified opportunity; the md5 RFC-1321 vectors and two ignored
  throughput benches stay protected (T11).
* B6 is a *scoped follow-up*, not a first move; B1–B5 capture most of its benefit.
* Capture's `bundle_source_key()` hashes all of `src/**`, so B1 and B5 are one
  task (T05) to avoid repeated recaptures.
* E2's rayon-thread caveat and B5's `digest_file` writer-path equivalence are
  the two findings most likely to need a fallback to a thin process smoke test.
* The report is a snapshot: exact spawn/file counts are static call-site counts
  and some will differ after harnesses are refactored; the ~13–18 convertible
  A1 cases and ~85 convertible E1 cases are estimates — confirm per case.

## Persistent context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from a task file, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume. Short,
context-only statements — no session ids, dates, machine data or chronological
change log.

* T03 (D3): report predicted "−1 sled fixture", but `build_sled_db` is still required by the `--force` conversion whose `(2,1)` result is asserted — real effect is −1 conversion only. Not a missed optimization; CP1 reviewer should not flag it.
* T04 (D5): plan.rs went 13 → 5 tests (−8), not the report's approximated "−7 / ~6", because fully folding the ten explicitly-listed anchors into the two required tables yields −8. Internal consistency, not a missed case; CP1 reviewer should not flag it.
* T08 (E6) bullet 2 (`src/scan.rs:240–248` harness pin) was deliberately skipped: dropping `heartbeat_entries()` removes zero tests/fixtures/spawns (report calls it "marginal; skip if unclear") and the pure helper already covers unset→2000. Constant pin `HEARTBEAT_ENTRIES == 2000` untouched. CP2 reviewer should confirm this skip.
* T08 deviation: `fixtures_classify_as_folders` was folded into the relying test `the_wider_handle_window_still_refuses_a_self_collision` as `debug_assert`s, not literally into `pair()`/`fixture()` — no local `fixture()` exists and `pair()` lives in `girpr-tempdir` (cannot depend on girsync's `classify`). The same three assertions are preserved.
* T10 (A3): report predicted "−2 spawns and mock servers", but only −2 binary spawns are realizable test-only — the downshifted cases still issue HTTP via `hyp_base_override`/`sophon_base_override`, so each keeps its `MockServer`; removing them needs a production data-injection seam the task does not call for. Not a missed optimization; CP3 reviewer should not flag it.
* T11 clippy diagnostics resolved by CP4 review: `repair/phases/plan.rs:573` type_complexity originated in T02's merged table; `tests/repair_machinery.rs:119` dead_code (`Run::word`) was exposed by T09's last-caller conversion. Both benign test-only lints, no `-D warnings` gate, no behavior/coverage/scale impact — reviewer deliberately opened no follow-up.
* T16: converted record tests now pin the real file cache because the path-keyed in-memory registry could leak across in-process runs (a few cases are slower); T18's injected cache backend could restore in-memory speed. `Report` lost `Copy`; all three by-ref `let report = log.report()` sites were audited. One pre-existing unrelated clippy warning remains in `src/hash/tests.rs`.
* T16 CP7 review noted for T17: the two remaining `stat_trust/flag_surface.rs` spawns (`bin`/`bin_mem`) assert a WARN string ("no-trust") emitted through the process subscriber; downshifting them needs T17's scoped subscriber. Reviewer deliberately did not open a standalone follow-up — fold into T17's scope if desired.
* T17 deviations/fallbacks: `subscriber_for` returns `(impl Subscriber, Option<WorkerGuard>)` rather than the plan's `-> impl Subscriber` — the tuple is required so callers keep the WorkerGuard alive past the `with_default` closure. Rayon caveat verified NOT binding: scan-done / effective-done:phase-A events are main-thread. Deliberate fallbacks: (1) `an_unrelated_rust_log…` kept as a process smoke — it pins `init_tracing`'s own `log start` event which a scoped stack cannot render; (2) the CP7 stat_trust fold was attempted then REVERTED — the scoped WARN assertion was intermittently flaky under threaded `cargo test` (stable under `cargo nextest`); kept its 2 spawns rather than ship a flaky test. CP8 reviewer confirmed both.
* T19 stale-report findings: the report listed six compare reimplementations but omitted a seventh (`resolve_both` in `crates/tempdir/src/test_support.rs`) — converted too. `tests/helpers.rs::phased` was listed but is actually the single-side update pipeline (no classify/open_side/plan_pairs/diff_maps), so the compare seam cannot serve it — left untouched deliberately. `compare_detailed` enters and returns its span before `cmd_compare` renders, so `end`/`report` log events are no longer inside the `girsync.compare` span; no test pins span membership and stdout is identical.
* T12 deviation: the injected item is a `DiskFigure` enum (`Measured(FileLen, bool, PathBuf)` / `Unresolved`) rather than the plan's `(FileLen, bool)` tuple — a plain tuple cannot distinguish the escaping-rel case from `(Absent, false)` broken symlink. Symlink/dir → `temp` is preserved via `symlink_metadata(...).is_file()` (same probe `resolve_mode` uses). Report's "6 files" fixture count was stale; 4 files were removed. CP4 reviewer sanity-check: symlink length now read via `symlink_metadata` not `metadata` — affects only the unreadable-warning path for symlinks with inaccessible targets.
* T13 (B4) guardrail-driven deviation: the plan proposed folding the `:1092` request-log row-pin test onto the shared bundle, but implementer verified empirically that "same world ⇒ same requests" is FALSE for the byte-exact pins — `seq` is assigned at issue time (`src/http/log.rs:63–73`), default `--jobs` is 4 (`cli/mod.rs:200`) vs the test's `--jobs 1`, so seq→row mappings differ. Folding it would have weakened the pins, so `:1092` was kept separate. Net still 4 tests → 2 (−2 captures, −2 mocks); the folded ordering assertion is causal and stable over 5 runs. CP5 reviewer should confirm.
* T14 (A7) partial implementation: only the purge-only trio was merged (12 → 10 tests, −2 mocks/temp dirs); the plan-out pair at `repair_purge.rs:422`/`:455` was deliberately SKIPPED. The task is internally contradictory — its Goal says merge that pair, but its Acceptance criteria and explicit guardrail say the complementary pair at `:421`/`:454` (the same tests) must NOT be folded, and the report's actual do-not-merge list (L826–L841) contains no `repair_purge.rs` pair. The implementer honored the authoritative guardrail; acceptance criteria are still met. CP5 reviewer must confirm the skip, not flag it as missed.
