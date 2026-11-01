# T06 - C4: write `--plan-out` atomically

Report: `../../OPTIMIZATIONS_REPORT.md` lines 144-163 (C4).

Dependencies: none.

## Goal

`scope::report::PlanDocument::write_to` serializes the document to a `String`,
copies it again through `format!`, and calls `std::fs::write` on the requested
path. A full disk or a crash leaves a truncated artifact where the caller asked
for a parseable plan, and the plan is materialized twice in memory.

Serialize directly to a buffered writer on a temp path in the target
directory, then rename. Keep the same JSON shape and trailing newline.

## Where to start

- `src/repair/scope/report.rs`: `PlanDocument::write_to` (around line 351) and
  the document tests around lines 717-743.
- `src/repair/mod.rs`: the `--plan-out` call in `plan_phase` (around line 421)
  maps the error to `RunFailure::write`; do not change the exit code.
- `src/util/ini.rs` and `src/repair/audio.rs` show the existing temp-then-rename
  shape; T07 unifies their names, and T28 later extracts the shared helper.
  This task does not need to wait for either.

## Steps

1. Create the parent directory when it is missing (the current behavior).
2. Compute a temp path beside the requested path. Append a `.girpr_tmp` suffix
   to the whole file name, not `with_extension`, so `plan.v1.json` becomes
   `plan.v1.json.girpr_tmp`. Use an `OsString` append so a non-UTF-8 path is not
   lossy.
3. Open the temp file, wrap it in a `BufWriter`, and call
   `serde_json::to_writer_pretty(&mut writer, self)` directly. Write exactly
   one trailing `\n`, then `flush`. Do not build an intermediate `String` and do
   not use `format!`.
4. Drop/close the writer before the rename. Rename the temp over the target.
5. On any error, remove the temp file if it exists (ignore `NotFound`) and
   return the original error with context. The target must be left as it was.
   Do not leave a temp behind on the error path.
6. Do not add `sync_all` by default. This is a user-requested artifact, not the
   launcher's version bookkeeping; the report's requirement is "no truncated
   artifact", which temp-then-rename provides. If the implementer adds a
   durability flag, keep it internal and default false for this writer.

## Tests

- The existing `writing_creates_the_parent_directory` test must still pass.
- Add a test that the on-disk bytes are valid JSON with the documented shape
  and end in exactly one newline; the existing document-shape test can be
  reused.
- Add a test that a path which cannot be renamed (for example the target is an
  existing directory) leaves no `.girpr_tmp` sibling and returns an error.
- Add a test that a pre-existing valid target is not truncated when the write
  fails; the directory-as-target case is a deterministic way to force the
  publish failure.
- Do not assert peak memory in a unit test; the one-buffer property is a code
  review point.

## Acceptance

- `--plan-out` is never observable as a truncated file: readers see the old
  file or the complete new one.
- The serialized document is byte-identical in shape to today (pretty JSON,
  trailing newline).
- No temp file survives a failed write.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(plan): write the plan document through a temp file`
2. `test(plan): pin atomic plan-out failure cleanup`

## Notes

- The temp file is beside the requested path, which the user may not expect.
  That is the documented trade of atomic publishing; the file is removed on
  failure and is not part of the install.
- T07 changes the suffix rule for the other writers; use the same rule here so
  the later unification is mechanical.
