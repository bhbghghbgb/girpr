# T19 - A2: make `--jobs` the primary name for concurrent files

Report: `../../OPTIMIZATIONS_REPORT.md` lines 860-878 (A2).

Dependencies: T11 (the runtime sizing/worker change). `RunCtx.jobs` already
exists; the CLI flag still says `--io-threads`.

## Goal

`--io-threads` bounds in-flight file tasks, not tokio worker threads. Rename
the user-facing flag to `--jobs`, keep `--io-threads` as a compatibility alias,
and stop the flag from being an unbounded resource foot-gun. The documented
meaning must match the loop: "concurrent files; chunks inside a file stay
sequential".

## Where to start

- `src/cli.rs`: `Args::io_threads` (around line 41), the validation in
  `into_ctx` (around line 256), `RunCtx.jobs` assignment (around line 285).
- `src/config.rs`: `RunCtx.jobs` (around line 131) and its doc.
- `src/main.rs`: the start log already prints `jobs`.
- `README.md`: the "Concurrent files" row and any `--io-threads` example.
- `tests/cli_contract.rs`: flag parsing/help assertions.

## Steps

1. Rename the `Args` field to `jobs` (or keep the field and change only the
   clap attribute; prefer renaming so the internal name matches the flag).
   Use `#[arg(long = "jobs", visible_alias = "io-threads", default_value_t =
   4)]`. Clap will accept both spellings; help shows `--jobs`.
2. Keep the `>= 1` validation. Its message should name `--jobs` (and mention
   the alias only if the value came from it, which clap does not expose
   cheaply; do not add argument-introspection just for the wording).
3. Add a soft upper bound/warning rather than a hard cap:
   - compute the available parallelism (`std::thread::available_parallelism`)
     or, if the runtime is already up, the worker count;
   - if `jobs` exceeds it, `tracing::warn!` once with the requested value and
     the worker count, saying that more file tasks than workers can starve
     timers and network polling and that the flag bounds files, not threads.
   Do not refuse the run; a power user may have a reason.
4. Update `RunCtx.jobs` docs and the README row. Remove claims that the flag
   sets worker threads. State that `--io-threads` is a deprecated alias.
5. Do not change the repair loop's use of `ctx.jobs` or the default of 4.
6. Keep the summary/log output's `jobs=` key unchanged; it is the data plane.

## Tests

- `Args::try_parse_from` accepts `--jobs 8` and `--io-threads 8` and produces
  the same `RunCtx.jobs`.
- `--jobs 0` is exit `1` with a message naming the flag.
- A `--jobs` value above the worker count logs the warning but still runs;
  assert the warning text only if the test has a subscriber that captures it,
  otherwise test the predicate/helper directly.
- Existing CLI contract tests that mention `--io-threads` keep passing through
  the alias; update expected help text to `--jobs`.
- `cargo test -p girpr` passes.

## Acceptance

- `--jobs` is the documented primary flag; `--io-threads` still works.
- The help and README describe "concurrent files", not worker threads.
- Zero is still a usage error; an excessive value warns and proceeds.
- No change to the default or to the repair loop.
- `cargo test -p girpr` passes.

## Commit plan

1. `feat(cli): make --jobs the primary concurrent-files flag`
2. `docs: describe the file-concurrency flag accurately`
3. `test(cli): pin the alias, the zero check, and the warning`

## Notes

- This is a CLI compatibility change. Keep the alias for at least this work;
  removing it is a later, separate decision.
- Do not map `--jobs` to tokio worker threads; that is the misunderstanding the
  flag currently invites, and T11 already sized the runtime explicitly.
