# T33 — Error `Display` impls → `thiserror`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §F8 (lines 708–720).

## Scope
- Package: `tools/capture`, `src/http/error.rs` (`ApiError`),
  `src/cli/mod.rs` (`UsageError`), `src/failure/mod.rs` (`Failure`).

## Implementation plan
1. Derive `thiserror::Error` with `#[error("…")]` format strings copied verbatim
   from the manual `Display`/`Error` impls.
2. Keep `ApiError::exit_code()`, `url()` and `Failure::summary` hand-written.

## Constraints / validation
- No message change if format strings are copied verbatim; skip if the derive
   cannot preserve messages exactly.
- Tests named in §F8: `failure/mod.rs` summary-shape test and `http/tests.rs`
   message-fragment assertions. Marginal payoff (§F8).

## Done when
The manual impls are replaced by derives with identical messages and tests pass.
