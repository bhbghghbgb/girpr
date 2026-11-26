# T22 — mtime conversion/stamping → filetime

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-3 (lines 563–573) — hand-written
  mtime nanosecond conversion and stamping become `filetime` (`unix_seconds()`/
  `nanoseconds()`, `set_file_handle_times`/`set_file_mtime`, `from_unix_time`).
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/src/util.rs:516–523` (`mtime_ns_of`);
  `src/commands/sync/apply/copy.rs:256–278` (`set_modified`) and `:91–98` (`sync_all`);
  `src/scan.rs:206–216`; `test-support/src/lib.rs:402–412` (`age()`).
- Tests `src/scan.rs:401–425`; mtime-preserving copy tests `tests/sync.rs:1213–1240`;
  `tests/compare.rs:67–80`, `tests/why/flags.rs:228`. Affected manifests.

## Implementation plan
1. Validate the current i64 cache value's contract: exact nanoseconds since epoch, with
   pre-epoch currently returning 0. Keep `mtime_ns_of` as the single shim and handle
   pre-epoch explicitly (§SYNC-3 line 568).
2. Use `FileTime::from_system_time(...)` and its accessors for the cache value,
   `set_file_handle_times`/`set_file_mtime` for stamping, and `FileTime::from_unix_time`
   in `age()`.
3. Keep `sync_all` ordering relative to the mtime stamp (§SYNC-3 line 565).
4. Run the scan/copy/compare/why tests; assert no new rounding beyond std's (§SYNC-3
   line 573). `filetime 0.2.30` is not currently locked; verify a resolved build.

## Constraints / validation
- Exact ns are the cache contract (§SYNC-3 line 573). Do not introduce rounding by
   going through seconds-only APIs.
- Preserve mtime-preserving copy behavior and the pre-epoch convention, or record an
  intentional change in the task note.

## Done when
All hand-written conversions use `filetime` behind the existing shim, the mtime tests
pass with exact values, and both platforms build. If a premise is stale or a keeper
would change, **stop** and record `skipped`.
