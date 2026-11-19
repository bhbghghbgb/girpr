# T04 — FNV literals → named constants

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B3 (line 399) — FNV literals
  to named constants.
- Constraint source: §A11 (lines 333–345), especially the lock-file FNV note.

## Scope
- Package: root `girpr`, `src/repair/lock.rs` (lines 40–43, 125–170 in the
  report).

## Implementation plan
1. Replace the hand-written FNV literals with `fnv` crate named constants
   (offset basis / prime), without changing any produced bytes.

## Constraints / validation
- The lock-file key byte sequence **must not change** or existing lock files are
  orphaned (§A11). Only the literal → constant spelling may change; do not alter
  the algorithm, input or key layout.

## Done when
Literals are replaced by named constants and lock-file bytes are unchanged.
