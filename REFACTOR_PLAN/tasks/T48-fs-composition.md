# T48 — `girpr-fs` composition leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F53 (lines 684–693) — a cross-program fs
  composition leaf would sit on top of already-shared predicates; candidate boundary is a
  `girpr-fs` leaf exposing `canonical_no_verbatim`, identity from path/handle and `LinkClass`,
  each with an explicit host/fail-closed policy parameter. Confidence medium-low.
- Assumptions to validate: §A4 (48–56), §A3 (37–47), §A5 (57–67), §A2 (29–36), §A14 (129–135), §N5 (713–716).

## Scope
- New leaf (proposed `girpr-fs`) on top of `girpr-fs-names`, `girpr-fs-identity`,
  `crates/util-path`, `crates/sync-leaves`. Sources: girpr lock no-follow body guard
  (`crates/lock/`), girsync hard-link refusal and reparse classification
  (`tools/sync/DESIGN-NOTES.md`), gircap canonical visited set in `verify/list.rs`.
  Locate before editing.

## Implementation plan
1. Validate §F53: confirm the higher-level combinations are local while predicates are
   shared; confirm the one deliberate `unsafe` Windows handle call lives here.
2. If attempted, expose the three operations with explicit host/fail-closed policy
   parameters; keep reparse classification as domain logic in callers where it is such.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Given medium-low confidence and reparse domain logic, a validated skip is legitimate if
   the policies cannot be parameterized safely.

## Constraints / validation
- High-risk (§F53): reparse classification is domain logic; the one deliberate `unsafe`
   Windows handle call lives here and must satisfy the stricter product policy (§A3). The
   device-name drift between girpr and gircap is an open defect candidate — do not "fix" it
   silently; raise a follow-up (§A9).
- Do not absorb the rw gate/symlink classification (N5). gircap independence and doc seams
   (§A4, §A14) mandatory.

## Done when
A policy-parameterized fs leaf is shared with domain logic kept local and bytes unchanged,
or a validated skip with the one-line reason is recorded.
