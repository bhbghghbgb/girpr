# T18 - A1: narrow the public surface

Report: `../../OPTIMIZATIONS_REPORT.md` lines 839-858 (A1).

Dependencies: T17. Do this after the behavioural fixes so the tests are not
rewritten twice.

## Goal

`lib.rs` exposes `biz`, `cli`, `config`, `hyp`, `logging`, `repair`, `report`,
`sleep`, `sophon`, and `util` as `pub`. The binary is the product; integration
tests are the only other consumer. Exposing `hyp`, `sophon`, and the repair
internals makes every internal type a compatibility surface and lets tests
couple to implementation details.

Reduce the surface to an intentional seam, and give tests one explicit,
documented way to construct a run and inspect its results.

## Where to start

- `src/lib.rs`: the module list and the re-exports.
- `src/cli.rs`: `Args`, `into_ctx`, `parse_audio_selection`,
  `normalize_audio_lang`, `is_audio_none`.
- `src/config.rs`: `RunCtx`, `RunFailure`, `RepairMode`.
- `src/report/mod.rs`: `Summary`, `OutputFormat`, `summary_record`, `Record`.
- `src/repair/test_support.rs`: the existing test helpers.
- Integration tests: `rg -n "use girpr::|girpr::" tests`.

## Steps

1. Decide the product seam. Keep public and re-exported at the crate root:
   - `run`, `RunCtx`, `RunFailure`, `Summary`, `OutputFormat`, `Biz`,
   - the CLI types needed to parse a command line (`Args`, the audio helpers),
   - `Level`/`girpr_log` if the binary still needs it.
2. Make the implementation modules private (`mod biz; mod cli; ...`) and keep
   the individual items `pub` only where the seam re-exports them. Do not
   leave a `pub mod hyp` / `pub mod sophon` / `pub mod repair::file` just
   because a test used it.
3. Add one explicit test seam, for example `pub mod test_support` in
   `lib.rs` (mark it `#[doc(hidden)]`), that re-exports exactly what the
   integration tests construct:
   - `Limits`, `PathScope`, `RepairMode`, `NoSleep`, `RealSleep`;
   - `SophonChunk`, `SophonChunkFile`, `SophonChunkManifest`, `ChunkSink`;
   - `summary_record`, `Record`, `Lock`, `EMPTY_MD5`.
   The types stay `pub` inside their private modules so the re-export works;
   the seam is the only path an external crate can use.
4. Keep `RunCtx` fields public: tests build it directly, and making them
   private would force a builder this task does not need. Do not add a builder
   for its own sake.
5. Rewrite integration-test imports to use the root re-exports or
   `girpr::test_support`. Move a test that needs a genuinely internal function
   (not just a type) into a `#[cfg(test)] mod tests` beside that function; do
   not widen the public seam to keep an integration test in `tests/`.
6. Do not change behaviour. If a test currently drives `fetch_manifest` or
   `RepairPlan` directly, either move it inward or drive the public `run` with
   the existing fixture server. Keep the fixture server; it is the public
   integration path.
7. Check `cargo doc -p girpr --no-deps` renders only the intended surface.

## Tests

- `cargo test -p girpr` and `cargo test --workspace` (the tools must still
  build; they do not depend on this crate).
- `cargo doc -p girpr --no-deps` must not warn about private-in-public items.
- No integration test may import a private module; the compiler is the check.

## Acceptance

- `hyp`, `sophon`, and the repair implementation modules are no longer public.
- The public seam is small, named, and documented.
- All existing tests pass with imports moved to the seam.
- No behaviour changes.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(lib): expose one intentional public seam`
2. `test: consume the public seam instead of internal modules`

## Notes

- A `#[doc(hidden)] pub mod test_support` is still public. It is acceptable
  because it is one named seam with a narrow list, not every internal module.
  Do not put production logic there.
- Do not gate the seam behind a feature that normal `cargo test` does not
  enable; a silently skipped integration suite is worse than a visible seam.
- This task touches many test imports. Keep the commits separate from any
  behaviour change so a revert is clean.
