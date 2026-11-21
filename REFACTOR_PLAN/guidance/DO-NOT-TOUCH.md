# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are low-value/local items that
are **not** tasks. Before editing anything, a session should confirm its target is not
on this list. Full reasoning is in the report; only pointers are given here.

## Deliberately kept — changing them is out of scope

- **Report §5 (lines 486–501), the "explicitly not recommended" list.** Do not scale
  fixtures down further (§A5), do not re-add sleeps or the "without fail-fast" /
  "without terminal 404" control runs (§A6), do not share `OnceLock` fixtures across
  nextest tests (§A1), do not add `rstest` for existing loop tables (F7), do not
  re-recommend no-fsync hooks / shared tempdir & test-support crates / already-landed
  leaf extraction / `profile.test` tuning / ignored bridge & throughput suites.
- **Report §A4 (lines 67–73).** The no-fsync environment and scratch-tree policy are
  already in force. They are facts, not candidates.
- **Report A10 (lines 108–110).** `.config/nextest.toml` pins exact packages, binaries
  and fully-qualified test paths. Do not delete or rename one without updating its
  filter in the same change (tasks that move tests say so explicitly).
- **Report F37 (lines 456–462).** Narrowing capture's shared-bundle cache key is a
  considered and rejected change — it accepts a stale-bundle risk the current comment
  deliberately avoids. Do not reopen it without profiling.
- **Byte contracts (report §6, lines 503–510).** Bundle, report and dump bytes are
  pinned by tests. A change must not alter key order, casing, spacing, pinned field
  names or flag conditions. Tasks that freeze manifest bodies (T11) must preserve the
  checksums.

## Non-candidates and maintenance (not tasks)

- **Report F29 (lines 376–382).** `md5.rs` ignored benchmarks, per-process zstd
  re-encodes, one-time `create_dir_all`, fs-identity's scratch dir, report-core's
  `serde_json` dev-dep and junction_support's `cmd.exe` seam are all validated
  non-candidates.
- **Report F34 (lines 426–431).** sled2redb fixtures, its no-sled path and justified
  converter spawns; `output.rs` spawns; do not lower `GIRSYNC_CACHE_SIZE_MIB` (redb
  5.0.0 treats it as a budget, not an allocation).
- **Report F7 (lines 206–213).** Do not add `rstest`; keep loop tables and assertion
  messages.
- **Report F5 (lines 174–182) and F24 (lines 339–342).** Disk cleanup (`target/`
  PDBs, ignored `tests/scripts/io-profile/iot-bench/target/`) is maintenance, not a
  version-controlled test change. Perform it locally if useful; do not track it as a
  task or reference it in code.

## Merge/downshift caveats for test-merging tasks

- **Report §A1 (lines 46–52).** Merging tests reduces process spawns but coarsens
  failure granularity. Preserve labelled subcase blocks and assertion messages so a
  failure is still diagnosable. Do not merge a test named in a nextest filter, a
  host-dependent skip, or a log-capture user (§A10, F26).
- **Report F26 (lines 352–360) and F35 (lines 435–443).** Before a broad rewrite,
  measure nextest per-test overhead; a merge that saves no wall time is not worth the
  loss of granularity.
- **Report F21 (lines 320–325).** Keep one full TCP end-to-end case in artifact-server
  tests — the TCP path is the contract under test. Do not expect cross-test fixture
  sharing (§A1).
- **Report F36 (lines 445–454).** Switch only read-only bundle callers to
  `shared_bundle`; do not change spawn-based or flag-sensitive cases, and do not
  rewrite the pinned 32 KB blob (§A5).

## Cross-program and seam boundaries

- A shared test helper must not make one program's test depend on another's. The new
  loopback mock (T05) must stay dependency-free so `girpr-fetch` and
  `girpr-test-fixtures` can use it without a cycle (§A9, lines 101–106).
- `gircap` independence remains a hard boundary: no dependency on the `girpr` crate,
  no invoking its binary, only leaf crates holding no program logic. If a task adds a
  leaf dependency, update the doc tables and manifest comments in the same work.
- **Report F30 (lines 386–403).** The in-memory sync cache registry is path-keyed and
  shared within a process. Migrate record/FS-asserting cases to the memory seam, but
  audit dry-then-real pairs and use the existing forget helper when a cold cache is
  required. Keep the file backend where the cache file is the subject of the test.
