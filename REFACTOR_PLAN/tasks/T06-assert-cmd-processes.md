# T06 — Process assertions → assert_cmd + predicates

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R4 (lines 186–196) — hand-built
  `(code, stdout, stderr)` tuples from `CARGO_BIN_EXE_girpr` should become
  `assert_cmd` + `predicates` chained assertions.
- Validate assumptions first: §A1 (15–16), §A4 (22–23). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tests/common/mod.rs:51–64`, `tests/cli_contract.rs:46–64`, `tests/process.rs:24–40`.
- Every call site that destructures the tuple; root `Cargo.toml` dev-dependencies.
- Locate all destructuring sites in the current tree before editing; the list above
  is a hint.

## Implementation plan
1. Identify which of the three helpers are used by tests sharing a Tokio runtime with
   the mock server. Migrate the synchronous `cli_contract.rs::run` helper first to
   `assert_cmd::Command::cargo_bin("girpr").assert().code(n).stdout(...)`.
2. Rewrite each tuple-destructuring call site as chained assertions, choosing
   `predicates` matchers that preserve the semantic check (exit code class, stdout
   plane, stderr content).
3. Keep the async `tokio::process::Command` seam in `tests/common/mod.rs` for
   mock-backed tests — `assert_cmd` is synchronous. Keep explicit
   `remove_run_lock_for_args` cleanup at the call sites that need it.
4. Run all integration roots (`contracts`, `repair`, `repair_post`,
   `artifact_repair_bridge`) and confirm no tuple helper remains.

## Constraints / validation
- No product behavior changes (§R4 lines 196–197).
- The async seam must stay for tests that share a runtime with the mock server
  (§X3-adjacent note, report lines 124–125). Do not force those tests into `assert_cmd`.
- Lock cleanup stays explicit; do not rely on assertion order for cleanup.

## Done when
All synchronous spawn helpers use `assert_cmd`/`predicates`, the async seam is intact,
and the integration suites pass. If a premise is stale or a keeper would change,
**stop** and record `skipped`.
