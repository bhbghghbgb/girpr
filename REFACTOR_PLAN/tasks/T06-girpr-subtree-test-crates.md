# T06 — Move the two girpr-only test crates

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F4 (lines 104–114) — `test-fixtures` and
  `test-support` are girpr-only; if `crates/` is to mean "shared only", they belong in the
  same girpr-owned subtree as T04/T05.
- §F1 rows for `test-fixtures` (75) and `test-support` (76) — consumers are the root
  package (dev) and other girpr-only crates.
- §F6 (lines 126–129) — `girpr-test-support` depends on the root `girpr` package as a
  normal (test-only inversion) dependency; it cannot be placed where the product depends
  on it in a shipped build. The inversion must survive.
- §F8 (lines 159–162) — the root integration tests shim these two crates through
  `tests/common/mod.rs`; the shims are thin and the crate identities behind them are what
  change.
- §F7 (lines 141–148) — keep the workspace lint table on both crates.
- Use the girpr subtree path recorded in `STATE.md` Persistent context by T04. If it is
  absent, **stop**.
- Assumptions to validate: §A1 (8–13), §A3 (19–23, dev-deps count), §A5 (30–32), §A6
  (33–35). Keep items: §F5 (116–123), §F9 (164–177), `guidance/DO-NOT-TOUCH.md`.

## Scope
- Move `crates/test-fixtures/` and `crates/test-support/` into the subtree chosen in T04.
- Root `Cargo.toml`: workspace `members`, `dev-dependencies` on both crates.
- Their manifests: `girpr-test-support` depends on the root package (path to workspace
  root), on `test-fixtures`, and on crates moved in T04/T05; `test-fixtures` depends on
  `fetch` (moved in T05) and shared crates. Repoint all paths for the new depth.
- Root `tests/common/mod.rs` and the nested `tests/` shims that name these crates.
- Path text: root `README.md` crate table and prose about the thin shims,
  `.config/nextest.toml` comments, source doc comments. Locate occurrences before editing.

## Implementation plan
1. Validate §F4/§F6/§F1: confirm the two crates are girpr-only and that the test-only
   inversion to the root package still exists. If a second program family consumes either,
   stop (§A6, §F5).
2. Move the two directories without renaming them; keep the workspace lint table (§F7).
3. Repoint `girpr-test-support`'s dependency on the root package to the workspace root from
   the new location, and every other path dep to its new relative location (subtree peers
   for crates moved in T04/T05, `crates/` for shared crates). Keep it a **normal**
   dependency — do not turn it into a dev-dependency or move it under the product crate.
4. Repoint `test-fixtures` deps (including `fetch` in the subtree) without changing
   features or versions.
5. Update the root dev-dependencies and workspace `members`, then the `tests/common/mod.rs`
   shims: keep each call site's `mod common;`, `common::helper(..)` and `use common::*`
   spelling; change only crate names in `pub use` if any and the path text in doc comments.
6. Update all §F8 seams: README table/prose, nextest comment path text (package names do
   not change), doc comments.
7. Verify a locked build and that all four root integration test roots still compile and
   pass; `Cargo.lock` names are unchanged for this task.
8. Deliver in several commits if useful: move + manifests, shims/consumers, docs.

## Constraints / validation
- Preserve the test-only inversion (§F6) and the facade-free / facade-coupled split
  between `test-fixtures` and `test-support`; do not merge them.
- Do not move tests between binaries or change nextest filters/FQNs (§F8).
- Keep nextest one-process-per-test isolation; the shared support must stay a library
  reached through thin shims.
- Do not move or rename any shared crate or gircap crate (§F5, §F9).

## Done when
Both crates live in the recorded subtree with unchanged names/lints, the root-package
inversion survives, the root test shims compile unchanged at call sites, all §F8 seams are
updated and the test suite passes. If a premise is stale or the inversion would be
collapsed, **stop** and record `skipped` with a one-line reason.
