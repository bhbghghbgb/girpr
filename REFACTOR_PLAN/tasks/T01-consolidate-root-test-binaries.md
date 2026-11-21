# T01 — Consolidate root integration test binaries

## Goal / pointers
- `docs/10-test-optimization-research.md` §F1 (lines 131–143) — root `tests/` has 14
  `*.rs` targets, 11 of which inline `mod common;` and link the fixture tree; each is a
  full link of the workspace tree (~98 s aggregate clean build). Fix: `autotests = false`
  plus 3–4 `[[test]]` roots that `#[path]`-include the existing files; one crate-root
  `common` module with `use crate::common::*;`.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §A2 (54–59), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- Root `Cargo.toml` test-target declarations, the 14 files under `tests/`,
  `tests/common/mod.rs` (and its `#[path]` includes), and
  `.config/nextest.toml` inventory comments/filters.
- Existing directory-target pattern (`#[path]`-include), e.g.
  `tools/sync/tests/why/main.rs` as precedent. Locate every `mod common;` and
  `#[path]` site in the current tree before editing.

## Implementation plan
1. Validate §F1: list all root test targets and confirm which link the fixture tree
   and which `mod common;`-inline it; measure clean-build wall time before.
2. Set `autotests = false` in root `Cargo.toml` and declare 3–4 `[[test]]` roots that
   `#[path]`-include the existing test files, grouping by shared-link affinity.
3. Change per-file `mod common;` to a single crate-root module and update each file's
   imports to `use crate::common::*;`; keep per-file `#[path]` resolution working.
4. Update `.config/nextest.toml` binary names and any fully-qualified test-path filters
   affected by the new binary grouping (§A10).
5. Re-measure clean-build wall time and run the full nextest suite; deliver in a few
   commits if the target declarations and the module rewrite are easier to review apart.

## Constraints / validation
- Isolation under nextest is per *test*, not per binary, so a shared `common` module
  does not weaken `set_var`-based assumptions (§A1). Verify cases that were relying on
  process-local globals still pass.
- Keep every test's semantic assertion and name; do not drop tests to make the binary
  count smaller.
- Pinned nextest filters must be updated in the same change (§A10). No byte contracts
  change.
- Do not make gircap tests depend on the root fixture tree (gircap independence).

## Done when
Root test targets number 3–4, every original test file is still compiled and run,
`common` is a single crate-root module, nextest filters match, the clean build no
longer links 14 full trees, and measured clean-build wall time does not regress.
