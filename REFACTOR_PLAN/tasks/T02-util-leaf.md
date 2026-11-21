# T02 — `girpr-util` leaf crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F14 (lines 234–239) —
  `src/util/{mod,atomic,counter,ini,md5,path}.rs` (~3,569 production lines) is
  the widest stable low-level surface in the product with no product-module
  dependencies outside `util`; today a change in repair/report/cli recompiles
  all of it.
- Assumptions to validate: §A2 (27–33), §A4 (40–46), §A1 (19–25), §A13 (108–114).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (INI/lock/walker keepers).

## Scope
- `src/util/mod.rs:22-36` and the files it declares: `atomic.rs`, `counter.rs`,
  `ini.rs`, `md5.rs`, `path.rs`.
- Test-only `junction_support.rs` (841) and `symlink_support.rs` (375) —
  deliberately unit-test-only per §F40; keep them out of the shipped leaf or
  feature-gate them.
- External deps named in §F14: std / anyhow / serde_json / md-5 / girpr-digest /
  tempfile (+ dev tempdir).
- Locate each in the current tree before editing.

## Implementation plan
1. Validate §F12/§F14: confirm no `use crate::` edge from `util` to a product
   module, and confirm the true external dep set. Note that `md5.rs` already
   re-exports from `girpr-digest` (prior extraction) — do not re-create digest
   primitives here.
2. Create `crates/util` (`girpr-util`) per §A4 holding the production `util`
   mechanics. Keep the test-only junction/symlink support out of the shipped
   target (§A11, §F20).
3. Apply the payoff test (§A2): make the high-level product code depend on
   `girpr-util` directly so a change in repair/report/cli no longer recompiles
   `util`. If the product facade merely re-exports the crate and stays an
   ancestor of every test binary, the split is cosmetic — stop.
4. Update nextest filters that name `util::ini::tests::…` or
   `util::path::tests::…` (§F31, `.config/nextest.toml:106-214`) when tests move.
5. If gircap consumes any part of this leaf, update the §A5/§F39 doc tables.

## Constraints / validation
- INI parsers, lock policies and walker policies are deliberate keepers
  (§A13, `DO-NOT-TOUCH.md`); do not "fix" or merge them while moving.
- No byte/behaviour change in any artifact (§A6).
- Preserve the test-only seam behaviour of junction/symlink support; do not
  promote it into shipped code.
- Keep the crate logic-free — no product data model types.

## Done when
`girpr-util` holds the production `util` mechanics, high-level product code
depends on it directly, the test-only support stays out of shipped builds, and
all digest/INI/path/atomic tests and pinned nextest filters pass.
