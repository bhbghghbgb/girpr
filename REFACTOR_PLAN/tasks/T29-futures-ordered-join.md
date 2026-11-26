# T29 — Bounded ordered join → futures

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §CAP-6 (lines 725–736) — the `JoinSet` +
  `HashMap<task::Id, slot>` bounded ordered join becomes
  `futures::stream::iter(...).map(...).buffer_unordered(jobs)` with ordered collection;
  the panic/JoinOutcome vocabulary stays.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md` — the fetcher retry loop must stay (line 735–736).

## Scope
- `tools/capture/http/src/tasks.rs` — `join_failure` (`:14–33`), `JoinOutcome` (`:35–50`),
  `join_ordered` (`:69–136`), `join_blocking` (`:138–154`), tests `:160–236`.
- `http/src/fetcher.rs:449–509` (consumer); `tools/capture/http/Cargo.toml`.

## Implementation plan
1. Validate the current semantics: bounded concurrency `jobs`, results in input order,
   panic recovery with payload downcast, and cancellation behavior. Keep
   `JoinOutcome`/`join_failure` as the error vocabulary.
2. Replace `join_ordered` with `stream::iter(...).map(...).buffer_unordered(jobs)` plus
   ordered collection. Keep panic recovery via `JoinSet` or `catch_unwind` where the
   library cannot express it — this is partial delegation (§CAP-6 line 730).
3. Keep `join_blocking` and its call site behavior; add `futures 0.3.34` to
   `tools/capture/http/Cargo.toml` (futures-util is already used elsewhere).
4. Keep all four tests at `:160–236`, checking especially cancellation semantics
   (§CAP-6 line 734). Do not touch the retry loop at `fetcher.rs:788–861`.

## Constraints / validation
- The panic-free sink and typed `FetchFailure.seq` contract must survive (§CAP-6 lines
  735–736). No `reqwest-retry`/`backoff` adoption.
- Ordered results and bounded concurrency are both required — verify both in tests.

## Done when
The slot/`JoinSet` reimplementation is replaced by `buffer_unordered` with ordered
collection, panic recovery and cancellation behavior are covered, and the four tests
plus fetcher tests pass. If a premise is stale or a keeper would change, **stop** and
record `skipped`.
