# T44 — Narrow loopback hit-count/assertion extension

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F49 (lines 644–652) — the shared
  loopback server is only the mechanical layer; scripting is repeated. Candidate boundary is
  a `ScriptedOrigin`/`ReplayHarness` extension of the loopback leaf owning journal
  assertions, each program supplying its route table. Confidence low for domain servers,
  **medium for a narrow hit-count/assertion extension**.
- Assumptions to validate: §A10 (98–105), §A4 (48–56), §A2 (29–36), §A14 (129–135), §N6 (718–720).

## Scope
- `girpr-loopback-mock` (`ScriptedServer`, dependency-free) and the repeated
  bind→script→journal→hit-count layer in `crates/test-fixtures`, `crates/test-support`,
  `crates/sync-test-support`, `tools/capture/test-support`. Locate before editing.

## Implementation plan
1. Validate §F49: confirm the repeated layer and that domain servers encode different
   contracts (N6).
2. Scope to the **narrow** extension only: a hit-count/journal assertion primitive; do not
   merge domain servers. Keep the leaf dependency-free (§F49, §A4).
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. Deliver in commits: extension, migrations, seams.

## Constraints / validation
- Domain servers stay independent (N6); nextest process isolation makes cross-test fixture
   state useless (§A10).
- The leaf must remain dependency-free so all programs can use it (§A4). Do not absorb the
   replay/bridge server (N6).

## Done when
The narrow hit-count/assertion extension is shared, the leaf stays dependency-free, domain
servers are untouched, and doc seams are updated. If only a domain-server merge is possible,
**stop** and record `skipped`.
