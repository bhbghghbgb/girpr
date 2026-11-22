# T05 — Move the mid-layer girpr-only product crates

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F4 (lines 104–114) — these are girpr-only
  build-time split leaves; if `crates/` is to mean "shared only", they belong in the same
  girpr-owned subtree as T04.
- §F1 rows for `chunk-verify` (69), `repair-plan` (72), `lock` (71), `fetch` (70),
  `report` (74) — their consumers are the root package and other girpr-only crates.
- §F6 (lines 130–131) — `girpr-repair-plan` depends on `girpr-fetch`; the edge stays inside
  one project and must survive the move.
- §F6 (lines 137–139) — `girpr-retry`, `girpr-log`, `girpr-time`, `girpr-digest` and
  `girpr-fs-identity` are shared dependencies of moving crates and stay in `crates/`.
- §F7 (lines 141–148) — keep `[lints] workspace = true` on all moved crates.
- §A7 (lines 36–39) and §F8 (lines 150–162) — workspace `members`, path deps,
  `Cargo.lock` (names unchanged here), README table/prose and doc-comment path text.
- Use the girpr subtree path recorded in `STATE.md` Persistent context by T04. If it is
  absent, **stop** — do not invent a second subtree.
- Assumptions to validate: §A1 (8–13), §A4 (24–29), §A5 (30–32), §A6 (33–35).
  Keep items: §F5 (116–123), §F9 (164–177), `guidance/DO-NOT-TOUCH.md`.

## Scope
- Move `crates/chunk-verify/`, `crates/repair-plan/`, `crates/lock/`, `crates/fetch/`,
  `crates/report/` into the subtree chosen in T04.
- Root `Cargo.toml`: workspace `members`, normal and dev dependencies (including the
  `clap` feature on `girpr-report` and the `test-support` feature on `girpr-fetch`).
- Consumers that stay in `crates/` for now: `crates/test-fixtures/` and
  `crates/test-support/` depend on `fetch`/`repair-plan`/`report`; repoint those edges to
  the subtree (T06 moves them later).
- Consumers already in the subtree (T04): adjust relative paths as needed; shared-crate
  deps point back into `crates/`.
- Path text: root `README.md` crate table and prose, `.config/nextest.toml` comments, doc
  comments, tool docs. Locate occurrences in the current tree before editing.

## Implementation plan
1. Validate §F4/§F1 for all five: no consumer outside the girpr family. If one is shared,
   stop and leave it in `crates/` (§A6, §F5).
2. Move the five directories without renaming them; keep the workspace lint table (§F7).
3. Repoint their path deps for the new depth: shared crates (`digest`, `proto`, `filter`,
   `fs-identity`, `report-core`, `chunk-identity`, `chunk-cost`, `tempdir`, `log`, …) stay
   under `crates/`; crates moved in T04 and this task are reached inside the subtree.
4. Preserve the `repair-plan → fetch` edge inside the subtree (§F6) and all feature wiring
   (`clap`, `test-support`) exactly.
5. Repoint root and remaining `crates/` consumers, then update all §F8 seams (workspace
   `members`, README table/prose, nextest comment path text, doc comments).
6. Verify a locked build/tests resolve; `Cargo.lock` package names are unchanged for this
   task, so do not hand-edit it unless validation shows a real change is needed.
7. Deliver in several commits if useful: move + manifest rewrites, consumer rewrites, docs.

## Constraints / validation
- Keep internal T04/T05 relative paths consistent; the intermediate tree must build before
  T06 moves the two test crates.
- Do not change the report plane, counter/ledger behaviour or chunk-verification logic —
  this is a location change only (§F4 describes them as mechanics, not shared leaves).
- Do not move or rename any shared crate or gircap crate (§F5, §F9).
- Keep nextest filters unchanged (package names do not change) and nextest process
  isolation intact (§F8).

## Done when
The five crates live in the recorded subtree with unchanged names/lints, all consumer
paths and §F8 seams are updated, `repair-plan → fetch` and feature wiring survive, and the
workspace builds/tests. If a premise is stale or a keeper would change, **stop** and
record `skipped` with a one-line reason.
