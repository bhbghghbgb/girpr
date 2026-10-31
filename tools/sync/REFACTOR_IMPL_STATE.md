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

- [r] D4 — add `CacheDb::entry_count` for the open log
- [r] G1 — src-then-dst-only exact diff
- [r] G2 — shared pair classification
- [r] G3 — precompute the run-wide `AlgoSet`s
- [r] G4 — shared shortfall scan and exact-key lookup
- [r] H1 — borrowed-slice codec decode
- [r] I2 — typed digest lookup with the foreign fallback
- [r] J1 — per-worker copy scratch buffer
- [r] E2 — `visit_verdict` with the collecting adapter retained

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

Phase 2 tasks finished but not yet reviewed by hand:

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

### Phase 1 — local performance and reporting — accepted

Reviewed the batch at `5e1dc78`, checked out in its own worktree so the later
phase-2 commits could not mask it, and checked each of the nine implementation
commits with `cargo check -p girsync --all-targets` on its own. The suite is
green for every target, and `cargo clippy -p girsync --all-targets -- -D warnings`
and `cargo fmt -p girsync -- --check` are clean at that commit. D4, G1, G2, G3,
G4, H1, I2, J1 and E2 preserve the documented contracts; nothing needs a revert
or a code fix. B1, B2, C2 and D3 are phase 2 and were not part of this batch, so
they stay in the pending list. The follow-ups below are non-blocking.

- **D4 — the count is pinned where it matters, not in the log.** `entry_count`
  returns the raw table length, reads a missing table as zero, and the new test
  holds it equal to `load_keys().len()` before and after a remove. The `entries`
  log field itself is not asserted by an integration test, but it only forwards
  this value; no action.
- **G1 — residual lookup, not a regression.** The union `HashSet` is gone, but a
  both-sides key still costs `dst.get` in the src pass plus `src.contains_key` in
  the dst pass, so the finding's "second lookup per key" is only removed for
  one-sided keys; for pairs the two lookups remain. The new comment ("one
  membership test and never revisits a pair") is accurate about classification.
  Correct the finding's expected saving rather than the code — the remaining
  membership test is inherent to the two-pass shape.
- **G2 — allocations are unchanged.** `classify_pair` calls `key.to_string()`
  for the bucket and again for `verdicts`; the two inlined blocks did the same.
  Not a regression, and one shared definition is the point. If the per-pair
  allocation ever matters, interned keys are the change.
- **G3 — `dst_all` is built in both modes.** Only `AllOf` consumes it; under
  `AnyOf` the per-pair `AlgoSet::one(pick)` is used. It is a two-byte value built
  once, so this is clarity only; moving it into the `AllOf` arm would remove the
  unused value if the loop grows.
- **G4 — helper signature is the only rough edge.**
  `record_shortfalls(&mut tally, &[(usize, &SideRequest<'_>, &SideEntry,
  &String)])` mirrors the local `sides` array exactly, and both call sites now
  record the same set; the `AnyOf` "nothing obtainable" branch is still the only
  place it runs under that mode. A tiny named struct would read better if a
  third field is ever added; no behavior to change.
- **H1 — wire format unchanged.** The borrowed `take` still fails on the same
  truncation and trailing-byte conditions, and `insert_named` still carries an
  unknown or wrong-length name. The byte-identity codec test is the guard; no
  follow-up.
- **I2 — case-insensitive parse is a latent trap for future callers.**
  `HashAlg::from_str` is ASCII-case-insensitive, so `get_named("MD5")` can find a
  foreign entry spelled `MD5`, while `get_algo(Md5)` only looks for the canonical
  `md5`. Every converted call site passes `a.as_str()`, so behavior is identical
  today; a sentence in `get_algo`'s doc would keep a future mixed-case caller
  honest.
- **J1 — the new test is the guard.** The buffer is reused without clearing, and
  correctness rests on only `buf[..n]` being written and digested;
  `a_reused_buffer_digests_only_the_bytes_that_were_read` runs a big file before
  a small one to pin exactly that. Keep it if the copy loop is ever restructured.
- **E2 — the streaming API is not at the crate root.** `verdict` is re-exported
  as `girsync::verdict`, but its streaming counterpart is only reachable as
  `girsync::report::visit_verdict`. Adding it to the `pub use report::{...}`
  line would give library callers the same choice the commands now make. Also,
  `sync` still builds the `SUMMARY` record and drops it in the closure;
  harmless, but a summary-skipping variant would avoid the work if that path
  grows.

## Skipped

Tasks skipped, with the reason reported by the implementing session:

_None._
