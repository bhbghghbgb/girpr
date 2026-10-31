# girsync refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`, in the recommended
implementation order (low risk first, rename path last). One task per commit
unless a task needs several commits.

Legend: `[ ]` pending · `[x]` done (awaiting review) · `[~]` skipped · `[r]` reviewed

## Phase 0 — pure cleanup

- [x] I1 — correct `DigestSet::merge`'s documented winner
- [x] D2 — add `ScanMode::new`
- [x] D1 — share the link-disjoint validation loop
- [x] E1 — gate `why_tag` behind `why`
- [x] E3 — allocation-free `Record` rendering, sorted JSON keys
- [x] F2 — one mixed-case collision helper
- [x] H2 — one backup/snapshot copy-aside helper
- [x] C1 — precompute the `type_conflict` set

## Phase 1 — local performance and reporting

- [x] D4 — add `CacheDb::entry_count` for the open log
- [x] G1 — src-then-dst-only exact diff
- [ ] G2 — shared pair classification
- [ ] G3 — precompute the run-wide `AlgoSet`s
- [ ] G4 — shared shortfall scan and exact-key lookup
- [ ] H1 — borrowed-slice codec decode
- [ ] I2 — typed digest lookup with the foreign fallback
- [ ] J1 — per-worker copy scratch buffer
- [ ] E2 — `visit_verdict` with the collecting adapter retained

## Phase 2 — behavior-adjacent

- [ ] B1 — one key snapshot for link pre-drops
- [ ] B2 — `DirPrefixes` across `build_plan`, `remove_unknown_dirs`, `prune_dst_cache`
- [ ] C2 — copy/fix membership sets in the link-storage guards
- [ ] D3 — shared compare/compare-self tail
- [ ] F1 — keys-only alternate index, on-demand adopted rows, adoption-aware dry-run prune count
- [ ] I3 — Windows ASCII fast path with the Unicode fallback

## Phase 3 — rename path

- [ ] A1 — `MoveIndex` for `plan_steps`, `final_key`, and `DiskSpelling`

## Pending review

Tasks finished but not yet reviewed by hand:

- I1 — `girsync: document the merge write direction` (`1844e29`)
- D2 — `girsync: add ScanMode constructor` (`7e6b1e2`)
- D1 — `girsync: share link-disjoint validation` (`ecbd519`)
- E1 — `girsync: render why tags only when requested` (`d378dfd`)
- E3 — `girsync: render records without per-field allocations` (`e30836d`)
- F2 — `girsync: share mixed-case collision check` (`3628ba6`)
- H2 — `girsync: share cache copy-aside helper` (`27aee3e`)
- C1 — `girsync: precompute planned type conflicts` (`938431a`)
- D4 — `girsync: add count-only cache entry method` (`2c6047e`)
- G1 — `girsync: walk exact diff keys without a union set` (`7cde85c`)

## Skipped

Tasks skipped, with the reason reported by the implementing session:

_None._
