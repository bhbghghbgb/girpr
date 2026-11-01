# T30 - D4: one rel-path spelling helper

Report: `../../OPTIMIZATIONS_REPORT.md` lines 690-706 (D4).

Dependencies: none.

## Goal

Three helpers answer "what is this path relative to the install?" with
slightly different spellings: `purge::rel_path` (strip + lossy + slash),
`post::join_rel` (dir string + `Path`), and `util::clean_rel`/`PathRoot` for
server rels. The purge and the post-phase walk the same tree and should agree
on the spelling they report.

Add shared helpers with both entry points, keep the lossy conversion explicit,
and use them in both modules.

## Where to start

- `src/repair/purge.rs`: `rel_path` (around line 310), its callers in the walk
  (around lines 150, 162), and tests.
- `src/repair/phases/post.rs`: `join_rel` (around line 415), its callers in
  `move_audio_cache` (around lines 342, 355), and tests.
- `src/util/path.rs`: `clean_rel` (around line 137), `contained_rel` (around
  line 62), `PathRoot`.
- Other walkers: `src/repair/phases/check.rs`, `src/repair/scope/report.rs` if
  they build a rel from a path.

## Steps

1. Add two functions to `src/util/path.rs` (or a small `util::rel` module):
   - `pub(crate) fn rel_from_base(path: &Path, base: &Path) -> Option<String>`:
     strip `base`, convert to `/`, lossy. Return `None` when `path` is not under
     `base` (the purge currently silently continues; preserve that by mapping
     `None` to the same `continue`).
   - `pub(crate) fn join_rel(dir_rel: &str, name: &Path) -> String`: the
     post-phase's spelling; `name` is lossy-`/`-converted, then joined to the
     clean `dir_rel` with exactly one `/`, handling an empty `dir_rel`.
2. Keep the lossy conversion in one place and document why: non-UTF-8 trees
   are possible, and the data plane/log needs a string. Do not switch to
   `to_str().unwrap()`.
3. Refactor `purge::rel_path` to call `rel_from_base`. Its current behavior on
   a path outside the base is to produce a weird string; the callers already
   `strip_prefix` or skip. Make the shared helper return `Option` and update
   callers to skip on `None`. Verify this does not change any classification
   for paths that are under the base.
4. Refactor `post::join_rel` to call the shared `join_rel`. Keep the exact
   separator behavior (never a leading `/`, never a double `//`).
5. Do not change `clean_rel`'s server-rel rules or `PathRoot`'s containment.
   This task is spelling, not validation. If a server rel needs cleaning before
   `join_rel`, the caller still calls `clean_rel` first.
6. Add a short module comment that names the two entry points and the one rule:
   "on-disk rel = strip the walk base, lossy, `/`; server rel = `clean_rel`;
   joining a rel dir and a name uses `join_rel`."

## Tests

- `rel_from_base` for a file directly under the base, a nested file, a path
  equal to the base (not a file; decide and test), a path outside the base
  (`None`), and a non-UTF-8 name on Unix (lossy result; skip on Windows if the
  fixture cannot be built).
- `join_rel("", Path::new("a.pck")) == "a.pck"`,
  `join_rel("Audio/cache", Path::new("sub/a.pck")) == "Audio/cache/sub/a.pck"`,
  and a Windows-style `name` with `\` becomes `/`.
- Purge and post tests still pass with identical log/output spellings.
- `cargo test -p girpr` passes.

## Acceptance

- One on-disk rel spelling helper and one join helper, used by purge and post.
- Lossy conversion is explicit and documented.
- No classification, deletion, or move decision changes.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(path): share the on-disk rel spelling`
2. `refactor(purge,post): use the shared rel helpers`
3. `test(path): pin rel stripping, joining, and lossy names`

## Notes

- Do not make `rel_from_base` canonicalize or validate. It reports the spelling
  of a path the walk already produced.
- Keep the server-rel rule (`clean_rel`) separate; it validates hostile input,
  which this helper does not.
