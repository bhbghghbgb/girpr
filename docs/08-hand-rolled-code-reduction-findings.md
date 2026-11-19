# 08 — Hand-rolled code reduction: findings and assumptions

This document is a **research report**, not a plan. It lists findings about code
that reimplements functionality a maintained third-party library provides, plus
the assumptions that need to be checked before any of it is implemented.
Nothing here is an instruction or an ordered task list; every suggestion is
subject to validation at implementation time.

Workspace covered (all six members): root `girpr`, `crates/log` (`girpr-log`),
`crates/tempdir` (`girpr-tempdir`), `tools/sync` (`girsync`),
`tools/capture` (`gircap`), `tools/sled2redb`. Line numbers are from `main` at
`5a910dc7`.

Selection rule used: only maintained, cross-platform (Windows + Linux) crates
with a real user base were considered. No MSRV constraint. Observable behavior
changes are acceptable and backwards compatibility is not required; the
"expected behavior change" and "tests affected" fields below are therefore
information, not objections.

---

## A. Cross-cutting findings (one decision per workspace, not per tool)

### A1. JSON object rendering is hand-rolled three times — high payoff

**Locations**

- `src/report/record.rs:169-183` (`Record::json`), `:194-251` (`bare`,
  `push_lowercase`, `push_json_string`, `push_json_event`, `push_json_value`).
- `tools/sync/src/report/record.rs:159-171` (`json`), `:179-199` (`write_json`),
  `:266-335` (`push_bare`, `push_json_member`, `push_json_value`,
  `push_json_string`, `push_json_char`).
- `tools/capture/src/report/mod.rs:281-321` (`to_json_line`), `:337-401`
  (`push_json_key`, `is_plain_ascii_json`, `json_string`, `push_json_value`).

**Finding.** ~80 LOC per module (~240 total) manually emit `{`, `"`, `,`, `:`,
a "is this string safe unescaped" predicate, an escaping fallback that already
calls `serde_json::to_string`, and bespoke renderings of a small `Val` enum.
All three carry the same comment: they avoid `serde_json::Map` because it is a
`BTreeMap` unless `preserve_order`, and cargo feature unification would alter
another tool's output. That premise is incomplete: `serde_json::Serializer`'s
`serialize_map`/`serialize_entry` emits entries in the order the `Serialize`
impl yields them; `preserve_order` only affects `serde_json::Map`/`Value`,
which these renderers never need.

**Candidate.** Implement `serde::Serialize` for each `Record`/`Val` using
`serialize_map(Some(n))` plus `serialize_entry(k, v)` in the current order;
`json()`/`to_json_line()` then become `serde_json::to_string(self)`. `serde`
and `serde_json` are already dependencies of all three crates.

**Restructure.** Minimal path: each module implements `Serialize` locally and
keeps its own API. Full path: one shared leaf crate holding `Record`/`Val`/
`OutputFormat` so all three tools render identically; reconciliation needed
because girpr uses `Str|UInt|Int` in declaration order, gircap uses
`Str|Int|Bool` event-first, and girsync uses `serde_json::Value` with
alphabetically sorted keys and `Layout::{Positional,Keyed,Pair}`.

**Tests affected.** `tools/sync/src/report/record.rs:337-452` (five tests pin
exact bytes including sorted keys), `tools/capture/src/report/tests.rs:15-54`
and `:235-282`, `:455-540`, `src/report/tests.rs`. Consumers that parse NDJSON
(`tools/capture/tests/common/mod.rs parse_ndjson`, `tools/sled2redb/tests/convert.rs:467-508`)
stay valid.

**Expected change / caveat.** girsync's key order changes if it adopts
declaration order. Escaping of control characters stays byte-identical;
gircap's `is_plain_ascii_json` currently leaves backslashes unescaped while
`serde_json` emits `\\`, so every Windows-path line changes unless a thin
escape helper is retained or the change is accepted.

### A2. Retry + backoff is implemented independently twice

**Locations.** `src/retry.rs:25-154` (`Retry`, `RetryState`, `retry`,
`retry_state`, salt-based linear backoff, deterministic injectivity jitter;
tests `:156-394`) and `tools/capture/src/http/transport.rs:8-54`
(`RetryPolicy::delay_before`, `should_retry_status`, `Sleep` trait) together
with `tools/capture/src/http/fetcher.rs:787-853` (retry loop).

**Finding.** girpr hand-rolls a retry driver and a custom de-synchronization
scheme (`NEXT_SALT`, `JITTER_RANGE`, `JITTER_MULTIPLIER`, `JITTER_ATTEMPT_MIX`)
plus ~95 LOC of tests proving jitter injectivity. gircap hand-rolls a clamped
exponential schedule and inherits `Sleep` injection. The two semantics are
deliberately different: girpr is linear + injective jitter with
`Fatal/Retry/Done` and state threading; gircap is exponential, no jitter,
status-classified, and logs every attempt while carrying a sequence number.

**Candidate.** `backoff` (`ExponentialBackoff`, `randomization_factor`,
`Clock`), optionally `reqwest-retry`/`reqwest-middleware` for the whole gircap
pipeline. Caveat: `backoff` 0.4 is very widely used but its release cadence is
old — verify the current maintained release line before adopting.

**Restructure.** girpr: map `Retry::Retry`→`Transient`, `Fatal`→`Permanent`,
replace `Arc<dyn Sleep>` with a configured `Backoff`, update call sites in
`src/config.rs`, `src/hyp.rs`, `src/repair/file/mod.rs`, `src/sophon/stream.rs`
and the `NoSleep` test constructors. gircap: replace `RetryPolicy::delay_before`
with `ExponentialBackoff` configured `jitter: None`, keep the loop and `Sleep`
trait. A full middleware adoption is **not** a drop-in: per-attempt `seq`
logging, `bytes_in` accounting on failed bodies and `HashingFile::restart`
cannot be expressed by plain middleware.

**Tests affected.** girpr `src/retry.rs:207-395` (backoff-shape and jitter
injectivity tests pin the hand-rolled math and disappear); gircap
`src/http/tests.rs:136-178` (backoff-cap and retryable-status tests).

**Expected change / caveat.** girpr's backoff becomes exponential and jitter
becomes RNG-based (deterministic de-synchronization is lost). gircap must keep
`jitter: None`: its artifact `api/requests.ndjson` is intentionally diffable
without jitter. `backoff::future::retry` does not natively carry state forward,
so the `retry_state` shape needs a captured-mutation adapter.

### A3. Hex encode/decode hand-rolled at five-plus sites

**Locations.** `src/util/md5.rs:39-64,80-88,326-334`; `tests/common/digest.rs:9`;
`tools/sled2redb/src/convert.rs:121-133` (`hex_to_bytes`);
`tools/sled2redb/tests/convert.rs:31-33`; `tools/capture/src/hash/mod.rs:218-244`
(`hex_bytes`, `hex_bytes_upper`, `hex_with`), `:31-39`;
`tools/capture/tests/common/mod.rs:2136-2140` (`md5_hex`, the per-byte
`format!("{b:02x}")` anti-pattern that `hash::hex_bytes` was written to remove).

**Finding / candidate.** ~55 LOC in girpr, ~27 in gircap plus duplicates in test
harnesses. `hex` 0.4 (`encode`, `encode_upper`, `decode`, `decode_to_slice`) is
tiny, ubiquitous, cross-platform. Keep public wrapper names as thin shims so
call sites do not change. girpr's `parse_hex` needs a fixed `[u8;16]` and its
"not a hex MD5" message — use `decode_to_slice` and keep the message.

**Tests affected.** `src/util/md5.rs:527-549`, `tools/capture/src/hash/tests.rs:546-572`,
`tools/sled2redb/tests/convert.rs:368-415`. Keep the context wrapping in
sled2redb so errors still mention the bad file and algorithm.

**Expected change / caveat.** Digest hex output is identical. `hex::decode`
error wording differs; low payoff individually, sensible as one sweep.

### A4. INI parsing: two different hand-rolls, conflicting verdicts

**Locations.** `src/util/ini.rs:390-540` (`Section`, `find_key`, `insert`,
`force`, `find_section`, `parse_ini`, `merge_ini`, `render_ini`; called from
`write_config_ini` `:334-388`), plus `read_game_version` `:20-57`;
`tools/capture/src/emit/gamedir.rs:44-97` (`config_ini` renderer and
`parse_game_version`, ~50 LOC).

**Finding.** girpr implements a full load → case-insensitive merge (duplicate
sections merged, duplicate keys last-wins, first-seen spelling kept, quotes
stripped, comments and blanks dropped, headerless keys treated as `[General]`)
→ deterministic re-render (first-seen sections, case-insensitively sorted keys,
blank line after each section). gircap implements a strict `[General]`-scoped
first-match reader. `ini` 2.0 / `configparser` 3.3 model the parse/merge core.

**Two positions found in the research.** One recommends delegating the
parse/merge/render core to `configparser`/`ini` (~150 LOC), keeping the forced
key table, the UTF-8/atomic boundary and `read_game_version` hand-rolled
(~150 LOC removable). The other recommends **not** migrating: the current
behavior is deliberate Starward/Collapse parity (drop comments, merge duplicate
sections, sort keys, force specific `[General]` keys, blank line after each
section), a general parser changes several of those rules, and the payoff is
small. Given the stated tolerance for observable behavior changes, both remain
viable; this is recorded as an open assumption rather than a verdict.

**Restructure if adopted.** Replace `Vec<Section>` with `configparser::Ini`;
keep the forced-key table and renderer if the crate's writer orders differently.
Do **not** drag the JSON-lines readers (`:59-216`, already `serde_json`) into a
config framework. For gircap, replace only `parse_game_version`, keep the
byte-exact `config_ini` renderer.

**Tests affected.** `src/util/ini.rs:905,963,1034,1391`, `:1342`;
`tools/capture/src/emit/gamedir.rs:272-326`. These pin the exact merge/render
and section rules and must be rewritten or removed with the crate.

**Expected change / caveat.** Emitted `config.ini` bytes may differ (spacing,
blank lines, key order). `read_game_version`'s "last match wins, no section
scope" deliberately differs from gircap's "first match in `[General]`" — keep
them distinct unless a shared configurable reader is designed.

### A5. HTTP mock servers: canned servers are delegable, domain servers are not

**Locations.** girpr `src/test_http.rs:17-57` (raw-socket canned replies) and
`tests/artifact_bridge/server.rs:311-322,915-1001,1034-1055` (four listeners,
manual request-line/query parsing, manual HTTP/1.1 response, hand-rolled
percent-decode). gircap `src/http/tests.rs:686-813` (`local_server`),
`tests/common/mod.rs:1210-1344` (`Mock`) and `:1869-1969` (`serve`), plus
`MemoryTransport` `:1346-1400`.

**Finding.** Two complete toy HTTP servers in gircap (~350-450 LOC) and ~130
LOC of raw wire handling in the bridge plus ~57 LOC in girpr, maintained solely
to feed `reqwest` controlled responses.

**Candidate.** `wiremock` 0.6 (tokio-native, request journal) or `mockito` 1.7
for canned replies; `axum`/`hyper` if custom routing with state is needed.
`wiremock`/`mockito` owns its own server lifecycle and adds a dev-dependency.

**Restructure.** Replace canned `serve_bodies`/`serve_status` with mocks and
`expect(n)`/`received_requests()`; keep `Served`, `Hit`, `HitKind`, `Bundle`,
`Response` and URL-prefix rewriting as shared domain state. In gircap, the
`HttpTransport`/`MemoryTransport` injection seam (`src/http/transport.rs:208-369`)
can be deleted entirely if a real mock server is fast enough for the 113 610-chunk
tests — another ~200 LOC. The bridge's four origins, route families, build-id
invariants and recorded failure shapes are **domain logic** a mock library
cannot express; keep them.

**Tests affected.** `tests/artifact_server.rs`, gircap tests asserting raw
request lines/headers (expressible as wiremock matchers). None pin the toy
server implementation itself, only its responses and request log.

**Expected change / caveat.** Incidental headers (`Date`, `Server`,
`transfer-encoding`) change — assert only headers the product reads (`etag`,
`content-md5`, `content-type`). Truncated/lying-`Content-Length` tests
(`src/http/tests.rs:716-732`) need one small raw-socket helper or a custom
hyper service; a well-behaved mock server cannot lie about length.

### A6. Directory walkers duplicated — `walkdir` already exists in the workspace

**Locations.** `tools/capture/src/emit/index.rs:269-347` (`walk`,
`canonical_under_root`, `rel`, ~70 LOC) and
`tools/capture/src/verify/list.rs:89-220` (`list`, `list_with`, `child_dirs`,
`canonical_under_root`, ~110 LOC). Two independent recursive `read_dir`
implementations with hand-built stacks, visited sets and canonical-root checks.

**Candidate.** `walkdir` 2.5 — already a direct dependency of the root
`girpr` package, maintained, cross-platform, with sorting, symlink loop
detection and per-entry errors.

**Restructure.** `WalkDir::new(root).follow_links(true).sort_by_file_name()`
plus the existing canonical-root containment filter; keep `list_with` only as
the fault-injection seam or rewrite that test at `Bundle::open`. Keep the
"MANIFEST.json is a bundle boundary — do not descend" rule as `filter_entry`.

**Tests affected.** `src/verify/tests.rs` (including injected read-error tests),
`tests/schema.rs`, `tests/capture_layout.rs`, `tests/offline_bundle.rs`,
`src/emit/selfcheck.rs`, `src/verify/checks.rs`.

**Expected change / caveat.** `walkdir` detects symlink loops via ancestor
checks rather than a global visited set; a symlink to a sibling reached by a
different path may be walked twice unless deduped by canonical path. Sorting is
already used, so document bytes should be unchanged.

### A7. Path canonicalization / `\\?\` stripping — `dunce`

**Locations.** `tools/sync/src/commands/sync/plan/link.rs:136-153`
(`without_verbatim`, incl. `\\?\UNC\`),
`tools/sync/src/effective/side.rs:329-335` (`display_path`),
`tools/sync/src/util.rs:254-279` (`canonical_or_raw`, `same_path`);
`src/util/path.rs:438-480` (`resolve_existing_prefix`).

**Finding.** Three small verbatim-prefix strippers in girsync plus a
canonicalize-nearest-existing-ancestor routine in girpr.

**Candidate.** `dunce::canonicalize` resolves symlinks like `std::fs::canonicalize`
but omits the Windows `\\?\` prefix — removes both strip helpers. Use it
consistently; mixing it with `std::canonicalize` recreates the prefix mismatch.

**Tests affected.** `tools/sync/src/commands/sync/plan/tests.rs:637-651`
(`without_verbatim_rebuilds_the_prefix_the_path_needs`), `effective/side.rs`
message assertions, `src/util/path.rs:908-1007` (symlink/junction containment).

**Expected change / caveat.** User-facing error paths lose `\\?\` without
post-processing. Keep girpr's `resolve_existing_prefix` unless the chosen crate
method demonstrably resolves symlinks (see §I.1). `tools/capture/src/paths.rs`
is purely lexical shape validation and has **no** canonicalization to replace.

### A8. Glob filtering duplicated — `globset` or a shared leaf crate

**Locations.** `src/repair/scope/filter.rs:5-8,37-134` (`MATCH_OPTIONS`,
`PathScope`, `compile_patterns` — its doc says it is a girsync port with no
shared code) and `tools/sync/src/filter.rs:25-147` (`compile_patterns`,
`Filter`, `matches_or_ancestor`), `:150+` (`ExcludeMemo`), publicly re-exported
at `tools/sync/src/lib.rs:67-72`. ~250 LOC total.

**Candidate.** Extract a shared leaf crate `girpr-filter` (preserves semantics
exactly) or migrate both to `globset` (`GlobBuilder::literal_separator(false).case_insensitive(true)`),
which compiles patterns once and is substantially faster.

**Tests affected.** `src/repair/scope/filter.rs:136-200+`,
`tools/sync/src/filter.rs:220+`, `tools/sync/tests/public_surface.rs:15-50`
(pins `girsync::glob::Pattern` in the public API).

**Expected change / caveat.** With `globset`, verify `*` still crosses `/`
(`src/repair/scope/filter.rs:183-198`), `**` semantics and dotfile rules, and
the "exclude beats include" and subtree rules; the public re-export type changes.

### A9. `tracing-appender` should be the single file-log writer

**Locations.** `src/logging.rs:57-105` (`FILE_BUFFER_CAPACITY`, `FileSink`,
`FILE_SINK`, `SharedWriter`, `SharedWriterGuard`), `:142-219`
(`init_tracing_with`, `flush_log`). ~120 LOC of custom `MakeWriter`,
`Arc<Mutex<Box<dyn Write>>>`, global sink slot and flush-on-exit path.
`tools/sync/Cargo.toml:54` and `tools/capture/Cargo.toml:75` already depend on
`tracing-appender 0.2.5`.

**Candidate.** `tracing_appender::non_blocking(f)` returns a `NonBlocking`
writer and a `WorkerGuard` whose drop flushes — a direct match for `FILE_SINK`
+ `flush_log`.

**Restructure.** Add the dependency to the root manifest, hold
`Option<WorkerGuard>` in a static mutex, and make `flush_log()` take-and-drop
the guard. `main.rs` calls `std::process::exit` on many paths, so `WorkerGuard`
drop will not run automatically — the explicit flush seam must remain and be
audited on every exit path. `girpr-log` keeps deciding *where* the file goes.

**Tests affected.** `src/logging.rs:345-506` (buffer/flush/reinstall tests)
and `tests/cli_contract.rs` log-file assertions.

**Expected change / caveat.** Logging becomes non-blocking (a writer thread);
`tail -f` still lags by a buffer, as today. `WorkerGuard` has no public
mid-run `flush()` (upstream issue #3267), so mid-run flush semantics are lost.

### A10. `girpr-tempdir`'s `TempDir` core is a hand-rolled `tempfile`

**Locations.** `crates/tempdir/src/lib.rs:91-242` (`ROOT`, `root`, `SEQ`,
`unique_path`, `TempDir::{new,absent,at,subdir,to_path_buf}`, `Drop`).

**Finding.** ~150 LOC of pid + nanos + atomic-sequence naming with dash folding
and best-effort cleanup — the standard `tempfile` role. `test_support.rs`
(258 LOC of shared fixture builders) is separate and worth keeping.

**Candidate.** `tempfile::Builder::new().prefix(..).tempdir_in(root())`;
`TempPath` for `at`/`absent`. Random names remove the manual counter and make
parallel `nextest` runs collision-free.

**Restructure.** Keep a thin policy wrapper (`ROOT`, recognizable
`<temp>/girpr-test/` root, `<prefix>-<tag>-…` naming); delegate creation and
cleanup. `absent()` stays custom (a deliberately uncreated path).

**Tests affected.** `crates/tempdir/src/lib.rs:264-407` — the 5-part name-shape
test `:275-313` and drop-cleanup test `:397-407` become obsolete; shared-root,
`absent`, `at` and `subdir` tests stay. Many integration tests use this helper;
names appearing in error-message assertions change.

**Expected change / caveat.** `tempfile` is a new dependency (not in the
lockfile today). Cleanup with open handles is best-effort in both. Related:
`crates/tempdir/Cargo.toml:20` pulls the entire `girsync` library (redb, rayon,
windows-sys, …) into every test build solely for `test_support.rs:22-33`;
moving those fixtures behind a girsync test-support feature would remove that
heavy dev dependency.

### A11. Level parsing, file locking and file reuse stay as they are

**Findings (explicitly not recommended).** `girpr-log`'s level parser
(`crates/log/src/level.rs:35-82`) is a closed five-name set with a deliberate
`warning` alias and custom error text; `tracing::LevelFilter::from_str` adds
`off` and loses the alias. `File::try_lock` / `try_lock_shared` in
`src/repair/lock.rs:68`, `tools/sync/src/cache/lock.rs:54-73` is already the
stable std OS lock — `fs2`/`fslock` add nothing. The `Md5`-gated
`FileReuse` index in `src/sophon/map.rs:67-134` can drop its `DefaultHasher`
key and use the 16-byte `Md5` value directly as a `HashMap` key (removing
collision bucketing), but the lock file's FNV key (`src/repair/lock.rs:40-43,125-170`)
must keep its exact byte sequence or existing lock files are orphaned; use
`fnv` only as a named-constant replacement for the literals.

---

## B. Root `girpr` findings

### B1. Stale zstd workaround can be deleted outright

**Location.** `src/sophon/stream.rs:484-536` in `stream_body`; comment `:487-492`;
hand-written `dec.finish()` at `:532`.

**Finding.** ~50 LOC wire `zstd::stream::raw::Decoder` into
`zstd::stream::zio::Writer` solely because a comment claims
`zstd::stream::write::Decoder` has no `finish()` (citing zstd 0.13.3).
The pinned `zstd = "0.14.4"` exposes `write::Decoder::finish(self) -> Result<W>`
and `do_finish(&mut self)`, documented as the only way to detect a truncated
frame — exactly the hand-rolled behavior.

**Restructure.** Replace the raw decoder + `zio::Writer` with
`write::Decoder::new(&mut hashing)?`, keep the `write_all` loop, then
`let mut hashing = dec.finish()?;`. No API change.

**Tests affected.** `src/sophon/stream.rs:634-659,702-742,779-788` should pass
unchanged. The multi-write test at `:702-742` rules out "simplify to
`decode_all`".

**Expected change.** None intended; identical truncated-frame semantics. This
is a pure workaround deletion and the lowest-risk finding in the report.

### B2. Hand-rolled HTTP status error and URL join duplicate `reqwest`/`url`

**Locations.** `status_failure` `src/sophon/stream.rs:559-568` (comment says
it produces "the shape `reqwest` would have produced for `error_for_status()`"),
`verdict` `:551-557`, `retryable` `:570-589`, `join_url` `:591-593`.

**Finding.** ~37 LOC: `format!("{}/{}", prefix.trim_end_matches('/'), id)` and
a hand-built status error. `reqwest::Response::error_for_status_ref()` (already
a dependency) produces the canonical error; `reqwest::Url::join` is the URL
equivalent.

**Restructure.** Resolve `url_prefix` into a `reqwest::Url` at the manifest
boundary, use `.join(id)?` per blob; keep the narrow `retryable(u16)` policy
(Starward/gircap parity) but express it with `StatusCode::is_server_error()`
plus `408 | 429`.

**Tests affected.** `src/sophon/stream.rs:951-997`; `tests/repair_offline.rs:40,1241`.

**Expected change / caveat.** `Url::join` percent-encodes ids and resolves
`.`/`..`; ids are opaque hashes so normal runs are unaffected. A prefix without
a trailing `/` replaces its last path segment — validate production prefixes.

### B3. JSON helper, FNV and concurrency details

- `src/report/record.rs:194-251` — covered by A1.
- `src/repair/lock.rs:40-43,125-170` — FNV literals → `fnv` named constants;
  key bytes must not change (see A11).
- `src/repair/file/mod.rs:207-267,384-447` — ~65 LOC of `JoinSet` window
  management while `src/repair/scan.rs:313-341`,
  `src/repair/phases/check.rs:88-122` and `phases/manifest.rs:150-157` already
  use `futures_util::stream::iter(..).buffer_unordered(jobs)`. Consolidate on
  `buffer_unordered` or extract one bounded-executor helper; `futures-concurrency`
  (`Tasks`/`Group`) preserves panic reporting if that is required. Tests:
  `src/repair/file/mod.rs:365-382`, `tests/repair_machinery.rs`. Caveat:
  `buffer_unordered` detaches on drop; the module doc at `file/mod.rs:59-63`
  treats abort-on-drop as intentional.
- `src/repair/phases/plan.rs:44-138,363+` — ~90 LOC of chunk-range tiling/
  gap/overlap validation; `nodit`, `rangemap` or `iset` can supply the interval
  set, but keep the allocation-free `is_tiled()` fast path and the exact
  `(offset,len)` / `(earlier,later)` tuples consumed by zero-fill. Tests:
  `plan.rs:554+,687,1017,1069`. Weakest crate-adoption case — see §I.5.
- `src/cli.rs:294-308`, `src/main.rs:94-96` — manual `jobs` range checks; a
  `clap` value parser with `.range(1..=1024)` could move validation into the
  parser, but preserving `Option<usize>` ("flag absent") needs care. Low payoff.
- `src/util/counter.rs:20-31,33-580` — a ~10-line saturating `AtomicU64` CAS
  surrounded by ~520 LOC of custom `Gate` + thread-local CAS-pause race
  harness. `fetch_update(Relaxed, Relaxed, |c| Some(c.saturating_add(n)))`
  replaces production code; `std::sync::Barrier` or `tokio::sync::Notify`
  replaces the test gate. Five race tests at `:274-579` pin the harness, not
  product behavior. This is a test-code reduction, not a library adoption.
- `src/phase.rs:45-106` — drop-based `PhaseTimer` with fixed log-field names;
  `tracing` spans can time phases, but the log-shape change is large and the
  payoff small — research did not find a clearly better library here.

### B4. Atomic publish helpers delegate to `atomicwrites` / `tempfile`

**Locations.** `src/util/atomic.rs:23-204` plus `temp_sibling` in
`src/util/path.rs:399-436` (~205 LOC);
`src/repair/phases/post.rs:475-542,583-734` (`copy_publish_with`,
`move_audio_cache_with`, rename→copy→remove fallback, ~120 LOC);
`src/repair/file/temp.rs:716-749` (`promote_tmp`).

**Candidate.** `atomic-write-file` 0.3.1 (~6.4M downloads, cross-platform,
`AtomicWriteFile::options().open(p) → write → commit()`), or `atomicwrites`
(`AtomicFile::new(path, OverwriteBehavior::AllowOverwrite)`), or
`tempfile::NamedTempFile::persist()`.

**Restructure.** `write_atomic` and the pure `copy_publish_with` path are the
clean candidates. Keep the lexical guards (`ends_in_a_file_name`, Windows
trailing-dot/space rules), the `create_dir_all` and the `durable` decision.
**Do not** delegate `promote_tmp` or the audio move: the `_tmp` files are
durable resume artifacts (length-based resume, rollback to chunk offset,
"keep source on failure"), which a generic temp-file crate would discard.

**Tests affected.** `src/util/atomic.rs:219-457`,
`src/util/path.rs:762-834` (`temp_sibling` tests),
`src/repair/phases/post.rs:758-1230`, `src/repair/file/temp.rs:1007-1103`.

**Expected change / caveat.** Temp names change (no `.girpr_tmp` sibling);
`GIRPR_TEST_NO_FSYNC` may lose its seam; sync/durability granularity may change.
Verify the crate's Windows overwrite semantics and whether it creates parents.

---

## C. `crates/log` (`girpr-log`)

### C1. EnvFilter spec is built by string concatenation

**Locations.** `crates/log/src/plan.rs:236-249` (`console_filter`,
`own_console_directive`), `crates/log/src/filter.rs:9-25`.

**Finding.** ~35 LOC assemble `"{RUST_LOG},{project}={level}"` as a `String`
and parse it, warning and falling back on a bad spec.

**Candidate.** `tracing-subscriber` (already a dependency):
`EnvFilter::builder().with_env_var("RUST_LOG").parse()` then
`.add_directive(Directive::from_str(&own)?)`, which appends the project
directive last and preserves "last wins".

**Tests affected.** `crates/log/src/filter.rs:70-112` (target-beats-global and
last-wins — keep, they pin upstream rules), `crates/log/src/plan.rs:572-736`,
`crates/log/src/lib.rs:90-116`.

**Expected change / caveat.** `with_default_directive()` is **not** equivalent
(it is target-less). Preserve the warn-on-broken-spec path; `parse_lossy`
silently drops bad directives. If the public return type changes from `String`
to `EnvFilter`, update `src/logging.rs:164-167`, `tools/sync/src/logging.rs`,
`tools/capture/src/logging`.

### C2. Timestamp format duplicated

**Locations.** `crates/log/src/plan.rs:320-326` (private `timestamp`) and
`tools/sync/src/util.rs:42-44` (`ts_now`), also used by
`tools/sled2redb/src/convert.rs:344`.

**Finding / candidate.** The same `chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ")`.
Export one `pub fn timestamp()` from `girpr_log` and let `ts_now` delegate —
`girpr-log` is already a girsync dependency. Two-line change, no new crate.

**Tests affected.** `plan.rs:397-418` (23-char shape), and a hard-coded stamp
in `tools/sync/src/commands/sync/plan/tests.rs:439`. No behavior change.

### C3. Keep the crate itself

`girpr-log` is a ~300 LOC policy layer (path, levels, env vars, precedence),
not a logging implementation; its extraction stops drift across three binaries.
Only C1/C2 are worth touching.

---

## D. `crates/tempdir` (`girpr-tempdir`)

Covered by A10. Summary: delegate `TempDir` creation/cleanup to `tempfile`
while keeping the `<temp>/girpr-test/` root policy, `absent()` and
`test_support.rs`; consider relocating the girsync option builders out of this
crate to drop the heavy `girsync` dev dependency.

---

## E. `tools/sync` (`girsync`)

### E1. Hand-rolled cache wire codec — high payoff, accepts a format break

**Location.** `tools/sync/src/cache/codec.rs:50-156` (`encode_rec`,
`push_hash`, `decode_rec` with a local `take()` cursor, `encode_meta`,
`decode_meta`; module doc `:1-15`).

**Finding.** ~130 LOC of length-prefixed little-endian serialization (format
byte, kind byte, LE ints, digest count, `u8 name_len + name + u8 digest_len +
digest` pairs) plus truncation/version/trailing-byte checks.

**Candidate.** `postcard` (serde-based, widely used, active) or `bincode` 2.x
(configurable fixed-width LE ints can preserve the exact byte layout); `rkyv`
is a heavier zero-copy option.

**Restructure.** Derive serde on `FileRec`/`Meta` (`cache/schema.rs`), model
`DigestSet` with a custom serde impl that keeps `iter()` then `foreign_iter()`
order, replace codec calls in `cache/write.rs`, `cache/db/cachedb.rs`,
`cache/open.rs:2033-2051`, delete `codec.rs`.

**Tests affected.** `cache/codec.rs:158-323`, especially
`the_wire_format_is_byte_identical_to_what_a_map_produced` (`:210-225`),
`an_unknown_name_is_carried_and_lands_after_the_known_ones` (`:232-256`),
`codec_preserves_unknown_algorithms` (`:286-299`),
`codec_rejects_truncated_and_bad_version` (`:301-309`).

**Expected change / caveat.** Cache files change format and old caches must be
rebuilt (`girsync update --dir` already does this). The forward-compatible
"unknown algorithm names carried verbatim" rule is **not** free from a derive —
the custom adapter must preserve it. `postcard` is positional, so schema
evolution needs the existing version gate.

### E2. Hand-rolled Windows file-identity FFI → `same-file`

**Location.** `tools/sync/src/hardlink.rs:21-57,68-78,86-139,156-158,171-177,184-203`
(`Identity`, Unix and Windows implementations with `CreateFileW`,
`GetFileInformationByHandle`, `CloseHandle`), plus a Windows-only `windows-sys`
dependency used only here.

**Candidate.** `same-file` 1.0.6 (BurntSushi, **already in `Cargo.lock` via
`walkdir`** — zero new dependency) provides `is_same_file` and
`FileId::from_path`/`from_file`. `file-id` is an alternative.

**Restructure.** Replace `Identity` + `identity_of` with `FileId`; keep
`LinkInfo` but source `links` from `MetadataExt::nlink` on Unix and a minimal
`nNumberOfLinks` read on Windows (or rely on `is_same_file` where the link-count
guard is not needed). Callers: `commands/sync/apply/copy.rs:201-213` and plan
safety in `commands/sync/plan/safety.rs`.

**Tests affected.** `hardlink.rs:212-292`, integration coverage in `tests/sync.rs`.

**Expected change / caveat.** None for normal runs. The "link count > 1"
guard must remain conservative — a pure identity check would miss a dst hard
link to a *different* src file (`copy.rs:194-199`).

### E3. Hand-rolled counting semaphore / concurrency gate

**Location.** `tools/sync/src/rw/gate.rs:17-72` (`Gate`: `Mutex<usize>` +
`Condvar`), `:80-171` (`RwLimiter`), `:179-190` (`RwPermit` + `Drop`) — ~170 LOC;
`rw/mod.rs:86-197` custom rayon `spawn_handler`.

**Candidate.** `tokio::sync::Semaphore`, or a small `semaphore`/`std-semaphore`
crate. `std::sync::Semaphore` is still unstable. **Note:** `rayon` is already a
dependency and already sizes the pool to `limits.pool_size()` — in the common
case the gate is redundant, so the largest simplification is removing it and
bounding with the pool, keeping two pools only for `RwLimits::Split`.

**Tests affected.** `rw/gate.rs:192-276`, `tests/rw_threads/*`, limit-resolution
integration tests. Keep `in_flight()` only for diagnostics.

**Expected change / caveat.** Preserve the two-gate `acquire_copy` ordering
(`gate.rs:126-146`) for deadlock freedom; any replacement must support
`try_acquire` and return permits on panic.

### E4. Hand-rolled JSON writer → `serde_json`

`tools/sync/src/report/record.rs:159-335` — ~70 LOC of alphabetical key sorting
and a manual JSON escaper duplicating `serde_json`'s rules; covered by A1.
Tests `record.rs:337-452` and `tests/output.rs` are the byte-level oracle.

### E5. Atomic publish / backup protocol in `cache/open.rs` — largest restructure

**Location.** `tools/sync/src/cache/open.rs` (3,644 lines): module doc `:1-124`,
`open_db` `:284`, `backup_db`/`snapshot_old` `:455/467`, `retain_copies` `:522`,
`copy_aside` `:598`, `publish_copy` `:709`, `publish_file` `:818`,
`link_file` `:895`, `publish_dir` `:931`, claim/hold helpers `:1118-1769`,
`retain_siblings` `:1909`.

**Finding.** ~2,000 lines of write-under-working-name, publish-by-atomic-hardlink-
or-rename-with-claim, OS-lock hold records, process-global registries and
age-ordered retention. No crate replaces the whole protocol, but individual
mechanics can be delegated: `atomicwrites` / `tempfile` for the file-copy path
and `tempfile` for working names. `fslock` would add nothing over std
`try_lock`.

**Restructure.** Extract a `backup/` submodule with a `Publisher` trait; move
the partial+rename mechanics behind it, leaving the cross-process claim
protocol and registries behind a narrow interface.

**Tests affected.** `tests/migration_lock.rs`, `cache/open.rs` inline tests
(`:2050+`) pin publish/retention names and behavior — the safety net for any
restructure.

**Expected change / caveat.** The module docs enumerate one-syscall gaps that
must survive: "a name in a copy list never holds half a copy" and fail-fast
semantics. Highest payoff, highest risk.

### E6. Other girsync findings

- **Digest containers** (`hash/digestset.rs:96-382`, `hash/set.rs:32-96`,
  `hash/algo.rs:25-115`, `hash/stream.rs:26-99`, ~400 LOC) — `bitflags`,
  `smallvec`/`arrayvec`, `smol_str` are possible mechanical swaps, but the
  80-byte/no-allocation guarantee is load-bearing and tests
  (`hash/tests.rs:324-333,868-875`) pin it. Research recommends **not**
  replacing.
- **Symlink/junction handling** (`link.rs:53-185`, ~120 LOC) — Windows
  reparse-point nuances (junction vs cloud placeholder vs dedup) are domain
  logic; no full library replacement. Keep.
- **Legacy recursive copy** (`util.rs:49-63`, ~15 LOC) — `fs_extra::dir::copy`
  or a `walkdir`-based copy; used only for sled-directory backups in
  `cache/open.rs:598`. Very low value.
- **Sidecar file lock** (`cache/lock.rs:32-73`, ~50 LOC) — `fslock` exists but
  std `try_lock` is already used correctly; no gain.

---

## F. `tools/capture` (`gircap`)

### F1. JSON line renderer — covered by A1

`tools/capture/src/report/mod.rs:281-401`, ~120 LOC, `is_plain_ascii_json`
fast path. Tests `tools/capture/src/report/tests.rs:15-54,144-182,235-282,455-540`).
Note the backslash-escaping caveat in A1.

### F2. HTTP mock servers — covered by A5

`src/http/tests.rs:686-813` and `tests/common/mod.rs:1210-1400,1869-1969`.
`md5_hex` at `tests/common/mod.rs:2136-2142` should use the crate's own
`hash::md5_hex` or `hex::encode`.

### F3. Directory walkers — covered by A6

`src/emit/index.rs:269-347`, `src/verify/list.rs:89-220`.

### F4. Retry/backoff — covered by A2

`src/http/transport.rs:10-138`, `src/http/fetcher.rs:788-850`. Keep the loop;
optionally replace only `delay_before` with `ExponentialBackoff` configured
with `jitter: None`. `reqwest-middleware` + `reqwest-retry` is a full-framework
option that needs restructuring for per-attempt logging and body accounting.

### F5. Percent encoding → `percent-encoding`

**Location.** `src/encoding.rs:18-77` (`percent_encode`, `percent_decode`,
`percent`, ~50 LOC).

**Candidate.** `percent-encoding` 2.x (rust-url ecosystem, already transitive
via `reqwest`/`url`). `const TEXT: &AsciiSet = &CONTROLS.add(b' ').add(b'%')
.add(b'\t').add(b'\n').add(b'\r');`, `percent_encode(bytes, TEXT)` and
`percent_decode_str(s).decode_utf8_lossy()`.

**Tests affected.** `src/report/tests.rs:292-320` and the text-line assertion
at `:177`.

**Expected change / caveat.** Current `percent_encode` leaves non-ASCII
characters untouched; a byte-oriented `AsciiSet` encoder would escape their
UTF-8 bytes unless bytes ≥ 0x80 are added to the allowed set. URL form must
stay uppercase. `report::percent_decode` is a public re-export
(`src/report/mod.rs:31`) — keep the path.

### F6. Hex encoding → `hex`

`src/hash/mod.rs:218-244`, covered by A3.

### F7. Windows device-name / path-shape rules

**Location.** `src/paths.rs:67-161` (`path_rule` ~72 LOC,
`is_windows_device_name`), `:179-232` (`escape_component`/`unescape_component`),
`src/config/mod.rs:635-700` (tag-shape rule, a second copy).

**Candidate.** `sanitize-filename` can supply the reserved-name predicate;
`path-clean`/`path-absolutize` can normalize `..`/`.` so
"normalized != original" detects traversal. Neither expresses the whole rule.

**Restructure.** Replace only `is_windows_device_name` (~10 LOC) and share one
rule between `paths.rs` and `config/mod.rs`. Reject-don't-rewrite semantics and
the `%` escape scheme stay hand-rolled.

**Tests affected.** `src/paths.rs:323-499`, `src/config/tests.rs`.

**Expected change / caveat.** A crate's reserved-name set may include/exclude
`CONIN$`, `CONOUT$`, `COM0` differently. The escape scheme is a bundle-format
contract in `diagnostics/path_map.json` — changing it changes artifact layout.

### F8. Error Display impls → `thiserror`

**Locations.** `src/http/error.rs:10-37,84-105` (`ApiError`),
`src/cli/mod.rs:328-358` (`UsageError`), `src/failure/mod.rs:36-120`
(`Failure`) — ~50 LOC of manual `Display`/`Error`.

**Candidate.** `thiserror` 2.x: `#[derive(thiserror::Error)]` with `#[error("…")]`.
Keep `ApiError::exit_code()`, `url()` and `Failure::summary` hand-written.

**Tests affected.** `failure/mod.rs every_failure_constructor_agrees_on_the_summary_shape`,
`http/tests.rs` message-fragment assertions.

**Expected change.** None if format strings are copied verbatim. Marginal payoff.

### F9. `IntOrStr` and range algebra — low payoff

- `src/hyp/types.rs:37-105` (`IntOrStr`, ~70 LOC) → `serde_with` 3.x
  (`PickFirst<(_, DisplayFromStr)>`) or `serde_aux`. Caveat: `PickFirst`
  re-serializes a string input as a number — acceptable only if the module's
  premise holds that typed values are never written (bodies are captured
  byte-exact). Keep if a test pins the write variant.
- `src/assemble/ranges.rs:27-113,145-235` (~180 LOC) → `rangemap` 1.5 can
  replace overlap/gap/containment checks but cannot represent inverted ranges
  or emit the A11 violation strings, which are the actual deliverable. Retain
  the diagnostics; not worth migrating.
- `src/tasks.rs:22-154` (~100 LOC bounded ordered `JoinSet` runner) — deliberately
  kept: `buffer_unordered` loses input order, `FuturesOrdered` loses the bound,
  `futures-concurrency` loses per-item panic isolation. Only the ~12-LOC panic
  payload downcast in `join_failure` is optional.

### F10. Small INI reader in `gamedir`

`src/emit/gamedir.rs:72-96` (`parse_game_version`, ~25 LOC) — covered by A4.
Keep `config_ini` (`:44-64`) hand-written for byte-exact artifact rendering.

---

## G. `tools/sled2redb`

### G1. Hand-rolled temp-file claim → `tempfile::NamedTempFile::persist_noclobber`

**Location.** `tools/sled2redb/src/convert.rs:304-324` (`replace_destination`),
`:342-379` (`claim_temp_sibling`), `:383-389` (`temp_candidate_name`),
`:398-423` (`cleanup_temp`); docs `:41-97`.

**Finding.** ~90 LOC of stat-then-`create_new` loops, `_2`/`_3` collision
spellings, `MAX_CLAIM_ATTEMPTS = 1000`, custom sidecar cleanup and error chaining.

**Candidate.** `tempfile::Builder::prefix(CONVERT_TEMP_PREFIX).suffix(".redb")
.tempfile_in(parent)`, then `.persist_noclobber(dst)` for an atomic rename with
a stronger no-clobber guarantee. Drop removes the temp, so `cleanup_temp`
shrinks dramatically.

**Restructure.** Claim the temp before reading the source (`:158-162`) as
today; still remove the girsync lock sidecar before persist (`:283-290`).

**Tests affected.** `tools/sled2redb/src/convert.rs:437-457` (pins the `_2`
spellings) and `tests/convert.rs:326-342,368-415,432-460` (overwrite, no residue
on failure, rewrite replaces).

**Expected change / caveat.** Temp names change to `tempfile`'s random suffix;
error text for a failed rename changes; the extensive docs about
pid-in-name and `_2…_1000` become obsolete and must be rewritten.

### G2. Hex decode → `hex`

`tools/sled2redb/src/convert.rs:121-133` (`hex_to_bytes`) — covered by A3.
Preserve the `.with_context(|| format!("decode {algo} hex of {rel}"))` so
`tests/convert.rs:368-415` still sees the bad filename and algorithm.

### G3. Timestamp helper

`tools/sled2redb/src/convert.rs:344` calls `girsync::util::ts_now()` — covered
by C2.

---

## H. Findings: hand-rolled code that should stay

These were examined and the research found no better library, or a library
would regress a load-bearing property. Listing them prevents wasted effort.

| Area | Location | Why keep |
|---|---|---|
| Progress reporting | `src/report/emit.rs:83-131` (`ProgressGuard`), `src/report/*` | stdout is a machine-readable data plane; `indicatif` would break the `REPORT`/`SUMMARY` contract |
| `_tmp` resume artifacts | `src/repair/file/temp.rs:716-749`, `src/repair/phases/post.rs:583-734` | durable length-based resume + MD5-gated promotion, not disposable temp files |
| OS advisory locks | `src/repair/lock.rs:68`, `tools/sync/src/cache/lock.rs:54-73` | std `try_lock` is stable and sufficient; `fs2`/`fslock` add nothing |
| Lock-file FNV key | `src/repair/lock.rs:40-43,125-170` | the exact byte sequence must not change or existing lock files are orphaned (use `fnv` only for named constants) |
| `DigestSet` / `AlgoSet` | `tools/sync/src/hash/digestset.rs`, `hash/set.rs` | 80-byte/no-allocation guarantee is load-bearing and heavily pinned |
| Symlink/reparse-point classification and removal | `tools/sync/src/link.rs:53-185` | "not every reparse point redirects" is domain logic with no library equivalent |
| Bounded ordered task runner | `tools/capture/src/tasks.rs:22-154` | needs input order + concurrency bound + per-item panic isolation simultaneously |
| Replay/bridge HTTP server | `tests/artifact_bridge/server.rs` | route families, URL-prefix rewriting and recorded failure shapes are domain logic, not a mock server |
| Range algebra / diagnostics | `src/assemble/ranges.rs`, `src/repair/phases/plan.rs` | inverted ranges and human-readable A11 violation strings are the deliverable; interval crates cover only part |
| Starward/Collapse parity rules | `src/sophon/filter.rs:24-27` (`chuncks` spelling), audio heuristic, `src/util/path.rs` refusal rules, `src/biz.rs` | deliberate observable behavior; changing them is not a clean-code win |
| `girpr-log` / `test_support` | `crates/log/src/*`, `crates/tempdir/src/test_support.rs` | policy layer and consolidated fixtures, not reinvented infrastructure |
| `clap`/`prost`/`walkdir`/`glob`/`serde`/`tracing` usage | throughout | already delegated; replacing them would be churn |

---

## I. Assumptions to validate during implementation

1. **`path-absolutize` method.** The recommendation to replace
   `src/util/path.rs:438-480` assumed `Absolutize::absolutize()` canonicalizes
   as far as the path exists. In fact `absolutize()` is purely lexical; the
   method that canonicalizes an existing prefix and appends the remainder is
   `canonicalize()`. Validate that it resolves symlinks (the current
   `contained_join` tests at `src/util/path.rs:937-1007` require symlink-escape
   detection) before using it. `dunce` does **not** replace this routine — it
   only canonicalizes fully-existing paths without the `\\?\` prefix.
2. **INI migration.** Two research positions conflict (A4). Validate which
   `configparser`/`ini` behaviors differ from the current merge/render rules
   and decide whether the byte-level `config.ini` contract matters to the
   launcher; if it does, adopt only the parser and keep the renderer.
3. **`backoff` maintenance and semantics.** Verify the current maintained
   release line and `future::retry` state threading; confirm that girpr's
   deterministic jitter is genuinely unnecessary and that gircap's
   no-jitter/diffable artifact requirement is preserved.
4. **`tracing-appender` flush gap.** `WorkerGuard` has no public mid-run
   flush; verify every `std::process::exit` path in `src/main.rs` drops the
   guard, or retain a flush seam.
5. **Interval/range crates.** `nodit`/`rangemap` may add more call-site code
   than they remove in `src/repair/phases/plan.rs` — validate by measuring the
   diff before adopting.
6. **`globset` semantics.** Validate `*` crossing `/`, `**`, dotfiles and
   `literal_separator(false)` against `src/repair/scope/filter.rs:183-198`
   before switching both filters.
7. **`percent-encoding` escape set.** Validate that bytes ≥ 0x80 stay
   unencoded in the text form (`src/report/tests.rs:292`) and that the URL
   form stays uppercase.
8. **`std::sync::Semaphore` stability.** It is still unstable; use
   `tokio::sync::Semaphore`, a small semaphore crate, or remove the girsync
   gate in favor of rayon pool sizing.
9. **Cache format break.** `postcard`/`bincode` changes girsync's on-disk
   cache; confirm that rebuilding via `girsync update --dir` is acceptable and
   that the unknown-algorithm forward-compatibility rule survives the custom
   serde adapter.
10. **`write::Decoder::finish` availability.** Confirm it exists and returns
    the inner writer (so the `HashingWriter` digest and byte count remain
    readable) in the pinned `zstd = "0.14.4"` before deleting the B1 workaround.
11. **`wiremock`/`mockito` malformed responses.** Confirm the 2–3
    truncated/lying-`Content-Length` tests can keep a small raw-socket helper;
    do not assume a well-behaved mock server can reproduce them.
12. **`junction` crate scope.** It is Windows-only by nature; keep it as a
    `cfg(windows)` dev-dependency and verify its feature flags do not require
    elevation for junction creation.
13. **`tempfile` naming assertions.** Several tests and error-message
    fixtures assume the `<prefix>-<tag>-<pid>-<nanos>-<seq>` shape; validate the
    blast radius of switching to random names.
14. **Cross-crate direct dependencies.** `crates/tempdir` currently depends on
    the full `girsync` library for fixtures; validate whether relocating those
    fixtures behind a girsync test-support feature changes any public API.
