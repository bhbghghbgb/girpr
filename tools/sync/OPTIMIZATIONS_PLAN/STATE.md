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
active_batch: F
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
    status: completed
    commits: [c648ae6d, 71f72513, 1f86077f]
    review: C
    note: review C kept; the conditional third commit accepted as the end-to-end mtime test the task asks for
  - id: T07
    title: Cache scan consolidation
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T07-cache-scan-consolidation.md
    batch: C
    report: [L280-L304, L580-L583]
    status: completed
    commits: [9bb075e6, ed6c2745]
    review: C
    note: review C kept; the counter-based observation and the in-crate no-op test accepted as forced by the crate-private counter
  - id: T08
    title: Phase A one tree pass
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T08-phase-a-one-pass.md
    batch: C
    report: [L246-L278]
    status: completed
    commits: [a545f9cd, 7b31a903]
    review: C
    note: review C kept; the wider ancestor pull-in judged correct and necessary, the task sketch's files-only shape proven to change the map
  - id: T09
    title: Backup policy and the failure story
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T09-backup-policy.md
    batch: D
    report: [L118-L143, L542-L548, L557-L563]
    status: completed
    commits: [42995b17, 86d2a027, 4692f4d9, 54b726d6, 5cdecf09, ce40c153]
    review: D
    note: review D fixed (ce40c153 corrected one inaccuracy in the failure story about what a failed --ignore-cache removal leaves behind)
  - id: T10
    title: Cache memory knob and one test-cache decision
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T10-cache-memory-and-test-cache.md
    batch: D
    report: [L370-L388, L536-L540, L550-L555]
    status: completed
    commits: [627407c3, 9a8c920e, 6c517a71, 5f5d7388, df31d02a]
    review: D
    note: review D kept; the Option<usize> parser and the saturation branch both judged within the task's latitude
  - id: T11
    title: Small performance notes
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T11-small-performance-notes.md
    batch: D
    report: [L390-L409]
    status: completed
    commits: [ef0d1bea, a8ede30e, 92b119ab, 65fa3190]
    review: D
    note: review D kept; the two-hook copy-failure case judged the right call
  - id: T12
    title: Filter bundle and pre-built side labels
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T12-filter-bundle-and-labels.md
    batch: E
    report: [L419-L435, L495-L511]
    status: completed
    commits: [6b50b4d0, 1e0075c5, d576a382]
    review: E
    note: review E kept; behaviour verified against a base binary over 14 filter and command combinations with 0 differences
  - id: T13
    title: Decompose copy_one around its guards
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T13-copy-one-decomposition.md
    batch: E
    report: [L437-L456]
    status: completed
    commits: [ce3f0f94, f37e231f, b5cea4e0]
    review: E
    note: review E kept; every bail! and with_context string byte-identical to the pre-refactor file, and the copy path verified by differential against a base binary
  - id: T14
    title: sled2redb uses clap
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T14-sled2redb-cli.md
    batch: F
    report: [L478-L493]
    status: pending_review
    commits: [2df937bf, fb913a2a, a2df3d11]
    review: F
    note: implemented by ses_e323038fbffedCTPykVAhY4s4l; no --version flag was added, so an unknown flag exits 3 as before
  - id: T15
    title: Named sync phases decision
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T15-named-sync-phases.md
    batch: F
    report: [L458-L476]
    status: pending_review
    commits: [af640af6]
    review: F
    note: implemented by ses_e3226296bffeAnLUMqf31diRVn; the decision is not to extract cmd_sync into phase functions, so the change is one docs commit naming the phases and their ordering invariants
  - id: T16
    title: Reconcile documentation with the code
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T16-docs-reconcile.md
    batch: F
    report: [L513-L531]
    status: in_progress
    commits: []
    review: F
    note: implementing session starting
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
    status: done
    tasks: [T06, T07, T08]
    base: 52af2907
    head: 7b31a903
    result: T06 kept (c648ae6d, 71f72513, 1f86077f); T07 kept (9bb075e6, ed6c2745); T08 kept (a545f9cd, 7b31a903); no fix or revert commits; 485 passed, 1 skipped; the T08 deviation proven required by patching the new code to the task sketch
    followups: [F9, F10, F11, F12, F13, F14]
  - batch: D
    status: done
    tasks: [T09, T10, T11]
    base: 84a3edc5
    head: ce40c153
    result: T09 fixed (ce40c153 corrects the failure story about what a failed --ignore-cache removal leaves behind); T10 kept (627407c3, 9a8c920e, 6c517a71, 5f5d7388, df31d02a); T11 kept (ef0d1bea, a8ede30e, 92b119ab, 65fa3190); 505 passed, 1 skipped; the never-auto-read and current-is-authoritative claims verified in code and end to end
    followups: [F15, F16, F17, F18, F19, F20]
  - batch: E
    status: done
    tasks: [T12, T13]
    base: 99cacd99
    head: b5cea4e0
    result: T12 kept (6b50b4d0, 1e0075c5, d576a382); T13 kept (ce3f0f94, f37e231f, b5cea4e0); no fix or revert commits; 516 passed, 1 skipped; both refactors verified behaviour-neutral by differential against a base binary
    followups: [F21, F22, F23, F24]
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
  - id: F9
    source: T08
    description: The comment above the ancestor walk says the directories above an entry are "part of the path the filter kept", which is exact for an included file but not for an included directory, whose own ancestors are pulled in without being kept; a sharper sentence would protect the map identity the fix-up exists to preserve.
    status: pending
    batch: null
    commits: []
  - id: F10
    source: T08
    description: is_excluded still rebuilds MatchOptions per entry and re-tests every exclude against every ancestor prefix; the adjacent phase-A micro-opportunity is still unaddressed.
    status: pending
    batch: null
    commits: []
  - id: F11
    source: T07
    description: The no-op test observes only the re-key pass's read-transaction count, not the run's total cache scans, so the broader claim that a no-op run does not scan the cache more than once is still unpinned.
    status: pending
    batch: null
    commits: []
  - id: F12
    source: T07
    description: CacheDb::read_transaction_count counts every read transaction on the handle rather than only entries-table reads, so a future read on another table would silently move the number the new tests pin.
    status: pending
    batch: null
    commits: []
  - id: F13
    source: T07
    description: get_many allocates a String per key; a borrow-shaped variant would avoid the copies for a caller that only needs the rows.
    status: pending
    batch: null
    commits: []
  - id: F14
    source: T06
    description: copy_one now carries both an AlgoSet and the request slice it is derived from, and nothing prevents the two from drifting apart.
    status: pending
    batch: null
    commits: []
  - id: F15
    source: T09
    description: util::unique_sibling now has no callers left in the crate and should be deleted or given one.
    status: pending
    batch: null
    commits: []
  - id: F16
    source: T09
    description: --backup-retain prunes only after a successful copy, so a folder whose cache is absent, or a --no-backup run, never prunes the copies already sitting beside it.
    status: pending
    batch: null
    commits: []
  - id: F17
    source: T09
    description: Retention can unlink a sibling a concurrent run is still copying into, so that run's backup can vanish while the run reports success.
    status: pending
    batch: null
    commits: []
  - id: F18
    source: T10
    description: The test the_default_is_the_size_without_a_variable no longer matches what it asserts after the fifth commit, which now pins the constant only.
    status: pending
    batch: null
    commits: []
  - id: F19
    source: T11
    description: The walk_live files/dirs counters are unpinned by any test; the crate's --log-file JSON channel, already used by tests/output.rs, could assert the scan done event.
    status: pending
    batch: null
    commits: []
  - id: F20
    source: T11
    description: The lowest-index copy-failure case discriminates the two orders only because a 1 MiB copy is slower than the injected failure, so it can pass vacuously on a very fast host.
    status: pending
    batch: null
    commits: []
  - id: F21
    source: T12
    description: tests/sync.rs:1794 still writes o.common.case_sensitive = false directly instead of set_case_sensitive(false), which leaves the bundled filter's cached options on the old mode; harmless only because that run's filter has no patterns.
    status: pending
    batch: null
    commits: []
  - id: F22
    source: T12
    description: compare labels a record side "record <path>" while compare-self labels it "record <path> (for <folder>)", so the same side is named two ways.
    status: pending
    batch: null
    commits: []
  - id: F23
    source: T12
    description: Filter is reachable only as girsync::filter::Filter while CommonOpts is re-exported at the crate root, so a hand-built CommonOpts literal must name a second module path for its new field's type.
    status: pending
    batch: null
    commits: []
  - id: F24
    source: T13
    description: copy_one still carries both algos: AlgoSet and the requested: &[HashAlg] slice derived from it, and nothing prevents the two from drifting.
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
| C | T06 | completed | c648ae6d, 71f72513, 1f86077f | review C kept |
| C | T07 | completed | 9bb075e6, ed6c2745 | review C kept |
| C | T08 | completed | a545f9cd, 7b31a903 | review C kept |
| D | T09 | completed | 42995b17, 86d2a027, 4692f4d9, 54b726d6, 5cdecf09, ce40c153 | review D fixed (ce40c153) |
| D | T10 | completed | 627407c3, 9a8c920e, 6c517a71, 5f5d7388, df31d02a | review D kept |
| D | T11 | completed | ef0d1bea, a8ede30e, 92b119ab, 65fa3190 | review D kept |
| E | T12 | completed | 6b50b4d0, 1e0075c5, d576a382 | review E kept |
| E | T13 | completed | ce3f0f94, f37e231f, b5cea4e0 | review E kept |
| F | T14 | pending_review | 2df937bf, fb913a2a, a2df3d11 | implemented by ses_e323038fbffedCTPykVAhY4s4l |
| F | T15 | pending_review | af640af6 | implemented by ses_e3226296bffeAnLUMqf31diRVn |
| F | T16 | in_progress | — | implementing session starting |
| G | T17 | pending | — | — |
| G | T18 | pending | — | — |
| G | T19 | pending | — | — |

## Transition log

Newest first. One line per committed state change: date, task/review, from → to,
commit.

- 2026-11-12 — T16 selected: `pending` → `in_progress`.
- 2026-11-12 — T15 implemented: `pending` → `in_progress` → `pending_review`,
  commit `af640af6`; working tree clean.
- 2026-11-12 — T15 selected: `pending` → `in_progress`.
- 2026-11-12 — T14 implemented: `pending` → `in_progress` → `pending_review`,
  commits `2df937bf`, `fb913a2a`, `a2df3d11`; working tree clean.
- 2026-11-12 — batch F started: T14 `pending` → `in_progress`.
- 2026-11-12 — review E `done`: T12 kept, T13 kept, no fix or revert commits;
  follow-ups F21–F24 appended with status `pending`; T12 and T13
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: E` → `F`. A stale detached worktree left by an earlier
  session was removed.
- 2026-11-12 — batch E review checkpoint opened: `phase: implement` →
  `review`, review E `pending` → `in_progress`, base `99cacd99`, head
  `b5cea4e0`. T13 implemented: `pending` → `in_progress` → `pending_review`,
  commits `ce3f0f94`, `f37e231f`, `b5cea4e0`; working tree clean.
- 2026-11-12 — T13 selected: `pending` → `in_progress`.
- 2026-11-12 — T12 implemented: `pending` → `in_progress` → `pending_review`,
  commits `6b50b4d0`, `1e0075c5`, `d576a382`; working tree clean.
- 2026-11-12 — batch E started: T12 `pending` → `in_progress`.
- 2026-11-12 — review D `done`: T09 fixed (ce40c153), T10 kept, T11 kept;
  follow-ups F15–F20 appended with status `pending`; T09, T10 and T11
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: D` → `E`.
- 2026-11-12 — batch D review checkpoint opened: `phase: implement` →
  `review`, review D `pending` → `in_progress`, base `84a3edc5`, head
  `65fa3190`. T11 implemented: `pending` → `in_progress` → `pending_review`,
  commits `ef0d1bea`, `a8ede30e`, `92b119ab`, `65fa3190`; working tree clean.
- 2026-11-12 — T11 selected: `pending` → `in_progress`.
- 2026-11-12 — T10 implemented: `pending` → `in_progress` → `pending_review`,
  commits `627407c3`, `9a8c920e`, `6c517a71`, `5f5d7388`, `df31d02a`; working
  tree clean.
- 2026-11-12 — T10 selected: `pending` → `in_progress`.
- 2026-11-12 — T09 implemented: `pending` → `in_progress` → `pending_review`,
  commits `42995b17`, `86d2a027`, `4692f4d9`, `54b726d6`, `5cdecf09`; working
  tree clean.
- 2026-11-12 — batch D started: T09 `pending` → `in_progress`.
- 2026-11-12 — review C `done`: T06 kept, T07 kept, T08 kept, no fix or revert
  commits; follow-ups F9–F14 appended with status `pending`; T06, T07 and T08
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: C` → `D`.
- 2026-11-12 — batch C review checkpoint opened: `phase: implement` →
  `review`, review C `pending` → `in_progress`, base `52af2907`, head
  `7b31a903`. T08 implemented: `pending` → `in_progress` → `pending_review`,
  commits `a545f9cd`, `7b31a903`; working tree clean.
- 2026-11-12 — T08 selected: `pending` → `in_progress`.
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
