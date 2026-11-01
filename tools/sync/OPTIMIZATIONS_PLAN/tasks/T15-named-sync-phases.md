---
id: T15
title: Named sync phases decision
batch: F
checkpoint: F
report: [L458-L476]
design_notes: "The phase order is the invariant; extracting phases for its own sake risks the one property the module exists to guarantee"
depends_on: [T09]
size: S
risk: low
---

# T15 — Named sync phases decision

## Goal

Decide **not** to extract `cmd_sync` into a set of phase functions. Instead,
name and document the phases in the module (or function) so a reader can see
the invariant and its inputs/outputs without moving code.

This is a documentation-only task unless a tiny extraction is needed to name
something that T09 already introduced (the backup step). Do not restructure
the orchestrator.

## Why

The report's §3.3 is explicit: the phase order is the invariant, moving code can
change borrows, lifetimes, backup timing, or when a cache is opened, and the
direct gain is readability only. The current shape is explicit and auditable.
The right deliverable is a named-phase contract, not a riskier refactor.

## Files to read

- `src/commands/sync/mod.rs` — the module doc and `cmd_sync`'s numbered
  comments
- `src/commands/sync/apply/mod.rs` — the apply phase order doc
- `src/commands/sync/rename.rs` — where the rename sits relative to the plan
- `README.md` — the sync step list
- `tools/sync/DESIGN-NOTES.md` — the phase/invariant notes

## Constraints

- **No behaviour change.** No cache open/snapshot/rename/plan/apply call may
  move.
- Do not introduce a `Phase` trait, a phase enum, or a generic runner.
- Do not split `cmd_sync` into functions that each take a large subset of its
  locals merely to mirror the numbering. If a helper is useful (for example the
  backup step from T09), extract only that helper and keep the call in place.
- Keep the existing numbered comments; add the names and the contract around
  them rather than replacing the numbering with prose.
- Do not claim bindings that are not there (for example a “backup phase” when
  T09 made it conditional). Describe what the code actually does.

## Implementation

1. In `commands/sync/mod.rs`, expand the module doc with a phase table:
   | # | phase | invariants |
   | --- | --- | --- |
   | 1 | validate + backups/snapshot | before any cache is opened; dry run skips |
   | 2 | open caches + phase A each side | link-disjoint checks run after each walk |
   | 3 | pair plan | pairs as on disk, before the rename |
   | 4 | resolve both sides | phase C hashes only what the plan asked |
   | 5 | mkdir preflight | before the first filesystem change |
   | 6 | rename dst to src casing | before the diff; re-keys `dm` even in a dry run |
   | 7 | diff + plan + guards | plan print is the diff, actions follow |
   | 8 | dry-run records or apply | apply order lives in `Applier::apply` |
   Use the actual numbers and names from the code; adjust if T09 added a helper.
2. Add a short paragraph on the three ordering facts that are easy to break:
   pair-before-rename, rename-before-diff, and pre-drop-before-filesystem.
   Cross-reference `apply/mod.rs` rather than duplicating its full list.
3. If T09 extracted a `backup_before_run(...)` helper, give it a doc comment
   describing the policy and link it from the table. Do not extract anything
   else.
4. Update `README.md`'s sync step list only if the names in the table differ
   from the wording there; keep the user-facing steps.
5. No tests are required for a documentation change. If a comment claims an
   ordering that is not covered by a test, note the existing test that pins it
   (for example the case-only content test named in the README).

## Commit plan

1. `docs(sync): name the sync phases and their ordering invariants`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync
```

## Acceptance

- The module doc names every phase in `cmd_sync` in order.
- Each entry states the invariant that makes the order load-bearing.
- No code is moved except, optionally, the backup step from T09.
- No new trait/enum/runner is introduced.
- The README and module doc agree on the phase names.

## Reviewer focus

- Does the documentation match the code as shipped after T09?
- Did any call move, even “harmlessly”?
- Is the pair-before-rename reason stated correctly?
- Is the pre-drop/commit point described without overstating it?
- Did the change add a second vocabulary for the same phases?
