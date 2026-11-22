# T38 — Scratch/fixture support (merge vs split assessment)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F43 (lines 587–597) — scratch/fixture
  support is simultaneously too fine and too coarse; candidate boundary is either one
  generic fixture-builder crate, or splitting fixture-builder/lock-cleanup/replay-server.
  **Both directions high-risk.** Confidence high.
- Assumptions to validate: §A2 (29–36), §A4 (48–56), §A10 (98–105), §A14 (129–135), §F2 (156–163), §N6 (718–720).

## Scope
- `girpr-tempdir` (`TempDir` + program-free file/link/mtime helpers),
  `girpr-test-fixtures` (one-line `temp_dir` wrapper + four unrelated concerns: lock cleanup,
  artifact replay server, fixture builders/digest, scratch/log glue),
  `crates/sync-test-support` (re-exports tempdir test surface), `tools/capture/test-support`
  (depends on tempdir). Locate before editing.

## Implementation plan
1. Validate §F43: confirm the current split and the four concerns in `girpr-test-fixtures`.
2. Assess both directions: a merge reintroduces girsync's rebuild cascade (§F2) and
   violates §A4; a split multiplies shims/rebuild domains. If both are net-negative, record
   a validated skip with the one-line reason.
3. Implement only on hard evidence of a net win; keep the domain replay servers independent
   (N6) and nextest isolation intact (§A10). Move through §A14 seams if done.

## Constraints / validation
- A merge must not make one program's tests depend on another (§A4, §F2). A split must not
   multiply dev-dependency rebuild domains the leaves were created to reduce (§A2).
- Domain replay servers stay independent (N6).

## Done when
A validated decision is recorded — expected `skipped` for both directions, or a safe
reorganization with unchanged test behavior.
