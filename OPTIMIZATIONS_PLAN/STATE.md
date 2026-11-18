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

* Next action: wait for the CP4 review-and-fix session to finish, then apply its
  verdicts and follow-up recommendations and mark CP4 `done` (`ORCHESTRATION.md` §4).
* Active phase: review. CP4 `in_progress`; both CP4 tasks are `pending_review`.
* Active checkpoint: none yet (CP2 `open`). Active batch: none.
* Blocked on user: no. (The user supplies the session-spawning mechanism.)

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
| T11 | Inject the MD5 buffer; shrink 513 KiB files | C2 | pending_review | P1 | CP4 | Implemented in 787c35f0; private seam, behavior unchanged. |
| T12 | Pure `DiskPlan::from_figures` seam | C5 | pending_review | P1 | CP4 | Implemented in 08ca4e98; `DiskFigure` enum seam (deviation), symlink/dir → temp preserved. |
| T13 | Capture: merge mock / no-network full captures | B4 | todo | P1 | CP5 | 4 tests → 2; verify exact row/byte pins hold on shared capture. |
| T14 | `repair_purge.rs` plan-out + purge-only merges | A7 | todo | P1 | CP5 | Two tables over the named case sets. |
| T15 | Parameterize the sync heartbeat period | E4 | todo | P1 | CP6 | If counters stay log-only, E2 (T17) is still required. |
| T16 | Sync injectable record sink + `run_captured` | E1 | todo | P2 | CP7 | Highest-value item (~85 spawns). Keep ~3 smoke process tests + output.rs log tests. |
| T17 | Sync scoped subscriber + injectable console writer | E2 | todo | P2 | CP8 | Verify rayon event threading before downshifting output.rs:703. Sequence after T16. |
| T18 | Sync injected `TestHooks` for ambient env hooks | E5 | todo | P2 | CP8 | Sequence after T16; keep env fallback for `.config/nextest-test-env`. |
| T19 | Richer `CompareOutcome`, delete duplicated pipelines | E3 | todo | P2 | CP9 | Merge harnesses, not cases — grouped suites test different mechanisms. |
| T20 | `repair_scan.rs` estimate/mode downshift + merge | A2 | todo | P2 | CP10 | Keep stdout/stderr process pins on the binary. |
| T21 | `peak_heap.rs` measurement → scale-free invariant | A5 | todo | P2 | CP10 | Invariant must not pass vacuously; may drop workspace `unsafe` exemption. |
| T22 | Expose `Args` + conversion record from sled2redb lib | D1 | todo | P2 | CP11 | Unblocks D8 (follow-up F02). Keep one spawn for the flag→record→exit-0 path. |
| T23 | Split `convert_into` at the sled-read / redb-write boundary | D2 | todo | P2 | CP11 | Keep one real sled DB for round-trip, one bad-row DB for error path. |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T04 | done | P0 root unit + small crates + sled2redb low-risk — all completed, no fixes/reverts |
| CP2 | T05–T08 | done | P0 capture + sync small findings — all completed, no fixes/reverts |
| CP3 | T09–T10 | done | `repair_machinery` + `repair_plan` downshifts — both completed, no fixes/reverts |
| CP4 | T11–T12 | in_progress | root-unit production seams (MD5, DiskPlan) |
| CP5 | T13–T14 | open | capture mock merge + `repair_purge` merges |
| CP6 | T15 | open | sync heartbeat parameterization |
| CP7 | T16 | open | sync record sink + `run_captured` (largest single change) |
| CP8 | T17–T18 | open | sync logging + `TestHooks` seams |
| CP9 | T19 | open | sync `CompareOutcome` pipeline dedup |
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
| T11 | `6117c5ff..787c35f0` | — (pending CP4 review) |
| T12 | `ae909714..08ca4e98` | — (pending CP4 review) |
| T13–T23 | — (not started) | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a batching review session has decided the batches (`ORCHESTRATION.md`
§5). The first three are seeded from the report.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | **B6:** in-process capture seam — call `run::capture/verify/materialize/list` directly and assert on `Outcome` records; removing the TCP mock needs an injected HTTP transport for `Fetcher` (report §B6 L318–L330). | report | proposed | Largest capture lever, wide/risky; B1–B5 (T05/T06/T07/T13) capture most benefit first. |
| F02 | **D8:** render the real `sled2redb::Args::command()` for the help pin instead of the hand-built `clap::Command` (report §D8 L583–L591). | report | proposed | Depends on T22 exposing `Args`; maintenance, no test-count change. |
| F03 | Table-ize the ~15 near-identical fixture/expected-record tests in `tools/sync/tests/lazy/compare.rs` (report §E3 L717–L719). | report | proposed | Optional in the report; ~14 tests; do only after T19's shared harness exists. |
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
| F15 | Replace T11's per-call 512 KiB `Vec` allocations in the production `scan_reader`/`md5_reader_digest` wrappers with a caller-owned or pooled buffer. | T11 implementer | proposed | Production perf change, beyond test-scale scope; CP4 reviewer likely to prune. |
| F16 | Add a pure `DiskFigure::Unresolved`/`Unreadable` figure test for stronger T12 coverage of the warning path. | T12 implementer | proposed | Adds coverage, no scale benefit; CP4 reviewer likely to prune. |
| F14 | ~~Pin T10's in-process plan to its expected 2 `repair` entries for a stronger "plan reported" assertion.~~ | T10 implementer | removed | CP3 reviewer removed it: adds coverage, not scale reduction (same rationale as F04). |

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
* T11 implementer reported clippy diagnostics in `repair/phases/plan.rs:573` and `tests/repair_machinery.rs:119` described as pre-existing (files touched by T02/T09). CP4 reviewer should confirm they are not regressions from those tasks.
* T12 deviation: the injected item is a `DiskFigure` enum (`Measured(FileLen, bool, PathBuf)` / `Unresolved`) rather than the plan's `(FileLen, bool)` tuple — a plain tuple cannot distinguish the escaping-rel case from `(Absent, false)` broken symlink. Symlink/dir → `temp` is preserved via `symlink_metadata(...).is_file()` (same probe `resolve_mode` uses). Report's "6 files" fixture count was stale; 4 files were removed. CP4 reviewer sanity-check: symlink length now read via `symlink_metadata` not `metadata` — affects only the unreadable-warning path for symlinks with inaccessible targets.
