# girsync optimization checklist

This file is the orchestrator's committed state. Only the orchestrator edits
it. Implementing and reviewing sessions receive it read-only.

The YAML block is the source of truth. The table below it is a human view; if
the two disagree, the YAML block wins.

```yaml
schema: 1
project: girsync
plan_dir: tools/sync/OPTIMIZATIONS_PLAN
report: tools/sync/OPTIMIZATIONS_REPORT.md
phase: implement
active_batch: C
spawn: openchamber session.create (agent build, model opencode/minimax-m3.1-flash-free), working dir = repo root E:\hoccode\hocrust\girpr, one session at a time
last_updated: 2026-11-11T00:26:52Z
tasks:
  - id: T01
    title: By-name digest lookup and its test
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T01-by-name-digest-lookup.md
    batch: A
    report: [L59-L87, L572-L575]
    status: completed
    commits: [0e9e49c3, e87f89af]
    review: A
    note: review A kept; no fix or revert commits
  - id: T02
    title: Plan-time early returns and prefix memoization
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T02-plan-time-early-returns.md
    batch: A
    report: [L191-L206, L344-L368, L584-L586]
    status: completed
    commits: [df473b05, 43f5620d, 1c6110b5, bfa650ff]
    review: A
    note: review A kept; no fix or revert commits
  - id: T03
    title: Document the case-fold rules
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T03-one-case-fold.md
    batch: B
    report: [L88-L116, L576-L579]
    status: completed
    commits: [e3c1416a, 022792ae, 91bccb15]
    review: B
    note: review B kept; the extra commit and the one-time amend judged a process deviation only, content sound
  - id: T04
    title: Shared one-side case index
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T04-shared-case-index.md
    batch: B
    report: [L216-L244]
    status: completed
    commits: [2078adde, 6b27a8ca, 778d99d2, b95e9fd4]
    review: B
    note: review B kept; commit reorder and the clippy commit judged justified
  - id: T05
    title: Cache meta open fast path
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T05-cache-meta-fast-path.md
    batch: B
    report: [L145-L164]
    status: completed
    commits: [4333d9e1, 477f1e61]
    review: B
    note: review B kept; the entries-table check judged a strengthening, not a scope break
  - id: T06
    title: Copy-path round trips
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T06-copy-path-round-trips.md
    batch: C
    report: [L306-L342]
    status: pending_review
    commits: [c648ae6d, 71f72513, 1f86077f]
    review: C
    note: implemented by ses_e34d04059ffe9JFJSKWLsbXy48; the conditional third commit was used for a new end-to-end mtime test
  - id: T07
    title: Cache scan consolidation
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T07-cache-scan-consolidation.md
    batch: C
    report: [L280-L304, L580-L583]
    status: pending_review
    commits: [9bb075e6, ed6c2745]
    review: C
    note: implemented by ses_e346c48c9ffeqd376JFuF6RZ6g; the no-op case is observed with read-transaction counters, and the no-op test lives in the crate because the counter is crate-private
  - id: T08
    title: Phase A one tree pass
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T08-phase-a-one-pass.md
    batch: C
    report: [L246-L278]
    status: pending
    commits: []
    review: C
    note: null
  - id: T09
    title: Backup policy and the failure story
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T09-backup-policy.md
    batch: D
    report: [L118-L143, L542-L548, L557-L563]
    status: pending
    commits: []
    review: D
    note: null
  - id: T10
    title: Cache memory knob and one test-cache decision
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T10-cache-memory-and-test-cache.md
    batch: D
    report: [L370-L388, L536-L540, L550-L555]
    status: pending
    commits: []
    review: D
    note: null
  - id: T11
    title: Small performance notes
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T11-small-performance-notes.md
    batch: D
    report: [L390-L409]
    status: pending
    commits: []
    review: D
    note: null
  - id: T12
    title: Filter bundle and pre-built side labels
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T12-filter-bundle-and-labels.md
    batch: E
    report: [L419-L435, L495-L511]
    status: pending
    commits: []
    review: E
    note: null
  - id: T13
    title: Decompose copy_one around its guards
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T13-copy-one-decomposition.md
    batch: E
    report: [L437-L456]
    status: pending
    commits: []
    review: E
    note: null
  - id: T14
    title: sled2redb uses clap
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T14-sled2redb-cli.md
    batch: F
    report: [L478-L493]
    status: pending
    commits: []
    review: F
    note: null
  - id: T15
    title: Named sync phases decision
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T15-named-sync-phases.md
    batch: F
    report: [L458-L476]
    status: pending
    commits: []
    review: F
    note: null
  - id: T16
    title: Reconcile documentation with the code
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T16-docs-reconcile.md
    batch: F
    report: [L513-L531]
    status: pending
    commits: []
    review: F
    note: null
  - id: T17
    title: Converter atomicity and failure tests
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T17-converter-atomicity.md
    batch: G
    report: [L166-L189, L587-L589]
    status: pending
    commits: []
    review: G
    note: null
  - id: T18
    title: Feature-gate sled
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T18-feature-gate-sled.md
    batch: G
    report: [L597-L610]
    status: pending
    commits: []
    review: G
    note: null
  - id: T19
    title: Benchmark harness over three tree shapes
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T19-benchmark-harness.md
    batch: G
    report: [L590-L593]
    status: pending
    commits: []
    review: G
    note: null
reviews:
  - batch: A
    status: done
    tasks: [T01, T02]
    base: 8b40214f
    head: bfa650ff
    result: T01 kept (0e9e49c3, e87f89af); T02 kept (df473b05, 43f5620d, 1c6110b5, bfa650ff); no fix or revert commits; 465 passed, 1 skipped; the two new T01 tests verified failing on the pre-fix code
    followups: [F1, F2, F3, F4, F5]
  - batch: B
    status: done
    tasks: [T03, T04, T05]
    base: e3f3e9e3
    head: 477f1e61
    result: T03 kept (e3c1416a, 022792ae, 91bccb15); T04 kept (2078adde, 6b27a8ca, 778d99d2, b95e9fd4); T05 kept (4333d9e1, 477f1e61); no fix or revert commits; 473 passed, 1 skipped; the T03 and T04 deviations judged justified
    followups: [F6, F7, F8]
  - batch: C
    status: pending
    tasks: [T06, T07, T08]
    base: null
    head: null
    result: null
    followups: []
  - batch: D
    status: pending
    tasks: [T09, T10, T11]
    base: null
    head: null
    result: null
    followups: []
  - batch: E
    status: pending
    tasks: [T12, T13]
    base: null
    head: null
    result: null
    followups: []
  - batch: F
    status: pending
    tasks: [T14, T15, T16]
    base: null
    head: null
    result: null
    followups: []
  - batch: G
    status: pending
    tasks: [T17, T18, T19]
    base: null
    head: null
    result: null
    followups: []
followups:
  - id: F1
    source: T01
    description: foreign_known resolves names through HashAlg::from_str, whose error path allocates an UnknownHash string for every non-matching foreign name; an ASCII-case comparison against the canonical spelling would be allocation-free.
    status: pending
    batch: null
    commits: []
  - id: F2
    source: T01
    description: DESIGN-NOTES "A digest is looked up by name, not by slot" still says "two lookups" and does not mention get_algo or the ASCII-case widening; T16 may already cover this reconciliation.
    status: pending
    batch: null
    commits: []
  - id: F3
    source: T02
    description: spare_link_storage no longer canonicalizes the dst root when there is no kept link, so an unresolvable dst root surfaces later instead of failing at plan time with "resolve dst root".
    status: pending
    batch: null
    commits: []
  - id: F4
    source: T02
    description: The copy-free early return drops a possible plan-time IO error from stat'ing a planned delete/fix-dir target that a copy-free run used to surface.
    status: pending
    batch: null
    commits: []
  - id: F5
    source: T02
    description: a_prefix_is_asked_about_once_per_call pins the PrefixStats memo directly rather than observing the guard's probe count; an end-to-end proof would need an injectable stat hook.
    status: pending
    batch: null
    commits: []
  - id: F6
    source: T03
    description: DESIGN-NOTES.md still says "Seven of them" about the load-bearing invariants, but there are now eleven such sections; the count was already stale before this task.
    status: pending
    batch: null
    commits: []
  - id: F7
    source: T05
    description: A cache holding a meta row but no ENTRIES table still does not get one created, because the write path creates it inside a transaction it then drops uncommitted; the new entries_present guard only steers such a cache to the write path rather than repairing it.
    status: pending
    batch: null
    commits: []
  - id: F8
    source: T05
    description: Plan bookkeeping only, not a code change: the T05 task file's design_notes pointer resolves to README.md, not to DESIGN-NOTES.md, which never mentions the cache's meta row.
    status: pending
    batch: null
    commits: []
```

## Human checklist

| batch | id | status | commits | note |
| --- | --- | --- | --- | --- |
| A | T01 | completed | 0e9e49c3, e87f89af | review A kept |
| A | T02 | completed | df473b05, 43f5620d, 1c6110b5, bfa650ff | review A kept |
| B | T03 | completed | e3c1416a, 022792ae, 91bccb15 | review B kept |
| B | T04 | completed | 2078adde, 6b27a8ca, 778d99d2, b95e9fd4 | review B kept |
| B | T05 | completed | 4333d9e1, 477f1e61 | review B kept |
| C | T06 | pending_review | c648ae6d, 71f72513, 1f86077f | implemented by ses_e34d04059ffe9JFJSKWLsbXy48 |
| C | T07 | pending_review | 9bb075e6, ed6c2745 | implemented by ses_e346c48c9ffeqd376JFuF6RZ6g |
| C | T08 | pending | — | — |
| D | T09 | pending | — | — |
| D | T10 | pending | — | — |
| D | T11 | pending | — | — |
| E | T12 | pending | — | — |
| E | T13 | pending | — | — |
| F | T14 | pending | — | — |
| F | T15 | pending | — | — |
| F | T16 | pending | — | — |
| G | T17 | pending | — | — |
| G | T18 | pending | — | — |
| G | T19 | pending | — | — |

## Transition log

Newest first. One line per committed state change: date, task/review, from → to,
commit.

- 2026-11-12 — T07 implemented: `pending` → `in_progress` → `pending_review`,
  commits `9bb075e6`, `ed6c2745`; working tree clean.
- 2026-11-12 — T07 selected: `pending` → `in_progress`.
- 2026-11-12 — T06 implemented: `pending` → `in_progress` → `pending_review`,
  commits `c648ae6d`, `71f72513`, `1f86077f`; working tree clean.
- 2026-11-11 — batch C started: T06 `pending` → `in_progress`.
- 2026-11-11 — review B `done`: T03 kept, T04 kept, T05 kept, no fix or revert
  commits; follow-ups F6–F8 appended with status `pending`; T03, T04 and T05
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: B` → `C`.
- 2026-11-11 — batch B review session ses_e36131fa8ffekFiiNAc8v61wo2 failed
  twice (ECONNRESET, then a generic failure); it stayed frozen with no
  commits and a clean tree, so it was abandoned and a new review session was
  spawned with the same prompt. Base and head are unchanged.
- 2026-11-11 — batch B review checkpoint opened: `phase: implement` →
  `review`, review B `pending` → `in_progress`, base `e3f3e9e3`, head
  `477f1e61`. T05 implemented: `pending` → `in_progress` → `pending_review`,
  commits `4333d9e1`, `477f1e61`; working tree clean.
- 2026-11-11 — T05 retry: `pending` → `in_progress`.
- 2026-11-11 — T05 session ses_e37a0ed8affeNu57TQG5mQAgMa froze on a network
  error: no commits, clean tree, so T05 `in_progress` → `pending` for a retry.
- 2026-11-11 — T05 selected: `pending` → `in_progress`.
- 2026-11-11 — T04 implemented: `pending` → `in_progress` → `pending_review`,
  commits `2078adde`, `6b27a8ca`, `778d99d2`, `b95e9fd4`; working tree clean.
- 2026-11-11 — T04 selected: `pending` → `in_progress`.
- 2026-11-11 — T03 implemented: `pending` → `in_progress` → `pending_review`,
  commits `e3c1416a`, `022792ae`, `91bccb15`; working tree clean.
- 2026-11-11 — batch B started: T03 `pending` → `in_progress`.
- 2026-11-11 — review A `done`: T01 kept, T02 kept, no fix or revert commits;
  follow-ups F1–F5 appended with status `pending`; T01 and T02
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: A` → `B`.
- 2026-11-11 — batch A review checkpoint opened: `phase: implement` →
  `review`, review A `pending` → `in_progress`, base `8b40214f`, head
  `bfa650ff`. T02 implemented: `pending` → `in_progress` → `pending_review`,
  commits `df473b05`, `43f5620d`, `1c6110b5`, `bfa650ff`; working tree clean.
- 2026-11-11 — T02 selected: `pending` → `in_progress`.
- 2026-11-11 — T01 implemented: `pending` → `in_progress` → `pending_review`,
  commits `0e9e49c3`, `e87f89af`; working tree clean.
- 2026-11-11 — spawn mechanism settled (`openchamber session.create`, agent
  `build`, repo root as working directory, one session at a time).
  `phase: blocked-on-spawn` → `implement`; T01 `pending` → `in_progress`.
- 2026-11-11 — plan revision against the post-merge tree (`opt/girpr`, merge
  `a783a9e5`). T03 rescoped to "Document the case-fold rules" (the merge
  landed `util::path_key`), T01 re-grounded on the current `get_algo` /
  `get_named` behaviour, T18's `windows-sys` trim step withdrawn (the feature
  is required), T12 corrected so `case_sensitive` stays on `CommonOpts`, T10
  re-pointed at the command-start resolution, and every verification block
  moved to `cargo nextest run`. No task's status changed; all are still
  `pending`.
- (not started) — `phase: blocked-on-spawn`; waiting for the user's spawn
  mechanism.
