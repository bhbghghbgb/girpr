# T39 - B3: use `futures-util` directly

Report: `../../OPTIMIZATIONS_REPORT.md` lines 991-1005 (B3).

Dependencies: none.

## Goal

`futures` is used for `StreamExt`, `stream::iter`, and one `Stream` bound.
`futures-util` provides those without the `futures` facade's executor/channel/
sink surface. Use the narrower crate.

## Where to start

- `Cargo.toml`: the `futures = "0.3"` dependency.
- `src/sophon/stream.rs`: `use futures::StreamExt;`, the `Stream` bound
  (around line 446), `futures::stream::iter` in tests (around lines 594, 693,
  716, 787).
- `src/repair/scan.rs`: `use futures::StreamExt;` and
  `futures::stream::iter` (around line 233).

## Steps

1. Replace the dependency with `futures-util = "0.3"` (default features are
   fine; do not enable the `sink`/`io` surfaces unless used).
2. Update imports:
   - `futures_util::StreamExt`;
   - `futures_util::stream::iter`;
   - for the `Stream` trait bound, use `futures_core::Stream` and add
     `futures-core = "0.3"` if `futures_util` does not re-export it. Do not
     rely on a transitive re-export; name the direct dependency.
3. Do not change any stream composition, buffering, or ordering. This is an
   import/dependency change only.
4. If a test uses a `futures` item not in `futures-util`, move that test to
   the equivalent `futures_util` item or keep a test-only `futures` dev-
   dependency with a comment. Prefer removing it.

## Tests

- `cargo test -p girpr` (the stream tests and the scan parallelism).
- `cargo tree -p girpr` no longer lists the `futures` facade (transitive
  `futures-*` crates are expected).
- `cargo check --workspace` still passes.

## Acceptance

- The main package depends on `futures-util` (and `futures-core` if needed),
  not the `futures` facade.
- No behavior change in streaming or scan concurrency.
- `cargo test -p girpr` passes.

## Commit plan

1. `build(deps): use futures-util instead of the futures facade`

## Notes

- This is a dependency-graph win, not a performance change. Keep it separate
  from P2/P3 code changes.
- Do not add `futures-executor`; the crate uses tokio for execution.
