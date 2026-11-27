# Refactor checklist state

Single source of truth for orchestration. Updated by the orchestrator in commits
separate from implementation commits. See `ORCHESTRATION.md` for the protocol and
`README.md` for ordering, checkpoints and coverage.

Initial state: no task started, no reviews done, no follow-ups.

## Tasks

Status values: `todo`, `in progress`, `pending review`, `completed`, `reverted`, `skipped`.

| ID | Task file | Status | Commits | Notes |
| --- | --- | --- | --- | --- |
| T01 | `tasks/T01-chunk-cost-arithmetic.md` | completed | 4 commits: chunk-cost primitives; repair-plan/chunk-verify adoption; gircap manifest totals; repair scan/plan | CP1 kept; unused import fixed in review |
| T02 | `tasks/T02-chunk-layout.md` | completed | 4 commits: chunk-layout leaf; repair-plan tiling adoption; gircap range/tiling adoption; repair/fetch reuse checks | CP1 kept |
| T03 | `tasks/T03-xxh64-hex-boundary.md` | completed | 4 commits: digest XXH64 conversions; persistent-plan/fetch adoption; cache-json hex wrappers; chunk-identity normalize_hex re-export | CP1 kept |
| T04 | `tasks/T04-digest-internals.md` | completed | 3 commits: restartable sink unification; Md5/Sha256 templating; shared buffered hash/copy loop | CP1 kept |
| T05 | `tasks/T05-retryable-status.md` | completed | 3 commits: shared retry predicate; fetch adoption; capture-http adoption | CP2 kept |
| T06 | `tasks/T06-async-retry-driver.md` | completed | 3 commits: retry-driver leaf; fetch loop adoption; capture-http loop adoption | CP2 kept |
| T07 | `tasks/T07-bounded-join.md` | completed | 4 commits: async-join leaf; capture-http ordered join; repair adoptions; test-helper downcast | CP2 kept |
| T08 | `tasks/T08-ledger-cas-helpers.md` | completed | 3 commits: counter CAS primitives; meter charge/refund/settle; RAM reserve/release | CP3 kept |
| T09 | `tasks/T09-whole-file-verify.md` | completed | 3 commits: shared whole-file gate; repair check/temp adoption; persistent-plan/fetch/sophon adoption | CP3 kept |
| T10 | `tasks/T10-bounded-byte-io.md` | completed | 3 commits: util-hash bounded read promotion; util-io leaf; repair file writer adoption | CP3 kept; clippy drop fix in review |
| T11 | `tasks/T11-clear-readonly.md` | completed | 4 commits: util-path readonly leaf; fetch/persistent-exec adoption; file/mode propagation; leaf test formatting | CP3 kept |
| T12 | `tasks/T12-request-url.md` | pending review | 5 commits: request-url grammar leaf; fetch Sophon URLs; capture biz/probes; dispatch/persistent-plan path append; fixture query lookups | CP4 batch |
| T13 | `tasks/T13-line-and-bom-readers.md` | todo | — | |
| T14 | `tasks/T14-claim-publish.md` | todo | — | |
| T15 | `tasks/T15-fs-path-helpers.md` | todo | — | |
| T16 | `tasks/T16-global-cli-and-format.md` | todo | — | before T18 |
| T17 | `tasks/T17-subscriber-and-log-internals.md` | todo | — | |
| T18 | `tasks/T18-report-core.md` | todo | — | after T16 |
| T19 | `tasks/T19-test-fixture-helpers.md` | todo | — | |
| T20 | `tasks/T20-test-http-servers.md` | todo | — | |
| T21 | `tasks/T21-hyp-wire.md` | todo | — | |
| T22 | `tasks/T22-capture-emit-stores.md` | todo | — | |
| T23 | `tasks/T23-girsync-heartbeat.md` | todo | — | |
| T24 | `tasks/T24-workspace-inheritance.md` | todo | — | |

## Reviews

| Checkpoint | Reviews | Status | Outcome |
| --- | --- | --- | --- |
| CP1 | T01–T04 | done | all kept; T01 unused-import defect fixed in review |
| CP2 | T05–T07 | done | all kept; no fixes or reverts |
| CP3 | T08–T11 | done | all kept; T10 clippy drop fix in review |
| CP4 | T12–T14 | open | |
| CP5 | T15–T16 | open | |
| CP6 | T17–T18 | open | |
| CP7 | T19–T20 | open | |
| CP8 | T21–T22 | open | |
| CP9 | T23–T24 | open | |

## Follow-ups

Status values: `proposed`, `accepted`, `declined`, `done`.

| ID | Source task | Recommendation | Status |
| --- | --- | --- | --- |
| F01 | T01 | Fold remaining i64→u64 clamps in `src/repair/reuse.rs`, `src/repair/file/{mod,temp,inplace}.rs` (incl. `staged_payload` total) and `tools/capture/src/{select/mod,select/tests,blobs/props,assemble/assembly}.rs` through chunk-cost primitives; overlaps other task boundaries. | proposed |
| F02 | T01 | T02's half-open range/tiling leaf can consume `checked_end` directly; no change needed in T01. | done |
| F03 | T02 | Give capture's A11 `Layout::of` a typed-problem adapter: adjacent-previous diagnostic semantics differ from the coverage cursor and its strings are published in `verify/selfcheck.json`, so it stayed local by contract. | proposed |
| F04 | T02 | Decide explicitly what an unrepresentable signed chunk end means for tiling; the fold clamps at `i64::MAX` (matching capture's old behaviour) and plan-time validation rejects those ranges before layout. | proposed |
| F05 | T04 | Evaluate composing `tools/capture/hash`'s `HashingFile::write` with the shared sink while keeping write-before-hash ordering at the call site. | proposed |
| F06 | T05 | Optionally collapse the thin delegating wrappers (`fetch::retryable`, `RetryPolicy::should_retry_status`) into direct `girpr_retry::should_retry_status` calls; kept local to preserve the published `http` module surface and test seams. | proposed |
| F07 | T06 | Optionally collapse `RetryPolicy::backoff()` / `BackoffPolicy::build()` schedule wrappers into direct `girpr_retry::Schedule::exponential(...)` calls once test seams are reviewed; kept local to preserve published module surfaces. | proposed |
| F08 | T06 | If in-place (not by-value) state carry is ever needed in capture, consider an HRTB/boxed-future variant of the driver; not worth the per-attempt allocation now. | proposed |
| F09 | T07 | Repair phases still render `Failed(JoinError)` with their own `Display` templates; switching to the payload-aware `join_failure` would change published error text and needs an explicit wording decision. | proposed |
| F10 | T07 | `inplace.rs::bounded_window` stays local: an ordered buffered window with no index or panic recovery and file-local reservation semantics. | proposed |
| F11 | T07 | Collapsing `spawn_blocking_traced` and the file driver's setup wrappers further would bury the load-bearing "capture at spawn time" documentation and the diagnostics-owner hook. | proposed |
| F12 | T07 | `LocalRead::join_failure` in `src/repair/phases/metadata.rs` changed its cancellation wording to "the {label} task was cancelled: {JoinError}"; no test/golden asserts the old text — decide explicitly if stderr wording is a published contract (with F09). | proposed |
| F13 | T07 | `src/repair/persistent_exec.rs` now gives a lost task a policy (`files_failed += 1`, fail-fast abort) where the old loop discarded results; confirm this deliberate behavior addition, covered by tests. | proposed |
| F14 | T08 | Consider an explicit leaf primitive returning the "within ceiling" verdict so `settle` need not pass the `u64::MAX` sentinel; current form is deliberate to keep the settled-limit rule in `Meter`. | proposed |
| F15 | T09 | `src/repair/file/temp.rs::promote_tmp` still has a hand-written length+MD5 final gate with its own delete-and-bail policy; could be routed through the shared gate if a single implementation is wanted there. | proposed |
| F16 | T09 | Consider relocating the pure whole-file gate to a lower crate; `app/persistent-plan` now depends on `girpr-chunk-verify` (transitively proto/util-hash) for a gate that needs only `std`. | proposed |
| F17 | T10 | `ChunkFileSink::write` in `temp.rs` is a decoder-fed sink, not a loop; its per-piece ceiling charge stayed local — a shared charge helper would touch the `sophon::ChunkSink` seam and retry/refund tests. | proposed |
| F18 | T10 | Consider relocating `read_up_to`/`reserve_exact` into `util-io` to make the leaf dependency-free (currently `util-io` depends on `util-hash` for one read); moves a public primitive between crates. | proposed |
| F19 | T10 | `capture_bounded` downcasts `read_up_to`'s `anyhow::Error` back to `io::Error`; an `io::Result`-typed read primitive would remove the downcast but needs an explicit decision on reserve-failure wording. | proposed |
| F20 | T11 | Add an injectable seam to force a `set_permissions` failure so the `SetFailed` warning branches can be covered directly. | proposed |
| F21 | T11 | Consider relocating the `clear_readonly` leaf to a `std`-only crate; `girpr-util-path` carries dependencies the mutation needs none of. | proposed |
| F22 | CP3 | Update the README shared-leaf table for `app/util-io`, `util-path::readonly` and the whole-file gate (the util-counter row was updated in T08). | proposed |
| F23 | T10 | A truncated in-file copy now reports the leaf wording ("in-file copy ended after X of Y bytes") instead of `read_exact`'s message; still a hard error and no contract pins the string. | proposed |
| F24 | T12 | Add a `girpr-request-url` row to the README shared-leaf table (same class as F22). | proposed |
| F25 | T12 | Consider a policy-parameterized HYP URL seam to unify fetch's and capture's HYP builders; deliberately not done because girpr's hardcoded/unencoded language spelling and capture's `needs_channel`/override policy are call-site contracts. | proposed |

## Persistent context

- Report: `docs/09-shared-code-refactor-findings.md` (temporary, deleted at finish).
- Every finding is mapped to a task in `README.md`; non-candidates are listed there.
- LF-14's cross-reference to X14 is a pointer, not a grouping; LF-14 is in T17.
- Findings are static readings; implementers must validate against current code.
- These plan files and the report must not be referenced from code or permanent docs.
- Report line numbers are shifted relative to the current tree; implementers validate named sites against code, not cited lines.
- F01's remaining single i64→u64 clamps overlap other task boundaries; they were left local in T01.
- T01 and T02 implemented and validated (see task table); awaiting CP1 review.
- CP1 note: T02 reports a leftover unused `saturating_nonneg_sum` import in `src/repair/scan.rs` from T01; reviewer should fix it as a T01 defect.
- T02's tiling leaf consumes T01's `checked_end` (F02 done) and clamps unrepresentable signed ends at `i64::MAX`, matching capture's prior behaviour.
- T03 implemented and validated; no follow-ups proposed. Remaining `{:016x}` sites in gircap are opaque manifest-id prefixes, not hashes.
- T04 implemented and validated; `Sha256::try_parse_hex` is additive and public paths are unchanged.
- CP1 done: T01–T04 kept; the T01 unused `saturating_nonneg_sum` import was scoped to the test module in review. Follow-ups F01/F03/F04/F05 remain open.
- CP1 minor observations, no action taken: a redundant `RangeProblem::NegativeOffset` match arm in `manifest/proto.rs`; `hash_file_with_buf` opens the file before the leaf's empty-buffer check, so an empty-buffer-plus-missing-file error reports the open failure.
- T05 finding: the report's out-of-range-u16 discrepancy claim does not hold in this tree (http 1.5.0 `from_u16` accepts 100..=999 and `is_server_error` is 500..600); the shared 500..=599 rule preserves prior behavior exactly.
- T06 note: the report's 897-898 "retry drivers stay local" note does not block the task — the new leaf holds no classification or side effects, and jitter/logging/error types stayed in the products.
- T06 observation: clippy reports one warning in `crates/chunk-layout` (T02's crate, already reviewed at CP1); not re-opened.
- T07 guardrails held: capture's buffered-future panics and repair's `Handle::block_on` cancellation remain distinct; dispatcher and `DiagnosticsOwner` survive via the thread-setup hook; published fallback strings unchanged.
- CP2 done: T05–T07 kept; no fixes or reverts; workspace 2297 passed / 8 skipped; only the out-of-batch chunk-layout clippy warning remains.
- T08 keeper: the remaining `stopped_by` CAS in `app/report/src/ledger.rs` is an idempotent first-wins flag, not a saturating tally; routed through no leaf deliberately.
- T09 keeper: `src/repair/file/inplace.rs` scanned-verdict reuse digest comparisons remain local `matches_hex` calls by the reuse-guard contract.
- T10 boundary: `inplace.rs` was not edited — it already calls the `mod.rs` wrappers that delegate to the leaf and downloads through `temp.rs`, so it consumes both mechanics transitively.
- CP3 done: T08/T09/T10/T11 kept; T10's `drop(rw)` calls removed for clippy; only the out-of-batch chunk-layout `collapsible_if` warning remains when clippy runs with -D warnings.
- T12 keeper: CAP-06 blob-id shape helpers left local by guardrail — tightening girpr to capture's strict rule risks rejecting real ids.
