# T07 - C8: one temp-file naming rule

Report: `../../OPTIMIZATIONS_REPORT.md` lines 227-244 (C8).

Dependencies: none.

## Goal

The two atomic writers name their temp files differently:

- `util::ini::write_config_ini` uses `path.with_extension("ini.girpr_tmp")`,
  which happens to produce `config.ini.girpr_tmp`;
- `repair::audio::write_scan_file` uses `path.with_extension("girpr_tmp")`,
  which replaces the real extension: `scan.txt` -> `scan.girpr_tmp`.

`with_extension` also surprises on multi-dot paths. One rule makes leftovers,
scope globs, and documentation simpler.

## Where to start

- `src/util/ini.rs`: `write_config_ini` (around line 264), the `tmp` creation
  around line 410.
- `src/repair/audio.rs`: `write_scan_file` (around line 162), the `tmp`
  creation around line 173.
- `src/repair/scope/report.rs`: T06 already uses the same rule for
  `--plan-out`; make the helper usable by all three if it is cheap.
- Tests that assert a temp name in `src/util/ini.rs` and `src/repair/audio.rs`.

## Steps

1. Add one small `pub(crate)` helper, preferably in `src/util/path.rs` next to
   the other path utilities:
   `pub(crate) fn temp_sibling(path: &Path) -> PathBuf`.
   It appends the literal `.girpr_tmp` to the **whole file name** using
   `OsString::push`, so it is not lossy and does not replace an extension.
   Examples: `config.ini` -> `config.ini.girpr_tmp`;
   `scan.txt` -> `scan.txt.girpr_tmp`; `plan.v1.json` ->
   `plan.v1.json.girpr_tmp`.
2. Use the helper in `write_config_ini` and `write_scan_file`. If T06 has
   already landed, use it there too; otherwise leave T06's local path alone and
   let T28 unify it.
3. Keep every writer's cleanup behavior: on a failed write or rename, remove
   the temp if it exists; ignore `NotFound`; do not turn a cleanup failure into
   the primary error.
4. Do not change the durability policy: `config.ini` still syncs (unless the
   test hook says otherwise), the scan file still does not.
5. Update any test that names the old temp file. Do not add a purge rule just
   for the new suffix; the purge already deletes files that are not in the live
   manifest.

## Tests

- Unit-test `temp_sibling` with:
  - `config.ini` -> `config.ini.girpr_tmp`;
  - `scan.txt` -> `scan.txt.girpr_tmp`;
  - `a.b.txt` -> `a.b.txt.girpr_tmp`;
  - a path with no extension;
  - (where the platform permits) a non-UTF-8 name, asserting the suffix is
    appended without lossy conversion.
- Keep the existing config/scan write tests passing; update their temp-name
  assertions if they have any.
- Add no test that depends on a temp file surviving; leftovers are never part
  of the contract.

## Acceptance

- Both atomic writers use one naming rule.
- Multi-dot and extensionless paths are handled without `with_extension`
  surprises.
- No writer's durability or cleanup behavior changes.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(util): add one temp-sibling naming rule`
2. `refactor(repair): use the shared temp name in both atomic writers`
3. `test(util): pin extension and multi-dot temp names`

## Notes

- This is naming, not the full atomic-write helper. T28 extracts the shared
  writer; do not build that abstraction twice.
- Changing the suffix changes what an older run's leftover looks like. The
  purge already removes unknown files, and a leftover is not a contract.
