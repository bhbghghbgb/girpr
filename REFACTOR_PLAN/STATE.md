# Refactor state

Single source of truth for orchestration. The orchestrating session may be replaced at
any time; all durable knowledge lives here. See `ORCHESTRATION.md` for the procedure
and `CONVENTIONS.md` for the format.

States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
`skipped`. Every change to this file is its own commit. Commit refs are pointers for
reviewers only — no dates, session/tool IDs, machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Consolidate root integration test binaries | completed | 080a0d45 | 4 root targets; kept artifact_bridge & repair_post standalone (pinned filters/host-skip/log-capture) so no nextest.toml edit; consolidated FQNs gained module prefix (repair::/contracts::); run_lock uses crate::common |
| T02 | Memory cache seam for girsync record/FS cases | pending_review | b54115fd, 60382ea2 | 2 commits (seam + migration); file opens 302→183 (~39%), sync wall 62.6→41-46s; dry-run pairs skip cache writes so no forget call needed (wrapper exported); kept 4 cache-file-subject cases on file backend; F31 falls out for migrated cases; cold-cache pattern = rw_threads/limiter.rs drop_caches() |
| T03 | Structured repair diagnostics to drop log-only spawns | todo | | |
| T04 | Pure log-announcement helper to drop cli_contract spawns | todo | | |
| T05 | Replace wiremock with a std-only loopback HTTP mock | todo | | |
| T06 | Trial rust-lld for the test profile | todo | | |
| T07 | Pin the stable toolchain to an exact release | todo | | |
| T08 | Narrow `girpr-util` rebuild edges | todo | | |
| T09 | Narrow the small rebuild-locality edges | todo | | |
| T10 | Gate the fully-ignored `bundle_shapes` binary | todo | | |
| T11 | Root dev-dependency hygiene and frozen manifest bodies | todo | | |
| T12 | Root `src/` unit-test cleanups | todo | | |
| T13 | Collapse duplicated root integration fixtures | todo | | |
| T14 | Group `artifact_server.rs` bundle tests | todo | | |
| T15 | Remove cross-file duplicate process-level assertions | todo | | |
| T16 | Fold tiny pure-test clusters | todo | | |
| T17 | Leaf-crate test cleanups | todo | | |
| T18 | Reduce row-reading redb reopens | todo | | |
| T19 | Shrink the sync-cache digest fixture and add `dirs()` | todo | | |
| T20 | Merge pure gircap test clusters | todo | | |
| T21 | Use `shared_bundle` for read-only capture tests; drop `fs2` | todo | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01 | done | |
| B | T02 | pending | |
| C | T03, T04 | pending | Split if the batch proves too heavy |
| D | T05 | pending | |
| E | T06, T07 | pending | |
| F | T08 | pending | |
| G | T09, T10, T11 | pending | |
| H | T12, T13 | pending | |
| I | T14, T15 | pending | |
| J | T16, T17 | pending | |
| K | T18, T19 | pending | Split if T18 is pre-satisfied by T02 |
| L | T20, T21 | pending | |

## Follow-ups

No follow-ups yet. Only a review session creates or removes follow-ups; the
orchestrator records them here. Planning review (Phase 3) decides batching and drops.

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU01 | Re-measure clean-build wall time on CI with a fresh target dir | open | Report figure is host-specific; 14→4 link reduction verified structurally |
| FU02 | Merging repair_post into repair needs pinned binary(=repair_post) overrides updated in same change | open | Correctly left separate in T01 |
| FU03 | Narrow blanket allow(dead_code, unused_imports) in tests/common/mod.rs to per-item allows | open | Cosmetic only |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code should record that in
the relevant task's `note` instead of silently proceeding):

- **Reports are static, host-specific claims (§1, lines 12–42; §6, lines 503–510).**
  Every finding is a hypothesis to validate against the current code; confirm
  before/after wall and clean-build times and that the semantic assertion survives a
  merge. Line numbers may have drifted; trust the code.
- **One process per test under nextest (§A1, lines 46–52).** A process-wide cache,
  `OnceLock` or static fixture helps only within one test. Merging test functions
  saves spawns; parameterized cases add spawns. Do not try to share fixtures across
  tests.
- **Rebuild and split payoffs (§A2, lines 54–59; §A3, lines 61–65).** Editing a file
  invalidates its target and downstream dependents. A split pays off only when a
  high-level consumer depends directly on the smaller crate; a re-exporting facade is
  not a win. Several findings are turnaround wins invisible in a feature-unified
  workspace run (§A11, lines 112–116) — measure them with `-p <crate>`.
- **Already-done policy is not a candidate (§A4, lines 67–73; §5, lines 486–501).**
  No-fsync and scratch policy, tiny fixtures, leaf extraction already landed, and
  `profile.test` tuning are existing facts. Do not re-add sleeps or the "without
  fail-fast"/"without terminal 404" control runs; do not scale fixtures down further.
- **No sleeps (§A6, lines 82–89).** The suites contain no sleeps; guards wake on
  notify/channel and never fire on a passing run. Adding sleeps is a regression.
- **Host dependence (§A7, lines 91–95).** Linker, cache opens and Windows process
  creation dominate on the research host; other hosts rank findings differently. A
  local measurement is a hypothesis for CI, not a universal result.
- **Wiremock's footprint (§A9, lines 101–106).** wiremock 0.6 exclusively pulls
  `regex`, `futures`, `deadpool`, `assert-json-diff` and `base64 0.22`. A std-only
  loopback mock pattern already exists in the tree; keep the new mock dependency-free
  so `girpr-fetch` and `girpr-test-fixtures` can use it without a cycle.
- **nextest name filters are pinned (§A10, lines 108–110).** Renaming, moving or
  merging a named binary or fully-qualified test path requires editing
  `.config/nextest.toml` in the same change.
- **Explicitly not recommended (§5, lines 486–501) and the non-candidate findings
  (F7, F29, F34, F37) are out of scope.** See `guidance/DO-NOT-TOUCH.md`. Disk
  maintenance (F5, F24) is not tracked as a task; it changes no version-controlled
  artifact.
