---
id: T04
title: Shared one-side case index
batch: B
checkpoint: B
report: [L216-L244]
design_notes: "DESIGN-NOTES.md: the planner's Lowercase pairing differs from the diff's Exact pairing"
depends_on: [T03]
size: L
risk: medium
---

# T04 — Shared one-side case index

## Goal

Build the "lowercase key -> original key (and value)" index in one place and use
it from every insensitive join, without moving any pairing decision into the
helper.

## Why

`shared_pairs`, `diff_maps`, `case_renames`, `ensure_mkdir_targets_free`, and
`phase_a`'s alternate index each lowercase one or both full trees and keep their
own map. A sync lowercases every key several times and retains several duplicate
maps at once. The helper may own normalization and index construction; it must
not decide which paths are the same path.

## Files to read

- `src/util.rs` — `fold_key` from T03
- `src/planner/pairs.rs` — `shared_pairs`
- `src/diff/mod.rs` — the insensitive branch
- `src/commands/sync/rename.rs` — `case_renames`
- `src/effective/phase_a.rs` — `alt_index`
- `src/commands/sync/plan/safety.rs` — `ensure_mkdir_targets_free`
- `src/commands/sync/plan/link.rs` — `folded_keys`, `map_spelling`

## Constraints

- Keep the planner's `CasePairing::Lowercase` and the diff's
  `CasePairing::Exact` deliberately different.
- Keep each caller's collision policy and direction:
  - `shared_pairs` looks dst up in src;
  - `diff_maps` walks src then dst-only keys, keyed by src spelling;
  - `case_renames` moves dst onto src;
  - `ensure_mkdir_targets_free` asks the post-rename dst view;
  - `link.rs` is deliberately Windows-only.
- Keep every explicit sort. These maps are iterated to produce records, and
  unsorted `HashMap` order is not a contract.
- Do not keep several indexes alive longer than the current code does. Build,
  use, drop.

## Implementation

1. In `src/util.rs`, add one generic helper:
   ```rust
   pub(crate) fn folded_index<'a, V>(
       map: &'a HashMap<String, V>,
   ) -> HashMap<String, (&'a String, &'a V)>
   ```
   It maps `fold_key(k) -> (k, v)`. Document that a mixed-case collision is
   already fatal before any caller reaches this helper, so the map is
   unambiguous, and that the helper does **not** pair anything.
2. Use it in `shared_pairs`: build the src index once, fold each dst key once,
   and keep the existing `out.sort()`.
3. Use it in `diff_maps`' insensitive branch: build `slow` and `dlow` from the
   helper, keep the two passes (src keys then dst-only keys), and keep the
   bucket sorts.
4. Use it in `case_renames`: build both indexes from the helper, keep the
   `by_lower.sort()` ordering.
5. Use it in `ensure_mkdir_targets_free` for the `dst_lower` value map; keep
   `dst_after_rename`'s exact-first lookup in `case_sensitive` mode.
6. Leave `phase_a`'s alternate index as an exception: it stores a `Vec` of
   stale spellings per folded key, not one value, so it continues to use
   `fold_key` directly (from T03). Write one comment saying why it is not the
   shared one-value index.
7. Leave `link.rs`'s `folded_keys`/`map_spelling` as the Windows-only filesystem
   view; call `fold_key` inside them but do not route them through the helper,
   because the helper must not become Windows-only.
8. Add unit tests for the helper:
   - every key is present under its folded spelling;
   - the original key and value are returned;
   - an exact key and a differently-cased key are one entry only when
     `fold_key` says they are.
   Then rely on the existing planner/diff/rename tests for pairing.

## Commit plan

1. `refactor(sync): add a shared one-side case index`
2. `refactor(sync): build every insensitive join from the shared index`
3. `test(sync): pin the shared index's key and value mapping`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync util::
cargo test -p girsync planner::
cargo test -p girsync diff::
cargo test -p girsync sync::
cargo test -p girsync
```

## Acceptance

- One lowercase pass per side per join, not one per caller site.
- The planner still pairs pre-rename by lowercase and the diff still pairs
  post-rename exactly.
- Record order is unchanged in text and JSON output.
- The helper contains no pairing, direction, or collision policy.
- `case_renames` still sorts by lowercase src key.

## Reviewer focus

- Compare each rewritten join with the old one for *which side wins* on a
  collision and for the exact output order.
- Did any index gain a lifetime that keeps a second full map alive?
- Is the helper generic without becoming a trait or a policy object?
- Does anything now fold a name that used to be compared exactly?
