# T01 — Merge `cli_contract.rs` helper, layout and parser self-tests

Priority P0 · root product integration tests · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L85–L94` (§B.1, first
  three clusters).
* Report verdict/context: `docs/08-test-suite-reduction-audit.md:L12–L35`
  (headline verdict — no large block of useless tests; merge pure-function cases).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T01-cli-contract-self-tests.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/cli_contract.rs` devotes ~19 tests to testing its *own* parsing and
assertion helpers rather than product behavior. Collapse them without losing any
case:

* Help-entry / option-body / indent self-tests (12 tests at lines 201, 213, 228,
  260, 289, 308, 340, 360, 381, 399, 568, 589) → 1–2 table-driven tests covering
  LF/CRLF, short alias, boundary, panic text and indent arithmetic.
* Real-help layout (clap dependency behavior) tests (476, 511, 540) → 1
  parametrized test over the indent columns.
* `text_records` parser self-tests (1698, 1719, 1742, 1763) → 1 test, using
  `catch_unwind` for the `#[should_panic]` case.

Expected ~15–16 tests removed, **no coverage lost**.

## Implementation plan

Code anchors from the report (validate against current code before editing;
line numbers are from the report snapshot):

* `tests/cli_contract.rs:201–589` — the help/indent self-test clusters.
* `tests/cli_contract.rs:476/511/540` — real-help layout.
* `tests/cli_contract.rs:1698–1763` — `text_records` parser self-tests.

Possible commits:

1. Replace the help/indent self-tests with one or two tables; keep every case as
   a row (LF/CRLF, short alias, boundary, panic text, indent arithmetic).
2. Parametrize the real-help layout tests over indent columns.
3. Collapse the `text_records` parser tests into one table with a `catch_unwind`
   row for the panic case.

## Acceptance criteria

* Every original case is still asserted; only the test *functions* are
  consolidated.
* No product code touched; `cli_contract` suite passes.
* Report follow-up ideas without implementing them.

## Guardrails

* Validate first: if a cluster no longer exists or is already merged, stop and
  mark `skipped` with a note rather than forcing a change.
* Do not touch the log-file process tests or the golden self-test removal —
  those belong to T13.
* Test-only change. Do not change what is asserted about product behavior.
