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
phase: blocked-on-spawn
active_batch: A
spawn: null
last_updated: null
tasks:
  - id: T01
    title: By-name digest lookup and its test
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T01-by-name-digest-lookup.md
    batch: A
    report: [L59-L87, L572-L575]
    status: pending
    commits: []
    review: A
    note: null
  - id: T02
    title: Plan-time early returns and prefix memoization
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T02-plan-time-early-returns.md
    batch: A
    report: [L191-L206, L344-L368, L584-L586]
    status: pending
    commits: []
    review: A
    note: null
  - id: T03
    title: One documented case fold
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T03-one-case-fold.md
    batch: B
    report: [L88-L116, L576-L579]
    status: pending
    commits: []
    review: B
    note: null
  - id: T04
    title: Shared one-side case index
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T04-shared-case-index.md
    batch: B
    report: [L216-L244]
    status: pending
    commits: []
    review: B
    note: null
  - id: T05
    title: Cache meta open fast path
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T05-cache-meta-fast-path.md
    batch: B
    report: [L145-L164]
    status: pending
    commits: []
    review: B
    note: null
  - id: T06
    title: Copy-path round trips
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T06-copy-path-round-trips.md
    batch: C
    report: [L306-L342]
    status: pending
    commits: []
    review: C
    note: null
  - id: T07
    title: Cache scan consolidation
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T07-cache-scan-consolidation.md
    batch: C
    report: [L280-L304, L580-L583]
    status: pending
    commits: []
    review: C
    note: null
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
    title: Feature-gate sled and trim windows-sys features
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
    status: pending
    tasks: [T01, T02]
    base: null
    head: null
    result: null
    followups: []
  - batch: B
    status: pending
    tasks: [T03, T04, T05]
    base: null
    head: null
    result: null
    followups: []
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
followups: []
```

## Human checklist

| batch | id | status | commits | note |
| --- | --- | --- | --- | --- |
| A | T01 | pending | — | — |
| A | T02 | pending | — | — |
| B | T03 | pending | — | — |
| B | T04 | pending | — | — |
| B | T05 | pending | — | — |
| C | T06 | pending | — | — |
| C | T07 | pending | — | — |
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

- (not started) — `phase: blocked-on-spawn`; waiting for the user's spawn
  mechanism.
