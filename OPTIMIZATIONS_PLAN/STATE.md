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

* Next action: pick the next `todo` task in seeded order — T08 — and spawn its
  implementing session (`ORCHESTRATION.md` §3).
* Active phase: implementation. CP1 `done`; T05, T06, T07 `pending_review`, T08 `todo` (CP2).
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
| T05 | Capture: reduce full captures 3→1 (Ceiling + GzipThreshold) | B1, B5 | pending_review | P0 | CP2 | Implemented in 4a097a31; no smoke fallback needed (unit tests reach prod path). |
| T06 | Capture: device-name flag refusal → config unit test | B2 | pending_review | P0 | CP2 | Implemented in 6b7dfdde; guardrail confirmed (refusal before HTTP). |
| T07 | Capture: downscale `m7_assembly` megabyte bodies | B3 | pending_review | P0 | CP2 | Implemented in e6aefb87; one measured-length pin test retained. |
| T08 | Sync smaller findings: helper meta-tests + fixture trims | E6 | todo | P0 | CP2 | `benches/tree_shapes.rs` is SKIP (timing, not a gate). |
| T09 | `repair_machinery.rs` in-process `Summary` downshift | A1 | todo | P1 | CP3 | Large; ~13–18 of 28 cases convertible (validate each). Keep log-asserting cases + `--io-threads` alias case on the binary. |
| T10 | `repair_plan.rs` audio-plan partial downshift | A3 | todo | P1 | CP3 | Keep binary spawn for `no_scope` only. |
| T11 | Inject the MD5 buffer; shrink 513 KiB files | C2 | todo | P1 | CP4 | Production seam: `*_with_buf` private helpers. |
| T12 | Pure `DiskPlan::from_figures` seam | C5 | todo | P1 | CP4 | Preserve symlink/dir → `temp` semantics; do not change `resolve_mode` itself. |
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
| CP2 | T05–T08 | open | P0 capture + sync small findings |
| CP3 | T09–T10 | open | `repair_machinery` + `repair_plan` downshifts |
| CP4 | T11–T12 | open | root-unit production seams (MD5, DiskPlan) |
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
| T05 | `e5cf89dc..4a097a31` | — (pending CP2 review) |
| T06 | `ab96ebab..6b7dfdde` | — (pending CP2 review) |
| T07 | `c9dc8307..e6aefb87` | — (pending CP2 review) |
| T08–T23 | — (not started) | — |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a batching review session has decided the batches (`ORCHESTRATION.md`
§5). The first three are seeded from the report.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | **B6:** in-process capture seam — call `run::capture/verify/materialize/list` directly and assert on `Outcome` records; removing the TCP mock needs an injected HTTP transport for `Fetcher` (report §B6 L318–L330). | report | proposed | Largest capture lever, wide/risky; B1–B5 (T05/T06/T07/T13) capture most benefit first. |
| F02 | **D8:** render the real `sled2redb::Args::command()` for the help pin instead of the hand-built `clap::Command` (report §D8 L583–L591). | report | proposed | Depends on T22 exposing `Args`; maintenance, no test-count change. |
| F03 | Table-ize the ~15 near-identical fixture/expected-record tests in `tools/sync/tests/lazy/compare.rs` (report §E3 L717–L719). | report | proposed | Optional in the report; ~14 tests; do only after T19's shared harness exists. |
| F04 | Widen T01's no-u64-wrap assertion scope if jitter's test coverage is extended beyond attempts `1..=5`; the proof currently asserts the bound only for that range. | T01 implementer | proposed | Non-blocking; only relevant if jitter tests are extended past attempt 5. |
| F05 | Fold `plan.rs::a_file_with_no_chunks_and_no_size_is_tiled` into T02's merged table (redundant with its `"empty file"` row). | T02 implementer | proposed | Non-blocking; outside T02's named cluster, left for later cleanup. |
| F06 | ~~Optional D6 hygiene: downshift the `Targets` `Layer`/`Arc<Mutex<Vec>>` harness in `filter.rs:36–68` to `EnvFilter::enabled` with constructed metadata (~30 lines).~~ | T04 implementer | removed | CP1 reviewer removed it: no test-count effect and riskier than the fold; avoid a future implement+review cycle for zero scale benefit. |
| F07 | Simplify away the now-near-trivial `BundleVariant` enum and its `key()`/`extra()` methods (only `Default` remains after T05) — e.g. an argument-less `shared_bundle()`. | T05 implementer | proposed | Low value / mostly hygiene, no test-count effect; CP2 reviewer may prune like F06. |
| F08 | Metadata-only assertion path for T07's retained measured test: compute totals from declared chunk sizes via a pure seam instead of materializing ~4.7 MB of bodies. | T07 implementer | proposed | Production change, out of scope and riskier; CP2 reviewer may prune. |
| F09 | Share the chunk-length constants between `m7_assembly` and `m6_chunk_store` (different fixture structure, carries md5 strings). | T07 implementer | proposed | Low value, no test-count effect; CP2 reviewer may prune. |

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
