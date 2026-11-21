# T06 — Trial rust-lld for the test profile

## Goal / pointers
- `docs/10-test-optimization-research.md` §F4 (lines 167–172) — heavy test binaries cost
  6–8 s dominated by the MSVC link even with `codegen-units=1`, `debug=0`,
  `incremental=false`; `rust-lld.exe` ships in the toolchain. Fix: trial
  `[target.x86_64-pc-windows-msvc] linker = "rust-lld"` in a scratch config, measure
  clean `cargo nextest run` wall time, adopt only if all targets link.
- Assumptions to validate: §A7 (91–95), §A12 (118–121), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- Workspace `Cargo.toml` (or `.cargo/config.toml` if that is how target linker settings
  are expressed in this tree) and a scratch measurement setup. No test code changes.

## Implementation plan
1. Validate §F4: confirm current codegen/debug/incremental settings and record a
   clean-build/wall baseline.
2. Trial the rust-lld linker in a scratch config; build and link every workspace test
   target, running the full suite to catch link-time-only failures.
3. Measure clean `cargo nextest run` wall time before/after. If all targets link and
   wall time improves, adopt the setting in version control. If any target fails to
   link or time does not improve, revert the scratch change and record the result as a
   skip/revert with the reason.
4. Keep the change minimal — one config line plus any comment explaining the trial.

## Constraints / validation
- This is explicitly a "do not assume it works" finding: workspace-wide risk. A failed
  trial must be reverted, not papered over.
- Host-dependent (§A7): record the result as a pointer to §F4/§A7, not as a universal
  guarantee; CI should validate independently.
- Do not change `profile.test` debug/codegen settings as part of this task.

## Done when
Either the linker setting is adopted with a measured improvement and all targets
linking, or the scratch change is fully reverted and the task records why; no
half-applied config remains.
