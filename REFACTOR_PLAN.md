# Refactor Plan: `girpr` (structure-only)

**Repository:** `E:\hoccode\hocrust\girpr` (branch `all/refactor`)
**Scope:** ONLY the `girpr` crate (workspace root). `crates/log`, `crates/tempdir`, `tools/sync` (`girsync`), and `tools/capture` (`gircap`) are untouched — confirmed: neither `girsync` nor `gircap` depends on `girpr` (they depend only on `girpr-log`/`girpr-tempdir`), so preserving `girpr`'s public API paths keeps the whole workspace building.
**Test runner:** `cargo nextest`. Baseline verified: `cargo check -p girpr --all-targets` is green.
**Rules:** no logic changes, no optimizations, no changes to public behavior/outputs/flags/file formats. Every step must compile (`cargo check -p girpr --all-targets`) and pass (`cargo nextest run -p girpr`) before the next step begins.

---

## 1. Current structure summary

### Crate layout
```
girpr (workspace root, package "girpr")
├── Cargo.toml              bin "girpr" (src/main.rs) + lib (src/lib.rs);
│                           workspace members: ".", "crates/log", "crates/tempdir",
│                           "tools/sync", "tools/capture"
├── src/
│   ├── main.rs             thin entry: parse args → tracing → repair::run → report::emit → exit
│   ├── lib.rs              module map + public API (see below)
│   ├── cli.rs              Args + Args::into_ctx + audio parsing
│   ├── config.rs           RunCtx, RunFailure, RepairMode
│   ├── hyp.rs              HypClient + HoYoPlay/Sophon API types
│   ├── sophon.rs           protobuf messages, streaming chunk/manifest fetch, retry policy,
│   │                       local chunk-reuse map
│   ├── biz.rs              Biz enum + endpoints/channel tuples
│   ├── report.rs           Record, Summary, SummaryFlags, text/JSON rendering
│   ├── logging.rs          init_tracing
│   ├── sleep.rs            Sleep trait, RealSleep, NoSleep
│   ├── util.rs             MD5, scan_file/scan_reader, config.ini, ignore/blacklist, normalize_rel
│   └── repair/             the pipeline; run() coordinator + phases
│       ├── mod.rs          run() (phase coordinator, ~537 lines)
│       ├── audio.rs        audio scan file read/write/normalize
│       ├── check.rs        --check-only verify_plan
│       ├── file.rs         per-file repair (1622 lines): repair_all, repair_one_file,
│       │                   attempt_inplace/attempt_temp, tiering, reuse, downloads, promotion
│       ├── lock.rs         single-writer girpr.lock
│       ├── manifest.rs     manifest fetch + local reuse map
│       ├── metadata.rs     local + server metadata (docs/02 steps 1–2), audio resolution
│       ├── post.rs         post-phase (deprecated, audio cache→res, purge-after, config.ini bump)
│       ├── reuse.rs        salvage order (ascending/descending)
│       ├── scan.rs         plan-phase --scan (ScanReport)
│       ├── scope/          scope of work: filter / limits / ledger / report (+ PathScope)
│       │   ├── filter.rs   PathScope (--include/--exclude)
│       │   ├── limits.rs   Limits ceilings + pre-flight refusal checks
│       │   ├── ledger.rs   Meter + StoppedBy runtime ledger
│       │   └── report.rs   PLAN line + --plan-out document
│       └── plan/           the work list
│           ├── mod.rs      RepairPlan, PlannedFile, Layout, build_plan
│           └── purge.rs    Collapse-style files-cleanup (plan_purge/apply_purge/classify_purge_path)
```

### Public API (from `src/lib.rs`)
```rust
pub mod biz; pub mod cli; pub mod config; pub mod hyp; pub mod logging;
pub mod repair; pub mod report; pub mod sleep; pub mod sophon; pub mod util;
pub use girpr_log;
pub use biz::Biz;
pub use cli::{Args, is_audio_none, normalize_audio_lang};
pub use config::{RunCtx, RunFailure};
pub use girpr_log::Level;
pub use repair::run;
pub use report::{OutputFormat, Summary};
```

### Tests (`tests/`)
| file | what it covers |
|---|---|
| `tests/repair_offline.rs` | offline integration/e2e vs mock server; library API (`repair::run`) + binary spawns |
| `tests/repair_machinery.rs` | repair decisions on crafted fixtures; runs the real binary |
| `tests/cli_contract.rs` | binary-level arg/exit-code contract |
| `tests/artifact_server.rs` | replay-server contract vs synthetic mini-bundle |
| `tests/artifact_bridge.rs` | cross-tool claim vs real gircap bundles (`#[ignore]`, expensive, run by hand) |
| `tests/peak_heap.rs` | heap measurement (`#[ignore]`) |
| `tests/common/mod.rs` | shared mock HTTP server + fixtures (`#[path]` from machinery/offline) |
| `tests/artifact_bridge/{server,bundle}.rs` | `#[path]` modules of artifact_bridge |
| `tests/scripts/io-profile/{iotime,iot-bench}` | separate bench crates, not girpr tests |

### Public paths that tests name directly (must keep resolving)
`girpr::repair::scope::{Limits, PathScope}`, `girpr::sleep::{NoSleep, RealSleep}`, `girpr::config::RepairMode`, `girpr::report::{OutputFormat, summary_record}`, `girpr::{Biz, RunCtx, repair, OutputFormat}`, `girpr::repair::lock::Lock`, `girpr::sophon::{SophonChunk, SophonChunkFile, SophonChunkManifest}`, `girpr::cli::Args`.

### Intra-crate paths that will change (all use-site updates are itemized in Step 3)
- `crate::repair::plan::{RepairPlan, PlannedFile}` → `crate::repair::phases::plan::{…}`
- `crate::repair::plan::{PurgePlan, PurgeCandidate}` → `crate::repair::purge::{…}`
- `crate::repair::manifest::LocalChunkMap` → `crate::repair::phases::manifest::LocalChunkMap`
- `crate::repair::plan::purge` → `crate::repair::purge`
- `crate::repair::file::{Mode, resolve_mode}` → unchanged path (re-exported from `file/mod.rs`)

---

## 2. Proposed target structure

```
src/main.rs                 unchanged
src/lib.rs                  unchanged public re-exports; module-map doc updated
src/cli.rs                  unchanged
src/config.rs               unchanged
src/hyp.rs                  unchanged
src/sophon.rs               unchanged
src/biz.rs                  unchanged
src/report.rs               unchanged
src/logging.rs              unchanged
src/sleep.rs                unchanged
src/util.rs                 unchanged
src/repair/mod.rs           run() coordinator unchanged (logic identical);
                            phases + purge + Meta/PlannedFile/… re-exports
src/repair/phases/mod.rs    NEW — doc-only phase-order table;
                            pub mod check; manifest; metadata; plan; post;
src/repair/phases/metadata.rs   moved from repair/metadata.rs (content unchanged)
src/repair/phases/manifest.rs   moved from repair/manifest.rs (content unchanged)
src/repair/phases/plan.rs       moved from repair/plan/mod.rs (content unchanged;
                            `pub mod purge` + its re-export removed)
src/repair/phases/check.rs      moved from repair/check.rs (content unchanged)
src/repair/phases/post.rs       moved from repair/post.rs (content unchanged)
src/repair/purge.rs             moved from repair/plan/purge.rs (content unchanged)
src/repair/file/mod.rs        moved from repair/file.rs: repair_all, repair_one_file,
                            FileCtx, Repaired, ChunkStats, FileWrites, Mode,
                            resolve_mode (pub(super)), Inplace, shared write
                            primitives (note_write/write_at/copy_within/zero_fill/
                            reserve_write), unit tests
src/repair/file/mode.rs       NEW: open_inplace, open_rw, clear_readonly
src/repair/file/inplace.rs    NEW: attempt_inplace, attempt_inplace_inner,
                            trim_is_free, RamReservation, Tier
src/repair/file/temp.rs       NEW: attempt_temp, repair_attempt, repair_attempt_inner,
                            try_reuse_slice, rollback_tmp, promote_tmp,
                            write_downloaded_chunk (pub(super)), ChunkFileSink,
                            ChunkOutcome
src/repair/audio.rs           unchanged
src/repair/lock.rs            unchanged
src/repair/scan.rs            unchanged
src/repair/reuse.rs           unchanged
src/repair/scope/             unchanged (filter, limits, ledger, report)
```

### Rationale
- `repair/mod.rs` = phase coordinator only; each docs/02 step becomes its own module under `repair/phases/`.
- `purge` is promoted out of `plan`: the files-cleanup (docs/02 steps 5 & 7) is a peer of the work list (step 4), not a child of it.
- `file.rs` (1622 lines) is split by responsibility: mode resolution, in-place patch, temp path, shared write primitives.
- Top-level modules (`cli`, `config`, `hyp`, `sophon`, `biz`, `report`, `logging`, `sleep`, `util`) are small and focused; intentionally left flat.
- Every moved item keeps its visibility; all public paths survive through `pub use` re-exports.

### Public API preservation table

| path | before | after |
|---|---|---|
| `girpr::repair::run` | `pub use repair::run` (fn in mod.rs) | unchanged |
| `girpr::repair::Meta` | `pub use metadata::Meta` | `pub use phases::metadata::Meta` |
| `girpr::repair::PathScope` | `pub use scope::PathScope` | unchanged |
| `girpr::repair::PlannedFile, RepairPlan` | `pub use plan::{PlannedFile, RepairPlan}` | `pub use phases::plan::{PlannedFile, RepairPlan}` |
| `girpr::repair::PurgeCandidate, PurgePlan, PurgeVerdict` | `pub use plan::{PurgeCandidate, PurgePlan}`; `pub use plan::purge::PurgeVerdict` | `pub use purge::{PurgeCandidate, PurgePlan, PurgeVerdict}` |
| `girpr::repair::file::Mode`, `resolve_mode` | `pub` / `pub(super)` in file.rs | `pub` / `pub(super)` in file/mod.rs (still visible to `repair::scope::report`) |
| `girpr::repair::lock::Lock` | `pub mod lock` | unchanged |
| `girpr::repair::{scan, reuse, audio}` | `pub mod` | unchanged |

No `Cargo.toml` member/path/dependency changes are required anywhere.

---

## 3. Exact ordered steps

Run from `E:\hoccode\hocrust\girpr`. The gate after **every** substep:

```
cargo check -p girpr --all-targets
cargo nextest run -p girpr
```

If a step fails to compile or a test regresses, `git restore`/`git rm` the files touched in that step and do not proceed.

### Step 0 — Baseline (do first, once)
Record the passing test count from `cargo nextest run -p girpr`; compare against it after every later step.

### Step 1 — Create `repair/phases/` skeleton and move `metadata`

1a. **Create** `src/repair/phases/mod.rs`:
```
//! The run's phases: metadata → manifests → plan → check → post.
//!
//! [`crate::repair::run`] (in `src/repair/mod.rs`) owns phase order; each
//! phase here owns its own step of docs/02. No phase imports another phase.
//!
//! | phase | module | docs/02 |
//! |---|---|---|
//! | local state + server metadata | `metadata` | steps 1–2 |
//! | manifests + local chunk map | `manifest` | step 3 |
//! | work list + purge candidates | `plan` | steps 4, 5, 7 |
//! | `--check-only` verify | `check` | step 6.5 |
//! | post-phase | `post` | step 7 |

pub mod check;
pub mod manifest;
pub mod metadata;
pub mod plan;
pub mod post;
```

1b. **Create** `src/repair/phases/metadata.rs` = byte copy of `src/repair/metadata.rs`, with exactly one use change:
- `use super::audio;` → `use crate::repair::audio;`
(all other `crate::…` paths are absolute and unchanged).

1c. **Edit** `src/repair/mod.rs`:
- Remove the line `    mod metadata;`
- Change the `pub use` block:
```
//! old:
    pub use metadata::Meta;
//! new:
    pub use phases::metadata::Meta;
```
- Add `pub mod phases;` next to the other `pub mod` declarations (e.g. after `pub mod scope;`).
- Inside `run()`, no change yet: `metadata::fetch(...)` resolves once `use phases::metadata;` exists — add:
```
use phases::{metadata};   // temporary single import; grows in later steps
```
placed with the other `use crate::…` imports near the top of mod.rs (line ~63 area).

1d. **Delete** `src/repair/metadata.rs`.

1e. **Verify** (`cargo check` + `cargo nextest run -p girpr`).

> Why this compiles mid-refactor: `repair/mod.rs` still declares `mod check; mod manifest; mod plan; mod post;`, so the not-yet-moved modules stay at their old paths; only `metadata` has a new home.

### Step 2 — Move `manifest`

2a. **Create** `src/repair/phases/manifest.rs` = byte copy of `src/repair/manifest.rs`, with one use change:
- `use super::RunFailure;` → `use crate::config::RunFailure;`

2b. **Edit** `src/repair/mod.rs`:
- Remove `    mod manifest;`
- Extend the phase import: `use phases::{metadata, manifest};`

2c. **Delete** `src/repair/manifest.rs`.

2d. **Verify**.

### Step 3 — Move `plan`, promote `purge` out of `plan` (biggest use-site ripple)

3a. **Create** `src/repair/phases/plan.rs` = byte copy of `src/repair/plan/mod.rs`:
- Delete line `pub mod purge;`
- Delete line `pub use purge::{PurgeCandidate, PurgePlan};`
(everything else, including the `#[cfg(test)] mod tests`, is unchanged; `use super::manifest::LocalChunkMap;` and `use super::scope::PathScope;` still resolve from `phases/`).

3b. **Create** `src/repair/purge.rs` = byte copy of `src/repair/plan/purge.rs`, with use changes:
- `use super::RepairPlan;` → `use crate::repair::phases::plan::RepairPlan;`
- in its test mod: `use crate::repair::plan::PlannedFile;` → `use crate::repair::phases::plan::PlannedFile;`

3c. **Edit** `src/repair/mod.rs`:
- Remove `    mod plan;`
- Extend phase import: `use phases::{metadata, manifest, plan};`
- Replace the public re-export block:
```
//! old:
    pub use metadata::Meta;
    pub use plan::purge::PurgeVerdict;
    pub use plan::{PlannedFile, PurgeCandidate, PurgePlan, RepairPlan};
    pub use scope::PathScope;
//! new:
    pub use phases::metadata::Meta;
    pub use phases::plan::{PlannedFile, RepairPlan};
    pub use purge::{PurgeCandidate, PurgePlan, PurgeVerdict};
    pub use scope::PathScope;
```
- Inside `run()`, update call sites:
  - `plan::build_plan(...)` → `phases::plan::build_plan(...)`
  - `plan::purge::plan_purge(...)` → `purge::plan_purge(...)`
  - `plan::purge::keep_set()` → `purge::keep_set()`

3d. **Edit external use sites** (all `crate::repair::plan::…` references outside `repair/mod.rs`):
- `src/repair/scan.rs:30`: `use crate::repair::plan::{PlannedFile, RepairPlan};` → `use crate::repair::phases::plan::{PlannedFile, RepairPlan};`
- `src/repair/scope/limits.rs:26`: `use crate::repair::plan::{PurgePlan, RepairPlan};` →
  `use crate::repair::purge::PurgePlan; use crate::repair::phases::plan::RepairPlan;`
- `src/repair/scope/limits.rs:187` (test mod): `use crate::repair::plan::{PlannedFile, PurgeCandidate};` → `use crate::repair::phases::plan::PlannedFile; use crate::repair::purge::PurgeCandidate;`
- `src/repair/scope/report.rs:30`: `use crate::repair::plan::{PurgePlan, RepairPlan};` → `use crate::repair::purge::PurgePlan; use crate::repair::phases::plan::RepairPlan;`
- `src/repair/scope/report.rs:310` (test mod): `use crate::repair::plan::{PlannedFile, PurgeCandidate};` → same split as limits.rs.

3e. **Delete** `src/repair/plan/mod.rs`, `src/repair/plan/purge.rs`, and the now-empty `src/repair/plan/` directory.

3f. **Verify**.

### Step 4 — Move `check`

4a. **Create** `src/repair/phases/check.rs` = byte copy of `src/repair/check.rs` (no use changes needed: from `phases/`, `super::plan` resolves to `phases::plan` and `super::scan` to `repair::scan`).

4b. **Edit** `src/repair/mod.rs`:
- Remove `    mod check;`
- Extend phase import: `use phases::{metadata, manifest, plan, check};`
- Inside `run()`: `check::verify_plan(...)` → `phases::check::verify_plan(...)`

4c. **Delete** `src/repair/check.rs`.

4d. **Verify**.

### Step 5 — Move `post`

5a. **Create** `src/repair/phases/post.rs` = byte copy of `src/repair/post.rs`, with one use change:
- `use super::plan::{RepairPlan, purge};` → `use super::plan::RepairPlan; use crate::repair::purge;`
(`use super::metadata::Meta;` and `use super::scope::PathScope;` still resolve from `phases/`/`repair/`.)

5b. **Edit** `src/repair/mod.rs`:
- Remove `    mod post;`
- Extend phase import: `use phases::{metadata, manifest, plan, check, post};`
- Inside `run()`: `post::run(...)` → `phases::post::run(...)` and `post::PostOutcome` → `phases::post::PostOutcome`

5c. **Delete** `src/repair/post.rs`.

5d. **Verify**.

At this point `src/repair/mod.rs` contains `mod audio; mod file; pub mod lock; pub mod phases; pub mod purge; pub mod reuse; pub mod scan; pub mod scope;` plus the `use phases::{…}` import and the `pub use …` re-export block shown in Step 3c. `src/repair/phases/mod.rs` declares exactly `{check, manifest, metadata, plan, post}`.

### Step 6 — Split `repair/file.rs` into `repair/file/`

6a. **Create** `src/repair/file/mode.rs`:
- `open_inplace`, `open_rw`, `clear_readonly` (moved from file.rs). `open_inplace` marked `pub(super)` so `inplace.rs` (a sibling under `file`) can call it via `super::mode::open_inplace`; `open_rw`/`clear_readonly` stay private (used only within mode.rs).
- No dependency on `file/mod.rs` items (purely `std` + `crate::util` + `anyhow` + `tracing`).

6b. **Create** `src/repair/file/temp.rs`:
- `attempt_temp`, `repair_attempt`, `repair_attempt_inner`, `try_reuse_slice`, `rollback_tmp`, `promote_tmp`, `ChunkFileSink`, `ChunkOutcome` (moved from file.rs).
- `write_downloaded_chunk` marked `pub(super)` so `inplace.rs` can call it via `super::temp::write_downloaded_chunk`.
- Uses `super::{ChunkStats, FileWrites, note_write, reserve_write}` and crate paths for `StoppedBy`, `Summary`, `Sleep`, `sophon`, `util`.

6c. **Create** `src/repair/file/inplace.rs`:
- `attempt_inplace`, `attempt_inplace_inner`, `trim_is_free`, `RamReservation`, `Tier` (moved from file.rs).
- Imports from `super`: `ChunkStats`, `FileWrites`, `Inplace`, `Mode`, `resolve_mode`, `note_write`, `write_at`, `copy_within`, `zero_fill`, `reserve_write`, plus `super::mode::open_inplace` and `super::temp::write_downloaded_chunk`.
- Crate paths for `reuse`, `sophon::SophonChunk`, `util`, and `crate::repair::phases::plan::{PlannedFile, RepairPlan}`.

6d. **Create** `src/repair/file/mod.rs` = byte copy of `src/repair/file.rs` with these edits:
- Use-statement updates:
  - `use super::manifest::LocalChunkMap;` → `use crate::repair::phases::manifest::LocalChunkMap;`
  - `use super::plan::{PlannedFile, RepairPlan};` → `use crate::repair::phases::plan::{PlannedFile, RepairPlan};`
- Module declarations: add `mod inplace; mod mode; mod temp;` (keep `mod tests;` at the bottom).
- Remove from mod.rs the definitions that now live in submodules: `open_inplace`/`open_rw`/`clear_readonly` (→ mode.rs), `attempt_inplace`/`attempt_inplace_inner`/`trim_is_free`/`RamReservation`/`Tier` (→ inplace.rs), `attempt_temp`/`repair_attempt`/`repair_attempt_inner`/`try_reuse_slice`/`rollback_tmp`/`promote_tmp`/`write_downloaded_chunk`/`ChunkFileSink`/`ChunkOutcome` (→ temp.rs).
- Keep in mod.rs: `repair_all`, `repair_one_file`, `FileCtx`, `Repaired`, `ChunkStats`, `FileWrites`, `Mode`, `resolve_mode` (`pub(super)`), `Inplace`, `MAX_ATTEMPTS`, `COPY_BUF`, and the shared write primitives `note_write`, `write_at`, `copy_within`, `zero_fill`, `reserve_write` (private; children reach them via `super::`).
- In `repair_one_file`: call `inplace::attempt_inplace(...)` and `temp::attempt_temp(...)` instead of `attempt_inplace`/`attempt_temp`; match on `Inplace::*` (defined in mod.rs).
- Add a private re-export so the existing unit test keeps compiling: `use self::inplace::trim_is_free;` (the test mod does `use super::*;` and calls `trim_is_free(...)`).

6e. **Delete** `src/repair/file.rs`.

6f. **Verify**. (`repair/mod.rs` needs no edit: `mod file;` and `file::repair_all(...)` still resolve — `file/mod.rs` keeps `pub async fn repair_all`.)

### Step 7 — Final verification

```
cargo check -p girpr --all-targets
cargo nextest run -p girpr
cargo check --workspace          # confirm girsync/gircap still build (untouched)
```

Expected: identical test pass count to baseline; zero compile errors; zero changed test expectations.

---

## 4. Verification at each step

| step | command | what it proves |
|---|---|---|
| every step | `cargo check -p girpr --all-targets` | the crate and all its test targets (including `#[ignore]`d `artifact_bridge`/`peak_heap`) compile after the move/use edits |
| every step | `cargo nextest run -p girpr` | every non-ignored test passes; pass count equals baseline |
| step 3 | also grep `crate::repair::plan::` / `repair::plan::purge` | no stale path references remain outside the re-export block |
| step 6 | `cargo doc -p girpr --no-deps` (optional) | public docs still build; `girpr::repair::file::{Mode, resolve_mode}` still reachable |
| final | `cargo check --workspace` | sibling dev tools (`girsync`, `gircap`) unaffected |

The single combined gate `cargo check -p girpr --all-targets && cargo nextest run -p girpr` is run after each step; a step is complete only when both succeed.

---

## Notes / constraints honored

- **No `Cargo.toml` changes:** no new dependencies, no member or path changes; the workspace member list stays `".", "crates/log", "crates/tempdir", "tools/sync", "tools/capture"`.
- **No logic changes:** moved files are byte copies except for `use`-statement path fixes required by the new location; `run()`'s phase order, the `file.rs` repair logic, and all `pub` item signatures are identical.
- **Public API preserved:** every path tests and other crates name (`girpr::repair::run`, `Meta`, `PathScope`, `PlannedFile`, `Purge*`, `RepairPlan`, `Mode`, `resolve_mode`, `lock::Lock`, `cli::Args`, `report::{OutputFormat, summary_record}`, `config::{RunCtx, RunFailure, RepairMode}`, `sleep::{NoSleep, RealSleep}`, `sophon::{…}`) resolves after the refactor via re-exports or unchanged locations.
- **Tests reference only public paths:** `tests/common/mod.rs` uses `girpr::sophon::{…}` (unchanged); `tests/cli_contract.rs` uses `girpr::cli::Args` (unchanged); `tests/repair_*.rs` use `girpr::repair::scope`, `girpr::repair::lock`, `girpr::config`, `girpr::report`, `girpr::sleep`, `girpr::{Biz, RunCtx, repair}` — all unchanged.
- **Doc line reference:** `tests/artifact_bridge.rs` mentions `src/repair/mod.rs 4.2`; the empty-`--include` guard it names stays in `repair/mod.rs`, so the reference remains accurate — no doc edits required. `docs/` prose references to `src/repair/…` paths are not compiled and are out of scope for this structure-only change.
- **Rollback:** each step touches a small, well-defined set of files; `git status`/`git diff` shows exactly what moved, and any failed step is reverted before continuing.
