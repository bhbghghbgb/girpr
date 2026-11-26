# T19 — path-shape normalization → relative-path

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-8 (lines 507–518) — the lexical
  `split('/')` normalization in `path-shape` may delegate component classification to
  `relative-path`'s `RelativePath`/`normalize()`, while all caller-supplied
  device/colon/control/trailing-dot rules and the fixed first-violation order stay local.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A12 (48–50). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/path-shape/src/lib.rs` — `validate` (`:99–183`) and all tests (`:223–340`),
  especially `strict_reports_the_first_violation_in_the_fixed_order` (`:258–269`) and
  `normalizing_returns_the_clean_slash_form` (`:285–303`).
- `crates/path-shape/Cargo.toml`; callers of `girpr-path-shape` in both products.

## Implementation plan
1. **Validation gate:** check whether `relative-path::normalize()` preserves the exact
   first-violation order and rejects (rather than collapses) `..` components. If it
   cannot, **stop** and keep the current loop — this is the report's stated fallback
   (§LEAF-8 line 518).
2. If the gate passes, use `RelativePath` only for the `/`-separated component
   classification; keep every device/colon/control/trailing-dot rule and the exact
   `ShapeProblem` order in `validate`.
3. Keep all path-shape tests unchanged, in particular the fixed-order and clean-slash
   form tests (§LEAF-8 lines 514–517). Add a `..` case if the library now rejects it.
4. Verify both product consumers still build and behave identically.

## Constraints / validation
- Partial delegation only — do not let the library own policy the report says stays local.
- `relative-path 2.0.3` is not currently locked; verify a resolved build.
- Behavior changes are allowed (§A4) but not here: the pinned violation order is a
  contract unless a task explicitly changes it.

## Done when
Normalization uses the library for component classification (or the loop is deliberately
retained after the gate), all pinned tests pass, and both products build. If the gate
fails, record `skipped` with a one-line reason.
