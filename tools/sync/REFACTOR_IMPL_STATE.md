# girsync refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`, in the recommended
implementation order (low risk first, rename path last). One task per commit
unless a task needs several commits.

Legend: `[ ]` pending · `[x]` done (awaiting review) · `[~]` skipped · `[r]` reviewed

## Phase 0 — pure cleanup

- [ ] I1 — correct `DigestSet::merge`'s documented winner
- [ ] D2 — add `ScanMode::new`
- [ ] D1 — share the link-disjoint validation loop
- [ ] E1 — gate `why_tag` behind `why`
- [ ] E3 — allocation-free `Record` rendering, sorted JSON keys
- [ ] F2 — one mixed-case collision helper
- [ ] H2 — one backup/snapshot copy-aside helper
- [ ] C1 — precompute the `type_conflict` set

## Phase 1 — local performance and reporting

- [ ] D4 — add `CacheDb::entry_count` for the open log
- [ ] G1 — src-then-dst-only exact diff
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

_None._

## Skipped

Tasks skipped, with the reason reported by the implementing session:

_None._
