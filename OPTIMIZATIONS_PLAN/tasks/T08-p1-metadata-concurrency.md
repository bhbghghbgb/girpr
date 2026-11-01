# T08 - P1: overlap the metadata calls

Report: `../../OPTIMIZATIONS_REPORT.md` lines 276-299 (P1).

Dependencies: none.

## Goal

`phases::metadata::fetch` serializes five calls that are mostly independent:
`getGameConfigs` -> `getGameBranches` -> `getBuild(latest)` -> `getBuild(local)`,
then the local ignore/blacklist reads. Only two dependencies are real:
`getBuild` needs the branch, and the local build needs the branch and the local
`config.ini` version. `getGameConfigs` and `getGameBranches` can start
together, and the two `getBuild` calls can run together. Overlap the local file
reads with the network work.

## Where to start

- `src/repair/phases/metadata.rs`: `fetch` (around line 82); `Meta` fields;
  `hyp_client_for`.
- `src/hyp.rs`: `HypClient::game_config`, `game_branch`, `chunk_build`; they are
  independent async methods.
- `src/util/ini.rs`: `read_game_version`, `read_ignore_categories`,
  `read_blacklist` (sync).
- `src/repair/audio.rs`: `resolve_effective_audio` (sync, local read).
- `src/repair/mod.rs`: `plan_phase` calls `metadata::fetch(ctx, &game_dir)`.
- Existing metadata tests in `tests/artifact_bridge.rs`, `tests/repair_offline.rs`,
  and the mock server under `tests/artifact_server.rs`.

## Steps

1. Start the local version read (`util::read_game_version`) on the blocking
   pool at the top, together with the first network wave. It only needs
   `game_dir`; move an owned `PathBuf` into the closure. Keep the existing
   error mapping (`RunFailure::usage` for read/validation errors).
2. Run `game_config()` and `game_branch()` with `tokio::join!` (not
   `try_join!`); then inspect the results deterministically:
   - if `game_config` failed, return that error;
   - otherwise if `game_branch` failed, return that error.
   This keeps the mode refusal authoritative and makes the error choice
   independent of which future happened to finish first.
3. Check `config.default_download_mode` **after** both calls but before using
   the branch for `getBuild`. This means a non-chunk mode may issue one
   `getGameBranches` call it discards; that is the accepted trade. Keep the
   usage error text and exit `1`.
4. After config is available, start the ignore/blacklist/audio local reads on
   the blocking pool. They need only `game_dir` and the cleaned config dirs;
   clean the three audio dirs first, as today.
5. Run `chunk_build(&branch, None)` and, when a local version exists,
   `chunk_build(&branch, Some(lv))` with `tokio::join!`. The latest call is
   fatal; the local call remains best-effort: log the warning and set
   `local_build = None` on error. If there is no local version, do not issue
   the local build.
6. Join the local reads with the build wave, not after it. Preserve every
   validation error (ignore/blacklist parse, audio refusal, `--audio` scope
   contradiction) and its existing exit class.
7. Keep the log lines. They may interleave because of concurrency; each line
   must still name its own fact. Do not log a "done" line before its value is
   available.

## Tests

- Existing offline/artifact tests must produce the same `Meta`, `REPORT`, and
  exit codes.
- Add one mock-server test for error precedence: a non-chunk `default_download_mode`
  plus a branch endpoint that would fail. The run must be the usage error
  (`1`) from the mode check, not the branch's metadata error (`2`).
- Add one test that a missing local version does not issue a local
  `getBuild`; the existing tests may already assert the request count. If the
  request log is not available, assert the observable result (`local_build =
  None`, reuse map empty) instead.
- Do not add wall-clock/timing assertions. The gain is latency, not output.

## Acceptance

- The two first-wave calls overlap; the two build calls overlap; local reads
  overlap with network work.
- Error classes and messages are unchanged for the same failures.
- The mode refusal still wins over a branch/build failure.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(metadata): overlap independent server and local reads`
2. `test(metadata): pin error precedence and the no-local-build path`

## Notes

- Do not use `try_join!` if it makes the winning error depend on poll order;
  the deterministic check after `join!` is the point.
- Retry sleeps are per call and now overlap. That is a gain, but log lines from
  different calls may interleave; do not serialize them again just to keep the
  old order.
