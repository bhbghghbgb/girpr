# T35 — `IntOrStr` → `serde_with` (optional, low priority)

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §F9 (lines 723–728). This is
  the one candidate in §F9 that may be worth delegating; the other §F9 items
  (ranges, task runner) are "keep" (see `DO-NOT-TOUCH.md`).

## Scope
- Package: `tools/capture`, `src/hyp/types.rs` (`IntOrStr`, lines 37–105).

## Implementation plan
1. Validate the module's premise that typed values are never written (bodies are
   captured byte-exact).
2. If it holds, replace the manual deserializer with `serde_with`
   (`PickFirst<(_, DisplayFromStr)>`) or `serde_aux`.

## Constraints / validation
- `PickFirst` re-serializes a string input as a number — acceptable only if the
   premise holds. If a test pins the write variant, **keep** the hand-rolled type
   and mark this `skipped`.
- Low payoff; safe to skip without harming the overall goal.

## Done when
Either the type is delegated with the premise validated, or it is skipped with
the reason recorded.
