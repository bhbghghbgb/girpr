# T37 - B1: trim the tokio feature set

Report: `../../OPTIMIZATIONS_REPORT.md` lines 952-969 (B1).

Dependencies: none. Independent build change; review with the other build
tasks.

## Goal

`tokio` is pulled with `features = ["full"]`. The crate uses the multi-thread
runtime, macros, time, `spawn_blocking`, and task ids. It does not use tokio's
net, process, signal, fs, or sync surfaces directly (reqwest brings its own).
Trim the normal dependency to the features actually used.

## Where to start

- `Cargo.toml`: the `tokio` dependency.
- `src/main.rs`: `#[tokio::main]` (multi-thread by default).
- `src/sleep.rs`: `tokio::time::sleep`.
- `src/repair/file/mod.rs`: `tokio::spawn`, `spawn_blocking`,
  `tokio::task::try_id`, `JoinHandle`/`JoinSet` if present.
- `src/repair/scan.rs`: `spawn_blocking`.
- `tests/`: `#[tokio::test]`, `tokio::process::Command` (test-only).

## Steps

1. Start with the smallest set the library needs:
   `["rt-multi-thread", "macros", "time"]`. Add `rt` if `spawn_blocking`/task
   APIs require it separately on the pinned version (they are part of `rt`;
   verify with `cargo check`).
2. If a build error names a missing feature, add only that feature and note
   why. Do not re-add `full`.
3. Tests use `tokio::process::Command` and `#[tokio::test(flavor =
   "multi_thread")]`. If `process` is needed only by tests, add a
   `[dev-dependencies] tokio = { version = "1", features = ["full"] }` entry
   so test builds get it without widening the normal graph. Cargo unifies
   features for a test build; the normal `cargo build` stays trimmed.
   If a self-duplicate is rejected by the resolver, put `process` in the
   normal list and document why.
4. Do not remove `tokio` from `tools/`; this task is the main package only.
   The workspace includes the tools, but their `full` feature is their own
   dependency and is out of scope.
5. Verify with `cargo tree -p girpr -e features` (or `cargo build -p girpr`)
   that the feature list is the intended one.

## Tests

- `cargo build -p girpr` (normal, trimmed features).
- `cargo test -p girpr` (dev features may be broader; all tests must pass).
- `cargo check --workspace` still passes.
- If the host has `cargo nextest`, run it as the repo's configured runner.

## Acceptance

- The normal `girpr` dependency does not use `features = ["full"]`.
- Every feature in the list is justified by a use in the crate.
- Tests still build and pass.
- No change to runtime behavior.

## Commit plan

1. `build(deps): trim the tokio feature set`

## Notes

- A missing feature is an immediate compile error, so this is low-risk. Keep
  the list honest as code changes.
- Do not remove `reqwest`'s features; it has its own `json`/`stream` needs.
