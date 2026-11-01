# P4-03 — Parse `getBuild` once per capture

Report: `../../OPTIMIZATIONS_REPORT.md` lines 710-721 (D3).

Dependencies: none.

## Goal

Stop re-parsing the two `getBuild` bodies at multiple points in the capture
flow, and stop carrying related facts in parallel representations that can
drift. Keep the raw bodies for `api/` output.

Current path: `Captured::validate` parses both builds (and branches via
`branch_params`), then `run::capture` calls `old_build()`/`new_build()` again,
and `branch_params()` is parsed again in `run` after `HypClient::capture`
already parsed it.

## Where to start

- `src/hyp/mod.rs`: `Captured`, `CapturedBody`, `new_build`/`old_build`
  (around line 153), `branch_params` (around line 145), `validate` (around line
  170), `HypClient::capture` (around line 360).
- `src/run/mod.rs`: `validate()` call (around line 96), `branch_params()`
  (around line 110), `old_build()`/`new_build()` (around lines 159-165).
- `src/manifest/fetch.rs`: `wants` takes `&GetBuild`; `fetch_all` uses them.
- Tests in `src/hyp/mod.rs`, `src/run/mod.rs`, `src/manifest/fetch.rs`.

## Design

Introduce a parsed view that is created once and carried through the flow, e.g.
`ValidatedCapture { params: BranchParams, old_build: GetBuild, new_build:
GetBuild }`, returned by `Captured::validate`. `run::capture` keeps it and uses
it for logging, `manifest::fetch::wants`, and any later build access. Keep the
raw `CapturedBody`s on `Captured` for `emit::write_api`.

Do not remove or restructure `VersionPair`; it is the resolved tag/build-id
pair and is still needed.

## Steps

1. Change `Captured::validate` (or add `parse`/`validate_and_parse`) to return
   the parsed struct. Keep the tag-echo and storage checks in the same order
   and with the same error text.
2. In `run::capture`, bind the returned value; use its `params` and builds
   instead of calling `captured.branch_params()`, `captured.old_build()`, and
   `captured.new_build()` again.
3. If `HypClient::capture` already has `params`, pass it through rather than
   reparsing: either store it in the returned struct or return it alongside
   `Captured`. Prefer the smallest change that removes the second parse. Do
   not duplicate `BranchParams` in two fields that can drift.
4. Update test call sites of `validate()` for the new return type. Tests that
   only assert an error can ignore the `Ok` value.
5. Do not change `GetBuild`'s serde shape or the raw `api/getBuild.*.json`
   bytes.
6. If the change would require touching many hand-built `Captured` fixtures,
   keep a `validate_only()` wrapper for tests or update fixtures mechanically;
   do not add a cache field that is invisible to the type system.

## Tests

- Existing validate tests pass, including tag-echo and storage refusals.
- A test asserts the returned builds equal the parsed raw bodies.
- The run-level path uses the returned builds; if a parse counter/seam exists,
  assert `getBuild` is parsed once per capture on the happy path.
- `manifest::fetch::wants` receives the same `GetBuild` values as before.
- Artifact bytes (`api/getBuild.*.json`) are unchanged.

## Acceptance

- One parse of each `getBuild` per capture on the normal path (and one of the
  branch body if it is also carried).
- Raw bodies still written byte-for-byte.
- No artifact/document change.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(hyp): return the parsed capture view from validation`
2. `refactor(run): carry the parsed params and builds through capture`
3. `test(hyp): pin the once-parsed build path`

## Notes

- Keep the raw bytes for `api/` and for evidence; do not serialize the parsed
  struct as a substitute.
- Do not reference the report or plan in code comments.
