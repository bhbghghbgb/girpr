# Refactor state

Single source of truth for orchestration. The orchestrating session may be replaced at
any time; all durable knowledge lives here. See `ORCHESTRATION.md` for the procedure and
`CONVENTIONS.md` for the format.

States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
`skipped`. Every change to this file is its own commit. Commit refs are pointers for
reviewers only — no dates, session/tool IDs, machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Move and rename `girpr-sync-cache` → `girsync-cache` | in_progress | | §F3, §F7, §F8, §F6: girsync-only crate to `tools/sync/cache/`, tools-only lints; preserve direct sled2redb edge and `postcard` digest vocabulary |
| T02 | Move and rename `girpr-sync-leaves` → `girsync-leaves` | todo | | §F3, §F7, §F8: girsync-only crate to `tools/sync/leaves/`, tools-only lints |
| T03 | Move and rename `girpr-sync-test-support` → `girsync-test-support` | todo | | §F3, §F7, §F8, §F6: girsync-only crate to `tools/sync/test-support/`, tools-only lints; preserve the `→ girsync` inversion and both dev-dep edges |
| T04 | Establish the girpr-owned subtree and move the low-level leaves | todo | | §F4, §F7, §F8: choose subtree name, move `audio`, `util-counter`, `util-hash`, `util-ini`, `util-path`, `repair-policy`; keep `girpr-` names and workspace lints. Record chosen path in Persistent context |
| T05 | Move the mid-layer girpr-only product crates | todo | | §F4, §F6, §F8: move `chunk-verify`, `repair-plan`, `lock`, `fetch`, `report` into the chosen subtree; preserve `repair-plan → fetch` edge and feature wiring. Use subtree path from Persistent context |
| T06 | Move the two girpr-only test crates | todo | | §F4, §F6, §F8: move `test-fixtures`, `test-support`; preserve the `test-support → root` test-only inversion and the root shims. Use subtree path from Persistent context |
| T07 | Ownership consistency sweep | todo | | §F5, §F7, §F8, §F9: final classification audit, remaining path text, Cargo.lock and lint-table verification, gircap untouched, fix misleading manifest descriptions |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | pending | |
| B | T03 | pending | |
| C | T04 | pending | |
| D | T05 | pending | |
| E | T06, T07 | pending | |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. Reviewers decide which are worth doing and may prune them to avoid run-away.
A planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| (none yet) | | | |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code records that in the
relevant task's `note` instead of silently proceeding):

- **The report is a static classification of one tree (§A1, lines 8–13).** Every §F item
  is a claim to validate against current manifests; line numbers may have drifted; trust
  the code. The ownership rule is direct Cargo dependency edges, dev-dependencies
  included (§A3, lines 19–23).
- **Program families (§A1–A2, lines 8–18).** Four programs: root `girpr`, `gircap`,
  `girsync`, `sled2redb`. `sled2redb` belongs to the girsync family; a crate consumed
  only by girsync + sled2redb is girsync-owned, not shared.
- **Ownership convention (§A4, lines 24–29).** A project-owned crate lives under the
  owning project's directory, carries that project's name prefix, and uses the tools-only
  lint table when the owner is a dev tool. `tools/capture/{http,hash,test-support}` is the
  in-repo precedent.
- **No shared crate is renamed or moved (§A6, lines 33–35; §F5, 116–123; §F9, 164–177).**
  The 19 shared crates stay in `crates/` with their names; gircap's three crates are
  already correct; `girpr-audio-language` and the other gircap-consumed crates are shared.
- **Location and `girpr-` prefix are not evidence of sharing (§A5, lines 30–32).** The
  consumer map §F1 decides; do not classify by directory or prefix.
- **Consumer edges and wire vocabulary must survive (§F6, lines 125–139).** In particular
  sled2redb's direct edge to the cache and the cache's pinned `postcard` digest
  vocabulary, the `girpr-test-support → root` test-only inversion, and the
  `girpr-sync-test-support → girsync` inversion.
- **Lint tables follow the owner (§F7, lines 141–148).** Crates under `tools/sync/`
  switch to the tools-only `[lints.rust]`/`[lints.clippy]` table; crates in the girpr
  subtree keep `[lints] workspace = true`.
- **A move changes more than the directory (§A7, lines 36–39; §F8, 150–162).** Workspace
  `members`, dependent path deps, `Cargo.lock` package entries (for the three renames),
  path text in READMEs/doc comments and the three `tests/common/mod.rs` shims all move
  together.
- **The girpr subtree name is not fixed by the report (§F4, lines 104–114).** T04 chooses
  it; orchestrator records the chosen path here before T05/T06. Until then, no task
  names it.
- **No lingering references at cleanup.** No source, test, comment, manifest, doc table
  or commit message may reference `REFACTOR_PLAN/` or
  `docs/shared-crate-ownership-findings.md`.
