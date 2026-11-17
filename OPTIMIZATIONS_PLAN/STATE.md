# STATE — checklist (single source of truth)

Owned by the orchestrator. Committed to version control. This file is the only
thing a replacement orchestrator needs to resume; keep it current. See
`ORCHESTRATION.md` for the process and `README.md` for the file index.

Do not put session-spawning mechanisms, dates/times, session ids, machine data
or a historical change log in this file. Record context, decisions, status and
commit ranges (the functional review handoff data) only.

## Current position

* Next action: spawn one **review session for Batch A** (F01) with the batch's
  scope/pointers and commit range recorded below. Batch A is the last batch.
  After it is `done`, evaluate the remaining follow-ups; if none are actionable,
  make the final cleanup commit (delete OPTIMIZATIONS_PLAN/ and
  docs/08-test-suite-reduction-audit.md, verify no implementation file refers
  to either).
* Active batch: **Batch A** (`pending_review`). Phase: follow-ups (§5). CP1–CP5 all `done`.
* Active checkpoint: none (`done`). Status: all 13 tasks terminal; Batches B and C `done`.
* Blocked on user: no.

## Status vocabulary

* Tasks: `todo` → `in_progress` → `pending_review` → `completed` /
  `completed (fixed)` / `reverted`; or `todo`/`in_progress` → `skipped`.
* Checkpoints: `open` → `in_progress` → `done`.
* Follow-ups: `proposed` → `scheduled` → `in_progress` → `pending_review` →
  `completed` / `removed`. Follow-up batches reuse the per-task / checkpoint
  loop and its separate state commits (ORCHESTRATION.md §5).

## Tasks

Seeded order: P0 safe merges/removes first, then P1 verification-required work.

| ID | Title | Status | Priority | Checkpoint | Notes |
|---|---|---|---|---|---|
| T01 | Merge `cli_contract.rs` helper/layout/parser self-tests | completed | P0 | CP1 | impl `a0a45fc8`; CP1 review confirmed completed. 19 self-tests → 4 tables (49→34), all cases preserved (several assertions strengthened); no product/process/golden code touched. |
| T02 | Remove duplicated fixture-helper meta-tests | completed (fixed) | P0 | CP1 | impl `d6dfb324`, review fix `e4439122`. 7 duplicate meta-tests removed; reviewer confirmed the flagged assertions were only transitively covered and folded them verbatim into `a_broken_fixture_fails_the_case_through_the_shared_decision` (no test function added, count reduction preserved). |
| T03 | Consolidate remaining low-risk root-unit cases | completed (fixed) | P0 | CP2 | commits `5e8d593f..185cfa4c`, review fix `cd80083b`. All 3 bullets done (−9 tests); the four distinct salvage_order cases kept separate. Review fix derived `live_count` in the reuse table loop instead of storing a redundant column, retaining the all-dead explicit check. No product logic changed. |
| T04 | Table-drive sync hash dedup/order asserts | completed | P0 | CP2 | impl `6f6a47af` (test-only); CP2 review confirmed completed. Four cases → `dedup_keeps_first_seen_order` table (41→38), 8 assertions preserved, parse-vs-parse_any via fn pointer. Review flags judged not worth changing (nested grouping keeps one comment per regression; named struct unnecessary for 4 fixed rows). |
| T05 | Remove capture no-op test; merge same-function pairs | completed | P0 | CP3 | commits `0e8496e9..30e15ae4`; CP3 review completed. No-op removed + 4 pairs merged (670→665). Flagged skip `config/tests.rs:411/483` CONFIRMED: different functions/contracts (end-to-end rejection vs diagnostic wording), report premise stale; folding not worth forcing. |
| T06 | sled2redb removes + usage-error parametrization | completed | P0 | CP3 | impl `e72f5859`; CP3 review completed. Subsumed help + temp-name tests removed, 4 usage-error cases parametrized (integration 16→11). Strict pins and convert.rs:515 (F03) intact. |
| T07 | `crates/log` + `tempdir` low-risk merges | completed | P0 | CP3 | commits `bd151a08..ef7fbfc1`; CP3 review completed. 3 tests removed; tempdir pairs folded keeping every assertion; log policy still asserted via LogPlan::resolve. |
| T08 | `repair_plan` scope table + optional root-integration merges | completed | P0 | CP3 | commits `11e108ce..79c52b93`; CP3 review completed. Scope table (3→1) + branch-failure and non-UTF-8 switch pairs merged (4 saved). Flagged skips CONFIRMED as complementary: repair_offline (success vs exhaustion) and repair_scope (purge_only/glob/cleanup/scenario differences). Do-not-merge list and T13 tests untouched. |
| T09 | Consolidate `run_lock.rs` case clusters | completed | P1 | CP4 | impl `327fd01e`; CP4 review completed. 15 → 5 tests, every assertion preserved, end-to-end cleanup case retained with real Lock::acquire/remove; product lock code untouched (repair_offline.rs anchors intact). run_lock 5/5 pass. |
| T10 | Consolidate `purge.rs` classifier early tests | completed | P1 | CP4 | impl `564b341b`; CP4 review completed. 4 early tests removed (purge 27→23). Parity independently confirmed: wrapper ≡ visible(true) under PathScope::default(); 16 NEW rows added to cached_keys table, 8 duplicate case-instances (corrected count per reviewer); both kept tests byte-for-byte unchanged. No product change. |
| T11 | Trim `migration_lock.rs` clear duplicates | skipped | P1 | CP5 | No commits. Validated all 5 candidates against current code; none is a proven duplicate - each has unique assertions (443 byte-identical cache mtime+length; 376 second-update backup; 241 cache-row content + convergence under hash-none; 279 sha256 digest-in-cache row; 480 empty-dirs/nesting). Report '≈' is thematic overlap, not property identity. Whole-target deletion correctly avoided. CP5 review CONFIRMED skip. |
| T12 | Capture: remove redundant process test; fold assertion-free test | completed (fixed) | P1 | CP5 | impl `2202ea9f` + review fix `edc1d76d` (both test-only). Closeout assertion-free test removed; reviewer applied F19 fix: pinned the unique process-level stdout-empty guarantee into `every_argument_error_is_one` (12 parser-refusal cases) and deleted the redundant `a_bad_log_level_is_refused_by_the_parser`. Full task complete; workspace green. |
| T13 | Cross-binary log-file policy reduction | completed | P1 | CP5 | commits `d1905b33..7f19b8d6` (4 test-only); CP5 review completed. 19 tests removed; all sub-parts done. girpr keeps 5 E2E process tests; gircap/girsync one smoke each + env-prefix unit. Every removed case has a retained equivalent; log policy implementation untouched. Full workspace nextest 2154 passed / 12 ignored / 0 failures. |

## Review checkpoints

| CP | Tasks | Status | Notes |
|---|---|---|---|
| CP1 | T01–T02 | done | root CLI self-tests + root-unit fixture dedup; T01 completed, T02 completed (fixed e4439122) |
| CP2 | T03–T04 | done | remaining root-unit merges + sync hash table; T03 completed (fixed cd80083b), T04 completed |
| CP3 | T05–T08 | done | capture, sled2redb, small crates, root-integration merges; T05/T06/T07 completed, T08 completed (skips confirmed). No review fixes needed. |
| CP4 | T09–T10 | done | `run_lock.rs` + `purge.rs` (verification-required); T09 and T10 both completed, parity independently confirmed, no review fixes needed |
| CP5 | T11–T13 | done | `migration_lock.rs`, capture medium, cross-binary log policy; T11 skip confirmed, T12 completed (fixed edc1d76d), T13 completed. All checkpoints done. |

## Commit ranges for review

Filled by the orchestrator when a task moves to `pending_review`; consumed by
the review-and-fix session. Keep each range until its checkpoint is `done`.

| Task | Commit range | Review verdict |
|---|---|---|
| T01 | a0a45fc8 (single commit) | completed |
| T02 | d6dfb324 + fix e4439122 | completed (fixed) |
| T03 | 5e8d593f..185cfa4c + fix cd80083b | completed (fixed) |
| T04 | 6f6a47af (single commit) | completed |
| T05 | 0e8496e9..30e15ae4 (2 commits) | completed (skip confirmed) |
| T06 | e72f5859 (single commit) | completed |
| T07 | bd151a08..ef7fbfc1 (2 commits) | completed |
| T08 | 11e108ce..79c52b93 (2 commits) | completed (skips confirmed) |
| T09 | 327fd01e (single commit) | completed |
| T10 | 564b341b (single commit) | completed |
| T11 | — (skipped, no commits) | skipped (CP5 confirmed) |
| T12 | 2202ea9f + fix edc1d76d | completed (fixed) |
| T13 | d1905b33..7f19b8d6 (4 commits) | completed |

## Follow-ups

Reviewer-controlled. Record proposed ideas here; only implement after all tasks
are done and a review session has decided the batches. The first four are seeded
from the report's cross-cutting / optional items (no or minimal test-count
effect) so the final follow-up phase is meaningful.

| ID | Description | Source | Status | Note |
|---|---|---|---|---|
| F01 | Extract the byte-identical `tests/common/mod.rs` shared by `tools/sync` and `tools/sled2redb` into the `girpr-tempdir` crate (report cross-cutting, L226–233). Saves maintenance, not test count. | report | scheduled | Batch A. Batching review confirmed still byte-identical (SHA-256 D3394032…, 27,348 bytes each). Largest refactor, run last. |
| F02 | ~~Remove the stale doc note in `tools/sync/DESIGN-NOTES.md:907` referencing an ignored test that no longer exists (report cross-cutting, L239–240).~~ | report | removed | CP4 reviewer pruned: one-line stale doc note too trivial to track through the final phase. |
| F03 | ~~Drop the two redundant full binary conversions in `tools/sled2redb/tests/convert.rs:515`.~~ | report | removed | Batching review deleted as a NO-OP: after T06's rewrite the two conversions assert distinct properties (end-to-end flag-before-path spelling; `--output=json` vs `--output json` byte-equality) the JSON-record test cannot provide. Report premise stale; T06 kept them deliberately. |
| F04 | ~~Consider merging the measured-fixture logic-name duplicates in `tools/capture/tests/measured_fixture.rs` (§E.5, L192–199) — only if an explicit size target exists; report recommends KEEP.~~ | report | removed | CP1 reviewer pruned: conditional on a non-existent size target, MEDIUM risk, report recommends KEEP; idea stays in report §E.5. |
| F05 | ~~If later tasks add more `catch_unwind` assertions in other integration-test binaries, move the `panic_message` helper into `tests/common/mod.rs`.~~ | T01 implementer | removed | CP3 reviewer pruned: `panic_message` exists in only one binary and other `catch_unwind` sites don't use it; no duplication to fix. |
| F06 | ~~Audit remaining `#[should_panic]` tests in other root integration-test files for single-helper clusters that could be folded into tables the same way.~~ | T01 implementer | removed | CP3 reviewer pruned: vague, no concrete remaining target, overlaps T09/T10 and F11. |
| F07 | ~~Fold `the_runtime_keeps_a_worker_beyond_every_file_window` (main.rs) into T03's `worker_threads_arithmetic` table using exact-value rows; saves one test.~~ | T03 implementer | removed | CP2 reviewer pruned: saves only 1 test while replacing a general `> jobs` headroom invariant (already covered by exact-value + saturation rows) with exact examples; runaway risk. |
| F08 | ~~If more sled2redb usage-error shapes with distinct assertions are added later, replace the parametrized test's `is_version` boolean with a per-row assertion closure.~~ | T06 implementer | removed | CP3 reviewer pruned: purely conditional on hypothetical future shapes; speculative. |
| F09 | Add a `cargo nextest run --release -p girpr-log` line to the documented test gate so the release branch of the default-file policy test is exercised (batch C). | T07 implementer | scheduled | Batching review re-scoped: there is NO CI config in this repo; the real gate is the documented command list in README.md:437-456, which already has `--release` nextest invocations. Genuine untested branch. |
| F10 | ~~Add a brief note to the surviving log policy test that `default_file_is_written()` is an intentional branch selector, not the assertion under test.~~ | T07 implementer | removed | CP3 reviewer pruned: trivial one-line doc note; marginal. |
| F11 | Factor the repeated 'branch call was issued / no getBuild followed / no config bump' checks in `repair_offline.rs` into a shared assertion helper. | T08 implementer | scheduled | Batch B. Batching review confirmed duplication at repair_offline.rs:506/587/632/724 and 509/645/728; allow per-case omissions. |
| F12 | Narrow the `audio_langs=en-us` process-level assertion in the merged repair_plan test to the no-scope case, leaving scope-specific assertions for all cases. | T08 implementer | scheduled | Batch B. Batching review: src/report/tests.rs:407-460 tests formatting only, so the process check isn't fully redundant - but repeating it per scope is. |
| F13 | ~~In `repair_offline.rs:514/560`, derive the `expect_no_branch_endpoint_in_text` flag from `case.failure == BranchFailure::terminal(-1001)` instead of storing it per row.~~ | T08 implementer | removed | CP3 reviewer pruned: micro-refactor within T08's table; feasible but not worth tracking. |
| F14 | ~~Add a shared test-only argv builder for the repeated `["--game-path", ..., "--biz", "hk4e_global"]` shape in the two consolidated CLI-heavy run_lock tests.~~ | T09 implementer | removed | CP4 reviewer pruned: explicit argv spellings are the assertion under test; a builder would hide them and re-churn the just-consolidated file. |
| F15 | ~~Split `tests/common/run_lock.rs` into per-consumer modules to remove the `#[allow(dead_code)]` on `mode_takes_lock`.~~ | T09 implementer | removed | CP4 reviewer pruned: LOW-need test-support refactor outside scope. |
| F16 | ~~`a_blacklisted_path_is_kept_by_the_cleanup` (still present) mixes a pure classifier assertion with real plan_purge/apply_purge I/O; move only the pure row into the parametrized table.~~ | T10 implementer | removed | CP4 reviewer pruned: no test-count benefit; pure+I/O layering is intentional. |
| F17 | ~~Doc-comment notes on `cached_keys_classify_every_keep_rule`: visibility computation only in scope tests; group 'everything else purges' rows under a provenance comment.~~ | T10 implementer | removed | CP4 reviewer pruned: already done by commit `564b341b` (provenance comment at lines 853–857 plus per-group comments). |
| F18 | ~~Document which `tools/sync/tests/migration_lock.rs` tests are deliberately file-backend-specific despite the file's backend-agnostic header.~~ | T11 implementer | removed | CP5 reviewer pruned: cosmetic doc note only; no coverage/test-count value. |
| F19 | ~~Pin process-level stdout-empty into a neighbouring test, then delete `a_bad_log_level_is_refused_by_the_parser`.~~ | T12 implementer | removed | CP5 reviewer resolved: implemented as fix commit `edc1d76d`, T12 now complete. |
| F20 | Restore a compact synthetic self-test proving `check_golden` REJECTS bad event sequences (reorder, duplicate terminal, misplaced progress, renamed event) - T13 removed the only such test (batch C). | T13 implementer | scheduled | Batching review kept (low priority): real gap in test-helper mutation coverage; real goldens only hit the happy path. |
| F21 | ~~Strengthen gircap/girsync env-prefix assertions to verify `girpr_log::env_options` reads those exact names end-to-end.~~ | T13 implementer | removed | CP5 reviewer pruned: tautological strengthening needing racy env mutation; env_options covered generically in crates/log. |
| F22 | ~~Remove the now-unused `default_log_path()` wrappers in girpr and girsync after T13's unit-duplicate removals.~~ | T13 implementer | removed | CP5 reviewer pruned: minor dead-code cleanup, no coverage/test-count value; girpr wrapper tied to an intra-doc link. |
| F23 | ~~If other repair integration binaries repeat the 'no getBuild / no config bump' invariant, promote `assert_metadata_phase_stopped` into `tests/common/mod.rs`.~~ | Batch B implementer | removed | Batch B reviewer pruned: precondition false - the full invariant exists only in repair_offline.rs; other binaries have isolated config.ini checks only. |
| F24 | ~~Add a process case with `--audio en-us,ja-jp` to close the F12 end-to-end gap.~~ | Batch B implementer | removed | Batch B reviewer pruned: multi-language formatting already asserted at unit level; adds a test back against the reduction goal. |
| F25 | ~~Fix the now-failing documented clippy gate: dead-code `default_log_path()` at src/logging.rs:117.~~ | Batch C implementer | removed | Batch C reviewer resolved by fix commit `58f14fc3` (removed wrapper + retargeted intra-doc link); gate now clean. Do not re-add. |
| F26 | Split `girpr-tempdir::test_support` into pure-std helpers (file/link/temp-root) vs girsync-coupled cache/option helpers, so crates that only need `TempDir` don't pull `girsync` into their test builds. | Batch A implementer | proposed | Extraction added girsync/serde_json deps to girpr-tempdir; conditional on whether that transitive dep matters. Reviewer may prune as larger churn. |
| F27 | Confine the two `#[macro_export]` macros behind a dedicated feature/module path for a stricter test-support boundary. | Batch A implementer | proposed | Requires touching call sites; implementer left as-is to preserve behavior. Reviewer likely prunes. |

## Follow-up batches

After all tasks and checkpoints are done, a review session decides how to batch
the surviving follow-ups (and may add/remove). Record the result here; implement
one batch at a time with one implementing session then one review session, using
the same separate state commits as the task/checkpoint loop. Statuses: `open` →
`in_progress` → `pending_review` → `done`.

| Batch | Follow-ups | Status | Commit range | Verdict | Scope / pointers | Notes |
|---|---|---|---|---|---|---|
| B | F11, F12 | done | 6b82c98a..614f3aa4 (2 test-only commits) | F11 completed; F12 completed; no fixes | Root repair integration cleanup: `tests/repair_offline.rs` shared assertion helper for repeated branch-issued / no-getBuild / no-config-bump checks (allow per-case omissions); `tests/repair_plan.rs` narrow `audio_langs=en-us` assertion to the no-scope case. Pointers: report §C kept L143-148, `OPTIMIZATIONS_PLAN/tasks/T08-root-integration-optional-merges.md`, `tests/repair_offline.rs:470-733`, `tests/repair_plan.rs:150-205`, `src/report/tests.rs:405-460`. No case merges, no coverage loss, no product code. | Run first (small/concrete). |
| C | F09, F20 | done | 1cbb6d3a..ce669675 + fix 58f14fc3 | F09 completed; F20 completed; F25 completed (fixed) | Coverage hardening: add `cargo nextest run --release -p girpr-log` to the documented gate in `README.md:437-456` (no CI config exists - do not create one; skip if no appropriate place); restore a compact synthetic `check_golden` rejection self-test in `tests/cli_contract.rs` (reorder, duplicate terminal, misplaced progress, renamed event => Err). Pointers: report §F.4 L218-220, T07/T13 task files, `crates/log/src/plan.rs:273-297,420-443`, `README.md:430-456`, `.config/nextest.toml`, `tests/cli_contract.rs:1702-1770`. Note F20 adds a test back (low priority). | Run second. REVIEW FLAG: implementer found the README gate's `cargo clippy -p girpr --all-targets -- -D warnings` line FAILS on a pre-existing dead-code warning for `default_log_path()` at src/logging.rs:117 (leftover F22, pruned by CP5 reviewer). F09/F20 themselves pass; reviewer must decide whether to fix this in review or re-approve as a follow-up (product-code cleanup, not test coverage). |
| A | F01 | pending_review | f9b6914e (single commit) | — | Shared tempdir helper extraction: move the verbatim `tests/common/mod.rs` from `tools/sync` and `tools/sled2redb` into the existing `girpr-tempdir` crate; rewire both test targets; leave root/capture `common/mod.rs` separate. Pointers: report cross-cutting L226-233, `tools/sync/tests/common/mod.rs`, `tools/sled2redb/tests/common/mod.rs`, `crates/tempdir/src/lib.rs`. F01 targets `common/mod.rs`, not T06's rewritten `convert.rs`. Test-only; preserve all helper behavior. | Run last (largest refactor, single item). |

## Known constraints from the report (persistent)

* The report is a snapshot: line numbers, even whole recommendations, may be
  stale. Implementers always validate against the current code before editing.
* No large block of useless tests: most tests pin a named defect. **Doing
  nothing is a defensible outcome**; do not force changes to hit a number.
* 12 `#[ignore]`d tests are KEEP (skipped by default; removing them would not
  reduce the running count). `tools/zz_probe*.rs` contain `#[test]` but are not
  Cargo test targets — leave them in place.
* "Leave alone": §C.6, §D.6, §E.6, `repair_*` integration invariants, capture
  mutation-driven HTTP/verify/emit suites, sync race/guard suites.
* §E.5 measured-fixture merges are optional (recommend KEEP).
* Do-not-merge pairs in §B.4: `repair_post.rs:512/596`,
  `repair_purge.rs:421/454`, `repair_scope.rs:212/342`,
  `repair_plan.rs:477/503` (complementary, not duplicate).
* Area overlap is counted once: all log-file-policy CLI tests (§A, §B.1 4–5,
  §D.1, §D.3) belong to **T13**. **T04** covers only the sync hash dedup table
  (§D.2). **T12** covers only capture `cli_contract.rs:214` + `closeout.rs:1000`.
  Do not create duplicate work across these tasks.

## Persistent context / decisions

Append-only notes for a replacement orchestrator: stale-report findings,
deliberate deviations from a task file, why a task was skipped/reverted,
checkpoint boundary changes, and anything else needed to resume. Write these as
short, context-only statements — no session ids, dates, machine data or
chronological change log.

* T11 skipped: report §D.4 '≈' overlaps in `migration_lock.rs` are thematic,
  not property-identical; all five candidates carry unique assertions. Do not
  revisit unless the code changes; whole-target deletion is HIGH risk.
* Watch (not a coverage issue): `migration_lock.rs` `cache_snapshot` compares
  mtime and is filesystem-timing-sensitive; note only if it flakes in CI.
* Follow-up batching review: F03 deleted (NO-OP - T06's two sled2redb conversions
  assert distinct properties, report premise stale); F09 re-scoped from "in CI"
  to the documented README.md test gate (no CI config exists in this repo).
  Surviving follow-ups batched B (F11,F12) -> C (F09,F20) -> A (F01), independent.
