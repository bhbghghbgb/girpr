## Refactor Plan: girsync (structure-only)

**Scope:** crate `girsync` at `E:\hoccode\hocrust\girpr/tools/sync` only. Workspace root `girpr`, siblings `girpr` and `gircap` are out of scope.

---

### 1. Current structure summary

**Crate metadata** (`tools/sync/Cargo.toml`):
- Package `girsync`, edition 2024, dev-only one-way folder mirror with redb hash cache.
- Lib target: `src/lib.rs` (crate name `girsync`).
- Bin targets: `girsync` (`src/main.rs`), `sled2redb` (`src/bin/sled2redb.rs`).
- Runtime deps: clap, girpr-log, anyhow, walkdir, sled, serde, serde_json, md5, sha2, glob, chrono, rayon, tracing, tracing-subscriber, tracing-appender, redb.
- Dev-deps: md5, serde_json, clap, girpr-tempdir.
- **No `examples/` content** (directory is empty).

**Source layout (`src/`):**

Top-level single-file modules:
- `lib.rs` — module tree + public re-exports (`pub mod cache; cli; commands; config; convert; diff; effective; filter; hash; logging; planner; report; rw; scan; util;`, `pub use girpr_log;`, plus item re-exports).
- `main.rs` — binary entry, exit codes 0/3/4, `run(cli, plan)`.
- `cli.rs` — `Cli`, `Cmd`, `CommonArgs`, `RwArgs`, `TrustArgs`, `TrustSide` (~386 lines).
- `convert.rs` — `sled_to_redb`, `Converted` (~146 lines; only sled user).
- `filter.rs` — glob include/exclude (~35 lines).
- `hash.rs` — **`HashAlg`, `UnknownHash`, `AlgoSet`, `DigestSet`, `Digests`, `hash_file`, `CHUNK`, `parse_hash_list`, `parse_any_of` (~1050 lines; four concerns in one file).**
- `logging.rs` — `LogGuard`, `init_tracing`, `plan`, `PROJECT`, `default_log_path` (~249 lines).
- `scan.rs` — `walk_live`, `check_mixed_case`, `LiveEnt` (~193 lines).
- `util.rs` — `elapsed_s`, `ts_now`, `unique_sibling`, `copy_dir_all`, `is_record_path`, `is_cache_rel`, `mtime_ns_of` (~79 lines).

Directory modules (facade `mod.rs` + leaves):
- `cache/` — `mod.rs`, `schema.rs`, `codec.rs`, `db.rs` (**644 lines**), `open.rs`, `write.rs`.
- `commands/` — `mod.rs`, `compare.rs`, `compare_self.rs`, `update.rs`, `sync/` (`mod.rs` ~396 lines, `apply.rs` **~824 lines**, `plan.rs` ~337 lines, `rename.rs` ~268 lines).
- `config/` — `mod.rs`, `common.rs`.
- `diff/` — `mod.rs` (~534 lines), `verdict.rs`.
- `effective/` — `mod.rs`, `entry.rs` (~502 lines), `phase_a.rs`, `phase_c.rs`, `record.rs`, `side.rs` (~286 lines).
- `planner/` — `mod.rs`, `plan.rs` (~329 lines), `pairs.rs` (~597 lines), `stat_trust.rs`, `coverage.rs`, `fixtures.rs` (test-only).
- `report/` — `mod.rs` (~286 lines), `record.rs`.
- `rw/` — `mod.rs` (~267 lines), `gate.rs`, `limits.rs`, `fixtures.rs` (test-only).

**Public API** (re-exported from `lib.rs`, consumed by `tests/`): `Cli`, `Cmd`, `run`, `CommonOpts`/`CompareOpts`/`CompareSelfOpts`/`SyncOpts`/`UpdateOpts`/`LogCtx`/`ScanMode`, `EffRec`, `ScanStats`, `SideCapability`, `SideEntry`, `SideScan`, `OutputFormat`, `Record`, `Report`, `verdict`, `HashPlan`, `Required`, `RwLimits`, `RwPermit`, `RwRuntime`, `RwSide`, `LogGuard`, `init_tracing`, `plan`, `Level`, `girpr_log`. Tests also import `girsync::hash::{HashAlg, AlgoSet, DigestSet, parse_hash_list, hash_file}` and `girsync::cache::{CacheDb, TestCacheBackend, …}`.

**Tests** (`tests/`): 326 tests total (1 skipped) per baseline `cargo nextest run -p girsync`. Integration binaries: `cli_dispatch.rs`, `compare.rs`, `compare_self.rs`, `convert.rs`, `helpers.rs`, `migration_lock.rs`, `output.rs`, `sync.rs`, `update.rs`, plus grouped suites `hash_mode/`, `lazy/`, `rw_threads/`, `stat_trust/`, `sync_plan/`, `why/` (each with its own `main.rs`), and shared `tests/common/mod.rs`. No `.cargo/config`, no `nextest.toml`.

Baseline verified: `cargo nextest run -p girsync` → 326 passed, 1 skipped. `cargo check -p girsync --all-targets` passes.

---

### 2. Proposed target structure

The public API, CLI flags, outputs, file formats, and `Cargo.toml` are unchanged. Three oversized single-file modules are split into directory modules whose `mod.rs` is a facade re-exporting every `pub` item, so all `crate::hash::X`, `crate::cache::CacheDb`, `crate::cache::TestCacheBackend`, `sync::apply::Applier` paths keep resolving.

```
src/
  lib.rs                 (unchanged declarations; doc module-map updated)
  main.rs                (unchanged)
  cli.rs                 (unchanged)
  convert.rs             (unchanged)
  filter.rs              (unchanged)
  hash/                  [NEW directory, replaces hash.rs]
    mod.rs               facade: `pub use algo::{HashAlg, UnknownHash};`
                         `pub use digestset::DigestSet;`
                         `pub use set::{AlgoSet, parse_hash_list, parse_any_of};`
                         `pub use stream::{CHUNK, Digests, hash_file};`
    algo.rs              HashAlg + UnknownHash (+ all impls)
    set.rs               AlgoSet (+ FromIterator), parse_hash_list, parse_any_of
    digestset.rs         DigestSet (+ all impls)
    stream.rs            CHUNK, Digests, hash_file
    tests.rs             [moved #[cfg(test)] mod tests body]
  logging.rs             (unchanged)
  scan.rs                (unchanged)
  util.rs                (unchanged)
  cache/
    mod.rs               (unchanged re-exports; `db` stays a module)
    schema.rs            (unchanged)
    codec.rs             (unchanged)
    db/                  [NEW directory, replaces db.rs]
      mod.rs             facade: `pub use backend::TestCacheBackend;`
                         `pub use cachedb::CacheDb;`
                         `pub(super) use backend::redb_builder;`
      backend.rs         CACHE_SIZE, redb_builder, memory_registry, TestCacheBackend
      handle.rs          Handle enum + ReadableDatabase impl
      cachedb.rs         CacheDb impl (all methods)
      tests.rs           [moved #[cfg(test)] mod tests body; fixed use paths]
    open.rs              (unchanged)
    write.rs             (unchanged)
  commands/
    mod.rs               (unchanged)
    compare.rs           (unchanged)
    compare_self.rs      (unchanged)
    update.rs            (unchanged)
    sync/
      mod.rs             (unchanged; `use apply::Applier;` still resolves)
      apply/             [NEW directory, replaces apply.rs]
        mod.rs           Applier + Applied + impl {apply, mkdirs, fix_dirs, predrop_cache_entries}
        phases.rs        impl Applier {delete_extras, copy_files, remove_unknown_dirs, prune_dst_cache}
        copy.rs          copy_one (pub(super)) + copy_and_digest, verify_digests, test_no_fsync + #[cfg(test)] mod tests
        [apply.rs deleted]
      plan.rs            (unchanged; `use super::apply::Applied;` still resolves)
      rename.rs          (unchanged)
  config/                (unchanged)
  diff/                  (unchanged)
  effective/             (unchanged)
  planner/               (unchanged)
  report/                (unchanged)
  rw/                    (unchanged)

tests/                   (NO changes required — all public paths preserved)
Cargo.toml               (NO changes required — no new targets or deps)
```

---

### 3. Guiding rules (invariants to preserve)

1. **Structure-only:** no function body, match arm, constant, flag, exit code, or output byte changes. Only file moves, `mod` declarations, `use`-statement redistribution, visibility annotations (`pub`/`pub(super)`/`pub(crate)` kept equivalent), and re-exports.
2. **Public API frozen:** every `pub` item currently reachable as `crate::hash::X`, `girsync::hash::X`, `crate::cache::CacheDb`, `girsync::cache::CacheDb`, `girsync::cache::TestCacheBackend`, `sync::apply::Applier` must remain reachable by the same path after the step.
3. **Visibility equivalence:** an item that was `pub(super)` (visible to the parent module) stays `pub(super)` relative to its new parent, or `pub` where it already was. No item becomes more or less visible.
4. **Verify after every step:** `cargo check -p girsync --all-targets` then `cargo nextest run -p girsync` must both be green before proceeding.
5. **Tests untouched:** no file under `tests/` is modified, added, or removed.

---

### 4. Exact ordered steps

Conventions: paths are relative to `tools/sync/`. File moves = create new file with identical content (adjusting `use` paths as noted), then delete the old file. All edits preserve existing line endings (CRLF/LF as-is).

#### Phase 0 — Baseline (already verified)
- Run `cargo check -p girsync --all-targets` and `cargo nextest run -p girsync`.
- Expected: compile clean; 326 passed, 1 skipped.
- Commit this baseline (or note `git status` is clean) so each later step has a clean diff to roll back to.

#### Phase 1 — Split `src/hash.rs` (1050 lines) → `src/hash/`

Rationale: `HashAlg` (command-line vocabulary), `AlgoSet` (bitfield set), `DigestSet` (row storage), and `Digests`/`hash_file` (streaming) are four distinct concerns.

**Files to create:**

`src/hash/mod.rs`:
```rust
//! Hash algorithm selection and streaming file digests.
//! ... (move the existing module doc here, add a submodule map table)
mod algo;
mod digestset;
mod set;
mod stream;
#[cfg(test)]
mod tests;

pub use algo::{HashAlg, UnknownHash};
pub use digestset::DigestSet;
pub use set::{AlgoSet, parse_hash_list, parse_any_of};
pub use stream::{CHUNK, Digests, hash_file};
```

`src/hash/algo.rs` — move from `hash.rs`: `HashAlg` enum + `ALL`, `as_str`, `digest_len`, `bit`, `names`, `FromStr`, `Debug`, `Display`, and `UnknownHash` + its `Display`/`Error` impls. `use std::str::FromStr;`

`src/hash/set.rs` — move from `hash.rs`: `AlgoSet` (+ `FromIterator`), `parse_hash_list`, `parse_any_of`. `use crate::hash::algo::HashAlg;` (or `use super::algo::HashAlg;`), `use anyhow::{Context, Result, anyhow, bail};`

`src/hash/digestset.rs` — move from `hash.rs`: `DigestSet` + all impls. `use super::algo::HashAlg;`

`src/hash/stream.rs` — move from `hash.rs`: `CHUNK`, `Digests`, `hash_file`. `use anyhow::{Context, Result, anyhow, bail};`, `use sha2::Digest;`, `use std::io::Read;`, `use std::path::Path;`, `use crate::hash::{AlgoSet, DigestSet, HashAlg};` (via `super::`).

`src/hash/tests.rs` — move the entire `#[cfg(test)] mod tests { ... }` body from `hash.rs` here. It uses `use super::*;` which now brings all `pub use`d items from `hash/mod.rs` (HashAlg, AlgoSet, DigestSet, Digests, parse_hash_list, parse_any_of, hash_file, CHUNK) plus `girpr_tempdir::TempDir` (dev-dep, already used). **No other `use` changes needed for these tests.**

**File to delete:** `src/hash.rs`

**`use`-statement impact on other files:** none. Every `src/*` and `tests/*` file that does `use crate::hash::{…}` or `use girsync::hash::{…}` keeps resolving because all `pub` items are re-exported from `hash/mod.rs` (confirmed by grep: 20 src files + 8 test files import from `crate::hash`/`girsync::hash`, all by name).

#### Phase 2 — Split `src/cache/db.rs` (644 lines) → `src/cache/db/`

Rationale: `db.rs` mixes the redb `Handle` type/backend setup (`CACHE_SIZE`, `redb_builder`, `memory_registry`, `TestCacheBackend`) with the `CacheDb` handle API. Splitting gives `backend.rs` (backend concerns), `handle.rs` (`Handle`), `cachedb.rs` (`CacheDb` methods).

**Files to create:**

`src/cache/db/mod.rs`:
```rust
mod backend;
mod cachedb;
mod handle;
#[cfg(test)]
mod tests;

pub use backend::TestCacheBackend;
pub use cachedb::CacheDb;
pub(super) use backend::redb_builder;
```
(`cache/mod.rs` already does `pub use db::{CacheDb, TestCacheBackend};` — unchanged, still resolves.)

`src/cache/db/backend.rs`: `CACHE_SIZE` const, `redb_builder()`, `memory_registry()`, `TestCacheBackend` enum. `use redb::{Builder, Database, ReadableDatabase, backends::InMemoryBackend};`, `use std::collections::HashMap;`, `use std::path::{Path, PathBuf};`, `use std::sync::{Arc, Mutex, OnceLock};`

`src/cache/db/handle.rs`: `pub(super) enum Handle` + `impl Handle { writable }` + `impl ReadableDatabase for Handle`. `use redb::{ReadableDatabase, ReadableTable, …};` as needed.

`src/cache/db/cachedb.rs`: the full `impl CacheDb` block (all methods: `open_writable`, `open_read_only`, `init_meta`, `open_file`, `open_temp`, `open_memory_cache`, `memory_cache_at`, `forget_memory_cache`, `open_record`, `is_read_only`, `writable`, `get`, `load_keys`, `load_all`, `put`, `remove`, `begin_write`, `begin_write_with`) + `#[cfg(test)] mod tests`. `use` statements needed:
- `use anyhow::{Context, Result, bail};`
- `use redb::{Database, ReadableDatabase, ReadableTable, backends::InMemoryBackend};`
- `use std::collections::HashMap;`
- `use std::path::{Path, PathBuf};`
- `use std::sync::{Arc, Mutex, OnceLock};`
- **Cache-level items previously reached via `use super::{ENTRIES, META_KEY, META_TBL}` (super was `cache`):** now `use crate::cache::{ENTRIES, META_KEY, META_TBL};`
- `use crate::cache::codec::{decode_meta, decode_rec, encode_meta};`
- `use crate::cache::open::{is_table_missing, open_read_only, read_meta, require_current_version};`
- `use crate::cache::schema::{CACHE_VERSION, FileRec, Meta};`
- `use crate::cache::write::{CacheWrite, CommitLimits};`
- `use super::{ENTRIES, META_KEY, META_TBL}` → replaced by the `crate::cache::` paths above (db is now one level deeper, so `super` = `cache::db`, not `cache`).
- Tests in cachedb.rs: `use super::*;` now brings `CacheDb`/`TestCacheBackend`/`redb_builder` from `db/mod.rs` ✓, but cache-level items (`ENTRIES`, `META_KEY`, `META_TBL`, `encode_meta`, `Meta`, `CACHE_VERSION`, `FileRec`, `CACHE_PREFIX`) are no longer in `super` (db). Add to the tests module: `use crate::cache::{CACHE_VERSION, ENTRIES, META_KEY, META_TBL, FileRec, Meta};`, `use crate::cache::codec::encode_meta;`, `use crate::cache::schema::CACHE_PREFIX;` (the rest — `open_db`, `CacheOpen`, `DigestSet`, `girpr_tempdir`, `PathBuf` — are already explicit `use crate::…` in the test body).

**File to delete:** `src/cache/db.rs`

**`use`-statement impact on other files:** `cache/open.rs` line 17 `use super::db::{CacheDb, redb_builder};` — **unchanged**; `db` is still a module (`cache::db`), `CacheDb` and `redb_builder` are re-exported from `db/mod.rs`. All other files use `crate::cache::{CacheDb, TestCacheBackend, …}` (resolved via `cache/mod.rs` re-exports) — unchanged.

#### Phase 3 — Split `src/commands/sync/apply.rs` (824 lines) → `src/commands/sync/apply/`

Rationale: `apply.rs` mixes the `Applier` coordinator (`apply()` + phase order), four filesystem phases, and the copy/verify machinery (`copy_one`, `copy_and_digest`, `verify_digests`, `test_no_fsync`) plus their unit tests.

**Files to create:**

`src/commands/sync/apply/mod.rs`:
```rust
mod copy;
mod phases;

pub(super) struct Applier<'a> { … }       // fields stay pub
pub(super) struct Applied { … }

impl<'a> Applier<'a> {
    pub(super) fn apply(&self, plan: &Plan) -> Result<Applied> { … }   // phase order
    fn mkdirs(&self, plan: &Plan) -> Result<()> { … }
    fn fix_dirs(&self, plan: &Plan) -> Result<()> { … }
    fn predrop_cache_entries(&self, plan: &Plan) -> Result<()> { … }
}
```
`use` statements: `anyhow::{Result, bail}`, `std::path::Path`, `tracing::{debug, info, trace}`, `crate::cache::{CacheDb, FileRec}`, `crate::config::CommonOpts`, `crate::effective::EffRec`, `crate::report::{Record, Report}`, `crate::rw::RwRuntime`, `crate::util::mtime_ns_of`, `super::plan::Plan`. (Items needed only by `copy_files`/`delete_extras`/… move to `phases.rs`; items needed only by copy helpers move to `copy.rs`.)

`src/commands/sync/apply/phases.rs`:
```rust
impl<'a> Applier<'a> {
    fn delete_extras(&self, plan: &Plan) -> Result<usize> { … }
    fn copy_files(&self, plan: &Plan) -> Result<usize> { … }   // calls super::copy::copy_one
    fn remove_unknown_dirs(&self, plan: &Plan) -> Result<usize> { … }
    fn prune_dst_cache(&self, plan: &Plan) -> Result<()> { … }
}
```
`use` statements: everything those four methods need (`anyhow::{Context, Result, bail}`, `std::collections::{HashMap, HashSet}`, `std::io::{Read, Write}`, `std::path::Path`, `tracing::{debug, error, info, trace}`, `crate::cache::{CacheDb, FileRec}`, `crate::config::CommonOpts`, `crate::effective::EffRec`, `crate::hash::{AlgoSet, CHUNK, DigestSet, Digests, HashAlg, hash_file}`, `crate::report::{Record, Report}`, `crate::rw::RwRuntime`, `crate::util::mtime_ns_of`, `super::plan::Plan`, `super::copy::copy_one`). Note `copy_one` is `pub(super)` in `copy.rs` → visible to the `apply` module; `phases.rs` is a child of `apply`, so `super::copy::copy_one` resolves.

`src/commands/sync/apply/copy.rs`:
```rust
pub(super) fn copy_one(src_root: &Path, dst_root: &Path, rel: &str, algos: &[HashAlg]) -> Result<FileRec> { … }
fn copy_and_digest(src: &mut File, dst: &mut File, algos: &AlgoSet) -> Result<DigestSet> { … }
fn verify_digests(path: &Path, algos: &[HashAlg], src: &DigestSet, dst: &DigestSet) -> Result<()> { … }
fn test_no_fsync() -> bool { … }
#[cfg(test)]
mod tests { … }   // moved from apply.rs bottom
```
`use` statements: `anyhow::{Context, Result, bail}`, `std::io::{Read, Write}`, `std::path::Path`, `tracing::{debug, error, info, trace}`, `crate::cache::FileRec`, `crate::hash::{AlgoSet, CHUNK, DigestSet, Digests, HashAlg, hash_file}`, `crate::report::Record`, `crate::rw::RwRuntime`, `crate::util::mtime_ns_of`, `girpr_tempdir::TempDir` (tests), `std::path::Path`, `std::sync::…` (tests), `use super::{copy_and_digest, copy_one, verify_digests};` (tests). `copy_and_digest`, `verify_digests`, `test_no_fsync` stay private to `copy` (they were only ever used inside `apply.rs`; now only inside `copy.rs` + `copy::tests`). `copy_one` is `pub(super)` so `phases.rs` can call it.

**File to delete:** `src/commands/sync/apply.rs`

**`use`-statement impact on other files:** `commands/sync/mod.rs` line 69 `use apply::Applier;` — **unchanged** (`Applier` is `pub(super)` in `apply/mod.rs`, visible to parent `sync`). `commands/sync/plan.rs` line 9 `use super::apply::Applied;` — **unchanged** (`Applied` `pub(super)` visible to `sync`; `plan.rs` is a child of `sync`). All `crate::…` imports inside apply submodules use absolute paths, unchanged.

#### Phase 4 — Doc touch-up (no behavior change)

- Update `src/lib.rs` module-map doc comment to list `hash` as a directory module whose `mod.rs` is a facade (it currently calls out `cache`, `config`, `diff`, `effective`, `planner`, `report`, `rw`; add `hash`).
- Optionally update `src/cache/mod.rs` submodule table to note `db` is now `db/` with `backend`/`cachedb`/`handle` leaves.
- These are comment-only; run `cargo check` after.

#### Phase 5 — Final full verification
- `cargo check -p girsync --all-targets`
- `cargo nextest run -p girsync` (expect 326 passed, 1 skipped — identical to baseline)
- `git diff --stat` should show only moves/reorganizations within `src/`, no `tests/` changes, no `Cargo.toml` changes.

---

### 5. Verification at each step

After **each** phase (and after each individual file creation within a phase, if you prefer finer granularity):

1. `cargo check -p girsync --all-targets` — catches `use`-path and visibility errors across lib, both bins, and all test targets.
2. `cargo nextest run -p girsync` — must report the same pass/skip count as baseline (326 passed, 1 skipped). nextest runs all integration binaries + unit tests in-process.
3. `git status` / `git diff --stat` — confirm only `src/` changed; `tests/` and `Cargo.toml` untouched.

If a step fails: revert that phase's files (`git checkout -- <deleted files>` + remove new files) and return to the last green commit before proceeding. Because every step keeps all `pub` paths re-exported, a failure should be limited to a `use` path or visibility annotation within the moved file — never a logic or public-API issue.

---

### 6. Why this scope and what is deliberately out of scope

- **In scope:** `hash.rs`, `cache/db.rs`, `commands/sync/apply.rs` — the three files far larger than their siblings (1050 / 644 / 824 lines) that each combine multiple concerns. Splitting them into facade-led directory modules matches the pattern already used by `cache/`, `commands/sync/`, `effective/`, `planner/`, `report/`, `rw/`.
- **Out of scope (not proposed):** `planner/pairs.rs` (597 lines) and `diff/mod.rs` (534 lines) — each is one coherent function (`plan_pairs` / `diff_maps`) with tightly coupled helpers; splitting risks more churn than clarity gain. `effective/entry.rs`, `planner/plan.rs`, `commands/sync/mod.rs` are already well-factored. Top-level `cli.rs`, `logging.rs`, `scan.rs`, `util.rs`, `filter.rs`, `convert.rs` are small and single-concern. No module merges are proposed (e.g. `filter`+`util` would merge unrelated concerns). No `Cargo.toml`, target, feature, or profile changes.
- **Never changed:** public API, CLI flags, exit codes, stdout/stderr formats, redb wire format, test files, behavior under `--dry-run`/`--output`/`--hash-*`/`--rw-*`/`--case-sensitive`.

---

### Rollback

`git` is available in the workspace. Each phase is committed separately on a clean baseline; if `cargo nextest run -p girsync` regresses at any step, `git reset --keep` (or `git checkout`) to the previous phase's commit and re-attempt with corrected `use` paths/visibility. Since no logic moves, a green `cargo check` + `cargo nextest` at a step is sufficient evidence that step is behavior-preserving.
