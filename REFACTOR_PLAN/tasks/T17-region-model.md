# T17 — Region model leaf (`girpr-model::region`)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F25 (lines 343–352) — region
  identity, ids and base URLs duplicated.
- Assumptions to validate: §A9 (78–81), §A2 (21–27), §A1 (13–19).
- Related: T12 (`girpr-cli` owns clap-facing helpers).

## Scope
- Candidate shared crate `crates/model` (`girpr-model`), module `region`, with
  clap behind an optional feature (§F25).
- Report names: `src/biz.rs` (`Biz`, `as_str`, `endpoints`, `channel_tuple`),
  `tools/capture/src/cli/mod.rs` (clap `ValueEnum`), `tools/capture/src/biz/mod.rs`
  (`Biz::resolve`, `Endpoints<'a>` override seam), `src/hyp.rs` (base URLs,
  `DOWNLOAD_MODE_CHUNK`, deprecated-channel rule),
  `tools/capture/src/hyp/types.rs`. Locate before editing.

## Implementation plan
1. Validate §F25: `Biz` values, the four production base URLs, the
   channel-only-on-deprecated rule and `DOWNLOAD_MODE_CHUNK` are duplicated; both
   test suites pin the values.
2. Create `girpr-model::region` holding the region values, ids and base URLs;
   put the clap `ValueEnum` derive behind an optional `clap` feature so a
   non-clap consumer does not pull clap in.
3. Migrate girpr and capture to the shared values. Keep capture's
   `Endpoints<'a>` override seam local (§F25) and keep each tool's resolve policy
   at its call site.
4. Update §A2 doc tables because gircap gains a new leaf dep.

## Constraints / validation
- Region strings, base URLs and channel rules are byte/contract stable and
  pinned by tests (§A3, §F25); no value change.
- No shared exit codes or report semantics (§A9); no program logic in the leaf
  (§A1, §A2).
- If sharing forces capture to drop its override seam, stop and raise a follow-up.

## Done when
Region values/URLs come from `girpr-model::region`, clap is feature-gated,
capture's override seam stays local, and both region test suites pass.
