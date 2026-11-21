# T19 — `girpr-log` split payoff assessment

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F35 (lines 412–417) — the
  `girpr-log` split has low fan-out payoff: `install.rs` (190) pulls
  `tracing-appender`/`tracing-subscriber`, `level.rs` (120) is pure std, but all
  three programs consume level + filter + plan + install. The genuinely
  isolatable `test-support` is already feature-gated.
- Assumptions to validate: §A2 (27–33), §A7 (63–70), §F37 (424–430).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/log/src/{lib,install,level,filter,plan,test_support}.rs`,
  `crates/log/Cargo.toml:15-20`.
- Consumer manifests: root `Cargo.toml:46,129`, tools/sync and tools/capture.
- Locate current feature/dep edges before editing.

## Implementation plan
1. This is primarily an **assessment**. Validate §F35 against current code: map
   which log pieces each program consumes and whether any consumer uses a pure
   subset. Apply §A2: if every consumer needs the whole crate, a split adds a
   crate without removing a rebuild edge — record the reason and mark the task
   `skipped`, do not force a cosmetic split.
2. If a genuine subset exists (e.g. a pure-std `level` half consumed by a
   program that does not install tracing), split that half out and point the
   consumer at it directly (§A2). Keep `test-support` feature-gated (§A7).
3. Update §A5/§F39 doc tables and nextest filters only if a split is actually
   made.

## Constraints / validation
- No cost-free split: §A2 must pass, otherwise skip with a one-line reason.
- Do not change log output bytes or the pinned `format_timestamp` (§A6).
- Do not collapse the already feature-gated `test-support` seam (§A7).
- §F37 leaves where further splitting won't help are excluded.

## Done when
Either a log half is split out with a verified direct consumer and all log tests
pass, or the task is recorded as `skipped` with the §A2 reason for why no
rebuild edge shrinks.
