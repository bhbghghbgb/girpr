# T02 — Default `run_binary_raw` to the memory cache backend

Priority P0 · crate girsync · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L261–L270` (§6.1 B).
* Report priority row: `docs/08-test-runtime-optimization.md:L471` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T02-girsync-run-binary-raw-memory-cache.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`run_binary_raw()` in `tests/sync.rs` does not set `GIRSYNC_TEST_CACHE=memory`
while its sibling `run_binary()` does, so ~20 callers open a full 1 MB file-backed
redb cache (plus lock sidecar + meta commit) per binary run — ~200–500 ms each on
HDD, for tests that only assert rows/verdicts. Default the raw helper to the
memory backend and remove the now-redundant per-caller env entries. Opt out only
where the test genuinely inspects on-disk cache bytes.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/tests/sync.rs:67–88` — `run_binary()` (sets the memory env).
* `tools/sync/tests/sync.rs:95–113` — `run_binary_raw()` (missing it).
* Redundant per-caller entries to clean up: `:438`, `:486`, `:536`, `:593`,
  `:728–733`, `:901`, `:934`, `:982`, `:1034`, `:1208`, and the emit sites
  around `:1182` / `:1208`.
* Keep the file backend where bytes matter: the nested-case-rename cache test
  (around `:820`) and the real-fsync case (`:1735`, `:1743`, `:1763`, `:1818`,
  where `GIRSYNC_TEST_NO_FSYNC=0` is set). Several of these are in T20's
  guardrail scope — do not change fsync behavior.

Possible commits:

1. Make `run_binary_raw` inject `GIRSYNC_TEST_CACHE=memory` by default, with a
   way for specific callers to override/opt out (e.g. an env slice that is
   appended after the default, so an explicit value wins — confirm how the
   child env is built).
2. Remove the redundant `("GIRSYNC_TEST_CACHE", "memory")` entries from callers
   that now inherit the default; keep the env on the few opt-out cases.
3. Verify the opt-out cases still open a file cache and assert on its bytes.

## Acceptance criteria

* All sync tests that assert verdicts/rows no longer create a file-backed cache
  through `run_binary_raw`.
* Tests that assert on-disk cache bytes still use the file backend and still pass.
* No duplicate env entries remain; the helper has one clear default.

## Guardrails

* Validate against current code first; if the two helpers or the env plumbing
  have changed, stop and mark `skipped` with a note.
* Test-only change. Do not touch the cache-backend selection in product code
  (`tools/sync/src/config/common.rs`, `src/cache/*`) or the existing no-fsync /
  durability hooks.
* Preserve the `GIRSYNC_TEST_NO_FSYNC=0` opt-outs intact.
* You may improvise the opt-out API if cleaner. Run the girsync `sync` tests to
  verify; report follow-up ideas without implementing them.
