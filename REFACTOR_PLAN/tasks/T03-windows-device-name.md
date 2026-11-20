# T03 — Windows device-name predicate leaf (`girpr-fs-names`)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F6 (lines 155–164) — the
  duplicated and drifted device-name predicate.
- Assumptions to validate: §A10 (83–88), §A3 (29–35), §A2 (21–27).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§A10).

## Scope
- Candidate shared crate `crates/fs-names` (`girpr-fs-names`), a leaf of pure
  predicates (§F6).
- Report names: `src/util/path.rs` (product predicate, `contained_rel`),
  `tools/capture/src/paths.rs` (capture predicate and module doc). Locate before
  editing.

## Implementation plan
1. Validate the textual difference in §F6 against the current code: product
   reserves `CON PRN AUX NUL CONIN$ CONOUT$ CLOCK$ COM1-9 LPT1-9` and trims
   trailing space/dot before an ASCII-case-insensitive compare; capture omits
   `CLOCK$` and compares the raw uppercased stem.
2. Create a shared predicate whose reserved set and trim behavior are
   **caller-supplied parameters**, so girpr keeps its exact rule and capture
   keeps its exact rule. The goal is to share the mechanism, not to pick a
   winner between the two rules.
3. Route both call sites through the shared predicate with their current
   parameters, confirming bundle bytes and containment decisions are unchanged.
4. Record the behavioral-divergence question from §A10 as a follow-up for a
   reviewer; do **not** change either predicate's behavior in this task.

## Constraints / validation
- Bundle bytes must not move (§A3). No `CLOCK$` add/remove and no trim change.
- If the only way to share the rule is to change a predicate's behavior, stop
  and mark `skipped` with the §A10 reason; raise a follow-up instead.
- A new gircap leaf dep requires the §A2 doc tables to be updated.

## Done when
Both device-name checks call the shared parameterized predicate, each keeps its
current reserved set and trimming, and no bundle/containment test changes.
