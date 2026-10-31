# girsync refactor implementation state

Checklist for the findings in `REFACTOR_FINDINGS.md`, in the recommended
implementation order (low risk first, rename path last). One task per commit
unless a task needs several commits.

Legend: `[ ]` pending · `[x]` done (awaiting review) · `[~]` skipped · `[r]` reviewed

## Phase 0 — pure cleanup

- [r] I1 — correct `DigestSet::merge`'s documented winner
- [r] D2 — add `ScanMode::new`
- [r] D1 — share the link-disjoint validation loop
- [r] E1 — gate `why_tag` behind `why`
- [r] E3 — allocation-free `Record` rendering, sorted JSON keys
- [r] F2 — one mixed-case collision helper
- [r] H2 — one backup/snapshot copy-aside helper
- [r] C1 — precompute the `type_conflict` set

## Phase 1 — local performance and reporting

- [x] D4 — add `CacheDb::entry_count` for the open log
- [x] G1 — src-then-dst-only exact diff
- [x] G2 — shared pair classification
- [x] G3 — precompute the run-wide `AlgoSet`s
- [x] G4 — shared shortfall scan and exact-key lookup
- [x] H1 — borrowed-slice codec decode
- [x] I2 — typed digest lookup with the foreign fallback
- [x] J1 — per-worker copy scratch buffer
- [x] E2 — `visit_verdict` with the collecting adapter retained

## Phase 2 — behavior-adjacent

- [x] B1 — one key snapshot for link pre-drops
- [x] B2 — `DirPrefixes` across `build_plan`, `remove_unknown_dirs`, `prune_dst_cache`
- [x] C2 — copy/fix membership sets in the link-storage guards
- [x] D3 — shared compare/compare-self tail
- [ ] F1 — keys-only alternate index, on-demand adopted rows, adoption-aware dry-run prune count
- [ ] I3 — Windows ASCII fast path with the Unicode fallback

## Phase 3 — rename path

- [ ] A1 — `MoveIndex` for `plan_steps`, `final_key`, and `DiskSpelling`

## Pending review

Tasks finished but not yet reviewed by hand:

- D4 — `girsync: add count-only cache entry method` (`2c6047e`)
- G1 — `girsync: walk exact diff keys without a union set` (`7cde85c`)
- G2 — `girsync: share pair classification in diff` (`3877e90`)
- G3 — `girsync: precompute run-wide algorithm sets` (`9dd10fe`)
- G4 — `girsync: share shortfall scan in pair planning` (`fbd8e89`)
- H1 — `girsync: decode cache rows from borrowed slices` (`747c0e1`)
- I2 — `girsync: add typed digest lookup` (`20802e6`)
- J1 — `girsync: reuse copy buffer per worker` (`867f9a9`)
- E2 — `girsync: stream verdict records` (`5e1dc78`)
- B1 — `girsync: snapshot cache keys once for link pre-drops` (`84edec5`)
- B2 — `girsync: share wiped-subtree prefix checks` (`173e54d`)
- C2 — `girsync: precompute link storage membership` (`08707f6`)
- D3 — `girsync: share compare finish path` (`82e81f6`)

## Review notes

### Phase 0 — pure cleanup — accepted

Reviewed the batch at `938431a` (in its own worktree, so later commits could not
mask it) and again at `3e32eb38b565`. `cargo test -p girsync` is green for every
target and `cargo clippy -p girsync --all-targets -- -D warnings` is clean.
I1, D2, D1, E1, E3, F2, H2 and C1 preserve the documented behavior; nothing
needs a revert or a code fix. The follow-ups below are non-blocking.

- **D1 — pin the diagnostics.** The shared helper reproduces both wrappers'
  sentences exactly, but no test holds them: `tests/symlink.rs` and
  `tests/sync.rs` only assert stderr contains `disjoint` and `link`. Add one
  assertion per side and direction for the full sentence (`may not read its own
  destination`, `write into its own source`, and the two `contains` variants),
  plus the `resolve src link` / `resolve dst link` context, so a future edit to
  `LinkSide` cannot silently change the advice a user acts on.
- **E3 — latent duplicate-key divergence.** `Record::write_json` matches
  `Record::json().to_string()` byte-for-byte for every record built today (the
  new unit tests cover key order, the `UInt`/`Int` split, arrays, and escaping).
  It would diverge if a field key were `event` or repeated: the map keeps the
  last value, the writer emits both members. Keys are static and unique today,
  so this is latent only; a `debug_assert!` on uniqueness, or skipping an
  `event` field in `write_json`, would close it.
- **F2 — helper placement.** `check_mixed_case_keys` lives in `scan` and is
  imported by `effective::record`, whose module doc says the disk walk and the
  table read deliberately do not share a module. `util.rs` would keep that
  layering, the same move B2's `DirPrefixes` makes.
- **D2 — adopt the constructor in tests.** Production call sites all use
  `ScanMode::new`; the integration-test helpers still build the literal
  (`tests/common/mod.rs`, `tests/lazy/*`, `tests/hash_mode/*`,
  `tests/compare.rs`, ...). Test-only and optional, but it is what makes one
  edit enough when a field is added.

## Skipped

Tasks skipped, with the reason reported by the implementing session:

_None._
