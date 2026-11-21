# T10 — Gate the fully-ignored `bundle_shapes` binary

## Goal / pointers
- `docs/10-test-optimization-research.md` §F8 (lines 215–219) — all three tests in
  `tools/capture/tests/bundle_shapes.rs` are `#[ignore]`d and hand-run, yet the binary
  is compiled. Fix: `[[test]] name = "bundle_shapes" required-features = ["manual-tests"]`
  with an empty feature and a crate-level cfg, documenting the manual command.
- Assumptions to validate: §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- `tools/capture/Cargo.toml` (`[[test]]` declarations and features),
  `tools/capture/tests/bundle_shapes.rs`, and any doc that describes the manual test
  command.

## Implementation plan
1. Validate §F8: confirm all tests in the target are ignored and that no nextest filter
   relies on the binary being built.
2. Add the `manual-tests` feature (empty) and gate the test target with
   `required-features`; add a crate-level cfg guard so the file is inert without it.
3. Document the manual command beside the target or in the crate README.
4. Verify a normal workspace build no longer compiles the binary and that the manual
   command still runs it.

## Constraints / validation
- Do not remove the ignored tests — they are hand-run checks, not dead code.
- The feature must be empty (no new dependencies). Do not change the tests' behavior.
- Preserve any pinned nextest filter or update it in the same change (§A10).

## Done when
The binary compiles only with `manual-tests`, the manual command is documented and
works, and a normal build no longer links it.
