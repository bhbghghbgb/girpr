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
phase: followups-implement
active_batch: B5
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
    status: completed
    commits: [2df937bf, fb913a2a, a2df3d11]
    review: F
    note: review F kept; the missing --version judged acceptable since it keeps the previous exit-3 behaviour
  - id: T15
    title: Named sync phases decision
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T15-named-sync-phases.md
    batch: F
    report: [L458-L476]
    status: completed
    commits: [af640af6]
    review: F
    note: review F kept; a diff of cmd_sync with comments filtered out is identical, so no call moved and nothing was extracted
  - id: T16
    title: Reconcile documentation with the code
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T16-docs-reconcile.md
    batch: F
    report: [L513-L531]
    status: completed
    commits: [efb71184, 5b9a405a, 49830066, de233c05, 371a7413, 930efdf3]
    review: F
    note: review F fixed (930efdf3 corrects a README paragraph that contradicted the mixed-case collision refusal); the reviewer's fix message was amended once for text only
  - id: T17
    title: Converter atomicity and failure tests
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T17-converter-atomicity.md
    batch: G
    report: [L166-L189, L587-L589]
    status: completed
    commits: [71fe4368, 4cc52111, efdacd32]
    review: G
    note: review G kept; the --force preservation test was verified to be the only failure on the pre-T17 code
  - id: T18
    title: Feature-gate sled
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T18-feature-gate-sled.md
    batch: G
    report: [L597-L610]
    status: completed
    commits: [cce4fed3, efe471f8, 2a244f26]
    review: G
    note: review G kept; the default build carries no sled and the converter tests run only under the convert feature
  - id: T19
    title: Benchmark harness over three tree shapes
    plan: tools/sync/OPTIMIZATIONS_PLAN/tasks/T19-benchmark-harness.md
    batch: G
    report: [L590-L593]
    status: completed
    commits: [eac1d900, 96e48eb9, 9c0b1c49]
    review: G
    note: review G kept; harness = false, no criterion, no assert in the bench, and the suite does not build or run it
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
    status: done
    tasks: [T14, T15, T16]
    base: 7b05768e
    head: 930efdf3
    result: T14 kept (2df937bf, fb913a2a, a2df3d11); T15 kept (af640af6); T16 fixed (930efdf3 corrects a README paragraph that contradicted the mixed-case collision refusal); 520 passed, 1 skipped; every remaining README and DESIGN-NOTES claim checked against the tree
    followups: [F25, F26, F27, F28, F29]
  - batch: G
    status: done
    tasks: [T17, T18, T19]
    base: 2c23bd11
    head: 9c0b1c49
    result: T17 kept (71fe4368, 4cc52111, efdacd32); T18 kept (cce4fed3, efe471f8, 2a244f26); T19 kept (eac1d900, 96e48eb9, 9c0b1c49); no fix or revert commits; 512 passed without the feature and 524 with it, 1 skipped; the converter atomicity test was verified to be the only failure on the pre-T17 code
    followups: [F30, F31, F32]
  - batch: B1
    status: done
    tasks: [F1, F2]
    base: 107d8751
    head: b7e332c2
    result: F1 fixed (b7e332c2 removes a rustdoc link from a public doc to the module-private resolver, the crate's only rustdoc warning); F2 kept (a74b92e2); 513 passed, 1 skipped; the matching rule verified byte-for-byte identical to the old one
    followups: [F33, F34, F35]
  - batch: B2
    status: done
    tasks: [F3, F4, F5]
    base: ac4cdf20
    head: 4aa601b6
    result: F3 kept (3c30627d); F4 kept (00f56668, 4aa601b6); F5 kept (79f64048); no fix or revert commits; 516 passed, 1 skipped; each new test was verified to fail with its fix reverted
    followups: [F36, F37, F38]
  - batch: B3
    status: done
    tasks: [F7, F11, F12, F13, F18]
    base: 1e9685dd
    head: d4548755
    result: F7 kept (f1b638e1); F11 kept (d4548755); F12 kept (cb4456f4); F13 kept (0b3afb32); F18 kept (84af494b); no fix or revert commits; 520 passed, 1 skipped; the repair was verified to commit the table it creates
    followups: [F39, F40]
  - batch: B4
    status: done
    tasks: [F9, F10]
    base: 1dbf85a5
    head: 6f88f283
    result: F9 kept (a29c139a); F10 fixed (6f88f283 makes the three new Filter primitives private and states the whole rule in the public doc; the memo work in a7b24113 kept); 522 passed, 1 skipped; the memo's answer verified equal to Filter::is_excluded over 400 random trees, 7 visit orders and 2 case modes, and the phase-A map byte-identical over 15 filter shapes
    followups: [F41]
  - batch: B5
    status: pending
    tasks: [F14, F24]
    base: null
    head: null
    result: null
    followups: []
  - batch: B6
    status: pending
    tasks: [F15, F23]
    base: null
    head: null
    result: null
    followups: []
  - batch: B7
    status: pending
    tasks: [F16, F30]
    base: null
    head: null
    result: null
    followups: []
  - batch: B8
    status: pending
    tasks: [F25, F26, F31]
    base: null
    head: null
    result: null
    followups: []
  - batch: B9
    status: pending
    tasks: [F19, F20, F21]
    base: null
    head: null
    result: null
    followups: []
  - batch: B10
    status: pending
    tasks: [F22, F27]
    base: null
    head: null
    result: null
    followups: []
  - batch: B11
    status: pending
    tasks: [F6, F28, F29, F32]
    base: null
    head: null
    result: null
    followups: []
  - batch: B12
    status: pending
    tasks: [F17]
    base: null
    head: null
    result: null
    followups: []
  - batch: B13
    status: pending
    tasks: [F33, F34, F35]
    base: null
    head: null
    result: null
    followups: []
  - batch: B14
    status: pending
    tasks: [F36, F37, F38]
    base: null
    head: null
    result: null
    followups: []
  - batch: B15
    status: pending
    tasks: [F39, F40]
    base: null
    head: null
    result: null
    followups: []
  - batch: B16
    status: pending
    tasks: [F41]
    base: null
    head: null
    result: null
    followups: []
followups:
  - id: F1
    source: T01
    description: foreign_known resolves names through HashAlg::from_str, whose error path allocates an UnknownHash string for every non-matching foreign name; an ASCII-case comparison against the canonical spelling would be allocation-free.
    status: completed
    batch: B1
    commits: [b55cc628, b7e332c2]
    note: review B1 fixed (b7e332c2 removes a rustdoc link from a public doc to the module-private resolver); the matching rule is byte-for-byte the old one
  - id: F2
    source: T01
    description: DESIGN-NOTES "A digest is looked up by name, not by slot" still says "two lookups" and does not mention get_algo or the ASCII-case widening; T16 may already cover this reconciliation.
    status: completed
    batch: B1
    commits: [a74b92e2]
    note: review B1 kept; the paragraph already read "three lookups" and named get_algo at the base, so only the parsing sentence was stale
  - id: F3
    source: T02
    description: spare_link_storage no longer canonicalizes the dst root when there is no kept link, so an unresolvable dst root surfaces later instead of failing at plan time with "resolve dst root".
    status: completed
    batch: B2
    commits: []
  - id: F4
    source: T02
    description: The copy-free early return drops a possible plan-time IO error from stat'ing a planned delete/fix-dir target that a copy-free run used to surface.
    status: completed
    batch: B2
    commits: []
  - id: F5
    source: T02
    description: a_prefix_is_asked_about_once_per_call pins the PrefixStats memo directly rather than observing the guard's probe count; an end-to-end proof would need an injectable stat hook.
    status: completed
    batch: B2
    commits: []
  - id: F6
    source: T03
    description: DESIGN-NOTES.md still says "Seven of them" about the load-bearing invariants, but there are now eleven such sections; the count was already stale before this task.
    status: pending
    batch: B11
    commits: []
  - id: F7
    source: T05
    description: A cache holding a meta row but no ENTRIES table still does not get one created, because the write path creates it inside a transaction it then drops uncommitted; the new entries_present guard only steers such a cache to the write path rather than repairing it.
    status: completed
    batch: B3
    commits: []
  - id: F8
    source: T05
    description: Plan bookkeeping only, not a code change: the T05 task file's design_notes pointer resolves to README.md, not to DESIGN-NOTES.md, which never mentions the cache's meta row.
    status: skipped
    batch: null
    commits: []
    note: skipped: the only file it names is a task file in this plan directory, which is deleted at the end of the plan
  - id: F9
    source: T08
    description: The comment above the ancestor walk says the directories above an entry are "part of the path the filter kept", which is exact for an included file but not for an included directory, whose own ancestors are pulled in without being kept; a sharper sentence would protect the map identity the fix-up exists to preserve.
    status: completed
    batch: B4
    commits: []
  - id: F10
    source: T08
    description: is_excluded still rebuilds MatchOptions per entry and re-tests every exclude against every ancestor prefix; the adjacent phase-A micro-opportunity is still unaddressed.
    status: completed
    batch: B4
    commits: []
  - id: F11
    source: T07
    description: The no-op test observes only the re-key pass's read-transaction count, not the run's total cache scans, so the broader claim that a no-op run does not scan the cache more than once is still unpinned.
    status: completed
    batch: B3
    commits: []
  - id: F12
    source: T07
    description: CacheDb::read_transaction_count counts every read transaction on the handle rather than only entries-table reads, so a future read on another table would silently move the number the new tests pin.
    status: completed
    batch: B3
    commits: []
  - id: F13
    source: T07
    description: get_many allocates a String per key; a borrow-shaped variant would avoid the copies for a caller that only needs the rows.
    status: completed
    batch: B3
    commits: []
  - id: F14
    source: T06
    description: copy_one now carries both an AlgoSet and the request slice it is derived from, and nothing prevents the two from drifting apart.
    status: in_progress
    batch: B5
    commits: []
  - id: F15
    source: T09
    description: util::unique_sibling now has no callers left in the crate and should be deleted or given one.
    status: pending
    batch: B6
    commits: []
  - id: F16
    source: T09
    description: --backup-retain prunes only after a successful copy, so a folder whose cache is absent, or a --no-backup run, never prunes the copies already sitting beside it.
    status: pending
    batch: B7
    commits: []
  - id: F17
    source: T09
    description: Retention can unlink a sibling a concurrent run is still copying into, so that run's backup can vanish while the run reports success.
    status: pending
    batch: B12
    commits: []
  - id: F18
    source: T10
    description: The test the_default_is_the_size_without_a_variable no longer matches what it asserts after the fifth commit, which now pins the constant only.
    status: completed
    batch: B3
    commits: []
  - id: F19
    source: T11
    description: The walk_live files/dirs counters are unpinned by any test; the crate's --log-file JSON channel, already used by tests/output.rs, could assert the scan done event.
    status: pending
    batch: B9
    commits: []
  - id: F20
    source: T11
    description: The lowest-index copy-failure case discriminates the two orders only because a 1 MiB copy is slower than the injected failure, so it can pass vacuously on a very fast host.
    status: pending
    batch: B9
    commits: []
  - id: F21
    source: T12
    description: tests/sync.rs:1794 still writes o.common.case_sensitive = false directly instead of set_case_sensitive(false), which leaves the bundled filter's cached options on the old mode; harmless only because that run's filter has no patterns.
    status: pending
    batch: B9
    commits: []
  - id: F22
    source: T12
    description: compare labels a record side "record <path>" while compare-self labels it "record <path> (for <folder>)", so the same side is named two ways.
    status: pending
    batch: B10
    commits: []
  - id: F23
    source: T12
    description: Filter is reachable only as girsync::filter::Filter while CommonOpts is re-exported at the crate root, so a hand-built CommonOpts literal must name a second module path for its new field's type.
    status: pending
    batch: B6
    commits: []
  - id: F24
    source: T13
    description: copy_one still carries both algos: AlgoSet and the requested: &[HashAlg] slice derived from it, and nothing prevents the two from drifting.
    status: in_progress
    batch: B5
    commits: []
  - id: F25
    source: T14
    description: --output=json and flags placed before the positionals are now accepted where the hand-rolled parser rejected them; a harmless widening, but nothing in the tree records it.
    status: pending
    batch: B8
    commits: []
  - id: F26
    source: T14
    description: If the converter ever needs --version, it needs #[command(version)]; today --version is an unknown flag and exits 3.
    status: pending
    batch: B8
    commits: []
  - id: F27
    source: T15
    description: The cmd_sync module doc and the call-site comment still say the planner pairs "by lowercase" while CasePairing::Lowercase folds ASCII case only; the README now says "ASCII-case fold".
    status: pending
    batch: B10
    commits: []
  - id: F28
    source: T16
    description: The README source-layout table still omits src/convert.rs, src/hardlink.rs and src/link.rs.
    status: pending
    batch: B11
    commits: []
  - id: F29
    source: T16
    description: The design notes claim every known limit is stated in README.md, but "neither fold is the filesystem's own case table" is only in the design notes.
    status: pending
    batch: B11
    commits: []
  - id: F30
    source: T17
    description: The conversion temporary is a sibling of the cache inside the destination root and carries no reserved prefix, so a hard-killed conversion leaves a file a later sync would treat as ordinary content rather than tool state.
    status: pending
    batch: B7
    commits: []
  - id: F31
    source: T18
    description: The MSRV job runs cargo check -p girsync --all-targets --locked without features, so sled and the converter are no longer MSRV-checked now that the dependency is optional; they do still build on 1.89 today. The script is tests/scripts/check-msrv.ps1.
    status: pending
    batch: B8
    commits: []
  - id: F32
    source: T18
    description: The DESIGN-NOTES pointer that cites tests/convert.rs for the blake3 cases does not say the file now only runs under the convert feature.
    status: pending
    batch: B11
    commits: []
  - id: F33
    source: F1
    description: The allocation-free property has no test to pin it, because the crate has no allocator-counting harness and the guidelines bar timing assertions; the property currently rests on reading from_name.
    status: pending
    batch: B13
    commits: []
  - id: F34
    source: F1
    description: DigestSet::insert_named replaces a foreign entry only on an exact spelling match, so one row can hold the same algorithm twice under two spellings and the earlier one answers; pre-existing and documented as "first match wins", so it needs a test pinning it rather than a behaviour change.
    status: pending
    batch: B13
    commits: []
  - id: F35
    source: F2
    description: The "first match wins" rule is stated in the design notes but pinned by no test: no case holds two foreign entries that both resolve to one algorithm.
    status: pending
    batch: B13
    commits: []
  - id: F36
    source: F3
    description: The Probes doc says a failing probe is "a refusal at plan time, before the run has changed anything", which is true for the mkdir preflight but false for the three step-7 guards, since the rename is the run's first filesystem change; the wording also contradicts its own first paragraph.
    status: pending
    batch: B14
    commits: []
  - id: F37
    source: F3
    description: The Probes doc names only the link-chain walk as outside the seam, while ensure_link_safe's link_ancestor_above and plan/link.rs's read_link/try_is_link/read_dir also ask the filesystem directly.
    status: pending
    batch: B14
    commits: []
  - id: F38
    source: F3
    description: DESIGN-NOTES "A kept dst file link's target is its content, not an extra" lists the tests that enforce it, and the new an_unresolvable_dst_root_is_refused_at_plan_time is not among them.
    status: pending
    batch: B14
    commits: []
  - id: F39
    source: F11
    description: CacheDb::entries_read_count's doc still says "this handle has served", while the shared test backend makes the figure a per-cache count; the wording should name the cache, not the handle.
    status: pending
    batch: B15
    commits: []
  - id: F40
    source: F12
    description: begin_entries_read's doc says "Every read of the entries table goes through here", but the batch write handle's reads of the same table do not, so the count covers the read handle's reads only.
    status: pending
    batch: B15
    commits: []
  - id: F41
    source: F10
    description: The other half of the phase-A micro-note is still open: the alternate-key index still folds every cached key eagerly in insensitive mode, one extra String per cached key, even when no exact-case miss ever occurs.
    status: pending
    batch: B16
    commits: []
  - id: F42
    source: F10
    description: ExcludeMemo::is_excluded takes &'a str tied to the filter's lifetime, so a caller whose paths are not borrowed for that lifetime has to work around the public type.
    status: skipped
    batch: null
    commits: []
    note: skipped: the public type has no caller outside the crate, so no user is behind the ergonomic limit
  - id: F43
    source: F10
    description: The memo keeps one decision per visited directory, a hash insert per directory, so a run over a directory-heavy tree pays that on top of the directory index phase A already builds.
    status: skipped
    batch: null
    commits: []
    note: skipped: this is the accepted trade-off of the memo, already measured as 1.4-3.4x faster than the plain rule with exclude patterns
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
| F | T14 | completed | 2df937bf, fb913a2a, a2df3d11 | review F kept |
| F | T15 | completed | af640af6 | review F kept |
| F | T16 | completed | efb71184, 5b9a405a, 49830066, de233c05, 371a7413, 930efdf3 | review F fixed (930efdf3) |
| G | T17 | completed | 71fe4368, 4cc52111, efdacd32 | review G kept |
| G | T18 | completed | cce4fed3, efe471f8, 2a244f26 | review G kept |
| G | T19 | completed | eac1d900, 96e48eb9, 9c0b1c49 | review G kept |

## Transition log

Newest first. One line per committed state change: date, task/review, from → to,
commit.

- 2026-11-13 — follow-up batch B5 started: F14 and F24 `pending` →
  `in_progress`; `active_batch: B4` → `B5`.
- 2026-11-13 — review B4 `done`: F9 kept, F10 fixed (6f88f283); follow-up F41
  appended with batch `B16`; F42 and F43 recorded as `skipped` with reasons
  (no caller behind the public type's lifetime limit; the memo's per-directory
  insert is the accepted, measured trade-off); F9 and F10 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B4 review checkpoint opened: F9 and F10
  `in_progress` → `pending_review`, commits `a7b24113`, `a29c139a`; review B4
  `pending` → `in_progress`, base `1dbf85a5`, head `a29c139a`.
- 2026-11-12 — follow-up batch B4 started: F9 and F10 `pending` →
  `in_progress`; `active_batch: B3` → `B4`.
- 2026-11-12 — review B3 `done`: F7, F11, F12, F13 and F18 kept, no fix or
  revert commits; follow-ups F39–F40 appended with status `pending` and batch
  `B15` (two doc wordings in the cache db); F7, F11, F12, F13 and F18
  `pending_review` → `completed`.
- 2026-11-12 — follow-up batch B3 review checkpoint opened: F7, F11, F12, F13
  and F18 `in_progress` → `pending_review`, commits `84af494b`, `cb4456f4`,
  `0b3afb32`, `f1b638e1`, `d4548755`; review B3 `pending` → `in_progress`,
  base `1e9685dd`, head `d4548755`.
- 2026-11-12 — follow-up batch B3 started: F7, F11, F12, F13 and F18 `pending`
  → `in_progress`; `active_batch: B2` → `B3`.
- 2026-11-12 — review B2 `done`: F3 kept, F4 kept, F5 kept, no fix or revert
  commits; follow-ups F36–F38 appended with status `pending` and batch `B14`
  (the Probes doc wording and the design-notes test list, all documentation);
  F3, F4 and F5 `pending_review` → `completed`.
- 2026-11-12 — follow-up batch B2 review checkpoint opened: F3, F4 and F5
  `in_progress` → `pending_review`, commits `3c30627d`, `00f56668`,
  `79f64048`, `4aa601b6`; review B2 `pending` → `in_progress`, base
  `ac4cdf20`, head `4aa601b6`.
- 2026-11-12 — follow-up batch B2 started: F3, F4 and F5 `pending` →
  `in_progress`; `active_batch: B1` → `B2`.
- 2026-11-12 — review B1 `done`: F1 fixed (b7e332c2), F2 kept (a74b92e2);
  follow-ups F33–F35 appended with status `pending` and batch `B13`; F1 and F2
  `pending_review` → `completed`. B13 is a single test-only batch that pins
  the digest name matching rules (the allocation-free resolver, the exact
  replacement rule, and first-match-wins); it is scheduled last and closes
  the digest area so the follow-up chain terminates there.
- 2026-11-12 — follow-up batch B1 review checkpoint opened: F1 and F2
  `in_progress` → `pending_review`, commits `b55cc628`, `a74b92e2`; review B1
  `pending` → `in_progress`, base `107d8751`, head `a74b92e2`.
- 2026-11-12 — follow-up batch B1 started: F1 and F2 `pending` →
  `in_progress`.
- 2026-11-12 — follow-up grouping recorded: 31 pending follow-ups split into
  batches B1–B12, and a `reviews[]` row added for each. Execution order is
  B1, B2, B3, B4, B5, B6, B7, B12, B8, B9, B10, B11: B12 (F17) runs directly
  after B7 because the evaluator judged that F17's concurrency rule for
  retention must land on the retention policy that F16 settles, and doing it
  earlier would design the protection twice. F5 may be promoted to its own
  batch if the injectable stat hook needs a wider seam than the plan tests.
  `phase: followups-evaluate` → `followups-implement`, `active_batch: B1`.
- 2026-11-12 — F8 `pending` → `skipped`: it names only a task file in this plan
  directory, which is deleted when the plan ends, so there is no code work to
  do. 31 follow-ups remain `pending`.
- 2026-11-12 — review G `done`: T17 kept, T18 kept, T19 kept, no fix or revert
  commits; follow-ups F30–F32 appended with status `pending`; T17, T18 and T19
  `pending_review` → `completed`. All 19 tasks are now `completed`.
  `phase: review` → `followups-evaluate`.
- 2026-11-12 — batch G review checkpoint opened: `phase: implement` →
  `review`, review G `pending` → `in_progress`, base `2c23bd11`, head
  `9c0b1c49`. T19 implemented: `pending` → `in_progress` → `pending_review`,
  commits `eac1d900`, `96e48eb9`, `9c0b1c49`; working tree clean.
- 2026-11-12 — T19 selected: `pending` → `in_progress`.
- 2026-11-12 — T18 implemented: `pending` → `in_progress` → `pending_review`,
  commits `cce4fed3`, `efe471f8`, `2a244f26`; working tree clean.
- 2026-11-12 — T18 selected: `pending` → `in_progress`.
- 2026-11-12 — T17 implemented: `pending` → `in_progress` → `pending_review`,
  commits `71fe4368`, `4cc52111`, `efdacd32`; working tree clean.
- 2026-11-12 — batch G started: T17 `pending` → `in_progress`.
- 2026-11-12 — review F `done`: T14 kept, T15 kept, T16 fixed (930efdf3);
  follow-ups F25–F29 appended with status `pending`; T14, T15 and T16
  `pending_review` → `completed`. `phase: review` → `implement`,
  `active_batch: F` → `G`.
- 2026-11-12 — batch F review checkpoint opened: `phase: implement` →
  `review`, review F `pending` → `in_progress`, base `7b05768e`, head
  `371a7413`. T16 implemented: `pending` → `in_progress` → `pending_review`,
  commits `efb71184`, `5b9a405a`, `49830066`, `de233c05`, `371a7413`; working
  tree clean.
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
