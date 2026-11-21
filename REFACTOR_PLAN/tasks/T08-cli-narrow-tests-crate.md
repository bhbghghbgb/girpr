# T08 — CLI / config crate for narrow tests

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F22 (lines 285–291) —
  `tests/cli_contract.rs:38` needs only `girpr::Args`, `tests/process.rs:9-13`
  drives the binary; if cli/config/report were a crate, CLI-contract tests could
  link that crate instead of the whole product lib. The shipped
  `pub mod test_support` blocks simple `cfg(test)` gating.
- Related: §F21 (276–283) on `src/lib.rs` and `src/repair/mod.rs` as the biggest
  rebuild triggers; §F30 (365–375) on deliberate binary coupling.
- Assumptions to validate: §A2 (27–33), §A6 (55–61), §A7 (63–70).

## Scope
- `src/cli.rs`, `src/config.rs`, `src/lib.rs:111-125` (`pub mod test_support`).
- Narrow tests: `tests/cli_contract.rs`, `tests/process.rs`.
- Existing `girpr-cli` / `girpr-model` crates (§A3) — reuse, do not duplicate.
- Locate edges in the current tree before editing.

## Implementation plan
1. Validate §F22: confirm exactly what the narrow tests need and whether a
   cli/config crate can expose it without pulling the whole lib. Keep binary
   coupling (`CARGO_BIN_EXE_girpr`) where it is deliberate (§F30).
2. Extract the cli/config surface into a crate and point the narrow tests at it
   directly (§A2), so a repair-internal edit no longer forces those test
   binaries to compile and link the whole product lib.
3. Keep the shipped `pub mod test_support` working for the tests that use it
   (§F22) — it cannot simply be `cfg(test)`-gated; promote seams via the §A7
   pattern if needed.
4. No change to CLI bytes: help text, defaults, flag names, error strings and
   exit codes stay identical (§A6).
5. Update nextest filters (§F31) and any gircap doc tables if a leaf is added
   (§A5/§F39).

## Constraints / validation
- §A2 payoff: a narrow test must actually link the smaller crate, not still go
   through the facade. If the facade remains an ancestor, stop.
- CLI/help/error bytes are contracts (§A6).
- Do not break the deliberate binary-driven tests (§F30).

## Done when
CLI-contract and process tests link the smaller crate directly, CLI bytes are
unchanged, the shipped `test_support` still works, and both suites pass.
