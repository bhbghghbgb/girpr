# T16 - P3: consume the `--scan` verdicts in the repair loop

Report: `../../OPTIMIZATIONS_REPORT.md` lines 329-356 (P3).

Dependencies: T12 (whole-file digest), T15 (shared local map). This task uses
the scan result the plan phase already produced; it must not add a second scan
of its own.

## Goal

Under `--scan`, the plan phase reads every planned file, then the in-place
repair opens and reads it again to reach the same three answers: whole-file
MD5, per-chunk destination verdicts, and per-source verdicts. Reuse the plan
phase's verdict in the in-place writer when a cheap stat guard proves the file
has not changed since it was scanned. On a mismatch, fall back to the current
fresh scan.

The temp path keeps its own whole-file gate; do not wire the scan verdict into
it in this task. State that rule in the module comment: the plan scan feeds the
in-place writer only, because the temp path's gate is a single digest over the
finished temp file, not a verdict about the target.

## Where to start

- `src/repair/scan.rs`: `Verdict` (around line 39), `ScanReport` (around line
  57), `scan_one` (around line 323), `scan_ranges` (around line 290).
- `src/repair/mod.rs`: `PlanPhase.scan` (around line 115), the scan construction
  (around line 313), the destructure in `execute_phase` (around line 458), the
  `file::repair_all` call (around line 574).
- `src/repair/file/mod.rs`: `repair_all` (around line 194), `FileCtx` (around
  line 383).
- `src/repair/file/inplace.rs`: `attempt_inplace_inner` (around line 69), the
  scan call (around line 109), the discard replacement (around line 137).
- `src/util/md5.rs`: `Scan` (around line 117), `scan_reader` (around line 207).

## Steps

1. Add the facts the guard needs to `Verdict`: the file length the scan saw and
   its modification time. Fill them in `scan_one` from one `std::fs::metadata`
   of the resolved path. If the stat fails, leave the guard absent (`None`);
   an absent guard must never authorise reuse.
2. Change `PlanPhase.scan` to `Option<Arc<ScanReport>>`. Wrap the result of
   `scan::scan_plan` once. Update the estimate, `--print-plan`, `--plan-out`,
   and the check-only call to deref the `Arc`.
3. Pass `Option<Arc<ScanReport>>` into `file::repair_all` and store it in
   `FileCtx`. Do not clone the report per file.
4. Add a small helper in `inplace.rs`, for example:
   `fn reusable_verdict<'a>(ctx: &FileCtx, file: &PlannedFile, handle: &File)
   -> Option<&'a Verdict>`. It returns `Some` only when:
   - the report exists and has this file's rel;
   - the open handle's length equals the verdict's recorded length;
   - the path's current modification time equals the recorded one.
   Treat every uncertainty as "not reusable". Do not compare only size, and do
   not use a timestamp window.
5. In `attempt_inplace_inner`, after opening the target and before the scan:
   - call the guard; if it is reusable, build a `util::Scan` from the verdict:
     `file_md5`, `file_len`, `dest_ok` (one per chunk), and a per-source-range
     `source_ok` built through `source_owner` (chunk -> range). Log at debug
     that the plan scan was reused and how old the snapshot is.
   - otherwise call `util::scan_reader` exactly as today.
   Keep the discard branch: deleting the target replaces the scan with
   `util::Scan::absent`, even if a verdict was reused for the skip check.
6. The skip check remains "whole-file MD5 matches `file.md5`", and the bad
   chunks remain `!dest_ok[i]`. Do not change the patch, tier, salvage, or
   verify logic.
7. If `ctx.dry_run`, the reused verdict may avoid the read; that is allowed
   because nothing is written. An open failure still returns `Unavailable` or
   `DryRun` exactly as today.
8. Do not add a cache to `PathRoot`, and do not change `ScanReport`'s public
   shape beyond the guard fields. T27 will choose the durable keying.

## Tests

- Unit-test the guard with a temp file: same length and mtime is reusable;
  changed length is not; a changed mtime is not; a missing verdict is not.
  Keep the mtime test deterministic by setting the file time explicitly, not
  by sleeping.
- A `--scan` repair and a non-scan repair produce the same bytes, the same
  `ChunkStats`, and the same exit code. The existing offline fixtures should
  cover this; add a case only if both modes are not already compared.
- A file changed between the plan scan and the repair is repaired correctly
  (the guard falls back). Simulate by mutating the file after the plan phase in
  a test seam; if that is not reachable, test the guard helper alone and leave
  the end-to-end case to T44.
- `cargo test -p girpr` passes.

## Acceptance

- With an unchanged file, `--scan` does not read the file a second time in the
  in-place writer before writing.
- Any length or mtime mismatch triggers the existing fresh scan.
- The temp path's behaviour and its whole-file gate are unchanged.
- The scan snapshot is shared by `Arc`, not cloned per file.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(repair): reuse a guarded plan scan in the in-place writer`
2. `test(repair): pin the scan-reuse guard and fallback`

## Notes

- The guard is deliberately conservative: a false "not reusable" costs one
  read; a false "reusable" writes against a stale layout. Prefer the read.
- Modification time granularity differs by filesystem. Exact equality is the
  only safe comparison; do not "round" it.
- T27/D1 may replace the rel-keyed lookup with a dense file id. Keep the guard
  helper separate from the keying so that change is local.
