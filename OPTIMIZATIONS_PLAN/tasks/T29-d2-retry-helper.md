# T29 - D2: one retry/backoff scaffold

Report: `../../OPTIMIZATIONS_REPORT.md` lines 645-670 (D2).

Dependencies: T09 (manifest concurrency). Keep the policy tables in their
modules; this task is the loop, not the classification.

## Goal

Retry/backoff is written four times: `hyp::get_node`, `hyp::get_direct`,
`sophon::fetch_manifest`, `sophon::stream_chunk_to` (with `try_stream_chunk`),
plus the per-file retry loop in `repair::file`. Each copy implements attempts,
last error, linear sleep, and terminal-vs-retryable classification with
slightly different details. One scaffold, with the policy supplied by the
caller, removes the divergence and gives one place to add jitter or change the
attempt count.

## Where to start

- `src/sleep.rs`: the `Sleep`/`NoSleep` seam.
- `src/hyp.rs`: `get_node` (around line 278), `get_direct` (around line 325),
  the retcode classifier (around line 179).
- `src/sophon/stream.rs`: `fetch_manifest` (around line 24),
  `stream_chunk_to` (around line 306), `try_stream_chunk` (around line 357),
  `Attempt`/`verdict` (around line 518), `retryable` (around line 564).
- `src/repair/file/mod.rs`: `MAX_ATTEMPTS` (around line 93), the per-file
  loop (around line 475), the sleep calls (around line 532).

## Steps

1. Add a small async helper, for example in `src/sleep.rs` or a new
   `src/retry.rs`:
   ```rust
   pub enum Retry<T> { Done(T), Fatal(anyhow::Error), Retry(anyhow::Error) }
   pub async fn retry<T, F, Fut>(
       attempts: u32,
       sleep: &dyn Sleep,
       mut attempt: F,
   ) -> anyhow::Result<T>
   where
       F: FnMut(u32) -> Fut,
       Fut: std::future::Future<Output = Retry<T>>;
   ```
   The loop:
   - calls `attempt(n)` for `n = 1..=attempts`;
   - `Done` returns immediately;
   - `Fatal` returns immediately with the error;
   - `Retry` sleeps `n` seconds (the current linear policy) and continues;
   - after the last attempt, returns the last retry error with context
     "after {attempts} attempts".
   Keep the error type `anyhow::Error` (all four sites already use it) or make
   the helper generic over `E` if the file loop needs a different type.
2. Do **not** move the retcode/status tables into the helper. Each module keeps
   its classifier and maps its own error to `Retry`/`Fatal`. The helper only
   owns attempts, sleep, and last-error handling.
3. Add jitter in one place. A small deterministic jitter derived from
   `n` and a per-run salt (for example the process id) plus `Sleep` means tests
   with `NoSleep` still assert counts, not durations. If jitter complicates the
   `Sleep` contract, record it as a follow-up rather than adding a random
   dependency.
4. Convert the four sites:
   - `get_node`/`get_direct`: attempts 5, linear sleep, retcode classification;
   - `fetch_manifest`: attempts 5, linear sleep, status/stream classification;
   - `stream_chunk_to`: attempts 5, linear sleep, status/stream classification;
   - the file loop: `MAX_ATTEMPTS` (or 1 under `--fail-fast`), linear sleep,
     and the file's own error classification. The file loop wraps the whole
     attempt, so it may need a different body type; keep the helper's shape
     general enough or write a second thin wrapper.
5. Preserve each site's log lines and attempt numbering. Do not change which
   error is fatal: a blob content-decode error stays fatal, a manifest content
   error stays retryable, a refused retcode stays fatal, and the file cap
   stays.
6. Tests that assert sleep durations must move to the `Sleep` seam and assert
   attempt counts. `NoSleep` is the standard seam.

## Tests

- Helper unit tests: success first try, success after retries, fatal stops
  immediately (one attempt), exhausting attempts returns the last error, and
  the sleep is called exactly `attempts - 1` times on exhaustion. Use a
  recording `Sleep`.
- Each converted site has a test that a retryable error is retried and a fatal
  one is not. The existing `sophon`/`hyp` tests likely cover the classifiers;
  keep them.
- The file loop's `--fail-fast` still caps attempts at one.
- `cargo test -p girpr` passes.

## Acceptance

- One retry loop; four call sites supply only classification/policy.
- Attempt counts, classification, and error messages are unchanged (except the
  new "after N attempts" context).
- Jitter, if added, is in the helper and does not make tests timing-dependent.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(retry): add one async retry scaffold`
2. `refactor(hyp): use the shared retry loop`
3. `refactor(sophon): use the shared retry loop`
4. `refactor(file): use the shared retry loop`
5. `test(retry): pin counts, fatal stops, and exhaustion`

## Notes

- A generic helper over different classifiers can become harder to read than
  four straight-line loops. Keep the policy tables separate and the helper
  small.
- Do not add a retry dependency; the existing `Sleep` seam is enough.
- T35/D9 may group the fetch parameters into a `ChunkFetch`; keep that
  signature work separate.
