# P1-04 — A14 scans `game/` on disk

Report: `../../OPTIMIZATIONS_REPORT.md` lines 150-166 (A6).

Dependencies: P0-01.

## Goal

Make A14's claim true: `game/` contains only `config.ini`, the audio scan file,
and captured paths. Today `emit::selfcheck::closing` builds the set from the
`written` vector, so a stray file that existed before the run is invisible.

## Where to start

- `src/emit/selfcheck.rs`: `closing` (around line 454), specifically the A14
  block that builds `game_files` from `written`.
- `src/emit/index.rs`: `walk` and `rel` (around line 245/283) for a sorted
  recursive walk.
- `src/emit/closeout.rs`: `close` passes `&written`; keep the parameter if other
  rows still need it, otherwise narrow it.
- `src/run/mod.rs`: what paths are written under `game/` (config, scan file,
  captured files) to confirm the allowed set.

## Steps

1. Replace the `written`-derived `game_files` set with a walk of
   `dir.join("game")`. Convert each path to a `/`-separated path relative to
   `game/` using `emit::index::rel` or `strip_prefix`. Do not include the
   `game` component itself.
2. Keep the existing allowed set: `config.ini`, `scan_dir`, and every captured
   path after `paths::escape_path`. A missing `game/` directory is a failure
   row, not a panic; use the walk error in the actual text.
3. Preserve the A14 wording and row shape. The detail should keep naming the
   file count and the first stray paths.
4. Decide whether to reconcile the walk against `written` as an additional
   detail. Do not add a second assertion id. If a written path is absent from
   disk, A16/A18 will catch it; A14's job is the stray direction.
5. Keep determinism: the walk is sorted, and the stray list should be sorted.

## Tests

- Existing A14 tests should still pass; if they built `written` synthetically,
  switch them to a real temporary `game/` tree.
- New test: a pre-existing file under `game/` that is not allowed produces an
  A14 failure naming it.
- New control: the same tree without the stray file passes.
- If the P0-01 characterization test exists, flip its expectation.

## Acceptance

- A14 is derived from the tree on disk, not from the writer's intent.
- A dirty or reused bundle directory can now fail A14 (intended).
- The normal fresh-capture path is unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(selfcheck): derive A14 from the game tree on disk`
2. `test(selfcheck): cover stray and clean game trees`

## Notes

- This is an extra directory walk at close-out; cheap relative to hashing.
- Do not follow symlinks differently from `emit::index::walk`; symlink policy is
  a separate task.
- Do not reference the report or plan in code comments.
