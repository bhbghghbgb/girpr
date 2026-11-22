# T07 — Ownership consistency sweep

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F5 (lines 116–123) — the shared set is stable
  under the ownership split; re-derive it from the current manifests and confirm no move
  removed a shared crate's second consumer.
- §F2 (lines 85–89) — target end state: 19 shared crates in `crates/`, 13 girpr-only in
  the girpr subtree, 3 girsync-only under `tools/sync/`, plus gircap's 3 under
  `tools/capture/`.
- §F7 (lines 141–148) — verify lint tables follow the owner after all moves.
- §F8 (lines 150–162) — verify every seam of every move is complete.
- §F9 (lines 164–177) — gircap's crates were not candidates; `girpr-audio-language` and
  the other gircap-consumed crates are shared; `girpr-lock`'s manifest description is
  misleading (lines 175–177) and may be corrected here.
- §A7 (lines 36–39) — a move changes more than the crate directory.
- Keep items: §F5 (116–123), §F9 (164–177), `guidance/DO-NOT-TOUCH.md`.

## Scope
- All manifests and docs touched by T01–T06: root `Cargo.toml`, `Cargo.lock`, crate
  manifests, root `README.md`, `tools/sync/README.md`, `tools/sync/DESIGN-NOTES.md`,
  `tools/sled2redb/README.md`, `.config/nextest.toml` comments and source doc comments.
- No crate directory moves in this task; it is an audit plus small documentation and
  wording corrections.

## Implementation plan
1. Rebuild the consumer map from the current manifests (direct edges, dev-dependencies
   counted, §A3) and compare it with §F1/§F2. Confirm each crate's location, name and lint
   table match its owning family. Do not reclassify a crate based on directory or prefix
   (§A5, lines 30–32).
2. Confirm no shared crate was moved or renamed and gircap's three crates are untouched
   (§F5, §F9). If a previous task misclassified one, revert that part as a fix in this
   task or record it for the reviewer — do not expand scope into a new shared-crate
   extraction.
3. Sweep for stale path text left by T01–T06: old `crates/sync-*` paths, old crate names,
   old subtree paths in READMEs, tool docs, nextest comments and doc comments. Fix only
   text; do not change nextest filters or FQNs.
4. Verify `Cargo.lock` package entries match the three renames and that a locked build
   resolves; verify lint tables (§F7) on every moved crate.
5. Correct manifest descriptions that wrongly claim a girpr-only crate is shared, notably
   `girpr-lock` (§F9, lines 175–177). Keep the technical prose otherwise unchanged.
6. Confirm no source, test, comment, manifest, doc table or commit message in version
   control references `REFACTOR_PLAN/` or `docs/shared-crate-ownership-findings.md`.
7. Deliver in one or a few small commits: audit-driven doc/wording fixes, lockfile touch-up
   if required.

## Constraints / validation
- This is a consistency pass, not a new refactor: do not move crates, change byte
  contracts, change public APIs or reclassify shared crates (§F5, §F9).
- Do not change nextest package filters or test FQNs; only comment path text may change
  (§F8).
- Any fix that would alter behaviour or classification belongs to a reviewer or an
  endorsed follow-up, not to this sweep.

## Done when
The final layout matches §F2's counts, lint tables match §F7, every §F8 seam is clean,
misleading descriptions are corrected, a locked build passes, and no reference to this
package or the report survives in version control. If a wider problem is found, **stop**
and record `skipped` with a one-line reason so the reviewer can decide.
