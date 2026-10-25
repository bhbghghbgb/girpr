# Refactor plan: `gircap` (structure-only)

**Package:** `gircap` at `tools/capture/` (workspace member). **Scope:** only `gircap`. **Constraint:** no logic, optimization, output, flag, or file-format changes; every step keeps `cargo check -p gircap --all-targets` and `cargo nextest run -p gircap` green.

---

## 0. Baseline (verified)

```
cargo check -p gircap --all-targets   -> Finished (dev profile)
cargo nextest run -p gircap           -> 622 tests run: 622 passed, 0 skipped
```

This is the gate. After every step below, re-run both. If a step fails, revert it (`git`) before continuing. Per `docs/02-plan.md`, one commit per step with the gate result in the message.

**Why each step is safe to verify in isolation:** `cargo nextest` runs each test in its own process and the mock servers bind `127.0.0.1:0` ephemeral ports, so moving code between modules cannot create cross-test interference. All integration tests (`tests/`) import the crate's public paths (`gircap::cli`, `gircap::report`, `gircap::exit`, …), which this plan preserves verbatim.

---

## 1. Current structure summary

**`tools/capture/Cargo.toml`** — package `gircap 0.1.0`, edition 2024. Lib at `src/lib.rs`, bin at `src/main.rs`.

Runtime deps: `clap`, `girpr-log` (`../../crates/log`), `tokio`, `reqwest`, `serde`, `serde_json`, `prost`, `zstd`, `md5`, `sha2`, `flate2`, `tracing`, `tracing-subscriber`, `tracing-appender`, `chrono`. Dev-deps: `serde_json`, `girpr-tempdir` (`../../crates/tempdir`). **No Cargo.toml changes are required by this plan.**

**Public API surface** (`src/lib.rs`): `pub mod` for 17 modules; `pub use exit::ExitCode;`, `pub use girpr_log;`, `pub const VERSION`, `pub const SCHEMA_VERSION`. `src/main.rs` is the parse→tracing→run→records→exit shim.

**Module inventory** (`src/`):

| kind | files | owns |
|------|-------|------|
| flat top-level | `main.rs`, `lib.rs`, `cli.rs`, `exit.rs`, `failure.rs`, `config.rs`, `biz.rs`, `hash.rs`, `run.rs`, `report.rs`, `logging.rs` | CLI surface, exit table, `Failure`, plan resolution, region ids, digests, command dispatch, stdout data plane, stderr tracing |
| `http/` | `mod.rs` (3180 lines), `probe.rs`, `negative.rs` | `Fetcher`, `RequestLog`, `Transport`, `ApiError`, `Kind`, `RequestRecord`, tag probe, negative probes |
| `hyp/` | `mod.rs` (1036 lines), `types.rs` | 4 HoYoPlay endpoints + `getBuild`, captured bodies, DTOs |
| `manifest/` | `mod.rs` (1208 lines), `fetch.rs`, `proto.rs`, `dump.rs`, `diff.rs` | zstd→verify→decode, wire proto, dump, diff/scoring |
| `select/` | `mod.rs` (1312 lines), `classes.rs`, `budget.rs`, `decide.rs`, `require.rs` | candidate pool, six classes, budget admission, decision, `--require-class` |
| `blobs/` | `mod.rs`, `chunks.rs`, `plan.rs`, `props.rs`, `fill.rs` | content-addressed chunk store, retention sets, plan, fill |
| `assemble/` | `mod.rs`, `ranges.rs` | build old/new/patch file from chunks |
| `emit/` | `mod.rs`, `tree.rs`, `fixture.rs`, `gamedir.rs`, `index.rs`, `selfcheck.rs`, `closeout.rs` | bundle writer: API docs, tree, fixture, MANIFEST/checksums, selfcheck, closeout |
| `verify/` | `mod.rs`, `materialize.rs` | offline `verify`, `materialize`, `list` |

Largest files: `http/mod.rs` (3180), `report.rs` (1360), `select/mod.rs` (1312), `manifest/mod.rs` (1208), `config.rs` (1144), `hash.rs` (1043), `hyp/mod.rs` (1036).

**Tests:** `tests/` (8 files + `tests/common/mod.rs` local mock server) plus inline `#[cfg(test)]` in every module. 622 tests, no network, none ignored. No stored fixtures.

Notable cross-crate references the plan must preserve:
- `crate::http::{Fetcher, RequestLog, Transport, ApiError, Kind, RequestLog, …}` — used by `blobs`, `manifest`, `hyp`, `emit`, `run`, `failure`, `verify`, `http/probe.rs`, `http/negative.rs`.
- `crate::biz::percent` — used by `src/report.rs` (line 114). `crate::biz::enc` / `crate::biz::push_pair` — called in code by `src/http/probe.rs` (line 608) and referenced in doc comments.
- `crate::http::now_rfc3339` — used by `src/hyp/mod.rs` tests (line 993) and `http/mod.rs` tests.
- `crate::report::percent_decode` — public, used by `report.rs` tests.

---

## 2. Proposed target structure

```
tools/capture/
├── Cargo.toml            # unchanged (no dependency changes)
├── README.md             # unchanged
├── docs/                 # unchanged (00-spec … 04-bundle-differ)
├── src/
│   ├── main.rs           # bin shim (unchanged)
│   ├── lib.rs            # module map + re-exports; adds `mod encoding;`
│   ├── encoding.rs       # NEW: shared percent-encoding (text + URL)
│   ├── cli/mod.rs        # was cli.rs (clap surface)
│   ├── exit/mod.rs       # was exit.rs (ExitCode + exit_with)
│   ├── failure/mod.rs    # was failure.rs (Failure + summaries)
│   ├── config/mod.rs     # was config.rs (plans + validation)
│   ├── biz/mod.rs        # was biz.rs (ids/bases; `percent` def moved out,
│   │                     #  `pub use encoding::percent` keeps the path)
│   ├── hash/mod.rs       # was hash.rs (digests, gzip rule, zstd helpers)
│   ├── run/mod.rs        # was run.rs (command dispatch)
│   ├── report/mod.rs     # was report.rs (stdout plane; imports encoding)
│   ├── logging/mod.rs    # was logging.rs (tracing + log file)
│   ├── http/
│   │   ├── mod.rs        # module declarations + re-exports only (+ inline tests)
│   │   ├── error.rs      # ApiError
│   │   ├── transport.rs  # Transport, RetryPolicy, Sleep, RealSleep, RecordingSleep, DEFAULT_MAX_RESPONSE_BYTES
│   │   ├── record.rs     # Kind, RequestRecord, ACCEPT, ACCEPT_ENCODING, header_map
│   │   ├── log.rs        # RequestLog, now_rfc3339
│   │   ├── fetcher.rs    # Fetcher, Fetched, Streamed, Destination + private helpers
│   │   ├── probe.rs      # unchanged
│   │   └── negative.rs   # unchanged
│   ├── hyp/, manifest/, select/, blobs/, assemble/, emit/, verify/  # unchanged
├── tests/                # unchanged
├── tools/                # unchanged (zz_probe*, *.py, *.ps1, README — not part of crate)
└── docs/                 # unchanged
```

**Public paths preserved verbatim** (no call-site or integration-test changes needed):
`crate::http::Fetcher`, `crate::http::RequestLog`, `crate::http::Transport`, `crate::http::ApiError`, `crate::http::Kind`, `crate::http::RequestRecord`, `crate::http::now_rfc3339`, `crate::biz::Biz`, `crate::biz::percent`, `crate::biz::enc`, `crate::biz::push_pair`, `crate::report::Record`, `crate::report::percent_decode`, `crate::cli::Cli`, `crate::exit::ExitCode`, `crate::VERSION`, `crate::SCHEMA_VERSION`, and every `tests/` import.

---

## 3. Rules (structure-only)

- No function body is altered; no `async`/`sync` boundary changes; no behavior change.
- No dependency added/removed; no `Cargo.toml` change.
- No public path removed or renamed; every `pub` item stays reachable at the same path (via re-export where its defining file moves).
- No `--flag`, output line, document field, or `SCHEMA_VERSION` change.
- Each step ends with `cargo check -p gircap --all-targets` AND `cargo nextest run -p gircap` green before the next step begins.

---

## 4. Exact ordered steps

### Phase 0 — Baseline (already done; repeat as the gate for every step)

- Step 0.1 `cargo check -p gircap --all-targets` → clean.
- Step 0.2 `cargo nextest run -p gircap` → 622 passed, 0 skipped.

### Phase 1 — Extract shared percent-encoding into `src/encoding.rs`

Today `biz.rs` defines uppercase URL `percent` (used by `enc`/`push_pair`), and `report.rs` defines `percent_encode`/`percent_decode`/`TEXT_ESCAPES` and calls `crate::biz::percent` for non-ASCII text. Two encoders live in two modules; one caller reaches across modules. Extract one shared module.

- **Step 1.1** Create `src/encoding.rs` containing, moved verbatim (no body changes):
  - `const TEXT_ESCAPES`, `fn percent_encode`, `fn percent_decode` (from `report.rs`);
  - `pub fn percent(bytes: &[u8]) -> String` (uppercase URL encoding, from `biz.rs`), using `crate::hash::hex_bytes_upper`.
  - `percent_encode` calls `percent(...)` directly (same module).
- **Step 1.2** Edit `src/biz.rs`:
  - Remove the `pub fn percent` definition and its doc comment.
  - In `enc`, call `percent(...)` after `use crate::encoding::percent;`.
  - Add `pub use crate::encoding::percent;` so `crate::biz::percent` keeps resolving (public path preserved).
- **Step 1.3** Edit `src/report.rs`:
  - Remove `TEXT_ESCAPES`, `percent_encode`, `percent_decode` definitions.
  - Add `use crate::encoding::{self, percent};` and `pub use crate::encoding::percent_decode;` (preserves `crate::report::percent_decode`).
  - `text()` calls `encoding::percent_encode(s)` and `encoding::percent(...)` instead of `crate::biz::percent(...)`.
  - In `src/report.rs`'s `#[cfg(test)] mod tests`, add `use crate::encoding::percent_encode;` (the tests use `percent_encode`; `percent_decode` arrives via `use super::*` from the `pub use`).
- **Step 1.4** Edit `src/lib.rs`: add `mod encoding;` (private; alphabetically between `exit` and `failure`). Do **not** add it to the public module-map doc table (it is not a public API addition).
- **Verification:** `cargo check -p gircap --all-targets`, then `cargo nextest run -p gircap`.

### Phase 2 — Split `src/http/mod.rs` into submodules (public paths preserved via re-exports)

`http/mod.rs` (3180 lines) mixes `ApiError`, `Transport`/retry/sleep, `Kind`/`RequestRecord`, `RequestLog`, and `Fetcher` plus its tests. Split by concern into leaf files; `http/mod.rs` becomes declarations + re-exports. Because every item is re-exported from `mod.rs`, **no `use` statement anywhere else in the crate changes**, and no integration test changes.

- **Step 2.1** Create `src/http/error.rs` with `ApiError` (+ impls), moved verbatim from `http/mod.rs`. In `http/mod.rs`: add `pub mod error;` and `pub use error::ApiError;`.
- **Step 2.2** Create `src/http/transport.rs` with `RetryPolicy`, `SleepFuture`, `Sleep`, `RealSleep`, `RecordingSleep`, `DEFAULT_MAX_RESPONSE_BYTES`, `Transport`. In `http/mod.rs`: `pub mod transport;` + `pub use transport::{RetryPolicy, SleepFuture, Sleep, RealSleep, RecordingSleep, Transport, DEFAULT_MAX_RESPONSE_BYTES};`.
- **Step 2.3** Create `src/http/record.rs` with `Kind`, `RequestRecord`, `ACCEPT`, `ACCEPT_ENCODING`, and `pub(crate) fn header_map`. In `http/mod.rs`: `pub mod record;` + `pub use record::{Kind, RequestRecord, ACCEPT, ACCEPT_ENCODING};`.
- **Step 2.4** Create `src/http/log.rs` with `RequestLog` (+ `LogState`) and `pub fn now_rfc3339`. In `http/mod.rs`: `pub mod log;` + `pub use log::{RequestLog, now_rfc3339};`.
- **Step 2.5** Create `src/http/fetcher.rs` with `Fetcher`, `Fetched`, `Streamed`, `Destination`, `Run` and the private helpers `drain_capped`, `body_failure`, `is_read_failure`, `too_big`, `shared_req_headers`, `client_default_headers`, `get_json`. In `http/mod.rs`: `pub mod fetcher;` + `pub use fetcher::{Fetcher, Fetched, Streamed, Destination};` + `pub(crate) use fetcher::{body_failure, shared_req_headers, client_default_headers};` (the last three are needed by the inline tests via `use super::*`).
- **Step 2.6** Slim `http/mod.rs` to: the module doc comment, the `pub mod` declarations, the public `pub use` re-exports, the `pub(crate) use` test helpers, `use crate::exit::ExitCode;` (tests use it), and the imports the inline tests still need via `use super::*` (`std::collections::BTreeMap`, `std::time::{Duration, Instant}`, `std::sync::{Arc, Mutex}`); drop `std::fmt`, `std::path::Path`, `serde::Serialize` (now used only by moved code, which carries its own imports — keep any of these if unsure; unused imports warn but do not fail `cargo check`/`cargo nextest`). Keep the existing `#[cfg(test)] mod tests { use super::*; … }` block unchanged.
  - `http/probe.rs` and `http/negative.rs` need no edits: their `crate::http::{ApiError, Fetcher, Kind, Observation}` paths still resolve through the re-exports.
- **Verification after each of Steps 2.1–2.6:** `cargo check -p gircap --all-targets`, then `cargo nextest run -p gircap`.

### Phase 3 — Group flat library modules into directories (pure file moves, zero code change)

Rust resolves `pub mod X;` to either `src/X.rs` or `src/X/mod.rs`, and all call sites use paths like `crate::X::…`, so moving a file into `src/X/mod.rs` changes **no code, no `mod` declaration, and no `use` statement**. These 9 steps are independent and order-flexible; verify after each (or after any batch).

For each `X` in `[cli, exit, failure, config, biz, hash, run, report, logging]`:

- **Step 3.X** `mkdir -p src/X` and `git mv src/X.rs src/X/mod.rs` (delete the old `.rs`, create the `mod.rs`). No edit to `src/lib.rs`; no edit to any `use` site.
  - Note: after Step 1, `biz/mod.rs` contains `pub use crate::encoding::percent;` and `report/mod.rs` imports `crate::encoding` — both still resolve because `mod encoding;` is declared at crate root.
- **Verification:** after each move, `cargo check -p gircap --all-targets`, then `cargo nextest run -p gircap`.

### Phase 4 — Finish

- **Step 4.1** Final gate: `cargo check -p gircap --all-targets` and `cargo nextest run -p gircap` (expect 622 passed, 0 skipped — identical to baseline).
- **Step 4.2 (recommended, docs-only)** Update the module tree in `tools/capture/docs/02-plan.md` §2 to match the new layout, and optionally add `encoding` to `src/lib.rs`'s module-map table only if you treat it as public. This does not affect compilation or tests.

---

## 5. Verification commands (every step)

```powershell
# from the workspace root E:\hoccode\hocrust\girpr
cargo check -p gircap --all-targets
cargo nextest run -p gircap
```

`--all-targets` compiles lib, bin, and integration tests, which is what catches a broken public path. `cargo nextest run -p gircap` runs all 622 tests in isolated processes. Baseline: 622 passed, 0 skipped; every step must end at the same result.

**Rollback:** each step is a single commit (per `docs/02-plan.md` discipline). If `cargo check` or `cargo nextest` fails at a step, `git reset --hard` that step's commit and re-attempt before proceeding.

---

### What this plan deliberately does NOT do (out of scope)

- Does not merge `run.rs` helpers (`manifest_stats`, `selection_fields`, `satisfied`, `covered_ids`, `selfcheck_slug`) into their owning modules — that would change call sites and is a deeper cohesion refactor.
- Does not split `report.rs`, `config.rs`, `hash.rs`, or `manifest/mod.rs` further — they are large but coherent, and splitting them has no clear behavioral boundary without risking call-site churn.
- Does not touch `src/main.rs`, `Cargo.toml`, `tests/`, `tools/`, or `docs/` (except the optional Step 4.2 doc refresh).
- Does not change any `pub` path, flag, output line, document field, or `SCHEMA_VERSION`.
