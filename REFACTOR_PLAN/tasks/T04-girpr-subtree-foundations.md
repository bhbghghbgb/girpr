# T04 — Establish the girpr-owned subtree and move the low-level leaves

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F4 (lines 104–114) — the 13 girpr-only crates
  are build-time split leaves whose `girpr-` prefix already names their owner; no rename
  is implied, but if `crates/` is to mean "shared only" they belong in a girpr-owned
  subtree at the workspace root. **The report does not fix that subtree's name.**
- §F1 rows for `audio` (68), `util-counter` (77), `util-hash` (78), `util-ini` (79),
  `util-path` (80), `repair-policy` (73) — these six are girpr-only and moved here;
  their consumers are the root package and other girpr-only crates.
- §F7 (lines 141–148) — girpr-owned crates keep `[lints] workspace = true`; the tools-only
  table is only for dev-tool-owned crates.
- §F6 (lines 130–131, 137–139) — `repair-plan → fetch` stays inside one project;
  `girpr-retry`, `girpr-log`, `girpr-time`, `girpr-digest` and `girpr-fs-identity` are shared
  dependencies of moving crates and stay in `crates/`.
- §A7 (lines 36–39) and §F8 (lines 150–162) — workspace `members`, dependent path deps,
  `Cargo.lock` package entries and path text in READMEs/doc comments.
- Assumptions to validate: §A1 (8–13), §A4 (24–29), §A5 (30–32), §A6 (33–35).
  Keep items: §F5 (116–123), §F9 (164–177), `guidance/DO-NOT-TOUCH.md`.

## Scope
- Choose one girpr-owned subtree directory at the workspace root (e.g. `app/` — a hint,
  not a requirement) and record the chosen path in `STATE.md` Persistent context so T05
  and T06 use the same location.
- Move `crates/audio/`, `crates/util-counter/`, `crates/util-hash/`, `crates/util-ini/`,
  `crates/util-path/`, `crates/repair-policy/` into that subtree.
- Root `Cargo.toml`: workspace `members`, normal dependencies and dev-dependencies
  (note the `test-support` features on `util-hash`/`util-path` and the `clap` feature on
  `repair-policy`).
- Consumer manifests: `crates/report/Cargo.toml` (`util-counter`), `crates/repair-plan/`,
  `crates/fetch/`, `crates/lock/`, `crates/chunk-verify/`, `crates/test-support/` — repoint
  only what actually depends on the six moving crates; locate current edges before editing.
- Path text: root `README.md` crate table and prose, `.config/nextest.toml` comments,
  source doc comments, tool READMEs/design notes. Locate occurrences in the current tree.

## Implementation plan
1. Validate §F4/§F1: confirm each of the six has consumers only in the girpr family. If any
   is consumed by gircap or girsync, stop — it is shared and must not move (§A6, §F5).
2. Pick the subtree name, record it in `STATE.md` Persistent context (this is the one
   state write that accompanies the task result), and create the directory.
3. Move the six crate directories without renaming them or changing their lint tables
   (§F4, §F7). Convert their intra-workspace path deps to the new depth, keeping shared
   deps (`girpr-filter`, `girpr-digest`, `girpr-fs-identity`, `girpr-tempdir`) pointing back
   into `crates/`.
4. Repoint every consumer manifest and the root manifest's dependencies/dev-dependencies
   to the new paths, preserving features and versions exactly.
5. Update all §F8 seams: workspace `members`, README table/prose path text, nextest
   comment path text (package names do not change, so nextest filters must not change),
   doc comments and any tool doc text.
6. `Cargo.lock` names are unchanged for this task; verify a locked build succeeds rather
   than hand-editing lockfile entries.
7. Deliver in several commits if useful: establish subtree + first crates, remaining
   crates and consumer rewrites, doc seams.

## Constraints / validation
- No rename of these crates; their `girpr-` prefix already names their owner (§F4).
- Keep the workspace lint table on all six (§F7). Do not switch them to the tools-only
  table just because they move.
- Do not move or rename any of the 19 shared crates or touch gircap's crates (§F5, §F9).
- Preserve feature wiring (`test-support`, `clap`) and test-only inversion edges (§F6).
- The intermediate tree must build after this task even though T05/T06 have not moved the
  rest of the girpr-only crates yet.

## Done when
The subtree exists, its name is recorded in `STATE.md`, the six crates live there with
unchanged names/lints, all consumer paths and §F8 doc seams are updated, and the workspace
builds. If a crate turns out to be shared or a keeper would change, **stop** and record
`skipped` with a one-line reason.
