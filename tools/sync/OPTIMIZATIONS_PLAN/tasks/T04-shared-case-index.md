---
id: T04
title: Shared one-side case index
batch: B
checkpoint: B
report: [L216-L244]
design_notes: "DESIGN-NOTES.md: the planner's Lowercase pairing differs from the diff's Exact pairing"
depends_on: []
size: M
risk: medium
---

# T04 — Shared one-side case index

## Goal

Build one helper that constructs the "folded key → original key" index, and use
it at the sites that build that exact shape. The fold itself already exists
(`util::path_key`); this task is about the *index*, not the fold.

## Why

Five places build a folded-key index over one or both trees, and four of them
build the same shape:

| site | shape | notes |
| --- | --- | --- |
| `planner/pairs.rs::shared_pairs` | `HashMap<String, &String>` | src side only |
| `diff/mod.rs::diff_maps` | `HashMap<String, &String>` × 2 | src and dst |
| `commands/sync/rename.rs::case_renames` | `HashMap<String, &String>` × 2 | src and dst |
| `commands/sync/plan/safety.rs::ensure_mkdir_targets_free` | `HashMap<String, &EffRec>` | folded → **value**, not key |
| `commands/sync/plan/link.rs::folded_keys` | `HashMap<String, String>` | Windows only; owns its strings |

Only the first three build the same thing, and they build it three times each.
The other two have a different shape and stay as they are — with a comment
saying why.

The performance argument is now weaker than the report assumed: `path_key` is
already one function, so the win is not "one fold instead of three" but "one
index construction instead of three", plus fewer chances for two sites to
disagree about which spelling wins a collision.

## Files to read

- `src/util.rs` — `path_key`, and where the shared helper should live
- `src/planner/pairs.rs` — `shared_pairs`
- `src/diff/mod.rs` — `diff_maps`, the insensitive branch
- `src/commands/sync/rename.rs` — `case_renames`
- `src/commands/sync/plan/safety.rs` — `ensure_mkdir_targets_free`
- `src/commands/sync/plan/link.rs` — `folded_keys`, `map_spelling`
- `src/effective/phase_a.rs` — the alternate-key index

## Constraints

- Keep the planner's `CasePairing::Lowercase` and the diff's
  `CasePairing::Exact` deliberately different.
- Keep each caller's collision policy and direction:
  - `shared_pairs` looks dst up in src;
  - `diff_maps` walks src keys then dst-only keys, keyed by src spelling;
  - `case_renames` moves dst onto src;
  - `ensure_mkdir_targets_free` asks the post-rename dst view;
  - `link.rs` is deliberately Windows-only.
- Keep every explicit sort. These maps are iterated to produce records, and
  unsorted `HashMap` order is not a contract.
- The helper owns **construction only**. It must not decide pairing, direction,
  or which side's spelling wins.
- `ensure_mkdir_targets_free` folds to a *value*, not a key. Either leave it
  alone or give it its own one-liner; do not force it through the key-shaped
  helper.
- `phase_a`'s alternate-key index stores a `Vec` of stale spellings per folded
  key. It is not the same shape and stays as it is, with a comment saying why.
- `link.rs::folded_keys` owns its strings and is Windows-only. Leave it alone.
- Do not build or drop indexes earlier or later than they are today. Build,
  use, drop.

## Implementation

1. In `src/util.rs`, add one helper that builds the folded-key index over a
   map's keys. Something like:

   ```rust
   pub(crate) fn folded_keys<'a, V>(
       map: &'a HashMap<String, V>,
   ) -> HashMap<String, (&'a String, &'a V)>
   ```

   It maps `path_key(k) → (k, v)`. Document that a mixed-case collision is
   already fatal before any caller reaches this helper, so the map is
   unambiguous, and that the helper does **not** pair anything.

2. Use it in `shared_pairs`, `diff_maps`, and `case_renames`. Each currently
   builds one or two such maps inline; each becomes one or two calls.

   `diff_maps` and `case_renames` each build a src map and a dst map. The
   helper is per-map, so each of those two sites calls it twice — which is
   still one definition of the shape rather than two hand-rolled copies.

3. Leave `ensure_mkdir_targets_free`, `phase_a`'s alternate-key index, and
   `link.rs::folded_keys` as they are, and add a one-line comment at each
   saying why it does not share the helper. That comment is the point of the
   task: the next reader should not "fix" the inconsistency.

4. Add unit tests for the helper:
   - every key is present under its folded spelling;
   - the original key and the value are both returned;
   - an exact key and a differently-cased key are one entry only when
     `path_key` says they are.

   Then rely on the existing planner/diff/rename tests for pairing.

## Commit plan

1. `refactor(sync): add a shared folded-key index helper`
2. `refactor(sync): build every folded-key index from the shared helper`
3. `test(sync): pin the shared folded-key index`

## Verification

```text
cargo fmt --all -- --check
cargo nextest run -p girsync util::
cargo nextest run -p girsync planner::
cargo nextest run -p girsync diff::
cargo nextest run -p girsync sync::
cargo nextest run -p girsync
```

## Acceptance

- One definition of the folded-key index shape, used by `shared_pairs`,
  `diff_maps`, and `case_renames`.
- The three non-conforming sites each carry a comment saying why they differ.
- The helper contains no pairing, direction, or collision policy.
- `case_renames` still sorts by folded src key.
- Record order in the text and JSON output is unchanged.

## Reviewer focus

- Compare each rewritten index with the old one for *which side wins* on a
  collision and for the exact output order.
- Did any index gain a lifetime that keeps a second full map alive?
- Is the helper generic without becoming a trait or a policy object?
- Does anything now fold a name that used to be compared exactly?
- Do the three exception comments explain the *shape* difference, or just say
  "different"?
