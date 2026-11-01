---
id: T02
title: Plan-time early returns and prefix memoization
batch: A
checkpoint: A
report: [L191-L206, L344-L368, L584-L586]
design_notes: "A filtered dst blocker is refused before the rename; A hard-linked destination is refused at plan time; A kept dst file link's target is its content"
depends_on: []
size: M
risk: low-medium
---

# T02 — Plan-time early returns

## Goal

Stop doing plan-time filesystem work that cannot change a decision:

- `spare_link_storage` should not sort every dst key, canonicalize dst, or
  build a lowercase map when no kept dst file link exists.
- `ensure_hard_linked_targets_safe` should return immediately when
  `plan.copy` is empty.
- `ensure_mkdir_targets_free` should memoize prefix stats instead of asking
  the disk for the same ancestor repeatedly.

## Why

These guards run on every sync, including link-free and copy-free runs, and
their inputs are often enough to prove they have nothing to do. The report's
§2.5 and §1.6 also warn that the early return must be *sufficient*, not a
heuristic: a wrong return would skip a refusal.

## Files to read

- `src/commands/sync/plan/safety.rs` — all three guards
- `src/commands/sync/plan/link.rs` — `folded_keys`, `LinkStorage`
- `src/commands/sync/plan/build.rs` — `Plan` shape
- `tests/sync.rs` and `src/commands/sync/plan/tests.rs` — existing refusals
- `tools/sync/DESIGN-NOTES.md` sections named above

## Constraints

- Do not change the order of checks or the error text of any refusal.
- Do not weaken the link-storage, hard-link, or mkdir guards.
- Do not skip a guard merely because `plan.copy` is empty for a guard that is
  also protecting deletes or renames.
- Do not touch `unique_sibling` or backup code; that is T09.

## Implementation

1. **`spare_link_storage`**
   - Replace the "collect all `dm` keys, sort them" step with a first pass
     that collects only *kept* links: `rec.is_link && rec.kind == "file"` and
     `sm.get(rel)` is a file.
   - If that list is empty, return `Ok(())` before `canonicalize(dst)` and
     before `folded_keys(dm)`.
   - If it is non-empty, sort only that list. Build `folded_keys` and
     `canonical_dst` only then.
   - Keep the existing chain walk and `terminals` grouping unchanged.
   - This is a pure reordering: the set of kept links is the same.
2. **`ensure_hard_linked_targets_safe`**
   - Return `Ok(())` at the top when `plan.copy.is_empty()`.
   - Justification comment: a copy-free run cannot truncate an inode, so the
     preflight has nothing to protect. Do not generalise this to "plan is
     empty"; deletes and fix-dirs are handled elsewhere.
3. **`ensure_mkdir_targets_free`**
   - Add a local `HashMap<String, Option<bool>>` (or `HashMap<&str, …>` with
     the right lifetime) memoizing `disk_is_dir` by prefix.
   - Reuse the memo for every candidate and every strict prefix. The same
     prefix is currently statted once per descendant.
   - Keep the shortest-prefix-first error selection: the first blocking prefix
     still names the entry that blocks the path.
4. Add tests for report §5.4. Use the existing `plan::tests` helpers where
   possible:
   - a kept dst link with a mirrored target still refuses;
   - a link-free tree reaches the same plan with no spare work (the assertion
     is the plan/refusal, not a timing);
   - a copy-free tree with hard-linked extras is accepted (this is the new
     early return);
   - a deep mkdir path with a blocked prefix is still refused, and the error
     names the blocker, not a deeper action.

## Commit plan

1. `perf(sync): skip link-storage work when no kept link exists`
2. `perf(sync): skip the hard-link preflight when there is nothing to copy`
3. `perf(sync): memoize mkdir prefix stats`
4. `test(sync): pin the plan-time early-return refusals`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync sync::
cargo test -p girsync plan::
cargo test -p girsync
```

## Acceptance

- Every existing refusal still fires with the same text.
- A copy-free run with hard-linked extras is no longer refused by the
  hard-link preflight.
- A link-free run does not canonicalize or fold the dst map.
- Prefix stats are memoized within one `ensure_mkdir_targets_free` call.
- Dry run and real run still agree on refusals.

## Reviewer focus

- Is "no kept link" truly sufficient for skipping `spare_link_storage`? Check
  the `sm`/`dm` filters, case mode, and link chains.
- Is `plan.copy.is_empty()` truly sufficient for the hard-link early return?
- Does the memo key include the path spelling used on disk (dry-run
  `DiskSpelling`)?
- Did any refusal text or check order change?
