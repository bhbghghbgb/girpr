# T04 — Writer / file-repair crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F15 (lines 241–245) —
  `src/repair/file/{mod,inplace,temp,mode}.rs` (~4,826 lines, ~2.1k tests) is
  the largest cluster of reqwest + zstd + tokio + tempfile repair code; it is
  high-level with few dependents but names plan/scan/ledger types.
- Assumptions to validate: §A2 (27–33), §A10 (86–94), §A6 (55–61), §A4 (40–46).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/file/mod.rs:75-85`, `inplace.rs:13-23`, `temp.rs`, `mode.rs`.
- Its dependents: `src/repair/mod.rs`, `src/repair/scan.rs`,
  `src/repair/scope/report.rs`.
- Test-only hooks in `src/repair/file/mod.rs:367-395,484-486`.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Map the interface: which `plan`, `scan`, `ledger` types cross the boundary.
   The payoff depends on a narrow interface (§F15); if the boundary requires
   moving most of `repair` with it, stop.
2. Move shared types that the writer needs **down** below the boundary first
   (§A10) rather than duplicating them, so the new crate stays a leaf.
3. Create the crate and make the few high-level dependents reference it directly
   (§A2). Confirm a change in unrelated repair phases no longer recompiles the
   writer cluster.
4. Preserve the test-only hooks in the shipped file (§F20) or promote them via
   the §A7 "public lib + thin shim" pattern; update pinned nextest filters for
   `repair::file::inplace::tests::…` (§F31).
5. If gircap consumes the crate, update the §A5/§F39 doc tables.

## Constraints / validation
- reqwest/zstd/tokio/tempfile are the heavy deps here (§F27) — the gain is
  keeping them cached when other repair code changes, not merely moving them.
- No byte/behaviour change in repair output (§A6).
- Do not cut through a cycle (§A10); if a cycle spans the boundary, stop and
  raise a follow-up.
- Do not pull `DO-NOT-TOUCH` items (locks, walkers, atomic publish) into the
  new crate to make it bigger.

## Done when
The writer cluster is a crate with a narrow interface, its high-level consumers
depend on it directly, and all file-repair and pinned nextest tests pass with
unchanged output bytes.
