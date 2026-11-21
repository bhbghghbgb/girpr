# T07 — Pin the stable toolchain to an exact release

## Goal / pointers
- `docs/10-test-optimization-research.md` §F3 (lines 162–165) — `rust-toolchain.toml`
  pins `channel = "stable"`, not an exact release; every stable release invalidates all
  cached artifacts, including third-party crates. Fix: pin `channel = "1.xx.0"` and bump
  deliberately; not an MSRV floor. Benefit Med, effort S.
- Assumption to validate: §A8 (97–99), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- `rust-toolchain.toml` and any documentation that describes the toolchain policy.

## Implementation plan
1. Validate §A8: confirm the channel is `stable` and check what exact toolchain is
   currently in use so the pin does not itself trigger an unintended change.
2. Pin to that exact release (channel with an explicit version), leaving components and
   targets as-is.
3. Confirm the workspace builds and tests with the pinned toolchain; update any doc
   that says "stable" as the policy.

## Constraints / validation
- This is not an MSRV floor; the workspace still refuses to support old compilers. Do
  not add or imply an MSRV guarantee.
- The pin invalidates caches once by design; that is the trade-off being accepted.
  Record the result as a pointer to §F3/§A8 rather than a dated change log.

## Done when
`rust-toolchain.toml` names an exact release, docs agree with the policy, and the
workspace builds and tests with the pinned toolchain.
