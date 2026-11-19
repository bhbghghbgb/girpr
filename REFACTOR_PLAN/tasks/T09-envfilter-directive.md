# T09 — EnvFilter spec via `add_directive`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §C1 (lines 460–481).

## Scope
- Package: `crates/log` (`src/plan.rs` `console_filter` / `own_console_directive`,
  `src/filter.rs`). Consumers: `src/logging.rs`, `tools/sync/src/logging.rs`,
  `tools/capture/src/logging` (if the public return type changes).

## Implementation plan
1. Build the filter with `EnvFilter::builder().with_env_var("RUST_LOG").parse()`
   then `.add_directive(Directive::from_str(&own)?)`, appending the project
   directive last so "last wins" is preserved.
2. If the public return type moves from `String` to `EnvFilter`, update all three
   consumers.

## Constraints / validation
- Do **not** use `with_default_directive()` (it is target-less) and do **not**
  use `parse_lossy()` (it silently drops bad directives); keep the
  warn-and-fall-back path for a broken spec.
- Tests named in §C1 pin "target beats global" and "last wins" — keep them.

## Done when
The string-concatenated EnvFilter spec is gone and the precedence tests pass.
