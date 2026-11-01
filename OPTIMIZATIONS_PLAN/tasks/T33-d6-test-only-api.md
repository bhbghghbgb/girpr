# T33 - D6: remove test-only production API and an unreachable parameter

Report: `../../OPTIMIZATIONS_REPORT.md` lines 725-743 (D6).

Dependencies: T20 (dry-run post-phase). T32 precedes this task and documents
the current shape; update any sentence it leaves about the removed item in the
same commit.

## Goal

Three pieces read as production surface but are test-only or unreachable:

- `report::Record::event()` duplicates the lowercase/JSON event naming that the
  JSON renderer builds inline, and is used only by tests.
- `RepairPlan::expected_paths()` is a convenience used mainly by tests;
  `expected_rels()` is the allocation-free view.
- `write_config_ini(..., dry_run: bool)` is always `false` from production;
  read-only runs never reach the post-phase, and `--purge-only` passes the real
  `ctx.dry_run`.

Make the surface honest: one event-name implementation, no test-only
convenience as public production API, and no unreachable parameter.

## Where to start

- `src/report/record.rs`: `event()` (around line 144), `json()` (around line
  177), `push_json_event` (around line 237), the tests.
- `src/report/tests.rs`: uses of `event()`.
- `src/repair/phases/plan.rs`: `expected_paths()` (around line 170),
  `expected_rels()` (around line 164), tests (around lines 651, 734, 770).
- `src/repair/purge.rs`: `expected_rels()` call (around line 104) and tests.
- `src/util/ini.rs`: `write_config_ini` (around line 264), the `dry_run` block
  (around line 405), tests and callers.
- `src/repair/phases/post.rs`: the `write_config_ini(..., false)` call (around
  line 110).

## Steps

1. `Record::event`: either
   - make `json()` call `event()` so there is exactly one lowercase
     implementation, and keep `event` public; or
   - remove `event()` and make the tests assert the rendered JSON.
   Prefer the first if the renderer can use it without allocating twice; the
   report says one event-name implementation. Do not leave two spellings.
2. `expected_paths`: remove it if no production caller remains. Update tests to
   compare `expected_rels().collect::<Vec<_>>()` or a sorted set built from it.
   If an integration test needs an owned set, build it in the test. Do not
   replace it with another public convenience.
3. `write_config_ini`: remove the `dry_run` parameter and its
   `if dry_run { ... return }` branch. The function always writes. Update the
   post-phase call and every test. If a test used `dry_run = true` to observe
   rendering without writing, extract a pure `render_config_ini(...) ->
   String` (or a private helper) and test that, so no production parameter
   exists just for tests.
4. Keep the public write path's durability and temp naming from T28. Do not
   change the rendered bytes.
5. If T32's doc says `dry_run` is unreachable, update that sentence in the same
   commit; the parameter no longer exists.

## Tests

- `Record` JSON output is byte-identical for every existing record before and
  after the event-name change. The existing record tests can assert the full
  JSON line.
- `expected_paths` callers are gone; the purge still receives the same rels.
- `write_config_ini` produces the same `config.ini` and still syncs; no test
  can pass a dry-run flag because the parameter is gone.
- `cargo test -p girpr` passes.

## Acceptance

- One JSON event-name implementation.
- No test-only `expected_paths` in production API.
- `write_config_ini` has no unreachable `dry_run` parameter.
- Output bytes and `config.ini` content are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(report): use one event-name implementation`
2. `refactor(plan): drop the test-only expected_paths helper`
3. `refactor(ini): remove the unreachable dry-run parameter`
4. `test: update callers for the narrowed surface`

## Notes

- Removing a public item is allowed here because the binary is the product and
  T18 already narrowed the surface. Do not keep a `#[cfg(test)]` copy in the
  production module if the tests can use the renderer.
- Do not change `--purge-only`'s no-bump behavior; that is a post-phase
  decision, not this parameter.
