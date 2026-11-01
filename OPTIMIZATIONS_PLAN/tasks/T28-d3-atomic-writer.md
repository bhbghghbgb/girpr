# T28 - D3: one atomic-write helper

Report: `../../OPTIMIZATIONS_REPORT.md` lines 672-688 (D3), with the temp
naming finding at lines 227-244 (C8) and the plan-out writer at lines 144-163
(C4).

Dependencies: T05, T06, T07 (the three call sites already write through a temp
and follow one naming rule).

## Goal

`util::ini::write_config_ini`, `repair::audio::write_scan_file`, and the
`--plan-out` writer each have their own temp name, error cleanup, and flush
policy. The scan-file writer does not fsync; the config writer does. One helper
should own temp creation, cleanup on failure, and the optional durable flush.
Keep `--plan-out`'s serialization out of the helper.

## Where to start

- `src/util/ini.rs`: `write_config_ini` (around line 264), the temp write
  (around lines 409-434), `read_game_version`.
- `src/repair/audio.rs`: `write_scan_file` (around line 158), the temp at line
  173.
- `src/repair/scope/report.rs`: `PlanDocument::write_to` (around line 351).
- `src/util/mod.rs`: where a new module would be declared.
- `src/util/path.rs`: `PathRoot`/containment helpers, if the helper needs a
  path validation step (it should not; callers already hold a validated path).

## Steps

1. Add a small `util::atomic` module with one entry point, for example:
   ```rust
   pub fn write_atomic(path: &Path, bytes: &[u8], durable: bool)
       -> anyhow::Result<()>;
   ```
   It must:
   - create the parent directory when it is non-empty;
   - derive the temp name from the final file name, preserving the extension:
     `file_name + ".girpr_tmp"` (for `config.ini` this is
     `config.ini.girpr_tmp`; for `scan` it is `scan.girpr_tmp`; for
     `plan.json` it is `plan.json.girpr_tmp`). Do not use `with_extension`,
     which replaces the extension;
   - create the temp in the same directory as the target;
   - write all bytes;
   - if `durable`, `sync_all()` the file, and if the platform supports it,
     fsync the parent directory after the rename (best-effort; do not fail the
     publish on a directory fsync error if the file rename succeeded, but log
     it);
   - rename over the target;
   - on any error before the rename, remove the temp and return the original
     error, with the temp path in the context;
   - on a rename error, remove the temp and return the rename error.
2. Do not put serialization in the helper. `--plan-out` serializes into a
   `String` (or a buffered writer) and passes the bytes. The config writer
   renders its `String` and passes it. The audio writer joins its lines and
   passes the bytes plus a trailing newline if that is the current format.
3. Do not fold the dry-run case into the helper. D6 removes the config writer's
   unreachable `dry_run` parameter; until then, `write_config_ini` returns
   before calling the helper. The helper always writes.
4. Move the test-only fsync suppression (`test_no_fsync()`) into the helper's
   `durable` decision or keep it at the config call site and pass
   `durable = !test_no_fsync()`. Do not make production scan/plan writes
   suddenly durable if they were not; preserve each call site's current
   `durable` value: config `true`, scan/plan `false` (unless C4 already chose
   durable; match the implemented behavior and document it).
5. Keep the temp name in one constant/function (`temp_sibling(path)`), so a
   future sweep and the tests share one rule.
6. Update the three call sites and their tests. The config test that asserts
   `config.ini.girpr_tmp` and the scan test that asserts `scan.girpr_tmp`
   should keep passing; update them only to use the shared helper if the
   spelling is unchanged.

## Tests

- `write_atomic` publishes bytes and leaves no temp on success.
- A write failure (short/failing writer via an injected writer, or a read-only
  directory on a host that supports it) leaves no temp and the target
  unchanged. Use the helper's internal write function with an injected writer
  if a real failure is not portable.
- A rename failure removes the temp and returns the rename error.
- `durable = true` calls `sync_all` (test with a wrapper/counter, not by
  inspecting the filesystem).
- The three production writers still produce byte-identical files and leave no
  temp.
- A multi-dot path gets `name.ext.girpr_tmp`, not `name.girpr_tmp`.
- `cargo test -p girpr` passes.

## Acceptance

- One atomic-write implementation and one temp-naming rule.
- Config remains durable; scan/plan match their current durability decision.
- No serialization inside the helper; no dry-run parameter.
- Cleanup on every pre-rename failure.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(util): add one atomic-write helper`
2. `refactor: publish config, scan, and plan artifacts through it`
3. `test(util): pin temp naming and failure cleanup`

## Notes

- The helper takes bytes. A future large plan could take a writer callback, but
  that is not needed now; `--plan-out` already serializes once in C4.
- Do not add `panic = "abort"` or any profile change here; B5 is separate.
