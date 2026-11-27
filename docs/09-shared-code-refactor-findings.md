# Shared-code refactor research — assumptions and findings

Research date: 2026-11-27. Source state: `main` at `9ff548f6`. Scope: every cargo
workspace member (root `girpr`, `app/*`, `crates/*`, `tools/sync*`, `tools/capture*`).
Method: read-only source survey divided by area, with `file:line` evidence for every
claim. Nothing was changed and no implementation was attempted.

This document contains **only assumptions and findings**. It is not an implementation
plan: it does not order the work, group findings into tasks, or recommend a sequence.

---

## Assumptions

- **A1 — No validation by compilation.** Every claim is a static reading of the tree
  at the commit above. A finding may be invalidated by a closer read of a call site,
  by a feature gate, or by a coherence/type detail that only a build would reveal.
- **A2 — Scope boundary.** The git submodules under `reference-projects/` (C# sources)
  and the standalone Windows profiling crates in `tests/scripts/io-profile/**` are
  outside the workspace build and are not surveyed. The Python/PowerShell instruments
  under `tools/capture/tools/` are surveyed only where they participate in a Rust
  contract (see CAP-20, which is deliberately not a candidate).
- **A3 — Project-local `girpr-*` leaves are deliberate.** Many mechanics are already
  extracted into narrow, policy-bearing leaves. Those are generally *not* candidates
  for re-extraction; where an existing leaf is named, the finding is that its surface
  could be widened or that a duplicate could be moved into it.
- **A4 — Policy at call sites, mechanics in leaves.** This is the established house
  style (see `crates/report-core/src/lib.rs:1-22`, `crates/log/src/install.rs:1-14`,
  `crates/value-ranges/src/lib.rs:1-70`). Findings propose moving mechanics or values;
  error wording, flag names, exit codes, field order and program policy are treated as
  call-site concerns unless a finding says otherwise.
- **A5 — Byte contracts are policy.** Published bundle documents, `api/requests.ndjson`
  rows, stdout record order, exit codes, serde/protobuf key names and golden snapshots
  are program contracts (README.md:141-203; REPAIR_FLOW.md). A generalization that
  would change them must parameterize the differing spelling rather than converge it.
  Where two programs already differ, the finding records the difference explicitly.
- **A6 — "Deliberately local" comments are evidence of intent, not proof of no
  duplication.** Several sites explain why two copies exist (e.g.
  `app/fetch/src/lib.rs:26-28`, `src/test_http.rs:7-11`, `crates/retry/src/lib.rs:4-8`,
  `tools/capture/hash/src/lib.rs:563-580`). Such duplication is still reported, with
  the stated intent recorded as a caveat.
- **A7 — Test-only code counts.** Helpers behind `cfg(test)`, a `test-support` feature,
  or an integration-test root are included when the same mechanic is compiled in more
  than one crate. Blast radius for those findings is confined to tests, but the test
  compilation model matters: `Cargo.toml:8-38` declares four roots instead of
  autotests, nextest runs one process per test (`tests/common/mod.rs:1-11`), and
  `CARGO_BIN_EXE_girpr` is bound only into integration targets.
- **A8 — No MSRV constraint.** The workspace targets stable Rust (edition 2024,
  `rust-toolchain.toml`); no `rust-version` floor is set.
- **A9 — Prior refactor debt is out of scope.** A previous pass already delegated small
  mechanics to third-party crates (`fnv`, `bitflags`, `strum`, `globset`, `axum`,
  `wiremock`, `trycmd`, `url`, `form_urlencoded`, `strum`, `filetime`, `junction`,
  `redb`, `postcard`) and extracted the leaves named in A3. Those delegations are not
  re-proposed here; only remaining *in-workspace* duplication is reported.
- **A10 — `serde_json/preserve_order` must not be enabled.** Where insertion order is
  required, findings assume a local ordered representation (as in
  `crates/report-core/src/lib.rs`); enabling the feature globally would change
  unrelated consumers including `app/report`.
- **A11 — Feature-unification and dependency-graph risks are inferred, not measured.**
  Several findings add a leaf edge to a consumer that currently avoids it (e.g.
  `girpr-tempdir` is girsync-free, `app/util-path` is a product leaf). Whether an edge
  creates a cycle or widens a shipped build must be checked at implementation time.
- **A12 — "High-risk generalization" is read as permission to widen or merge leaf
  surfaces and to move mechanics across program boundaries**, not to change published
  bytes by accident. Each finding therefore records its own blast radius and the
  policy that must remain at its call sites.

---

## Cross-cutting findings

### X1 — The five global CLI flags and `OutputFormat` are declared three times
The flags `--log-level`, `--log-file-level`, `--log-file`, `--no-log-file` and
`--output` appear nearly identically in `tools/sync/src/cli.rs:20-69`,
`tools/capture/src/cli/mod.rs:64-120` and `src/cli.rs:233-282`, with the same clap
attributes, parser wiring and help text shape. `OutputFormat` (enum + `as_str` +
`Display`, with capture adding `FromStr`) is defined in `tools/sync/src/report/mod.rs:57-85`,
`app/report/src/record.rs:31-67` and `tools/capture/src/report/mod.rs:36-67`. The
existing `girpr-cli` owns only clap-error mapping (`crates/cli/src/lib.rs:11-21`) and
`girpr-report-core` only the ordered writer (`crates/report-core/src/lib.rs:28-111`).
The duplication is consistent with a flattenable global-args struct in `girpr-cli`
(parameterized by project name and `global`) and a shared `OutputFormat` behind an
optional `clap` feature in `girpr-report-core`. Differences to preserve: the product's
flags are not `global`, each program's `value_name`/default spelling differs, and
capture parses the format by hand (SYNC-01, SYNC-02, CAP-11, CAP-12). Blast radius:
`--help` bytes and exit classification in all three binaries.

### X2 — The tracing subscriber stack is assembled three times
The same sequence — assemble `LogOptions`, resolve `LogPlan`, emit env warnings,
console `fmt` layer + filter, degrade when the file cannot be opened, build a
non-blocking file sink, choose JSON vs text layer, `registry().with(..).try_init()` —
is written in `tools/sync/src/logging.rs:50-212`, `tools/capture/src/logging/mod.rs:56-214`
and `src/logging.rs:77-221`. `crates/log/src/install.rs:1-14` already owns the
pre-layer mechanics (open file, `file_sink`, env warnings, announcement macro) and
explicitly leaves layer assembly to each program. The duplicated remainder is
consistent with a parameterized `subscriber_for(plan, console_writer, format,
span_fields)` in `girpr-log::install`; guard ownership must stay per program (girpr's
process-global slot + `flush_log`, the tools' `LogGuard` drop/finish) and `PROJECT`,
warning prefixes, targets and file format must stay caller parameters (SYNC-04,
CAP-10). Blast radius: every log-capture assertion in the workspace.

### X3 — Three report planes sit on one thin core
`Record`/value objects with ordered fields, text and NDJSON rendering, and a
"render → mirror to log → write one line" emitter exist in
`tools/sync/src/report/record.rs:22-282` + `tools/sync/src/report/mod.rs:167-189`,
`app/report/src/record.rs:79-234` + `app/report/src/emit.rs:40-54`, and
`tools/capture/src/report/mod.rs:69-371,715-736`. The shared core
(`crates/report-core/src/lib.rs:28-111`) owns only `Order`, `write_json_object` and
`write_line`. A generic field-list/`Record` mechanic plus a sink in that core is the
candidate boundary; the value vocabularies are deliberately incompatible (girpr
forbids booleans and renders flags as `1`/`0`, capture renders `true`, girsync carries
`serde_json::Value` and positional layouts), `Order::Alphabetical` is girsync's
contract, and stdout locking/broken-pipe policies differ (SYNC-03, SYNC-10, CAP-12).
Blast radius: the stdout byte contract for all three binaries.

### X4 — The retryable-status rule is duplicated with an explicit "keep in sync" note
`app/fetch/src/sophon/stream.rs:745-765` (`retryable`) and
`tools/capture/http/src/transport.rs:52-59` (`RetryPolicy::should_retry_status`)
classify `408`, `429` and `5xx` the same way; the fetch doc at `:760-761` states the
two rules must not disagree. The two spellings already differ on out-of-range `u16`
values (`StatusCode::from_u16` rejects, the numeric range accepts). A logic-free
`should_retry_status(u16)` in `crates/retry` is the candidate boundary; jitter,
attempt counts, classification of HYP `retcode` and per-attempt logging stay local
(APP-03, CAP-03). Blast radius: retry counts are published in both tools.

### X5 — Bounded concurrent join and panic-payload recovery are reimplemented
The indexed `buffer_unordered(jobs.max(1))` window, result restoration by input slot,
and recovery of a message from a lost task appear in `tools/capture/http/src/tasks.rs:22-153`
and, with different failure policies, in `src/repair/scan.rs:304-352`,
`src/repair/phases/check.rs:109-175`, `src/repair/persistent_exec.rs:72-86`,
`src/repair/file/inplace.rs:699-710` and `src/repair/file/mod.rs:527-608`.
`src/repair/phases/metadata.rs:126-129` drops the panic payload that capture
preserves. The same downcast also appears in `tests/cli_contract.rs:76-88` and
`app/util-path/src/junction_support.rs:148-156`. A logic-free `async-join` leaf
(indexed collection, `jobs.max(1)`, payload downcast, optional dispatcher-carrying
`spawn_blocking`) is the candidate boundary. The failure meaning is policy and
differs per caller (collect failures, fatal exit, conservative unknown, fail-fast);
capture catches panics inside buffered futures while the repair file driver uses
`Handle::block_on` on the blocking pool, so cancellation semantics must not be
collapsed (R-01, R-02, R-10, CAP-04).

### X6 — `clear_readonly` is written three times with two error policies
The same stat → `permissions().readonly()` → `set_readonly(false)` →
`set_permissions` sequence appears in `app/fetch/src/persistent.rs:425-446`,
`src/repair/persistent_exec.rs:442-465` and `src/repair/file/mode.rs:65-83`, with
only the failure handling differing (warn-and-continue in the first two, propagate
in the third). A result-returning `clear_readonly(path) -> ReadOnlyOutcome` in an
fs leaf is the candidate boundary; logging, fallbacks and fatality stay at call sites
(APP-01, R-04). Blast radius: a security-relevant mutation on real game files.

### X7 — Whole-file/byte presence → length → digest gates are parallel
`src/repair/phases/check.rs:482-532`, `src/repair/file/temp.rs:46-82`,
`app/fetch/src/persistent.rs:92-113,407-421` and the persistent-row checks at
`app/fetch/src/persistent.rs:359-365` express the same decision tree with an
injectable digest source. A pure `verify_whole_file(expected_size, expected_digest,
source) -> WholeFileVerdict` extending `app/chunk-verify` is the candidate boundary.
The scanned-digest reuse contract (`--check-only --scan`, `check.rs:7-14,44-54`),
XXH64-vs-MD5 row choice, exit codes and warnings stay local (APP-05, R-06).
Blast radius: correctness-critical verify paths.

### X8 — Saturating `i64 → u64` byte folds and range/layout algebra are scattered
The clamp-negatives-then-saturating-add rule extracted once in
`crates/chunk-cost/src/lib.rs:48-58` is re-derived in at least twenty sites, including
`tools/capture/src/manifest/diff.rs:424-651`, `tools/capture/src/manifest/mod.rs:713-758`,
`tools/capture/test-support/src/lib.rs:1130-1138`, `app/chunk-verify/src/lib.rs:69-82`,
`app/repair-plan/src/plan.rs:63,119,133`, `src/repair/scan.rs:742` and
`src/repair/scope/report.rs:154,176,305`. The same structural question — do chunks
tile `[0,size)` with no gap/overlap — is modelled three ways in
`app/repair-plan/src/plan.rs:43-151`, `tools/capture/src/assemble/ranges.rs:27-235`
and `tools/capture/src/manifest/mod.rs:722-787`. Candidate boundaries: pure
`nonneg` / `saturating_nonneg_sum` / `checked_end` primitives in `girpr-chunk-cost`,
and a logic-free half-open range + tiling fold in a new `girpr-chunk-layout` leaf.
Signed cursors must use `checked_end`, not the u64 helper; capture's A11 violation
strings and girpr's gap-fill policy stay local (LF-03, LF-10, APP-08, APP-17, CAP-05).

### X9 — The XXH64 numeric/hex boundary is owned by no leaf
`crates/digest/src/lib.rs:84-87` spells XXH64 as hex, but the numeric value is
re-derived in `app/persistent-plan/src/digest.rs:140-147`, parsed back in
`app/persistent-plan/src/manifest.rs:99-107`, read from bytes in
`app/fetch/src/persistent.rs:403` and re-spelled in `app/persistent-plan/src/digest.rs:117`.
`app/persistent-plan/Cargo.toml:31-32` depends on `hex` and `xxhash-rust` directly,
duplicating `crates/digest/Cargo.toml:18,27`. Candidate boundary: widen `girpr-digest`
with `xxhash64_u64`, `parse_xxhash64_hex`, `format_xxhash64_hex` and a numeric accessor,
and adopt the existing `encode_hex`/`decode_hex` wrappers in `girsync-cache-json`
(`tools/sync/cache-json/src/ndjson.rs:94,97,121`) and `normalize_hex` in
`crates/chunk-identity/src/lib.rs:102-114`. The exact 16-digit lowercase spelling and
the digest-wins policy stay local (APP-04, LF-04, LF-07, LF-15).

### X10 — URL assembly and query rewriting are reimplemented across fetch and capture
`tools/capture/src/biz/mod.rs:201-230,320-336` and `app/fetch/src/hyp.rs:359-383,636-663,689-691`
implement the same HYP/Sophon query grammar over the one shared `QUERY_VALUE_SET`
(`crates/model/src/query.rs:10-20`): literal `game_ids[]`, literal separator choice,
percent-encoded values, trailing-slash trim and ordered `branch, package_id, password,
tag` pairs. A third rewriter is `tools/capture/src/http/probe.rs:577-611`. CDN segment
appending is duplicated in `app/dispatch/src/dispatch.rs:172-200` and
`app/persistent-plan/src/placement.rs:136-205`. Candidate boundaries: a
`girpr-request-url` leaf for pair/encode/build/replace, and a URL-path append helper.
`Biz::resolve`, base overrides, `needs_channel`, prefix selection and girpr's
unencoded/hardcoded-language spelling stay call-site policy. Golden URLs and recorded
requests make the blast radius high (APP-07, CAP-01, CAP-06, CAP-17).

### X11 — Line-oriented readers and canonical renderings are parallel
BOM-stripping, 1-based line accounting and blank-line skipping are reimplemented in
`app/audio/src/lib.rs:133-159`, `app/util-ini/src/ini.rs:132-196`,
`app/persistent-plan/src/manifest.rs:54-74` and `app/dispatch/src/proto.rs:199-220`;
the BOM rationale in `util-ini` explicitly points at the audio reader. The same
canonical-order audio projection is written with opposite trailing-newline contracts
in `tools/capture/src/emit/gamedir.rs:113-144` and `app/audio/src/lib.rs:161-178`,
and `game_version=` is read first-match by capture
(`tools/capture/src/emit/gamedir.rs:61-111`) but last-match by girpr
(`app/util-ini/src/ini.rs:11-57`). Candidate boundaries: a line/BOM iterator in a new
leaf, and parameterized `render_scan_file` / `game_version_from_text` in the existing
audio-language and util-ini leaves. The BOM-only-file distinction, lenient-vs-strict
failure policy, section requirement and newline spelling must remain parameters
(APP-06, CAP-07, CAP-08, LF-08).

### X12 — Test-support mechanics are duplicated across every test surface
The same small helpers appear in multiple binaries and fixture crates: directory
`entry_names` snapshots (five copies: `crates/tempdir/src/test_support.rs:257-266`,
`src/repair/phases/post.rs:862-871`, `src/repair/scope/report.rs:522-532`,
`app/util-path/src/atomic.rs:178-187`, `app/util-ini/src/ini.rs:585-594`), `strip_ansi`
(`tests/cli_contract.rs:72-74` vs `crates/tempdir/src/test_support.rs:39-49`), mtime
setters, JSON/zstd fixture helpers (`tests/artifact_repair_bridge/bundle.rs:280-289`,
`tests/artifact_server.rs:388-398` vs `app/test-fixtures/src/artifact_server.rs:224-233`
and `app/test-fixtures/src/lib.rs:196-198`), argv-prefix incantations (nine copies in
`tools/sync/test-support/src/lib.rs:619-793`), binary-runner + lock-cleanup helpers
(four in the root tests), the lock-taking rule mirrored in fixtures
(`app/lock/src/lib.rs:104-111` vs `app/test-fixtures/src/run_lock.rs:113-124`), and
loopback-server scaffolds (`crates/loopback-mock`, `app/test-fixtures/src/lib.rs:909-989`,
`tools/capture/test-support/src/lib.rs:1209-1489`, `src/test_origin.rs:19-250`).
Most of these are consistent with widening existing leaves (`girpr-tempdir::test_support`,
`girpr-test-fixtures`, `girpr-cli`, `girpr-loopback-mock`) rather than new crates.
Caveats: the `girpr-cache*` filter differs between copies, `CARGO_BIN_EXE_girpr` must
be passed explicitly if a helper leaves the integration root, and the raw malformed
server must keep lying about `Content-Length` (R-07–R-13, SYNC-05, SYNC-07, SYNC-09,
CAP-15, CAP-16, APP-09–APP-11, LF-06).

### X13 — "Claim a unique sibling, then publish" is implemented four times
`tools/sync/cache-json/src/convert.rs:278-311`,
`tools/sync/cache/src/backup/publish.rs:74-117,457-502,690-710`,
`tools/sync/src/commands/sync/rename.rs:200-241` and `app/util-path/src/atomic.rs:62-130`
each create a uniquely named temp file beside the target, hold it across a write and
publish with an overwrite/no-clobber choice, cleaning up on error. A claim/publish
primitive extending `app/util-path::atomic` is the candidate boundary; prefixes, the
two-step case-rename protocol, hard-link-vs-claim choice, sidecar cleanup and error
context stay local. These are crash-safety paths and tests pin exact residue names
(SYNC-06, LF-05).

### X14 — Package metadata and dependency versions are repeated in every manifest
No `[workspace.package]` or `[workspace.dependencies]` exists
(`Cargo.toml:40-63`); all 39 members restate `version`, `edition` and `[lib]`, and
versions such as `anyhow 1.0.105`, `tokio 1.54.1`, `serde 1.0.229`, `clap 4.6.10`,
`tempfile 3.27.0`, `tracing 0.1.44`, `url 2.5.9` and `zstd 0.14.5` appear across
root, `app/*`, `crates/*`, `tools/sync*` and `tools/capture*`. The tools' smaller
lint block also appears verbatim in all nine tool manifests. The duplication is
consistent with workspace inheritance for versions/edition and shared dependencies;
the lint subset cannot be inherited without weakening the product lint table, and
exact pins (`redb = "=5.0.0"`, `bitflags = "=2.13.2"`) and narrow feature lists must
be preserved (APP-15, SYNC-13, CAP-19, LF-14).

---

## Findings — root package (`src/`, `tests/`)

### R-01 — Bounded task windows and panic-cause rendering in four phases
Locations: `src/repair/scan.rs:304-352`; `src/repair/phases/check.rs:109-175`;
`src/repair/persistent_exec.rs:72-86`; `src/repair/file/inplace.rs:699-710`;
`src/repair/file/mod.rs:527-608`; the payload-preserving version is already in
`tools/capture/http/src/tasks.rs:22-153`. The mechanic is the indexed concurrency
window plus a useful name for a lost task. The failure policy differs per phase
(`files_failed` + fail-fast, fatal `RunFailure::write`, conservative unknown, ignored).
`src/repair/phases/metadata.rs:126-129` drops the payload. Candidate boundary: the
X5 `async-join` leaf. Blast radius: repair phases plus gircap's fetcher; the scoped
tracing dispatcher in `file/mod.rs:580` must survive. See X5.

### R-02 — `spawn_blocking_traced` is duplicated inside the file-task driver
`src/repair/mod.rs:1363-1370` captures the current dispatcher and reinstalls it on a
blocking thread; `src/repair/file/mod.rs:543-582` repeats it, additionally adopting
`DiagnosticsOwner` (`src/repair/diagnostics.rs:87-102`). The "capture at spawn time"
rule is load-bearing and documented in three places (`mod.rs:1351-1357`,
`phases/metadata.rs:549-555`, `app/test-fixtures/src/lib.rs:764-769`). A shared helper
needs a thread-setup hook; which closures run blocking and how failures are
classified stay local. Blast radius: every traced blocking call in the facade.

### R-03 — Bounded byte-copy, zero-fill and piece-wise write mechanics
`src/repair/file/mod.rs:108-129,842-969` (`COPY_BUF`, `reserve_write`, `copy_within`,
`zero_fill`) and `src/repair/file/temp.rs:365-410,557-588,604-658` (`capture_slice`,
`copy_reuse_pieces`, `ChunkFileSink`) repeat the bounded-piece loop with a
write-ceiling charge. `src/repair/file/inplace.rs:571-605` consumes both. Candidate
boundary: an `app/util-io` leaf with `copy_bounded`/`zero_bounded`/`capture_bounded`
and a caller-supplied charge callback. Caveat: `copy_within` reads each piece before
writing it so an overlapping source survives; `capture_slice` uses a bounded `Take`
to avoid zero-filling spare capacity. Blast radius: both correctness-critical writers.

### R-04 — `clear_readonly` with two error policies
`src/repair/file/mode.rs:65-83` propagates errors; `src/repair/persistent_exec.rs:445-465`
and `app/fetch/src/persistent.rs:425-446` warn and continue. See X6 / APP-01.

### R-05 — `io::Result<Metadata>` classification written four ways
`app/util-path/src/path.rs:22-45` (`FileLen`/`classify_len`),
`src/repair/file/inplace.rs:951-979` (`InplaceLen`, whose doc at `:960-962` says it
matches the other two), `src/repair/phases/post.rs:527-552` (`StaleStat`), and an
inline three-arm classification in `src/repair/scope/report.rs:115-119`. Candidate
boundary: a generalized `classify_metadata(io::Result<Metadata>) -> MetadataFact` in
`app/util-path`, with `FileLen` and `is_file` as thin views. Caveats: `scope/report`
uses `symlink_metadata` while the others follow links; `Unreadable` must not be
counted as `Absent`, and `Failed` must not collapse into `NotAFile`. Blast radius:
presence/size decisions in check, temp, post and scope.

### R-06 — Whole-file presence/length/digest gate
`src/repair/phases/check.rs:482-532` and `src/repair/file/temp.rs:46-82` are the same
decision tree with different next actions; `app/fetch/src/persistent.rs:92-113,407-421`
and `check.rs:299-366` apply it to persistent rows. See X7 / APP-05.

### R-07 — Byte-identical test fixture helpers
`entry_names` in five locations (listed in X12), `strip_ansi` in
`tests/cli_contract.rs:72-74` and `crates/tempdir/src/test_support.rs:39-49`, and mtime
setters in `src/repair/file/inplace.rs:1853-1860` and
`tests/repair_machinery.rs:587-594`. Candidate boundary: widen
`crates/tempdir::test_support` with a predicate-parameterized `entry_names` and
`set_mtime`, and re-export the one `strip_ansi`. Caveat: the tempdir copy filters
`girpr-cache*` while the other four do not.

### R-08 — Bundle fixture I/O helpers duplicated with `app/test-fixtures`
`tests/artifact_repair_bridge/bundle.rs:280-289` and
`app/test-fixtures/src/artifact_server.rs:224-233` are byte-identical for `read_json`
and `str_field`; `tests/artifact_server.rs:388-394` and
`tests/artifact_repair_bridge.rs:1022-1028` are byte-identical for `write_json`;
`tests/artifact_server.rs:396-398` matches `app/test-fixtures/src/lib.rs:196-198` for
`zstd_bytes`. Candidate boundary: expose these on `girpr-test-fixtures` and
re-export through `tests/common/mod.rs:17-42`; keep bundle schema parsing and field
requirements local. Blast radius: test-only.

### R-09 — Flat JSON key-order scanner duplicated
`tests/cli_contract.rs:1543-1622` and `tools/capture/src/report/tests.rs:58-120` both
read top-level keys in byte order because `serde_json::Map` is a `BTreeMap`. The root
version handles nested values and escapes; capture's is a simpler char scanner.
Candidate boundary: a feature-gated `test_support` surface on `girpr-report-core`
exposing `json_keys_in_order`, using the root implementation. Both comments reject
`preserve_order` (A10).

### R-10 — Panic-payload downcast in three crates, dropped in a fourth
`tests/cli_contract.rs:76-88`, `tools/capture/http/src/tasks.rs:22-29` and
`app/util-path/src/junction_support.rs:148-156` downcast `String`/`&str` with
different fallbacks; `src/repair/phases/metadata.rs:125-129` renders a `JoinError`
without the payload. Candidate boundary: the X5 leaf with parameterized fallback and
task label. The fallback strings are asserted in tests.

### R-11 — Full 24-field `RunCtx` literals repeated four times
`app/test-support/src/lib.rs:50-84`, `src/repair/phases/metadata.rs:504-533`,
`src/repair/phases/post.rs:1589-1616` and `src/repair/mod.rs:1927-1952` repeat the
default context; the production constructor is `src/cli.rs:327-370`. A documented
`ctx_default` builder in the existing `girpr::test_support` seam (`src/lib.rs:145-166`)
is the candidate boundary. `Args::into_ctx` validation and its warning/exit ordering
(`src/cli.rs:302-323`) must not move into the builder.

### R-12 — Route-based loopback origin parallels the canned server
`src/test_origin.rs:19-250` implements reply variants, in-flight gating, a bind/driver,
a stop signal and `Drop`; `crates/loopback-mock/src/lib.rs:28-220` owns the scripted
server, and `app/test-fixtures/src/lib.rs:909-973` and
`tools/capture/test-support/src/lib.rs:1254-1320` repeat the bind/spawn/journal
scaffold. Candidate boundary: a route-driven `RouteServer` in `girpr-loopback-mock`
(or a sibling leaf if axum should not enter the wiremock-based leaf), sharing only
socket/stop/gate mechanics. The two server contracts and the per-test reply policies
must not be merged.

### R-13 — Binary invocation + run-lock cleanup in four integration helpers
`tests/common/mod.rs:44-64`, `tests/process.rs:19-40`,
`tests/cli_contract.rs:1112-1156` and `tests/artifact_repair_bridge.rs:544-571` each
spawn `CARGO_BIN_EXE_girpr`, apply env/args and call `remove_run_lock_for_args`.
Candidate boundary: one helper in `tests/common/mod.rs` parameterized by stdout
policy (captured/closed/piped); if it moves into a library, the exe path must be an
explicit parameter because `CARGO_BIN_EXE` is integration-only.

### R-14 — Minor root duplications recorded without a proposed extraction
`src/repair/purge.rs:536-540,557-561` repeat a stale-name guard;
`rsplit('/')` last-component extraction appears in `purge.rs:581-583`,
`phases/manifest.rs:394` and several fixture/test sites; `src/repair/file/temp.rs:228-239`
reproduces the `chunk_end` arithmetic of `app/repair-plan/src/plan.rs:373-391` with a
different error policy. `src/repair/scope/limits.rs:183-184` and gircap's budget code
share a wording shape but not mechanics; no extraction is indicated there.

---

## Findings — `app/*`

### APP-01 — `clear_readonly` three times
`app/fetch/src/persistent.rs:425-446`, `src/repair/persistent_exec.rs:442-465`,
`src/repair/file/mode.rs:65-83`. See X6 / R-04. Candidate boundary: a fs leaf returning
`ReadOnlyOutcome`, with no logging; each caller keeps its warning text and fallback
(temp sibling, delete, retry open).

### APP-02 — Five hand-written CAS/saturating loops in one crate
`app/report/src/ledger.rs:202-228` (`charge_live`), `:242-263` (`refund_live`),
`:294-317` (`settle`), `:505-526` (`reserve_ram_bytes`), `:535-558`
(`release_ram_bytes`); the only shared primitive is
`app/util-counter/src/counter.rs:19-23` (`add_saturating`), used by the fast paths.
Candidate boundary: extend `girpr-util-counter` with `try_charge`/`try_refund`
returning a structured refund outcome. `Meter` keeps the `u64::MAX` sentinel, the
live/settled distinction, the `fetch_max` peak update and all warnings. Boundary
tests pin exact limit semantics (`ledger.rs:653-677,833-851,931-958,968-992`).

### APP-03 — Retryable HTTP status rule with a "keep in sync" comment
`app/fetch/src/sophon/stream.rs:745-765` (doc at `:760-761`) and
`tools/capture/http/src/transport.rs:52-59`. See X4 / CAP-03.

### APP-04 — XXH64 numeric hashing and hex parsing outside the digest leaf
`app/persistent-plan/src/digest.rs:140-147` (`xxhash64_u64`),
`app/persistent-plan/src/manifest.rs:99-107` (`parse_hash_hex`),
`app/fetch/src/persistent.rs:399-405` (`u64::from_be_bytes`), plus direct `hex` and
`xxhash-rust` dependencies at `app/persistent-plan/Cargo.toml:31-32`. See X9 / LF-04.

### APP-05 — Two size-first digest gates with hand-written verdict enums
`app/fetch/src/persistent.rs:65-113` (`ManifestVerifyError`) and
`app/persistent-plan/src/digest.rs:61-133` (`DigestVerdict`) both check size first,
then compare the preferred digest case-insensitively, and spell the size mismatch the
same way. A third MD5-only comparison is `app/fetch/src/sophon/stream.rs:206-217`.
Candidate boundary: a pure `digest-gate` value pair returning a structured verdict,
with each program mapping it to its own error type. The fatal-vs-retry policy and
MD5-vs-XXH64 preference stay local. See X7 / R-06.

### APP-06 — Four BOM-stripping, 1-based line readers
`app/audio/src/lib.rs:133-159`, `app/util-ini/src/ini.rs:132-196`,
`app/persistent-plan/src/manifest.rs:54-74`, `app/dispatch/src/proto.rs:199-220`.
See X11 / CAP-08. The BOM-only-file distinction is opposite in audio
(`lib.rs:96-101`, test `:299-315`) and util-ini (`ini.rs:924-931`).

### APP-07 — CDN URL path-segment append twice
`app/dispatch/src/dispatch.rs:172-200` and
`app/persistent-plan/src/placement.rs:136-205` both use `Url::path_segments_mut` with
`/`-splitting and empty-part skipping. See X10. Caveat: `dispatch::cdn_url` relies on
`trimmed_url` removing trailing slashes while `asset_urls` uses `pop_if_empty`; error
types and prefix selection stay local.

### APP-08 — Chunk-range validation duplicated between repair plan and capture
`app/repair-plan/src/plan.rs:356-400` and
`tools/capture/src/manifest/proto.rs:72-100` apply the same non-negative/no-overflow
checks to the same `SophonChunk` wire type. `plan.rs:68-79,118-135` repeats the rule
inside `layout`/`is_tiled`. Candidate boundary: a typed problem enum in
`girpr-chunk-identity` or a new `chunk-range` leaf. Capture's `file_size` bound is not
known at its validation site and must stay local; message wording stays local.
See X8 / CAP-05.

### APP-09 — Lock-taking rule mirrored in test fixtures
`app/lock/src/lib.rs:104-111` and `app/test-fixtures/src/run_lock.rs:113-124` have the
same body, and `app/test-fixtures/Cargo.toml:25-27` already depends on `girpr-lock`.
The fixture copy is redundant; the CLI-token rule `args_take_lock`
(`run_lock.rs:131-135`) is genuinely different and stays.

### APP-10 — Test-side canonical-path reconstruction parallels `resolve_existing_prefix`
`app/test-fixtures/src/run_lock.rs:34-87` vs `app/util-path/src/path.rs:491-533`.
Candidate boundary: expose the ancestor walk from `girpr-util-path` (public or under
its `test-support` feature). The fixture's lexical `.`/`..` replay and relative-path
anchoring stay local.

### APP-11 — `key=value` field scan in three helpers
`app/lock/src/lib.rs:385-393`, `app/test-fixtures/src/lib.rs:70-76` and
`tests/repair_machinery.rs:133-143` find a `name=` token among space-separated
fields with three return policies. Candidate boundary: a small token helper in
`crates/report-core` (which already owns line writing); parse-or-panic wrappers stay
local.

### APP-12 — Reserved-capacity bounded range read duplicated
`app/util-hash/src/md5.rs:37-54,310-334,406-430` and
`src/repair/file/temp.rs:365-410` both reserve exact spare capacity, read through a
bounded `Take` into it, and classify a short read. Candidate boundary: promote
`read_into_vec` to a public `read_up_to`/`reserve_exact` primitive in `girpr-util-hash`.
`capture_slice` keeps `COPY_BUF` chunking and `SliceCapture`; short-read semantics
must not change. See R-03 / LF-02.

### APP-13 — Two async retry drivers over the same schedule/sleep leaf
`app/fetch/src/retry.rs:106-134,147-181` and
`tools/capture/http/src/fetcher.rs:789-861` repeat the attempt loop, first/last-attempt
wait rule, injected sleep, fatal-stops-immediately and last-error reporting.
Candidate boundary: a policy-free generic driver in a leaf *above* `girpr-retry`,
because `crates/retry/src/lib.rs:4-8` explicitly leaves classification, carried work
and side effects to callers. Jitter choice (`fetch/retry.rs:36-58` vs
`transport.rs:43-49`), per-attempt journal logging and typed failures stay local.
Blast radius: every HTTP attempt in both products.

### APP-14 — Two thin canned-server wrappers
`src/test_http.rs:15-37` and `app/fetch/src/test_http.rs:17-69` wrap
`ScriptedServer::start` with the same body→`Reply` mapping. Both files and the leaf
(`crates/loopback-mock/src/lib.rs:10-14`) state the local contract is deliberate;
the duplication is ~30 lines and no extraction is indicated beyond an optional
`script_bodies` helper on the leaf.

### APP-15 — Third-party versions repeated in every `app/*` manifest
Examples: `anyhow 1.0.105` in eight manifests, `tracing 0.1.44` in six,
`serde_json 1.0.154` in six, `tokio 1.54.1` in three files at two versions of the
feature set, `url 2.5.9` and `zstd 0.14.5` in two each. See X14. Feature lists and the
explanatory isolation comments stay local.

### APP-16 — ASCII case-folded rel keys written by hand
`app/persistent-plan/src/placement.rs:49-55` (`rel_key = to_ascii_lowercase`) vs
`app/util-path/src/path.rs:195-234` (`path_key` → `girpr_fs_names::fold_ascii_case`).
Candidate boundary: call the existing leaf helper with `fold_case = true`, or
re-export a `fold_always` helper from `girpr-util-path`, which `persistent-plan`
already depends on. The *when to fold* policy stays at the call site.

### APP-17 — `layout()` and `is_tiled()` implement the same tiling rule twice
`app/repair-plan/src/plan.rs:62-107` and `:109-136`; the fast path is a deliberate
allocation-free variant (doc at `:109-117`). Candidate boundary: one canonical cursor
predicate in the leaf, with the allocation-free shape preserved.
`offset + len` at `:93-94` can overflow where `:133` saturates — a latent difference
worth resolving independently of the refactor. See X8 / CAP-05.

### APP-18 — Three "first out-of-scope item" predicates
`app/persistent-plan/src/marker.rs:82-90,92-104,106-118` are the same `find` with
different key extraction and an extra hotfix filter. A private generic helper in the
same module is the candidate boundary; no new crate. The three names and docs are the
public contract.

### APP-19 — One crate imports `Md5` from two leaves
`app/fetch/src/persistent.rs:31` uses `girpr_digest::Md5` while
`app/fetch/src/sophon/stream.rs:10` uses `girpr_util_hash::Md5`, which re-exports the
same type (`app/util-hash/src/md5.rs:61`, `lib.rs:16`). Standardizing on the digest
path is a clarity change; removing the re-export is a separate API decision because
`util-hash` documents it (`app/util-hash/src/lib.rs:7-10`).

---

## Findings — `crates/*`

### LF-01 — Two restartable byte-sink traits
`crates/digest/src/lib.rs:34-37` (`BodyFile: Write + Send`) and
`app/fetch/src/sophon/stream.rs:334-337` (`ChunkSink: Write`) express the same
`restart()` contract; `Vec<u8>` and `&mut T` impls exist only on the fetch side
(`stream.rs:339-352`), file impls are split between `src/repair/file/temp.rs:649-657`
and `tools/capture/hash/src/lib.rs:239-243`. Candidate boundary: widen `girpr-digest`
with a `RestartableWrite` base trait and move the generic impls into it; `BodyFile`
must keep `Send` for its `dyn` use. Coherence with existing explicit impls is the main
hazard, and `ChunkSink` is re-exported publicly (`src/lib.rs:157`).

### LF-02 — Caller-buffered file-hash loop twice
`crates/digest/src/stream.rs:248-262` (`hash_reader`) and
`tools/sync/cache/src/hash/stream.rs:96-113` (`hash_file_with_buf`) contain the same
read→`Sink::update` loop. Candidate boundary: delegate the cache to `hash_reader` and
move the empty-buffer refusal into the leaf. Algorithm-set translation, chunk size
(`CHUNK = 512 KiB`) and digest comparison stay in the cache. See APP-12 / SYNC-11.

### LF-03 — Saturating non-negative byte accumulation in ~20 sites
Extracted once at `crates/chunk-cost/src/lib.rs:48-58`, re-derived across capture
(`tools/capture/src/manifest/diff.rs:424-651`, `manifest/mod.rs:713-758`,
`test-support/src/lib.rs:1130-1138`), repair (`app/chunk-verify/src/lib.rs:69-82`,
`app/repair-plan/src/plan.rs:63,119,133`) and the facade. Candidate boundary: pure
`nonneg`, `saturating_nonneg_sum` and `checked_end` primitives in `girpr-chunk-cost`.
Signed cursors and byte totals must use `checked_end`; u64 budget saturations that
never came from `i64` must stay out of the helper. See X8.

### LF-04 — XXH64 ↔ hex/number conversions have no leaf owner
`crates/digest/src/lib.rs:84-87`, `crates/digest/src/stream.rs:121-129,198-202,276-279`,
`app/persistent-plan/src/digest.rs:117,140-147`,
`app/persistent-plan/src/manifest.rs:99-107`, `app/fetch/src/persistent.rs:403`.
See X9 / APP-04.

### LF-05 — Atomic publish through a temp sibling twice
`app/util-path/src/atomic.rs:62-130` and
`tools/sync/cache-json/src/convert.rs:278-311`, with related protocols in
`tools/sync/cache/src/backup/publish.rs` and `tools/sync/src/commands/sync/rename.rs`.
See X13 / SYNC-06.

### LF-06 — `strip_ansi` implemented twice
`crates/tempdir/src/test_support.rs:47-49` and `tests/cli_contract.rs:72-73`; the
underlying dev-dependency is also repeated (`Cargo.toml:222`,
`crates/tempdir/Cargo.toml:19`). Candidate boundary: use the leaf version and remove
the root helper. Test-only; see X12.

### LF-07 — Hex encode/decode wrappers bypassed in `cache-json`
`tools/sync/cache-json/src/ndjson.rs:94,97,121` calls `hex` directly despite
`crates/digest/src/lib.rs:60-72` owning the wrappers; the dependency is repeated at
`tools/sync/cache-json/Cargo.toml:32`. Candidate boundary: adopt the wrappers and drop
the direct dependency. Error context wording stays local. See X9.

### LF-08 — Four-language display-name → code map re-derived
`crates/audio-language/src/lib.rs:51-73` owns code→name and position;
`app/audio/src/lib.rs:142-150` hand-rolls the inverse four-arm match and `:110-114`
hard-codes the name list. `tools/capture/src/emit/gamedir.rs:20,128,135-136` adopts the
leaf correctly. Candidate boundary: a parameterized reverse lookup taking caller
aliases, plus a joined display-name helper; the alias map stays in `app/audio`. The
leaf's docs currently promise no alias map (`lib.rs:9-14`), so the parameter must be
explicit. See X11 / CAP-07.

### LF-09 — Reserved device-name sets copied four times
`app/util-path/src/path.rs:76-79`, `tools/capture/src/paths.rs:67-70`,
`crates/fs-names/src/lib.rs:154-157` and `crates/path-shape/src/lib.rs:189-192`.
The predicate itself is already shared (`crates/fs-names/src/lib.rs:53-75`). Candidate
boundary: named constants or a `DeviceRule` enum in `girpr-fs-names`, referenced by
the two test copies; the choice of rule stays at call sites. This would change a leaf
that currently owns no policy.

### LF-10 — Half-open byte-range algebra modelled independently
`tools/capture/src/assemble/ranges.rs:27-235` uses an `i64` `Range` struct with
published `start`/`end` keys; repair uses tuples/`u64` in `app/repair-plan/src/plan.rs:62-151`,
`src/repair/reuse.rs:135-138`, `src/repair/file/inplace.rs:1012-1015` and `app/fetch/src/sophon/map.rs:187`.
Candidate boundary: a `girpr-byte-range` leaf with `at`, `new`, `len_sat`, `is_empty`,
`is_inverted`, `contains`, `overlaps`, `gap_to`, `checked_end`. Signedness, published
keys and repair's gap-record type must not change silently. See X8 / CAP-05.

### LF-11 — Backslash→slash rewrites spelled inline despite a shared helper
`crates/fs-names/src/lib.rs:111-124` (`forward_slash`) is used by `path-shape` but
bypassed by `crates/filter/src/lib.rs:84`, `app/persistent-plan/src/placement.rs:169`
and `src/repair/persistent_exec.rs:473`; the inverse rewrite appears in
`crates/tempdir/src/test_support.rs:239,249` and two test files. Candidate boundary:
adopt the helper and add an inverse `native_separator`. `girpr-filter`'s
`Backslash::Escape` policy depends on the `Cow` shape and must be preserved.

### LF-12 — `Md5` and `Sha256` newtypes duplicate ~10 methods
`crates/digest/src/lib.rs:97-187` and `:189-258` differ only in array length, error
text and the presence of `Md5::try_parse_hex` (`:131-138`). Candidate boundary: an
internal macro/const-generic template with error spelling as an argument. Public
signatures must stay identical, and the no-allocation parse property used at
`crates/chunk-identity/src/lib.rs:88-99` must survive.

### LF-13 — Two `Arc<Mutex<Vec<u8>>>` writers and two `Targets` layers inside `girpr-log`
`crates/log/src/test_support.rs:47-75,115-134` vs `crates/log/src/install.rs:129-140,193-206`
and `crates/log/src/filter.rs:77-97`. Candidate boundary: internal test helpers in
`girpr-log`, keeping `CaptureBuffer` `pub(crate)` unless the feature surface is
intentionally widened. No blast radius outside the crate's tests.

### LF-14 — Log filter parse-and-warn duplicated in one module
`crates/log/src/filter.rs:20-28` and `:45-66` repeat `try_new` → warn → fallback.
Candidate boundary: a private `parse_or_warn(spec, fallback, what)` helper. The two
warning strings are asserted (`filter.rs:115-129,162-194`) and must stay byte-identical
unless the tests are updated deliberately. See X14 for the manifest finding in this
area.

### LF-15 — `normalize_hex` sits one leaf away from the other hex mechanics
`crates/chunk-identity/src/lib.rs:102-114` owns in-place lowercasing; all other hex
mechanics are in `crates/digest/src/lib.rs:59-87`. Sole consumer:
`tools/capture/src/manifest/proto.rs:17,103,145-146`. Candidate boundary: move it to
`girpr-digest` and re-export from `girpr-chunk-identity`; do not move it into
`girpr-proto` (would change dump bytes). The *when to normalize* decision stays at
the capture decode boundary.

---

## Findings — `tools/sync*`

### SYNC-01 — Five global CLI flags declared three times
`tools/sync/src/cli.rs:20-69`, `tools/capture/src/cli/mod.rs:64-120`, `src/cli.rs:233-282`.
See X1. Candidate boundary: a flattenable clap `Args` struct in `girpr-cli`,
parameterized by project name, `global` and the caller's format type. Help bytes and
exit classification are asserted in all three programs.

### SYNC-02 — `OutputFormat` defined three times
`tools/sync/src/report/mod.rs:57-85`, `app/report/src/record.rs:31-67`,
`tools/capture/src/report/mod.rs:36-67` (with `FromStr`). See X1 / CAP-12. Candidate
boundary: move the value set, `as_str`, `Display` and `FromStr` to `girpr-report-core`
behind an optional `clap` feature; capture's error wording stays local.

### SYNC-03 — Three `Record` planes over one core
`tools/sync/src/report/record.rs:22-282`, `app/report/src/record.rs:79-234`,
`tools/capture/src/report/mod.rs:69-371`. See X3. Candidate boundary: a generic
ordered field-list/record mechanic in `girpr-report-core`; `Val` vocabularies, layouts,
`Order` choice and event-name derivation stay local. Blast radius: stdout bytes for
three binaries.

### SYNC-04 — Tracing subscriber stack built three times
`tools/sync/src/logging.rs:50-212`, `tools/capture/src/logging/mod.rs:56-214`,
`src/logging.rs:77-221`. See X2. Caveat: `crates/log/src/install.rs:1-14` documents the
current split as intentional; gircap wraps its file in a thread-safe `MakeWriter`,
girpr writes text rather than JSON, and girsync already exposes a `subscriber_for` seam.

### SYNC-05 — Directory-link creation with a junction fallback duplicated
`crates/tempdir/src/test_support.rs:112-145` (`junction::create`) and
`tools/sync/src/commands/sync/apply/copy.rs:530-573` (`cmd /C mklink /J`) repeat the
symlink→junction fallback and the `"{first}; junction fallback failed: {e}"` wording.
Candidate boundary: one helper with the fallback strategy as a parameter; both
mechanisms must remain available. Test-only.

### SYNC-06 — "Claim a unique sibling, then publish" four times
`tools/sync/cache-json/src/convert.rs:40-45,278-326`,
`tools/sync/cache/src/backup/publish.rs:30-117,457-502,690-710`,
`tools/sync/src/commands/sync/rename.rs:200-281`, `app/util-path/src/atomic.rs:62-130`.
See X13. Caveat: the protocols are not the same (hard-link publish, one-shot
`persist`, two-rename case fix); share only name claiming and cleanup.

### SYNC-07 — `girpr-cache*` name reservation re-spelled in a test helper
`crates/tempdir/src/test_support.rs:253-266` filters the literal case-sensitively,
while production folds ASCII case on Windows (`tools/sync/src/util.rs:126-227`,
`tools/sync/cache/src/schema.rs:12`). The two currently disagree on `GIRPR-CACHE*`.
Candidate boundary: a tiny `girpr-cache-names` leaf owning the prefix and predicates;
`girpr-tempdir` cannot depend on `girsync-cache`, so a new leaf is the clean home.

### SYNC-08 — Path-prefix iteration written three ways
`tools/sync/src/commands/sync/plan/link.rs:43-57,130-135` and
`tools/sync/leaves/src/link.rs:148-185` differ in whether the full path is included.
Candidate boundary: `path_prefixes`/`strict_path_prefixes` in `girsync-leaves`; the
membership/disk-stat questions and short-circuit order stay at call sites.

### SYNC-09 — argv-prefix incantation repeated nine times
`tools/sync/test-support/src/lib.rs:619-793`; parallels in
`tools/capture/test-support/src/lib.rs:140-148` and root tests. Candidate boundary:
`argv_with_program(program, args)` in `girpr-cli`. Capture records full argv in
`MANIFEST.json → tool_argv`, so the sentinel spelling is a byte contract.

### SYNC-10 — Stdout emit (render → log → write) three implementations
`tools/sync/src/report/mod.rs:167-189`, `app/report/src/emit.rs:14-54`,
`tools/capture/src/report/mod.rs:715-736`. See X3. Candidate boundary: a
`write_record_line(writer, line, log_hook)` sink in `girpr-report-core` with the
broken-pipe policy parameterized. girsync's `println!` and capture's locked stdout
must not be silently unified.

### SYNC-11 — `copy_and_digest` re-implements the shared streaming-hash read loop
`tools/sync/src/commands/sync/apply/copy.rs:33-54`,
`crates/digest/src/stream.rs:242-262`, `tools/sync/cache/src/hash/stream.rs:96-110`,
`tools/capture/hash/src/lib.rs:84-152`. Candidate boundary: a `copy_with_sink` helper
in `crates/digest/src/stream.rs`. Write-before-hash ordering and the `io::copy`
empty-set fast path stay local. See LF-02 / APP-12.

### SYNC-12 — Three progress-heartbeat loops
`tools/sync/src/scan.rs:9-49,220-226`, `effective/phase_a.rs:386-388,488-498`,
`effective/phase_c.rs:110-124,184-195`; one shared piece (`elapsed_s`, `util.rs:36-39`).
Candidate boundary: a `Heartbeat` struct in `girsync-leaves` with an injected trigger
predicate. The test-injected `GIRSYNC_TEST_HEARTBEAT_ENTRIES` seam stays.

### SYNC-13 — Manifest-level duplication across five `tools/sync` manifests
Identical lint blocks, `version`/`edition` and repeated dependency versions.
See X14. Caveat: Cargo cannot inherit a *subset* of workspace lints, so the tools'
opt-out must stay local; exact pins and per-member feature lists must be preserved.

### SYNC-14 — Areas checked with sharing already correct
No new-crate candidate was found for effective/plan records, test HTTP servers, or
the already-shared hash/filter/log-policy/time/clap/error/range/test-fixture leaves.
Girsync has no HTTP test server of its own. The hash sink, glob adapters and timestamp
formatter are adopted at their intended call sites.

---

## Findings — `tools/capture*`

### CAP-01 — HYP/Sophon request URL assembly twice plus a query rewriter
`tools/capture/src/biz/mod.rs:201-230,305-310,320-336`,
`tools/capture/src/http/probe.rs:577-611`, `app/fetch/src/hyp.rs:359-383,636-663,689-691`.
See X10. Candidate boundary: a `girpr-request-url` leaf with `push_pair`,
`encode_value`, `sophon_get_build_url`, `hyp_endpoint_url` and `replace_query_value`.
`Biz::resolve`, base overrides, `CAPTURE_LANGUAGE`, `needs_channel`, POST policy and
the literal bracketed key stay local. URLs are recorded verbatim in golden NDJSON.

### CAP-02 — One `User-Agent` literal duplicated
`tools/capture/http/src/lib.rs:78-89` and `app/fetch/src/hyp.rs:327-332`, with golden
and test literals at `tools/capture/src/logging/mod.rs:235-242`,
`tools/capture/http/src/tests.rs:362-368`, `tools/capture/tests/recorded_json_golden.rs:8-74`.
Candidate boundary: an API-identity constant in `crates/model` or a small leaf;
capture's re-export through `logging` stays.

### CAP-03 — Retryable-status predicate twice
`tools/capture/http/src/transport.rs:52-59` and `app/fetch/src/sophon/stream.rs:745-765`.
See X4.

### CAP-04 — Bounded ordered join with panic recovery
`tools/capture/http/src/tasks.rs:15-153` vs the repair loops in
`src/repair/scan.rs:304-352`, `phases/check.rs:110-175`, `persistent_exec.rs:71-81`,
`file/mod.rs:548-589`. See X5. Callers keep `JoinOutcome` → `ApiError` mapping and the
scoped dispatcher adoption.

### CAP-05 — Chunk tiling/gap/overlap algebra three times
`app/repair-plan/src/plan.rs:43-151`, `tools/capture/src/assemble/ranges.rs:27-235`,
`tools/capture/src/manifest/mod.rs:722-787`. See X8 / LF-10. Candidate boundary:
`girpr-chunk-layout` with a half-open range and a parameterized diagnostic sink.
A11 violation strings are published in `verify/selfcheck.json`; repair's gap-fill
decisions drive disk writes.

### CAP-06 — Blob-id shape rules and prefix/id/suffix joins parallel
`tools/capture/src/paths.rs:258-287`, `tools/capture/src/hyp/types.rs:596-625`,
`tools/capture/src/blobs/mod.rs:210-268`, `app/fetch/src/sophon/stream.rs:680-743`.
Candidate boundary: `validate_blob_id(&str, Mode::{StrictHexPair, OpaqueSegment})` and
`join_prefix_id_suffix` in `crates/model` or a `girpr-blob-id` leaf. Tightening girpr
to the strict shape could reject real ids; rejection rule IDs and `Url::join`
assertions stay local.

### CAP-07 — Audio scan-file rendering with opposite newline contracts
`tools/capture/src/emit/gamedir.rs:113-144` (trailing newline) vs
`app/audio/src/lib.rs:161-178` (no trailing newline), both over
`crates/audio-language/src/lib.rs:44-73`. Candidate boundary:
`render_scan_file(codes, trailing_newline: bool)` in that leaf. The differing
validation policy and file I/O stay local; `tools/capture/Cargo.toml:238-242` claims
identical bytes, which the flag must make explicit rather than converge.

### CAP-08 — `config.ini` version readers with opposite selection policies
`tools/capture/src/emit/gamedir.rs:61-111` (first match in `[General]`) vs
`app/util-ini/src/ini.rs:11-57` (last match, no section filter). Candidate boundary:
`game_version_from_text(text, Selection, SectionRequirement)` in `app/util-ini`;
capture could then drop its `ini_core` dependency. A13 self-check wording and
repair's last-match tests must stay pinned.

### CAP-09 — JSON envelope and string-or-number DTO handling duplicated
`tools/capture/src/hyp/types.rs:108-121,44-105`, `tools/capture/src/hyp/mod.rs:83-87`,
`tools/capture/http/src/fetcher.rs:863-898`, `app/fetch/src/hyp.rs:60-72,276-304,727`.
Candidate boundary: a `girpr-hyp-wire` leaf (or an extension of `crates/proto`) owning
envelope DTOs, `terminal_retcode` and a configurable number decoder. The two programs
disagree on non-zero `retcode` meaning; girpr's raw-byte retention, `SophonNotFound`
mapping and capture's typed dumps are policy.

### CAP-10 — Logging subscriber construction across all three binaries
`tools/capture/src/logging/mod.rs:51-221`, `tools/sync/src/logging.rs:45-212`,
`src/logging.rs:56-238`. See X2. Caveat: capture's `AppendFile` serializes writes;
girpr's static guard must remain for `flush_log` on every exit path.

### CAP-11 — clap-error/exit shim repeated in three `main`s
`tools/capture/src/main.rs:8-38,61-83`, `tools/capture/src/cli/mod.rs:393-405,477-502`,
`tools/capture/src/exit/mod.rs:96-105`, `src/main.rs:15-46,141-165`,
`tools/sync/src/main.rs:13-42`. Mechanism already shared at
`crates/cli/src/lib.rs:11-21` and `crates/exit/src/lib.rs:26-76`. Candidate boundary:
a generic `parse_or_report_error` in `girpr-cli` with injected print/exit, plus a
shared `log_file_announcement`. Process boundaries stay local: capture flushes stdout
before `process::exit`, girpr returns `ExitCode` so destructors run.

### CAP-12 — `OutputFormat` and emitter parallel across three report planes
`tools/capture/src/report/mod.rs:36-67,69-371,715-736`,
`app/report/src/record.rs:31-234`, `app/report/src/emit.rs:26-54`,
`tools/sync/src/report/mod.rs:57-85`. See X1 / X3 / SYNC-02. Candidate boundary:
shared `OutputFormat` plus `emit_line` in `girpr-report-core`; `Val` must not move
because the boolean and escaping contracts differ.

### CAP-13 — Two deliberately duplicated zstd measurement loops
`tools/capture/hash/src/lib.rs:499-626`; rationale at `:563-580`; error strings pinned
at `tools/capture/hash/src/tests.rs:322-394,484-489`. Candidate boundary: an injected
error formatter inside `gircap-hash`, not a workspace crate. Error text must not be
unified (`tests/offline_bundle.rs:691-695`).

### CAP-14 — Two streaming JSON+gzip writers coexist in `emit`
`tools/capture/src/emit/mod.rs:417-444,446-522,524-593`. Candidate boundary: one
`serialize_pretty_to` + `gzip_from_reader` pair with a caller-chosen count-first vs
single-serialization driver. The `BufWriter` placement is measured (12 s vs 46 s) and
byte-equality is pinned; only the common body may be factored.

### CAP-15 — Raw-socket canned HTTP servers re-implemented in capture tests
`tools/capture/src/hyp/mod.rs:641-702,996-1081`, `tools/capture/http/src/tests.rs:712-873`,
`tools/capture/src/blobs/tests.rs:140,777,902,1033`; the shared canned server is
`crates/loopback-mock/src/lib.rs:135-177`. Candidate boundary: an opt-in raw-scripted
reply mode (status, body, claimed length, omitted header, journal) in that leaf. The
malformed tests require a server that can lie about `Content-Length`, so a
wiremock-only replacement is impossible.

### CAP-16 — Two full fixture HTTP servers repeat the same scaffold
`tools/capture/test-support/src/lib.rs:113-125,1209-1489,1872-1937` and
`app/test-fixtures/src/lib.rs:909-989` / `artifact_server.rs:304-399`. Candidate
boundary: a test-only `girpr-test-http` leaf owning ephemeral bind, helper runtime,
journal and Drop shutdown with an injected handler. `docs/00-spec.md` §2 forbids
sharing gircap's *fixtures* with repair, so only server mechanics may move.

### CAP-17 — Query-parameter lookup/rewrite written three ways
`tools/capture/src/http/probe.rs:595-611`,
`tools/capture/test-support/src/lib.rs:2058-2064`,
`app/test-fixtures/src/lib.rs:1306-1314`. Candidate boundary: `literal_query_value`
and `replace_query_value` in the X10 leaf or `crates/model::query`. A `Url`-based
parser would escape `game_ids[]` and change every recorded row.

### CAP-18 — Bounded deterministic stores and schema-version reads repeated
`tools/capture/src/rejections.rs:40-181`, `tools/capture/src/pathmap.rs:31-125`,
`tools/capture/http/src/error_bodies.rs:38-265`, and schema reads in
`tools/capture/src/verify/bundle.rs:344-512`. Candidate boundary: an internal
`bounded_store` helper with keyed/byte-budget policy injected and a single
`declared_schema_version` helper. Row order and `omitted`/`total_retained_bytes`
fields are published; the three stores genuinely differ.

### CAP-19 — Dependency versions and lint tables repeated across four manifests
`tools/capture/Cargo.toml:95-99,125-299`, `hash/Cargo.toml:24-52`,
`http/Cargo.toml:17-62`, `test-support/Cargo.toml:18-60`. See X14. Narrow feature
lists (`tools/capture/Cargo.toml:171-185`, `http/Cargo.toml:40-46`) must not widen.

### CAP-20 — Python/Rust class-set duplication is deliberate, not a candidate
`tools/capture/tools/case_coverage.py:47-105` vs
`tools/capture/src/select/classes.rs:24-29,95-167`, gated by
`tools/capture/tests/case_coverage.rs:1-47`. The instrument is stdlib-only and exists
to measure a class set the crate does not yet have; any shared representation would
destroy that gap-detection purpose.

---

## Non-findings and explicit keepers

These were examined and are recorded as *not* shared-crate candidates. They are not
to be changed as a side effect of a finding above.

- `girpr-exit::define_exit_table!` (`crates/exit/src/lib.rs:27-76`) is already the one
  mechanism; the per-program code values (`src/config.rs:28`,
  `tools/capture/src/exit/mod.rs:28-53`) are program contracts.
- `girpr-cli::clap_error_code` (`crates/cli/src/lib.rs:16-21`) is adopted by all three
  programs and `girsync-cache-json`; only the surrounding shim is repeated (CAP-11).
- `girpr-filter` is already shared by girpr and girsync; backslash-escape and case
  adapters are call-site policy. Device-name *predicates* are shared
  (`crates/fs-names/src/lib.rs:53-75`); only the named sets are duplicated (LF-09).
- `girpr-loopback-mock` (wiremock canned replies), `app/test-fixtures`' axum replay
  server and `tools/capture/test-support`'s axum world server serve deliberately
  different contracts; the duplication is in scaffolding only (R-12, CAP-15, CAP-16).
- `girpr-digest::Sink`/`Algorithms`/`DigestBytes`, `girpr-time`, `girpr-value-ranges`,
  `girpr-model::query::QUERY_VALUE_SET`, `girpr-fs-identity`, `girpr-report-core`'s
  ordered writer and line writer, and `girpr-tempdir::test_support` are already adopted
  at their intended call sites; only their surfaces or bypassing call sites are findings.
- `app/report/src/ledger.rs` saturating `AtomicU64` reservation is intentionally local
  (APP-02 shares only the CAS mechanics); `app/util-hash/src/md5.rs` one-pass
  multi-range MD5, `app/repair-plan/src/plan.rs` fingerprint and `tools/sync/cache/src/hash/digestset.rs`
  zero-allocation row were examined without a credible new boundary.
- Retry drivers stay local: `crates/retry/src/lib.rs:4-8` deliberately leaves
  classification and side effects to callers (APP-13); capture's per-attempt journal
  and girpr's typed failures are policy.
- `tools/sync/cache/src/open.rs` backup protocol, `db/*`, `write.rs`, `backend.rs`,
  `tools/sync/leaves/src/rw/*` and `link.rs` were examined; their mechanics already
  have library homes or bespoke policies with no equivalent elsewhere.
- `tools/capture/src/manifest/diff.rs`, `assemble/ranges.rs`, `assemble/assembly.rs` and
  `tools/capture/http/src/error_bodies.rs` contain domain scoring and fault-split
  policies where violations are data; only their arithmetic mechanics are findings.
- `tools/capture/src/paths.rs`, `pathmap.rs` and `encoding.rs` already delegate to
  `girpr-path-shape`, `girpr-fs-names` and `percent-encoding`; they are not themselves
  duplicate mechanics.
- `tests/scripts/io-profile/**` and the Python/Rust class-set gap (CAP-20) are out of
  scope by design.
