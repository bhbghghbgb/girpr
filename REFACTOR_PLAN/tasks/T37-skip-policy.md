# T37 — Shared precondition/skip policy

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F42 (lines 578–585) — two different
  "host refused a fixture" skip policies; candidate boundary is one shared precondition/skip
  policy. Confidence high.
- Assumptions to validate: §A10 (98–105), §A5 (57–67), §A14 (129–135), §F42 (578–585).

## Scope
- `crates/util-path/src/lib.rs` (`FixtureRefusal {Host, Injected}`, `skip_fixture_refusal`,
  `required_env` switch, load-bearing `SKIP precondition:` wording) and
  `crates/tempdir/src/test_support.rs` (`skip_if_no_link!`/`skip_if_case_insensitive!`,
  plain `skip: this platform refused …` wording, no required-case switch). Locate before editing.

## Implementation plan
1. Validate §F42: confirm the two policies and their visible skip-line wording.
2. If attempted, define a shared precondition/skip policy parameterized by wording and the
   required-case switch; keep each host-detection rule local.
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If a shared policy would turn an injected refusal into a host skip (false coverage), stop.

## Constraints / validation
- High-risk (§F42): nextest overrides key on visible skip lines (§A10); an injected refusal
   reported as a host skip would claim coverage a passing test never had. Preserve wording
   byte-for-byte (§A5).
- Do not change `.config/nextest.toml` filters. Move tests only with filter updates (§A14).

## Done when
A shared policy is used with all skip wording and switches intact, or a validated skip with
the one-line reason is recorded.
