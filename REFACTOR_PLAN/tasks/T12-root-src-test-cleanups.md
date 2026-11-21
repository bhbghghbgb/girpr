# T12 — Root `src/` unit-test cleanups

## Goal / pointers
- `docs/10-test-optimization-research.md`:
  - §F12 (lines 251–256) — eight hand-rolled current-thread runtimes in repair tests;
    use `#[tokio::test(flavor = "current_thread")]` except the one case that asserts
    cross-thread progress.
  - §F13 (lines 258–262) — a 12-way repeated audio-move fixture in `post.rs`; one
    `audio_cache_fixture()` plus a table test for the pure alias cases.
  - §F15 (lines 271–274) — 10 s/5 s hang guards never fire on a passing run; lower to
    2 s for faster failure feedback.
  - §F16 (lines 276–280) — clap-arg boilerplate in five pure CLI tests; a `parse(extra)`
    helper plus one table test, combined with the CLI leaf work in T08 if applicable.
- Assumptions to validate: §A1 (46–52), §A6 (82–89), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/phases/check.rs`, `post.rs`, `src/repair/mod.rs`, `src/cli.rs`, and the
  guard sites in `manifest.rs` / `file/mod.rs`. Locate all named sites in the current
  tree before editing.

## Implementation plan
1. Validate each sub-item in the current tree; skip any that has already been done.
2. Replace the hand-rolled runtimes with `#[tokio::test]` where semantics match; leave
   the deliberate cross-thread assertion case on a manual runtime.
3. Extract the audio-cache fixture and convert the pure alias cases to a labelled table
   test, preserving assertion messages.
4. Lower the hang guards to 2 s only where they wake on notify/channel; leave any guard
   whose timing is load-bearing unchanged.
5. Add the CLI `parse(extra)` helper and table test; coordinate with T08 only if the CLI
   leaf move landed — otherwise keep it local.
6. Deliver in separate commits per sub-item (this is a batch of small changes).

## Constraints / validation
- Do not add sleeps (§A6). The guards must still wake on notify/channel; lowering them
  must not make a legitimately slow CI run flaky — validate on a loaded run.
- Preserve every assertion and its message (§6); no test named in nextest filters may be
  silently removed (§A10).
- No production behavior or bytes change; these are test-only cleanups.

## Done when
All validated sub-items are applied in commits, the one cross-thread test still uses a
manual runtime, no sleeps were added, guards still wake on notify, and the root unit
suite passes with unchanged assertions.
