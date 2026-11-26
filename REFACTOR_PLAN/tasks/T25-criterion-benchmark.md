# T25 — Benchmark harness → criterion

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-8 (lines 624–634) — the hand-rolled
  `Instant`/`println!` benchmark harness with `harness = false` should become
  `criterion` with `criterion_group!`/`criterion_main!`, three shapes as `BenchmarkId`s
  and `iter_batched` to separate fixture construction from measured work.
- Validate assumptions first: §A1 (15–16), §A2 (17–18). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/Cargo.toml:27–33` (bench declaration and `harness = false`).
- `tools/sync/benches/tree_shapes.rs:88–195` (the manual harness).
- No tests pin benchmark output (§SYNC-8 line 632).

## Implementation plan
1. Validate that the three tree shapes are the only measured cases and that fixture
   build time is currently folded into the measurement. Preserve both facts in the new
   benchmark.
2. Add `criterion 0.8.2` as a bench/dev dependency, remove `harness = false`, and define
   the three shapes as `BenchmarkId`s.
3. Use `iter_batched` so fixture construction is outside the measured region; let
   criterion provide warm-up/statistics/regression output in place of the printed
   `files_per_s`/`mib_per_s`.
4. Confirm `cargo bench --no-run` (or equivalent) resolves and that no normal build or
   test is affected.

## Constraints / validation
- Only the bench declaration changes; no product path changes (§SYNC-8 lines 632–633).
- Do not add criterion to normal dependencies; keep it bench/dev-only.
- Re-check the version (the report corrects an earlier figure — §A11 lines 44–47).

## Done when
The hand-written harness is gone, criterion measures the three shapes with fixture
construction excluded, and the bench target resolves. If a premise is stale or a keeper
would change, **stop** and record `skipped`.
