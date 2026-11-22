# T22 — Shared retry decision vocabulary / `drive_attempts`

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F27 (lines 424–433) — retry decisions
  share a mechanics layer but disagree on classifier and jitter; candidate boundary is a
  shared `drive_attempts` in `girpr-retry` (classification local) or, less invasively, a
  shared decision vocabulary `RetryDecision::{Done,Fatal,Transient}` with a caller-supplied
  status set. Confidence high.
- Assumptions to validate: §A9 (88–97), §A5 (57–67), §A2 (29–36), §A14 (129–135), §A11 (106–113).

## Scope
- Deepen `crates/retry` (`Schedule`/`Sleep` drivers). Sources: `crates/fetch/src/retry.rs`
  (`Retry`/`RetryState` attempt driver) and `tools/capture/http/src/lib.rs` /
  `src/http/fetcher.rs` (third loop with per-attempt logging and retained error bodies).
  Locate before editing.

## Implementation plan
1. Validate §F27: confirm the three loops and that shared `girpr-retry` owns only sleep
   drivers; confirm the status set is published (`error_shapes.json`).
2. Prefer the less-invasive option: add a shared decision vocabulary with a caller-supplied
   status set; keep classifiers, jitter and per-attempt logging local. Only attempt a shared
   `drive_attempts` if logging/body retention can be parameterized without loss.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit vocabulary/driver, migrations, seams.

## Constraints / validation
- Jitter choices encode opposite goals (girpr 0.5, gircap 0.0) — keepers (§A9); do not
  unify them.
- The status set is published (`error_shapes.json`) — any wording/order change breaks §A5.
- Do not absorb the transport wrapper (T21). Feature unification can add compiled variants
  (§A11); measure if consolidating surfaces.

## Done when
A shared decision vocabulary (or safely parameterized driver) exists, classifiers/jitter
stay local and published error shapes are byte-identical. If jitter/body retention cannot
be preserved, scope down to the vocabulary or **stop** and record `skipped`.
