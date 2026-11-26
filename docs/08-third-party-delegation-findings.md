# Third-party delegation audit — findings and assumptions

Research date: 2026-11-26. Source state: `main` at `73ff053a`. Scope: every workspace member
(the root package, `app/*`, `crates/*`, `tools/sync*`, `tools/capture*`). Method: read-only source
survey divided by area, with crate metadata checked against crates.io. Nothing was changed and no
implementation was attempted; every item below is a finding to be validated at implementation time.

This document intentionally contains **only assumptions and findings**. It is not an implementation
plan and does not order the work.

---

## Assumptions

- **A1 — No validation by compilation.** Line numbers, call sites and test names come from the
  surveyed tree at the commit above. A finding may be invalidated by a closer read of a call site.
- **A2 — Library metadata is a snapshot.** Versions, download counts and maintenance status were
  checked on 2026-11-26. All named crates are published, cross-platform on Windows and Linux (except
  `junction`, which is recommended only inside `#[cfg(windows)]` code it already replaces), and have
  a substantial user base. Re-check the version at implementation time.
- **A3 — No MSRV constraint.** The workspace targets stable Rust (no `rust-version` floor). Crates
  with an MSRV are acceptable; that field is informational here.
- **A4 — Behavior changes are acceptable.** Observable output, byte spelling, error text, exact
  log formatting, generated fixture bytes and golden snapshots may all change. Backwards
  compatibility is not required.
- **A5 — Tests that pin replaced behavior may be removed**, as may tests that only exercise the
  mechanics of a hand-rolled implementation rather than the behavior the library now guarantees.
  Tests of the intended function should be adapted, not deleted.
- **A6 — "Already in `Cargo.lock`" means transitively present.** Promoting such a crate to a direct
  dependency changes manifests but adds no crate to the resolved graph. Crates not in the lockfile
  are called out explicitly.
- **A7 — Feature unification matters.** Enabling `serde_json/preserve_order` anywhere in this
  workspace would change `serde_json::Value` key ordering for unrelated consumers
  (e.g. `app/report/src/report/mod.rs` deliberately avoids it). Findings that need ordered JSON
  propose a local ordered representation instead.
- **A8 — Project-local `girpr-*` leaves are deliberate.** Many mechanics are already extracted into
  narrow leaf crates holding policy, not boilerplate. Those are generally *not* candidates; the
  explicit non-findings at the end record this.
- **A9 — `std::fs::File::lock`/`try_lock` are stable on current Rust.** No new file-lock dependency
  is needed merely to acquire an advisory lock; the remaining lock code is policy (identity,
  fail-closed checks, cleanup), not missing mechanics.
- **A10 — Several findings replace *test* servers or *test* helpers.** Blast radius is confined to
  tests even when many files change. Findings that touch production paths are marked as such.
- **A11 — Version numbers quoted below come from crates.io API**, not from `Cargo.lock`, unless
  stated otherwise. Two corrections to earlier research are included: `criterion` is `0.8.2`, and
  the suggested `json-line` crate does not exist on crates.io (finding SYNC-6 uses `serde_json`
  alone).
- **A12 — Library guarantees, not library quirks, are the contract.** After adoption, no test should
  pin the library's internal formatting, header casing or iteration details beyond what its
  documented API guarantees.

---

## Cross-cutting findings

### X1 — At least ten independent hand-rolled HTTP/1.1 servers

Locations (partial list):
`crates/loopback-mock/src/lib.rs:1-317`;
`tools/capture/test-support/src/lib.rs:11-18, 1235-1266, 1888-2153`;
`src/test_origin.rs:17-275`;
`app/test-fixtures/src/artifact_server.rs:319-328, 930-1030, 1063-1098`;
`app/test-fixtures/src/lib.rs:926-962, 1525-1543`;
`app/fetch/src/hyp.rs:842-899`;
`tools/capture/src/hyp/mod.rs:659-691, 996-1075`;
`tools/capture/src/blobs/tests.rs:107-140, 740-777, 864-902, 997-1033`;
`tools/capture/src/http/tests.rs:147-186`;
`tools/capture/src/http/probe.rs:861-886`;
`tools/capture/http/src/tests.rs:823-862`.

Each binds a loopback `TcpListener`, parses the request line and headers by hand, writes a
`HTTP/1.1 <status>` status line plus `Content-Length`, and implements its own accept-loop lifecycle
and shutdown. The project even documents the rationale in two places
(`tools/capture/test-support/src/lib.rs:11-18`, `crates/loopback-mock/src/lib.rs:1-7`), yet the
implementations have grown to hundreds of lines plus duplicated framing.

Libraries: `axum 0.8.9` (Tokio team, MIT, MSRV 1.80, ~623M downloads; `hyper 1.13` is already in
the lockfile) for servers that need routing, gates and held-open responses;
`wiremock 0.6.5` (~111M downloads) or `httpmock 0.9.0` for pure canned-reply servers with request
journals. Both are cross-platform and actively released.

Adaptation: keep each caller's reply/scripting policy as router state, but delegate listener, HTTP
parsing, keep-alive, routing and header writing. The one deliberately malformed raw-socket helper
(`tools/capture/src/http/tests.rs:114+`, lying `Content-Length`, omitted header) has no library
substitute and should remain raw. Test impact: recorded request headers/bodies and golden files will
change spelling; regenerate rather than delete semantic assertions. Lifecycle tests such as
`crates/loopback-mock/src/lib.rs:328-364 drop_does_not_hang_on_an_unfinished_request_line` become
obsolete because the framework owns shutdown.

### X2 — URL assembly and percent-encoding reimplemented several times

Locations: `app/fetch/src/hyp.rs:401-416, 669-696, 719-735`; `app/dispatch/src/url.rs:104-164`;
`app/persistent-plan/src/placement.rs:142-184`; `tools/capture/src/biz/mod.rs:195-228, 318-351`;
`tools/capture/src/http/probe.rs:577-611`; `tools/capture/test-support/src/lib.rs:1915-1923,
2097-2130`; `app/test-fixtures/src/artifact_server.rs:1063-1098`.

Two hand-written URL grammars (the capture code explicitly calls one "a second URL grammar living
beside" the other) plus three hand-written percent-encoders/decoders over `A-Za-z0-9-_.~` and
`%XX`/`+`.

Libraries: `url 2.5.9` and `form_urlencoded 1.2.3` are already in the lockfile; `percent-encoding
2.3.2` is already there too. `Url::query_pairs_mut()`, `Url::join`, `form_urlencoded::parse` and
`Serializer` replace all of the above.

Adaptation: store base URLs as `Url` at parse time; decide once per program whether bracketed keys
such as `game_ids[]` are sent literally or as `%5B%5D`. Test impact: exact URL pins
(`app/fetch/src/hyp.rs:1158,1181,1204`; `tools/capture/src/http/tests.rs:9-30`;
`tools/capture/src/http/probe.rs:689-735`; `app/dispatch/src/tests.rs:244,591`) need regeneration.

### X3 — Test helpers that parse text emitted by other libraries

Locations: `tests/cli_contract.rs:66-205` (clap `--help` layout parser), `crates/tempdir/src/test_support.rs:39-62`
(ANSI stripper), `src/repair/phases/check.rs:587-652` and `tests/repair_machinery.rs:318-342`
(tracing human-format parsing).

Libraries: `trycmd 1.2.1` for CLI snapshots; `strip-ansi-escapes 0.2.1` (~65M downloads) for
escape stripping; `tracing-subscriber 0.3.23` with its `json` feature plus already-present
`serde_json 1.0.154` for structured log capture. `assert_cmd 2.2.2` with `predicates 3.1.4` replaces
hand-built `(code, stdout, stderr)` tuples in `tests/common/mod.rs:51-64`,
`tests/cli_contract.rs:46-64`, `tests/process.rs:24-40`.

Test impact: whole-output snapshots replace per-flag help assertions; the "numeric token is not a
level" workaround (`src/repair/phases/check.rs:1064-1075`) disappears once logs are structured.
The async process seam in `tests/common/mod.rs` must stay for mock-backed tests; `assert_cmd` is
synchronous.

### X4 — Small hand-written bit/hash/map mechanics

`fnv 1.0.7` (already locked) replaces the FNV-1a loops in `app/lock/src/lib.rs:145-190` and
`tools/capture/test-support/src/lib.rs:207-214`; `bitflags 2.13.2` (already locked) replaces
`tools/sync/cache/src/hash/set.rs:7-96`; `strum 0.28.0` (~807M downloads) replaces the enum name
tables in `tools/sync/cache/src/hash/algo.rs:25-47,75-86,211-269`, `crates/model/src/region.rs:76-83,140-144`
and `crates/log/src/level.rs:40-82`. These are true mechanics delegations with no policy loss if
spelling and canonical order are preserved.

---

## Findings — root package (`src/`, root `tests/`)

### R1 — Hand-rolled bounded parallel map in the purge stage — production
- **Location:** `src/repair/purge.rs:28-96`; call sites `:245` (stat) and `:376` (delete).
- **Code:** `MAX_PURGE_THREADS` plus `parallel_map`: `thread::scope`, manual chunking into
  `Vec<Option<R>>` slots, `available_parallelism().clamp(1, 8)`, and an `unreachable!` pass to fill
  the `Option`s. Exists to overlap independent `metadata()`/`remove_file()` calls in input order.
- **Library:** `rayon 1.12.0` (already in the lockfile via the dev tools; MSRV 1.80, MIT/Apache,
  widely used). `par_iter().map(f).collect::<Vec<_>>()` is this helper with work stealing and
  ordered indexed collect.
- **Adaptation:** delete both helpers, add `rayon::prelude::*`; if the eight-thread ceiling is
  required, `ThreadPoolBuilder::new().num_threads(8)` with `pool.install(...)`. Callers already emit
  logs on the calling thread, so the thread-local tracing dispatcher concern in the comment does not
  change.
- **Tests:** `parallel_delete_emits_purge_lines_in_plan_order` (`:910-940`) should pass unchanged;
  no test pins worker count or slot mechanics.
- **Assumptions:** `T: Sync, R: Send` already hold; work stealing does not change the ordering that
  matters to the log sweep.

### R2 — Hand-rolled HTTP/1.1 test server — test only
- **Location:** `src/test_origin.rs` (whole file; `Reply :17-50`, `InFlight :60-133`, `serve
  :177-221`, `handle_connection :224-275`).
- **Code:** raw `TcpListener` + `JoinSet`, request-target extraction via `buf.windows(4)` /
  `split_whitespace().nth(1)`, hand-written status line, plus `NeverAfterHit` / `StatusAfterHit` /
  `GatedBody` reply variants for gating and in-flight counting.
- **Library:** `axum 0.8.9` — `Router` + `axum::serve`; `wiremock 0.6.5` is viable for the simpler
  canned cases but a worse fit for held-open/gated responses.
- **Adaptation:** keep `Reply`/`Route` policy as handler state and keep `InFlight` with its
  `Notify`/watch gate; delete the accept loop and parser. A handler can hold a request open with
  `std::future::pending()`.
- **Tests:** setup in `src/repair/phases/manifest.rs:554-865` needs rewriting; the attempt-count,
  cancellation, bounded-concurrency and timeout assertions stay.
- **Assumptions:** a dev-dependency on axum is acceptable; `TestServer::base()` stays a string.

### R3 — Parsing clap's rendered `--help` text by hand — test only
- **Location:** `tests/cli_contract.rs:66-205` (`strip_ansi`, `HELP_*`, `option_body`, `help_entry`),
  helpers at `:482,514,531`, tests throughout `:224-592,751,790,852,1013`.
- **Code:** ~150 lines of indentation/token parsing reconstructing clap's flat help layout,
  including CRLF handling, to assert that a flag's help entry exists and says something. This pins
  clap's *rendering*, not its behavior.
- **Library:** `trycmd 1.2.1` (from the clap/assert-rs maintainers, MSRV 1.85) with `.trycmd` cases
  and `.stdout` snapshots; `clap 4.6.9` introspection (`Command::get_arguments()`, `Arg::get_long()`)
  for flag presence. `strip-ansi-escapes 0.2.1` if the helper survives at `:1365`.
- **Tests:** the help-layout tests named above, plus `src/cli.rs:579-593`, are replaced by snapshots
  or removed. Runtime tests (exit codes, JSON plane, log-file echo) stay as ordinary Rust tests.
- **Assumptions:** pinning whole help output is an acceptable change in what is asserted; CLI output
  itself need not change.

### R4 — Hand-built process-spawn/output tuples — test only
- **Location:** `tests/common/mod.rs:51-64`, `tests/cli_contract.rs:46-64`, `tests/process.rs:24-40`.
- **Code:** each spawns `CARGO_BIN_EXE_girpr`, lossy-decodes stdout/stderr, extracts
  `status.code().expect(...)`, and returns `(i32, String, String)` destructured by every caller.
- **Library:** `assert_cmd 2.2.2` + `predicates 3.1.4`.
- **Adaptation:** `assert_cmd::Command::cargo_bin("girpr").assert().code(n).stdout(...)`. The async
  `tokio::process::Command` seam in `tests/common/mod.rs` stays for tests that share a runtime with
  the mock server; migrate the synchronous `cli_contract.rs::run` first.
- **Tests:** every tuple-destructuring call site must be rewritten as chained assertions; lock
  cleanup (`remove_run_lock_for_args`) stays explicit.
- **Assumptions:** no product behavior changes.

### R5 — Hand-rolled "settle primary, retain sibling" combinator — test/retry policy
- **Location:** `src/repair/phases/metadata.rs:163-179` (`Settled`, `wait`), `:190-210`
  (`settle_primary`), call sites `:261,399`.
- **Code:** a ~20-line `tokio::select!` loop polling a primary and sibling future so that the
  sibling's value is kept if it settles first, while the sibling is cancelled otherwise.
- **Library:** `futures-util 0.3.34` (already a direct dependency with `std`): `future::select`
  and `Either`.
- **Adaptation:** `Either::Right` must still await the primary to completion — the "never cancel the
  primary" rule is load-bearing. `Settled` can remain or become `Either`.
- **Tests:** `a_settled_primary_cancels_the_sibling_retry_schedule` (`:650`),
  `a_sibling_error_is_kept_when_the_primary_succeeds` (`:692`),
  `a_config_failure_is_authoritative_over_a_retrying_branch` (`:726`) should pass unchanged.
- **Assumptions:** boxing the two futures is acceptable.

### R6 — Conservative salvage-order heuristic where a graph library exists — production, higher risk
- **Location:** `src/repair/reuse.rs:50-56,83-172`; consumed by `src/repair/scan.rs:260`,
  `src/repair/file/inplace.rs:519-520`, `src/repair/file/temp.rs`.
- **Code:** counts two trivially computable directions (`r_i >= o_i` ascending, `r_i <= o_i`
  descending) and picks the larger. The comment explicitly defers the optimum: topological order
  over "chunk j's write clobbers chunk i's source", breaking cycles by downloading.
- **Library:** `petgraph 0.8.3` (MIT/Apache, MSRV 1.64, ~641M downloads) — `DiGraph` with
  `toposort` and explicit cycle handling.
- **Adaptation:** build nodes per bad chunk, edges on `W_j ∩ R_i`, topologically sort, break cycles
  by download, retain the current rule as tie-break/fallback. Because correctness is byte-level, a
  narrow leaf crate with property tests would be needed before changing writer call sites.
- **Tests:** `salvage_order_source_boundary_cases` (`reuse.rs:228`) and the worked multi-chunk cases,
  plus `est_download_bytes`/`chunks_reused` pins in `src/repair/scan.rs`, `src/repair/file/temp.rs`,
  `src/repair/file/inplace.rs` and `tests/repair_scan.rs`, would need re-baselining.
- **Assumptions:** this is a lower-confidence finding; the current rule has a proven safety argument
  that must be re-established. Reuse counts changing is acceptable; writing bad bytes is not.

### R7 — Hand-rolled reorder buffer for out-of-order manifest fetches — production
- **Location:** `src/repair/phases/manifest.rs:100-144` (`OrderedResults`), `:154-232`
  (`ManifestFetches`, `ManifestStream`, `from_stream`, `next_ordered`); tests `:447,478`.
- **Code:** `Vec<Option<T>>` + cursor converts `buffer_unordered` results into selection order so
  the first selection-order failure is reported and later fetches are cancelled.
- **Library:** `futures-util 0.3.34` — `stream::FuturesOrdered` (already a direct dependency).
- **Adaptation:** keep concurrency bounded by acquiring the existing `ManifestBudget` semaphore
  *inside* each future; the test seam at `from_stream` changes type.
- **Tests:** `results_are_reordered_incrementally_in_selection_order` (`:447`) is removed;
  `ordered_results_follow_selection_order_through_an_out_of_order_stream` (`:478`),
  `fetch_latest_reports_the_first_failure_in_selection_order` (`:605`), and
  `local_reuse_map_keeps_the_union_and_selection_order` (`:874`) are adapted.
- **Assumptions:** `FuturesOrdered` alone does not bound concurrency; wiring the semaphore must not
  regress peak manifest memory.

### R8 — Tests parse `tracing`'s human log format by hand — test only
- **Location:** `src/repair/phases/check.rs:587-652` (`logged_level`, `log_records`,
  `check_warnings`, `check_summary`), used at `:723-747,857,938,1064-1075`;
  `tests/repair_machinery.rs:318-342`.
- **Code:** text parsing of `"<timestamp> <LEVEL> <target>: <message>"` with a special rule that
  numeric tokens earlier in the prefix are not levels because `str::parse::<Level>()` accepts `"2"`.
- **Library:** `tracing-subscriber 0.3.23` with `json`, plus `serde_json 1.0.154` (both already
  present).
- **Adaptation:** install a JSON-formatted capturing subscriber in `girpr-log::test_support` and
  read `level`/`message` fields directly.
- **Tests:** the boundary tests around `:1064-1075` become unnecessary; warning-set/summary tests
  keep their assertions via structured fields.
- **Assumptions:** `girpr-log` must gain a JSON capture surface; no shipped log format changes.

---

## Findings — `app/*`

### APP-1 — Hand-written INI parser / merger / renderer — production
- **Location:** `app/util-ini/src/ini.rs:334-388` (`write_config_ini`), `:390-419` (`Section`),
  `:421-439` (`find_section`/`ensure_general`), `:441-495` (`parse_ini`), `:497-520` (`merge_ini`),
  `:522-540` (`render_ini`); `:10-57` (`read_game_version`).
- **Code:** ~150 lines splitting on the first `=`, lowercasing section/key names, merging duplicate
  sections, dropping comments, forcing `[General]` keys, sorting keys, emitting blank lines, then
  publishing atomically; plus a separate "last match wins" raw line scan for the game version.
- **Library:** `rust-ini 0.21.3` with `features = ["case-insensitive"]` (149M downloads, MSRV 1.64,
  cross-platform; not currently in the lockfile). `configparser 3.3.0` is an alternative but its
  lowercasing `HashMap` storage fits less well.
- **Adaptation:** replace `Section` with `ini::Ini`, move headerless keys into `Some("General")`,
  retain a thin policy layer for the forced keys, sort order and blank-line shape, and keep handing
  the rendered string to `write_atomic`. `read_game_version` can stay a deliberate Starward-parity
  line scan or become `Ini::get` (changing last-match to first-match semantics).
- **Tests:** byte-exact pins at `ini.rs:575,589,604,915,973,1044,1352,1401` need adjustment or
  removal.
- **Assumptions:** `rust-ini` canonicalizes escaping (non-ASCII may become `\x…`); game values are
  effectively ASCII.

### APP-2 — Percent-encoder and `format!` URL builders in fetch — production
- **Location:** `app/fetch/src/hyp.rs:401-416` (`hyp_url`), `:669-696` (`sophon_build_url`,
  `patch_build_url`), `:719-735` (`urlencode`).
- **Code:** `format!`-built query strings and a custom RFC3986 encoder escaping every byte outside
  `[A-Za-z0-9-.~_]`, even though `reqwest::Url` is already used in `sophon/stream.rs:680-743`.
- **Library:** `url 2.5.9` (already in the lockfile).
- **Adaptation:** store bases as `Url`, use `query_pairs_mut().append_pair` and `Url::join`; decide
  explicitly how `game_ids[]` is spelled. If byte-for-byte output matters, use already-locked
  `percent-encoding 2.3.2` with a custom `AsciiSet` instead.
- **Tests:** `hyp.rs:1158,1181,1204` pin exact strings including `%20` and literal `[]`;
  `app/test-fixtures/src/artifact_server.rs:1063-1098` (`query_value`/`urldecode`) becomes
  unnecessary.
- **Assumptions:** the server accepts form-style `+`/`~`/`%5B%5D` normalization, or the exact
  `AsciiSet` is retained.

### APP-3 — Custom serde "number or string" visitor in fetch — production
- **Location:** `app/fetch/src/hyp.rs:236-270` (`num_from_string`), applied at `:286,288`; tests
  `:769-823`.
- **Code:** a 35-line `Visitor` accepting a JSON integer or numeric string with custom error text.
- **Library:** `serde-aux 4.7.0` (32M downloads, MSRV 1.62, MIT; not in the lockfile) —
  `field_attributes::deserialize_number_from_string` is exactly this contract.
- **Adaptation:** `#[serde(default, deserialize_with = "deserialize_number_from_string")]` on
  `compressed_size`/`uncompressed_size`; delete the visitor.
- **Tests:** `num_from_string_both_shapes` (`hyp.rs:780`) pins exact error substrings that will
  change; `negative_sizes_are_kept_for_the_consumer_to_clamp` (`:819`) should still pass.
- **Assumptions:** error wording is not an output contract; `i64` range and negative values survive.

### APP-4 — XML tag scanner in dispatch — production
- **Location:** `app/dispatch/src/metadata.rs:254-294` (`rsa_key_from_xml`), especially `:255-267`.
- **Code:** `find("<Modulus>")`/`find("</Tag>")` substring extraction for the five RSA components,
  base64 decode, `BoxedUint::from_be_slice`, `PrivateKey::from_components`; deliberately ignores
  .NET `InverseQ`.
- **Library:** `quick-xml 0.42.0` (580M downloads, MSRV 1.86, actively maintained).
- **Adaptation:** deserialize a struct with five renamed `String` fields via `quick_xml::de::from_str`;
  keep the base64/key construction. Unmapped `InverseQ` becomes "field not in struct".
- **Tests:** `dispatch/src/tests.rs:382 xml_key_ignores_inverse_q_and_tampered_components_fail`
  should survive; tests relying on naive substring semantics would be removed.
- **Assumptions:** the key XML is well formed; tolerating whitespace/attributes/entities is an
  improvement.

### APP-5 — Manual dispatcher URL assembly and query validation — production
- **Location:** `app/dispatch/src/url.rs:104-136` (`build_dispatcher_url`), `:138-164`
  (`validate_dispatcher_url`); `app/dispatch/src/dispatch.rs:172-179,275-307`;
  `app/persistent-plan/src/placement.rs:142-184` (`asset_urls`).
- **Code:** `replacen` + `split_once("://")` to force a host, `split(['?','&'])` to check fixed query
  parameters, and `format!` + `trim_end_matches('/')` for CDN URLs.
- **Library:** `url 2.5.9` (already in the lockfile).
- **Adaptation:** return `Url` from the builder, validate with `query_pairs()`, build CDN URLs with
  `join`/`set_path`.
- **Tests:** `dispatch/src/tests.rs:244` and `:591` pin exact URL strings and need byte adjustment
  (default ports, trailing slashes, escaping).
- **Assumptions:** the server accepts equivalent URLs; normalization does not change requests.

### APP-6 — `cmd /c mklink /J` junction fallback — production, Windows only
- **Location:** `app/util-path/src/junction_support.rs:78-124` (`try_junction_dir`,
  `junction_command`), `:152-163` (`reject_percent_path`), `:166-173` (`run_command_line`).
- **Code:** shells out to `cmd`, hand-quotes paths, refuses any `%` in a path because `cmd` expands
  `%NAME%` even inside quotes, then inspects the reparse point.
- **Library:** `junction 2.1.0` (Win32 API, 7.1M downloads, MIT, published 2026-09-24, MSRV 1.85) —
  `junction::create`, `exists`, `get_target`. Windows-only, matching the code it replaces.
- **Adaptation:** replace the command path with `junction::create(target, link)`; keep the
  skip/fail classifier and `JunctionFixture` policy. The `%` refusal and shell quoting disappear.
- **Tests:** `app/util-path/src/path.rs:1218`, `app/util-ini/src/ini.rs:1188` and the classifier
  unit tests need updates for new error kinds.
- **Assumptions:** NTFS; `junction::create` supports dangling targets as its docs show.

### APP-7 — Hand-rolled FNV-1a key hash in the run lock — production
- **Location:** `app/lock/src/lib.rs:40-43` (constants), `:145-190` (`key_for`, `fnv_step`, `fnv_mix`).
- **Code:** FNV-1a over a lossy path string then raw OS bytes for non-UTF-8 paths, with ASCII case
  folding on Windows.
- **Library:** `fnv 1.0.7` (already in the lockfile).
- **Adaptation:** use `FnvHasher`, `write(bytes)` for the lossy string, a `0xff` separator, then raw
  bytes / folded UTF-16 units; keep case folding in the caller.
- **Tests:** `lock/src/lib.rs:797,824,841,853` pin case folding and distinct keys; they keep working
  only if byte order is preserved.
- **Assumptions:** preserving the exact offset/prime/order keeps existing lock filenames; if not,
  old lock files simply become unrelated.

### APP-8 — Hand-rolled protobuf wire decoder in dispatch — production
- **Location:** `app/dispatch/src/proto.rs:89-98` (`read_varint`), `:100-157` (`walk_fields`).
- **Code:** manual varint and tag/wire-type/fixed32/fixed64/length decoding, purely to log byte
  lengths of opaque fields omitted from the checked-in `.proto`.
- **Library:** `prost 0.14.6` (already a direct dependency elsewhere) — `encoding::decode_varint`
  and `encoding::decode_key`.
- **Adaptation:** replace only the decoding primitives; keep `WireValue`/`OpaqueFields` and the
  "never used for placement" policy. Map prost's `DecodeError` into `DispatchError::Proto`.
- **Tests:** `proto.rs:256 walker_reads_every_wire_type_and_rejects_a_group` should remain; error
  strings change.
- **Assumptions:** prost rejects overlong encodings; that is acceptable.

### APP-9 — Base64 padding normalization — production
- **Location:** `app/dispatch/src/crypto.rs:15-21` (`decode_base64`).
- **Code:** filters whitespace, appends `=` padding, then `STANDARD.decode`.
- **Library:** `base64 0.23.1` (already a direct dependency) — `general_purpose::STANDARD_NO_PAD`.
- **Adaptation:** `STANDARD_NO_PAD.decode(compact.as_bytes())`, keeping the whitespace filter; keep
  `STANDARD` as a fallback if both shapes occur.
- **Tests:** covered indirectly by `dispatch/src/tests.rs:244,321,402`.
- **Assumptions:** extra `=` may need tolerance depending on launcher fields.

### APP-10 — Hex→integer and hex round-trip in persistent-plan — production
- **Location:** `app/persistent-plan/src/manifest.rs:99-106` (`parse_hash_hex`),
  `app/persistent-plan/src/digest.rs:140-145` (`xxhash64_u64`).
- **Code:** manual 16-hex-digit validation + `from_str_radix`; the digest helper computes a hash,
  renders hex, then re-parses the hex back to `u64`.
- **Library:** `hex 0.4.3` (already in the lockfile); the XXH64 hasher behind `girpr-digest` can
  return `finish()` directly.
- **Tests:** `manifest.rs:127` (uppercase hex), `:135` (error text), `digest.rs:232`
  (stream/in-memory agreement) need review.
- **Assumptions:** uppercase acceptance is retained; error wording is not an output contract.

### APP-11 — Hand-rolled HTTP mock server and query parser in test-fixtures — test only
- **Location:** `app/test-fixtures/src/artifact_server.rs:930-1030` (`accept_loop`/`serve_one`),
  `:1063-1098` (`query_value`, `urldecode`); `app/test-fixtures/src/lib.rs:926-962,1525-1543,70-76`.
- **Code:** raw accept loops, manual target splitting and percent-decoding, hand-formatted
  `HTTP/1.1 …` headers, `Mutex<Vec<_>>` request journals.
- **Library:** `wiremock 0.6.5`; plus `url`/`percent-encoding` for query decoding.
- **Adaptation:** mount mocks for each HYP/getBuild/getPatchBuild/manifest/chunk route and read
  `received_requests()`. A thin custom server may still be needed for deliberately truncated bodies
  and `on_first_chunk` hooks that wiremock cannot express.
- **Tests:** nearly every consumer of `MockServer::chunk_hits`, `request_targets`, `first_target_with`,
  `truncate_bytes`, `on_first_chunk` in `app/fetch/tests/*` and `app/audio/tests/*` needs rewriting.
- **Assumptions:** the truncated-body behavior justifies a retained small server if needed.

### APP-12 — Duplicated BOM stripping — minor
- **Location:** `app/util-ini/src/ini.rs:156`, `app/audio/src/lib.rs:133-135`.
- **Code:** two one-line `strip_prefix('\u{feff}')` calls.
- **Library:** `unicode-bom 2.0.3` (64M downloads, stable).
- **Tests:** `audio/lib.rs:296,317` and `ini.rs:708-748` pin BOM behavior and should stay.
- **Assumptions:** low payoff; may be deliberately left alone.

---

## Findings — `crates/*`

### LEAF-1 — `ScriptedServer`: hand-rolled HTTP server, parser, writer and lifecycle — test only
- **Location:** `crates/loopback-mock/src/lib.rs:1-317` (notably `:26-91`, `:93-226`, `:228-317`);
  `Cargo.toml` declares no dependencies.
- **Code:** own listener, driver thread, request-line/header parsing, case-insensitive `Headers`,
  status-phrase table `reason()`, hit counting, stop flag + socket shutdown + `join()` on drop.
- **Library:** `wiremock 0.6.5` (or `mockito` if a synchronous `start()` matters); alternatively
  `httpmock 0.9.0`. For a smaller change, `http 1.5.0` (already in the lockfile) supplies
  `StatusCode::canonical_reason()` and `HeaderMap`.
- **Adaptation:** keep `Reply`/`RecordedRequest`/`ScriptedServer` as a thin wrapper: map replies to
  `ResponseTemplate`, register ordered mocks with `up_to_n_times(1)` (last repeats), use `expect(n)`
  for `start_expecting`, and translate the journal into the local types. Delete the driver thread,
  `serve_one`, `reason()` and the custom `Headers`.
- **Tests:** `drop_does_not_hang_on_an_unfinished_request_line` (`:328-364`) becomes obsolete;
  downstream assertions on `base()`, `hits()`, `requests()` and header values stay.
- **Assumptions:** adding dev-dependencies to the previously dependency-free leaf is acceptable;
  "last reply repeats" and "assert hit count on drop" are reproducible.

### LEAF-2 — Hand-written ANSI stripper in test support — test only
- **Location:** `crates/tempdir/src/test_support.rs:39-62`.
- **Code:** char loop skipping `ESC`, `[` and consuming to an alphabetic byte or `~`; does not
  handle OSC sequences or the full CSI grammar.
- **Library:** `strip-ansi-escapes 0.2.1` (65M downloads, stable, tiny API).
- **Adaptation:** replace the body with `strip_ansi_escapes::strip_str(s)`; no signature change.
- **Tests:** no local unit test; downstream console-output assertions stay.
- **Assumptions:** downstream only needs plain text; removing more sequences than before is fine.

### LEAF-3 — `cmd /c mklink /J` junction fallback in test support — Windows, test only
- **Location:** `crates/tempdir/src/test_support.rs:125-174` (`dir_link`, especially `:138-164`).
- **Code:** after `symlink_dir` fails, spawns `cmd`, rewrites `/` to `\` with a closure, and formats
  combined error text from stderr.
- **Library:** `junction 2.1.0`.
- **Adaptation:** call `junction::create(link, target)` in the Windows branch; keep the original
  error as context; remove `Command`, `native()` and stderr parsing.
- **Tests:** no test pins the `cmd` invocation; `skip_if_no_link!` behavior stays.
- **Assumptions:** the same absolute/native targets are accepted and the target exists.

### LEAF-4 — Hand-maintained protobuf structs and tags
- **Location:** `crates/proto/src/lib.rs:29-166` (nine hand-written structs with `#[prost(...)]`
  tags); `Cargo.toml:23-30`.
- **Code:** wire layout, the misspelled `chuncks`, opaque tag-6 and serde JSON keys maintained by hand
  from a pinned upstream `Sophon.proto`.
- **Library:** `prost-build 0.14.6` (official prost project, MSRV 1.85, ~448M downloads) plus a
  vendored `protoc` such as `protoc-bin-vendored` so Windows and Linux builds need no system protobuf.
- **Adaptation:** vendor the pinned `.proto`, add `build.rs` with
  `Config::new().type_attribute(".", "#[derive(serde::Serialize)]").compile(...)`, delete the
  hand-written structs, keep explanatory docs as module comments.
- **Tests:** keep `tag_six_is_the_opaque_int64_passthrough` (`:175-190`) and
  `patch_manifest_delete_tag_round_trips` (`:195-237`); add a regression for dump keys `chuncks` and
  `unknown_field6`.
- **Assumptions:** upstream field names match the frozen artifacts; adding a build script/protoc to
  the toolchain story is acceptable.

### LEAF-5 — Glob matching plus hand-written ancestor walker
- **Location:** `crates/filter/src/lib.rs:18-90`; `Cargo.toml:18-25`.
- **Code:** `matches_or_ancestor` re-implements prefix walking on `/` boundaries with a manual
  `end` index; `compile` normalizes `\`.
- **Library:** `globset 0.4.22` (BurntSushi/ripgrep, 307M downloads, MSRV 1.88, active).
- **Adaptation:** compile `GlobSet` with `case_insensitive(!case_sensitive)` and
  `literal_separator(false)`; fold the ancestor rule in by adding `pattern/**` alongside `pattern`.
  Keep backslash normalization and the `anyhow` error wording.
- **Tests:** keep `separators_and_leading_dots_are_not_special` (`:101-119`),
  `backslash_mode_controls_normalization` (`:121-135`, minus two `as_str()` pins) and
  `ancestor_matching_respects_the_separator_boundary` (`:137-144`); add a `**` case.
- **Assumptions:** the re-export change touches callers in both products; `globset`'s default
  "dot is not special" matches the current rule.

### LEAF-6 — Boxed-future plumbing for the injectable sleep hook
- **Location:** `crates/retry/src/lib.rs:106-181` (`SleepFuture`, `Sleep`, `RealSleep`, `ZeroSleep`,
  `RecordingSleep`).
- **Code:** `Pin<Box<dyn Future + Send + 'static>>` exists only because a dyn-compatible trait
  cannot use `async fn`; each implementation manually `Box::pin`s a trivial future.
- **Library:** `async-trait 0.1.92` (853M downloads, MSRV 1.71).
- **Adaptation:** `#[async_trait] pub trait Sleep { async fn sleep(&self, d: Duration); }`, delete
  `SleepFuture` and the `Box::pin` sites. The macro still boxes internally — the win is removing
  hand-written type plumbing.
- **Tests:** keep `schedule_contracts` (`:190-258`) and
  `zero_and_recording_sleepers_return_immediately` (`:260-276`).
- **Assumptions:** if a caller relies on `RecordingSleep` recording at *call* time rather than first
  poll, that expectation changes. `backoff 0.4.0` stays the schedule engine — `backoff::future::retry`
  would absorb caller-owned retry policy and is **not** recommended.

### LEAF-7 — Hand-written enum↔string tables — low priority
- **Location:** `crates/model/src/region.rs:76-83,140-144`; `crates/log/src/level.rs:40-82`.
- **Code:** separate `as_str`, `Display` and `parse` match tables; `Level::parse` also has a
  `warning` alias and custom error text.
- **Library:** `strum`/`strum_macros 0.28.0` — derive `Display`, `EnumString`, `EnumIter`.
- **Adaptation:** annotate variants with `#[strum(serialize = "hk4e_cn", …)]` and keep thin `parse`
  wrappers for the alias/error text and existing `clap::ValueEnum` derive.
- **Tests:** keep `every_region_has_one_id_and_one_set_of_ids` (`region.rs:163-250`) and
  `every_level_parses_in_any_case_and_rejects_the_rest` (`level.rs:88-104`).
- **Assumptions:** modest payoff; `endpoints()` tables remain local.

### LEAF-8 — Lexical path normalization in `path-shape` — medium/low confidence
- **Location:** `crates/path-shape/src/lib.rs:99-183` (`validate`).
- **Code:** `split('/')`, two `Vec<&str>` passes, manual `.`/empty filtering, `contains(&"..")`,
  string reassembly, with a fixed first-violation order.
- **Library:** `relative-path 2.0.3` (MSRV 1.85, 139M downloads) — `RelativePath` and `normalize()`.
- **Adaptation:** use it for the `/`-separated component classification, but keep all caller-supplied
  device/colon/control/trailing-dot rules and the exact `ShapeProblem` order.
- **Tests:** all tests at `path-shape/src/lib.rs:223-340`, especially
  `strict_reports_the_first_violation_in_the_fixed_order` (`:258-269`) and
  `normalizing_returns_the_clean_slash_form` (`:285-303`), must be kept.
- **Assumptions:** partial delegation only; if `normalize()` cannot preserve the first-violation
  contract or collapses `..` instead of rejecting it, keep the current loop.

### LEAF-9 — Test scratch paths built from pid + counter — minor
- **Location:** `crates/fs-identity/src/lib.rs:184-205` (`Scratch::new`),
  `crates/log/src/install.rs:120-128` (`scratch_dir`).
- **Code:** `env::temp_dir().join(format!("…-{pid}-{n}"))` with bespoke `Drop`/`remove_dir_all`.
- **Library:** `tempfile 3.27.0` (already present via `girpr-tempdir`).
- **Adaptation:** back the helper with `tempfile::tempdir()` for random names and RAII cleanup;
  keep `join`/path helpers.
- **Tests:** no test pins the exact name format; content assertions stay.
- **Assumptions:** Windows cleanup can still fail if a test leaves a file open, as today.

### LEAF-10 — Documentation drift in `girpr-log` (not a library finding)
- **Location:** `crates/log/src/filter.rs:131-137`, `crates/log/src/plan.rs:57-88`.
- **Code/docs:** comments still describe the old `"{rust_log},{project}={level}"` joined-spec
  implementation, while the code (`filter.rs:45-65`, `plan.rs:239-247`) now appends `Directive`s.
- **Adaptation:** update the comments; no crate applies.

---

## Findings — `tools/sync*`

### SYNC-1 — `AlgoSet` hand-written bitfield
- **Location:** `tools/sync/cache/src/hash/set.rs:7-96`; `cache/src/hash/algo.rs:103-113` (`bit()`).
- **Code:** `struct AlgoSet(u16)` with hand-rolled masks, `count_ones`, canonical-order iteration and
  set difference, one value per path in `planner/plan.rs:30-32,110-118` and `planner/pairs.rs:271-355`.
- **Library:** `bitflags 2.13.2` (already locked, rust-lang maintained, 2.37B downloads).
- **Adaptation:** use `bitflags! { impl AlgoSet: u16 { … } }` so the public type and two-byte layout
  stay; keep custom `iter()` for `HashAlg::ALL` order and custom `Debug` for log spelling.
- **Tests:** keep `cache/src/hash/tests.rs:246-333` and the `size_of == 2` pin at `:916-919`;
  `:926-935` pins the hand-written `Debug`.
- **Assumptions:** preserving the two-byte size and `["md5", …]` debug spelling matters.

### SYNC-2 — `HashAlg` name/parse/format plumbing
- **Location:** `tools/sync/cache/src/hash/algo.rs:25-47` (`as_str`, `ALL`), `:75-86`
  (`const from_name`), `:115-119` (`names()`), `:155-198` (const self-test), `:211-269`
  (`Debug`, `Display`, `FromStr`, `UnknownHash`).
- **Library:** `strum 0.28.0` with `derive` — `EnumIter`, `EnumString`, `Display`, `AsRefStr`.
- **Adaptation:** keep `digest_len()`/`bit()` local; keep `UnknownHash` by mapping `strum::ParseError`;
  keep manual `Display` if the `{:<12}` padding (`algo.rs:234-245`, pinned by
  `planner/coverage.rs:110-112` and `tests.rs:324-344`) must survive. The const `from_name` and its
  compile-time assertion block cannot be generated; keep or delete deliberately.
- **Assumptions:** strum derives do not touch serde, so `postcard` wire bytes for `DigestSet` stay
  identical.

### SYNC-3 — mtime conversion and stamping
- **Location:** `tools/sync/src/util.rs:516-523` (`mtime_ns_of`), `src/commands/sync/apply/copy.rs:256-278`
  (`set_modified`) and `:91-98` (`sync_all`), `src/scan.rs:206-216`,
  `test-support/src/lib.rs:402-412` (`age()`).
- **Library:** `filetime 0.2.30` (482M downloads, MSRV 1.75, cross-platform).
- **Adaptation:** `FileTime::from_system_time(...).unix_seconds()/nanoseconds()` for the i64 cache
  value, `set_file_handle_times`/`set_file_mtime` for the stamp, `FileTime::from_unix_time` for
  `age()`. Keep `mtime_ns_of` as the single shim; handle pre-epoch explicitly (current code returns 0).
- **Tests:** `src/scan.rs:401-425`; mtime-preserving copy tests at `tests/sync.rs:1213-1240`;
  `tests/compare.rs:67-80`, `tests/why/flags.rs:228`.
- **Assumptions:** exact ns remain the cache contract; no new rounding beyond std's.

### SYNC-4 — Case-fix temporary name claimed by a hand-written loop
- **Location:** `tools/sync/src/commands/sync/rename.rs:193-259` (`claim_case_tmp`), called at `:448`.
- **Code:** probes and claims `.girsync_case_tmp.<n>` up to 1000 times with separate file/dir paths
  and re-stat race handling.
- **Library:** `tempfile 3.27.0` (already locked); the same workspace already uses
  `Builder::tempfile_in` in `cache/src/backup/publish.rs:690-710`.
- **Adaptation:** `Builder::new().prefix(".girsync_case_tmp.").tempfile_in(parent)` with
  `into_parts().1.keep()` for files; `tempdir_in(parent)` + `into_path()` for dirs. Keep returning
  `PathBuf` so fail_step1/fail_step2 (`rename.rs:261-299`) are unchanged.
- **Tests:** `tests/sync.rs:746-830` pins literal `_0/_1/_2` suffixes and `:931-960` pins the
  long-name case; rewrite to assert "a unique sibling in the tool namespace, pre-existing files
  untouched".
- **Assumptions:** random suffixes are acceptable; deterministic numbering is not a user contract.

### SYNC-5 — Lexical path normalization in link-target handling
- **Location:** `tools/sync/src/commands/sync/plan/link.rs:143-165` (`absolute_alias`), `:173-188`
  (`alias_target`); `leaves/src/link.rs:157-172`; `src/commands/sync/plan/safety.rs:728-756`.
- **Code:** push/pop `Component::Normal`/`ParentDir` on `Vec<&str>` to clean junction/symlink target
  text.
- **Library:** `path-clean 1.0.1` (dependency-free, 42M downloads) or `pathdiff 0.2.3`.
- **Adaptation:** apply `Path::clean()` after stripping the canonical prefix / building the candidate;
  keep the refusal of paths escaping the root and the `try_is_link`/`on_disk_name` decisions.
- **Tests:** behavior is pinned through `tests/sync_plan/table.rs`, `tests/sync_plan/plan_print.rs`,
  `tests/why/table.rs`; add direct `..`-escape cases.
- **Assumptions:** lexical (not filesystem) normalization is still wanted.

### SYNC-6 — NDJSON import parses lines by hand
- **Location:** `tools/sync/cache-json/src/convert.rs:89-188`; `cache-json/src/ndjson.rs:134-148`.
- **Code:** `lines()` + `line_no += 1` + `parse_line` with string-inspection classification of blank
  and malformed lines.
- **Library:** `serde_json 1.0.154` (already used) — `Deserializer::from_reader(...).into_iter::<Line>()`.
  (The initially suggested `json-line` does not exist on crates.io; see A11.)
- **Adaptation:** fold over the stream while keeping meta-first and duplicate-path rules; the
  deserializer reports `byte_offset()`, not a line number, so either drop "line N" wording or count
  `\n` bytes. `write_line` stays.
- **Tests:** `cache-json/tests/roundtrip.rs:143-217` pins `"line 2"`, `"line 3"`, `"blank line"`,
  `"second meta line"`, `"repeats path"` and temp cleanup.
- **Assumptions:** accepting whitespace-separated JSON objects instead of strict one-per-line is OK.

### SYNC-7 — Hex digest spelling through a project-local wrapper
- **Location:** `tools/sync/cache-json/src/ndjson.rs:94,97,121`; `cache-json/Cargo.toml:37-38`.
- **Code:** `girpr_digest::encode_hex`/`decode_hex` are one-line wrappers over `hex::encode`/`decode`
  (`crates/digest/src/lib.rs:59-72`).
- **Library:** `hex 0.4.3` (already in the lockfile).
- **Adaptation:** swap the three calls and drop `girpr-digest` from this crate's manifest.
- **Tests:** `cache-json/tests/roundtrip.rs:183-187` pins bad-hex error containing `line 2`; the
  underlying `FromHexError` type is the same.
- **Assumptions:** shrinking this one dependency is desirable; no behavior change.

### SYNC-8 — Hand-rolled benchmark harness
- **Location:** `tools/sync/Cargo.toml:27-33` (`harness = false`, "no criterion, no new dependency");
  `tools/sync/benches/tree_shapes.rs:88-195`.
- **Code:** `Instant`, `println!`, manual rate arithmetic and fixture-build timing; no warm-up,
  statistics or regression comparison.
- **Library:** `criterion 0.8.2` (crates.io current; Windows/Linux; statistics-driven).
- **Adaptation:** `criterion_group!/criterion_main!`, three shapes as `BenchmarkId`s,
  `iter_batched` to separate fixture construction from the measured work; remove `harness = false`.
- **Tests:** none pin benchmark output; only the bench declaration changes.
- **Assumptions:** adding a bench/dev dependency is acceptable now that the original reason is
  revisited; throughput estimates replace the printed `files_per_s`/`mib_per_s`.

### SYNC-9 — Walk + include/exclude split across `walkdir` and a memo — low confidence
- **Location:** `tools/sync/src/scan.rs:115-251`; `tools/sync/leaves/src/filter.rs:46-188`
  (`Filter`, `ExcludeMemo`); `Cargo.toml:51,65`.
- **Code:** a bespoke per-directory memo implements "excluded directory covers subtree" and
  include-on-path-only semantics on top of a raw `walkdir` iterator.
- **Library:** `ignore 0.4.35` (ripgrep, 223M downloads, active) — `WalkBuilder` with
  `OverrideBuilder`.
- **Adaptation:** disable hidden/gitignore/parent handling, translate include/exclude to overrides,
  keep cache pruning, `link::try_is_link` classification and heartbeat as post-filters;
  `ExcludeMemo` and `girpr_filter::any_matches_or_ancestor` become removable from this consumer.
- **Tests:** `leaves/src/filter.rs:190-429`, `scan.rs:301-399` and filter-driven integration suites
  need re-baselining; `ignore` uses globset semantics, not `glob::Pattern`.
- **Assumptions:** medium/low confidence — adopt only after a behavior-diff harness confirms the new
  matcher agrees with the pinned tables.

---

## Findings — `tools/capture*`

### CAP-1 — Hand-rolled HTTP/1.1 mock server — test only
- **Location:** `tools/capture/test-support/src/lib.rs:11-18` (rationale), `:1235-1266` (`Mock::start`),
  `:1888-1988` (`serve`), `:1992-1994` (`is_blob_route`), `:1996-2020` (flaky-failure table),
  `:2040-2095` (`respond`), `:2132-2153` (`write_response`).
- **Code:** ~270 lines of raw listener and per-connection thread: request-line/header parsing,
  keep-alive, literal path routing for four HYP endpoints, `getBuild` and two CDN blob families,
  query extraction, an armed 503 "fail once" table, a hand-built `NoSuchKey` XML body and response
  serialization.
- **Library:** `axum 0.8.9` (`Router` + `axum::serve` + `State<Table>`); `wiremock 0.6.5` /
  `httpmock 0.9.0` if only canned replies are needed.
- **Adaptation:** keep `Table`, `build_table`, `Recorded`, `Mock::in_memory`/`transport` unchanged;
  replace `start`/`serve`/`write_response` with routes and a state lock for `flaky`. Path guards
  become extractors. Recorded `resp_headers` in `api/requests.ndjson` will change spelling.
- **Tests:** request-log rows in `tests/capture_layout.rs` (~1127, ~1194-1201), plus
  `tests/measured_fixture.rs`, `tests/offline_bundle.rs`, `tests/matrix.rs`,
  `tests/recorded_json_golden.rs`, `tests/schema.rs:558` need regeneration, not deletion.
- **Assumptions:** the in-memory `HttpTransport` seam survives; the deliberately malformed raw-socket
  helper in `http/src/tests.rs:114+` stays raw.

### CAP-2 — Bespoke ordered-JSON value and string escaper — test only
- **Location:** `tools/capture/test-support/src/lib.rs:1511-1523` (`J`), `:1536-1572`
  (`render`/`write`), `:1580-1594` (`write_string`), `:1596-1608` (`envelope`/`game_ref`),
  `:1613-1616` (`compact`), `:1619-1849` (body construction).
- **Code:** a JSON tree built solely to preserve insertion order (serde_json's `json!` sorts keys)
  with a hand-written string escaper; the comment says "Forty lines is the right price" but it is
  ~100 lines.
- **Library:** `serde_json 1.0.154` with a local `Serialize` impl over `Vec<(String, J)>` calling
  `serialize_map` in order — **without** `preserve_order` (see A7).
- **Adaptation:** render with `serde_json::to_string`; delete `write_string`; keep `compact()`
  validity checking or drop it.
- **Tests:** `tests/schema.rs:558 the_api_bodies_are_byte_exact_and_the_generated_one_is_marked`,
  `tests/recorded_json_golden.rs`, and capture-layout pins on stored `api/*` bytes.
- **Assumptions:** fixture strings use only escapes serde_json emits identically.

### CAP-3 — Hand-rolled query-string building — production
- **Location:** `tools/capture/src/biz/mod.rs:195-228` (`hyp_url`/`get_build_url`), `:318-324`
  (`push_pair`), `:326-351` (`enc`); `tools/capture/src/http/probe.rs:577-611` (`with_param`).
- **Code:** two local URL grammars; `push_pair` chooses `&`/`?` by `contains('?')`, `enc` performs
  RFC3986 escaping, `with_param` splits on `?`/`&` and rewrites literal keys.
- **Library:** `url 2.5.9` / `form_urlencoded 1.2.3` (both already in the lockfile).
- **Adaptation:** use `query_pairs_mut()` / `form_urlencoded::Serializer`; either keep literal
  `game_ids[]` by appending the raw key plus an encoded value, or accept `%5B%5D` and regenerate
  pins.
- **Tests:** `src/http/tests.rs:9-30`, `src/http/probe.rs:689-735`, `src/biz/mod.rs` URL tests,
  `tests/capture_layout.rs` and `tests/recorded_json_golden.rs` pins containing request URLs.
- **Assumptions:** byte-stable URL spelling is load-bearing only for diffs; regeneration is allowed.

### CAP-4 — Hand-rolled query parsing / percent-decoding in the mock — test only
- **Location:** `tools/capture/test-support/src/lib.rs:1915-1923`, `:2097-2106` (`query_param`),
  `:2108-2130` (`percent_decode`), `:2040-2077` (route reads).
- **Code:** another form-urlencoded parser: splits on `&`/`=`, handles `%XX` and `+`, lossy UTF-8.
- **Library:** `form_urlencoded 1.2.3` (already locked) or `Url::query_pairs()`.
- **Adaptation:** parse once per request into a map and look up the three keys; delete the two local
  helpers. Raw `Recorded.query` logging stays unchanged.
- **Tests:** negative-probe cases in `tests/matrix.rs` and `tests/offline_bundle.rs`.
- **Assumptions:** lossy UTF-8 behavior matches.

### CAP-5 — Hand-rolled FNV hash and recursive source/bundle discovery — test only
- **Location:** `tools/capture/test-support/src/lib.rs:207-214` (`fnv1a`), `:216-229`
  (`collect_rs_files`), `:231-258` (`capture_source_key`), `:271-287` (`bundle_source_key`),
  `:401-419` (`find_bundle`).
- **Code:** a six-line FNV-1a, hand-written recursive `read_dir` walker and stack walk with sorting.
- **Libraries:** `fnv 1.0.7` and `walkdir 2.5.1` (both already in the lockfile).
- **Adaptation:** `FnvHasher` over path/file bytes; `WalkDir` with `.rs`/`MANIFEST.json` filters and
  explicit `sort_by_file_name` for stable fingerprints.
- **Tests:** no test pins the numeric key; bumping `FORMAT` (`:271-274`) invalidates the shared
  bundle cache once. `capture_layout.rs`, `offline_bundle.rs`, `schema.rs` read `shared_bundle()` but
  do not pin the key string.
- **Assumptions:** fnv's FNV-1a constants match the local implementation; sorting is still required.

### CAP-6 — Hand-rolled bounded ordered-join and panic-payload extraction
- **Location:** `tools/capture/http/src/tasks.rs:14-33` (`join_failure`), `:35-50` (`JoinOutcome`),
  `:69-136` (`join_ordered`), `:138-154` (`join_blocking`); used by `http/src/fetcher.rs:449-509`.
- **Code:** `JoinSet` + `HashMap<task::Id, slot>` reimplements bounded concurrency with results in
  input order, plus manual `&str`/`String` panic-payload downcast.
- **Library:** `futures 0.3.34` — `stream::iter(...).map(...).buffer_unordered(jobs)` plus ordered
  collection (partial delegation: panic recovery still needs `JoinSet` or `catch_unwind`).
- **Adaptation:** keep `JoinOutcome`/`join_failure` as the error vocabulary; add `futures` to
  `http/Cargo.toml`.
- **Tests:** `http/src/tasks.rs:160-236` (four tests) must be kept; check cancellation semantics.
- **Assumptions:** `reqwest-retry 0.9.1` / `backoff` should **not** replace `http/src/fetcher.rs:788-861`'s
  retry loop — it would lose the per-attempt log, typed `FetchFailure.seq` and panic-free sinks.

### CAP-7 — Two hand-rolled TSV writers
- **Location:** `tools/capture/src/manifest/dump.rs:209-243` (`paths_tsv`);
  `src/manifest/diff.rs:656-713` (second `paths_tsv`, whose doc at `:670-673` admits "neither escapes
  a tab").
- **Code:** manual header constant, `write!` with `\t`, manual `push('\n')`, per-row `to_string()`.
- **Library:** `csv 1.4.0` (BurntSushi, 303M downloads, MSRV 1.73) with
  `WriterBuilder::delimiter(b'\t').has_headers(true)`.
- **Adaptation:** define row structs and serialize through `csv::Writer`; keep the existing sort and
  tie-break logic.
- **Tests:** `src/manifest/dump.rs:553-691` and `src/manifest/diff.rs:1499-1533` pin header, LF-only
  newlines, row content and column count; quoting of tab-containing fields is a beneficial observable
  change.
- **Assumptions:** no current fixture path contains a tab.

### CAP-8 — Hand-rolled Markdown table in generated README — cosmetic
- **Location:** `tools/capture/src/emit/index.rs:2058-2157` (`Manifest::readme`), especially
  `:2094-2118`.
- **Code:** `push_str("| … |")` plus `format!` per cell for the matrix table.
- **Library:** `tabled 0.22.0` with `Style::markdown()`, or `comfy-table 8.0.1`.
- **Adaptation:** build a table from matrix rows with explicit labels; keep values quoted from
  `self`/`selfcheck`.
- **Tests:** `tests/capture_layout.rs:996-1030` pins `README_SKELETON` line by line with digits
  blanked; regenerate on padding changes. `tests/bundle_diff.rs` blanks prose totals.
- **Assumptions:** low-priority cosmetic change.

### CAP-9 — Hand-rolled semver-ish "legacy staleness" parse
- **Location:** `tools/capture/src/hyp/types.rs:434-457` (`GamePackages::staleness`), `:416-432`.
- **Code:** splits tags on `.`, parses the leading component with `u64::from_str`, `checked_sub`;
  comments deliberately avoid fabricating a middle version.
- **Library:** `semver 1.0.28` (dtolnay, 1.26B downloads, already in the lockfile).
- **Adaptation:** `Version::parse(...)` on both tags and compare `.major`; keep `checked_sub` and the
  `(u64, Vec<String>)` return.
- **Tests:** `src/hyp/mod.rs:897-898`, `src/hyp/types.rs:1106-1136`.
- **Assumptions:** fixture tags are strict `x.y.z`; refusing `7` or prerelease suffixes is an
  acceptable behavior change — verify against fixtures first.

### CAP-10 — Enum spelling via serde string round-trip — minor
- **Location:** `tools/capture/src/run/mod.rs:957-963` (`selfcheck_slug`).
- **Code:** `serde_json::to_string` then `trim_matches('"')` to obtain an enum's lowercase wire spelling.
- **Library:** none required; `strum 0.28.0`'s `IntoStaticStr`, or a local `const fn as_str()` matching
  the pattern already used by `State`, `Kind`, `Class`, `Encoding`.
- **Tests:** summary pins asserting `result=ok` (`tests/matrix.rs`, `tests/cli_contract.rs`) should not
  change.
- **Assumptions:** purely cosmetic.

### CAP-11 — Python bundle-differ — optional, not a dependency swap
- **Location:** `tools/capture/tools/bundle_differ.py:28-53` (regex scrub), `:75-149` (line-based
  blanking), `:157-229` (walk + fingerprint + set diff).
- **Code:** bespoke cross-commit normalizer blanking volatile fields by scanning text lines.
- **Libraries if ported to Rust:** `regex`, `serde_json`, `walkdir 2.5.1`, optionally `similar 3.x`.
- **Adaptation:** only viable as a deliberate Python→Rust migration with structural `serde_json::Value`
  mutation; keep `DERIVED` exemptions and `--strict-exemptions`.
- **Tests:** `tests/bundle_diff.rs` pins exempt rows and blanking behavior.
- **Assumptions:** optional; no Rust crate can be adopted into a Python tool without a rewrite.

---

## Areas examined without a credible candidate

These are **not findings**; they are recorded so implementation does not re-litigate them.

- **`crates/exit/src/lib.rs:27-75`** — the `define_exit_table!` macro's const `ALL` array and
  compile-time "codes contiguous from zero" pin are not preserved by `strum`/`num_enum`.
- **`crates/fs-names/src/lib.rs:41-124`** — policy-parameterized device-name predicate and ASCII
  fold; `dunce 1.0.5` is already correctly delegated at `:37-39`.
- **`crates/fs-identity/src/lib.rs:26-182`** — already delegates to `same-file` and `winapi-util`.
- **`crates/digest/src/**`** — hex/MD5/SHA-256/XXH64 already delegated to `hex`, `md-5`, `sha2`,
  `xxhash-rust`; the newtypes are thin adapters.
- **`crates/report-core/src/lib.rs:58-111`** — ordered emission via `serde::Serializer::serialize_map`
  is deliberate; do not enable `serde_json/preserve_order` (A7).
- **`crates/time`, `crates/cli`, `crates/value-ranges`, `crates/chunk-cost`, `crates/chunk-identity`,
  `crates/audio-language`, `crates/tempdir`** — small pure-fact crates already on the right crates.
- **`app/report/src/ledger.rs:98-328`** — saturating `AtomicU64` with all-or-nothing ceiling
  reservation; `compare_exchange_weak` is the right tool.
- **`app/util-hash/src/md5.rs:161-308`** — one-pass multi-range MD5 with interned states; no library
  hashes arbitrary overlapping byte ranges in one file read.
- **`app/repair-plan/src/plan.rs:62-136,182-200,281-286`** — chunk layout algebra and a non-persisted
  in-process fingerprint; no library benefit.
- **`app/fetch/src/retry.rs:106-181`** — retry driver with fatal/transient classification and
  cross-attempt state; `backoff 0.4.0` already owns schedule math via `girpr-retry`.
- **`app/util-path/src/path.rs:491-533`** (`resolve_existing_prefix`) — canonicalize-longest-existing-
  prefix with fail-closed errors; `dunce`/`path-absolutize` do not preserve the safety property.
- **`app/util-path/src/path.rs` junction/symlink error classifiers** — OS-error → skip/fail policy is
  bespoke test infrastructure.
- **`app/dispatch/src/dispatch.rs:104-135`, `metadata.rs:296-321`** — permissive version spellings such
  as `7.2_live` are rejected by `semver`.
- **`src/repair/file/**`, `src/repair/scan.rs`, `src/repair/phases/post.rs`, `persistent*.rs`** — the
  remaining logic is correctness-critical policy and bespoke in-place byte movement; path, digest,
  hex, INI, glob, atomic-write, retry and range mechanics are already in `girpr-*` leaves.
- **`tools/sync/cache/src/open.rs` (2713 lines)** — backup/snapshot publish protocol, hold records,
  process-wide registries and retention rules; no library expresses the hard-link publish + claim
  identity check. `File::lock` is stable (A9).
- **`tools/sync/cache/src/db/*`, `write.rs`, `backend.rs`** — already delegated to `redb 5.0.0` and
  `postcard 1.2.0`.
- **`tools/sync/cache/src/hash/digestset.rs`** — `SmallVec` would *increase* `size_of::<DigestSet>()`
  by 32 bytes per row; the pinned 88-byte zero-allocation design must stay.
- **`tools/sync/src/util.rs:63-124`, `cache/src/helpers.rs:52-74`** — ASCII-case helpers deliberately
  independent of `std`.
- **`tools/sync/src/report/record.rs:196-282`, `planner/coverage.rs:78-190`** — stdout is NDJSON via
  `girpr-report-core`; there is no hand-rolled table to replace.
- **`tools/sync/leaves/src/rw/*`** — already uses `tokio::sync::Semaphore` and `rayon`; the drop-joining
  `spawn_handler` is a targeted Windows TLS-deadlock fix.
- **`tools/sync/leaves/src/link.rs:35-146`** — "only redirecting reparse points" has no library
  equivalent; std calls are the maintained primitive.
- **`tools/capture/hash/src/lib.rs:38-658`** — MD5+SHA-256 one-pass digest, gzip rule and bounded
  zstd decode already delegate to `md-5`, `flate2`, `zstd`, `girpr-digest`.
- **`tools/capture/src/manifest/diff.rs:282-586`**, **`src/assemble/ranges.rs:27-241`**,
  **`src/assemble/assembly.rs`** — domain scoring and interval algebra where violations are data,
  not errors; general interval crates would purge the findings.
- **`tools/capture/src/emit/index.rs:286-393`** — `WalkDir` already used; symlink containment is
  policy. `canonical_under_root` is duplicated with `src/verify/list.rs:233-236` — consolidate
  locally, no crate needed.
- **`tools/capture/http/src/error_bodies.rs:113-263`**, **`http/src/fetcher.rs:121-344`** — byte-budget
  accounting and status-vs-body fault split are bespoke policy.
- **`tools/capture/src/paths.rs`, `pathmap.rs`, `encoding.rs`** — already delegate to
  `girpr-path-shape`, `girpr-fs-names`, `percent-encoding`.
- **`tests/scripts/io-profile/**`** — standalone Windows-only profiling scripts, outside the product
  build; no cross-platform crate migration applies.
