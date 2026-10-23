# Shared-crate refactor report

**Scope:** candidate duplication across the three projects in this workspace:

1. `girpr` — `src/` (main repair binary + library)
2. `girsync` — `tools/sync/src/` (dev-only folder mirror)
3. `gircap` — `tools/capture/src/` (dev-only artifact capture)

**Existing shared crates (already extracted):** `girpr-log` (log-file / log-level policy) and
`girpr-tempdir` (scratch-directory policy). Both are leaf crates: they depend on no program and
carry policy, not logic. Any new shared crate should follow the same shape.

**Independence constraints that bound this report:**

- `gircap` must not depend on the `girpr` crate, invoke the `girpr` binary, or share
  fixtures (`tools/capture/docs/00-spec.md` §2). A shared crate is acceptable only if it is a
  leaf that none of the three programs owns.
- `girsync` is documented as sharing no flags, exit codes, or code with `girpr`.
- Therefore the safe direction is *downward*: extract leaf utilities that all three already
  reimplement, never pull a higher-level program into another's dependency graph.

## 1. Methodology

- Read the module maps and the shared-crate precedents (`crates/log`, `crates/tempdir`).
- Compared sibling modules by name and responsibility: `logging.rs`, `util.rs`/`hash.rs`,
  `cli.rs`, `config.rs`, `biz.rs`, `hyp.rs`/`hyp/*`, `manifest/proto.rs`, `filter.rs`,
  `report.rs`, `emit/gamedir.rs`.
- Ran an exact-line duplicate scan over `src/`, `tools/sync/src/`, `tools/capture/src/`
  (lines ≥ 40 chars, excluding comments): **48 cross-project duplicate lines**, **22
  six-line windows** duplicated across projects. The largest clusters are the tracing
  subscriber setup in all three `logging.rs` files and the `OutputFormat`/record rendering
  in `report.rs`.
- Risk is defined as in the in-tree refactor plans: **probability of changing an
  observable behaviour a test or user could notice**, not edit difficulty. A mechanical
  change to a load-bearing invariant is high risk; a large mechanical change to a leaf is
  low risk.

## 2. Risk tiers

| Tier | Meaning | Landing rule |
|---|---|---|
| **Low** | Behaviour-preserving by construction, or provably dead / pure utility | One commit each; green tests sufficient |
| **Medium** | Behaviour-preserving in intent, but touches a shared path, an output contract, or a type that crosses crate boundaries | One commit each, plus the named characterisation test added *before* the change |
| **High** | Changes something a user or downstream reader can observe, or reconciles two divergent implementations | Characterisation test committed first, plus a doc/README update in the same change |
| **Very high** | Violates an independence rule or couples a program to another program's internals | Do not do without an explicit decision to relax the independence rule |

Items are ordered within each tier by value (most duplicated surface first).

---

## 3. Low-risk candidates

### L1 — MD5 / SHA-256 hex and one-pass dual digest

- **Locations:** `tools/capture/src/hash.rs` (`md5_hex`, `sha256_hex`, `hex_bytes`,
  `hex_bytes_upper`, `Digests`, `DualDigest`, `md5_sha256`); `src/util.rs` (`md5_file`,
  `EMPTY_MD5`); `tools/sync/src/hash.rs` (uses `md5::Context`/`sha2` directly).
- **Duplication:** three spellings of "lowercase hex of a digest" and two spellings of
  "feed bytes to MD5+SHA-256 in one pass". `capture` already centralised this; the other
  two re-derive it.
- **Proposed crate API:** `girpr-hash` (or fold into a new `girpr-bytes` crate):
  `md5_hex`, `sha256_hex`, `hex_bytes`, `DualDigest: Write`, `md5_file`, `EMPTY_MD5`.
- **Risk:** Low. Pure functions / `Write` impls with no side effects; golden-digest tests
  pin the spelling.
- **Caveat:** keep `md5_file`'s 512 KiB streaming buffer and `NotFound` handling in the
  shared version so the main binary's single-read scan is unchanged.

### L2 — gzip / gunzip / zstd helpers + magic detection

- **Locations:** `tools/capture/src/hash.rs` (`gzip`, `gunzip`, `ZSTD_MAGIC`, `is_zstd`,
  `zstd_decompress`, `zstd_decompress_from`, `Measure`, `zstd_measure`,
  `zstd_measure_from`); `src/sophon.rs` (uses `zstd::decode_all` / streaming decoder);
  `src/repair/file.rs`.
- **Duplication:** zstd magic check and "decode all with a label" are written twice.
- **Proposed crate API:** `zstd_decompress(bytes, what)`, `zstd_decompress_from(reader, what)`,
  `is_zstd`, `gzip`/`gunzip`, `Measure`.
- **Risk:** Low for the simple helpers. **Do not** replace `src/sophon.rs`'s streaming
  decoder with the buffered helper — that is a separate, high-risk item (H5).
- **Validation:** round-trip + truncated-frame tests already exist in `capture`; port them.

### L3 — Opening a log file (append, create parent)

- **Locations:** `src/logging.rs::open_log_file`, `tools/sync/src/logging.rs::open_log_file`,
  `tools/capture/src/logging.rs::open_log_file` (plus `AppendFile` wrapper in capture).
- **Duplication:** three copies, differing only in whether the parent is created when empty
  and whether the handle is mutex-wrapped.
- **Proposed crate API:** `girpr_log::open_log_file(path) -> Result<impl Write + Send + Sync>`
  (append, create parent when non-empty). Capture keeps its `Arc<Mutex<File>>` wrapper on top
  if it still needs `MakeWriter`.
- **Risk:** Low. Standardise on "create parent if non-empty"; this only adds a directory
  creation in the degenerate case and cannot change a successfully opened file.

### L4 — HTTP retryable-status classification

- **Locations:** `src/sophon.rs::retryable` (with tests at lines 790–814);
  `tools/capture/src/http/mod.rs::RetryPolicy::should_retry_status` (with tests at 1696–1710).
- **Duplication:** byte-identical rule: `matches!(status, 408 | 429) || (500..=599).contains(&status)`.
- **Proposed crate API:** `girpr_http::is_retryable_status(u16) -> bool` (leaf crate) or a
  free function in a new `girpr-net` crate.
- **Risk:** Low. Pure function, already pinned by tests in both projects.
- **Caveat:** only share the *classification*. The full `RetryPolicy` (attempts, backoff,
  `Sleep` trait) stays project-local (see H2).

### L5 — User-Agent constant

- **Locations:** `tools/capture/src/logging.rs::USER_AGENT` / `user_agent()`; `src/hyp.rs`
  and `src/sophon.rs` (hardcoded string in `reqwest::Client::builder()`).
- **Duplication:** the same Unity player string in two projects, three spellings.
- **Proposed crate API:** `girpr_http::USER_AGENT` (or `girpr-log` if we prefer no new crate).
- **Risk:** Low.

### L6 — Glob pattern compilation (core)

- **Locations:** `tools/sync/src/filter.rs::compile_patterns`;
  `src/repair/scope/filter.rs::compile_patterns` (adds backslash → `/` normalisation).
- **Duplication:** identical `Pattern::new` loop, differing in backslash handling and in
  whether the error names the glob.
- **Proposed crate API:** `girpr-glob::compile_patterns(list, normalize_backslashes: bool)`.
- **Risk:** Low-Medium. The backslash difference is a deliberate main-side feature; making
  it a parameter preserves both behaviours exactly. Tests for both spellings must be ported.

### L7 — `read_game_version` (config.ini parser)

- **Locations:** `src/util.rs::read_game_version` (regex, last-match-wins);
  `tools/capture/src/emit/gamedir.rs::parse_game_version`.
- **Duplication:** two parsers for the same `[General] game_version=` line, with slightly
  different whitespace handling.
- **Proposed crate API:** `girpr-config::read_game_version(text) -> Option<String>` with
  last-match-wins semantics (the main binary's documented behaviour).
- **Risk:** Low if the shared parser adopts the main binary's last-match-wins rule and the
  capture side is checked against it. Capture's parser must be migrated to call it, and its
  fixture tests re-run.

### L8 — `join_url` (trim trailing slash + append id)

- **Locations:** `src/sophon.rs::join_url`; inlined `format!("{}/...", trim_end_matches('/'))`
  in `tools/capture/src/biz.rs`, `config.rs`, `http/probe.rs`.
- **Duplication:** the same one-liner, four spellings.
- **Proposed crate API:** `girpr_http::join_url(prefix, id) -> String`.
- **Risk:** Low.

---

## 4. Medium-risk candidates

### M1 — HoYoPlay / Sophon API types and constants (types only, not clients)

- **Locations:** `src/hyp.rs` (`GameConfig`, `GameBranch`, `GameBranchPackage`, `ChunkBuild`,
  `ChunkManifestMeta`, `ManifestFile`, `ManifestUrl`, `DeprecatedConfig`, `DeprecatedFile`,
  `terminal_retcode`, `production_bases`, `DOWNLOAD_MODE_CHUNK`); `tools/capture/src/hyp/types.rs`
  (parallel set: `GameConfigs`, `LaunchConfig`, `GameBranches`, `Branch`, `Category`,
  `DeprecatedFiles`, `DeprecatedConfig`, `DeprecatedFile`, `GetBuild`, `BuildManifest`,
  `ManifestRef`, `Download`, `Stats`, `DOWNLOAD_MODE_CHUNK`); `tools/capture/src/biz.rs`
  (`Biz`, `Host`, endpoints).
- **Duplication:** the same API field names, ids, channel tuples, and host bases, twice, with
  different Rust shapes.
- **Proposed crate API:** a leaf `girpr-hyp` crate carrying the deserialization structs,
  `Biz`, `Host`, `terminal_retcode`, `production_bases`, and `DOWNLOAD_MODE_CHUNK`. HTTP
  *clients* stay in each program.
- **Risk:** Medium. The capture side uses `IntOrStr` deserialization and extra fields
  (`LegacyMain`, `GamePackages`) that the main binary does not need; the shared structs must
  be supersets (all fields `#[serde(default)]`) so both deserialise. Field renames would
  silently break capture's recorded requests, so the shared types need a contract test.
- **Validation:** capture's offline bundle tests + main's mock-server tests.

### M2 — Sophon manifest protobuf structs

- **Locations:** `src/sophon.rs` (`SophonChunkManifest`, `SophonChunkFile`, `SophonChunk`);
  `tools/capture/src/manifest/proto.rs` (same structs plus `Serialize`, `is_folder_entry`,
  `chunk_bytes`, `covered_end`, `identity`, `normalise`).
- **Duplication:** the prost message definitions, byte-identical on the wire.
- **Proposed crate API:** shared prost structs in `girpr-hyp` (or a `girpr-manifest` crate),
  with capture's helper methods and `Serialize` impl. Add `normalise()` to the shared structs
  and call it where a manifest enters each program.
- **Risk:** Medium. `capture` serialises these to the artifact with exact key names
  (`unknown_field6`, etc.); the shared crate must keep field names. `normalise()` lowercases
  digests; the main binary currently compares case-insensitively rather than normalising, so
  adopting `normalise()` changes the strings stored in logs/plans (observable in `--plan-out`
  JSON and log lines) even though comparison outcomes do not change.
- **Validation:** capture's `the_serialized_keys_are_the_documented_ones` test must pass
  unchanged against the shared type.

### M3 — Path scope / glob filtering

- **Locations:** `tools/sync/src/filter.rs` (`compile_patterns`, `is_excluded`); `src/repair/scope/filter.rs`
  (`PathScope`, `compile_patterns`, `is_visible`, `is_open`, `is_active`, `has_includes`).
- **Duplication:** the same glob semantics, ported once already; the doc in main says so explicitly.
- **Proposed crate API:** `girpr-scope::PathScope` with configurable case sensitivity and
  backslash normalisation, defaulting to each caller's current behaviour.
- **Risk:** Medium. Filtering decides which files are touched; a default flip would silently
  change `--include`/`--exclude` results. Characterisation tests must be ported verbatim
  (both projects already have them) before extraction.
- **Caveat:** sync's `case_sensitive` flag must remain expressible; main hardcodes `false`.

### M4 — Report record rendering helpers

- **Locations:** `src/report.rs` and `tools/capture/src/report.rs` (shared `json_string`,
  escaping, `String::with_capacity` capacity logic, `OutputFormat::as_str`); `tools/sync/src/report/mod.rs`
  and `record.rs` (`keyed`, `fields()` iteration).
- **Duplication:** JSON-string escaping and the `key=value` rendering loop, three spellings.
- **Proposed crate API:** `girpr-report::Record`, `Val`, `render_text`, `render_json`,
  `json_string`.
- **Risk:** Medium-High. These strings are the CLI output contract (`REPORT`/`SUMMARY`/
  `PLAN` lines) and are asserted byte-for-byte by `cli_contract` tests in all three projects.
  Extraction must be byte-identical; any change needs a simultaneous update of every golden test.
- **Recommendation:** extract only the pure escaping helpers (`json_string`, `hex`) first (Low),
  and leave the record types and stdout vocabulary project-local.

### M5 — CLI log-flag block

- **Locations:** `tools/sync/src/cli.rs` and `tools/capture/src/cli.rs` (identical
  `--log-level` / `--log-file-level` / `--log-file` / `--no-log-file` arg blocks, lines 26–110);
  `src/cli.rs` (same flags with `GIRPR_` env prefix).
- **Duplication:** the clap derive block, three copies, differing only in env-var prefix and
  default value.
- **Proposed crate API:** a `LogArgs` struct in `girpr-log` with `#[command(flatten)]`,
  parameterised by project name for env vars (or three thin wrappers).
- **Risk:** Medium. Clap derive across crate boundaries needs care with `#[arg(env = ...)]`
  (env names are compile-time strings), so the shared struct probably exposes fields and each
  binary sets env via a wrapper, or uses a macro. Behaviour (conflicts, defaults) must be
  preserved exactly; `rw_threads`/`cli_contract` tests cover this.

### M6 — `config.ini` rendering (shared read + minimal write)

- **Locations:** `src/util.rs::read_game_version`, `write_config_ini`; `tools/capture/src/emit/gamedir.rs`
  (`config_ini`, `parse_game_version`, `GENERAL_SECTION`).
- **Duplication:** the INI key set and `game_version` parsing.
- **Proposed crate API:** share `read_game_version`/`parse_game_version` (Low) now; consider
  sharing a *renderer* for the fresh-file case later.
- **Risk:** Medium for sharing the renderer, because main's `write_config_ini` is a surgical
  rewrite (preserves sections, sorts keys, forced keys, passthrough keys) while capture's
  `config_ini` emits a fixed minimal block. They are intentionally different files. Do not
  unify the writers; unify only the parser and the key constants.

---

## 5. High-risk candidates

### H1 — Full `HypClient` / network behaviour

- **Locations:** `src/hyp.rs::HypClient`; `tools/capture/src/hyp/mod.rs::HypClient`.
- **Duplication:** both wrap `reqwest::Client` and hit the same endpoints.
- **Why high risk:** capture's client *records* every request (headers, bodies, timings) into
  the artifact; main's client does not. Unifying them would either lose capture's recording or
  add overhead/recording to the repair binary. The independence rule also forbids `gircap`
  depending on `girpr`.
- **Recommendation:** keep clients separate. Share only types/constants (M1).

### H2 — Full HTTP fetcher (retries, backoff, `Sleep` trait)

- **Locations:** `tools/capture/src/http/mod.rs::Fetcher`, `RetryPolicy`, `Sleep`;
  `src/sophon.rs` (retry loops in `fetch_manifest` / `stream_chunk_to`).
- **Duplication:** retry/backoff logic, with different shapes (capture has injectable `Sleep`
  and `Arc<dyn ...>`; main has inline loops).
- **Why high risk:** main's fetcher is fused with zstd streaming, MD5 hashing on the way past,
  and size ceilings (`HashingWriter`, `ChunkSink::restart`). Replacing it risks chunk-write
  correctness and the download-byte accounting. Capture's `Fetcher` is built for recording.
- **Recommendation:** share only `is_retryable_status` (L4) and `delay_before` if extracted as
  a pure function; keep fetchers separate.

### H3 — Config.ini surgical writer

- **Locations:** `src/util.rs::write_config_ini` (complex, many invariants).
- **Why high risk:** Collapse-parity behaviour (section preservation, duplicate-section merge,
  case-insensitive forced keys, passthrough keys, sorted keys, idempotent rerun) is pinned by
  a large test suite. Reimplementing or sharing it offers little value (only `girpr` writes it).
- **Recommendation:** leave in `girpr`. Capture does not write a real `config.ini` (it renders
  a fixture), so there is no duplication to remove.

### H4 — Report stdout vocabulary / output contract

- **Locations:** `src/report.rs`, `tools/sync/src/report/*`, `tools/capture/src/report.rs`.
- **Why high risk:** the `REPORT`/`PROGRESS`/`SUMMARY`/`PLAN` line shapes and exit codes are
  the documented contract; tests assert exact bytes. Coupling the three tools' output would make
  a change in one force changes in the others, which is exactly what the independence rule
  avoids.
- **Recommendation:** share only pure helpers (JSON escaping) as noted in M4; keep record
  types and stdout vocabulary per project.

### H5 — Manifest fetch + zstd streaming decoder

- **Locations:** `src/sophon.rs::fetch_manifest`, `try_fetch_manifest`, `stream_chunk_to`,
  `HashingWriter`, `ChunkSink`.
- **Why high risk:** this is the core repair data plane: byte ceilings, retry semantics,
  restart-on-retry, MD5-on-the-way-past, and the manifest size clamp. Capture fetches the same
  blobs but only to store them; it does not decode under ceilings.
- **Recommendation:** no sharing. The buffered helpers in L2 are enough.

### H6 — Sharing test infrastructure / mocks

- **Locations:** `tests/common/mod.rs` (main), `tools/capture/tests/common/mod.rs`,
  `tools/sync/tests/common/mod.rs`.
- **Why very high risk:** `gircap` builds its own mock server by design ("No shared fixtures").
  Sharing mocks would couple the tools' test oracles.
- **Recommendation:** keep separate. `girpr-tempdir` is the right granularity for shared test
  helpers (directory naming/cleanup only).

---

## 6. Proposed shared crates (summary)

| New crate | Contents (from candidates above) | Tier |
|---|---|---|
| `girpr-hash` | L1, L2 (helpers only) | Low |
| `girpr-http` | L4, L5, L8 | Low |
| `girpr-glob` | L6 | Low |
| `girpr-config` | L7, plus read-only config.ini helpers | Low/Medium |
| `girpr-hyp` | M1, M2 (types + constants + protobuf; no clients) | Medium |
| `girpr-scope` | M3 | Medium |
| `girpr-report` | M4 (escaping helpers first; record types later) | Medium/High |

Each must be a leaf crate with no dependency on `girpr`, `girsync`, or `gircap`, matching the
precedent set by `girpr-log` and `girpr-tempdir`.

## 7. Recommended landing order

1. **L4** `is_retryable_status` — one function, two identical tests; lowest possible risk.
2. **L1** hex digest helpers — pure, already centralised in capture.
3. **L3** `open_log_file` into `girpr-log` — finishes the log policy crate.
4. **L5** `USER_AGENT`.
5. **L8** `join_url`.
6. **L6** glob compilation (parameterised).
7. **L7** `read_game_version` parser.
8. **L2** zstd/gzip simple helpers.
9. **M1/M2** API types + protobuf (types only) — needs contract tests first.
10. **M3** path scope.
11. **M4/M5/M6** report helpers, CLI log flags, config.ini parser — only if the output/CLI
    contract tests are updated in the same change.
12. Leave H1–H6 and V1–V2 alone unless the independence rule is deliberately relaxed.

## 8. What this report does not claim

- It does not measure performance impact; several items (L1, L2) are primarily about removing
  duplicated spelling, not speed.
- It does not assert that every listed item is worth extracting: some (L8) are a few lines and
  may be better left as-is. They are included for completeness so the choice is explicit.
- It does not change any code. No source files were modified to produce this report.
