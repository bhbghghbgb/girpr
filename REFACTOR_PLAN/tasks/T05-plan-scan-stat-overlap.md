# T05 — Eliminate the duplicate plan/scan per-file stat pass

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §2 **S-06** (lines 186–198) — `DiskPlan::new`
  sequentially `symlink_metadata`s every planned file on the blocking pool; with
  `--scan`, `scan_one` already stats the same file, and the two blocking tasks are
  awaited one after the other.
- Change suggested in the report: feed the scan's length/mtime/is-file into
  `DiskFigure::Measured`, or hold both `JoinHandle`s and await them together, or
  shard the stat work by index like check/scan.
- Keep items: estimate/plan correctness, scan trust semantics, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/scope/report.rs` (`DiskPlan::new` ~83–123, `DiskFigure`).
- `src/repair/scan.rs` (`scan_one` ~382–384, its stat).
- `src/repair/mod.rs` (~245, ~400–407) where the two blocking tasks are launched and
  awaited.
- Locate current call sites before editing; the list above is a hint.

## Implementation plan
1. Confirm in the current tree that both passes stat the same planned files under
   `--scan`. If scan data cannot be reused safely, overlap the two tasks by holding
   both `JoinHandle`s instead of eliminating the pass.
2. Prefer feeding the scan's length/mtime/is-file into `DiskFigure::Measured` so one
   stat serves both; otherwise await both tasks together or shard by index.
3. Preserve the semantics of "measured" versus "estimated" figures in the plan.
4. Deliver in several commits if useful: data plumbing, task overlap, tests.

## Constraints / validation
- Plan estimates and `DiskFigure` classification must remain correct; a scan-derived
  figure must describe the same file state the plan expects.
- Do not change plan/progress output (lines 527–530) or any §12 non-finding.

## Done when
The repeated per-file stat is eliminated or genuinely overlapped, plan figures remain
correct, and tests pass. If the scan data cannot be reused safely, **stop** and record
`skipped` with a one-line reason.
