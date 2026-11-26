# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are items that are **not**
tasks. Before editing anything, a session should confirm its target is not on this
list. Full reasoning is in the report; only pointers are given here. If a session
finds one of these should change after all, it **stops** and raises a follow-up —
it does not change the keeper in place.

## Non-findings — examined without a credible candidate (report lines 795–855)

Do not open a task for any of these, and do not change them as a side effect:

- `crates/exit/src/lib.rs` `define_exit_table!` const `ALL` + contiguity pin (lines 799–800).
- `crates/fs-names/src/lib.rs` policy-parameterized device predicate / ASCII fold;
  `dunce` is already delegated (801–802).
- `crates/fs-identity/src/lib.rs` — already delegates to `same-file` and
  `winapi-util` (803).
- `crates/digest/src/**` — hex/MD5/SHA-256/XXH64 already delegated; newtypes are thin
  adapters (804–805).
- `crates/report-core/src/lib.rs` ordered emission via `serialize_map` (806–807).
- The small pure-fact crates `time`, `cli`, `value-ranges`, `chunk-cost`,
  `chunk-identity`, `audio-language`, `tempdir` (808–809).
- `app/report/src/ledger.rs` saturating `AtomicU64` with all-or-nothing reservation
  (810–811).
- `app/util-hash/src/md5.rs` one-pass multi-range MD5 with interned states (812–813).
- `app/repair-plan/src/plan.rs` chunk-layout algebra and non-persisted fingerprint
  (814–815).
- `app/fetch/src/retry.rs` retry driver; `backoff 0.4.0` already owns schedule math
  via `girpr-retry` (816–817).
- `app/util-path/src/path.rs` `resolve_existing_prefix` fail-closed prefix walk and
  junction/symlink error classifiers (818–821).
- `app/dispatch/src/dispatch.rs` / `metadata.rs` permissive version spellings such as
  `7.2_live` that `semver` rejects (822–823) — note the contrast with T31 §CAP-9,
  which applies only to the strict fixture tags.
- `src/repair/file/**`, `scan.rs`, `phases/post.rs`, `persistent*.rs` — correctness-critical
  policy and bespoke byte movement; mechanics are already in `girpr-*` leaves (824–826).
- `tools/sync/cache/src/open.rs` backup/snapshot publish protocol, hold records,
  registries and retention rules; `File::lock` is stable (827–829).
- `tools/sync/cache/src/db/*`, `write.rs`, `backend.rs` — already on `redb` + `postcard`
  (830–831).
- `tools/sync/cache/src/hash/digestset.rs` — `SmallVec` would grow the pinned 88-byte
  row; the zero-allocation design stays (832–833).
- `tools/sync/src/util.rs` / `cache/src/helpers.rs` ASCII-case helpers deliberately
  independent of `std` (834–835).
- `tools/sync/src/report/record.rs` / `planner/coverage.rs` stdout already NDJSON via
  `girpr-report-core` (836–837).
- `tools/sync/leaves/src/rw/*` — already uses `tokio::sync::Semaphore` and `rayon`;
  the drop-joining `spawn_handler` is a targeted Windows TLS-deadlock fix (838–839).
- `tools/sync/leaves/src/link.rs` "only redirecting reparse points" has no library
  equivalent (840–841).
- `tools/capture/hash/src/lib.rs` one-pass digest/gzip/zstd already delegated (842–843).
- `tools/capture/src/manifest/diff.rs`, `src/assemble/ranges.rs`,
  `src/assemble/assembly.rs` — domain scoring and interval algebra where violations
  are data, not errors (844–846).
- `tools/capture/src/emit/index.rs` `WalkDir` already used; symlink containment is
  policy. `canonical_under_root` duplication is consolidated locally, no crate needed
  (847–849).
- `tools/capture/http/src/error_bodies.rs` / `fetcher.rs` byte-budget accounting and
  status-vs-body fault split (850–851).
- `tools/capture/src/paths.rs`, `pathmap.rs`, `encoding.rs` — already delegate to
  `girpr-path-shape`, `girpr-fs-names`, `percent-encoding` (852–853).
- `tests/scripts/io-profile/**` — standalone Windows-only profiling scripts outside
  the build (854–855).

## Explicit keep items inside otherwise-active findings

- **The deliberately malformed raw-socket helper** (`tools/capture/src/http/tests.rs:114+`,
  lying `Content-Length`, omitted header) has no library substitute and stays raw
  (§X1 line 84, §CAP-1 line 672). Do not "fix" it while rewriting T27.
- **`serde_json/preserve_order` must not be enabled anywhere (§A7, lines 33–34).**
  T28's ordered fixture value uses a local `Serialize` implementation over
  `Vec<(String, J)>`; any global feature change affects `app/report` and others.
- **`std::fs::File::lock`/`try_lock` are stable (§A9, lines 40–41).** T11 touches only
  the FNV key computation; no file-lock dependency is introduced.
- **Retry drivers stay local.** `backoff::future::retry` would absorb caller-owned
  policy (§LEAF-6 line 493); `reqwest-retry` / `backoff` must not replace the capture
  fetcher's retry loop because it would lose per-attempt logs, typed
  `FetchFailure.seq` and panic-free sinks (§CAP-6 lines 735–736).
- **`tabled`/`comfy-table` (T31) and `unicode-bom` (T32) are low-priority.** They may be
  skipped as churn; do not expand them into broader formatting cleanups.

## Cross-cutting themes are not tasks

X1–X4 (report lines 56–134) describe themes, not standalone units of work. Work on
them only through their assigned tasks: X1 → T04/T14/T15/T27; X2 → T09; X3 → T05/T07;
X4 → T11/T16/T21. Do not open a second "HTTP server sweep" or "URL sweep" task that
overlaps those assignments.

## Scope boundaries

- Tests of intended function must be adapted, not deleted (§A5, lines 26–28);
  generated fixture bytes and golden snapshots may change spelling and should be
  regenerated. Assertions must target documented library guarantees, not internal
  formatting quirks (§A12, lines 48–50).
- A shared leaf changed by one task (e.g. `girpr-log` in T07, `girpr-filter` in T20,
  `girpr-proto` in T12) must keep working for its unrelated consumers; do not change
  its public behavior for them without an endorsed follow-up.
- This package and the report are deleted at cleanup. No shipped source, test,
  comment, manifest, doc table or commit message may reference them.
