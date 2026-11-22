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
| T01 | Move and rename `girpr-sync-cache` → `girsync-cache` | completed | 1aa45d2b, 8fe2e94c | §F3, §F7, §F8, §F6: girsync-only crate to `tools/sync/cache/`, tools-only lints; preserve direct sled2redb edge and `postcard` digest vocabulary. Deviation: intra-workspace path deps use `../../../crates/*` (three levels deep), not the plan's hinted `../../crates/*`; extra doc path text in `docs/03-repair-rust-implementation-plan.md` updated |
| T02 | Move and rename `girpr-sync-leaves` → `girsync-leaves` | completed | 835c0c1a, dd37a6b1 | §F3, §F7, §F8: girsync-only crate to `tools/sync/leaves/`, tools-only lints. Deviation: path deps use `../../../crates/*` (stale depth hint in plan); doc path text also fixed in `docs/03-repair-rust-implementation-plan.md` and `docs/04-scope-limitation-spec.md` |
| T03 | Move and rename `girpr-sync-test-support` → `girsync-test-support` | completed | e875e6f3, e222caa5 | §F3, §F7, §F8, §F6: girsync-only crate to `tools/sync/test-support/`, tools-only lints; preserve the `→ girsync` inversion and both dev-dep edges. Deviation: shared path deps use `../../../crates/*` (stale depth hint); sled2redb README and nextest.toml held no occurrences; `docs/03-repair-rust-implementation-plan.md` updated as a §F8 seam |
| T04 | Establish the girpr-owned subtree and move the low-level leaves | completed | 8ddd15cf, d72b6a8f | §F4, §F7, §F8: chose `app/` subtree and moved `audio`, `util-counter`, `util-hash`, `util-ini`, `util-path`, `repair-policy`; names and workspace lints unchanged; Cargo.lock names unchanged. Chosen path recorded in Persistent context |
| T05 | Move the mid-layer girpr-only product crates | completed | 8d16a9d8, aa3fb5ed, 822cf4c6 | §F4, §F6, §F8: five crates moved into `app/` with names, lints, `repair-plan → fetch` edge and feature wiring intact; no lockfile diff. T06 note: test crates moving into `app/` must change `../../app/<crate>` deps to `../<crate>` and inversion dep `girpr` from `../..` to `../../` |
| T06 | Move the two girpr-only test crates | completed | 29154608, 9ffe9464 | §F4, §F6, §F8: both crates moved into `app/` with names/lints unchanged, inversion to root kept as a normal dep and the two crates kept separate; root shims, nextest filters/FQNs and isolation untouched. Inversion path written `../../` (textual no-op at same depth); shims held no old path text |
| T07 | Ownership consistency sweep | completed | 7fa1c193 | §F5, §F7, §F8, §F9: audit confirms 19 shared / 13 app / 3 tools/sync / 3 tools/capture, lint tables and locks correct, no forbidden references; F01 README wording and girpr-lock/fetch/report "shared" descriptions fixed. F01 closed by this task |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | done | Both completed; no fixes needed |
| B | T03 | done | Completed; no fixes needed |
| C | T04 | done | Completed; no fixes needed |
| D | T05 | done | Completed; no fixes needed |
| E | T06, T07 | done | Both completed; no fixes needed |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. Reviewers decide which are worth doing and may prune them to avoid run-away.
A planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Correct README's "three dev tools opt into a smaller lint subset" wording (now understated by girsync-cache/girsync-leaves) | done | Endorsed by reviews A-D, folded into T07 and confirmed satisfied by review E (commit 7fa1c193) |
| F02 | Normalize remaining pre-existing ownership wording: `shared` descriptions on girpr-only `app/` manifests (util-path, util-ini, util-hash, util-counter, audio, test-support, test-fixtures) and `two dev tools` phrasing in root Cargo.toml lint comment, digest/tempdir comments and crates/log doc comments | batched | Approved as a single batch "Doc-only ownership wording cleanup" by follow-up planning review; word-only edits, no behavior/classification/lint-table change; no new follow-ups proposed |

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
- **Girpr subtree chosen as `app/` in T04 (§F4, lines 104–114).** T05/T06 move into
  `app/<crate>`. From a crate already inside `app/`, moved-girpr deps are `../<crate>`
  while shared deps remain `../../crates/<crate>` (T04 path-dep follow-up for T05/T06).
- **No lingering references at cleanup.** No source, test, comment, manifest, doc table
  or commit message may reference `REFACTOR_PLAN/` or
  `docs/shared-crate-ownership-findings.md`.
