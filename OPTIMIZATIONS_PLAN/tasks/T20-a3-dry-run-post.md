# T20 - A3: exercise the post-phase in `--dry-run`

Report: `../../OPTIMIZATIONS_REPORT.md` lines 880-902 (A3).

Dependencies: none. T22 (exit path) and T23 (log buffering) follow it, so keep
the post-phase's dry-run discipline inside `post::run`.

## Goal

A dry run answers "what would this run do?", but today it never calls the
deprecated-delete or audio-move steps. `post::run` already threads a `dry_run`
flag through each destructive step; the only reachable caller is
`--purge-only`, not `--dry-run`. Call the post-phase for a dry run and make it
stop before the `config.ini` bump and before counting deleted/moved bytes.

## Where to start

- `src/repair/mod.rs`: `wants_deprecated` (around line 303), the `readonly`
  gate around `phases::post::run` (around line 595), the purge-only call
  (around line 553).
- `src/repair/phases/post.rs`: `run` (around line 48), `delete_deprecated`
  (around line 147), `move_audio_cache` (around line 279), the `config.ini`
  write (around line 110).
- `src/config.rs`: `RunCtx::readonly` (around line 191), `dry_run`,
  `check_only`, `print_plan`.

## Steps

1. Fetch the deprecated list for a dry run as well:
   `wants_deprecated = ctx.dry_run || (!readonly && !ctx.print_plan)`.
   `--check-only` and `--print-plan` still skip the network call. Keep the
   request after the "include matched nothing" guard so a typo still costs no
   network.
2. In `execute_phase`, replace the `if !readonly { post::run(...) }` gate with
   an explicit predicate: run the post-phase when the run actually mutates, or
   when it is a dry run that is not `--check-only`/`--print-plan`.
   `--purge-only` keeps its existing early call. Do not run the post-phase for
   `--check-only` or `--print-plan`.
3. In `post::run`, after `delete_deprecated`, `move_audio_cache`, and the
   optional purge pass, return **before** `write_config_ini` when
   `ctx.dry_run`. The post-phase currently reaches the bump for a dry run; a
   dry run must not touch `config.ini`.
4. Keep the byte accounting honest:
   - `delete_deprecated` already adds nothing in dry-run mode;
   - `apply_purge(.., true)` returns the bytes it would delete; a dry run must
     not add them to `summary.deleted_extra_bytes`. Check the current code and
     gate the `fetch_add` on `!dry`;
   - the audio move in dry-run only logs, so it contributes nothing.
5. The post-phase's `PostOutcome` must distinguish "dry run listed what it
   would do" from "a delete ceiling blocked the real pass". Reuse
   `DeletedNothing` for the dry-run purge result if it fits; otherwise add a
   variant and update `finish`. Do not report a limit that a dry run never hit.
6. Update the README's `--dry-run` paragraph: it now lists deprecated deletes,
   the audio cache move, and the cleanup, and still does not bump
   `config.ini`.
7. Do not change `--purge-only`'s real behaviour. It already passes
   `dry = ctx.dry_run`.

## Tests

- A `--dry-run` with a deprecated entry and an audio cache file logs
  "would delete deprecated …" and "would move audio cache …", and leaves both
  files and `config.ini` untouched.
- A `--dry-run` with `--purge-after` logs the would-delete lines and leaves
  `deleted_extra_bytes` at zero.
- A real run still deletes/moves and still bumps `config.ini`; the existing
  post tests cover this.
- `--check-only` and `--print-plan` still do not issue the deprecated request
  and still do not run the post-phase.
- Update tests that assert dry-run silence: the new "would …" lines are the
  intended output change.
- `cargo test -p girpr` passes.

## Acceptance

- `--dry-run` runs the post-phase's read-only paths and stops before the
  `config.ini` bump.
- No bytes are counted as deleted or moved in a dry run.
- `--check-only` and `--print-plan` remain post-phase-free.
- The README describes the new dry-run coverage.
- `cargo test -p girpr` passes.

## Commit plan

1. `feat(post): exercise the post-phase in --dry-run`
2. `fix(post): keep dry-run byte accounting at zero`
3. `test(post): pin dry-run deprecated and audio-move output`
4. `docs: describe what --dry-run now covers`

## Notes

- The post-phase's own `dry_run` discipline is deliberate. Keep consulting it
  inside each destructive step; do not rely only on the caller's gate.
- A dry run must remain safe on a fresh install where `game_dir` does not
  exist. The post-phase already treats a missing directory as empty.
