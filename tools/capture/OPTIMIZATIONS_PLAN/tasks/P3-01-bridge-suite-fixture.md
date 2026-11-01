# P3-01 — Verify once and share the bridge fixture

Report: `../../OPTIMIZATIONS_REPORT.md` lines 481-503 (C1), 590-619 (C8),
754-784 (E1-E3, E5). Test-only.

Dependencies: P2-04.

## Goal

Cut the bridge suite's repeated work: `tests/artifact_bridge.rs` calls
`bridge()` per test, which opens the bundle, runs `gircap verify`, and binds four
listeners. Six tests therefore pay six verifies and 24 binds. The bundle is
read-only and immutable during the suite.

## Where to start

- `tests/artifact_bridge.rs`: `bundle()`, `patchmaps()`,
  `assert_bundle_verifies`, `bridge()` (around line 278), and every test that
  calls `bridge()`.
- `tests/artifact_bridge/server.rs`: `Bridge::start`, `Bridge::hits`,
  `chunk_ids`, `chunk_requests`, `Drop`.
- `tests/artifact_bridge/bundle.rs`: `Bundle::open`, `Patchmaps::read`.
- `tests/artifact_server.rs`: keep using `Bridge::start` per mini-bundle; this
  task is about the real-bundle suite only.

## Design

Create one suite-level fixture:

- A process-wide `tokio::sync::OnceCell<Suite>` (or equivalent) that holds the
  opened `Bundle`, parsed `Patchmaps`, and the started `Bridge`.
- `Suite::get().await` opens/parses/verifies/binds once.
- `assert_bundle_verifies` runs inside the initializer, once.
- The request log is shared. Add a log checkpoint API so each test can ignore
  earlier tests' requests:
  - `Bridge::log_len() -> usize`
  - `Bridge::hits_since(start) -> Vec<Hit>`
  - `Bridge::chunk_ids_since(start)` / `chunk_requests_since(start)`
  or a small `BridgeRun { bridge, start }` returned per test.
- Tests use their checkpoint for all "what did this run request?" assertions.
  They may still read the shared `Bundle`/`Patchmaps` read-only.
- Each test still creates its own `Scratch` tree; per-capture working state is
  not shared.

## Steps

1. Add the checkpoint API to `tests/artifact_bridge/server.rs` (test-only
   module, no production code).
2. Add the suite fixture and replace `bridge().await` with
   `suite().await` / `suite_run().await` in `tests/artifact_bridge.rs`.
3. Remove the per-test `bundle()`/`patchmaps()` re-reads where the suite
   fixture already holds them. Keep helper functions if they are thin
   accessors.
4. Audit every `bridge.hits()` / `chunk_ids()` / `chunk_requests()` call and
   switch it to the `_since` form with the test's checkpoint.
5. Do not change what each test asserts, the scratch copy logic, the pre-pass
   run, or the real run.
6. Keep `Bridge::start` per-bundle for `artifact_server.rs`; do not force the
   synthetic tests through the suite fixture.

## Tests

- `cargo test -p girpr --test artifact_bridge` compiles.
- With a real bundle (or the existing ignored harness), the suite runs one
  `gircap verify` and one set of listener binds for all capture tests. Add a
  log line or a counter in the fixture so this is observable, but do not assert
  wall-clock time.
- Each test's request assertions still see only its own run's hits.
- `cargo test -p girpr --test artifact_server` is unchanged and passes.

## Acceptance

- One verify and one four-listener bridge per suite, not per test.
- Per-test request assertions remain isolated via checkpoints.
- No production code changes.
- The bridge suite still fails when a capture is wrong.

## Commit plan

1. `test(bridge): share one verified bundle and one listener set`
2. `test(bridge): scope the request log per test with checkpoints`

## Notes

- This suite is `#[ignore]`d and expensive; correctness of isolation matters
  more than the exact wall-clock win.
- Do not collapse the pre-pass and real run; the two failure modes are
  deliberate.
- Do not reference the report or plan in code comments.
