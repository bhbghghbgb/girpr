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

- [r] B1 — one key snapshot for link pre-drops
- [r] B2 — `DirPrefixes` across `build_plan`, `remove_unknown_dirs`, `prune_dst_cache`
- [r] C2 — copy/fix membership sets in the link-storage guards
- [r] D3 — shared compare/compare-self tail
- [ ] F1 — keys-only alternate index, on-demand adopted rows, adoption-aware dry-run prune count
- [ ] I3 — Windows ASCII fast path with the Unicode fallback

## Phase 3 — rename path

- [ ] A1 — `MoveIndex` for `plan_steps`, `final_key`, and `DiskSpelling`

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
  **Addressed in `5359071`:** both sides and both relations are asserted end to
  end, and the unresolvable-link context is pinned by a unit test.
- **E3 — latent duplicate-key divergence.** `Record::write_json` matches
  `Record::json().to_string()` byte-for-byte for every record built today (the
  new unit tests cover key order, the `UInt`/`Int` split, arrays, and escaping).
  It would diverge if a field key were `event` or repeated: the map keeps the
  last value, the writer emits both members. Keys are static and unique today,
  so this is latent only; a `debug_assert!` on uniqueness, or skipping an
  `event` field in `write_json`, would close it.
  **Addressed in `dce87a2`:** `put` now rejects a reserved or repeated key with
  one test per guard (debug builds only).
- **F2 — helper placement.** `check_mixed_case_keys` lives in `scan` and is
  imported by `effective::record`, whose module doc says the disk walk and the
  table read deliberately do not share a module. `util.rs` would keep that
  layering, the same move B2's `DirPrefixes` makes.
  **Addressed in `ff1d76c`:** moved unchanged in behavior and message.
- **D2 — adopt the constructor in tests.** Production call sites all use
  `ScanMode::new`; the integration-test helpers still build the literal
  (`tests/common/mod.rs`, `tests/lazy/*`, `tests/hash_mode/*`,
  `tests/compare.rs`, ...). Test-only and optional, but it is what makes one
  edit enough when a field is added.
  **Addressed in `3655645`:** every test-side literal calls the constructor,
  including the `const OPEN_MODE` initialiser.

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
  **Addressed in `a27cd1d`:** `dst_all` is now built once before the pair loop,
  under a `match mode` — `AllOf` builds it, `AnyOf` gets `AlgoSet::NONE` — so it
  is neither rebuilt per pair nor computed where it is unused. `src_all` stays
  separate and still feeds `fallback`, and plan order is unchanged.
- **G4 — helper signature is the only rough edge.**
  `record_shortfalls(&mut tally, &[(usize, &SideRequest<'_>, &SideEntry,
  &String)])` mirrors the local `sides` array exactly, and both call sites now
  record the same set; the `AnyOf` "nothing obtainable" branch is still the only
  place it runs under that mode. A tiny named struct would read better if a
  third field is ever added; no behavior to change.
  **Declined in the follow-up batch:** the helper is private with one caller
  shape, so the struct would add a type without removing awkwardness. Revisit
  with a third field.
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
  **Addressed in `6f11ee3`:** the canonical-name caveat and the `get_named`
  escape hatch are documented on `get_algo`.
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
  **Addressed in `b89c189`:** `visit_verdict` is re-exported at the crate root.
  The summary-skip variant was deliberately declined; the note calls it harmless,
  and it would need a public signature change or a second visitor to keep in
  sync, so no action.

### Phase 2 — behavior-adjacent, B1–D3 — accepted

Reviewed the batch at `82e81f6`, which is code-identical to `HEAD`:
`git diff 82e81f6..HEAD -- tools/sync/src` is empty, and only this state file
changed after it. Each commit was checked out in a separate worktree, reusing
the main target directory, and passed `cargo check -p girsync --all-targets`.
`cargo test -p girsync` is green for every target at the batch tip, and
`cargo clippy -p girsync --all-targets -- -D warnings` and
`cargo fmt -p girsync -- --check` are clean. Each commit's own guards were also
run in place: the link-removal `compare-self` case at `84edec5`, `DirPrefixes`
and the three wiped-subtree sites at `173e54d`, the shared-storage refusals at
`08707f6`, and the compare/compare-self exit codes and JSON summary at
`82e81f6`. B1, B2, C2 and D3 preserve the documented contracts; nothing needs a
revert or a code fix. F1 and I3 are still phase 2 `[ ]` tasks and remain
unimplemented. The follow-ups below are non-blocking.

- **B1 — one deleted link is pinned, several are not.** The existing
  `a_sync_that_removes_a_link_leaves_compare_self_clean` covers a single link
  with followed descendants, but the change's point is that one key snapshot
  serves *every* link. Add a fixture with two deleted links (siblings, and one
  nested under the other) whose descendants have cache rows, then assert
  `compare-self` is clean and each descendant row is gone. Without it, a future
  edit that moves `load_keys` back inside a per-link loop would still pass the
  suite and reintroduce the N×cache scan.
- **B2 — the `dm`-kind filter is only covered end-to-end.** The new unit test
  pins strict descendant matching and the component boundary, and the three
  call sites keep their integration guards
  (`rows_for_a_subtree_a_copy_wiped_are_dropped`,
  `a_folder_prunes_its_own_cache_before_the_planner_sees_it`,
  `a_copy_that_wipes_a_blocking_dir_is_not_also_planned_for_removal`). What is
  not directly pinned is `wiped_by_copy`'s "copy whose dst is a directory"
  filter: a unit test with a file-kind `dm` entry would hold it. The helper's
  placement in `plan/build.rs` and its re-export to `apply` follow the finding;
  `util.rs` remains the option if more path helpers land. No action for
  acceptance.
- **C2 — one comment names the wrong set.** The new comment above `copy_rels`
  says "copy and type-conflict sets", but the second set is `fix_dirs`; fix the
  sentence to "copy and fix-dir sets". The rest is faithful: the first
  `terminals` entry per key supplies the label, matching the old
  `terminals.iter().find`, and group order is still sorted by key. If a future
  change makes `terminals` large, build `copy_rels` once in
  `spare_link_storage` and pass it to `ensure_shared_storage_safe` rather than
  building it twice; today both are O(copies), so this is style only.
- **D3 — no direct test, but the commands hold it.** `report_and_finish` has no
  unit test of its own; the two exit paths and the JSON `SUMMARY` are covered by
  `a_case_only_pair_exits_four_with_total_diff_zero`,
  `compare_json_is_one_object_per_line_with_a_single_summary` and the
  compare-self drift tests. The `"end"` field list is now one definition, which
  removes the drift risk the finding named. An optional direct test could assert
  the helper returns `0` for an empty `Diff` and `4` otherwise, but that would
  duplicate what the command tests already do.

## Follow-up batches

Review follow-ups from the notes above, addressed in one batch per phase.

- Phase 0 batch — reviewed, accepted: `5359071` (D1 diagnostics pinned),
  `3655645` (D2 test constructors), `dce87a2` (E3 field-key guard),
  `ff1d76c` (F2 helper moved to `util`). Checked at `5359071` in its own
  worktree: `cargo test -p girsync` green on every target, and clippy and
  `cargo fmt --check` clean. The suite is green at HEAD too
  (`cargo nextest run -p girsync`: 439 passed, 1 skipped). One residual,
  non-blocking: the E3 guard is a `debug_assert!`, so a release-mode test run
  would not panic and the two `#[should_panic]` tests would fail. The project
  runner is nextest in the dev profile, where they pass; gate those tests on
  `debug_assertions` (or make the guard a hard `assert!`) if a release-mode
  suite is ever added.
- Phase 1 batch — reviewed, accepted: `6f11ee3` (I2 lookup caveat doc),
  `b89c189` (E2 visitor export), and `a27cd1d` (G3 `dst_all` built once per run,
  re-landed after `b647505` was rejected for rebuilding it per pair). G4's struct
  and the E2 summary-skip variant were deliberately skipped and accepted as such.
  Checked at `a27cd1d`: `cargo test -p girsync` green on every target, and
  clippy and `cargo fmt --check` clean.
- Phase 2 (B1–D3) batch — done, awaiting review: `a5bae82` (B1 multi-link
  pruning), `07db98c` (B2 copy-wiped kind filter), `d026655` (C2 comment);
  the C2 shared-set style point and the D3 direct test deliberately skipped.

## Skipped

Tasks skipped, with the reason reported by the implementing session:

_None._
