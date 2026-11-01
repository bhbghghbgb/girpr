# T09 - P2: fetch manifests concurrently

Report: `../../OPTIMIZATIONS_REPORT.md` lines 301-327 (P2).

Dependencies: none (T08 may land first; do not rewrite its work).

## Goal

`phases::manifest::fetch_latest` and `local_reuse_map` loop over selected
manifests and `await` each `sophon::fetch_manifest`. A build has several
selected manifests (base game, res categories, selected audio languages), and
each is a multi-megabyte blob. Fetch them with a bounded concurrency and
preserve selection order in the result and in error reporting.

## Where to start

- `src/repair/phases/manifest.rs`: `fetch_latest` (around line 30),
  `local_reuse_map` (around line 52), module docs lines 1-11.
- `src/sophon/mod.rs` and `src/sophon/stream.rs`: `select_manifests`,
  `fetch_manifest`, `WantedManifest`, `build_local_chunk_map`.
- `src/repair/mod.rs`: `plan_phase` calls both functions around lines 194-203.
- `src/config.rs`: `RunCtx::jobs`.
- Existing manifest/artifact tests.

## Steps

1. Add a `jobs: usize` parameter to both functions (or a single explicit
   `limit: usize`), and pass `ctx.jobs` from `plan_phase`. Choose a dedicated
   cap, for example `let limit = jobs.clamp(1, 4);`, because one decoded
   manifest can be tens of MiB. Log the chosen limit once at `info`:
   `manifest concurrency={limit}`.
2. In `fetch_latest`:
   - collect `select_manifests(...)` into a `Vec`;
   - fetch with `futures::stream::iter(...)` + `.map(...)` +
     `.buffer_unordered(limit).collect::<Vec<_>>()` (or the equivalent
     `FuturesUnordered`);
   - each future returns its selection index plus the result;
   - write results into a `Vec<Option<Result<...>>>` indexed by selection
     order;
   - after draining, iterate in index order. Return the **first error in
     selection order**, with the existing `manifest {matching_field}` context.
     Do not return the first error by completion time.
   - push `(m.chunk_download.url_prefix.clone(), files)` in selection order.
     `build_plan` sorts files, so completion order must not leak into the plan.
3. In `local_reuse_map`:
   - use the same bounded concurrent fetch;
   - preserve selection order when assembling `wanted`;
   - keep best-effort semantics: a failed manifest logs the same warning and
     is skipped; do not fail the run.
4. Keep the memory bound explicit. Do not use `join_all` over every selected
   manifest without a limit. Do not move the decoded buffers into a global
   cache; the report's S1 decision forbids a manifest cache in `game_dir`.
5. Update the module doc: manifests are fetched concurrently, bounded, and the
   result/error order is selection order. T32 will sweep the rest of the docs;
   this one is coupled to the change and belongs here.

## Tests

- Existing manifest/artifact tests must pass unchanged.
- Add a mock-server test with at least three selected manifests where one fails
  and another succeeds. Assert the returned error names the first failing
  manifest in **selection order**, not the fastest failure. If the test server
  cannot delay responses, assert the deterministic ordering on a pure helper
  instead: extract the "collect indexed results, pick first error" step into a
  small function and unit-test it with out-of-order results.
- Add a test that `local_reuse_map` still returns the union of the successful
  manifests when one fails.
- Do not assert elapsed time; the gain is latency.

## Acceptance

- Manifest fetches are bounded and concurrent, not serial.
- The `per_manifest` order and the `local_reuse_map` contents are unchanged for
  successful runs.
- `fetch_latest` fails with the first error in selection order.
- `local_reuse_map` remains best-effort.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(manifest): fetch selected manifests with bounded concurrency`
2. `test(manifest): pin selection-order errors and best-effort local reuse`
3. `docs(manifest): describe bounded concurrent fetch ordering`

## Notes

- Bound the concurrency explicitly; an unbounded `join_all` turns latency into
  peak memory.
- Do not change the retry policy inside `fetch_manifest`; T29 will unify retry
  scaffolding later.
