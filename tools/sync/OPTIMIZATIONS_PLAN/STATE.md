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
active_batch: M44
spawn: openchamber session.create (agent build, model opencode/minimax-m3.1-flash-free), working dir = repo root E:\hoccode\hocrust\girpr, one session at a time
last_updated: 2026-11-16T05:13:08Z
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
    status: done
    tasks: [F14, F24]
    base: c410f784
    head: 3f2d992e
    result: F14 kept and F24 kept (one commit 3f2d992e closes both); no fix or revert commits; 523 passed, 1 skipped; the --hash-any-of flag-order tie-break verified intact and the copy still digests every requested algorithm
    followups: [F44, F45, F46]
  - batch: B6
    status: done
    tasks: [F15, F23]
    base: e6ab7477
    head: b0342361
    result: F15 kept (e031b0e4); F23 kept (b0342361); no fix or revert commits; 523 passed without the feature and 535 with it, 1 skipped; the deletion confirmed to have no caller anywhere in the workspace
    followups: [F47, F48]
  - batch: B7
    status: done
    tasks: [F16, F30]
    base: 7dd8436d
    head: be9aed2e
    result: F16 kept with a fix (be9aed2e updates the --backup-retain help to say the bound covers every folder the run touches); F30 kept (f612e0d4, 4d1a8820, fc5956b6); 527 passed without the feature and 540 with it, 1 skipped; both behaviour changes confirmed empirically
    followups: [F49, F50, F51]
  - batch: B12
    status: done
    tasks: [F17, F49]
    base: 186d22a2
    head: 4f3b7e10
    result: F17 fixed (5b89a4fb stamps the published name with the time the copy became whole rather than when it started, so a slow copy is not ordered as old and taken away; 4f3b7e10 updates the notes and README); F49 kept; 533 passed, 1 skipped; the copy-ordering defect was verified by collapsing the two stamps back into one
    followups: [F52, F53, F54, F55, F56]
  - batch: B20
    status: done
    tasks: [F53]
    base: 80b419d6
    head: 4a42cf81
    result: F53 fixed (8505c72a separates the three answers of taking a record, removes the record when the lock cannot be taken, and publishes without one rather than failing the run; fc06393f adds the case the code got wrong; 420485dd and 4a42cf81 correct the record name in the docs and one false sentence); 541 passed without the feature and 554 with it, 1 skipped; the guarantee verified to leave no window where neither the record nor the cache lock protects a live run's copy
    followups: [F57, F58, F60, F61]
  - batch: B21
    status: done
    tasks: [F52, F54, F55, F57]
    base: a32632aa
    head: 776c3d1e
    result: F52 kept (9eebcc83); F54 kept (ca4d5ae4); F55 kept (86467207); F57 kept (85d1a2d1, 980e7a3d); the review also fixed two stale code doc comments (086c4d2f, 6a7e78a5); 549 passed, 1 skipped; the live-record sparing was confirmed on this host with a standalone lock probe
    followups: [F65, F66, F68]
  - batch: B22
    status: done
    tasks: [F56, F58, F60, F61]
    base: 0dba9367
    head: fb7dd522
    result: F56 kept (a39d5550, a1a0a602, fb7dd522); F58 kept (62f7f950); F60 kept (c9d86e57); F61 kept (3b9a60e3); no fix or revert commits; 553 passed, 1 skipped; three mutation experiments confirmed each new case pins what it claims
    followups: [F73, F74, F75]
  - batch: B23
    status: done
    tasks: [F65, F66, F68, F73, F74, F75]
    base: c010f012
    head: f07a728b
    result: F65 kept (046b53d3); F66 kept (d8ea86be); F68 kept (4d764527); F73 fixed (f07a728b brings three doc paragraphs in line with the run-scoped registry); F74 fixed (04dab01f makes the sort use sort_by_cached_key, which is what its message described); F75 kept (08959674); 559 passed, 1 skipped
    followups: [F79, F81, F82]
  - batch: B24
    status: done
    tasks: [F79, F81, F82]
    base: 6ea86b57
    head: 3341ec24
    result: F79 kept (fe0e4c95); F81 kept (ca6ad054, 7a04252e); F82 fixed (3341ec24 rewraps the paragraph the follow-up left broken, the claim itself was already true); 562 passed, 1 skipped; the identity comparison was checked to hold on both arms
    followups: [F87, F88, F89]
  - batch: X1
    status: done
    tasks: [F25, F26, F31, F51, F64]
    base: 7e3e1a02
    head: aca6f3ba
    result: F25 kept (d342a72f); F26 kept (1f0b8edf); F31 kept (aca6f3ba); F51 kept (0fdbf2c9); F64 kept (218be752); no fix or revert commits; 567 passed without the feature and 580 with it, 1 skipped; the MSRV script checked at 1.89 with the convert feature in a clean target dir
    followups: [F116, F117, F118, F120]
  - batch: X2
    status: done
    tasks: [F19, F20, F21, F44, F45, F46]
    base: 272effd3
    head: 4e90c67e
    result: F19 kept (927df439); F20 kept (0958cb81); F21 kept (6eb52866); F44 kept (eaf252c5); F45 kept (552e07d8, 4e90c67e); F46 kept (4c279930); no fix or revert commits; 569 passed, 1 skipped; the reviewer ran its own mutation checks, with the copy-phase mutation caught 25 of 25 runs
    followups: [F125, F126, F127, F129, F130]
  - batch: X3
    status: done
    tasks: [F22, F27, F47, F48]
    base: 90c57c19
    head: d774d7ce
    result: F22 kept (7279aee5); F27 kept (9ea19b36); F47 kept (cb9c08a3); F48 kept (d774d7ce); no fix or revert commits; 572 passed, 1 skipped; both new pins were mutation-checked, and the public_surface target fails to compile without the re-export
    followups: [F136]
  - batch: X7
    status: done
    tasks: [F87, F88, F89]
    base: 757e2bf0
    head: c50748d3
    result: F87 fixed (c50748d3 drops the newest promise from the --backup-retain help summary, states the rule in BackupPolicy and fixes two stale test docs); F88 kept (7772be18); F89 kept (df7e2c36); 565 passed, 1 skipped; the 36 test names cited in the notes' Enforced by list were checked to exist
    followups: [F94, F95, F96, F97]
  - batch: Y2
    status: done
    tasks: [F101, F102, F103]
    base: 13d0adcd
    head: 5a4bec0e
    result: F101 fixed (f0033492 corrects the outcome sentence, since the drop-and-retry path is the one a platform takes when its rename cannot replace a directory at all); F102 kept (33faafce); F103 fixed (5a4bec0e removes a README claim that nothing reclaims a leftover claim, which the code contradicts); 567 passed, 1 skipped; a comment-stripped compare confirmed no behaviour changed
    followups: [F108, F109, F110]
  - batch: Y1
    status: done
    tasks: [F90, F91, F94, F95, F96, F97]
    base: ef56b02c
    head: 0d287806
    result: F90 kept (4a9178e7); F91 kept (4a9178e7, 6bd5912c); F94 kept (4a9178e7); F95 kept (58a86fcd); F96 fixed (0531886c corrects two places in open.rs that still gave the too-strong reason); F97 fixed (0d287806 corrects the doc count); 567 passed, 1 skipped; the design notes backticked identifiers were swept against the sources with 0 missing
    followups: [F101, F102, F103]
  - batch: Z4
    status: done
    tasks: [F132, F133, F134, F136]
    base: e9914420
    head: 339c0957
    result: F132 kept (0fa1a663); F133 kept (c9a512e8); F134 kept (72d9e889); F136 kept (339c0957); no fix or revert commits; 572 passed, 1 skipped; the identity comparison verified byte-for-byte unchanged and the two case-fold rules still disagree
    followups: [F144, F145, F146]
  - batch: M23
    status: done
    tasks: [F209, F210, F211]
    base: 983e70bf
    head: 986fa9a0
    result: F209 kept (aaed2ebd, e03f21ab); F210 kept (aaed2ebd); F211 kept (986fa9a0); no fix or revert commits; 598 passed without the feature and 614 with it, 1 skipped; the reviewer confirmed the retired spelling survives only in the three places that deny it and in the one historical statement, which is still true, and saw both new cases fail when the rename and the guard were undone
    followups: [F304]
  - batch: M56
    status: pending
    tasks: [F304]
    base: null
    head: null
    result: null
    followups: []
  - batch: M24
    status: done
    tasks: [F212, F213, F214, F215]
    base: 00894206
    head: 13120f7a
    result: F212 kept (6c8cb53e); F213 kept (6c8cb53e); F214 kept (645829a9); F215 kept (13120f7a); no fix or revert commits; 598 passed, 1 skipped; the reviewer resolved all 48 table rows against the tree and confirmed the three-case rule covers exactly the six whole-path rows, that update rehashes a stat-only row through plan_one_side and merge_row, and that the five kinds cover the six limits in listing order
    followups: [F307, F308, F309]
  - batch: M57
    status: pending
    tasks: [F307, F308, F309]
    base: null
    head: null
    result: null
    followups: []
  - batch: M9
    status: done
    tasks: [F147, F148, F149, F150, F151, F152, F153, F154]
    base: c2295439
    head: 4a2acaea
    result: F147 kept (91d626b5); F148 kept (1f493787, 4a2acaea); F149 kept (dfaecf32); F150 kept (8bbba952); F151 kept (dfaecf32); F152 kept (dfaecf32); F153 kept (ccf14a43); F154 kept (1f493787, 4a2acaea); no fix or revert commits; 584 passed without the feature and 14 converter cases with it, 1 skipped; the reviewer verified each of the six known limits is stated in the README
    followups: [F212, F213, F214, F215]
  - batch: M1
    status: done
    tasks: [F6, F28, F29, F32, F124]
    base: d597f80d
    head: f841e1e8
    result: F6 kept (27e6f74f); F28 kept (f841e1e8); F29 kept (e7264fba); F32 kept (601d7efe); F124 kept (746f94cb); no fix or revert commits; 572 passed without the feature and 585 with it, 1 skipped; the count was verified against the tree at twelve, not the eleven the follow-up text said
    followups: [F152, F153, F154]
  - batch: M25
    status: done
    tasks: [F216, F217]
    base: 9a395087
    head: 90d32f86
    result: F216 kept (d42a90fd); F217 fixed (0339f805, 90d32f86 changes the field headline to what the field actually holds, since the shared sentence was false for a read-only handle on a file-backed cache); 598 passed, 1 skipped; the reviewer saw the one private-items warning before the fix and none after, and confirmed every changed line in the batch is a doc comment
    followups: [F310, F311, F312, F313]
  - batch: M58
    status: pending
    tasks: [F310, F311, F312]
    base: null
    head: null
    result: null
    followups: []
  - batch: M59
    status: pending
    tasks: [F313]
    base: null
    head: null
    result: null
    followups: []
  - batch: M26
    status: done
    tasks: [F218, F219, F220, F221]
    base: cb6bb33c
    head: 95cc2721
    result: F218 kept (f26a1837); F219 fixed (f26a1837, aa527d20 bounds the shared counter's exclusion too, 95cc2721 bounds the lock the bound takes where the disk cannot lock); F220 kept (f26a1837); F221 kept (82854f75); 598 passed, 1 skipped; the reviewer read redb 4.1.0 and confirmed the lock is skipped where the file cannot be locked, and saw no non-comment line changed in the range
    followups: [F315, F316, F317]
  - batch: M60
    status: pending
    tasks: [F315, F316, F317]
    base: null
    head: null
    result: null
    followups: []
  - batch: M11
    status: done
    tasks: [F156, F157, F158, F159]
    base: 8cd72166
    head: 2ce5c4cc
    result: F156 kept (6fc49e88); F157 kept (6603ecaf); F158 kept (dbc64313); F159 kept (880b1319, 2ce5c4cc); no fix or revert commits; 584 passed, 1 skipped; the private-items doc warnings went from 8 to 1, the one left being out of this batch scope
    followups: [F218, F219, F220, F221]
  - batch: M27
    status: done
    tasks: [F222, F223, F224]
    base: 36f93bdd
    head: 7e717ec5
    result: F222 kept (b8ba41ec); F223 kept (6643ccf7); F224 fixed (32ef16cf, 7e717ec5 closes three spellings of the disk modules that defeated the scan and makes a glob import fail loudly); 599 passed, 1 skipped; the reviewer checked DIRECT_QUESTIONS against the code by hand and found every direct call named and nothing else, and ran eight probe scripts
    followups: [F318, F319, F320, F321, F322, F323]
  - batch: M61
    status: pending
    tasks: [F318, F319]
    base: null
    head: null
    result: null
    followups: []
  - batch: M62
    status: pending
    tasks: [F320, F321, F322, F323]
    base: null
    head: null
    result: null
    followups: []
  - batch: M28
    status: done
    tasks: [F225, F227]
    base: 14aa3d3
    head: f368ae29
    result: F225 kept with no commit, the reviewer judged the field doc true for a file read-only handle, the read/write file handle and the shared backend; F227 kept (f368ae29), the reviewer checked every asks string against its call site and found the only non-comment line changed to be the read_dir entry's asks string, which is read only in a test message; 599 passed, 1 skipped
    followups: [F328]
  - batch: M64
    status: pending
    tasks: [F328]
    base: null
    head: null
    result: null
    followups: []
  - batch: M63
    status: pending
    tasks: [F324, F325]
    base: null
    head: null
    result: null
    followups: []
  - batch: M12
    status: done
    tasks: [F160, F161, F162, F163, F164, F165]
    base: 2f983689
    head: 80ba72d1
    result: F160 fixed (80ba72d1 removes a clause the premise could not support); F161 kept (312f8852); F162 kept (71434e08); F163 kept (7d01f95b); F164 kept (1ff3d1ac); F165 kept (ce0cab8a); 584 passed, 1 skipped
    followups: [F225, F227]
  - batch: M2
    status: done
    tasks: [F36, F37, F38, F39, F40]
    base: efdf4617
    head: 0d49c077
    result: F36 kept (fb7fdc23); F37 kept (773d1c53); F38 kept (f398aa2a); F39 kept (779de219); F40 kept (0d49c077); no fix or revert commits; 572 passed, 1 skipped; the reviewer enumerated every direct filesystem call reachable from the guards and found the seam list complete
    followups: [F160, F161, F162, F163, F164, F165]
  - batch: M29
    status: done
    tasks: [F228, F229, F230, F231]
    base: 058c2c24
    head: 2b305551
    result: F228 kept (8ed05795); F229 kept (75fb6c4b); F230 fixed (fbf844bb, 4d0c9a7e scopes the claim to the exact-match rule, since a known name is matched by the resolver's fold and not through the helper); F231 fixed (02b0a441, 2b305551 restates the rule, which the pinned case contradicts); 605 passed without the feature and 621 with it, 1 skipped; the reviewer checked the streaming prune's predicate and the read counts against the old code and confirmed the same rows go and the counts mean what they meant
    followups: [F329, F330, F331, F332, F333]
  - batch: M65
    status: pending
    tasks: [F329, F330]
    base: null
    head: null
    result: null
    followups: []
  - batch: M66
    status: pending
    tasks: [F331, F332]
    base: null
    head: null
    result: null
    followups: []
  - batch: M67
    status: pending
    tasks: [F333]
    base: null
    head: null
    result: null
    followups: []
  - batch: M30
    status: done
    tasks: [F232, F234]
    base: 9c8ee02d
    head: b0a92f16
    result: F232 kept (3a52da69); F234 kept (b0a92f16); no fix or revert commits; 608 passed without the feature and 624 with it, 1 skipped; the reviewer reproduced the old and new lookup side by side over 2,613,659 key-set and disk-spelling combinations with 0 mismatches, mutation-tested the new oracle case against ten wrong rules, and confirmed the clippy allow hides exactly one lint
    followups: [F338, F339, F340]
  - batch: M69
    status: pending
    tasks: [F338, F339]
    base: null
    head: null
    result: null
    followups: []
  - batch: M70
    status: pending
    tasks: [F340]
    base: null
    head: null
    result: null
    followups: []
  - batch: M13
    status: done
    tasks: [F166, F167, F168, F169, F170, F171]
    base: 1fe6901c
    head: 6236f9fa
    result: F166 fixed (82fd8a95 corrects a doc that still called the arm a slice); F167 kept (6955810c); F168 kept (a095f412, 34293501); F169 kept (515bfa49); F170 kept (95a2d302, 4db5ab6a); F171 fixed (6236f9fa corrects the index doc); 585 passed without the feature and 599 with it, 1 skipped
    followups: [F232, F234]
  - batch: M31
    status: done
    tasks: [F236]
    base: 5dd484e6
    head: 7864335e
    result: F236 kept (7864335e); no fix or revert commits; 609 passed without the feature and 625 with it, 1 skipped; the reviewer ran the pre-change merge and the new one side by side over 20,000 random row pairs with identical slots, names, digests and arm capacity, and saw the new case fail when the clone was restored
    followups: [F345, F346]
  - batch: M71
    status: pending
    tasks: [F334, F335, F337]
    base: null
    head: null
    result: null
    followups: []
  - batch: M72
    status: pending
    tasks: [F336, F341, F343, F344]
    base: null
    head: null
    result: null
    followups: []
  - batch: M73
    status: pending
    tasks: [F345, F346]
    base: null
    head: null
    result: null
    followups: []
  - batch: M32
    status: done
    tasks: [F240, F241, F242]
    base: 083c4739
    head: 8230e53e
    result: F240 kept (537ee293); F241 kept (1dd21971); F242 kept (8230e53e); no fix or revert commits; 611 passed without the feature and 627 with it, 1 skipped; the reviewer confirmed the const block derives every answer from the vocabulary, that the ASCII-case spelling is still asked of the whole vocabulary at run time, and that alt_lookups moves only on an exact miss in insensitive mode
    followups: [F347, F348, F349, F350, F351, F352]
  - batch: M74
    status: pending
    tasks: [F347, F348, F349]
    base: null
    head: null
    result: null
    followups: []
  - batch: M75
    status: pending
    tasks: [F350, F351]
    base: null
    head: null
    result: null
    followups: []
  - batch: M76
    status: pending
    tasks: [F352]
    base: null
    head: null
    result: null
    followups: []
  - batch: M14
    status: done
    tasks: [F172, F173, F174]
    base: 78244080
    head: 6dd4294d
    result: F172 kept (0f9e7d33, a747499a); F173 fixed (6dd4294d drops the claim that the merge rule has one place, which the new merge case invalidated); F174 kept (3a65ca7f, 41656cb5, 2d6014e6); 588 passed without the feature and 602 with it, 1 skipped; the const pin was verified to break a plain cargo build
    followups: [F240, F241, F242]
  - batch: M3
    status: done
    tasks: [F33, F34, F35, F41]
    base: a5bc410d
    head: 929908a7
    result: F33 fixed (2bd29e74 corrects the design notes, which still justified the allocation-free comparison by reading the body); F34 kept (0d7f7646); F35 kept (4fb0741f); F41 fixed (929908a7 corrects the README cost claim, which named the wrong condition); 577 passed without the feature and 590 with it, 1 skipped; the const pin was verified to fail when the const is dropped and when the resolver allocates
    followups: [F172, F173, F174]
  - batch: M33
    status: done
    tasks: [F243, F244]
    base: c888a545
    head: eefb96ca
    result: F243 kept (93ab7621); F244 fixed (ba8570a6, eefb96ca corrects the pin's comment, which claimed that any doc comment on the argument struct reaches --help, true only of a multi-paragraph one on clap 4.6.7); 611 passed without the feature and 628 with it, 1 skipped; the reviewer added a version to the converter and saw the new case fail, and confirmed the narrowed pin still fails for a changed converter help text, a renamed accepted value and a multi-paragraph doc comment
    followups: [F353, F354, F355, F356]
  - batch: M78
    status: pending
    tasks: [F353, F354, F356]
    base: null
    head: null
    result: null
    followups: []
  - batch: M79
    status: pending
    tasks: [F355]
    base: null
    head: null
    result: null
    followups: []
  - batch: M34
    status: done
    tasks: [F247, F248, F249]
    base: f8af1203
    head: 2d19bef1
    result: F247 kept (6572e643, 3edc3d5e); F248 fixed (3546a457, 2d19bef1 carries the shape the whole set cannot see into the README, which had repeated the reason without the exception); F249 kept (c11652be); 611 passed without the feature and 628 with it, 1 skipped; the reviewer enumerated the old and new configuration sets (n=4 gives 11 of 15, n=8 gives 37 of 255, n=12 gives 79 of 4095), saw the help pin fail for six mutations and pass under a patched clap, and saw the self-test fail for four broken rules
    followups: [F358, F359, F360, F361, F362, F363, F364]
  - batch: M80
    status: pending
    tasks: [F358, F359, F360]
    base: null
    head: null
    result: null
    followups: []
  - batch: M81
    status: pending
    tasks: [F361, F362]
    base: null
    head: null
    result: null
    followups: []
  - batch: M82
    status: pending
    tasks: [F363, F364]
    base: null
    head: null
    result: null
    followups: []
  - batch: M83
    status: pending
    tasks: [F367, F368, F369]
    base: null
    head: null
    result: null
    followups: []
  - batch: M15
    status: done
    tasks: [F175, F176, F177, F178, F183]
    base: 49e768cc
    head: e86a40aa
    result: F175 kept (b1610c1e); F176 fixed (e86a40aa states the rule in the script and the root README, since the old note said the opposite of what the new check does); F177 kept (493fce5b); F178 kept (b0081392); 588 passed without the feature and 603 with it, 1 skipped; the help pin was verified to fail when a doc comment is put back on Args
    followups: [F247, F248, F249]
  - batch: M35
    status: done
    tasks: [F251, F252]
    base: d3b4a295
    head: c298eedc
    result: F251 kept (274d90e5); F252 kept (c298eedc); no fix or revert commits; the script ran for real over all five members and the self-test passed; the reviewer confirmed the old rules returned an empty optional-feature list for a member with a feature named Default and no default, that the new rules return both, and that the named combination is one whose recorded exit code is zero
    followups: [F370, F372, F373]
  - batch: M83
    status: pending
    tasks: [F367, F368, F369, F370]
    base: null
    head: null
    result: null
    followups: []
  - batch: M84
    status: pending
    tasks: [F372, F373]
    base: null
    head: null
    result: null
    followups: []
  - batch: M85
    status: pending
    tasks: [F375, F376, F377, F378]
    base: null
    head: null
    result: null
    followups: []
  - batch: M36
    status: done
    tasks: [F254, F255, F256, F257]
    base: 8cbfed2b
    head: 0dc95b09
    result: F254 kept (ee82d44c); F255 kept (07f0ef2a); F256 kept (4693e6ff); F257 kept (bac52732, 0dc95b09); no fix or revert commits; 611 passed, 1 skipped, and the crate itself was untouched by the batch; the reviewer ran an exhaustive harness over every member shape of one to five optional features against every pass and fail assignment, 67,722 cases with 0 mismatches and 0 throws, so the refusal never fires and the failure lists are unchanged
    followups: [F379, F380]
  - batch: M86
    status: pending
    tasks: [F379, F380]
    base: null
    head: null
    result: null
    followups: []
  - batch: M87
    status: pending
    tasks: [F381, F382]
    base: null
    head: null
    result: null
    followups: []
  - batch: M88
    status: pending
    tasks: [F383, F384, F395, F408]
    base: null
    head: null
    result: null
    followups: []
  - batch: M89
    status: pending
    tasks: [F386, F387]
    base: null
    head: null
    result: null
    followups: []
  - batch: M16
    status: done
    tasks: [F179, F180, F181]
    base: 23a5974d
    head: 673b99b1
    result: F179 kept (c0348f54); F180 kept (c0348f54); F181 kept (673b99b1); no fix or revert commits; 588 passed, 1 skipped; the reviewer drove eight synthetic workspaces and saw the broken lone feature now fail where the old script reported it covered
    followups: [F254, F255, F256, F257]
  - batch: M4
    status: done
    tasks: [F111, F112, F116, F130]
    base: 25c832ec
    head: 335cda4b
    result: F111 kept (335cda4b); F112 kept (1cac4f4b); F116 kept (839bcf84); F130 kept (0e73b57f); no fix or revert commits; 577 passed without the feature and 591 with it, 1 skipped; the reviewer drove the MSRV script with a stubbed cargo across five feature scenarios and found the fallback cannot hide a real failure
    followups: [F179, F180, F181, F183]
  - batch: M37
    status: done
    tasks: [F258, F259, F260]
    base: a91a091e
    head: e81da0f2
    result: F258 kept (26f0ea58); F259 kept (53a51337); F260 kept (e81da0f2); no fix or revert commits; 613 passed without the feature and 630 with it, 1 skipped; the reviewer confirmed the label's text is unchanged for both kinds, that the compare harness's own label text did change but reaches only a coverage error that cannot fire there, and saw three mutations fail the new pins
    followups: [F385]
  - batch: M87
    status: pending
    tasks: [F381, F382, F385]
    base: null
    head: null
    result: null
    followups: []
  - batch: M38
    status: done
    tasks: [F261, F262]
    base: 1abfc980
    head: 5f86beca
    result: F261 kept (2ca98413); F262 kept (5f86beca); no fix or revert commits; 613 passed without the feature and 630 with it, 1 skipped; the reviewer ran both message cases with the real binary and read what each prints, confirmed a folder side never reaches a rendered message, and checked the dependency direction, noting that the effective-to-planner edge in phase_c predates this batch
    followups: [F389]
  - batch: M90
    status: pending
    tasks: [F389]
    base: null
    head: null
    result: null
    followups: []
  - batch: M91
    status: pending
    tasks: [F392, F393]
    base: null
    head: null
    result: null
    followups: []
  - batch: M92
    status: pending
    tasks: [F391]
    base: null
    head: null
    result: null
    followups: []
  - batch: M39
    status: done
    tasks: [F263, F264, F265]
    base: bb57d8fe
    head: c6663a65
    result: F263 kept (ec4566db); F264 kept (1c363bbb); F265 fixed (28a1a18b, c6663a65 replaces the claim that the other two ways out are still named, which is false for the default one-algorithm request); 613 passed without the feature and 630 with it, 1 skipped; the reviewer mutated the pass to sort by the exact src key and saw the new assert_ne fail on its own
    followups: [F394, F395]
  - batch: M93
    status: pending
    tasks: [F394]
    base: null
    head: null
    result: null
    followups: []
  - batch: M17
    status: done
    tasks: [F185, F186, F187, F188, F190, F191]
    base: 5c2e29fc
    head: 6c23cb4b
    result: F185 kept (f0cd71f7); F186 kept (a2bb2324); F187 kept (4c48b5cb); F188 kept (4d91eca7); F190 fixed (6c23cb4b corrects the two docs that still asserted the removed rule); F191 kept (4d91eca7); 594 passed without the feature and 609 with it, 1 skipped
    followups: [F263, F264, F265]
  - batch: M40
    status: done
    tasks: [F266]
    base: ee8a10a8
    head: a17b7270
    result: F266 kept (a4740687, 4719ce4f, a17b7270); no fix or revert commits; 616 passed without the feature and 633 with it, 1 skipped; the reviewer mutated the remedy back to the space-joined spelling, made the planner ignore the request order and gave the argument two values, and saw each new pin fail; the grep found one surviving two-value spelling, which deliberately names the refused form
    followups: [F400, F401, F402, F403, F404]
  - batch: M95
    status: pending
    tasks: [F400, F401]
    base: null
    head: null
    result: null
    followups: []
  - batch: M96
    status: pending
    tasks: [F402, F404]
    base: null
    head: null
    result: null
    followups: []
  - batch: M97
    status: pending
    tasks: [F403]
    base: null
    head: null
    result: null
    followups: []
  - batch: M94
    status: pending
    tasks: [F397, F398]
    base: null
    head: null
    result: null
    followups: []
  - batch: M41
    status: done
    tasks: [F267, F268]
    base: d53d952c
    head: 1f950a9a
    result: F267 kept (a6850a91); F268 kept (1f950a9a); no fix or revert commits; 616 passed without the feature and 633 with it, 1 skipped; the reviewer ran every synopsis line with the placeholders filled and saw all of them parse, reproduced the README walkthrough end to end, and confirmed the default request is one algorithm alone
    followups: [F408]
  - batch: M98
    status: pending
    tasks: [F406, F407]
    base: null
    head: null
    result: null
    followups: []
  - batch: M99
    status: pending
    tasks: [F405]
    base: null
    head: null
    result: null
    followups: []
  - batch: M42
    status: done
    tasks: [F269, F270, F271]
    base: 0ab89862
    head: 8966a091
    result: F269 kept (0ac9ccee); F270 fixed (8a81858d, 8966a091 changes the test comment's run of names to the family of names, since run means an execution of the tool in these documents); F271 kept (afe44b2e); 616 passed without the feature and 633 with it, 1 skipped; the reviewer mutated the candidate name so the third suffix became a padded spelling while the second was kept, and the case failed where the old two-name case would have passed
    followups: [F413, F414]
  - batch: M102
    status: pending
    tasks: [F413, F414]
    base: null
    head: null
    result: null
    followups: []
  - batch: M100
    status: pending
    tasks: [F409, F410]
    base: null
    head: null
    result: null
    followups: []
  - batch: M101
    status: pending
    tasks: [F411, F412]
    base: null
    head: null
    result: null
    followups: []
  - batch: M18
    status: done
    tasks: [F189, F192]
    base: 8308d0b6
    head: b7f6afda
    result: F189 kept (f4b00295); F192 fixed (b7f6afda completes the naming in the design notes, which named only one retry spelling and read as if two names were the most a killed conversion can leave behind); 594 passed without the feature and 610 with it, 1 skipped; the namespace claim was verified true
    followups: [F269, F270, F271]
  - batch: M5
    status: done
    tasks: [F117, F118, F120, F140, F145, F146]
    base: 51806c37
    head: b08451c7
    result: F117 kept (e30e6c6f); F118 kept (13304c2f); F120 kept (1409bbf6); F140 kept (4c285d30); F145 kept (0f3609a7); F146 kept (b08451c7); no fix or revert commits; 578 passed without the feature and 592 with it, 1 skipped; the remedy was checked end to end by running it, and the renamed fixture was mutation-checked
    followups: [F189, F190, F191, F192]
  - batch: M43
    status: done
    tasks: [F273, F274]
    base: db97d9f0
    head: df01aaf7
    result: F273 kept (c582a5b5); F274 kept (df01aaf7); no fix or revert commits; 616 passed for the crate and 2101 for the workspace, 13 skipped; the reviewer confirmed each of the three pointers lands on a file that exists and holds what the pointer says, and that all three failure points really leave something different behind
    followups: [F419, F420]
  - batch: M105
    status: pending
    tasks: [F419, F420]
    base: null
    head: null
    result: null
    followups: []
  - batch: M103
    status: pending
    tasks: [F415, F416]
    base: null
    head: null
    result: null
    followups: []
  - batch: M104
    status: pending
    tasks: [F417]
    base: null
    head: null
    result: null
    followups: []
  - batch: M44
    status: in_progress
    tasks: [F275]
    base: fb1fef37
    head: f6878407
    result: null
    followups: []
  - batch: M106
    status: pending
    tasks: [F421, F422]
    base: null
    head: null
    result: null
    followups: []
  - batch: M107
    status: pending
    tasks: [F423, F424]
    base: null
    head: null
    result: null
    followups: []
  - batch: M45
    status: pending
    tasks: [F276, F277]
    base: null
    head: null
    result: null
    followups: []
  - batch: M19
    status: done
    tasks: [F193, F194, F198]
    base: 7831eb0c
    head: 7e12b998
    result: F193 fixed (7e12b998, the step-1 rename hook row claimed a placeholder that only a file claim leaves); F194 fixed (0f243257 gives the rule a reason that holds everywhere, since the old reason was a Windows fact); F198 kept (a3bff56e); 595 passed, 1 skipped; the rule itself was kept, because it enforces the precondition the split relies on and preserves every valid spec
    followups: [F276, F277]
  - batch: M46
    status: pending
    tasks: [F278, F279]
    base: null
    head: null
    result: null
    followups: []
  - batch: M47
    status: pending
    tasks: [F280, F281, F282]
    base: null
    head: null
    result: null
    followups: []
  - batch: M48
    status: pending
    tasks: [F283, F284, F285]
    base: null
    head: null
    result: null
    followups: []
  - batch: M20
    status: done
    tasks: [F195, F196, F197, F199]
    base: fe3db912
    head: 858fa92e
    result: F195 kept (14f77fa3); F196 kept (30b37539); F197 kept (fa35b7d8); F199 kept (858fa92e); no fix or revert commits; 596 passed, 1 skipped; a workspace-wide search found nothing that branches on the renamed event strings
    followups: [F282, F283, F284, F285]
  - batch: M6
    status: done
    tasks: [F121, F122, F125, F126, F127, F129]
    base: e68043dd
    head: 5b6fdf3b
    result: F121 kept (50e81f06); F122 kept (2115d14e, 5a5cb7a2); F125 kept (c4f93a7f); F126 kept (6c16cf00); F127 kept (20b42517); F129 kept (a5b3b2c1, 9c327065); no fix or revert commits; 584 passed, 1 skipped, and the whole workspace at 2069 passed, 13 skipped; four mutation probes confirmed the new cases discriminate, and a mis-set gate was probed and failed that one copy loudly
    followups: [F198, F199]
  - batch: M49
    status: pending
    tasks: [F287, F288, F292]
    base: null
    head: null
    result: null
    followups: []
  - batch: M50
    status: pending
    tasks: [F289, F291]
    base: null
    head: null
    result: null
    followups: []
  - batch: M21
    status: done
    tasks: [F200, F201, F202, F203, F204]
    base: ead8e3bd
    head: 6b15098a
    result: F200 kept (7e40bfb6); F201 kept (b8b98771); F202 kept (c64275cf); F203 kept (cb20bfea); F204 kept (6b15098a); no fix or revert commits; 596 passed, 1 skipped; the reviewer confirmed all three fold sites keep their own rule and that nothing in the repo links to the retitled heading
    followups: [F291, F292]
  - batch: M51
    status: pending
    tasks: [F294, F296]
    base: null
    head: null
    result: null
    followups: []
  - batch: M52
    status: pending
    tasks: [F295, F297]
    base: null
    head: null
    result: null
    followups: []
  - batch: M53
    status: pending
    tasks: [F298, F301]
    base: null
    head: null
    result: null
    followups: []
  - batch: M54
    status: pending
    tasks: [F299, F300]
    base: null
    head: null
    result: null
    followups: []
  - batch: M55
    status: pending
    tasks: [F302, F303]
    base: null
    head: null
    result: null
    followups: []
  - batch: M22
    status: done
    tasks: [F206, F207, F208]
    base: 7c9a179d
    head: 0a942361
    result: F206 kept (e5171086); F207 kept (b683af1f); F208 kept (0a942361); no fix or revert commits; 596 passed, 1 skipped; the reviewer confirmed one implementation is left, the three questions stay apart, and the helper is not compiled on POSIX
    followups: [F298, F299, F300, F301, F302, F303]
  - batch: M7
    status: done
    tasks: [F138, F139, F141, F144]
    base: 0013d040
    head: 10ec5297
    result: F138 kept (8e5f5bd0); F139 fixed (10ec5297 names the path key fold in the design notes mkdir-candidate paragraph, which still used the retired wording); F141 kept (0d360416); F144 kept (5777bf8c); 584 passed, 1 skipped; the reviewer confirmed exactly three path fold sites remain and no fourth rule was created
    followups: [F206, F207, F208]
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
    commits: [3c30627d]
    note: review B2 kept (3c30627d); the root is resolved before the early return, restoring the original message and call site
  - id: F4
    source: T02
    description: The copy-free early return drops a possible plan-time IO error from stat'ing a planned delete/fix-dir target that a copy-free run used to surface.
    status: completed
    batch: B2
    commits: [00f56668, 4aa601b6]
    note: review B2 kept (00f56668, 4aa601b6); the removals are probed whether or not the plan copies, only the credit stays copy-only
  - id: F5
    source: T02
    description: a_prefix_is_asked_about_once_per_call pins the PrefixStats memo directly rather than observing the guard's probe count; an end-to-end proof would need an injectable stat hook.
    status: completed
    batch: B2
    commits: [79f64048]
    note: review B2 kept (79f64048); the guard now asks the disk 4 times for 4 distinct prefixes instead of 9
  - id: F6
    source: T03
    description: DESIGN-NOTES.md still says "Seven of them" about the load-bearing invariants, but there are now eleven such sections; the count was already stale before this task.
    status: completed
    batch: M1
    commits: [27e6f74f]
  - id: F7
    source: T05
    description: A cache holding a meta row but no ENTRIES table still does not get one created, because the write path creates it inside a transaction it then drops uncommitted; the new entries_present guard only steers such a cache to the write path rather than repairing it.
    status: completed
    batch: B3
    commits: [f1b638e1]
    note: review B3 kept (f1b638e1); the repair commits the table it created instead of dropping the transaction, and adds an empty table only
  - id: F8
    source: T05
    description: "Plan bookkeeping only, not a code change: the T05 task file's design_notes pointer resolves to README.md, not to DESIGN-NOTES.md, which never mentions the cache's meta row."
    status: skipped
    batch: null
    commits: []
    note: "skipped: the only file it names is a task file in this plan directory, which is deleted at the end of the plan"
  - id: F9
    source: T08
    description: The comment above the ancestor walk says the directories above an entry are "part of the path the filter kept", which is exact for an included file but not for an included directory, whose own ancestors are pulled in without being kept; a sharper sentence would protect the map identity the fix-up exists to preserve.
    status: completed
    batch: B4
    commits: [a29c139a]
    note: review B4 kept (a29c139a); the comment is accurate, and the pull-in is still recorded so it cannot depend on walk order
  - id: F10
    source: T08
    description: is_excluded still rebuilds MatchOptions per entry and re-tests every exclude against every ancestor prefix; the adjacent phase-A micro-opportunity is still unaddressed.
    status: completed
    batch: B4
    commits: [a7b24113, 6f88f283]
    note: review B4 fixed (6f88f283 makes the three new Filter primitives private and states the whole rule in the public doc); the memo work in a7b24113 kept
  - id: F11
    source: T07
    description: The no-op test observes only the re-key pass's read-transaction count, not the run's total cache scans, so the broader claim that a no-op run does not scan the cache more than once is still unpinned.
    status: completed
    batch: B3
    commits: [d4548755]
    note: review B3 kept (d4548755); the shared test backend hands every open of one path the same counter, so the case pins one entries-table read per side
  - id: F12
    source: T07
    description: CacheDb::read_transaction_count counts every read transaction on the handle rather than only entries-table reads, so a future read on another table would silently move the number the new tests pin.
    status: completed
    batch: B3
    commits: [cb4456f4]
    note: review B3 kept (cb4456f4); the only increment site is begin_entries_read and every entries-table read on the read handle goes through it
  - id: F13
    source: T07
    description: get_many allocates a String per key; a borrow-shaped variant would avoid the copies for a caller that only needs the rows.
    status: completed
    batch: B3
    commits: [0b3afb32]
    note: review B3 kept (0b3afb32); the borrowing read answers the same rows in the same order from the same snapshot in one transaction, and the public get_many is unchanged
  - id: F14
    source: T06
    description: copy_one now carries both an AlgoSet and the request slice it is derived from, and nothing prevents the two from drifting apart.
    status: completed
    batch: B5
    commits: [3f2d992e]
    note: review B5 kept (3f2d992e); the --hash-any-of flag-order tie-break is intact, the copy still digests every requested algorithm, and the set is still built once per phase
  - id: F15
    source: T09
    description: util::unique_sibling now has no callers left in the crate and should be deleted or given one.
    status: completed
    batch: B6
    commits: [e031b0e4]
    note: review B6 kept (e031b0e4); git grep found only the definition and no caller at that revision, and the three atomic claim helpers are still the ones in use
  - id: F16
    source: T09
    description: --backup-retain prunes only after a successful copy, so a folder whose cache is absent, or a --no-backup run, never prunes the copies already sitting beside it.
    status: completed
    batch: B7
    commits: [0a66e0cf, 58613c52, e8335f4d, 34015725, be9aed2e]
    note: review B7 kept with a fix; be9aed2e updates the --backup-retain help to say the bound covers every folder the run touches; the prune still keeps exactly the newest N per prefix per directory
  - id: F17
    source: T09
    description: Retention can unlink a sibling a concurrent run is still copying into, so that run's backup can vanish while the run reports success.
    status: completed
    batch: B12
    commits: [55d0b59a, dd816c83, 63ff19bf, 5b89a4fb, 4f3b7e10]
    note: review B12 fixed (5b89a4fb stamps the published name with the time the copy became whole rather than when it started, so a slow copy is not ordered as old and taken away; 4f3b7e10 updates the notes and README)
  - id: F18
    source: T10
    description: The test the_default_is_the_size_without_a_variable no longer matches what it asserts after the fifth commit, which now pins the constant only.
    status: completed
    batch: B3
    commits: [84af494b]
    note: review B3 kept (84af494b); the case now says what it asserts
  - id: F19
    source: T11
    description: The walk_live files/dirs counters are unpinned by any test; the crate's --log-file JSON channel, already used by tests/output.rs, could assert the scan done event.
    status: completed
    batch: X2
    commits: [927df439]
    note: review X2 kept (927df439); the case pins the walk own counts with the excluded file counted, and separately asserts the run own record says files=2, the effective map
  - id: F20
    source: T11
    description: The lowest-index copy-failure case discriminates the two orders only because a 1 MiB copy is slower than the injected failure, so it can pass vacuously on a very fast host.
    status: completed
    batch: X2
    commits: [0958cb81]
    note: review X2 kept (0958cb81); no timing assertion was added, the 1 MiB copy is replaced by a fixed count of syscalls, and the reviewer saw the case fail 25 of 25 with the phase mutated
  - id: F21
    source: T12
    description: tests/sync.rs:1794 still writes o.common.case_sensitive = false directly instead of set_case_sensitive(false), which leaves the bundled filter's cached options on the old mode; harmless only because that run's filter has no patterns.
    status: completed
    batch: X2
    commits: [6eb52866]
    note: review X2 kept (6eb52866); a real fix, since the helper built the filter with case_sensitive true alongside the field
  - id: F22
    source: T12
    description: compare labels a record side "record <path>" while compare-self labels it "record <path> (for <folder>)", so the same side is named two ways.
    status: completed
    batch: X3
    commits: [7279aee5]
    note: review X3 kept (7279aee5); both commands name a record side through commands::side_label, the folder side label text is byte-identical in both, and the new case was mutation-checked
  - id: F23
    source: T12
    description: Filter is reachable only as girsync::filter::Filter while CommonOpts is re-exported at the crate root, so a hand-built CommonOpts literal must name a second module path for its new field's type.
    status: completed
    batch: B6
    commits: [b0342361]
    note: review B6 kept (b0342361); it is a re-export only, src/filter.rs is untouched and the old module path still resolves
  - id: F24
    source: T13
    description: "copy_one still carries both algos: AlgoSet and the requested: &[HashAlg] slice derived from it, and nothing prevents the two from drifting."
    status: completed
    batch: B5
    commits: [3f2d992e]
    note: same change as F14 (3f2d992e)
  - id: F25
    source: T14
    description: --output=json and flags placed before the positionals are now accepted where the hand-rolled parser rejected them; a harmless widening, but nothing in the tree records it.
    status: completed
    batch: X1
    commits: [d342a72f]
    note: review X1 kept (d342a72f); verified against the built binary, a real conversion gave exit 0 and a byte-identical JSON record for the = and spaced spellings
  - id: F26
    source: T14
    description: If the converter ever needs --version, it needs #[command(version)]; today --version is an unknown flag and exits 3.
    status: completed
    batch: X1
    commits: [1f0b8edf]
    note: review X1 kept (1f0b8edf); --version and -V exit 3 and --help exits 0, and the claim that #[command(version)] would exit 0 is right
  - id: F27
    source: T15
    description: The cmd_sync module doc and the call-site comment still say the planner pairs "by lowercase" while CasePairing::Lowercase folds ASCII case only; the README now says "ASCII-case fold".
    status: completed
    batch: X3
    commits: [9ea19b36]
    note: review X3 kept (9ea19b36); the wording keeps the ASCII-case fold distinct from the Unicode identity fallback, so the two case-fold rules stay two rules in the text
  - id: F28
    source: T16
    description: The README source-layout table still omits src/convert.rs, src/hardlink.rs and src/link.rs.
    status: completed
    batch: M1
    commits: [f841e1e8]
  - id: F29
    source: T16
    description: The design notes claim every known limit is stated in README.md, but "neither fold is the filesystem's own case table" is only in the design notes.
    status: completed
    batch: M1
    commits: [e7264fba]
  - id: F30
    source: T17
    description: The conversion temporary is a sibling of the cache inside the destination root and carries no reserved prefix, so a hard-killed conversion leaves a file a later sync would treat as ordinary content rather than tool state.
    status: completed
    batch: B7
    commits: [f612e0d4, 4d1a8820, fc5956b6]
    note: review B7 kept; the reserved prefix matches is_cache_rel, so a leftover is in neither side map, and it collides with neither the live cache nor the two bounded lists
  - id: F31
    source: T18
    description: The MSRV job runs cargo check -p girsync --all-targets --locked without features, so sled and the converter are no longer MSRV-checked now that the dependency is optional; they do still build on 1.89 today. The script is tests/scripts/check-msrv.ps1.
    status: completed
    batch: X1
    commits: [aca6f3ba]
    note: review X1 kept (aca6f3ba); the script really runs the check with --features convert at 1.89 in a clean target dir, and the feature list cannot silently skip a feature, proved with a synthetic package
  - id: F32
    source: T18
    description: The DESIGN-NOTES pointer that cites tests/convert.rs for the blake3 cases does not say the file now only runs under the convert feature.
    status: completed
    batch: M1
    commits: [601d7efe]
  - id: F33
    source: F1
    description: The allocation-free property has no test to pin it, because the crate has no allocator-counting harness and the guidelines bar timing assertions; the property currently rests on reading from_name.
    status: completed
    batch: M3
    commits: [643ebcb2, 822254ab, 2bd29e74]
  - id: F34
    source: F1
    description: DigestSet::insert_named replaces a foreign entry only on an exact spelling match, so one row can hold the same algorithm twice under two spellings and the earlier one answers; pre-existing and documented as "first match wins", so it needs a test pinning it rather than a behaviour change.
    status: completed
    batch: M3
    commits: [0d7f7646]
  - id: F35
    source: F2
    description: "The \"first match wins\" rule is stated in the design notes but pinned by no test: no case holds two foreign entries that both resolve to one algorithm."
    status: completed
    batch: M3
    commits: [4fb0741f, 822254ab]
  - id: F36
    source: F3
    description: The Probes doc says a failing probe is "a refusal at plan time, before the run has changed anything", which is true for the mkdir preflight but false for the three step-7 guards, since the rename is the run's first filesystem change; the wording also contradicts its own first paragraph.
    status: completed
    batch: M2
    commits: [fb7fdc23]
  - id: F37
    source: F3
    description: The Probes doc names only the link-chain walk as outside the seam, while ensure_link_safe's link_ancestor_above and plan/link.rs's read_link/try_is_link/read_dir also ask the filesystem directly.
    status: completed
    batch: M2
    commits: [773d1c53]
  - id: F38
    source: F3
    description: DESIGN-NOTES "A kept dst file link's target is its content, not an extra" lists the tests that enforce it, and the new an_unresolvable_dst_root_is_refused_at_plan_time is not among them.
    status: completed
    batch: M2
    commits: [f398aa2a]
  - id: F39
    source: F11
    description: CacheDb::entries_read_count's doc still says "this handle has served", while the shared test backend makes the figure a per-cache count; the wording should name the cache, not the handle.
    status: completed
    batch: M2
    commits: [779de219]
  - id: F40
    source: F12
    description: begin_entries_read's doc says "Every read of the entries table goes through here", but the batch write handle's reads of the same table do not, so the count covers the read handle's reads only.
    status: completed
    batch: M2
    commits: [0d49c077]
  - id: F41
    source: F10
    description: "The other half of the phase-A micro-note is still open: the alternate-key index still folds every cached key eagerly in insensitive mode, one extra String per cached key, even when no exact-case miss ever occurs."
    status: completed
    batch: M3
    commits: [dfa16d0b, 1f66fcc7, 929908a7]
  - id: F42
    source: F10
    description: ExcludeMemo::is_excluded takes &'a str tied to the filter's lifetime, so a caller whose paths are not borrowed for that lifetime has to work around the public type.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the public type has no caller outside the crate, so no user is behind the ergonomic limit"
  - id: F43
    source: F10
    description: The memo keeps one decision per visited directory, a hash insert per directory, so a run over a directory-heavy tree pays that on top of the directory index phase A already builds.
    status: skipped
    batch: null
    commits: []
    note: "skipped: this is the accepted trade-off of the memo, already measured as 1.4-3.4x faster than the plain rule with exclude patterns"
  - id: F44
    source: F14
    description: "The copy verify's new canonical-order naming is unpinned: a_disagreement_between_the_two_digests_is_a_failure only ever lets one algorithm disagree, so nothing would fail if the iteration order went back to the request's flag order."
    status: completed
    batch: X2
    commits: [eaf252c5]
    note: review X2 kept (eaf252c5); the new case lets both digests disagree and spells the request the other way round, and the reviewer saw it fail when the verify named the last disagreeing algorithm
  - id: F45
    source: F14
    description: README.md documents the fixed order of the digest-differs tag but not of the copy verify's failure message, so the algorithm a failed copy names is discoverable only from the design notes.
    status: completed
    batch: X2
    commits: [552e07d8, 4e90c67e]
    note: review X2 kept (552e07d8, 4e90c67e); the README matches the code bail text and the same canonical order as the digest-differs tag
  - id: F46
    source: F14
    description: tools/sync/DESIGN-NOTES.md still ends without a trailing newline; pre-existing, but the file was appended to by this batch.
    status: completed
    batch: X2
    commits: [4c279930]
    note: review X2 kept (4c279930); a one-line diff with no content change
  - id: F47
    source: F23
    description: glob::Pattern is still not reachable from the crate root, so a caller that wants to hold a compiled pattern or read Filter::includes()/excludes() still needs a direct glob dependency; a root pub use glob, the way girpr_log is already re-exported, would close that.
    status: completed
    batch: X3
    commits: [cb9c08a3]
    note: review X3 kept (cb9c08a3); the re-export shadows nothing and mirrors the existing girpr_log precedent, and the new public_surface test fails to compile without it
  - id: F48
    source: F23
    description: The new root comment calls compile_patterns "the one way to build its patterns", but glob::Pattern::new is a second way for a caller that already depends on glob, so the wording could be narrowed to "the crate's own way".
    status: completed
    batch: X3
    commits: [d774d7ce]
    note: review X3 kept (d774d7ce); the claim is narrowed correctly and the style matches the existing usage in the same file
  - id: F49
    source: F16
    description: The backup bound is now applied on every cache open rather than only after a copy, so a folder with an idle cache is pruned by any run that merely opens it, which widens the window of the already-tracked retention race.
    status: completed
    batch: B12
    commits: [55d0b59a, dd816c83, 63ff19bf]
    note: review B12 kept; moving the bound after the open puts it under redb exclusive lock, so at most one run bounds a folder at a time
  - id: F50
    source: F16
    description: The bound covers the other prefix in a folder too, so a folder that is both source and destination loses snapshots it never wrote; consistent with the folder-scoped rule, but worth stating in the flag's own help.
    status: completed
    batch: B7
    commits: [be9aed2e]
    note: addressed by the review's own fix commit be9aed2e, which states the folder-scoped bound in the flag help
  - id: F51
    source: F30
    description: The conversion temporary sits inside the reserved namespace, so it is never cleaned up by the retention bound and accumulates in a destination root; the old name had the same fate, so this is not a regression, but nothing removes a stale girpr-cache-convert-*.redb.
    status: completed
    batch: X1
    commits: [0fdbf2c9]
    note: review X1 kept (0fdbf2c9); the decision to document rather than reclaim is right and consistent with the crate no-age-and-no-lease rule, and retain_siblings filters by the two published prefixes only
  - id: F52
    source: F17
    description: A published name exists empty for the microseconds between the create_new claim and the rename, so it is briefly a published name with no bytes; closing the window would need a hard link or a no-replace rename rather than a portable rename.
    status: completed
    batch: B21
    commits: [9eebcc83, 3eeeb395]
  - id: F53
    source: F17
    description: A copy another run has already published is still the bound's to take, so two concurrent runs can still have one lose its published copy; telling a published copy from one still in use needs a marker of its own.
    status: completed
    batch: B20
    commits: [cd3942ee, 29ea8fab, 214cc1ae, 0ce5c0f3, 8505c72a, fc06393f, 420485dd, 4a42cf81]
  - id: F54
    source: F49
    description: The bound now runs after the open, so a cache that fails to open, whether corrupt or a legacy sled directory the open refuses, no longer gets its folder bounded, and those copies accumulate until --ignore-cache rebuilds the cache.
    status: completed
    batch: B21
    commits: [ca4d5ae4, 3eeeb395]
  - id: F55
    source: F17
    description: "The legacy sled directory copy is not claimed atomically: the rename itself decides, and POSIX rename onto an existing empty directory succeeds, so exclusivity there is the rename rather than an exclusive create."
    status: completed
    batch: B21
    commits: [86467207, 3eeeb395]
  - id: F56
    source: F49
    description: The bound's "never take the run's own copy" record is process-wide and consumed by the first bound for that folder, so a second bound for the same folder in the same process no longer spares it; harmless today, but an assumption a new caller could break.
    status: completed
    batch: B22
    commits: [a39d5550, a1a0a602, fb7dd522]
  - id: F57
    source: F53
    description: A record that cannot be read or locked spares its copy for good, so on a filesystem that cannot lock a file a record left by an older build defeats the bound for that copy forever.
    status: completed
    batch: B21
    commits: [85d1a2d1, 980e7a3d, 77952c07]
  - id: F58
    source: F53
    description: retain_copies is pub, but its safety rests entirely on the caller already holding the folder's cache exclusive lock, and nothing in the signature or doc says so.
    status: completed
    batch: B22
    commits: [62f7f950]
  - id: F59
    source: F53
    description: The published-copy registry forgets a copy once the bound for its folder has run, so a second bound on the same folder inside one process would no longer spare the copy that run is still using.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same defect as F56, which is already scheduled; recording it twice would make the same fix look like two items"
  - id: F60
    source: F53
    description: The record's body (pid=, started=) is written but never read, so nothing reports or checks which run holds a record.
    status: completed
    batch: B22
    commits: [c9d86e57]
  - id: F61
    source: F53
    description: The copy's protection after the record is released rests on redb's exclusive file lock, which is redb's property rather than something the suite pins.
    status: completed
    batch: B22
    commits: [3b9a60e3]
  - id: F64
    source: F21
    description: The girpr-cache-partial-* working name is never reclaimed by any bound, so a killed run leaves it in the folder; the same stale-tool-state story as the conversion temporary.
    status: completed
    batch: X1
    commits: [218be752]
    note: review X1 kept (218be752); the reasoning is recorded where a reader will find it, in the WORK_PREFIX doc and the README
  - id: F65
    source: F52
    description: The published directory name is still briefly an empty directory between the exclusive create and the rename, and the README's "A copy appears whole, or not at all" paragraph does not repeat it; on a filesystem that cannot lock a file a bound can also remove that empty claim and make the publish fail loudly.
    status: completed
    batch: B23
    commits: [046b53d3]
  - id: F66
    source: F52
    description: publish_file warns and carries on when the working file cannot be removed after the link, so a girpr-cache-partial-<ts> can be left behind by a run that was never killed, while the README describes that name only as the result of a killed run.
    status: completed
    batch: B23
    commits: [d8ea86be]
  - id: F67
    source: F54
    description: On the open-failure path the bound runs with no lock at all when the cache is a directory, so a live run's copies there rest on their records alone.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a directory has no file to lock, and the records are the protection that already exists there; the notes state the limit"
  - id: F68
    source: F54
    description: take_the_cache_lock opens the cache read+write, so a cache file that is not writable fails the open and the bound then runs without the lock, with a warning; the design notes spell out the directory case but not this one.
    status: completed
    batch: B23
    commits: [4d764527]
  - id: F69
    source: F57
    description: A record that cannot be read or whose lock cannot be tested is never reclaimed, so girpr-cache-hold-* files left behind by an older build accumulate for good on a filesystem that cannot lock a file, even though the copy they spoke for is taken away.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same kind of follow-up as F57, and the copy is reclaimed; only the record file lingers, and only on a filesystem that cannot lock a file"
  - id: F73
    source: F56
    description: The registry identifies the copy the run applying the bound published as the newest copy this process published in that list, so when the current run publishes nothing into that list, an earlier run's copy is still spared and the bound may keep an older copy in place of a newer one.
    status: completed
    batch: B23
    commits: [777fe7b7, f07a728b]
  - id: F74
    source: F56
    description: newest_published picks the newest by full file name, so the _10 fallback suffix sorts before _9; the bound's own age order already assumes that lexicographic name order, so the two agree, but ten copies published inside one microsecond would be mis-ordered.
    status: completed
    batch: B23
    commits: [cb1f80f6, c4ba99aa, 04dab01f]
  - id: F75
    source: F61
    description: The new lock test asserts only that the second open fails, not that it failed on the lock; asserting the already-open error would say outright that the protection comes from the cache's exclusive lock.
    status: completed
    batch: B23
    commits: [08959674]
    note: review B23 kept; the case now requires the already-open error, which a corrupt cache would not produce
  - id: F79
    source: F65
    description: The claim check tests that something is at the name, not that the claim is still ours, so if a bound took the claim away and another run took the same name in that window the publish still lands over that run's copy; reachable only where the filesystem can neither link nor lock a file.
    status: completed
    batch: B24
    commits: [fe0e4c95]
    note: review B24 kept (fe0e4c95); the identity is read from the handle the claim was created with, so the platform pair cannot be freed and reused while the publish is in flight, and both arms read it through the same platform pair
  - id: F80
    source: F68
    description: An exclusive lock on a read-only handle is fine on Windows and on Linux, but is worth a check on the BSDs and macOS, where a refusal would degrade the folder back to bounded without its lock rather than weaken it.
    status: skipped
    batch: null
    commits: []
    note: "skipped: not verifiable on this Windows host, and the outcome is a bounded degradation rather than a weakening"
  - id: F81
    source: F73
    description: A copy published by a run that never opens the folder's cache stays pending and is adopted, and spared, by the next run that opens that folder, so an older copy is kept in place of a newer one for one bound cycle.
    status: completed
    batch: B24
    commits: [ca6ad054, 7a04252e]
    note: review B24 kept (ca6ad054, 7a04252e); the softened claim is true, and the registry is process-wide with one run per process, so the command line never meets the case
  - id: F82
    source: F74
    description: "The bound's exactly-N holds only while no copy in the folder is spoken for by a live run: such copies are skipped without spending the removal budget, so the folder can end with more than N, and the design notes' claim that the bound still holds the folder to its bound overstates it for that case."
    status: completed
    batch: B24
    commits: [0aeaa4a7, 3341ec24]
    note: review B24 fixed (3341ec24 rewraps the paragraph the follow-up left broken); the claim itself was already true and needed no code change
  - id: F87
    source: F82
    description: The bound's walk starts at the oldest copy and skips the spoken-for ones without spending the removal budget, so it can remove a copy that is among the newest N; the docs say the bound walks on to the next-oldest copy but never say that a newer copy can go.
    status: completed
    batch: X7
    commits: [6da18401, 91533514, c50748d3]
  - id: F88
    source: F79
    description: publish_dir's retry path removes the claim directory before renaming again, so another run's empty published directory copy, a copy of an empty legacy cache, can be removed in the window between the identity check and that removal; the notes' claim that the other run's copy is never overwritten holds for a copy that has content.
    status: completed
    batch: X7
    commits: [7772be18]
  - id: F89
    source: F81
    description: A run whose cache open fails also adopts the waiting copies, because begin_run runs before the bound in that path, while the documents say the next run that opens it.
    status: completed
    batch: X7
    commits: [df7e2c36]
    note: review X7 kept (df7e2c36); a failed open copies the pending list into the spared list instead of taking it, so the copies keep waiting for the next run that opens the folder
  - id: F90
    source: F88
    description: "publish_file's no-link fallback has the same check-then-act shape: the identity check runs before the rename and is not repeated, so the residual window between the check and the rename is one syscall wide, reachable only where the filesystem can neither link nor lock."
    status: completed
    batch: Y1
    commits: [4a9178e7, 6bd5912c]
  - id: F91
    source: F88
    description: publish_file's fallback removes the claim and reports the error rather than retrying the rename, so on a platform whose rename cannot replace a file that publish can never succeed; practically unreachable, since Windows can link and POSIX replaces.
    status: completed
    batch: Y1
    commits: [4a9178e7, 6bd5912c]
  - id: F92
    source: F87
    description: The bound's "keep the newest N" reading could be a behaviour choice instead of a documented one, keeping the newest N plus the spoken-for copies at the cost of the folder exceeding N; a policy decision, not a defect.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a policy decision rather than a defect, and the current behaviour is now documented"
  - id: F93
    source: F88
    description: A published directory copy of an empty legacy cache is indistinguishable from a claim to a bound; a marker would change the copy content, so it is a design question.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a design question with a behaviour cost, not a defect"
  - id: F94
    source: F88
    description: "The retry path is still check-then-act: a claim taken away between the identity check and remove_dir remains removable, because a directory claim has no portable handle to remove by identity."
    status: completed
    batch: Y1
    commits: [4a9178e7]
  - id: F95
    source: F88
    description: "publish_file's no-link fallback has the same shape: it removes the claim file after a failed rename, guarded by the handle-based identity check but with the same gap before the removal."
    status: completed
    batch: Y1
    commits: [58a86fcd, 4a9178e7]
  - id: F96
    source: F89
    description: The documents say the failed run's bound spares the waiting copies because they are the copies it published, but the waiting list can also hold an earlier run's copy in the same process, so the stated reason is too strong.
    status: completed
    batch: Y1
    commits: [5ee6d57f, 0531886c]
  - id: F97
    source: F87
    description: The end-to-end test run_sync_backup_retain_keeps_the_newest_of_each_prefix still says it keeps the newest N siblings of each prefix, which is true for the case it exercises but carries the naive reading in its name and doc.
    status: completed
    batch: Y1
    commits: [8c13c181, 0d287806]
    note: review Y1 fixed (0d287806 corrects the doc last count, which said one copy where it is two); 8c13c181 renamed the test after what it checks
  - id: F98
    source: F95
    description: The file publish's first rename still replaces unconditionally on a filesystem that can link but whose link failed for another reason, such as EPERM on an overlay or FUSE mount, so the copy lands over whatever is at the name; a branch that refuses the fallback when the link failed for a reason other than the name being taken would close it.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same kind as F90 and F95, and reachable only on an exotic filesystem; the limit is now documented"
  - id: F99
    source: F95
    description: drop_the_claim could take the removal call as a parameter instead of the NameClaim.is_dir flag, which would let the two claim kinds share one path without a new field.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a code-shape preference with no behaviour cost"
  - id: F100
    source: F96
    description: The bound's spare logic could distinguish the failed run's own copies from an earlier run's in the same process, which the registry knows, instead of sparing the whole waiting list, so a failed run does not over-spare an earlier run's copy.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the over-sparing is conservative, it spares more than needed and cannot lose a backup"
  - id: F101
    source: F94
    description: The directory retry's window between dropping the claim and the second rename is not stated, and publish_dir's doc claims that a name taken in the meantime makes that rename fail, which holds only for a name with content in it, since a POSIX rename replaces an empty directory.
    status: completed
    batch: Y2
    commits: [d52fdd47, f0033492]
  - id: F102
    source: F91
    description: The file publish's no-retry rationale ends with "a name is only worth a second try on a platform whose rename cannot replace anything at all", naming as the only retry-safe platform the very one the same paragraph says the copy never lands on, so the sentence reads as an argument for a retry rather than against one.
    status: completed
    batch: Y2
    commits: [33faafce]
  - id: F103
    source: F95
    description: The claim drop is best-effort, since drop_the_claim ignores the removal error and still reports the claim as gone, while the notes and the code doc say unconditionally that the claim goes and the name is left free; a removal that fails leaves an empty file under a published prefix, which the bound reads as a published copy.
    status: completed
    batch: Y2
    commits: [39706eae, 3dee6745, 5a4bec0e]
    note: review Y2 fixed (5a4bec0e removes the README claim that nothing reclaims a leftover claim, which the code contradicts); 39706eae stated the best-effort behaviour correctly everywhere
  - id: F104
    source: F103
    description: A failed claim removal is silent, since drop_the_claim swallows the error entirely and the run does not warn about the leftover, unlike the working-copy leftover which warns.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; a warning is new log output, a behaviour change, and the leftover is already documented"
  - id: F105
    source: F103
    description: "The bound cannot tell a leftover claim from a whole copy: it counts it towards the bound and may keep it in place of a whole copy; the bound could prefer removing an empty name under a published prefix."
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; preferring to remove an empty name is a change to the count rule, which is now documented"
  - id: F106
    source: F101
    description: "The directory retry's window is closable in principle: re-claiming the name with an exclusive create after dropping it would put the copy back onto a claim this run owns, closing the drop-to-rename window rather than only documenting it."
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; re-claiming changes the publish protocol, and the window is reachable only where the filesystem cannot lock a file"
  - id: F107
    source: F101
    description: publish_onto_the_claimed_name swallows the rename error when the name is gone, so it reads as a publish failure rather than the name having been taken; the two are not distinguished in the error the caller sees.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; distinguishing the two failures is a diagnostic nicety and the publish fails loudly either way"
  - id: F108
    source: F101
    description: The notes headline that the bound can only ever select a whole copy does not name the leftover-claim exception that the paragraph under it now gives.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; a wording consistency item in the same documents"
  - id: F109
    source: F101
    description: The module doc and the function docs order the three claim gaps differently from the notes, so the same three gaps are met in two orders.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; an ordering choice between two documents, not a defect"
  - id: F110
    source: F103
    description: The README best-effort and leftover-claim paragraphs name only girpr-cache-backup-<ts>, where a leftover claim can also sit under girpr-cache-old-<ts>.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the backup publish chain ends at this batch; a wording completeness item in the same documents"
  - id: F111
    source: F31
    description: The MSRV script checks each optional feature alone, so a member whose feature only builds in combination with another would need the group enabled instead.
    status: completed
    batch: M4
    commits: [335cda4b]
  - id: F112
    source: F25
    description: "A case in tools/sync/tests/convert.rs pinning the accepted argument forms, flags before the positionals and --output=json: the widening is documented but not pinned by a test."
    status: completed
    batch: M4
    commits: [1cac4f4b]
  - id: F113
    source: F26
    description: The main girsync binary has no --version either, while gircap has one, so the workspace has no single convention.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a workspace-wide convention decision, outside this crate"
  - id: F114
    source: F26
    description: The DisplayVersion arm in sled2redb error path stays unreachable until a version is declared.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the documented consequence of not declaring a version, which F26 deliberately kept"
  - id: F115
    source: F51
    description: A safe reclaim of a stale conversion temporary would need a liveness marker beside the name, a lock like the hold records; without one, any sweep can take a live conversion file.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same kind as the backup hold records, and F51 deliberately chose to document rather than reclaim"
  - id: F116
    source: F26
    description: The Args doc comment is clap's long_about, so the note with its literal ** markers and the #[command(version)] attribute now appears in sled2redb --help; if it is meant for source readers only it belongs in the module doc rather than the struct's.
    status: completed
    batch: M4
    commits: [839bcf84]
  - id: F117
    source: F51
    description: DESIGN-NOTES.md's "Known limits, deliberately not fixed" list does not carry the converter's stale temporary, although that section is where such hygiene trade-offs are meant to be recorded and the working-copy case is already there.
    status: completed
    batch: M5
    commits: [e30e6c6f]
  - id: F118
    source: F51
    description: The recorded reasoning that a sweep could not tell a live temporary from a dead one does not address why the process id in the temporary's name is not used to test liveness, given pid reuse and cross-host containers.
    status: completed
    batch: M5
    commits: [13304c2f]
  - id: F119
    source: F31
    description: The per-feature check enables each declared feature alone, so a member that ever declares two features that only compile together would not be covered by the MSRV gate.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same defect as F111, which is already scheduled in Z1"
  - id: F120
    source: F64
    description: The new paragraph in "Backups and failures" largely repeats the paragraph immediately above it, so the two could be merged.
    status: completed
    batch: M5
    commits: [1409bbf6]
  - id: F121
    source: F20
    description: The copy phase's failure-selection rule is still pinned only end-to-end; a unit test on the selection itself, in apply/phases.rs, would pin it without any dispatcher assumption.
    status: completed
    batch: M6
    commits: [50e81f06]
  - id: F122
    source: F19
    description: The walk's other counters, scan start and scan progress, and the effective-map counters are still unpinned; only scan done is now.
    status: completed
    batch: M6
    commits: [2115d14e, 5a5cb7a2]
  - id: F123
    source: F20
    description: The blocking-destination fixture costs about half a second on this host because of antivirus; a smaller tree would be cheaper but would give a thinner margin.
    status: skipped
    batch: null
    commits: []
    note: "skipped: a cost trade-off with no defect, and the slower fixture is what gives the case its margin"
  - id: F124
    source: F44
    description: The design notes Enforced by list for the verify does not name the new canonical-order case, so a reader may expect it to be listed.
    status: completed
    batch: M1
    commits: [746f94cb]
  - id: F125
    source: F19
    description: The counters are pinned for one update run, but a pruned girpr-cache* subtree absence from them, and the fact that a sync run two walks each report their own counts, are still unpinned.
    status: completed
    batch: M6
    commits: [c4f93a7f]
  - id: F126
    source: F20
    description: The discrimination still rests on the removal tree outlasting a worker wake-up rather than on an explicit barrier, so a case that gated one copy on the other failure would remove the last ordering dependence.
    status: completed
    batch: M6
    commits: [6c16cf00]
  - id: F127
    source: F20
    description: "The reported failure is pinned by grepping stderr for substrings, so nothing pins the exact error text, copy <rel>: <cause>, a user actually sees."
    status: completed
    batch: M6
    commits: [20b42517]
  - id: F128
    source: F21
    description: CommonOpts::case_sensitive is still a public field, so nothing stops a future test from assigning it directly and re-introducing the field and filter disagreement the setter exists to prevent.
    status: skipped
    batch: null
    commits: []
    note: "skipped: making the field private is an API decision already weighed in the crate surface batch, and the setter plus the doc make the correct use discoverable"
  - id: F129
    source: F44
    description: The new case pins the algorithm in the bail message, but the error event algo field, the other channel a parser could read, is unpinned.
    status: completed
    batch: M6
    commits: [a5b3b2c1, 9c327065]
  - id: F130
    source: F45
    description: The README states the copy failure-message shape in two places joined by a cross-link, and nothing keeps the two from drifting.
    status: completed
    batch: M4
    commits: [0e73b57f]
  - id: F131
    source: F46
    description: Nothing enforces the trailing newline, so an editor that strips it can reopen the same gap.
    status: skipped
    batch: null
    commits: []
    note: "skipped: an editor convention rather than a defect, and the file now ends with one"
  - id: F132
    source: F22
    description: commands/sync/mod.rs still builds its two folder labels inline instead of the shared side_label; identical text today, but a second spelling of the same rule.
    status: completed
    batch: M8
    commits: [0fa1a663]
  - id: F133
    source: F27
    description: The same lowercase versus ASCII-case fold wording survives elsewhere, in src/diff/mod.rs, src/commands/sync/rename.rs and src/planner/pairs.rs, which says "Pair by lowercase key".
    status: completed
    batch: M7
    commits: [c9a512e8]
  - id: F134
    source: F27
    description: src/util.rs still uses a raw to_lowercase() comparison for the identity fallback, which is worth a look for the same reason the wording was corrected.
    status: completed
    batch: M7
    commits: [72d9e889]
  - id: F135
    source: F27
    description: The variant name CasePairing::Lowercase itself says lowercase although it folds ASCII case only; renaming it would touch every call site and the design notes.
    status: skipped
    batch: null
    commits: []
    note: "skipped: renaming a public variant for no user-visible gain, and the design notes name the rule"
  - id: F136
    source: F22
    description: The existing case assertion that the label resolves to the folder to run it on now holds only because the folder path is a prefix of the record path, so it is weaker than its comment claims.
    status: completed
    batch: M8
    commits: [339c0957]
  - id: F137
    source: F47
    description: The crate root now re-exports the whole glob crate rather than only glob::Pattern, which is wider than the type the public signatures actually name; the same shape as the existing girpr_log re-export, so a choice rather than a defect.
    status: skipped
    batch: null
    commits: []
    note: "skipped: it mirrors the existing girpr_log re-export precedent, and narrowing the re-export to one type would be a second convention in the same block"
  - id: F138
    source: F133
    description: "Local names still abbreviate the fold as lower: slow and dlow in diff/mod.rs and commands/sync/rename.rs, and by_lower in rename.rs."
    status: completed
    batch: M7
    commits: [8e5f5bd0]
  - id: F139
    source: F133
    description: "The lowercase wording for the ASCII-only fold survives outside the three files: commands/sync/plan/safety.rs, commands/sync/plan/link.rs, and the test suites tests/lazy/compare.rs, tests/helpers.rs, tests/compare_self.rs, tests/sync_plan/table.rs and tests/sync_plan/main.rs."
    status: completed
    batch: M7
    commits: [4dd86821, 1148ad43, 10ec5297]
  - id: F140
    source: F136
    description: The coverage message's remedy still prints the placeholder girsync update --dir <the folder this record describes> rather than the actual folder path, which the message names one line above; making it copy-pasteable would need a README update.
    status: completed
    batch: M5
    commits: [4c285d30]
  - id: F141
    source: F134
    description: has_cache_prefix uses its own ASCII-only Windows fold rather than the identity rule; intentional and documented, but a third case-fold site a reader has to notice.
    status: completed
    batch: M7
    commits: [0d360416]
  - id: F142
    source: F134
    description: util::identity_fold's lossy conversion folds two distinct invalid-UTF-8 byte sequences onto the replacement character, so two different non-UTF-8 names can compare equal; documented in the code, not fixed.
    status: skipped
    batch: null
    commits: []
    note: "skipped: inherent to the lossy conversion the identity rule is defined by, and it is documented"
  - id: F143
    source: F133
    description: The CasePairing::Lowercase variant still says lowercase although it folds ASCII case only.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F135, already recorded as skipped"
  - id: F144
    source: F134
    description: commands/sync/plan/link.rs:219 still folds the on-disk lookup with an inline to_string_lossy().to_lowercase(), so after the identity fold was named there is one named Unicode fold and one anonymous Unicode fold that answer different questions, the on-disk rule being the third rule the design notes describe.
    status: completed
    batch: M7
    commits: [5777bf8c]
  - id: F145
    source: F132
    description: The integration-test helpers tests/lazy/sync.rs, tests/lazy/compare_self.rs and tests/hash_mode/main.rs still build side labels inline, and since side_label is pub(super) inside commands the tests cannot reach it, so one spelling for every command holds in src but the test suites keep a second spelling.
    status: completed
    batch: M5
    commits: [0f3609a7]
  - id: F146
    source: F133
    description: The rename-order test fixture cannot distinguish sorted by the folded key from sorted by the exact key, since all three src names start with an uppercase letter and both orders are the same; a fixture mixing cases would make the test pin the folded order it claims to be about.
    status: completed
    batch: M5
    commits: [b08451c7]
  - id: F147
    source: F29
    description: "Both documents use a flag that does not exist: README and DESIGN-NOTES name --hash none, while the CLI only has --hash-all-of and --hash-any-of, so the real spelling is --hash-all-of none."
    status: completed
    batch: M9
    commits: [91d626b5]
  - id: F148
    source: F29
    description: "The host-coverage limit is still README-absent: host coverage of the link cases, where Windows without developer mode refuses the symlink calls and the cases self-skip, is only in the design notes though the README audience meets it."
    status: completed
    batch: M9
    commits: [1f493787, 4a2acaea]
  - id: F149
    source: F28
    description: The src/bin/sled2redb.rs binary has no row of its own in the source-layout table; it is named inside the convert.rs row.
    status: completed
    batch: M9
    commits: [dfaecf32]
  - id: F150
    source: F29
    description: The stat-only row can get stuck limit has two different names, the design notes one and the README another, so the wording differs enough to read as two limits.
    status: completed
    batch: M9
    commits: [8bbba952]
  - id: F151
    source: F28
    description: The README source-layout table has no entry for src/hash/tests.rs or src/planner/fixtures.rs; the inline test modules are covered by the surrounding prose, so this is cosmetic.
    status: completed
    batch: M9
    commits: [dfaecf32]
  - id: F152
    source: F28
    description: The new link.rs row shares its basename with the commands/sync/plan/ row's link.rs; naming the src-relative path would remove the ambiguity for a reader scanning for link.rs.
    status: completed
    batch: M9
    commits: [dfaecf32]
  - id: F153
    source: F32
    description: The README Integration tests bullet omits tests/convert.rs, tests/symlink.rs and tests/migration_lock.rs, and never says the converter cases need the convert feature, which is the same gap this batch fixed in the design notes.
    status: completed
    batch: M9
    commits: [ccf14a43]
  - id: F154
    source: F29
    description: The known-limits preamble still says each is cache hygiene or a race, not a wrong answer, which describes neither the host-coverage limit nor the filesystem case table.
    status: completed
    batch: M9
    commits: [1f493787, 4a2acaea]
  - id: F155
    source: F28
    description: The source-layout table still omits src/rw/fixtures.rs and src/hash/tests.rs, even though it lists the sibling planner/fixtures.rs.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F151, already scheduled in M9"
  - id: F156
    source: F39
    description: cache/db/backend.rs's SharedCache::reads doc and cachedb.rs's with_handle doc describe the same counter from the other side, and with_handle never says why the fresh counter is the cache's count for a file-backed cache.
    status: completed
    batch: M11
    commits: [6fc49e88]
  - id: F157
    source: F40
    description: The preflight open also opens the entries table, a presence check with no rows read, outside begin_entries_read, so a reader of that function could ask whether that transaction is counted.
    status: completed
    batch: M11
    commits: [6603ecaf]
  - id: F158
    source: F37
    description: plan/link.rs's module doc says the helpers are pure over the effective maps plus the link-resolution primitives, which reads as if they touch no disk, while on_disk_name, target_rel and spare_link_components all do.
    status: completed
    batch: M11
    commits: [dbc64313]
  - id: F159
    source: F39
    description: Seven pre-existing broken intra-doc links remain, for CacheDb twice, codec, Summary, super::super::apply::copy, AlgoSet and crate::effective::merge_row.
    status: completed
    batch: M11
    commits: [880b1319, 2ce5c4cc]
  - id: F160
    source: F36
    description: The clause that a refusal there does not leave dst as it was holds for a real run only, since a dry run performs no rename and changes nothing.
    status: completed
    batch: M12
    commits: [0e48eda4, 80ba72d1]
  - id: F161
    source: F37
    description: The opener that the link questions are asked of the disk directly reads as universal, but the hard-link question goes through the seam, so a reader could conclude the hard-link guard bypasses it.
    status: completed
    batch: M12
    commits: [312f8852]
  - id: F162
    source: F37
    description: plan/link.rs's own module doc still says only read_link and canonicalize probes, omitting the try_is_link and the Windows read_dir probes the safety doc now lists.
    status: completed
    batch: M12
    commits: [71434e08]
  - id: F163
    source: F37
    description: The old rationale for the bypass, that no plan-time decision is lost with it, went with the sentence it belonged to, and a clause saying what a direct question costs the seam would say more than the new list does.
    status: completed
    batch: M12
    commits: [7d01f95b]
  - id: F164
    source: F38
    description: The notes name the new case but never state the rule it pins, that the dst root is resolved at plan time, so a reader has to infer what the case enforces from the case itself.
    status: completed
    batch: M12
    commits: [1ff3d1ac]
  - id: F165
    source: F39
    description: The headline that the count belongs to the cache and not to the handle is true only of the shared backend, so scoping it to that backend would keep the next sentence from reading as a correction.
    status: completed
    batch: M12
    commits: [ce0cab8a]
  - id: F166
    source: F34
    description: DigestSet::insert_named rebuilds the whole foreign slice on every insert, including an in-place replacement, and merge does the same.
    status: completed
    batch: M13
    commits: [29b218f8, 82fd8a95]
  - id: F167
    source: F41
    description: The adoption arm allocates a Vec of references per exact-case miss just to count the alternates.
    status: completed
    batch: M13
    commits: [6955810c]
  - id: F168
    source: F41
    description: The phase-A prune still walks the whole key set every scan; only the alternate index became lazy.
    status: completed
    batch: M13
    commits: [a095f412, 34293501]
  - id: F169
    source: F34
    description: DigestSet::merge and insert_named spell the same-name comparison two ways.
    status: completed
    batch: M13
    commits: [515bfa49]
  - id: F170
    source: F33
    description: The fold-equivalence case uses eq_ignore_ascii_case as its oracle, so it would follow std rather than catch a std change.
    status: completed
    batch: M13
    commits: [95a2d302, 4db5ab6a]
  - id: F171
    source: F41
    description: fold_alternates allocates a Vec of Strings per folded group, and groups with several stale spellings are the common shape there.
    status: completed
    batch: M13
    commits: [ae982636, 6236f9fa]
  - id: F172
    source: F33
    description: The const-context pin lives in the crate test module, so it bites only when the tests are built; a plain cargo build would still accept a resolver that stopped being const, though one that allocated would fail in every build.
    status: completed
    batch: M14
    commits: [0f9e7d33, a747499a]
  - id: F173
    source: F34
    description: DigestSet::merge applies the same exact-name replacement rule as insert_named, but no case holds a row with two spellings of one algorithm across a merge, so the rule is pinned on the insert path only.
    status: completed
    batch: M14
    commits: [0fe48358, 2d6014e6, 6dd4294d]
  - id: F174
    source: F41
    description: The comment above the index in phase_a.rs still phrases the saving as a scan whose cached keys all match the spelling the disk uses never asking for an alternate, leaving out the run that only adds a file and still pays for the index.
    status: completed
    batch: M14
    commits: [3a65ca7f, 41656cb5, 2d6014e6]
  - id: F175
    source: F116
    description: Nothing pins the converter help output, so a doc comment on Args could come back and print the source note again; the existing help case only asserts the flags are named.
    status: completed
    batch: M15
    commits: [b1610c1e]
  - id: F176
    source: F111
    description: The MSRV gate still does not cover a combination whose parts each build alone, so a member whose features all build alone but whose combination needs a newer Rust is not checked.
    status: completed
    batch: M15
    commits: [45c4649e, e86a40aa]
  - id: F177
    source: F116
    description: The converter refusal of --version, which exits 3, now lives only in the module doc; the README converter section does not mention it.
    status: completed
    batch: M15
    commits: [493fce5b]
  - id: F178
    source: F130
    description: The README still quotes verify hash mismatch, the log event message, near the failure-message shape verify hash(<algo>) <path>, the error text; two different strings, but a reader could confuse them.
    status: completed
    batch: M15
    commits: [b0081392]
  - id: F179
    source: F111
    description: The MSRV fallback cannot tell this feature needs another from this feature is broken alone, so for a member with two or more optional features whose group builds, a genuinely broken lone feature is reported as covered by the group.
    status: completed
    batch: M16
    commits: [c0348f54]
  - id: F180
    source: F111
    description: "The opposite slip is possible in the MSRV script: a member that has both a combination-only feature and a mutually exclusive feature is still failed, because all optional features at once is not a configuration that builds there."
    status: completed
    batch: M16
    commits: [c0348f54]
  - id: F181
    source: F111
    description: The MSRV script description calls the group the only one that builds such a member and the root README says the group's features enabled; both would be more accurate as every optional feature of the member enabled at once.
    status: completed
    batch: M16
    commits: [673b99b1]
  - id: F182
    source: F112
    description: The new argument-forms case lives in the convert-gated test target, so a plain cargo nextest run never builds or runs it; only a --features convert run exercises the pin.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the converter cases are feature-gated by design, so the pin runs where the converter runs"
  - id: F183
    source: F116
    description: The help case only asserts that --force, --output and the positional order appear, so nothing in the suite would fail if a doc comment were put back on Args and the note started printing in --help again.
    status: completed
    batch: M15
    commits: [b1610c1e]
  - id: F184
    source: F130
    description: The README still carries both the log event string and the error text near each other, and a reader could still confuse the two.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F178, already scheduled in M15"
  - id: F185
    source: F145
    description: "Five more test helpers still build a side label inline, with different wording, src <cache path> and dst <cache path>, rather than the shared spelling; the helper is public now, and the current form also renders as src: src <path> in a coverage message."
    status: completed
    batch: M17
    commits: [f0cd71f7]
  - id: F186
    source: F146
    description: tests/sync.rs run_sync_case_rename_steps_over_occupied_temporaries asserts a rename order from two src names that both start uppercase, so like the fixture just fixed it cannot tell the folded order from the exact one.
    status: completed
    batch: M17
    commits: [a2bb2324]
  - id: F187
    source: F140
    description: side_folder returns a dot for a record named on its own, and that edge case has no test.
    status: completed
    batch: M17
    commits: [4c48b5cb]
  - id: F188
    source: F140
    description: Coverage::folder is an Option, so a caller that forgets it silently loses the remedy bullet rather than failing.
    status: completed
    batch: M17
    commits: [4d91eca7]
  - id: F189
    source: F140
    description: The README example block spells three commands the CLI rejects, girsync update src, girsync sync src dst and girsync compare-self src, while the prose directly beneath spells the same remedy correctly.
    status: completed
    batch: M18
    commits: [f4b00295]
  - id: F190
    source: F140
    description: side_folder takes a record's parent, so a record whose name is not literally girpr-cache, such as girpr-cache-backup-1 which is_record_path also classifies as a record, gets a remedy that repopulates the folder's girpr-cache rather than the file that failed.
    status: completed
    batch: M17
    commits: [4c48b5cb, 94a17885, 6c23cb4b]
  - id: F191
    source: F140
    description: side_folder is public but has no caller outside the crate yet, so the surface is justified only by the same one-spelling argument as side_label.
    status: completed
    batch: M17
    commits: [4d91eca7]
  - id: F192
    source: F117
    description: The design notes and the README name the conversion temporary only as girpr-cache-convert-<pid>-<ts>.redb, omitting the underscore-n retry spellings claim_temp_sibling can also produce.
    status: completed
    batch: M18
    commits: [a85579d2, b7f6afda]
  - id: F193
    source: F126
    description: The README test-suite section still says the suite has one test-only switch of its own, GIRSYNC_TEST_CACHE=memory, while the crate now has several default-off hooks, FAIL_COPY, FAIL_FLUSH, FAIL_RENAME and GATE_COPY, that are listed nowhere.
    status: completed
    batch: M19
    commits: [1b0c1f92, 7e12b998]
  - id: F194
    source: F126
    description: GIRSYNC_TEST_GATE_COPY splits on a colon, harmless for relative paths, but the constraint is stated only in the hook doc comment.
    status: completed
    batch: M19
    commits: [140b2db4, 0f243257]
  - id: F195
    source: F122
    description: walk_live's heartbeat period, 2000 entries, is a bare literal that a test now depends on; a named constant would let the code and the pin agree in one place.
    status: completed
    batch: M20
    commits: [14f77fa3]
  - id: F196
    source: F121
    description: The copy phase's selection rule is pinned by a unit test on the comparison, but the mapping from a plan index to the path the report names is only covered end-to-end.
    status: completed
    batch: M20
    commits: [30b37539]
  - id: F197
    source: F122
    description: The effective done event is logged once per phase and the two are told apart only by the presence of the stat_only field; a message that named the phase would remove that subtlety.
    status: completed
    batch: M20
    commits: [fa35b7d8]
  - id: F198
    source: F129
    description: The design notes Enforced by list for the verify names the canonical-order case but not the new error-event case, so the second channel's case is unlisted, the same gap F124 closed for the first one.
    status: completed
    batch: M19
    commits: [a3bff56e]
  - id: F199
    source: F122
    description: The two phases' progress totals are asserted as the set of two values, so the case does not say which phase reports the walk's count and which the map's.
    status: completed
    batch: M20
    commits: [858fa92e]
  - id: F200
    source: F138
    description: src/planner/pairs.rs still has the same src-lowered abbreviation F138 fixed in the other two files.
    status: completed
    batch: M21
    commits: [7e40bfb6]
  - id: F201
    source: F138
    description: The lk and lkey locals in diff/mod.rs, commands/sync/rename.rs and planner/pairs.rs still abbreviate lowercase key.
    status: completed
    batch: M21
    commits: [b8b98771]
  - id: F202
    source: F139
    description: src/effective/phase_a.rs still says the cached keys are indexed by lowercase, which is the ASCII-only fold wording in a file F139 did not list, though the index there is built with path_key.
    status: completed
    batch: M21
    commits: [c64275cf]
  - id: F203
    source: F141
    description: The design notes three-questions section names the path key, the identity rule and the disk spelling, but the now-named cache reservation is a fourth fold site the section does not mention.
    status: completed
    batch: M21
    commits: [cb20bfea]
  - id: F204
    source: F144
    description: on_disk_name recomputes the on-disk fold for every directory entry, one allocation per entry, and it could be hoisted out of the loop.
    status: completed
    batch: M21
    commits: [6b15098a]
  - id: F205
    source: F139
    description: effective/phase_a.rs still says the cached keys are indexed by lowercase for the ASCII-only fold, the same retired wording, outside that follow-up scope.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F202, already scheduled in M21"
  - id: F206
    source: F141
    description: has_cache_prefix's doc names ascii_case_eq without marking it Windows-only, so a POSIX reader is pointed at a helper that is not compiled there.
    status: completed
    batch: M22
    commits: [e5171086]
  - id: F207
    source: F138
    description: The new locals are src_folded and dst_folded although they hold folded-key indexes; names ending in _keys would say what they hold more exactly.
    status: completed
    batch: M22
    commits: [b683af1f]
  - id: F208
    source: F144
    description: on_disk_fold and util::identity_fold are now byte-identical Unicode-lowercase helpers in two modules; both docs warn against unifying them, but the duplication is visible to a reader.
    status: completed
    batch: M22
    commits: [0a942361]
  - id: F209
    source: F147
    description: The nonexistent --hash none still names a flag in source rather than in documents, in src/hash/set.rs module doc, parse_hash_list doc and the user-facing refusal, src/hash/algo.rs, src/effective, src/planner, src/commands/update.rs, src/commands/sync/apply/copy.rs and several test files; the set.rs refusal is user-visible text and the sharpest instance.
    status: completed
    batch: M23
    commits: [aaed2ebd, e03f21ab]
    note: review M23 kept; the retired spelling survives only where a case denies it and in the one historical statement, which the reviewer confirmed is still true
  - id: F210
    source: F147
    description: src/hash/set.rs module doc still says the --hash and --hash-any-of parsers, stale in the same way.
    status: completed
    batch: M23
    commits: [aaed2ebd]
    note: review M23 kept; the module doc now names the two parsers the file holds
  - id: F211
    source: F151
    description: The README source table still has no row for src/commands/sync/plan/tests.rs, an inline test module; cosmetic, and the surrounding prose covers inline test modules.
    status: completed
    batch: M23
    commits: [986fa9a0]
    note: review M23 kept; the row follows the table's own whole-path rule, which the same rule already applied to the sibling src/hash/tests.rs row
  - id: F212
    source: F151
    description: The hash/ row is the only directory row that does not name its facade, so src/hash/mod.rs is the one file in the tree with neither a row nor a mention.
    status: completed
    batch: M24
    commits: [6c8cb53e]
    note: review M24 kept; the row now names mod.rs as the facade, the wording the other facade rows use, and a sweep of the tree found every source file named in the README
  - id: F213
    source: F151
    description: The table path rule, relative to src/ unless the bare name is ambiguous or the file sits outside the module tree, does not cover the four rows that spell the whole path for neither reason, though every path is correct.
    status: completed
    batch: M24
    commits: [6c8cb53e]
    note: review M24 kept; the rule now has three cases and covers exactly the six whole-path rows, and every other row is a qualified src-relative path
  - id: F214
    source: F150
    description: The stuck-row limit names the flag as the repair, but girsync update --dir <folder> also rehashes a stat-only row, and the README already names it as the fix for the neighbouring record case.
    status: completed
    batch: M24
    commits: [645829a9]
    note: review M24 kept; the reviewer traced update through plan_one_side and merge_row and confirmed the repopulate really rehashes a stat-only row and writes the digest back
  - id: F215
    source: F154
    description: The preamble first kind, state in the cache or the folder that nothing reclaims on its own, does not fit the directory-row limit, which the next prune does reclaim; the limit own body says so, the preamble does not.
    status: completed
    batch: M24
    commits: [13120f7a]
    note: review M24 kept; the preamble now names five kinds and the reviewer confirmed they cover the six limits in listing order, none ungrouped and none twice
  - id: F216
    source: F159
    description: effective/phase_a.rs has an intra-doc link to util::folded_keys that does not resolve, because util is not in scope in that module; it was introduced after the survey that produced the seven named links and is the one warning left in the private-items doc build.
    status: completed
    batch: M25
    commits: [d42a90fd]
    note: review M25 kept; the link now points at the crate root, the private-items doc build went from one warning to none, and the rendered page carries a real hyperlink
  - id: F217
    source: F156
    description: cache/db/cachedb.rs's CacheDb::reads field doc opens with the same wording as SharedCache::reads; consistent today, but a second place to edit if the wording ever moves.
    status: completed
    batch: M25
    commits: [0339f805, 90d32f86]
    note: review M25 fixed (90d32f86 changes the field headline to the reads counted into the counter this handle holds, since the shared sentence was false for a read-only handle on a file-backed cache)
  - id: F218
    source: F156
    description: The claim that a file-backed cache reaches the same answer needs a clause saying the counter is fresh per open, since a file-backed handle starts from zero every time.
    status: completed
    batch: M26
    commits: [f26a1837]
    note: review M26 kept; the fresh-per-open clause is in with_handle and entries_read_count, and the reviewer confirmed a file-backed handle never inherits an earlier open's reads
  - id: F219
    source: F156
    description: redb skips its file lock entirely on a filesystem that cannot lock a file, so the exclusion the counter story rests on does not hold there; the crate acknowledges such filesystems elsewhere.
    status: completed
    batch: M26
    commits: [f26a1837, aa527d20, 95cc2721]
    note: review M26 fixed (aa527d20 bounds the shared counter's exclusion in backend.rs, 95cc2721 bounds the lock the bound takes in the design notes, since a cache that cannot be locked has no lock to take either)
  - id: F220
    source: F157
    description: The preflight new sentence calls the counted reads the ones that fetch rows, but entry_count and load_keys are counted too and fetch none, so begin_entries_read should stay the stated definition.
    status: completed
    batch: M26
    commits: [f26a1837]
    note: review M26 kept; the preflight and begin_entries_read now agree, and the reviewer checked that entry_count and load_keys both go through begin_entries_read
  - id: F221
    source: F158
    description: The claim that three of the link helpers ask the disk counts on_disk_name, whose non-Windows version asks nothing at all.
    status: completed
    batch: M26
    commits: [82854f75]
    note: review M26 kept; the module doc now says two helpers ask the disk on POSIX and three on Windows, and the reviewer checked each one against the code
  - id: F222
    source: F160
    description: plan/safety.rs module doc still says the guards that run after the rename refuse a plan that is already one change in, which is also a real-run-only statement.
    status: completed
    batch: M27
    commits: [b8ba41ec]
    note: review M27 kept; the claim now says a real run, and the same false claim in the Probes trait doc was corrected with it
  - id: F223
    source: F161
    description: ensure_hard_linked_targets_safe own doc does not say that its link count is a seam probe; that fact lives only in the seam doc.
    status: completed
    batch: M27
    commits: [6643ccf7]
    note: review M27 kept; the reviewer traced the count through probes.link_info to the hard-link info call and confirmed it asks a different question from the link-resolution ones
  - id: F224
    source: F161
    description: The seam list of the questions asked directly is kept in step by hand with the direct calls in safety.rs and plan/link.rs, and nothing pins it, so a new direct call could be added without the list noticing.
    status: completed
    batch: M27
    commits: [32ef16cf, 7e717ec5]
    note: review M27 fixed (7e717ec5 closes three spellings of the disk modules that defeated the scan and makes a glob import fail loudly); the reviewer checked the list by hand and ran eight probe scripts
  - id: F225
    source: F165
    description: CacheDb::reads field doc still says entries-table reads this cache has served, which the now-scoped method doc no longer supports for a file read-only handles, since each serves only its own reads.
    status: completed
    batch: M28
    commits: []
    note: already handled by 90d32f86, which changed the headline to the reads counted into the counter this handle holds; the implementer made no change and invented none, and the reviewer judged the claim true for all three handle kinds
  - id: F226
    source: F160
    description: safety.rs module doc still reads the guards that run after the rename refuse a plan that is already one change in, which is a real-run statement since a dry run rename pass changes nothing.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F222, already scheduled in M27"
  - id: F227
    source: F161
    description: The label link-resolution is loose for link_ancestor_above, which asks whether a link sits above a path rather than what anything resolves to, though the list that follows enumerates the direct questions exactly.
    status: completed
    batch: M28
    commits: [f368ae29]
    note: review M28 kept; the group label is now link, and the read_dir entry of the machine-readable list was brought in step with it
  - id: F228
    source: F168
    description: The prune still materialises the whole key set, a Vec of Strings with one string per cached row; a streaming key API on CacheDb and CacheWrite would remove that allocation on every scan, and it needs the cache module rather than phase A.
    status: completed
    batch: M29
    commits: [8ed05795]
    note: review M29 kept; the reviewer checked that the streaming prune applies the identical predicate to the identical key set, that the read counts did not move, and that the key borrow is held only for the iteration
  - id: F229
    source: F166
    description: DigestSet.foreign is a Vec now, so its capacity can exceed its length after growth and every clone of a row clones that capacity.
    status: completed
    batch: M29
    commits: [75fb6c4b]
    note: review M29 kept; the reviewer confirmed with a standalone probe that a clone of a one-entry arm has capacity one on this toolchain, so the premise's second half did not hold, and that the change makes it true by construction
  - id: F230
    source: F169
    description: same_spelling is a one-line wrapper over an equality comparison; it is the single place to change if the arm match rule ever stops being exact.
    status: completed
    batch: M29
    commits: [fbf844bb, 4d0c9a7e]
    note: review M29 fixed (4d0c9a7e scopes the claim to the exact-match rule, since a known name is matched by the resolver's fold and not through the helper, which the crate's own invariants already state separately)
  - id: F231
    source: F171
    description: The group second spelling exists so the rule does not depend on the disk own spelling never being in a group, a case a lookup cannot normally reach; if that invariant is ever stated as load-bearing, the second spelling could go.
    status: completed
    batch: M29
    commits: [02b0a441, 2b305551]
    note: "review M29 fixed (2b305551 restates the rule, which the pinned case contradicts: a group of two that holds the disk's own yields the other spelling, a group of two that does not is adopted by nothing)"
  - id: F232
    source: F170
    description: The fold oracle letter test still comes from std is_ascii_uppercase, so written out rather than borrowed holds for the comparison but not for the classification of a byte.
    status: completed
    batch: M30
    commits: [3a52da69]
    note: review M30 kept; the reviewer confirmed is_ascii_uppercase is gone from the code, that removing the clippy allow yields exactly one lint, and that the new case fails against ten wrong rules
  - id: F233
    source: F168
    description: The prune key set is still an owned Vec of Strings, one allocation per cached row for keys it only borrows, so a borrowed key view is the next step down that cost.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F228, already scheduled in M29"
  - id: F234
    source: F171
    description: The index still folds every cached key at the first exact-case miss, one folded spelling per key, even when a single lookup needs one group.
    status: completed
    batch: M30
    commits: [b0a92f16]
    note: review M30 kept; the reviewer compared the old and new lookup over 2,613,659 combinations with 0 mismatches and confirmed both paths build the group the same way
  - id: F235
    source: F166
    description: The arm keeps its capacity at the high-water mark after a replacement, where the old boxed slice was rebuilt to fit, so a row that once carried many foreign digests holds that capacity for life.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F229, already scheduled in M29"
  - id: F236
    source: F173
    description: DigestSet::merge clones the other side's foreign entries, name and digest, into the arm; a merge that replaces an entry in place reuses the digest buffer, but an added entry allocates a fresh String and Vec, the same allocation the vector itself removed.
    status: completed
    batch: M31
    commits: [7864335e]
    note: review M31 kept; the reviewer confirmed every merge caller hands over a row it drops, and that the new case fails when the clone is restored
  - id: F237
    source: F168
    description: The prune still walks the whole key set every scan even when the index was built, because the filter over it is a full pass; the index could answer whether a row is live per key.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F228, already scheduled in M29"
  - id: F238
    source: F174
    description: phase_a.rs's AltIndex builds a folded String per cached key on first miss; a fold into the index's own arena, or a hash of the folded bytes, would avoid the per-key allocation on large caches.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F234, already scheduled in M30"
  - id: F239
    source: F169
    description: The same_spelling helper in digestset.rs is now a one-line equality; it survives as the single place the rule is written, but it could be inlined with a comment if the crate prefers fewer indirections.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F230, already scheduled in M29"
  - id: F240
    source: F172
    description: The const block answer list is a hand-written subset of HashAlg::ALL, so a third algorithm would be covered by the runtime resolver case but silently not by the const one.
    status: completed
    batch: M32
    commits: [537ee293]
    note: review M32 kept; the reviewer confirmed the const block derives every answer from the vocabulary, that from_name itself is untouched, and that the discriminant comparison is the only const option on this toolchain
  - id: F241
    source: F174
    description: The index note that a run which only adds a file pays for the index holds only in insensitive mode, since in sensitive mode there is no alternate to look for and the index is never built, and the note does not say so.
    status: completed
    batch: M32
    commits: [1dd21971]
    note: review M32 kept; the reviewer confirmed the index is created only in the insensitive arm and that the new sensitive-mode case fails when the mode is flipped
  - id: F242
    source: F174
    description: The new index case pins the index build through the entries-table read counter, so a prune that ever moved to the counting handle would change the asserted numbers for a reason unrelated to the index.
    status: completed
    batch: M32
    commits: [8230e53e]
    note: review M32 kept; the reviewer confirmed alt_lookups moves only inside the index's adoptable, so it moves only on an exact miss in insensitive mode, and that no output changed
  - id: F243
    source: F177
    description: No case pins that sled2redb --version exits 3; the README now documents it but the suite does not hold it.
    status: completed
    batch: M33
    commits: [93ab7621]
    note: review M33 kept; the reviewer added a version to the converter and saw the case fail, so the refusal is really held
  - id: F244
    source: F175
    description: The help pin couples the converter test to the shared OutputFormat doc wording; a narrower pin over the converter own text would drop that coupling.
    status: completed
    batch: M33
    commits: [ba8570a6, eefb96ca]
    note: review M33 fixed (eefb96ca corrects the pin's comment, which claimed that any doc comment on the argument struct reaches --help, true only of a multi-paragraph one)
  - id: F245
    source: F176
    description: The MSRV fallback still covers a lone feature with the group only, so this feature needs another and this feature is broken alone remain indistinguishable there.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F179, already scheduled in M16"
  - id: F246
    source: F176
    description: The MSRV combination check cannot tell a combination that needs a newer Rust from one that does not build at all, so a member with mutually exclusive features is failed by the gate.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F180, already scheduled in M16"
  - id: F247
    source: F175
    description: The pinned help text also carries clap own boilerplate, the possible-values line, the default marker and the print-help wording, so a deliberate clap bump will need the golden text updated by hand.
    status: completed
    batch: M34
    commits: [6572e643, 3edc3d5e]
    note: review M34 kept; the golden holds no clap boilerplate, and the reviewer saw the pin fail for six mutations and pass under a patched clap_builder
  - id: F248
    source: F176
    description: The number of cargo invocations grows as two to the n minus one for a member with n optional features, so a member with many features would make the gate slow.
    status: completed
    batch: M34
    commits: [3546a457, 2d19bef1]
    note: review M34 fixed (2d19bef1 carries the shape the whole set cannot see into the README, which had repeated the reason without the exception)
  - id: F249
    source: F176
    description: No member in the workspace has two optional features today, girsync has one, so the combination branch is never exercised by a real run of the script.
    status: completed
    batch: M34
    commits: [c11652be]
    note: review M34 kept; the reviewer saw the self-test fail when the whole set is dropped, the holding-pair test inverted, the pair's exit code ignored and the loop order swapped
  - id: F250
    source: F177
    description: The converter refusal of --version is stated in two documents but no case pins the exit code 3, so adding the version attribute later would not fail anything.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F243, already scheduled in M33"
  - id: F251
    source: F179
    description: A member whose feature needs two other features at once, with no single partner building, is now failed; the script could report the building combination alongside the failure so the reader sees which configuration does build.
    status: completed
    batch: M35
    commits: [274d90e5]
    note: review M35 kept; the reviewer confirmed the named configuration always has a recorded exit code of zero, so it can be neither one that was never built nor one that failed
  - id: F252
    source: F181
    description: Get-OptionalFeatures and Get-FeaturePairs compare feature names case-insensitively while cargo feature names are case-sensitive; harmless in practice, untidy in principle.
    status: completed
    batch: M35
    commits: [c298eedc]
    note: review M35 kept; the reviewer extracted the old rules and confirmed they read the wrong property for a member with a feature named Default, so that member's optional features were never checked
  - id: F253
    source: F176
    description: The pair lookup reuses the combination results, so a member with many optional features pays for every subset of size two or more.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F248, already scheduled in M34"
  - id: F254
    source: F179
    description: The new rule needs exactly one partner, so a feature that compiles only with two partners together is failed by the feature check as well, and the description broken on its own does not name that case.
    status: completed
    batch: M36
    commits: [ee82d44c]
    note: review M36 kept; the behavioural half was already in place from an earlier batch and the policy is recorded as deliberate elsewhere, so only the description was left
  - id: F255
    source: F179
    description: Get-FeaturePairs re-derives the combination name with its own join instead of taking it from Get-FeatureCombinations, and an absent key is read as did not build, so the lookup silently depends on the combination check naming staying byte-identical.
    status: completed
    batch: M36
    commits: [07f0ef2a]
    note: review M36 kept; the named functions no longer exist, the missing-entry half did hold, and the reviewer confirmed the refusal is structurally unreachable for a real member
  - id: F256
    source: F179
    description: The MSRV fallback rule has no committed regression test, since the synthetic workspaces that exercise it live outside the tree, so a later edit to the fallback would only be caught by hand.
    status: completed
    batch: M36
    commits: [4693e6ff]
    note: review M36 kept; the new case really distinguishes the rule, and the reviewer confirmed the ten cases are independent
  - id: F257
    source: F180
    description: A member whose optional features include a mutually exclusive pair is still failed by the combination check, which predates this batch; the failure list now names the combinations alone.
    status: completed
    batch: M36
    commits: [bac52732, 0dc95b09]
    note: review M36 kept; the failure list is pinned to the combinations that failed, with no feature that a partner covers
  - id: F258
    source: F185
    description: tests/compare.rs still builds its two side labels inline, in a third wording, a bare cache path with no kind word; it was not among the files F185 named.
    status: completed
    batch: M37
    commits: [26f0ea58]
    note: review M37 kept; the reviewer confirmed no inline label spelling remains in any test and that the harness label reaches only a coverage error that cannot fire for its fixtures
  - id: F259
    source: F185
    description: Nothing pins the spelling of side_label, although six harnesses now depend on it.
    status: completed
    batch: M37
    commits: [53a51337]
    note: review M37 kept; the pin witnesses the wording and the side-versus-file distinction, and the reviewer reproduced two mutations that fail it
  - id: F260
    source: F191
    description: Side::cache_path and side_label answer different questions for a folder side, the cache file against the root, so a caller has to know which one it wants.
    status: completed
    batch: M37
    commits: [e81da0f2]
    note: review M37 kept; the accessor had exactly one caller, which was the bug itself, and the reviewer confirmed the labels stay byte-identical
  - id: F261
    source: F188
    description: Repopulate::Nothing covers two different reasons, a folder side and a record that is not a folder cache, and the message cannot say which.
    status: completed
    batch: M38
    commits: [2ca98413]
    note: review M38 kept; the reviewer ran both cases with the real binary and confirmed each reason now names itself and offers the right remedy
  - id: F262
    source: F188
    description: The Repopulate type lives in planner::coverage while the rule that fills it lives in commands::side_folder, so one rule is split across two layers.
    status: completed
    batch: M38
    commits: [5f86beca]
    note: review M38 kept; the reviewer confirmed the planner still does not import the commands layer and that the crate-root re-export set is unchanged
  - id: F263
    source: F190
    description: util::is_cache_name is pub with no caller outside the crate, the same surface question F191 raised for side_folder, though it does match the visibility of its counterpart is_record_path.
    status: completed
    batch: M39
    commits: [ec4566db]
    note: review M39 kept; the reviewer confirmed the stated reason is true of the crate, including that the helper has exactly one production call site and no out-of-crate caller
  - id: F264
    source: F186
    description: The added assertion in the rename fixture sorts a vector the test itself builds, so it states the fixture property rather than the program one and can only fail if the fixture is edited.
    status: completed
    batch: M39
    commits: [1c363bbb]
    note: review M39 kept; the reviewer mutated the pass to sort by the exact src key and saw the new assert_ne fail on its own
  - id: F265
    source: F190
    description: The README Gotchas bullet still says the error names the folder to run that fix on, which now holds only for a record that is a folder own cache, while the new paragraph above states the exception.
    status: completed
    batch: M39
    commits: [28a1a18b, c6663a65]
    note: review M39 fixed (c6663a65 replaces the claim that the other two ways out are still named, which is false for the default one-algorithm request, where only the none remedy is offered)
  - id: F266
    source: F189
    description: The hash-any-of flag is spelled with two algorithms as one occurrence in the README, the design notes, the CLI definition and the coverage remedy, but the parser rejects the second value, so only the repeated form parses; the coverage remedy in particular prints a command the user cannot paste, and the flag-order tie-break works correctly only with the repeated form.
    status: completed
    batch: M40
    commits: [a4740687, 4719ce4f, a17b7270]
    note: review M40 kept; the reviewer confirmed the printed remedy parses with the crate's own parser, that the refusal is clap's own and not vacuous, and that one surviving two-value spelling deliberately names the refused form
  - id: F267
    source: F189
    description: The README synopsis block lists the hash flags with one value per occurrence, which reads consistently with the CLI but not with the prose above it; the wording depends on which way the two-value form is settled.
    status: completed
    batch: M41
    commits: [a6850a91]
    note: review M41 kept; the reviewer ran every synopsis line with the placeholders filled and saw them all parse
  - id: F268
    source: F189
    description: The README update command names a placeholder where a real path would go, so it is prose rather than a runnable example, unlike the rest of the block it sits in.
    status: completed
    batch: M41
    commits: [1f950a9a]
    note: review M41 kept; the reviewer reproduced the walkthrough end to end and confirmed the README's folder names are the ones the rest of the README uses
  - id: F269
    source: F192
    description: The retry family is bounded by the claim-attempt limit at the thousandth suffix, but neither the README nor the design notes says that the run of names is bounded.
    status: completed
    batch: M42
    commits: [0ac9ccee]
    note: review M42 kept; the reviewer checked the bound in all five places and confirmed both places that have a limit use the same value, though it is declared twice
  - id: F270
    source: F192
    description: The inline case pins the second suffix exactly but not the third, so a rule change that kept the second and altered the later suffixes would pass it.
    status: completed
    batch: M42
    commits: [8a81858d, 8966a091]
    note: review M42 fixed (8966a091 corrects the test comment's phrase, which collided with the crate's sense of run as an execution of the tool)
  - id: F271
    source: F192
    description: The README bullet reads the retry list in the singular while the other two places read it in the plural.
    status: completed
    batch: M42
    commits: [afe44b2e]
    note: review M42 kept; the plural is right in all three places, since the list has more than one member
  - id: F272
    source: F189
    description: The commit message of the example-command fix claims every other README example command parses, which is false because of the two-value hash flag; the message is part of the history and cannot be corrected without rewriting it.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the claim is in a commit message and history must not be rewritten; the README text itself is correct, and the false example is tracked as F266"
  - id: F273
    source: F193
    description: The workspace nextest configuration points at a source path that no longer exists, because the module was split into a directory, so the pointer a reader would follow leads nowhere.
    status: completed
    batch: M43
    commits: [c582a5b5]
    note: review M43 kept; the reviewer confirmed each pointer lands on a file that exists and holds what the pointer says, and that no override filter was touched
  - id: F274
    source: F193
    description: The three rename failure hooks are three separate variables where one variable with a stage value would match the shape of the copy and flush hooks; the asymmetry is undocumented.
    status: completed
    batch: M43
    commits: [df01aaf7]
    note: review M43 kept; every claim in the committed text is true of the code, including that the pass aborts at the first injected failure
  - id: F275
    source: F193
    description: The commit-policy overrides are read once per cache write handle while the durability switch is read once per transaction, so two sibling switches have different granularities.
    status: pending_review
    batch: M44
    commits: [819da84b, f6878407]
  - id: F276
    source: F194
    description: A malformed copy-gate value now degrades silently to no gate, and the crate already has a precedent for a parsed test-only value, which reports an unparseable limit in the log, so the same value could be made visible instead of leaving a gated test racy.
    status: pending
    batch: M45
    commits: []
  - id: F277
    source: F194
    description: The relative-path check accepts shapes that were never spelled out, a Windows rooted path, a parent component and a trailing separator, and those name no copy and are therefore inert, so the check could name the shape it wants rather than only the ones it excludes.
    status: pending
    batch: M45
    commits: []
  - id: F278
    source: F197
    description: The walk's heartbeat is the one remaining progress event whose name does not say what produced it, since it is told apart by its entries field, and naming it for the walk would make the family uniform.
    status: pending
    batch: M46
    commits: []
  - id: F279
    source: F197
    description: The start events name their phase by operation while the done and the progress events now name it by letter, so the family uses two spellings.
    status: pending
    batch: M46
    commits: []
  - id: F280
    source: F196
    description: The end-to-end copy-failure case assumes the plan's copy list is sorted by path, which the plan builder does, but nothing pins that ordering rule itself.
    status: pending
    batch: M47
    commits: []
  - id: F281
    source: F196
    description: The helper that names the copy for a plan index indexes the plan's copy list and would panic on an out-of-range index, which cannot happen today, while a borrowing form would fail loudly instead.
    status: pending
    batch: M47
    commits: []
  - id: F282
    source: F196
    description: The new unit case hand-builds a plan whose copies are listed in alphabetical order, so it cannot tell the plan's order from the sorted order, and a fixture whose plan order was not alphabetical would make the pin sharper.
    status: pending
    batch: M47
    commits: []
  - id: F283
    source: F197
    description: The README bullet says a side's map, but a record side runs neither the reading walk nor the hashing phase and so logs no done event at all.
    status: pending
    batch: M48
    commits: []
  - id: F284
    source: F199
    description: The hashing phase's total is the planner's pending count rather than the map's size, so the case's wording that it counts only what the map kept holds only because the fixture's cache is cold.
    status: pending
    batch: M48
    commits: []
  - id: F285
    source: F195
    description: The heartbeat constant is now public API purely so the integration test can read the same constant, and nothing else in the crate needs it public.
    status: pending
    batch: M48
    commits: []
  - id: F286
    source: F197
    description: The walk's heartbeat keeps the bare progress name while the phases now name their phase, so a grep for that name still matches three events and only the fields separate them.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F278, already scheduled in M46"
  - id: F287
    source: F204
    description: The planner's lowercase pairing branch still computes the path key for every destination path, one allocation per path, where a borrowed comparison or a destination index would remove it.
    status: pending
    batch: M49
    commits: []
  - id: F288
    source: F204
    description: The remaining per-entry allocation in the disk listing is the fold of the candidate name, and a non-allocating comparison with an ASCII fast path would remove it.
    status: pending
    batch: M49
    commits: []
  - id: F289
    source: F203
    description: The digest-set and report modules fold algorithm and event names with an ASCII lowercase, which is unrelated to the path folds, so a reader scanning the crate for folds meets it without knowing it is a different question.
    status: pending
    batch: M50
    commits: []
  - id: F290
    source: F203
    description: The cache-prefix helper's doc names the ASCII-case comparison without marking it Windows-only, so a POSIX reader is pointed at a helper that is not compiled there.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F206, already scheduled in M22"
  - id: F291
    source: F203
    description: The new fourth fold site names the cache-prefix helper but not its exact-name counterpart, which states the same ASCII-only comparison.
    status: pending
    batch: M50
    commits: []
  - id: F292
    source: F204
    description: The hoist folds the wanted name eagerly, before the listing and before the exact-match early return, so the common exact-match hit now pays one allocation where it previously paid none, and folding at the first non-exact candidate would keep that path allocation-free.
    status: pending
    batch: M49
    commits: []
  - id: F293
    source: F204
    description: The remaining per-entry allocation in the disk listing is the candidate own fold, which a non-allocating comparison with an ASCII fast path would remove.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F288, already scheduled in M49"
  - id: F294
    source: F207
    description: The plan-safety module still names its folded index as if it held keys, though it maps a folded key to a value rather than being the key-shaped index the other modules name.
    status: pending
    batch: M51
    commits: []
  - id: F295
    source: F206
    description: The hash test module has its own second spelling of the ASCII-case comparison, harmless but a second spelling of the same rule.
    status: pending
    batch: M52
    commits: []
  - id: F296
    source: F206
    description: The utility module's public surface is asymmetric, with the same-path and the path-within helpers public while the path key and the folded-keys helper are crate-private.
    status: pending
    batch: M51
    commits: []
  - id: F297
    source: F208
    description: No case pins that the identity rule and the on-disk lookup agree on a non-ASCII spelling now that they share one mapping, although the two sites are covered separately.
    status: pending
    batch: M52
    commits: []
  - id: F298
    source: F208
    description: The shared mapping's doc still says it is the approximation the identity rule has always made, naming only one of its two users now that it serves both.
    status: pending
    batch: M53
    commits: []
  - id: F299
    source: F208
    description: The design notes' fold section still says the on-disk lookup and the key sit side by side in one file, though the path key itself lives in the utility module.
    status: pending
    batch: M54
    commits: []
  - id: F300
    source: F208
    description: The new notes paragraph says the value is written once rather than twice in two modules, a comparison with the old state that the notes own rule against recording changes discourages.
    status: pending
    batch: M54
    commits: []
  - id: F301
    source: F208
    description: The on-disk lookup's doc points at the shared mapping as plain code where the two neighbouring references use intra-doc links.
    status: pending
    batch: M53
    commits: []
  - id: F302
    source: F207
    description: The renamed indexes carry a keys suffix while the helper's own doc calls it the folded-key index, so the names do not match the wording exactly.
    status: pending
    batch: M55
    commits: []
  - id: F303
    source: F207
    description: The POSIX build warns about an unused import in the sync test module, which predates this work.
    status: pending
    batch: M55
    commits: []
  - id: F304
    source: F210
    description: The new parse_any_of doc says the refusal is made there so it does not point a user who typed --hash-any-of at a flag they did not, while the message itself names --hash-all-of none as the remedy, a wording that reads as a contradiction on a quick pass.
    status: pending
    batch: M56
    commits: []
  - id: F305
    source: F209
    description: The crate still spells --hash-any-of sha256 md5 with two algorithms in one occurrence in the README, the design notes, the CLI definition and the coverage remedy, a spelling the parser rejects, so the two spellings now coexist after this batch.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F266, already scheduled in M40"
  - id: F306
    source: F211
    description: The new source-table row is a whole-path row, so the table path rule now has one more whole-path row to account for than the three it counted.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F213, already scheduled in M24, whose count has been updated to four rows"
  - id: F307
    source: F213
    description: The path rule's first case could say the row's own path, so a reader does not apply it to the bare file names inside a directory row's cell, since the commands/sync/plan/ row names link.rs too.
    status: pending
    batch: M57
    commits: []
  - id: F308
    source: F214
    description: The design notes' stuck-row limit still omits the README's describing content it cannot verify clause, so the two documents agree on the repair but not quite on the symptom.
    status: pending
    batch: M57
    commits: []
  - id: F309
    source: F215
    description: Only the design notes group the limits into kinds, so the README's Gotchas list has no counterpart that has to be kept in step with them.
    status: pending
    batch: M57
    commits: []
  - id: F310
    source: F217
    description: with_handle's closing sentence calls the shared backend the one case where several handles are alive on one cache at once, which contradicts the paragraph above it saying read-only handles on a file share the file with each other; what distinguishes the shared backend is the single counter, not the number of live handles.
    status: pending
    batch: M58
    commits: []
  - id: F311
    source: F217
    description: The rule that one counter is shared by every handle on a path is now spelled in four places, SharedCache::reads, CacheDb::reads, entries_read_count and with_shared_reads, so a change to it has four places to edit.
    status: pending
    batch: M58
    commits: []
  - id: F312
    source: F217
    description: The field's headline and its accessor entries_read_count now describe the same number in different words, and on the shared backend only the accessor's wording is the loose one.
    status: pending
    batch: M58
    commits: []
  - id: F313
    source: F216
    description: The same item is now spelled two ways in the tree, crate::util::folded_keys in the phase A doc link, but plain util::folded_keys in three prose mentions, in effective/phase_a.rs, commands/sync/plan/safety.rs and commands/sync/plan/link.rs.
    status: pending
    batch: M59
    commits: []
  - id: F314
    source: F216
    description: The workspace-wide private-items doc build still reports 16 warnings for girpr and 5 for gircap.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the warnings are in the other crates of the workspace, outside the crate this plan covers"
  - id: F315
    source: F219
    description: src/cache/open.rs's in-code comment says the cache's own exclusive lock is held, so no other run is in its run phase for the folder, which is false on a filesystem that cannot lock a file, where redb skips the lock entirely.
    status: pending
    batch: M60
    commits: []
  - id: F316
    source: F219
    description: retain_copies' doc says a folder with no lock to take has the copies a live run published there protected by the records beside them instead, which holds for a legacy sled directory but not on a filesystem that cannot lock a file, where the record is a lock and there is none.
    status: pending
    batch: M60
    commits: []
  - id: F317
    source: F219
    description: The README states the lock exclusion unconditionally in two places in the bound section, and its One cache per run bullet says a read/write open locks its file exclusively, with no mention of the filesystem that cannot lock one.
    status: pending
    batch: M60
    commits: []
  - id: F318
    source: F222
    description: ensure_link_safe's own doc still says it refuses before any apply phase changes anything, which is true of the apply phases but reads as before anything changes even though the rename has already changed dst in a real run.
    status: pending
    batch: M61
    commits: []
  - id: F319
    source: F223
    description: The hard-link probe answers with the link count and the inode identity, and the identity is what credits a planned removal, so it asks how many names a file has names only the half the refusal turns on.
    status: pending
    batch: M61
    commits: []
  - id: F320
    source: F224
    description: The scan covers only safety.rs and plan/link.rs, so a direct call made from a third file of the plan module would not be seen.
    status: pending
    batch: M62
    commits: []
  - id: F321
    source: F224
    description: The scan knows only std::fs, crate::link and crate::hardlink as disk-reaching modules, so a direct call to a newly added disk-reaching module would not be seen.
    status: pending
    batch: M62
    commits: []
  - id: F322
    source: F224
    description: The test compares the file and call pairs only, so the asks prose in each entry is unchecked and can drift from the code.
    status: pending
    batch: M62
    commits: []
  - id: F323
    source: F224
    description: The list's file field hardcodes two file names, so splitting the guards across a third file would need the field and the test's file list changed together.
    status: pending
    batch: M62
    commits: []
  - id: F324
    source: F227
    description: The list's group membership exists only in prose; DirectQuestion carries file, call and asks but nothing that says which group a question belongs to, so a machine-readable consumer cannot recover it.
    status: pending
    batch: M63
    commits: []
  - id: F325
    source: F227
    description: The link_ancestor_above entry says the action for the plan action it guards; it could name the planned action explicitly, as the guard does.
    status: pending
    batch: M63
    commits: []
  - id: F326
    source: F227
    description: The check that keeps the list in step scans only safety.rs and plan/link.rs, so a direct disk call made from a third file of the plan module would not be seen.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F320, already scheduled in M62"
  - id: F327
    source: F227
    description: The scan recognises only std::fs, crate::link and crate::hardlink as disk-reaching modules, so a call through a newly added disk-reaching module would not be seen.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F321, already scheduled in M62"
  - id: F328
    source: F227
    description: The seam's own Probes::canonicalize, the canonical form of path with every link resolved, is itself a link question asked through the seam, while the Probes doc now says the link questions are asked directly and carves out only the hard-link count; the label's scope could name the dst-root resolution too.
    status: pending
    batch: M64
    commits: []
  - id: F329
    source: F228
    description: The streaming callback runs while the cache's read transaction is still open, so a callback that opened a write transaction on the same cache would block on redb's write lock; neither visit_keys doc says so.
    status: pending
    batch: M65
    commits: []
  - id: F330
    source: F228
    description: CacheWrite::load_keys now has no in-crate caller, since the prune streams instead, so it survives only as the collecting counterpart its own doc points new callers at.
    status: pending
    batch: M65
    commits: []
  - id: F331
    source: F229
    description: The no spare capacity property is pinned by tests but still leans on collect's sizing hint; building the arm with an explicit Vec::with_capacity(len) would make it exact by construction rather than by the iterator's hint.
    status: pending
    batch: M66
    commits: []
  - id: F332
    source: F229
    description: merge's shrink_to_fit costs one reallocation for every row carrying a foreign digest, on a path both phase A and phase C walk.
    status: pending
    batch: M66
    commits: []
  - id: F333
    source: F231
    description: The mixed-case guard is what keeps the disk's own spelling out of a group in practice, and the group docs do not name it, so a reader has to re-derive why the invariant holds today.
    status: pending
    batch: M67
    commits: []
  - id: F338
    source: F232
    description: The design notes' Enforced by list for the digest-set invariant still names only the_resolver_folds_exactly_as_the_ascii_case_comparison_does, so the case that now pins the oracle's own classification is not listed.
    status: pending
    batch: M69
    commits: []
  - id: F339
    source: F234
    description: The README's alternate-case index paragraph still reads as though the whole index is built at the first exact-case miss and does not say that the folded groups are materialised only from the second lookup on.
    status: pending
    batch: M69
    commits: []
  - id: F340
    source: F234
    description: The path key's rule is now stated as a comparison in a second form, std::eq_ignore_ascii_case in phase_a against util::path_key's fold, with util::ascii_case_eq covering the Windows-only sites, so a cross-platform crate-owned comparison helper could hold the three together in one place.
    status: pending
    batch: M70
    commits: []
  - id: F341
    source: F232
    description: The oracle's independence from std is a source-level property that no behavioural case can catch, and the crate has a precedent for a source-scanning case in the plan safety module.
    status: pending
    batch: M72
    commits: []
  - id: F342
    source: F234
    description: The folded index's map keys are still one owned String per distinct folded key; the hash-of-the-folded-bytes route was rejected because a collision would merge two distinct folded spellings and could adopt the wrong row.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same cost as F336, the index's own per-row string, already scheduled in M72"
  - id: F343
    source: F234
    description: The index still copies the whole key set on the first miss, so a streaming first lookup would avoid that copy too, at the price of a second table pass for the prune.
    status: pending
    batch: M72
    commits: []
  - id: F344
    source: F234
    description: The prune's key set is still an owned Vec of Strings when the index is built, one allocation per cached row for keys the index holds.
    status: pending
    batch: M72
    commits: []
  - id: F345
    source: F236
    description: The new prose in DigestSet::merge claims a replacement allocates nothing, but extend_from_slice reallocates when the new digest is longer than the arm's buffer, and the crate's own insert_named words the same claim as when it is big enough, so the two writers' wording now drifts.
    status: pending
    batch: M73
    commits: []
  - id: F346
    source: F236
    description: The merge's foreign-add path is not reached by the crate's own runs, since Digests::finish fills the known slots only, so the two allocations it removes are not on a run's hot path; the allocation the merge path actually pays is the prior row's clone in merge_row.
    status: pending
    batch: M73
    commits: []
  - id: F334
    source: F228
    description: The apply-phase prunes still materialise the whole key set with load_keys and remove while walking it; they could stream and keep only the rows to remove.
    status: pending
    batch: M71
    commits: []
  - id: F335
    source: F228
    description: The rename re-key still collects the whole key set before reading the moved rows; it needs the pairs anyway, so the win is smaller.
    status: pending
    batch: M71
    commits: []
  - id: F336
    source: F234
    description: The alternate-case index still owns one String per cached row whenever it is built, since it names positions in them and hands a spelling back, so the key set's per-row allocation survives a run that misses a path exactly.
    status: pending
    batch: M72
    commits: []
  - id: F337
    source: F228
    description: A write handle cannot remove rows while streaming, since the callback borrows the handle; a prune-as-you-stream method would remove that collection, at the price of a cache-specific API.
    status: pending
    batch: M71
    commits: []
  - id: F347
    source: F240
    description: The const block's doc says the ASCII-case spelling is the one answer this block cannot hold, whereas what it cannot hold is the derived one, since a hand-written uppercase literal would fit in a const block and writing one is what this change removed.
    status: pending
    batch: M74
    commits: []
  - id: F348
    source: F240
    description: HashAlg::ALL is a fixed-size array, so adding a variant to the enum does not force ALL to grow, and a variant left out of ALL would be uncovered by anything; a const assertion that the entries are in order would catch an entry replaced rather than appended.
    status: pending
    batch: M74
    commits: []
  - id: F349
    source: F240
    description: hash_names_fold_ascii_case_only and from_str_is_case_insensitive_without_help still spell md5 and sha256 by hand; they read as examples rather than as coverage, but a reader could take them for coverage.
    status: pending
    batch: M74
    commits: []
  - id: F350
    source: F242
    description: The scan's one pass over the entries table's keys, which the README still asserts, is no longer pinned by any case, since the converted case's read-count assertion was its only pin and pinning it that way is what made it fragile.
    status: pending
    batch: M75
    commits: []
  - id: F351
    source: F242
    description: ScanStats::alt_lookups is returned but not logged; the effective done phase a event could carry it, which is user-visible and would need a README line.
    status: pending
    batch: M75
    commits: []
  - id: F352
    source: F242
    description: cache/db/cachedb.rs's entries_read_count seam now has no phase-A user left; the other users, cachedb's own cases and commands/sync/rename.rs, remain.
    status: pending
    batch: M76
    commits: []
  - id: F353
    source: F244
    description: The converter binary's own module doc states the same rule about a doc comment reaching --help without the multi-paragraph qualification, and should be aligned with the corrected test comment.
    status: pending
    batch: M78
    commits: []
  - id: F354
    source: F244
    description: without_shared_value_wording cuts every Possible values block rather than the --output one, so a future flag with its own value descriptions would have those cut too.
    status: pending
    batch: M78
    commits: []
  - id: F355
    source: F243
    description: The main girsync binary also declares no version, so girsync --version is an unknown flag too, and nothing in the suite pins that; the same shape as the converter's case, in the ungated target.
    status: pending
    batch: M79
    commits: []
  - id: F356
    source: F244
    description: The pinned help still contains the default marker, which comes from the shared enum's default variant's spelling, so renaming that variant would break the converter's pin.
    status: pending
    batch: M78
    commits: []
  - id: F357
    source: F243
    description: The no version line at all assertion checks stderr only, while a declared version would go to stdout.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the exit-code and stdout assertions already carry the refusal on their own, so there is nothing to change"
  - id: F358
    source: F249
    description: The script's rules live outside the crate's test suite; a Rust test that runs the self-test under cargo nextest, skipping where pwsh is absent, would put them under the usual gate.
    status: pending
    batch: M80
    commits: []
  - id: F359
    source: F249
    description: "Only the feature branch is injectable: the toolchain probe and the per-member default check are still untested."
    status: pending
    batch: M80
    commits: []
  - id: F360
    source: F248
    description: The feature checks now build through a scriptblock, so cargo's output goes through Out-Host and the CI log shows cargo's stderr before its stdout.
    status: pending
    batch: M80
    commits: []
  - id: F361
    source: F247
    description: The accepted-value names are pinned by an assertion inside the help pin's case rather than in a case of its own, so a value rename is reported under the render pin's test name.
    status: pending
    batch: M81
    commits: []
  - id: F362
    source: F247
    description: The golden is a hand-written mirror of the bin target's derive definition, so the converter's command is now written twice and the pin is the only thing keeping the two in step.
    status: pending
    batch: M81
    commits: []
  - id: F363
    source: F248
    description: The whole-set configuration exists only from three features up, since with two the pair is the whole set, which is correct but leaves that case implicit in the rules.
    status: pending
    batch: M82
    commits: []
  - id: F364
    source: F249
    description: The self-test prints the rules' own FAIL lines for its negative cases, so a green CI log contains FAIL lines.
    status: pending
    batch: M82
    commits: []
  - id: F365
    source: F248
    description: The pair lookup re-derived a pair's name and read an absent key as did not build.
    status: skipped
    batch: null
    commits: []
    note: "skipped: moot as written, since the plan now holds the names and every pair is checked before it is read"
  - id: F366
    source: F248
    description: Middle-sized subsets are no longer checked, so a member with four or more optional features that needs a newer Rust in exactly one subset would not be caught.
    status: skipped
    batch: null
    commits: []
    note: "skipped: documented in the script, the module and the README, and the shape it misses is named there too"
  - id: F367
    source: F252
    description: The metadata reader cannot hold case-differing feature names, since ConvertFrom-Json refuses a JSON object with two keys that differ only in case, so a member declaring alpha and Alpha fails at the metadata step rather than being modelled.
    status: pending
    batch: M83
    commits: []
  - id: F368
    source: F251
    description: The combination named beside a failing feature is the whole set, since the rules do not build the smaller combinations, so a member with many features whose failing feature needs only two of them is told about a wider configuration than it needs.
    status: pending
    batch: M83
    commits: []
  - id: F369
    source: F251
    description: A feature that needs two partners is failed even though the whole set builds; that is deliberate, since a feature alone is a configuration cargo allows, but it is a policy the rule states rather than a necessity.
    status: pending
    batch: M83
    commits: []
  - id: F370
    source: F251
    description: The README's new sentence says the two-partner failure names the combination that does build it, which is the whole set of the member's features, but when the whole set itself fails nothing is named beside the feature failure, so the sentence over-states the guarantee.
    status: pending
    batch: M83
    commits: []
  - id: F371
    source: F251
    description: Only singles, pairs and the whole set are built, so for a member with four or more optional features the minimal building combination is never built and cannot be named; the whole set is named instead.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F368, already scheduled in M83"
  - id: F372
    source: F252
    description: The default-set lookup chooses the exact name default and then re-reads the value through the case-insensitive property indexer, so a features object holding both default and Default would read the wrong value; it is unreachable today, but selecting the property rather than the name would be exact.
    status: pending
    batch: M84
    commits: []
  - id: F373
    source: F252
    description: The ordinal exit-code map and the exact partner and combination containment have no self-test case, because neither a PowerShell hashtable nor the self-test's ordered dictionary can hold two feature names differing only in case.
    status: pending
    batch: M84
    commits: []
  - id: F374
    source: F252
    description: cargo metadata piped into ConvertFrom-Json refuses a features object whose keys differ only in case, although cargo accepts such a manifest, so a member declaring both default and Default kills the MSRV script at the metadata parse.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F367, already scheduled in M83"
  - id: F375
    source: F256
    description: The toolchain-install fallback in check-msrv.ps1, installing an absent rust-version toolchain and refusing a probe failure that is not an absent toolchain, still has no committed test, since it needs a workspace whose declared rust-version is not installed.
    status: pending
    batch: M85
    commits: []
  - id: F376
    source: F256
    description: No committed case for a member that declares no default feature set, where every declared feature is then optional, nor for a member that declares no features at all.
    status: pending
    batch: M85
    commits: []
  - id: F377
    source: F257
    description: Middle subsets are never built, so a feature needing two partners is only named as building when the whole set builds; otherwise the failure says only that the feature does not build alone, with no configuration to try.
    status: pending
    batch: M85
    commits: []
  - id: F378
    source: F257
    description: The feature failure puts the combination inside the same parentheses as the exit code, so a parser splitting the list on the features flag can mis-read the feature name.
    status: pending
    batch: M85
    commits: []
  - id: F379
    source: F255
    description: The refusal's own path has no committed case, since the flow builds every configuration of two or more features before the lookups, so only a direct call to the helper would pin that a missing entry is refused rather than read as a build that failed.
    status: pending
    batch: M86
    commits: []
  - id: F380
    source: F255
    description: The helper takes a configuration's name rather than the configuration, so the rule that a lookup cannot ask for a spelling the check did not build rests on the two call sites passing the configuration's own name rather than on the type.
    status: pending
    batch: M86
    commits: []
  - id: F381
    source: F260
    description: Side::cache_file now has no caller in the crate, since its only one was the harness that built its own label; it is kept as documented public API, but the crate could decide to drop it and let callers join the cache prefix themselves.
    status: pending
    batch: M87
    commits: []
  - id: F382
    source: F260
    description: src/cache/open.rs's remove_cache_path still says path for a cache file, where the crate now says cache file for a side's file; the vocabulary could be aligned, or deliberately left since that function names a path it removes.
    status: pending
    batch: M87
    commits: []
  - id: F383
    source: F260
    description: SideRequest::label in the planner and Coverage::label still describe the label as a record or folder path, while it is now the kind word plus the side's own path.
    status: pending
    batch: M88
    commits: []
  - id: F384
    source: F259
    description: tests/compare_self.rs pins the record-side wording inside a coverage message, while the folder-side wording is only pinned by the new case, so that message-level pin could be widened to both kinds.
    status: pending
    batch: M88
    commits: []
  - id: F385
    source: F260
    description: The side-versus-cache-file rule is stated in the two accessors' docs and enforced by a test, but DESIGN-NOTES.md has no do-not-merge-these-two-accessors entry of the kind it gives the sibling scan_stat_only and load_record_side_from rule, so a reader deciding to fold them would re-derive the reasoning from the code.
    status: pending
    batch: M87
    commits: []
  - id: F386
    source: F262
    description: side_label still lives in commands while its sibling side_folder now lives in effective::side, so the two side-description rules are in two places.
    status: pending
    batch: M89
    commits: []
  - id: F387
    source: F262
    description: Coverage is not re-exported at the crate root, so Repopulate is not either; if a caller outside the crate is expected to build a Coverage by hand, both would want a root re-export.
    status: pending
    batch: M89
    commits: []
  - id: F388
    source: F261
    description: The README Gotchas bullet still claims the error names the folder to run the fix on, which holds only for a record that is a folder's own cache.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F265, already scheduled in M39"
  - id: F389
    source: F261
    description: planner::fixtures and the hash_mode tests' plan() use Repopulate::NotAFolderCache for abstract synthetic sides that are neither a record nor a folder, so a fixture-triggered coverage message would say not a folder's own cache about a folder-ish side.
    status: pending
    batch: M90
    commits: []
  - id: F390
    source: F262
    description: util::is_cache_name is pub with no caller outside the crate, and its only in-crate caller is now effective::side rather than commands.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F263, already scheduled in M39"
  - id: F391
    source: F264
    description: src/commands/sync/rename.rs's unit test the_rename_order_is_by_the_folded_key has the same shape the rename finding names, sorting a vector the test itself builds and asserting the literal.
    status: pending
    batch: M92
    commits: []
  - id: F392
    source: F263
    description: is_record_path is public with no out-of-crate caller either; if the crate ever wants the util surface narrowed, that is the pair to decide together.
    status: pending
    batch: M91
    commits: []
  - id: F393
    source: F263
    description: The util module publishes helpers no outside caller uses, so a stated policy for the module's surface would settle these in one place rather than one at a time.
    status: pending
    batch: M91
    commits: []
  - id: F394
    source: F263
    description: The design notes' four fold sites section never mentions the exact-name rule or the repopulation rule, so that heading no longer describes every fold site the crate has.
    status: pending
    batch: M93
    commits: []
  - id: F395
    source: F265
    description: The bullet says the error names the paths, while the message names at most five of them in full and then a count of the rest.
    status: pending
    batch: M88
    commits: []
  - id: F396
    source: F264
    description: src/commands/sync/rename.rs's unit test the_rename_order_is_by_the_folded_key still sorts a vector the test itself builds and asserts the literal.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the same item as F391, already scheduled in M92"
  - id: F397
    source: F266
    description: Clap's refusal for the two-values-in-one-occurrence form does not say how to ask for two; a friendlier message would have to name the repeated form.
    status: pending
    batch: M94
    commits: []
  - id: F398
    source: F266
    description: The flags' value names render as the flag's own placeholder, so the help asks for a value named after the flag rather than an algorithm.
    status: pending
    batch: M94
    commits: []
  - id: F399
    source: F266
    description: Coverage carries the algorithms in a map keyed by the algorithm, so a remedy always names them in canonical order rather than the order the user wrote them; the design notes already record that trade-off.
    status: skipped
    batch: null
    commits: []
    note: "skipped: the trade-off is already recorded in the design notes"
  - id: F400
    source: F266
    description: The design-notes section headed Two things that are one now holds five bolded paragraphs, so the heading understates its contents; it was already loose before this batch and the new spelling-rule paragraph adds one more.
    status: pending
    batch: M95
    commits: []
  - id: F401
    source: F266
    description: The new design-notes sentence names three places that spell the flags, the CLI's own help, the README and the remedy a coverage error prints, while the crate spells them in several source doc comments as well, so the enumeration reads narrower than the rule it states.
    status: pending
    batch: M95
    commits: []
  - id: F402
    source: F266
    description: The refusal pin covers the any-of flag only, while the all-of flag's two-value form is refused by the same clap shape and nothing pins it, and the pin asserts only that the message names the second algorithm rather than that the error is clap's unknown-argument kind.
    status: pending
    batch: M96
    commits: []
  - id: F403
    source: F266
    description: The coverage remedy prints the request in canonical order rather than the order the user typed, so the remedy's any-of line does not inherit the user's tie-break preference, and the new test's comment reads as if it did.
    status: pending
    batch: M97
    commits: []
  - id: F404
    source: F266
    description: The why-table's row label now reads hash-any-of md5 --hash-any-of sha256, record holds only md5, which is awkward for a label that is not a command.
    status: pending
    batch: M96
    commits: []
  - id: F405
    source: F268
    description: DESIGN-NOTES.md still spells the repair as an update command with a placeholder folder, the same placeholder wording the README no longer uses.
    status: pending
    batch: M99
    commits: []
  - id: F406
    source: F267
    description: The synopsis lists the concurrency family of flags only on the sync line, though the family is on every subcommand and the README says so under Concurrency.
    status: pending
    batch: M98
    commits: []
  - id: F407
    source: F267
    description: The synopsis now shows the hash flags twice while the other repeatable flags are shown once, so the synopsis mixes two ways of writing a repeatable flag.
    status: pending
    batch: M98
    commits: []
  - id: F408
    source: F268
    description: The compare-self section still says three ways out, all named in the error, but under the default single-algorithm request the error names only the repopulation and the none request, the same false count the Gotchas bullet had.
    status: pending
    batch: M88
    commits: []
  - id: F409
    source: F270
    description: The directory-claim case checks its second name with a suffix check and never asks for a third, so a rule change to the later directory-claim spellings would pass it.
    status: pending
    batch: M100
    commits: []
  - id: F410
    source: F270
    description: The converter's inline case pins the first and second retry spellings but not the third, the same gap the cache's claim case had.
    status: pending
    batch: M100
    commits: []
  - id: F411
    source: F269
    description: Nothing pins the claim-attempt limit itself, so no test asserts that the family of names stops at the thousandth suffix or that a claim over an exhausted family fails.
    status: pending
    batch: M101
    commits: []
  - id: F412
    source: F269
    description: The claim-attempt limit is declared twice, in the cache and in the converter, with the same value and no shared constant, so the two families could drift apart.
    status: pending
    batch: M101
    commits: []
  - id: F413
    source: F269
    description: The case-fix placeholder is a third bounded claim family, a thousand candidates numbered from zero, and the README's step-3 description of it says nothing about the bound.
    status: pending
    batch: M102
    commits: []
  - id: F414
    source: F269
    description: The design notes' new paragraph frames the bounded family around the published name only, while the README also lists the working partial name, which is claimed from the same bounded family.
    status: pending
    batch: M102
    commits: []
  - id: F415
    source: F273
    description: The workspace nextest config's hook list covers only the hooks the runner sets, so a reader of that file has no pointer to the crate's other test-only switches, which are named only in the crate's README.
    status: pending
    batch: M103
    commits: []
  - id: F416
    source: F273
    description: The workspace nextest config's override for the hand-run bridge cases is inert under a plain test run, since its cases are ignored by design, so the filter reads as if it were live.
    status: pending
    batch: M103
    commits: []
  - id: F417
    source: F274
    description: The rename failure hook's base name is the odd one out, being the only failure hook whose name carries no suffix, so it reads as a peer of the copy and flush hooks while being the rename's before-claim stage.
    status: pending
    batch: M104
    commits: []
  - id: F418
    source: F273
    description: The workspace nextest config has no override entry for the other package's Unix-only skips under a Windows host.
    status: skipped
    batch: null
    commits: []
    note: "skipped: not a defect, the cases exist under their platform gate, and it was noted only as adjacent to the override check"
  - id: F419
    source: F274
    description: The property that the rename pass aborts at the first injected failure is documented nowhere, so the design notes' separately-armable argument reads stronger than the code delivers.
    status: pending
    batch: M105
    commits: []
  - id: F420
    source: F274
    description: The design notes' list of what each stage leaves behind is loose for the step-1 stage, since the placeholder is removed by the cleanup and a directory stage leaves no placeholder at all, where the README and the code doc word it precisely.
    status: pending
    batch: M105
    commits: []
  - id: F421
    source: F275
    description: The new commit policy type is visible only inside the cache module while the commit limits it carries are public, so a caller outside the crate cannot name the whole policy.
    status: pending
    batch: M106
    commits: []
  - id: F422
    source: F275
    description: The commit-limit environment variables are read for the one-off meta write too, though that write uses only the durability, so every cache open pays three environment reads it does not use.
    status: pending
    batch: M106
    commits: []
  - id: F423
    source: F275
    description: The durability type has no equality and is marked non-exhaustive, so comparing two durabilities needs a hand-written match; a small crate-local helper would be reusable if more of the code ever compares them.
    status: pending
    batch: M107
    commits: []
  - id: F424
    source: F275
    description: The counters for the other test-only switches, the copy, flush, rename and gate ones, are still per-feature and ad hoc, unlike the new per-thread durability counter.
    status: pending
    batch: M107
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
| F | T14 | completed | 2df937bf, fb913a2a, a2df3d11 | review F kept |
| F | T15 | completed | af640af6 | review F kept |
| F | T16 | completed | efb71184, 5b9a405a, 49830066, de233c05, 371a7413, 930efdf3 | review F fixed (930efdf3) |
| G | T17 | completed | 71fe4368, 4cc52111, efdacd32 | review G kept |
| G | T18 | completed | cce4fed3, efe471f8, 2a244f26 | review G kept |
| G | T19 | completed | eac1d900, 96e48eb9, 9c0b1c49 | review G kept |

## Transition log

Newest first. One line per committed state change: date, task/review, from → to,
commit.

- 2026-11-16 — follow-up batch M44 review checkpoint opened: F275 `in_progress` →
  `pending_review`, commits `819da84b`, `f6878407`; review M44 `pending` →
  `in_progress`, base `fb1fef37`, head `f6878407`.
- 2026-11-16 — follow-up batch M44 started: F275 `pending` → `in_progress`;
  `active_batch: M43` → `M44`.
- 2026-11-16 — review M43 `done`: F273 kept (c582a5b5), F274 kept (df01aaf7); no
  fix or revert commits; follow-ups F419 and F420 appended with batch `M105`,
  and F418 recorded as `skipped` as not a defect. F273 and F274 `pending_review`
  → `completed`.
- 2026-11-16 — follow-up batch M43 review checkpoint opened: F273 and F274
  `in_progress` → `pending_review`, commits `c582a5b5`, `df01aaf7`; review M43
  `pending` → `in_progress`, base `db97d9f0`, head `df01aaf7`.
- 2026-11-16 — follow-up batch M43 started: F273 and F274 `pending` →
  `in_progress`; `active_batch: M42` → `M43`.
- 2026-11-16 — review M42 `done`: F269 kept (0ac9ccee), F270 fixed (8a81858d,
  8966a091), F271 kept (afe44b2e); follow-ups F413 and F414 appended with batch
  `M102`. F269–F271 `pending_review` → `completed`.
- 2026-11-16 — follow-up batch M42 review checkpoint opened: F269, F270 and F271
  `in_progress` → `pending_review`, commits `0ac9ccee`, `8a81858d`, `afe44b2e`;
  review M42 `pending` → `in_progress`, base `0ab89862`, head `0ac9ccee`.
- 2026-11-16 — follow-up batch M42 started: F269, F270 and F271 `pending` →
  `in_progress`; `active_batch: M41` → `M42`.
- 2026-11-16 — review M41 `done`: F267 kept (a6850a91), F268 kept (1f950a9a); no fix
  or revert commits; follow-up F408 appended with batch `M88`. F267 and F268
  `pending_review` → `completed`.
- 2026-11-16 — follow-up batch M41 review checkpoint opened: F267 and F268
  `in_progress` → `pending_review`, commits `a6850a91`, `1f950a9a`; review M41
  `pending` → `in_progress`, base `d53d952c`, head `1f950a9a`.
- 2026-11-16 — follow-up batch M41 started: F267 and F268 `pending` →
  `in_progress`; `active_batch: M40` → `M41`.
- 2026-11-16 — review M40 `done`: F266 kept (a4740687, 4719ce4f, a17b7270); no fix
  or revert commits; follow-ups F400–F404 appended with batches `M95`, `M96` and
  `M97`, and F399 recorded as `skipped` because the trade-off is already
  recorded in the design notes. F266 `pending_review` → `completed`.
- 2026-11-16 — follow-up batch M40 review checkpoint opened: F266 `in_progress` →
  `pending_review`, commits `a4740687`, `4719ce4f`, `a17b7270`; review M40
  `pending` → `in_progress`, base `ee8a10a8`, head `a17b7270`.
- 2026-11-16 — follow-up batch M40 started: F266 `pending` → `in_progress`;
  `active_batch: M39` → `M40`.
- 2026-11-16 — review M39 `done`: F263 kept (ec4566db), F264 kept (1c363bbb),
  F265 fixed (28a1a18b, c6663a65); follow-ups F394 and F395 appended with
  batches `M93` and `M88`, and F396 recorded as `skipped` as the same item as
  F391. F263–F265 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M39 review checkpoint opened: F263, F264 and F265
  `in_progress` → `pending_review`, commits `ec4566db`, `1c363bbb`, `28a1a18b`;
  review M39 `pending` → `in_progress`, base `bb57d8fe`, head `28a1a18b`.
- 2026-11-15 — follow-up batch M39 started: F263, F264 and F265 `pending` →
  `in_progress`; `active_batch: M38` → `M39`.
- 2026-11-15 — review M38 `done`: F261 and F262 both kept, no fix or revert
  commits; follow-up F389 appended with batch `M90`, and F388 and F390 recorded
  as `skipped` as the same items as F265 and F263. F261 and F262
  `pending_review` → `completed`. The reviewer noted that the plan's own state
  commits name a batch, which the plan prescribes and which the crate's rule
  against naming a batch in committed text does not cover, since the state file
  lives in the plan directory and is deleted at the end.
- 2026-11-15 — follow-up batch M38 review checkpoint opened: F261 and F262
  `in_progress` → `pending_review`, commits `2ca98413`, `5f86beca`; review M38
  `pending` → `in_progress`, base `1abfc980`, head `5f86beca`.
- 2026-11-15 — follow-up batch M38 started: F261 and F262 `pending` →
  `in_progress`; `active_batch: M37` → `M38`.
- 2026-11-15 — review M37 `done`: F258, F259 and F260 all kept, no fix or revert
  commits; follow-up F385 appended with batch `M87`. F258–F260 `pending_review` →
  `completed`.
- 2026-11-15 — follow-up batch M37 review checkpoint opened: F258, F259 and F260
  `in_progress` → `pending_review`, commits `26f0ea58`, `53a51337`, `e81da0f2`;
  review M37 `pending` → `in_progress`, base `a91a091e`, head `e81da0f2`.
- 2026-11-15 — follow-up batch M37 started: F258, F259 and F260 `pending` →
  `in_progress`; `active_batch: M36` → `M37`.
- 2026-11-15 — review M36 `done`: F254, F255, F256 and F257 all kept, no fix or
  revert commits; follow-ups F379 and F380 appended with batch `M86`.
  F254–F257 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M36 review checkpoint opened: F254, F255, F256 and
  F257 `in_progress` → `pending_review`, commits `ee82d44c`, `07f0ef2a`,
  `4693e6ff`, `bac52732`, `0dc95b09`; review M36 `pending` → `in_progress`,
  base `8cbfed2b`, head `0dc95b09`.
- 2026-11-15 — follow-up batch M36 started: F254, F255, F256 and F257 `pending` →
  `in_progress`; `active_batch: M35` → `M36`.
- 2026-11-15 — the YAML block was repaired so it parses: 81 values with an
  unquoted colon were quoted, and the review M16 entry's follow-ups line that
  had swallowed the M4 batch header was split back into two lines. The block
  had never parsed, so the file's own claim that it is the source of truth did
  not hold.
- 2026-11-15 — review M35 `done`: F251 kept (274d90e5), F252 kept (c298eedc); no
  fix or revert commits; the script ran for real over all five members and the
  self-test passed; the reviewer confirmed the old rules returned an empty
  optional-feature list for a member with a feature named Default and no
  default, that the new rules return both, and that the named combination is
  one whose recorded exit code is zero. Follow-ups F370, F372 and F373 appended
  with batches `M83` and `M84`; F371 and F374 recorded as `skipped` as the
  same items as F368 and F367. F251 and F252 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M35 review checkpoint opened: F251 and F252
  `in_progress` → `pending_review`, commits `274d90e5`, `c298eedc`; review M35
  `pending` → `in_progress`, base `d3b4a295`, head `c298eedc`.
- 2026-11-15 — follow-up batch M35 started: F251 and F252 `pending` →
  `in_progress`; `active_batch: M34` → `M35`.
- 2026-11-15 — review M34 `done`: F247 kept, F248 fixed (3546a457, 2d19bef1),
  F249 kept; follow-ups F358–F364 appended with batches `M80`, `M81` and `M82`,
  and F365 and F366 recorded as `skipped` (moot as written, and documented
  where a reader will find it). F247–F249 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M34 review checkpoint opened: F247, F248 and F249
  `in_progress` → `pending_review`, commits `6572e643`, `3546a457`, `c11652be`,
  `3edc3d5e`; review M34 `pending` → `in_progress`, base `f8af1203`, head
  `3edc3d5e`.
- 2026-11-15 — follow-up batch M34 started: F247, F248 and F249 `pending` →
  `in_progress`; `active_batch: M33` → `M34`.
- 2026-11-15 — review M33 `done`: F243 kept (93ab7621), F244 fixed (ba8570a6,
  eefb96ca); follow-ups F353–F356 appended with batches `M78` and `M79`, and F357
  recorded as `skipped` because the exit-code and stdout assertions already
  carry the refusal. F243 and F244 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M33 review checkpoint opened: F243 and F244
  `in_progress` → `pending_review`, commits `93ab7621`, `ba8570a6`; review M33
  `pending` → `in_progress`, base `c888a545`, head `ba8570a6`. The batch's
  "in progress" state commit landed after the batch's own commits, because the
  implementing session held the tree while it worked.
- 2026-11-15 — follow-up batch M33 started: F243 and F244 `pending` →
  `in_progress`; `active_batch: M32` → `M33`.
- 2026-11-15 — review M32 `done`: F240, F241 and F242 all kept, no fix or revert
  commits; follow-ups F347–F352 appended with batches `M74`, `M75` and `M76`.
  F240–F242 `pending_review` → `completed`. The reviewer noted that the three
  commit bodies use " - " where the crate's prose uses an em dash; the tree
  keeps the em dash and later batches should use it in commit bodies too.
- 2026-11-15 — follow-up batch M32 review checkpoint opened: F240, F241 and F242
  `in_progress` → `pending_review`, commits `537ee293`, `1dd21971`, `8230e53e`;
  review M32 `pending` → `in_progress`, base `083c4739`, head `8230e53e`.
- 2026-11-15 — follow-up batch M32 started: F240, F241 and F242 `pending` →
  `in_progress`; `active_batch: M31` → `M32`.
- 2026-11-15 — review M31 `done`: F236 kept (7864335e); no fix or revert commits;
  follow-ups F345 and F346 appended with batch `M73`. The candidate follow-ups
  reported by the M29 and M30 implementing sessions were appended at the same
  time as F334–F337 (F341–F344), with batches `M71` and `M72`; F342 recorded as
  `skipped` as the same cost as F336. F236 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M31 review checkpoint opened: F236 `in_progress` →
  `pending_review`, commit `7864335e`; review M31 `pending` → `in_progress`,
  base `5dd484e6`, head `7864335e`.
- 2026-11-15 — follow-up batch M31 started: F236 `pending` → `in_progress`;
  `active_batch: M30` → `M31`.
- 2026-11-15 — review M30 `done`: F232 kept (3a52da69), F234 kept (b0a92f16); no
  fix or revert commits; follow-ups F338–F340 appended with batches `M69` and
  `M70`. F232 and F234 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M30 review checkpoint opened: F232 and F234
  `in_progress` → `pending_review`, commits `3a52da69`, `b0a92f16`; review M30
  `pending` → `in_progress`, base `9c8ee02d`, head `b0a92f16`.
- 2026-11-15 — follow-up batch M30 started: F232 and F234 `pending` →
  `in_progress`; `active_batch: M29` → `M30`.
- 2026-11-15 — review M29 `done`: F228 kept, F229 kept, F230 fixed (fbf844bb,
  4d0c9a7e), F231 fixed (02b0a441, 2b305551); follow-ups F329–F333 appended with
  batches `M65`, `M66` and `M67`. F228–F231 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M29 review checkpoint opened: F228, F229, F230 and
  F231 `in_progress` → `pending_review`, commits `8ed05795`, `75fb6c4b`,
  `fbf844bb`, `02b0a441`; review M29 `pending` → `in_progress`, base `058c2c24`,
  head `02b0a441`.
- 2026-11-15 — follow-up batch M29 started: F228, F229, F230 and F231 `pending` →
  `in_progress`; `active_batch: M28` → `M29`.
- 2026-11-15 — review M28 `done`: F225 kept with no commit, the reviewer judged
  the field doc true for all three handle kinds; F227 kept (f368ae29); follow-up
  F328 appended with batch `M64`. F225 and F227 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M28 started: F225 and F227 `pending` →
  `in_progress`; `active_batch: M27` → `M28`.
- 2026-11-15 — review M27 `done`: F222 kept, F223 kept, F224 fixed (32ef16cf,
  7e717ec5); follow-ups F318–F323 appended with batches `M61` and `M62`.
  F222–F224 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M27 review checkpoint opened: F222, F223 and F224
  `in_progress` → `pending_review`, commits `b8ba41ec`, `6643ccf7`, `32ef16cf`;
  review M27 `pending` → `in_progress`, base `36f93bdd`, head `32ef16cf`.
- 2026-11-15 — follow-up batch M27 started: F222, F223 and F224 `pending` →
  `in_progress`; `active_batch: M26` → `M27`.
- 2026-11-15 — review M26 `done`: F218 kept, F219 fixed (f26a1837, aa527d20,
  95cc2721), F220 kept, F221 kept; follow-ups F315–F317 appended with batch
  `M60`. F218–F221 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M26 review checkpoint opened: F218, F219, F220 and
  F221 `in_progress` → `pending_review`, commits `f26a1837`, `82854f75`; review
  M26 `pending` → `in_progress`, base `cb6bb33c`, head `82854f75`.
- 2026-11-15 — follow-up batch M26 started: F218, F219, F220 and F221 `pending` →
  `in_progress`; `active_batch: M25` → `M26`.
- 2026-11-15 — review M25 `done`: F216 kept (d42a90fd), F217 fixed (0339f805,
  90d32f86); follow-ups F310–F313 appended with batches `M58` and `M59`, and
  F314 recorded as `skipped` because the warnings are in the other crates of the
  workspace. F216 and F217 `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M25 review checkpoint opened: F216 and F217
  `in_progress` → `pending_review`, commits `d42a90fd`, `0339f805`; review M25
  `pending` → `in_progress`, base `9a395087`, head `0339f805`.
- 2026-11-15 — follow-up batch M25 started: F216 and F217 `pending` →
  `in_progress`; `active_batch: M24` → `M25`.
- 2026-11-15 — review M24 `done`: F212, F213, F214 and F215 all kept, no fix or
  revert commits; follow-ups F307–F309 appended with batch `M57`. F212–F215
  `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M24 review checkpoint opened: F212, F213, F214 and
  F215 `in_progress` → `pending_review`, commits `6c8cb53e`, `645829a9`,
  `13120f7a`; review M24 `pending` → `in_progress`, base `00894206`, head
  `13120f7a`.
- 2026-11-15 — follow-up batch M24 started: F212, F213, F214 and F215 `pending` →
  `in_progress`; `active_batch: M23` → `M24`.
- 2026-11-15 — review M23 `done`: F209, F210 and F211 all kept, no fix or
  revert commits; follow-up F304 appended with batch `M56`, F305 recorded as
  `skipped` as the same item as F266, and F306 recorded as `skipped` as the same
  item as F213, whose count was updated to four rows. F209, F210 and F211
  `pending_review` → `completed`.
- 2026-11-15 — follow-up batch M23 review checkpoint opened: F209, F210 and F211
  `in_progress` → `pending_review`, commits `aaed2ebd`, `e03f21ab`, `986fa9a0`;
  review M23 `pending` → `in_progress`, base `983e70bf`, head `986fa9a0`.
- 2026-11-14 — M4 retry: F111, F112, F116 and F130 `pending` → `in_progress`.
- 2026-11-14 — follow-up batch M23 started: F209, F210 and F211 `pending` →
  `in_progress`; `active_batch: M22` → `M23`.
- 2026-11-14 — review M22 `done`: F206, F207 and F208 all kept, no fix or
  revert commits; follow-ups F298–F303 appended with batches `M53`, `M54` and
  `M55`. F206–F208 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M22 review checkpoint opened: F206, F207 and F208
  `in_progress` → `pending_review`, commits `e5171086`, `b683af1f`,
  `0a942361`; review M22 `pending` → `in_progress`, base `7c9a179d`, head
  `0a942361`. Follow-ups F294–F297 appended with batches `M51` and `M52`.
- 2026-11-14 — follow-up batch M22 started: F206, F207 and F208 `pending` →
  `in_progress`; `active_batch: M21` → `M22`.
- 2026-11-14 — review M21 `done`: F200, F201, F202, F203 and F204 all kept,
  no fix or revert commits; follow-ups F291 and F292 appended with batches
  `M49` and `M50`, and F293 recorded as `skipped` as the same item as F288.
  F200–F204 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M21 review checkpoint opened: F200, F201, F202,
  F203 and F204 `in_progress` → `pending_review`, commits `7e40bfb6`,
  `b8b98771`, `c64275cf`, `cb20bfea`, `6b15098a`; review M21 `pending` →
  `in_progress`, base `ead8e3bd`, head `6b15098a`. Follow-ups F287–F289
  appended with batches `M49` and `M50`; F290 recorded as `skipped` as the
  same item as F206.
- 2026-11-14 — follow-up batch M21 started: F200, F201, F202, F203 and F204
  `pending` → `in_progress`; `active_batch: M20` → `M21`.
- 2026-11-14 — review M20 `done`: F195, F196, F197 and F199 all kept, no fix
  or revert commits; follow-ups F282–F285 appended with batches `M47` and
  `M48`, and F286 recorded as `skipped` as the same item as F278. F195–F199
  `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M20 review checkpoint opened: F195, F196, F197
  and F199 `in_progress` → `pending_review`, commits `14f77fa3`, `30b37539`,
  `fa35b7d8`, `858fa92e`; review M20 `pending` → `in_progress`, base
  `fe3db912`, head `858fa92e`. Follow-ups F278–F281 appended with batches
  `M46` and `M47`.
- 2026-11-14 — follow-up batch M20 started: F195, F196, F197 and F199 `pending` →
  `in_progress`; `active_batch: M19` → `M20`.
- 2026-11-14 — review M19 `done`: F193 fixed (7e12b998), F194 fixed
  (0f243257), F198 kept (a3bff56e); follow-ups F276 and F277 appended with
  batch `M45`. F193, F194 and F198 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M19 review checkpoint opened: F193, F194 and F198
  `in_progress` → `pending_review`, commits `1b0c1f92`, `140b2db4`,
  `a3bff56e`; review M19 `pending` → `in_progress`, base `7831eb0c`, head
  `a3bff56e`. Follow-ups F273–F275 appended with batches `M43` and `M44`.
- 2026-11-14 — follow-up batch M19 started: F193, F194 and F198 `pending` →
  `in_progress`; `active_batch: M18` → `M19`.
- 2026-11-14 — review M18 `done`: F189 kept (f4b00295), F192 fixed
  (b7f6afda); follow-ups F269–F271 appended with batch `M42` and F272
  recorded as `skipped` because the false claim sits in a commit message that
  must not be rewritten. F189 and F192 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M18 review checkpoint opened: F189 and F192
  `in_progress` → `pending_review`, commits `f4b00295`, `a85579d2`; review M18
  `pending` → `in_progress`, base `8308d0b6`, head `a85579d2`. Follow-ups
  F266–F268 appended with batches `M40` and `M41`.
- 2026-11-14 — follow-up batch M18 started: F189 and F192 `pending` →
  `in_progress`; `active_batch: M17` → `M18`.
- 2026-11-14 — review M17 `done`: F185 kept (f0cd71f7), F186 kept (a2bb2324),
  F187 kept (4c48b5cb), F188 kept (4d91eca7), F190 fixed (6c23cb4b), F191 kept
  (4d91eca7); follow-ups F263–F265 appended with batch `M39`. F185–F191
  `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M17 review checkpoint opened: F185, F186, F187,
  F188, F190 and F191 `in_progress` → `pending_review`, commits `f0cd71f7`,
  `a2bb2324`, `4d91eca7`, `4c48b5cb`, `94a17885`; review M17 `pending` →
  `in_progress`, base `5c2e29fc`, head `94a17885`. Follow-ups F258–F262
  appended with batches M37 and M38.
- 2026-11-14 — follow-up batch M17 started: F185, F186, F187, F188, F190 and F191
  `pending` → `in_progress`; `active_batch: M16` → `M17`.
- 2026-11-14 — review M16 `done`: F179, F180 and F181 all kept, no fix or
  revert commits; follow-ups F254–F257 appended with batch `M36`. F179–F181
  `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M16 review checkpoint opened: F179, F180 and F181
  `in_progress` → `pending_review`, commits `c0348f54`, `673b99b1`; review M16
  `pending` → `in_progress`, base `23a5974d`, head `673b99b1`. Follow-ups
  F251 and F252 appended with batch `M35`; F253 recorded as `skipped` as the
  same item as F248.
- 2026-11-14 — follow-up batch M16 started: F179, F180 and F181 `pending` →
  `in_progress`; `active_batch: M15` → `M16`.
- 2026-11-14 — review M15 `done`: F175 kept (b1610c1e), F176 fixed (e86a40aa),
  F177 kept (493fce5b), F178 kept (b0081392); follow-ups F247–F249 appended
  with batch `M34` and F250 recorded as `skipped` as the same item as F243.
  F175–F183 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M15 review checkpoint opened: F175, F176, F177,
  F178 and F183 `in_progress` → `pending_review`, commits `b1610c1e`,
  `45c4649e`, `493fce5b`, `b0081392`; review M15 `pending` → `in_progress`,
  base `49e768cc`, head `b0081392`. Follow-ups F243 and F244 appended with
  batch `M33`; F245 and F246 recorded as `skipped` as the same items as F179
  and F180.
- 2026-11-14 — follow-up batch M15 started: F175, F176, F177, F178 and F183
  `pending` → `in_progress`; `active_batch: M14` → `M15`.
- 2026-11-14 — review M14 `done`: F172 kept (0f9e7d33, a747499a), F173 fixed
  (6dd4294d), F174 kept (3a65ca7f, 41656cb5, 2d6014e6); follow-ups F240–F242
  appended with batch `M32`. F172–F174 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M14 review checkpoint opened: F172, F173 and F174
  `in_progress` → `pending_review`, commits `0f9e7d33`, `a747499a`,
  `0fe48358`, `3a65ca7f`, `41656cb5`, `2d6014e6`; review M14 `pending` →
  `in_progress`, base `78244080`, head `2d6014e6`. Follow-up F236 appended
  with batch `M31`; F237, F238 and F239 recorded as `skipped` as the same
  items as F228, F234 and F230.
- 2026-11-14 — follow-up batch M14 started: F172, F173 and F174 `pending` →
  `in_progress`; `active_batch: M13` → `M14`.
- 2026-11-14 — review M13 `done`: F166 fixed (82fd8a95), F167 kept, F168 kept,
  F169 kept, F170 kept, F171 fixed (6236f9fa); follow-ups F232 and F234
  appended with batch `M30`, and F233 and F235 recorded as `skipped` as the
  same items as F228 and F229. F166–F171 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M13 review checkpoint opened: F166, F167, F168,
  F169, F170 and F171 `in_progress` → `pending_review`, commits `515bfa49`,
  `29b218f8`, `95a2d302`, `4db5ab6a`, `6955810c`, `ae982636`, `a095f412`,
  `af1faa25`, `34293501`; review M13 `pending` → `in_progress`, base
  `1fe6901c`, head `34293501`. Follow-ups F228–F231 appended with batch `M29`.
- 2026-11-14 — follow-up batch M13 started: F166, F167, F168, F169, F170 and F171
  `pending` → `in_progress`; `active_batch: M12` → `M13`.
- 2026-11-14 — review M12 `done`: F160 fixed (80ba72d1), F161–F165 kept; follow-
  ups F225 and F227 appended with batch `M28` and F226 recorded as `skipped`
  because it is the same item as F222. F160–F165 `pending_review` →
  `completed`.
- 2026-11-14 — follow-up batch M12 review checkpoint opened: F160, F161, F162,
  F163, F164 and F165 `in_progress` → `pending_review`, commits `0e48eda4`,
  `312f8852`, `71434e08`, `7d01f95b`, `1ff3d1ac`, `ce0cab8a`; review M12
  `pending` → `in_progress`, base `2f983689`, head `ce0cab8a`. Follow-ups
  F222–F224 appended with batch `M27`.
- 2026-11-14 — follow-up batch M12 started: F160, F161, F162, F163, F164 and F165
  `pending` → `in_progress`; `active_batch: M11` → `M12`.
- 2026-11-14 — review M11 `done`: F156, F157, F158 and F159 all kept, no fix
  or revert commits; follow-ups F218–F221 appended with batch `M26`. F156,
  F157, F158 and F159 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M11 review checkpoint opened: F156, F157, F158 and
  F159 `in_progress` → `pending_review`, commits `6fc49e88`, `6603ecaf`,
  `dbc64313`, `880b1319`, `2ce5c4cc`; review M11 `pending` → `in_progress`,
  base `8cd72166`, head `2ce5c4cc`. Follow-ups F216 and F217 appended with
  batch `M25`.
- 2026-11-14 — follow-up batch M11 started: F156, F157, F158 and F159 `pending` →
  `in_progress`; `active_batch: M9` → `M11`.
- 2026-11-14 — review M9 `done`: F147–F154 all kept, no fix or revert commits;
  follow-ups F212–F215 appended with batch `M24`. F147–F154 `pending_review` →
  `completed`.
- 2026-11-14 — follow-up batch M9 review checkpoint opened: F147, F148, F149,
  F150, F151, F152, F153 and F154 `in_progress` → `pending_review`, commits
  `91d626b5`, `1f493787`, `dfaecf32`, `8bbba952`, `ccf14a43`, `4a2acaea`;
  review M9 `pending` → `in_progress`, base `c2295439`, head `4a2acaea`.
  Follow-ups F209–F211 appended with batch `M23`.
- 2026-11-14 — follow-up batch M9 started: F147, F148, F149, F150, F151, F152,
  F153 and F154 `pending` → `in_progress`; `active_batch: M7` → `M9`.
- 2026-11-14 — review M7 `done`: F138 kept (8e5f5bd0), F139 fixed
  (10ec5297), F141 kept (0d360416), F144 kept (5777bf8c); follow-ups
  F206–F208 appended with batch `M22` and F205 recorded as `skipped` because
  it is the same item as F202. F138, F139, F141 and F144 `pending_review` →
  `completed`.
- 2026-11-14 — follow-up batch M7 review checkpoint opened: F138, F139, F141
  and F144 `in_progress` → `pending_review`, commits `8e5f5bd0`, `4dd86821`,
  `0d360416`, `5777bf8c`, `1148ad43`; review M7 `pending` → `in_progress`,
  base `0013d040`, head `1148ad43`. Follow-ups F200–F204 appended with batch
  `M21`.
- 2026-11-14 — follow-up batch M7 started: F138, F139, F141 and F144 `pending` →
  `in_progress`; `active_batch: M6` → `M7`.
- 2026-11-14 — review M6 `done`: F121, F122, F125, F126, F127 and F129 all kept,
  no fix or revert commits; follow-ups F198 and F199 appended to M19 and M20.
  F121, F122, F125, F126, F127 and F129 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M6 review checkpoint opened: F121, F122, F125,
  F126, F127 and F129 `in_progress` → `pending_review`, commits `50e81f06`,
  `a5b3b2c1`, `2115d14e`, `c4f93a7f`, `6c16cf00`, `20b42517`, `5a5cb7a2`,
  `9c327065`, `5b6fdf3b`; review M6 `pending` → `in_progress`, base
  `e68043dd`, head `5b6fdf3b`. Follow-ups F193–F197 appended with batches
  M19 and M20.
- 2026-11-14 — follow-up batch M6 started: F121, F122, F125, F126, F127 and F129
  `pending` → `in_progress`; `active_batch: M5` → `M6`.
- 2026-11-14 — review M5 `done`: F117, F118, F120, F140, F145 and F146 all
  kept, no fix or revert commits; follow-ups F185–F192 appended, with F185,
  F186, F187, F188, F190 and F191 in M17 and F189 and F192 in M18. F117,
  F118, F120, F140, F145 and F146 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M5 review checkpoint opened: F117, F118, F120,
  F140, F145 and F146 `in_progress` → `pending_review`, commits `e30e6c6f`,
  `13304c2f`, `1409bbf6`, `0f3609a7`, `4c285d30`, `b08451c7`; review M5
  `pending` → `in_progress`, base `51806c37`, head `b08451c7`. Follow-ups
  F185–F188 appended with batch `M17`.
- 2026-11-14 — follow-up batch M5 started: F117, F118, F120, F140, F145 and F146
  `pending` → `in_progress`; `active_batch: M4` → `M5`.
- 2026-11-14 — review M4 `done`: F111, F112, F116 and F130 all kept, no fix
  or revert commits; follow-ups F179–F184 appended, with F179, F180 and F181
  in M16, F183 in M15, and F182 and F184 recorded as `skipped` (F182 because
  the converter cases are feature-gated by design, F184 as the same item as
  F178). M10 and M8 were merged into M9 and M5 respectively. F111, F112,
  F116 and F130 `pending_review` → `completed`.
- 2026-11-14 — follow-up batch M4 review checkpoint opened: F111, F112, F116
  and F130 `in_progress` → `pending_review`, commits `839bcf84`, `1cac4f4b`,
  `0e73b57f`, `335cda4b`; review M4 `pending` → `in_progress`, base
  `25c832ec`, head `335cda4b`. Follow-ups F175–F178 appended with batch
  `M15`.
- 2026-11-14 — the first M4 session died before doing anything, with a clean
  tree and no commits, so F111, F112, F116 and F130 were reset
  `in_progress` → `pending` for a retry.
- 2026-11-14 — follow-up batch M4 started: F111, F112, F116 and F130 `pending` →
  `in_progress`; `active_batch: M3` → `M4`.
- 2026-11-13 — review M3 `done`: F33 fixed (2bd29e74), F34 kept (0d7f7646),
  F35 kept (4fb0741f), F41 fixed (929908a7); follow-ups F172–F174 appended
  with batch `M14`. F33, F34, F35 and F41 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch M3 review checkpoint opened: F33, F34, F35 and
  F41 `in_progress` → `pending_review`, commits `643ebcb2`, `0d7f7646`,
  `4fb0741f`, `822254ab`, `dfa16d0b`, `1f66fcc7`; review M3 `pending` →
  `in_progress`, base `a5bc410d`, head `1f66fcc7`. Follow-ups F166–F171
  appended with batch `M13`.
- 2026-11-13 — follow-up batch M3 started: F33, F34, F35 and F41 `pending` →
  `in_progress`; `active_batch: M2` → `M3`.
- 2026-11-13 — review M2 `done`: F36, F37, F38, F39 and F40 all kept, no fix
  or revert commits; follow-ups F160–F165 appended with batch `M12`. F36,
  F37, F38, F39 and F40 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch M2 review checkpoint opened: F36, F37, F38, F39
  and F40 `in_progress` → `pending_review`, commits `fb7fdc23`, `773d1c53`,
  `f398aa2a`, `779de219`, `0d49c077`; review M2 `pending` → `in_progress`,
  base `efdf4617`, head `0d49c077`. Follow-ups F156–F159 appended with batch
  `M11`.
- 2026-11-13 — follow-up batch M2 started: F36, F37, F38, F39 and F40 `pending`
  → `in_progress`; `active_batch: M1` → `M2`.
- 2026-11-13 — review M1 `done`: F6, F28, F29, F32 and F124 all kept, no fix
  or revert commits; follow-ups F152–F154 appended with batch `M10` and F155
  recorded as `skipped` because it is the same item as F151. F6, F28, F29,
  F32 and F124 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch M1 review checkpoint opened: F6, F28, F29, F32
  and F124 `in_progress` → `pending_review`, commits `27e6f74f`, `f841e1e8`,
  `e7264fba`, `601d7efe`, `746f94cb`; review M1 `pending` → `in_progress`,
  base `d597f80d`, head `f841e1e8`. Follow-ups F147–F151 appended with batch
  `M9`, the second documentation-accuracy batch.
- 2026-11-13 — follow-up batch M1 started: F6, F28, F29, F32 and F124 `pending`
  → `in_progress`; `active_batch: Z4` → `M1`.
- 2026-11-13 — state repair: 15 follow-ups (F19, F20, F21, F22, F25, F26, F27,
  F31, F44, F45, F46, F47, F48, F51, F64) were left at `pending_review` with
  no note, because the scripts that applied those review results only
  rewrote an existing note line and never set the status, and their entries
  had no note line to rewrite. All 15 are now `completed` with the review
  note recorded and their commit lists already correct. No follow-up is left
  at `pending_review`, and the 134 recorded follow-ups stand at 70 completed,
  34 pending and 30 skipped.
- 2026-11-13 — review Z4 `done`: F132, F133, F134 and F136 all kept, no fix or
  revert commits; follow-ups F144–F146 appended, and F143 recorded as
  `skipped` because it is the same item as F135. F132, F133, F134 and F136
  `pending_review` → `completed`.
- 2026-11-13 — the remaining small follow-up batches Z1–Z3, Z5, X4, X5 and
  X6 were consolidated into eight batches M1–M8 by subsystem, since each held
  only a few small items and each batch costs an implementing session and a
  review session. M1 documentation accuracy, M2 the doc wordings in the plan
  guards and the cache db, M3 the digest matching tests and the phase-A
  index, M4 the converter CLI and CI, M5 the stale-tool-state documents, M6
  more test pinning, M7 the case-fold wording and the third fold site, M8 the
  coverage message and the side labels.
- 2026-11-13 — follow-up batch Z4 review checkpoint opened: F132, F133, F134
  and F136 `in_progress` → `pending_review`, commits `0fa1a663`, `c9a512e8`,
  `72d9e889`, `339c0957`; review Z4 `pending` → `in_progress`, base
  `e9914420`, head `339c0957`.
- 2026-11-13 — follow-up batch Z4 started: F132, F133, F134 and F136 `pending`
  → `in_progress`; `active_batch: X3` → `Z4`.
- 2026-11-13 — review X3 `done`: F22, F27, F47 and F48 all kept, no fix or
  revert commits; follow-up F136 added to Z4 and F137 recorded as `skipped`
  because it mirrors the existing girpr_log re-export precedent. F22, F27,
  F47 and F48 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch X3 review checkpoint opened: F22, F27, F47 and
  F48 `in_progress` → `pending_review`, commits `7279aee5`, `9ea19b36`,
  `cb9c08a3`, `d774d7ce`; review X3 `pending` → `in_progress`, base
  `90c57c19`, head `d774d7ce`.
- 2026-11-13 — follow-up batch X3 started: F22, F27, F47 and F48 `pending` →
  `in_progress`; `active_batch: X2` → `X3`.
- 2026-11-13 — review X2 `done`: F19, F20, F21, F44, F45 and F46 all kept, no
  fix or revert commits; follow-ups F125–F131 appended, with F125, F126,
  F127 and F129 in Z3, F130 in Z2, and F128 and F131 recorded as `skipped`
  with reasons. F19, F20, F21, F44, F45 and F46 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch X2 review checkpoint opened: F19, F20, F21,
  F44, F45 and F46 `in_progress` → `pending_review`, commits `927df439`,
  `0958cb81`, `6eb52866`, `eaf252c5`, `552e07d8`, `4e90c67e`, `4c279930`;
  review X2 `pending` → `in_progress`, base `272effd3`, head `4e90c67e`.
- 2026-11-13 — follow-up batch X2 started: F19, F20, F21, F44, F45 and F46
  `pending` → `in_progress`; `active_batch: X1` → `X2`.
- 2026-11-13 — review X1 `done`: F25, F26, F31, F51 and F64 all kept, no fix
  or revert commits; follow-ups F116–F120 appended, with F116 in Z1, F117,
  F118 and F120 in a new batch Z2 for the stale-tool-state wording, and F119
  recorded as `skipped` because it is the same defect as F111. F25, F26, F31,
  F51 and F64 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch X1 review checkpoint opened: F25, F26, F31,
  F51 and F64 `in_progress` → `pending_review`, commits `d342a72f`,
  `1f0b8edf`, `0fdbf2c9`, `218be752`, `aca6f3ba`; review X1 `pending` →
  `in_progress`, base `7e3e1a02`, head `aca6f3ba`.
- 2026-11-13 — follow-up batch X1 started: F25, F26, F31, F51 and F64 `pending`
  → `in_progress`; `active_batch: Y2` → `X1`.
- 2026-11-13 — review Y2 `done`: F101 fixed (f0033492), F102 kept (33faafce),
  F103 fixed (5a4bec0e); follow-ups F108–F110 appended and recorded as
  `skipped` with reasons, since the backup publish chain ends here. F101, F102
  and F103 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch Y2 review checkpoint opened: F101, F102 and
  F103 `in_progress` → `pending_review`, commits `d52fdd47`, `33faafce`,
  `39706eae`, `3dee6745`; review Y2 `pending` → `in_progress`, base
  `13d0adcd`, head `3dee6745`.
- 2026-11-13 — follow-up batch Y2 started: F101, F102 and F103 `pending` →
  `in_progress`; `active_batch: Y1` → `Y2`.
- 2026-11-13 — review Y1 `done`: F90, F91, F94 and F95 kept; F96 fixed
  (0531886c) and F97 fixed (0d287806); follow-ups F101–F103 appended with batch
  `Y2`. Those three are documentation corrections to sentences that state
  something false or read backwards, so they get one final batch rather than
  being skipped; the state records that Y2 ends the backup publish chain and
  that anything further of the same kind is to be skipped with the reason.
  F90, F91, F94, F95, F96 and F97 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch Y1 review checkpoint opened: F90, F91, F94,
  F95, F96 and F97 `in_progress` → `pending_review`, commits `58a86fcd`,
  `4a9178e7`, `5ee6d57f`, `8c13c181`, `6bd5912c`, `71912478`; review Y1
  `pending` → `in_progress`, base `ef56b02c`, head `71912478`.
- 2026-11-13 — follow-up batch Y1 started: F90, F91, F94, F95, F96 and F97
  `pending` → `in_progress`; `active_batch: X7` → `Y1`.
- 2026-11-13 — review X7 `done`: F87 fixed (c50748d3), F88 kept (7772be18),
  F89 kept (df7e2c36); follow-ups F94–F97 appended and merged into Y1, which
  already held F90 and F91, because all six are the backup publish fallback
  windows and the last two doc wordings about them. The state records that Y1
  closes the backup publish chain: any further follow-up of the same kind is
  to be recorded as `skipped` with the reason. F87, F88 and F89
  `pending_review` → `completed`.
- 2026-11-13 — follow-up batch X7 review checkpoint opened: F87, F88 and F89
  `in_progress` → `pending_review`, commits `6da18401`, `7772be18`,
  `df7e2c36`, `91533514`; review X7 `pending` → `in_progress`, base
  `757e2bf0`, head `91533514`.
- 2026-11-13 — follow-up batch X7 started: F87, F88 and F89 `pending` →
  `in_progress`; `active_batch: B24` → `X7`.
- 2026-11-13 — review B24 `done`: F79 kept (fe0e4c95), F81 kept (ca6ad054,
  7a04252e), F82 fixed (3341ec24); follow-ups F87–F89 appended with batch
  `X7`. F79, F81 and F82 `pending_review` → `completed`.
- 2026-11-13 — the remaining small follow-up batches were consolidated into
  seven batches X1–X7 by subsystem, since each of B8–B19 and B25 held only
  one to five small items and each batch costs an implementing session and a
  review session. X1 converter CLI and stale tool state, X2 test-only items
  and the copy verify naming, X3 the command layer wording and the crate
  root re-export, X4 documentation accuracy and the cache db wordings, X5 the
  plan guards' doc wordings, X6 the digest matching tests and the phase-A
  alternate-key index, X7 the backup bound details. The old rows B8–B19 were
  replaced and B15, already merged into B14, was dropped.
- 2026-11-13 — follow-up batch B24 review checkpoint opened: F79, F81 and F82
  `in_progress` → `pending_review`, commits `fe0e4c95`, `0aeaa4a7`,
  `ca6ad054`, `7a04252e`; review B24 `pending` → `in_progress`, base
  `6ea86b57`, head `7a04252e`.
- 2026-11-13 — follow-up batch B24 started: F79, F81 and F82 `pending` →
  `in_progress`; `active_batch: B23` → `B24`. B15 was merged into B14 (five
  doc wordings in one batch) and recorded as such.
- 2026-11-13 — review B23 `done`: F65, F66, F68 and F75 kept; F73 and F74
  fixed (f07a728b, 04dab01f); follow-ups F79, F81 and F82 appended with batch
  `B24`, and F80 recorded as `skipped` because it cannot be checked on this
  Windows host and its outcome is a bounded degradation rather than a
  weakening. B24 closes the backup-publish chain. F65, F66, F68, F73, F74 and
  F75 `pending_review` → `completed`.
- 2026-11-13 — follow-up batch B23 review checkpoint opened: F65, F66, F68,
  F73, F74 and F75 `in_progress` → `pending_review`, commits `046b53d3`,
  `d8ea86be`, `4d764527`, `777fe7b7`, `cb1f80f6`, `08959674`, `c4ba99aa`,
  `a8856942`; review B23 `pending` → `in_progress`, base `c010f012`, head
  `a8856942`.
- 2026-11-13 — follow-up batch B23 started: F65, F66, F68, F73, F74 and F75
  `pending` → `in_progress`; `active_batch: B22` → `B23`.
- 2026-11-13 — review B22 `done`: F56, F58, F60 and F61 kept, no fix or
  revert commits; follow-ups F73–F75 appended and merged into B23, which
  already held F65, F66 and F68, because all six are small items in the same
  cache backup subsystem. F56, F58, F60 and F61 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B22 review checkpoint opened: F56, F58, F60 and
  F61 `in_progress` → `pending_review`, commits `a39d5550`, `62f7f950`,
  `c9d86e57`, `3b9a60e3`, `a1a0a602`, `fb7dd522`; review B22 `pending` →
  `in_progress`, base `0dba9367`, head `fb7dd522`.
- 2026-11-13 — follow-up batch B22 started: F56, F58, F60 and F61 `pending` →
  `in_progress`; `active_batch: B21` → `B22`.
- 2026-11-13 — review B21 `done`: F52, F54, F55 and F57 kept, plus two
  comment-only fixes (086c4d2f, 6a7e78a5); follow-ups F65, F66 and F68
  appended with batch `B23`, F64 added to B19, and F67 and F69 recorded as
  `skipped` with reasons (a directory cache has nothing to lock and the
  records already protect it; F69 is the same kind as F57 and the copy it
  spoke for is reclaimed). F52, F54, F55 and F57 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B21 review checkpoint opened: F52, F54, F55 and
  F57 `in_progress` → `pending_review`, commits `9eebcc83`, `86467207`,
  `85d1a2d1`, `ca4d5ae4`, `3eeeb395`, `77952c07`, `980e7a3d`, `776c3d1e`;
  review B21 `pending` → `in_progress`, base `a32632aa`, head `776c3d1e`.
- 2026-11-13 — follow-up batch B21 started: F52, F54, F55 and F57 `pending` →
  `in_progress`; `active_batch: B20` → `B21`.
- 2026-11-13 — review B20 `done`: F53 fixed (8505c72a, fc06393f, 420485dd,
  4a42cf81); follow-ups F57, F58, F60 and F61 appended, F59 recorded as
  `skipped` because it is the same defect as F56, which is already scheduled.
  B21 and B22 are scheduled next, and the state records that the concurrency
  chain ends there: any further follow-up of the same kind that B21 or B22
  produces is to be recorded as `skipped` with the reason. F53
  `pending_review` → `completed`.
- 2026-11-13 — follow-up batch B20 review checkpoint opened: F53
  `in_progress` → `pending_review`, commits `cd3942ee`, `29ea8fab`,
  `214cc1ae`, `0ce5c0f3`; review B20 `pending` → `in_progress`, base
  `80b419d6`, head `0ce5c0f3`.
- 2026-11-13 — follow-up batch B20 started: F53 `pending` → `in_progress`;
  `active_batch: B12` → `B20`.
- 2026-11-13 — review B12 `done`: F17 fixed (5b89a4fb, 4f3b7e10), F49 kept;
  follow-ups F52–F56 appended. F53 is the same kind of follow-up as F17 and
  gets its own batch `B20` with a design up front, scheduled before the
  smaller `B21` items that touch the same mechanism. If B20 produces yet
  another follow-up of the same kind, it is recorded as `skipped` with the
  reason, per the no-endless-loop rule; F17 and F49 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B12 review checkpoint opened: F17 and F49
  `in_progress` → `pending_review`, commits `55d0b59a`, `dd816c83`,
  `63ff19bf`; review B12 `pending` → `in_progress`, base `186d22a2`, head
  `63ff19bf`.
- 2026-11-13 — follow-up batch B12 started: F17 and F49 `pending` →
  `in_progress`; `active_batch: B7` → `B12`. This is the batch the evaluator
  asked to run with a concurrency design up front.
- 2026-11-13 — review B7 `done`: F16 kept with a fix (be9aed2e), F30 kept;
  follow-ups F49–F51 appended, with F49 joining B12 because it widens the
  retention race that F17 exists to close, F50 recorded as completed by the
  review's own fix commit, and F51 given batch `B19`; F16 and F30
  `pending_review` → `completed`.
- 2026-11-13 — state repair: the commits and note fields for follow-ups F3
  through F30 had been left empty by a scripting bug that cleared the current
  id too early. They are now filled in from the recorded review results; the
  statuses, batches and review rows were unaffected.
- 2026-11-13 — follow-up batch B7 review checkpoint opened: F16 and F30
  `in_progress` → `pending_review`, commits `0a66e0cf`, `58613c52`,
  `e8335f4d`, `34015725`, `f612e0d4`, `4d1a8820`, `fc5956b6`; review B7
  `pending` → `in_progress`, base `7dd8436d`, head `fc5956b6`.
- 2026-11-13 — follow-up batch B7 started: F16 and F30 `pending` →
  `in_progress`; `active_batch: B6` → `B7`.
- 2026-11-13 — review B6 `done`: F15 and F23 kept, no fix or revert commits;
  follow-ups F47–F48 appended with batch `B18`; F15 and F23 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B6 review checkpoint opened: F15 and F23
  `in_progress` → `pending_review`, commits `e031b0e4`, `b0342361`; review B6
  `pending` → `in_progress`, base `e6ab7477`, head `b0342361`.
- 2026-11-13 — follow-up batch B6 started: F15 and F23 `pending` →
  `in_progress`; `active_batch: B5` → `B6`.
- 2026-11-13 — review B5 `done`: F14 and F24 kept, no fix or revert commits;
  follow-ups F44–F46 appended with batch `B17` (the copy verify's naming order:
  a test, a README line, a trailing newline); F14 and F24 `pending_review` →
  `completed`.
- 2026-11-13 — follow-up batch B5 review checkpoint opened: F14 and F24
  `in_progress` → `pending_review`, commit `3f2d992e`; review B5 `pending` →
  `in_progress`, base `c410f784`, head `3f2d992e`.
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
