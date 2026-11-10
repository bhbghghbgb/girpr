---
id: T03
title: Document the case-fold rules
batch: B
checkpoint: B
report: [L88-L116, L576-L579]
design_notes: "DESIGN-NOTES.md: same_path's component rule and the case-only rename fixtures"
depends_on: []
size: S
risk: low
---

# T03 — Document the case-fold rules

## Goal

Record the case-fold decision in `DESIGN-NOTES.md`, and pin the one boundary
that is currently unpinned: where the ASCII-only key rule and the Windows
identity rule deliberately disagree.

This task no longer builds the shared fold — the merge already did that. See
"Superseded" below for what changed.

## Why

The crate now has **two** case rules, not three:

- `util::path_key` folds **ASCII case only** and is the key for every
  relative-path index (`check_mixed_case_keys`, `planner/pairs.rs`,
  `diff/mod.rs`, `effective/phase_a.rs`, `commands/sync/rename.rs`,
  `commands/sync/plan/safety.rs`, `commands/sync/plan/link.rs`).
- `util::component_eq` — used by `same_path` and `path_within` — keeps an
  **ASCII fast path plus a Unicode lowercase fallback**, because path *identity*
  is a different question from path *keying*: on Windows the filesystem really
  does treat `Ärger` and `ärger` as one file.

Neither rule is written down in `DESIGN-NOTES.md`, and no test says "these two
deliberately disagree". A later edit that unifies them — say, by making
`component_eq` ASCII-only — would silently change Windows path identity, and
nothing would fail.

A third, narrower rule exists and has no note at all:
`commands/sync/plan/link.rs::on_disk_name` compares directory entries with
Unicode `to_lowercase` to find the spelling the OS would open. That is
deliberately *not* the ASCII-only key rule: it is asking what the filesystem
would open, not what a run's map should key on.

## Superseded

The merge landed `util::path_key` and converted every relative-path index to
it, with `path_key_folds_ascii_case_only` and
`mixed_case_check_groups_by_the_ascii_only_key` already in `util.rs`. The
"replace the ad-hoc `to_lowercase` indexes with one helper" work is done. What
is left is the documentation and the missing boundary test.

Note the deliberate decision the merge recorded: the fold is ASCII-only, and
the Unicode `to_lowercase` fallback was **not** adopted for path keys, because
`to_lowercase` maps the Kelvin sign `K` onto `k` and would merge two spellings
the filesystem keeps apart.

## Files to read

- `src/util.rs` — `path_key` and its doc comment, `component_eq`,
  `check_mixed_case_keys`, the path-identity tests
- `src/planner/pairs.rs` — the `CasePairing::Lowercase` doc, which already
  states the ASCII-only rule
- `src/commands/sync/plan/link.rs` — `on_disk_name`, the third rule
- `tools/sync/DESIGN-NOTES.md` — the invariant sections, and where this note
  belongs

## Constraints

- **Do not change the fold.** `path_key` stays ASCII-only, and
  `component_eq` keeps its Unicode fallback. This task documents the rules; it
  does not re-decide them.
- **Do not merge the two rules.** They answer different questions.
- Do not change `on_disk_name`'s Unicode comparison.
- Do not change any pairing policy. The planner still pairs by folded key and
  the diff still pairs exactly.
- POSIX path identity stays exact byte equality.
- The note must be one rule per paragraph, in the voice of the rest of the
  file: bold invariant, explanation, "Enforced by" tests.

## Implementation

1. Add a `DESIGN-NOTES.md` section, next to the path-identity discussion,
   stating both rules and why they differ. Something like:

   - **The path key folds ASCII case only.** `util::path_key` is the key every
     relative-path index uses. ASCII-only is deliberate: a Unicode fold maps
     the Kelvin sign `K` onto `k`, which would merge two spellings the
     filesystem keeps apart.
   - **Path identity is not the path key.** `component_eq`, behind `same_path`
     and `path_within`, keeps a Unicode lowercase fallback, because on Windows
     the filesystem does treat `Ärger` and `ärger` as one file. The two rules
     are allowed to disagree; that is why they are separate.
   - **The on-disk spelling lookup is neither.** `on_disk_name` asks which
     directory entry the OS would open, so it compares Unicode-lowercased
     names.

2. Add a unit test in `src/util.rs` that pins the boundary: a pair of names
   that `path_key` keeps distinct and `component_eq` (on Windows) treats as
   equal. `Ärger` and `ärger` are the pair; the test must be `#[cfg(windows)]`
   for the identity half, and the key half is platform-independent.

   Name it so the intent is visible in the test list, e.g.
   `the_path_key_and_the_identity_rule_disagree_deliberately`.

3. Check whether `DESIGN-NOTES.md` names the fold anywhere already (it does
   not) and whether the "Known limits" section needs a line about the
   approximation. Keep it to one sentence if it adds anything.

## Commit plan

1. `docs(sync): record the two case-fold rules`
2. `test(sync): pin where the path key and the identity rule disagree`

## Verification

```text
cargo fmt --all -- --check
cargo nextest run -p girsync util::
cargo nextest run -p girsync planner::
cargo nextest run -p girsync
```

## Acceptance

- `DESIGN-NOTES.md` states both rules, says why they differ, and says that
  `on_disk_name` follows neither.
- A test fails if `component_eq` is changed to fold ASCII only.
- `path_key` is unchanged and still ASCII-only.
- No pairing policy changed.

## Reviewer focus

- Does the note say *why* the two rules differ, or only that they do?
- Does it claim the identity rule is an accident?
- Does the new test actually exercise the disagreement, or only the ASCII path?
- Did anything change in `path_key` or `component_eq` that the note now
  describes wrongly?
