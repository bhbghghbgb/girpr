# T36 - D10: name every exit code

Report: `../../OPTIMIZATIONS_REPORT.md` lines 813-830 (D10), and the exit-code
contract at lines 32-63.

Dependencies: T22 (ExitCode return path).

## Goal

`RunFailure` is typed, but `repair::run`, `execute_phase`, `plan_phase`,
`finish`, `apply_run_flags`, and `check::verify_plan` return or compare bare
`i32`s (`0`, `1`, `2`, `3`, `4`, `5`). The exit-code contract is the product's
automation API. Replace the magic integers with one enum with named variants,
`From<RunFailure>` for the fatal path, and one conversion to `i32` at the
process boundary.

## Where to start

- `src/config.rs`: `RunFailure` (around line 67), `usage`/`metadata`/`write`.
- `src/repair/mod.rs`: `run` (around line 82), `PlanOutcome` returns
  (around lines 386, 432), `execute_phase` returns (around lines 544, 558,
  592, 601, 615), `finish` (around line 698).
- `src/repair/phases/check.rs`: the `0`/`4` return.
- `src/main.rs`: the final `ExitCode` conversion.
- README / docs: the exit-code table (`0` success, `1` usage, `2`
  metadata/network, `3` write/verify, `4` check failed, `5` limit).

## Steps

1. Add `pub enum ExitCode { Success = 0, Usage = 1, Metadata = 2, Write = 3,
   CheckFailed = 4, Limit = 5 }` (put it in `config.rs` or `main.rs`; the
   report suggests keeping the `i32` conversion in `main`). Derive
   `Clone, Copy, Debug, PartialEq, Eq`. Add `fn code(self) -> i32` or
   `impl From<ExitCode> for i32`, and a `From<RunFailure>` that reads
   `failure.exit_code`.
2. Change `RunFailure.exit_code` to `ExitCode` (or keep an `i32` internally and
   make the constructors use variants; prefer the enum). Update its `Display`,
   `main`'s fatal log (`exit={}`, use `.code()`), and every construction.
3. Change `repair::run` to return `Result<(Summary, ExitCode), RunFailure>`,
   and thread the enum through `plan_phase`, `execute_phase`, `PlanOutcome`,
   `finish`, and the check-only branch. `finish` maps `PostOutcome` to
   `ExitCode`.
4. Convert once in `main`: `ExitCode::from(code)` (the std one) or
   `std::process::ExitCode::from(code.code() as u8)`. Do not keep a second
   integer conversion in a phase.
5. Update `check::verify_plan` to return/count as today, but the caller chooses
   `ExitCode::CheckFailed` vs `Success`; no `if bad == 0 { 0 } else { 4 }`.
6. Keep the documented codes unchanged. Update the README table to name the
   variants if that helps; do not change the numbers.
7. Search for remaining bare exit integers in the repair paths:
   `rg -n "\\b[0-5]\\b" src/repair src/config.rs src/main.rs` and justify each
   remaining one (array index, count, etc.). Do not replace unrelated numbers.

## Tests

- Every existing process-level exit-code assertion still passes.
- A unit test maps each `RunFailure` constructor to the expected `ExitCode`.
- `finish` maps `PostOutcome::Ok`/`DeletedNothing`/`DeleteCeiling` to the same
  codes as before.
- `cargo test -p girpr` passes.

## Acceptance

- One named enum for the process exit codes.
- No bare `0/1/2/3/4/5` exit codes in the repair pipeline.
- Numbers and behavior are unchanged.
- The i32 conversion is at the process boundary only.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(config): name the process exit codes`
2. `refactor(repair): return the exit-code enum through the phases`
3. `test: pin the exit-code mapping`

## Notes

- Keep `RunFailure` as the error type; the enum is its code, not a replacement
  for the error.
- Do not change clap's `--help`/`--version` `0` or the pre-logging usage `1`;
  those are process-boundary decisions.
