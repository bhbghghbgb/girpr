# T03 — Audit `opts()` → memory helpers in girsync semantic suites

Priority P0 · crate girsync · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L272–L280` (§6.1 B5),
  and `L293–L295` (§6.1 C7).
* Report priority row: `docs/08-test-runtime-optimization.md:L471` (§8, P0).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T03-girsync-opts-mem-audit.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

`tests/helpers.rs` uses the file-backed `opts()` ~20 times and never the
`*_mem` variants that already exist for exactly this purpose. Many tests only
read rows/verdicts through `recs_of`/`hashed`, so they pay a file-backed cache
open for no reason. Switch every such test to the in-memory backend while
keeping `File` where backup siblings or raw cache bytes are the assertion
(cache-focused cases in `compare`, `compare_self`, `update`, `migration_lock`).
Also stop using `pair_with(..., warmed: true)` (two file-backed `cmd_update`
opens per side) when a test only needs files on disk.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/sync/tests/common/mod.rs:273` — `opts()`; `:297` — `opts_mem()`.
* Related in-memory helpers: `pair_mem`, `sync_mem`, `compare_mem`; `recs_of`'s
  memory-first lookup around `:510–516`.
* `pair_with(..., warmed: true)` around `:624–667`.
* `hash_mode` has both `asking()` (file) and `asking_mem()`.
* `tests/helpers.rs` — the ~20 `opts()` call sites to audit.

Possible commits (one per suite is reasonable):

1. For each semantic suite, replace `opts()`/`pair()`/`sync()`/`compare()` with
   the `*_mem` equivalents where the assertions are rows/verdicts.
2. Leave the cache-byte assertion suites on `File`; add a brief comment at each
   kept site explaining why the file backend is required.
3. Replace `warmed: true` with `warmed: false` + targeted hashing, or `pair_mem`,
   where a full cache warm is not asserted.

## Acceptance criteria

* Only tests that assert on-disk cache bytes or backup siblings use `File`;
  every other audit site uses the memory backend.
* All audited suites still pass; no assertion was weakened to make a switch work.
* No test loses coverage of the file-backed path that was the point of the test.

## Guardrails

* Validate against current code first; the helper set may have evolved. If the
  audit target no longer exists, stop and mark `skipped` with a note.
* This is test-harness work; do not change product cache selection or the
  existing test hooks.
* This is an audit — be conservative. If it is unclear whether a test needs the
  file backend, keep it on `File` and note it rather than risking a false pass.
* You may batch commits per suite. Run the affected girsync suites; report
  follow-up ideas without implementing them.
