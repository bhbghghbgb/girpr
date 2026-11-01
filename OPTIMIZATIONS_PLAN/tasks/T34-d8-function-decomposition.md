# T34 - D8: split the functions doing too much

Report: `../../OPTIMIZATIONS_REPORT.md` lines 772-792 (D8).

Dependencies: T27, T28, T29. This is the largest pure refactor in the plan;
review it alone.

## Goal

Five functions carry a whole sequence with numbered comments and early
returns: `repair::plan_phase`, `repair::execute_phase`,
`file::inplace::attempt_inplace_inner`, `purge::plan_purge`, and
`util::ini::write_config_ini`. Extract small, testable steps so the sequence
is visible and a future phase change is a smaller diff. Prefer small pure
helpers over new trait objects; keep the existing comments next to the
extracted step.

## Where to start

- `src/repair/mod.rs`: `plan_phase` (around line 149), `execute_phase`
  (around line 453).
- `src/repair/file/inplace.rs`: `attempt_inplace_inner` (around line 69).
- `src/repair/purge.rs`: `plan_purge` (around line 92).
- `src/util/ini.rs`: `write_config_ini` (around line 264).

## Steps

### plan_phase

Split into helpers that return the values the next step needs:
1. `fetch_and_report(ctx, game_dir) -> (Meta, ...)` for metadata + REPORT.
2. `load_manifests_and_reuse(ctx, &meta) -> (per_manifest, local_map)`.
3. `build_and_assert_plan(...) -> (Arc<RepairPlan>, purge_plan, deprecated,
   scan)` for the work list, deprecated list, scan, estimate, and ceilings.
4. `publish_plan(...)` for `--print-plan`, `--plan-out`, and the disk plan.
Keep the early `PlanOutcome::Finished` returns in one place; do not let a
helper return a code that the caller must reinterpret.

### execute_phase

Split by mode:
1. `prepare_install` (create dir, canonicalize, root);
2. `acquire_lock` (T17's predicate);
3. `write_explicit_audio`;
4. `run_purge_before`;
5. `run_check_only`;
6. `run_purge_only`;
7. `run_repair`;
8. `run_post`.
Return a small `ExecuteOutcome`/`(Summary, ExitCode)`; keep one summary tail.

### attempt_inplace_inner

Split:
1. `open_and_scan` (open, scan/reuse, skip check, dry-run);
2. `discard_if_requested` (delete + absent scan);
3. `plan_bad_chunks` (dest/source remap, `bad`, kept stats);
4. `trim_dead_tail` (the eager/deferred trim decision);
5. `choose_tier` (payload, reservation, staged/ordered);
6. `stage_sources` (capture loop);
7. `patch_chunks` (the write loop);
8. `verify_and_retry` (first verify, redo, final).
Keep `ChunkStats` threaded through; do not allocate a new stats struct per
helper.

### plan_purge

Split into `walk_and_classify` (the WalkDir loop) and `finish_plan` (totals,
dirs, sorting). Return the same `PurgePlan`. Keep the scope/hidden/error counts.

### write_config_ini

Split into:
1. `parse_ini(text) -> Vec<Section>`;
2. `merge_ini(&mut sections, latest, biz, channel)`;
3. `render_ini(&sections) -> String`;
4. `publish_ini(path, bytes)` (T28's helper).
Keep the section/key spelling rules and the header-less `[General]` rule.

## Rules

- Behavior must be byte-for-byte identical. Every existing test must pass
  unchanged except for call-site updates.
- Do not introduce traits, boxed futures, or a phase framework. Free functions
  and small structs are enough.
- Keep the numbered design comments with the extracted step; do not delete the
  rationale.
- Do not combine this with T35's parameter structs or T36's exit-code enum.

## Tests

- The full suite, especially `repair_offline`, `repair_machinery`, the purge
  tests, and the config writer tests.
- If a helper is now pure, add a focused unit test for one decision table per
  function (for example the trim choice, the purge classification, the INI
  merge). Do not duplicate the end-to-end coverage.
- No new timing test.
- `cargo test -p girpr` passes.

## Acceptance

- Each listed function is a short sequence of named steps.
- No behavior change; all existing tests pass.
- No new trait-object abstraction.
- The design comments survive.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(plan): split the plan phase into named steps`
2. `refactor(repair): split the execute phase by mode`
3. `refactor(inplace): split the in-place attempt into steps`
4. `refactor(purge): split walk and finish`
5. `refactor(ini): split parse, merge, render, publish`
6. `test: pin the extracted decisions`

## Notes

- This is the task most likely to create a large, hard-to-review diff. Keep
  each commit to one function and do not move unrelated code.
- If a helper needs more than one new parameter bag, stop and leave that
  function for T35 instead of inventing a struct here.
