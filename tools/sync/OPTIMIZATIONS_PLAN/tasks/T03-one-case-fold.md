---
id: T03
title: One documented case fold
batch: B
checkpoint: B
report: [L88-L116, L576-L579]
design_notes: "DESIGN-NOTES.md: same_path's component rule and the case-only rename fixtures"
depends_on: []
size: M
risk: medium
---

# T03 — One documented case fold

## Goal

Replace the several accidental case folds used for relative-path keys with one
documented helper, and make path identity's non-ASCII fallback use the same
rule. Add the tests that pin the chosen rule where Unicode lowercasing and the
platform's identity rule can differ.

## Why

The crate currently uses `to_lowercase` for every string-key index, an
ASCII-fast-path plus `to_lowercase` fallback in `component_eq`, and a
Windows-only lowercase map in `plan/link.rs`. They usually agree. For dotted or
dotless I, ligatures, case-mapping expansions and some non-BMP names they can
disagree, which can produce a harmless-looking refusal or a missed collision.
`check_mixed_case_keys` and the joins must not be three spellings of one rule.

## Files to read

- `src/util.rs` — `component_eq`, `check_mixed_case_keys`, path-identity tests
- `src/planner/pairs.rs` — `shared_pairs`
- `src/diff/mod.rs` — the insensitive branch of `diff_maps`
- `src/effective/phase_a.rs` — the alternate-key index
- `src/commands/sync/rename.rs` — `case_renames`
- `src/commands/sync/plan/safety.rs` — `fixed_lower`, `dst_lower`
- `src/commands/sync/plan/link.rs` — `folded_keys`, `map_spelling`, `on_disk_name`

## Constraints

- **Do not change a pairing policy.** The planner still pairs on disk by
  lowercase; the diff still pairs post-rename exactly; rename still moves dst
  onto src's casing.
- On POSIX, path identity stays exact byte equality. Only the string-key
  indexes used by the tool's `--case-sensitive false` mode fold.
- A truly exact NTFS fold is not available portably. Centralize the existing
  approximation and state its limits; do not try to emulate the upcase table.
- Do not fold algorithm names. `hash/set.rs`'s parser is a different
  vocabulary.
- `plan/link.rs`'s `folded_keys` exists to emulate Windows filesystem matching;
  it stays empty on non-Windows even after this change.

## Implementation

1. Add one `pub(crate) fn fold_key(&str) -> String` in `src/util.rs`. It is the
   single definition of the case fold used for relative-path keys. Document at
   the function that it is Unicode `to_lowercase`, that it is an approximation
   of the Windows filesystem's upcase table, and that path identity on POSIX
   continues to use exact byte equality.
2. Make `check_mixed_case_keys` group by `fold_key` instead of calling
   `to_lowercase` inline.
3. In `component_eq`'s Windows non-ASCII fallback, compare with `fold_key` on
   the lossy strings. Keep the ASCII `eq_ignore_ascii_case` fast path; it is
   the same rule for ASCII and avoids an allocation. The non-Windows arm stays
   `a == b`.
4. Replace every relative-path `to_lowercase()` in `planner/pairs.rs`,
   `diff/mod.rs`, `effective/phase_a.rs`, `commands/sync/rename.rs`,
   `commands/sync/plan/safety.rs`, and `commands/sync/plan/link.rs` with
   `fold_key`. Inside `folded_keys`, `map_spelling`, `on_disk_name` and
   `spare_alias_chain`, call `fold_key` but leave the `#[cfg(windows)]` /
   `#[cfg(not(windows))]` gates exactly as they are.
5. Add unit tests in `util.rs`:
   - two names that Unicode lowercasing joins are reported as one mixed-case
     group;
   - a name that only the platform identity rule would join is not silently
     joined by the string helper unless Unicode lowercasing says so;
   - on Windows, `component_eq` still matches ASCII and non-ASCII case variants
     and still distinguishes names Unicode does not join;
   - on POSIX, `same_path` remains exact for case-only names.
   Use `#[cfg(windows)]` / `#[cfg(not(windows))]` so the suite pins the rule
   that actually applies on the host.
6. Add a note in `tools/sync/DESIGN-NOTES.md` beside the path-identity
   discussion: one fold, its limits, and why the POSIX arm is exact. (The full
   documentation reconciliation is T16; here only the invariant this task
   creates.)

## Commit plan

1. `refactor(sync): add one documented case-fold helper`
2. `refactor(sync): use the shared fold for every path-key index`
3. `test(sync): pin the case fold and its platform limits`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync util::
cargo test -p girsync planner::
cargo test -p girsync diff::
cargo test -p girsync
```

## Acceptance

- Exactly one string-key fold exists; every key index calls it.
- `component_eq`'s non-ASCII fallback uses the same rule; the ASCII fast path is
  documented as equivalent for ASCII only.
- POSIX path identity is still exact.
- Planner/diff/rename pairing rules are unchanged.
- The new tests fail if a later edit reintroduces an inline `to_lowercase` index.

## Reviewer focus

- Did any caller's collision or ordering assumption change?
- Is `folded_keys` still Windows-only, and is `map_spelling` still exact first?
- Do the tests actually distinguish the two folds, rather than passing for both?
- Is the documented limit honest about not matching NTFS exactly?
