# T32 - D7: reconcile documentation with the code

Report: `../../OPTIMIZATIONS_REPORT.md` lines 745-770 (D7).

Dependencies: T09, T16, T20. T33 follows this task in the ordering; do not
depend on it. Where a doc describes a parameter T33 will remove, state the
current truth and let T33 update the sentence in the same change that removes
the parameter.

## Goal

The codebase uses comments as design records, and several are stale:

- `phases::metadata` says the deprecated-file list is lazy and lives in the
  post-phase; it is fetched in `repair::plan_phase` and passed to the
  post-phase.
- `phases::manifest` says manifests are fetched "in order"; T09 makes that
  false.
- `inplace` says verification is "one more read"; it is one read but also a
  second digest over every chunk range (T12).
- The README's `--dry-run` paragraph describes purge listing but not that
  deprecated deletes and the audio move are (now) logged (T20).
- `write_config_ini` documents a dry-run mode production cannot reach (T33
  removes the parameter; until then, say it is unreachable).
- The lock paragraph and `--io-threads` row were updated by T17/T19; verify
  they agree with the code.

## Where to start

- `src/repair/phases/metadata.rs`: module docs around lines 1-30.
- `src/repair/phases/manifest.rs`: module docs and `fetch_latest` docs.
- `src/repair/file/inplace.rs`: module docs (around lines 1-45), the verify
  step (around line 317).
- `src/util/ini.rs`: `write_config_ini` docs (around line 264) and the
  `dry_run` parameter doc.
- `README.md`: `--dry-run`, the lock paragraph, the "Concurrent files" row.
- `src/repair/lock.rs`, `src/repair/mod.rs`: lock comments.
- `docs/` if it repeats the same claims.

## Steps

1. Read each listed doc and compare it with the code as it stands after the
   preceding tasks. Update the prose, not the behavior.
2. Metadata: say the deprecated list is fetched in the plan phase (only for a
   run that will reach the post-phase or a dry run) and passed in; remove the
   "lazy in post" claim.
3. Manifest: say fetches are concurrent and bounded, and that results are
   assembled in selection order so the plan is deterministic. If T09 chose a
   different error policy, describe that instead.
4. Inplace: say the first verify hashes the whole file (the gate) and also
   computes per-range digests; the per-range digests are used only when the
   whole-file gate fails. The final redo scan reads for the whole-file digest;
   say what it actually does after T12.
5. README dry-run: list all three post-phase steps and the "no config.ini
   bump" rule. Keep the output contract wording.
6. `write_config_ini`: state that `dry_run` is only reachable from
   `--purge-only` (not `--dry-run`) today; T33 will remove it. Do not describe
   a mode the code cannot enter.
7. Lock: confirm the README matches the T17 design (location, scope-free
   acquisition, recovery). `--io-threads`: confirm it matches T19 (primary
   `--jobs`, alias, bounds files).
8. Do not touch code or tests except for comment/doc text. If a doc is stale
   because the code is wrong, stop and report it rather than documenting a
   bug.

## Tests

- No behavior change; `cargo test -p girpr` must still pass.
- `cargo doc -p girpr --no-deps` (if run) must not gain warnings.
- A grep for the old claims ("lazy", "in order", "one more read", the old
  lock path) should find no remaining stale occurrence in the updated files.

## Acceptance

- Every doc listed above matches the code after the preceding tasks.
- No code behavior changes.
- The README's dry-run, lock, and jobs text is accurate.
- `cargo test -p girpr` passes.

## Commit plan

1. `docs: reconcile phase and writer comments with the code`
2. `docs: update the README for dry-run, locking, and jobs`

## Notes

- Update a doc in the change that makes it stale when possible. This task
  exists because several earlier tasks are already committed; do not create a
  second "doc pass" after this one.
- Do not add references to the report or plan in any doc.
