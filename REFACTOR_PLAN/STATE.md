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
| T05 | `tasks/T05-retryable-status.md` | pending review | 3 commits: shared retry predicate; fetch adoption; capture-http adoption | resolves T06 dependency |
| T06 | `tasks/T06-async-retry-driver.md` | pending review | 3 commits: retry-driver leaf; fetch loop adoption; capture-http loop adoption | CP2 batch complete |
| T07 | `tasks/T07-bounded-join.md` | todo | — | |
| T08 | `tasks/T08-ledger-cas-helpers.md` | todo | — | |
| T09 | `tasks/T09-whole-file-verify.md` | todo | — | |
| T10 | `tasks/T10-bounded-byte-io.md` | todo | — | |
| T11 | `tasks/T11-clear-readonly.md` | todo | — | |
| T12 | `tasks/T12-request-url.md` | todo | — | |
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
| CP2 | T05–T07 | open | |
| CP3 | T08–T11 | open | |
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
