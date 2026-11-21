# T09 — Narrow the small rebuild-locality edges

## Goal / pointers
- `docs/10-test-optimization-research.md` §F6 bullets 2–4 (lines 192–203):
  - `gircap-hash` depends on `gircap-http` solely for `impl BodyFile`; move the
    one-method trait into `girpr-digest` (already a dep of both) or a tiny leaf.
  - `girpr-log` bundles `logs_of_async` (tokio) into `test-support`; split sync/async
    support or move the async helper into its sole consumer.
  - `girpr-report` compiles clap for one `#[derive(clap::ValueEnum)]`; gate it behind a
    feature enabled by consumers that need the CLI flag type.
- Assumptions to validate: §A2 (54–59), §A3 (61–65), §A11 (112–116), §A10 (108–110),
  §6 (503–510). Guidance: `DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/hash/{Cargo.toml,src/lib.rs}`, `tools/capture/http/src/lib.rs`,
  `crates/digest/`, `crates/log/{Cargo.toml,src/test_support.rs}`,
  `crates/report/{Cargo.toml,src/record.rs}`, and their consumers.
- Locate the exact trait/derive/feature sites in the current tree before editing.

## Implementation plan
1. Validate each of the three premises independently: confirm the dependency exists
   only for the named reason and that removing it removes a rebuild edge for at least
   one high-level consumer (§A3). Drop any sub-item that does not pay off.
2. Move the `BodyFile` trait to a crate both sides already depend on, or a tiny leaf,
   and repoint the impl; verify `gircap-hash` no longer depends on `gircap-http`.
3. Split `girpr-log` test-support into sync/async surfaces (or relocate `logs_of_async`)
   so util's sync-only tests no longer compile tokio.
4. Gate the clap `ValueEnum` derive in `girpr-report` behind a feature enabled only by
   consumers that use the CLI flag type; verify clap leaves the default build.
5. Deliver in separate commits per sub-item; update any doc tables/manifest comments
   and nextest filters if a crate boundary or test moves (§A10).

## Constraints / validation
- Each sub-item must be a turnaround win with a direct consumer (§A3); a re-exporting
  facade or an always-on feature is not a fix.
- gircap independence and byte contracts (§6) must be preserved. If a trait move adds a
  gircap dependency on a program crate, stop.
- Workspace wall-clock may not change (§A11); measure isolated `-p` builds for the wins.

## Done when
Each validated sub-item is applied with its dependency edge removed, clap/tokio no
longer compile in the named default builds, and the suite and dependency-tree checks
pass; invalid sub-items are recorded as skipped with reasons.
