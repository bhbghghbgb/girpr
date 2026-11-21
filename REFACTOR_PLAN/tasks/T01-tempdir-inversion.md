# T01 — Break the `girpr-tempdir → girsync` inversion

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F3 (lines 140–152) and §F36
  (lines 419–422) — the dominant fan-out fact: `girpr-tempdir` carries 707 lines
  of `test_support` that depend on girsync, dragging a whole program into four
  dev graphs.
- Assumptions to validate: §A2 (27–33), §A7 (63–70), §A11 (96–101), §A5 (48–53).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`, `CONVENTIONS.md`.

## Scope
- `crates/tempdir/Cargo.toml` (normal deps on girsync + serde_json at `:24-25`),
  `crates/tempdir/src/lib.rs` (`pub mod test_support` at `:70-81`),
  `crates/tempdir/src/test_support.rs` (707 lines, builds girsync
  option/cache/planner types).
- The four dev graphs: root `girpr`, `gircap`, `girsync`, `sled2redb`.
- Compare `girpr-fs-identity`'s own scratch-directory tests
  (`crates/fs-identity/src/lib.rs:192-193`) and `girpr-report-core`'s dev-only
  serde_json (`crates/report-core/Cargo.toml:22-25`) as precedent.
- Locate each in the current tree before editing.

## Implementation plan
1. Validate §F3 against the current manifests: confirm the girsync/serde_json
   deps exist only to compile `pub mod test_support`, and map exactly which
   girsync types the helper constructs.
2. Separate the 360-line `TempDir` policy from the 707-line program-coupled
   helper. The helper must stop pulling girsync into the dev graphs of programs
   that do not otherwise depend on girsync — e.g. move it to a dedicated
   logic-free leaf crate, gate it behind a feature that consumers opt into, or
   invert it so girsync supplies the adapter. Prefer the option that removes the
   rebuild edge rather than merely duplicating the code.
3. Update the dev-dependency declarations of the four programs so their test
   builds no longer inherit girsync transitively through tempdir (root and
   gircap keep direct girsync only if they already depend on it for other
   reasons — verify).
4. If gircap gains or loses a leaf dependency, update the §A5/§F39 doc tables
   in the same work (`tools/capture/docs/00-spec.md:42`,
   `tools/capture/Cargo.toml:1-44`, `README.md:44-45`, `README.md:390-422`,
   `Cargo.toml:17`).
5. Update any nextest filters that reference moved tests (§A7, §F31).

## Constraints / validation
- The payoff test (§A2): a change to log/fs-names/fs-identity/digest/filter/
  report-core must no longer rebuild girsync → tempdir → girpr/gircap/sled2redb
  test builds. If the edge remains, the split has not paid off — stop.
- Preserve the `TempDir` policy behaviour and the test-support API used by the
  four programs (§A6 byte/behaviour contracts still apply).
- Do not let a shared helper make one program's test depend on another's (§A5).
- If a single crate cannot hold both concerns without the inversion, prefer two
  crates over a feature that is always on in the dev graphs.

## Done when
`girpr-tempdir`'s shipped/test builds no longer depend on girsync, the four
programs' test builds no longer pull girsync through tempdir, and the full test
suite passes with unchanged test-support behaviour.
