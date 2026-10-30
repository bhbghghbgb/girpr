# girpr refactoring and optimization opportunities

**Scope:** the main `girpr` crate only (`src/`). `tools/sync`, `tools/capture`, and
`tests/scripts/*` are out of scope.

**Baseline:** `cargo clippy --all-targets` is clean and the working tree is clean.
Nothing in this document is a lint failure; these are design, DRY, maintainability,
and theoretically-sound performance opportunities.

**Ground rules used when judging a change:**

- An unflagged run's `REPORT` / `PROGRESS` / `PLAN` / `SUMMARY` output must stay
  byte-identical.
- Conditional `SUMMARY` keys are emitted only when their flag is active.
- Exit codes `0`–`5` and the `--output text|json` NDJSON shape are contracts.
- `--scan` stays opt-in; an unflagged run must not gain a full install read.
- Path containment (`contained_rel` / `contained_join`) is a security boundary;
  do not weaken it for speed.
- The temp path's resume-by-length + rollback and the final whole-file MD5 gate
  are load-bearing; the in-place path's "only write regions that were already
  wrong" invariant is load-bearing.
- Deliberate non-fixes stay non-fixes: S1 (no manifest cache), S2 (same-file-only
  reuse), last-match-wins `game_version`, the `chuncks` protobuf spelling, and
  audio-filter parity with Starward.

## Risk legend

| Risk | Meaning |
|---|---|
| **Low** | Local, behavior-preserving refactor or a pure helper. Tests should pass unchanged. |
| **Medium** | Touches a gate, an ordering rule, a counter, a public type, or a scan verdict. Needs targeted tests and a careful diff. |
| **High** | Changes a documented invariant, a broad public API, or moves a large amount of pipeline logic. Split into smaller changes and land behind tests. |

## Findings at a glance

| ID | Area | Risk | Summary |
|---|---|---|---|
| A1 | reuse / chunk identity | Medium | One chunk-identity predicate; normalize MD5 case at the map boundary. |
| A2 | reuse / chunk identity | Medium | Index the per-file reuse entries instead of a linear scan per chunk. |
| A3 | reuse / salvage order | Low | `salvage_order` builds both direction vectors and discards one. |
| B1 | scan | Low | `scan_plan` deep-clones every `PlannedFile`; use `Arc` + index. |
| B2 | scan | Low | `est_file_download_bytes` clones chunks just to call `salvage_order`. |
| B3 | scan | Medium | `scan_one` and the in-place scan build source ranges differently. |
| B4 | scan | Low | `ScanReport` keys verdicts by `String`; a parallel `Vec` is enough. |
| C1 | repair writers | Low | Per-chunk 512 KiB buffers are allocated repeatedly. |
| C2 | repair writers | Medium | `try_reuse_slice` opens and reads the source slice twice. |
| C3 | repair writers | Low | `inplace` repeats the final verify/log block. |
| C4 | repair writers | Low | `inplace` clones chunks for `salvage_order` (same fix as B2). |
| C5 | repair writers | Low | `_tmp` path is built through `display()` (lossy on non-UTF-8 paths). |
| C6 | repair writers | Medium | `repair_all` spawns one task per file, all waiting on a semaphore. |
| C7 | repair writers | Low | `write_at` / `copy_within` / `zero_fill` repeat reserve/seek/write/note. |
| D1 | planning / checking | Low | `dedup_key` and `purge_key` are the same function in two modules. |
| D2 | planning / checking | Medium | `expected_paths()` clones, then `plan_purge` re-keys into another set. |
| D3 | planning / checking | Low | `build_plan` calls the sorting `layout()` just to log a warning. |
| D4 | planning / checking | Low | `build_plan` clones the URL prefix once per file. |
| D5 | planning / checking | Medium | `verify_plan` has two near-identical scan/fallback branches. |
| D6 | planning / checking | Low | `build_plan` counts empty chunks, then `retain`s in a second pass. |
| E1 | purge | Low | `plan_purge` calls `scope.is_visible` twice per file. |
| E2 | purge | Medium | Purge keep-sets allocate a new `String` per path. |
| F1 | reporting | Low | `Summary` has three parallel field lists (`new` / `Default` / `snapshot`). |
| F2 | reporting | Low | `conditional_extras` is a long hand-ordered ladder. |
| F3 | reporting | Low | `Record` rendering allocates a `String` per field. |
| G1 | utilities | Low | `RangeState::is_ok` formats a hex digest per range. |
| G2 | utilities | High | MD5 is compared as hex strings throughout; a digest type would be better. |
| G3 | utilities | Low | `read_game_version` compiles a regex on every call. |
| G4 | utilities | Low | `write_config_ini` formats every output line into a temporary `String`. |
| G5 | utilities | Medium | `contained_join` re-resolves the base path on every call. |
| G6 | utilities | Medium | `clean_rel` normalizes the same path twice. |
| G7 | utilities | Low | `md5_file_slice` has redundant imports and a dead `min`; `read_range` can over-allocate. |
| H1 | orchestration | High | `repair::run` is a 455-line coordinator mixing plan, assert, and execute. |
| H2 | orchestration | Medium | The `SUMMARY` flag assembly is a long `if` ladder inside `run`. |
| H3 | orchestration | Low | `finish` and the runtime-stop branch both set `summary.flags.stopped`. |
| H4 | orchestration | Medium | `RunCtx`'s many independent bools allow states that should be impossible. |

---

## A. Chunk identity and reuse

### A1 — One chunk-identity predicate, normalized once

**Risk:** Medium · **Kind:** DRY / correctness-adjacent · **Files:**
`src/sophon/map.rs:40-49`, `src/repair/reuse.rs:148-160`,
`src/repair/file/temp.rs:263-267`, `src/repair/phases/plan.rs:166-181`

The same rule — “this reuse entry is this chunk's bytes” — is written four times:

```rust
md5.eq_ignore_ascii_case(&c.uncompressed_md5) && *sz == c.uncompressed_size
```

`build_local_chunk_map` is the exception: it dedups with **exact** string equality
(`md5 == &c.uncompressed_md5`), while every consumer looks up case-insensitively.
The result is not currently wrong (first match still wins), but the map's comment
“keep first occurrence per md5” is not true for case variants, and two entries
that name the same chunk can survive. That costs memory and makes the first-match
rule depend on spelling.

**Suggested shape:** introduce one type and one predicate, e.g.

```rust
pub struct ReuseEntry {
    pub md5: String,        // normalized lowercase at construction
    pub size: i64,
    pub offset: i64,
}

impl ReuseEntry {
    pub fn matches(&self, c: &SophonChunk) -> bool {
        self.size == c.uncompressed_size
            && self.md5.eq_ignore_ascii_case(&c.uncompressed_md5)
    }
}
```

Normalize the MD5 once in `build_local_chunk_map` (`to_ascii_lowercase`) and use
`matches` in `sources_for`, `try_reuse_slice`, and `est_file_download_bytes`.
This removes the exact-vs-insensitive split and gives A2 a natural key.

**Contract notes:** first-entry-wins is pinned by
`sources_for_takes_the_first_entry_matching_the_chunk`; preserve it.

---

### A2 — Index the per-file reuse entries

**Risk:** Medium · **Kind:** performance / API · **Files:**
`src/repair/phases/manifest.rs:24-26`, `src/sophon/map.rs:20-53`,
`src/repair/reuse.rs:148-160`, `src/repair/file/temp.rs:263-267`,
`src/repair/phases/plan.rs:166-181`

`LocalChunkMap` is `HashMap<String, Vec<(String, i64, i64)>>`. Every consumer
walks the per-file vector for every chunk:

- `sources_for` is `chunks × entries`;
- `try_reuse_slice` does `entries.iter().find(...)` per chunk;
- `est_file_download_bytes` does `entries.iter().any(...)` per chunk.

The same file's entries are therefore scanned repeatedly by the plan estimate,
the scan estimate, and the writer. S2 keeps the map same-file-only, so this is
per-file rather than global, but a file with hundreds of chunks and hundreds of
distinct identities is still quadratic in the hot path.

**Suggested shape:** keep the deterministic `Vec` for first-match semantics, but
build an index beside it:

```rust
pub struct FileReuse {
    entries: Vec<ReuseEntry>,                 // first-match order preserved
    by_key: HashMap<(String, i64), usize>,    // lowercased md5 + size -> entry
}
```

Insert only when the key is absent, so “first wins” is preserved. `sources_for`
and the temp lookup become `by_key.get(&key).map(|&i| entries[i].offset)`.

**Contract notes:** the map is a best-effort optimization; a missing or duplicate
entry must still degrade to a download, never to a wrong result. Keep the
`build_local_chunk_map` skip-and-warn behavior for unclean paths.

---

### A3 — `salvage_order` computes both directions eagerly

**Risk:** Low · **Kind:** micro-optimization / clarity · **Files:**
`src/repair/reuse.rs:92-138`

`salvage_order` calls `survives` twice, each allocating `vec![false; n]`, then
keeps one vector and drops the other. It also counts the survivors inline and
`Order::live_count` counts again at the call site.

**Suggested shape:** compute the two counts with a cheap `survives_count` pass,
pick the direction, then materialize only the winning `Vec<bool>`. Alternatively
have `survives` return an iterator and collect only once. This is a small win,
but it makes the “tie goes to ascending” rule and the counting live in one place.

**Contract notes:** the direction and `live` vector are pinned by the existing
unit tests; keep them.

---

## B. Scan

### B1 — `scan_plan` deep-clones every `PlannedFile`

**Risk:** Low · **Kind:** performance / memory · **Files:**
`src/repair/scan.rs:213-259`, `src/repair/file/mod.rs:182-184`

`scan_plan` already receives `plan: Arc<RepairPlan>`, but the stream does:

```rust
plan.files.iter().cloned().map(|f| { ... scan_one(&game_dir, &f, &local_map) ... })
```

Each `PlannedFile` owns its rel, MD5, and full `Vec<SophonChunk>`, so this is a
deep clone per file. `file/mod.rs` documents that the same clone once cost ~6 GB
across ~3k tasks and was fixed there by sharing the `Arc` and passing an index.
The scan has the same problem.

**Suggested shape:** clone the `Arc`, not the file:

```rust
let plan = Arc::clone(&plan);
futures::stream::iter(0..plan.files.len()).map(move |idx| {
    let plan = Arc::clone(&plan);
    let game_dir = game_dir.to_path_buf();
    let local_map = Arc::clone(&local_map);
    async move {
        tokio::task::spawn_blocking(move || {
            scan_one(&game_dir, &plan.files[idx], &local_map)
        }).await
    }
})
.buffer_unordered(jobs.max(1))
```

`Scanned` can then carry `idx` instead of `rel`, which also helps B4.

**Contract notes:** none; this is a pure ownership change. The `Arc<RepairPlan>`
lifetime already outlives the stream.

---

### B2 — `est_file_download_bytes` clones chunks for `salvage_order`

**Risk:** Low · **Kind:** performance / DRY · **Files:**
`src/repair/scan.rs:166-197`, `src/repair/reuse.rs:92-138`

The in-place estimate builds `bad: Vec<&SophonChunk>`, then clones every bad
chunk into `order_chunks: Vec<SophonChunk>` only because `salvage_order` takes
`&[SophonChunk]`. It also walks `chunks` a second time to build `srcs`.

**Suggested shape:** let `salvage_order` accept borrowed chunks or a small span:

```rust
pub fn salvage_order_refs(chunks: &[&SophonChunk], sources: &[Option<u64>], usable_len: u64) -> Order
```

or pass `&[usize]` indices plus the original `&[SophonChunk]`. Build `bad` and
`srcs` in one pass over `chunks`. The same change removes the clone in C4.

**Contract notes:** the order rule and `usable_len = u64::MAX` reasoning in the
estimate must stay exactly as documented.

---

### B3 — Unify the scan's source-range construction

**Risk:** Medium · **Kind:** DRY / performance · **Files:**
`src/repair/scan.rs:287-326`, `src/repair/file/inplace.rs:87-109`

`scan_one` deliberately adds a source range **only** when `sources[i]` is `Some`,
and remaps `scan.source_ok` back through `source_owner`. The comment explains
why: coercing a missing source to offset 0 would ask the scan to verify the
chunk's bytes at the head of the file and could mark a non-existent source
intact.

The in-place path does exactly that coercion:

```rust
let source_ranges = file.chunks.iter().zip(sources.iter())
    .map(|(c, src)| (src.unwrap_or(0), c.uncompressed_size.max(0) as u64, ...))
    .collect();
```

Today the consumers guard with `sources[i].is_some()` (`staged_payload`, the
capture loop, and the ordered tier), so no wrong bytes are written. But the
scan still hashes the first `size` bytes of the file once per source-less chunk,
and the verdict it produces is semantically “a source that does not exist is
intact”. For a reflowed file with many distinct chunks this is wasted hashing
proportional to `chunks × head size`.

**Suggested shape:** extract one helper, used by both call sites:

```rust
pub(crate) fn scan_ranges<'a>(
    chunks: &'a [SophonChunk],
    sources: &[Option<u64>],
) -> (Vec<ScanRange<'a>>, Vec<ScanRange<'a>>, Vec<usize>)
```

`scan_one` already has this logic. In `inplace`, build `dest_ranges`,
`source_ranges`, and `source_owner`, then remap `source_ok` back to one verdict
per chunk before `staged_payload`, capture, and `salvage_order`. Use
`Scan::absent(dest_ranges.len(), source_ranges.len())` for the discard path and
remap the same way.

**Contract notes:** `Scan::absent` takes the number of ranges, not chunks, so the
remap must happen on the discard path too. The existing
`a_chunk_with_no_source_is_never_scanned_at_offset_zero` test is the guard.

---

### B4 — `ScanReport` keys verdicts by `String`

**Risk:** Low · **Kind:** performance / API · **Files:**
`src/repair/scan.rs:56-64`, `src/repair/scan.rs:96-123`,
`src/repair/phases/check.rs:43`

`ScanReport.files` is a `HashMap<String, Verdict>` keyed by rel. `scan_plan`
inserts `scanned.rel`; `est_download_bytes_by_file` looks up `&f.rel`;
`verify_plan` looks up `&f.rel`. The plan is already a stable, sorted `Vec`, so
the map is doing string hashing and holding a second copy of every rel.

**Suggested shape:** store `Vec<Option<Verdict>>` parallel to `plan.files` (or
`Vec<Verdict>` with an explicit “not scanned” state). `scan_plan` writes by
`idx`; consumers read by index. This pairs naturally with B1.

**Contract notes:** `ScanReport` is public and its `files` field is used in tests;
either keep a `files()` accessor or update the tests. Do not change the estimate's
“an unreached file is estimated in full” rule.

---

## C. Repair write paths

### C1 — Reuse a per-file scratch buffer

**Risk:** Low · **Kind:** performance · **Files:**
`src/repair/file/temp.rs:278-295`, `src/repair/file/mod.rs:602-655`,
`src/util/md5.rs:248-266`

Four hot paths allocate a 512 KiB buffer:

- `try_reuse_slice` — `vec![0u8; COPY_BUF]` per reused chunk;
- `copy_within` — `vec![0u8; COPY_BUF.min(len)]` per call;
- `zero_fill` — `vec![0u8; COPY_BUF.min(len)]` per call;
- `md5_file_slice` — `vec![0u8; MD5_BUF.min(1 << 20)]` per reuse candidate.

For a file with hundreds of chunks this is hundreds of large allocations and
frees. The buffers are sequential within a file (chunks are written one at a
time), so one scratch buffer per file is enough.

**Suggested shape:** add a small `FileScratch` (or just `&mut Vec<u8>`) owned by
`repair_attempt_inner` / `attempt_inplace_inner`, and thread it into
`try_reuse_slice`, `copy_within`, `zero_fill`, and a
`md5_file_slice_with_buf` variant. `zero_fill` can use a shared zero block
(a `static ZEROS: [u8; 64 * 1024]` or a `OnceLock<Vec<u8>>`) because it never
needs to be written.

**Contract notes:** do not change the write-ceiling charging points. In
particular `zero_fill` currently reserves the whole length before writing;
keep that reservation in the caller if the helper only writes.

---

### C2 — `try_reuse_slice` opens and reads the source slice twice

**Risk:** Medium · **Kind:** performance · **Files:**
`src/repair/file/temp.rs:263-307`, `src/util/md5.rs:248-266`

The reuse path first calls `md5_file_slice(src, ro, size)`, which opens `src`,
seeks, and reads the slice; on a match it then does `File::open(src)` again,
seeks again, and reads the same slice to copy it into the `_tmp`. That is two
opens and two full reads of the slice.

The two reads are deliberate: the bytes are verified before they are written, so
a corrupt source is never copied. Keep that property, but use one handle:

```rust
let mut sf = File::open(src)?;
let h = md5_reader_slice(&mut sf, ro, size)?;   // seek + read + hash
if h.eq_ignore_ascii_case(&c.uncompressed_md5) {
    sf.seek(SeekFrom::Start(ro))?;
    // copy loop
}
```

This removes one open and one seek, and composes with C1 (pass the scratch
buffer). A one-pass “hash while copying, roll back on mismatch” is possible
because `rollback_tmp` exists and the temp path already writes downloaded chunks
before their digest is known, but it is a larger behavioral change; the
single-handle two-pass is the low-risk step.

**Contract notes:** a short slice must still fall through to a download, and a
digest mismatch must still leave the `_tmp` in a state the next attempt can
resume. `rollback_tmp` covers the download failure path; do not weaken it.

---

### C3 — `inplace` repeats the final verify/log block

**Risk:** Low · **Kind:** DRY · **Files:** `src/repair/file/inplace.rs:330-389`

The success path after the first verify and the success path after the redo are
the same block: log `repaired in place` with the same seven fields and return
`Inplace::Repaired(stats.clone())`. Adding a counter or changing a field means
editing both, and the two can drift.

**Suggested shape:**

```rust
fn repaired_in_place(stats: &ChunkStats, md5: &str) -> Inplace {
    tracing::info!(
        chunks_total = stats.chunks_total,
        chunks_kept = stats.chunks_kept,
        chunks_reused = stats.chunks_reused,
        chunks_downloaded = stats.chunks_downloaded,
        download_bytes = stats.download_bytes,
        final_md5 = %md5,
        "repaired in place"
    );
    Inplace::Repaired(stats.clone())
}
```

Both sites call it with `after.file_md5` / `final_scan.file_md5`.

**Contract notes:** the log fields and message text are observable; keep them
identical.

---

### C4 — `inplace` clones chunks for `salvage_order`

**Risk:** Low · **Kind:** performance / DRY · **Files:**
`src/repair/file/inplace.rs:260-280`

Same shape as B2:

```rust
let chunks: Vec<SophonChunk> = bad.iter().map(|&i| file.chunks[i].clone()).collect();
```

Change `salvage_order` to borrow (`&[&SophonChunk]` or indices), or add a
`salvage_order_indices(&file.chunks, &bad, &srcs, usable)`. The ordered tier's
inputs are already indices into `file.chunks`, so no owned vector is needed.

**Contract notes:** the `usable = handle_len(&handle)` value and the log fields
(`ascending`, `salvaged`, `of`) stay the same.

---

### C5 — Build the `_tmp` path without `display()`

**Risk:** Low · **Kind:** correctness / clarity · **Files:**
`src/repair/file/mod.rs:402`

```rust
let tmp_path = PathBuf::from(format!("{}_tmp", final_path.display()));
```

`Path::display()` is lossy: a game directory that is not valid UTF-8 (possible
on Unix, and on Windows for some non-Unicode names) would get a different path
than the one intended. The suffix is a simple append, so no formatting is needed:

```rust
let mut tmp = final_path.as_os_str().to_owned();
tmp.push("_tmp");
let tmp_path = PathBuf::from(tmp);
```

**Contract notes:** the suffix must stay `_tmp` (Starward parity, and the purge
treats `*_tmp` as an orphan). `write_config_ini` uses a different temp name
(`config.ini.girpr_tmp`), which is unrelated.

---

### C6 — Bound the number of spawned file tasks

**Risk:** Medium · **Kind:** performance / concurrency · **Files:**
`src/repair/file/mod.rs:197-329`

`repair_all` spawns one `tokio` task for every planned file up front, and each
task waits on a semaphore permit:

```rust
for idx in 0..total_files {
    ...
    handles.push(tokio::spawn(async move {
        let _permit = sem.acquire_owned().await...;
        ...
    }));
}
```

For a 100k-file plan that is 100k tasks, 100k `JoinHandle`s, and 100k captured
`Arc<FileCtx>`s before the first chunk is fetched. The semaphore bounds the work,
not the task count. `scan_plan` already uses `buffer_unordered(jobs)`; the repair
loop needs the same bound while preserving the per-task panic accounting that
`join_file_tasks` exists for.

**Suggested shape:** use `tokio::task::JoinSet` with a bounded in-flight window:

```rust
let mut set = JoinSet::new();
let mut next = 0;
while next < total_files || !set.is_empty() {
    while set.len() < jobs && next < total_files {
        if ctx.summary.aborted() { break; }
        set.spawn(run_one(ctx.clone(), next, ...));
        next += 1;
    }
    if let Some(result) = set.join_next().await { /* same failure handling */ }
}
```

This keeps `JoinError` per file (so the `F-1` / `F-8` tests still hold), keeps
`--fail-fast` checking before a task starts, and avoids the unbounded task
spray. A fixed worker pool pulling from an `AtomicUsize` is equivalent.

**Contract notes:** do not switch to `buffer_unordered` around the file futures
themselves: a panic would then unwind the driver rather than producing a
`JoinError`, and `join_file_tasks`' failure accounting would be lost. Keep the
completion-order join.

---

### C7 — One write helper for reserve/seek/write/note

**Risk:** Low · **Kind:** DRY · **Files:** `src/repair/file/mod.rs:582-655`

`write_at`, `copy_within`, and `zero_fill` all perform some subset of:

1. reserve against the write ceiling,
2. seek,
3. write,
4. `note_write`.

The ordering and the reservation granularity differ deliberately:

- `write_at` reserves once for the whole slice;
- `copy_within` reserves per piece;
- `zero_fill` reserves the whole length once, then writes pieces.

**Suggested shape:** separate “reserve” from “write”:

```rust
fn write_reserved(ctx, writes, f, offset, bytes) -> Result<()> {
    f.seek(SeekFrom::Start(offset))?;
    f.write_all(bytes)?;
    note_write(ctx, writes, bytes.len() as u64)
}
```

Then `write_at` = `reserve_write` + `write_reserved`; `copy_within` reserves per
piece and calls `write_reserved`; `zero_fill` reserves `len` once and calls
`write_reserved` per zero piece. This removes the duplicated seek/write/note
without changing when the ceiling is charged.

**Contract notes:** the per-piece vs whole-length reservation difference is
observable under a tight `--max-write-bytes`; preserve it.

---

## D. Planning and checking

### D1 — `dedup_key` and `purge_key` are the same function

**Risk:** Low · **Kind:** DRY · **Files:** `src/repair/phases/plan.rs:229-240`,
`src/repair/purge.rs:312-322`

Both are:

```rust
if cfg!(windows) { rel.to_lowercase() } else { rel.to_string() }
```

They must agree: the plan's dedup and the purge's keep comparison are two halves
of the same “Windows paths are case-insensitive” rule. Keeping two copies means
a future change (e.g. Unicode folding) can update one and not the other.

**Suggested shape:** move it to `util::path` (next to `clean_rel`) as
`case_key(rel: &str) -> String` or `path_key`, and use it from both modules.

**Contract notes:** keep the compile-time `cfg!(windows)` behavior; the existing
`the_plan_dedups_case_insensitively_on_windows` and
`the_purge_folds_case_on_windows_only` tests pin it.

---

### D2 — Avoid the intermediate `expected_paths` set

**Risk:** Medium · **Kind:** performance / API · **Files:**
`src/repair/phases/plan.rs:126-130`, `src/repair/purge.rs:98-103`

`RepairPlan::expected_paths()` builds a `HashSet<String>` by cloning every
`PlannedFile.rel`. `plan_purge` then maps that set through `purge_key`, building
a second `HashSet<String>`. For a large plan this is two full sets and two
string clones per file before the walk even starts.

**Suggested shape:** have `plan_purge` build the keyed set directly:

```rust
let expected: HashSet<String> =
    plan.files.iter().map(|f| path_key(&f.rel)).collect();
```

Keep `expected_paths()` for callers/tests that want the raw spelling. If a
public iterator is preferred, add `expected_rels() -> impl Iterator<Item = &str>`
and key it at the purge boundary.

**Contract notes:** the purge's keep set must stay exactly the plan's cleaned
rels; do not accidentally include blacklisted/out-of-scope files.

---

### D3 — `build_plan` pays for `layout()` just to warn

**Risk:** Low · **Kind:** performance · **Files:**
`src/repair/phases/plan.rs:57-102`, `src/repair/phases/plan.rs:322-334`

`build_plan` sorts each file's chunks by offset (line 295), then calls
`file.layout()` for every file only to warn when the layout is not tiled.
`layout()` allocates a `Vec<(u64, u64)>`, sorts it again, and builds
`gaps`/`overlaps` vectors. The in-place writer later calls `layout()` again for
the gaps it actually needs.

**Suggested shape:** add a cheap `PlannedFile::is_tiled()` that walks the chunks
in order (the plan guarantees sorted) with a cursor and no allocation, and use
it in `build_plan`. Keep `layout()` for the writer and for the warning's detail.
Alternatively, make `layout()` detect an already-sorted input with
`is_sorted_by_key` and skip the sort.

**Contract notes:** the warning's `gaps` / `overlaps` counts are only needed on
the non-tiled path; a cheap tiling check can gate the full `layout()` call.

---

### D4 — Clone the URL prefix once per file

**Risk:** Low · **Kind:** performance / API · **Files:**
`src/repair/phases/plan.rs:260-283`, `src/repair/file/mod.rs:387-392`

`build_plan` does `prefix.insert(rel.clone(), url_prefix.clone())` for every
file in a manifest. A manifest's files all share one prefix, so this clones the
same string thousands of times. `repair_one_file` then clones it again out of
the map into a local `String`.

**Suggested shape:** store `Arc<str>` (or `Arc<String>`) in
`RepairPlan::url_prefix_by_file`, and pass `&str` to the attempt functions
(already the case). Longer term, put `url_prefix: Arc<str>` on `PlannedFile` and
drop the map, which also removes the duplicate `rel` key. The map is public and
constructed in tests, so the `Arc` value is the smaller first step.

**Contract notes:** the prefix is server data and must be carried unchanged; the
first manifest wins on a duplicate rel (pinned by
`plan_dedups_across_manifests_and_drops_folders`).

---

### D5 — Deduplicate `verify_plan`'s scan and fallback branches

**Risk:** Medium · **Kind:** DRY / maintainability · **Files:**
`src/repair/phases/check.rs:27-145`

`verify_plan` has two nearly identical `match` arms:

- with a scan verdict: path error, `file_len` (unreadable/absent/size), then
  compare `v.file_md5` to `f.md5` and count `v.damaged_chunks()`;
- without: the same path/`file_len` checks, then `util::md5_file(path)` and
  compare.

The log messages and fields are duplicated, and `v.damaged_chunks()` is called
twice in the scan arm (once for the counter, once for the log field). The
`files_skipped` update uses `(i + 1) - bad.min(i + 1)`, which is just “clean so
far” written obliquely.

**Suggested shape:** one helper that takes the digest source:

```rust
enum DigestSource<'a> {
    Scanned(&'a str),
    Read(Result<String>),
}

fn check_one(f: &PlannedFile, path: &Path, digest: DigestSource<'_>) -> CheckOutcome
```

`CheckOutcome` carries `ok`, `damaged_chunks: Option<u64>`, and the failure
shape (path escape, unreadable, absent, size, md5) so the caller emits one warn
site. Track a `clean` counter instead of deriving it from `bad`. Compute
`damaged_chunks()` once.

**Contract notes:** this is behavior-adjacent because the exact `tracing` fields
(`expect_size`, `actual_size`, `expect_md5`, `actual_md5`, `damaged_chunks`) and
the `files_skipped` / `files_failed` counters are observable. Keep the messages
and the “scan is a shortcut for the read, not for presence/length” rule.

---

### D6 — Count and drop empty chunks in one pass

**Risk:** Low · **Kind:** micro-optimization · **Files:**
`src/repair/phases/plan.rs:309-321`

`build_plan` counts `uncompressed_size == 0` chunks with a filter, then calls
`retain` in a second pass. One pass is enough:

```rust
let before = file.chunks.len();
file.chunks.retain(|c| c.uncompressed_size > 0);
let empty = before - file.chunks.len();
if empty > 0 { tracing::warn!(...); }
```

**Contract notes:** validation still runs **before** the drop, so a negative size
is refused rather than silently removed (pinned by
`malformed_chunk_ranges_are_refused_at_plan_time`).

---

## E. Purge

### E1 — Call `scope.is_visible` once per file

**Risk:** Low · **Kind:** performance / DRY · **Files:**
`src/repair/purge.rs:155-190`, `src/repair/purge.rs:398-427`

For every file, `plan_purge` computes:

```rust
let outside = !scope.is_visible(&rel);
match classify_purge_path(&rel, &expected, &server_keep, &blacklist, scope) { ... }
```

and `classify_purge_path` starts with `if !scope.is_visible(rel) { return Keep }`.
So the glob set is matched twice per file. The `outside` value is only used to
increment `hidden` and for a `debug_assert!`.

**Suggested shape:** compute `visible` once and pass it to a variant:

```rust
pub fn classify_purge_path_visible(
    rel: &str,
    visible: bool,
    expected: &HashSet<String>,
    server_keep: &HashSet<String>,
    blacklist: &HashSet<String>,
) -> PurgeVerdict
```

`classify_purge_path` remains the public pure wrapper that computes `visible`
for tests. In `plan_purge`, use `visible` for the directory branch and the
`hidden` count, and the variant for files.

**Contract notes:** rule 0 (“out of scope → keep”) is load-bearing and must stay
first. The `hidden` count and the warning are observable; preserve them.

---

### E2 — Purge keep-sets allocate a new `String` per path

**Risk:** Medium · **Kind:** performance / API · **Files:**
`src/repair/purge.rs:98-103`, `src/repair/purge.rs:316-322`

`plan_purge` builds three `HashSet<String>`s by mapping every path through
`purge_key`, which allocates even on Unix where the key is the path itself.
Combined with D2, a large plan allocates the expected set twice and the walked
rel once per candidate.

**Suggested shape:** start with D2 (remove the intermediate set) and `Cow`:

```rust
fn purge_key(rel: &str) -> Cow<'_, str> {
    if cfg!(windows) { Cow::Owned(rel.to_lowercase()) } else { Cow::Borrowed(rel) }
}
```

The sets can then be `HashSet<Cow<'_, str>>` scoped to `plan_purge`, with
lifetimes tied to the plan/keep inputs. A stronger version is a `PathKey`
newtype with case-insensitive `Hash`/`Eq` on Windows, but that changes the
public `classify_purge_path` signature and is not needed for the first win.

**Contract notes:** `classify_purge_path`'s documented contract says the sets
must already be keyed; keep that. Do not change the rule order or the
`is_audio_version_file` matching.

---

## F. Reporting

### F1 — `Summary` has three parallel field lists

**Risk:** Low · **Kind:** DRY / maintainability · **Files:**
`src/report/summary.rs:92-147`

`Summary::new`, `impl Default`, and `Summary::snapshot` each list every field.
Adding a counter requires three edits, and forgetting `snapshot` silently loses
the value after the tasks are joined. The only reason `Default` is hand-written
is `ram_limit`'s `u64::MAX` sentinel; `Meter` already hand-writes its `Default`
for the same reason (`src/repair/scope/ledger.rs:134-140`).

**Suggested shape:** introduce a small newtype for the sentinel:

```rust
#[derive(Debug)]
struct Ceiling(AtomicU64);
impl Default for Ceiling { fn default() -> Self { Self(AtomicU64::new(u64::MAX)) } }
```

Then `ram_limit: Ceiling` (or reuse `Meter`'s limit machinery) and
`#[derive(Default)]` for `Summary`; `new()` becomes `Self::default()`.
`snapshot` remains explicit because it must load each atomic, but it is now the
only field list that can drift. If the field count keeps growing, a
`summary_fields!` macro that generates the struct, `new`, and `snapshot` is the
alternative; prefer the newtype until the macro earns its complexity.

**Contract notes:** `ram_limit` must still default to `u64::MAX` (no ceiling),
and `snapshot` must preserve the stop marker and both ceilings. The
`the_stop_marker_survives_a_snapshot` test is the guard.

---

### F2 — `conditional_extras` is a hand-ordered ladder

**Risk:** Low · **Kind:** maintainability · **Files:**
`src/report/summary.rs:265-320`

The function is the single place that defines the conditional `SUMMARY` tail,
and its order is a contract. That is good. The cost is a long sequence of
`if let Some(...) { out.push(Extra::Num(...)) }`, with one grouped case
(`repair_mode` emits four keys) and one combined condition (`scoped ||
config_bumped_zero`). Adding a key means remembering the exact position.

**Suggested shape:** keep the order explicit, but reduce the noise with a small
builder:

```rust
struct Extras(Vec<Extra>);
impl Extras {
    fn num(&mut self, k: &'static str, v: u64) { self.0.push(Extra::Num(k, v)); }
    fn word(&mut self, k: &'static str, v: &'static str) { self.0.push(Extra::Word(k, v)); }
}
```

Optionally split `SummaryFlags` into `LimitFlags` / `RepairFlags` and have
`conditional_extras` delegate to two ordered sub-functions. Do **not** reorder
the existing keys; the `report::tests` assert the line shape.

**Contract notes:** an unflagged run's tail must stay empty, and every key must
remain conditional on its flag.

---

### F3 — `Record` rendering allocates per field

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`src/report/record.rs:141-229`

`Record::text` calls `bare(v)`, which returns a new `String` (cloning `Val::Str`),
and `Record::json` calls `json_string` for every key and value, each returning a
new `String`. `event()` also allocates a lowercase `String` on every render.
Reports are one line per event, so this is not hot, but the helpers are easy to
make allocation-free:

- `bare` can return `Cow<'_, str>`;
- `json_string` can become `push_json_string(&mut String, &str)`;
- `json_value` can push directly into the output;
- `event()` can be computed once (or the label can carry its lowercase form).

**Contract notes:** the rendered text and JSON must stay byte-identical,
including the `Val::UInt` vs `Val::Int` distinction and the JSON escaping path.

---

## G. Utilities

### G1 — `RangeState::is_ok` formats a hex digest per range

**Risk:** Low · **Kind:** performance · **Files:** `src/util/md5.rs:62-85`,
`src/util/md5.rs:118-188`

`scan_reader` calls `is_ok` for every destination and every source range. Each
call does:

```rust
format!("{:x}", self.ctx.clone().compute()) == self.expect
```

That is a 32-byte `String` allocation plus a string compare per range. The
expected digest is already lowercased in `intern`; parse it once into `[u8; 16]`
and compare the computed digest bytes:

```rust
struct RangeState {
    ...
    expect: Option<[u8; 16]>,   // None = empty/unknown
}

fn is_ok(&self) -> bool {
    self.seen == self.len
        && self.expect.is_some_and(|e| self.ctx.clone().compute().0 == e)
}
```

(`md5::Digest` is a tuple struct over `[u8; 16]`; compare `.0` or use the
crate's `as_slice`.) This keeps the “empty MD5 is unknown, not matches” rule.

**Contract notes:** the scan's `dest_ok` / `source_ok` verdicts and the
case-insensitive expectation must not change. The existing
`the_expected_md5_is_compared_case_insensitively` and
`an_empty_expectation_is_never_correct` tests are the guards.

---

### G2 — Compare MD5 as a digest, not a hex string

**Risk:** High · **Kind:** API / performance / consistency · **Files:**
`src/util/md5.rs`, `src/repair/file/temp.rs:48,274,457,526`,
`src/repair/file/inplace.rs:142,249,333,372`, `src/repair/phases/plan.rs:170-172`,
`src/sophon/map.rs:45`

Every gate compares 32-character hex strings with `eq_ignore_ascii_case`, and
every computed digest is formatted with `format!("{:x}", ...)`. The strings come
from the protobuf/JSON boundary (`SophonChunk.uncompressed_md5`,
`PlannedFile.md5`), so a full `Digest([u8; 16])` change is broad.

**Suggested shape:** a newtype with the boundary conversions:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Md5([u8; 16]);

impl Md5 {
    pub fn parse_hex(s: &str) -> Result<Self>;  // accepts either case
    pub fn to_hex(self) -> String;              // lowercase
}
```

Parse at the manifest/plan boundary (or keep the `String` fields and parse once
into a side table), and use `Md5` in the scan's `RangeState`, the writer's gates,
and the reuse keys. This removes case-folding from every comparison and makes
“an empty MD5 is unknown” a `Option<Md5>` rather than a string sentinel.

**Contract notes:** this touches every correctness gate. Do it as a separate,
mechanical series after G1, with the existing MD5/scan/writer tests unchanged.
Keep the public `SophonChunk`/`PlannedFile` fields as `String` until the
boundary parse is proven, so the wire/JSON shape does not move.

---

### G3 — Compile the `game_version` regex once

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`src/util/ini.rs:20-33`

`read_game_version` calls `regex::Regex::new(...)` on every invocation. It is
called once per run in production, but the test suite calls it often, and the
pattern is a literal. Use a `OnceLock<Regex>`:

```rust
static GAME_VERSION: OnceLock<regex::Regex> = OnceLock::new();
let re = GAME_VERSION.get_or_init(|| {
    regex::Regex::new(r"(?m)^game_version\s*=\s*(.+?)\s*$").expect("literal")
});
```

Alternatively, replace the regex with a line scan. If you do, preserve the exact
semantics: `^` anchors at the line start (no leading whitespace before
`game_version`), `\s*` around `=`, at least one character of value, trailing
whitespace trimmed, and **last match wins** (the documented Starward parity).
The manual version is easy to get subtly wrong; the `OnceLock` is the safer
first step.

**Contract notes:** do not change last-match-wins or the absent-vs-unreadable
split.

---

### G4 — Write `config.ini` lines directly into the output buffer

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`src/util/ini.rs:370-382`, `src/util/ini.rs:305-307`

The serializer does:

```rust
out.push_str(&format!("[{}]\n", s.name));
for (k, _, v) in &s.keys {
    out.push_str(&format!("{k}={v}\n"));
}
out.push('\n');
```

Each line allocates a temporary `String`. Use `std::fmt::Write` (aliased to avoid
the `std::io::Write` import at the top) and `writeln!`, or push the pieces
directly. The file is small, so this is cleanliness more than speed.

The read side also does `if path.exists() { let text = read_to_string(&path)?; }`.
A single `match read_to_string` with `NotFound => empty` avoids the double stat
and the TOCTOU window:

```rust
let text = match std::fs::read_to_string(&path) {
    Ok(t) => t,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
    Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
};
```

**Contract notes:** the output must stay byte-identical (section order, sorted
keys, blank line after each section, `sdk_version` creation, fsync + rename).
The existing idempotence tests are the guard.

---

### G5 — Cache the resolved base in `contained_join`

**Risk:** Medium · **Kind:** performance / security-adjacent · **Files:**
`src/util/path.rs:150-216`, callers in `src/repair/scan.rs`,
`src/repair/phases/plan.rs`, `src/repair/file/*`, `src/repair/scope/report.rs`

`contained_join(base, rel)` calls `resolve_existing_prefix(base)` on every
invocation. That walks up from `base` until `canonicalize` succeeds — normally
one `canonicalize(base)` syscall — and then does the same for the joined path.
It is called per file by the scan estimate, the scan itself, the in-place
writer, `repair_one_file`, `DiskPlan::new`, and the purge. For a 100k-file plan
that is 100k+ redundant resolutions of the same base.

**Suggested shape:** introduce a resolved-root type:

```rust
pub struct PathRoot {
    base: PathBuf,
    resolved: PathBuf,
}

impl PathRoot {
    pub fn new(base: &Path) -> Result<Self>;      // resolve once
    pub fn join(&self, rel: &str) -> Result<PathBuf>; // resolve only the joined path
}
```

Construct it once per run (and refresh after `create_dir_all` if the base did
not exist during the plan phase). Keep `contained_join` as the convenience
wrapper for tests and one-off callers.

**Contract notes:** the joined path must still be resolved and compared with
`starts_with(resolved_base)`, so a symlink inside the install that points out is
still refused. Do not replace the filesystem check with a lexical
`starts_with`. The missing-base lifecycle (`--print-plan` before `game_dir`
exists) must keep working; `PathRoot::new` can store the resolved existing
prefix exactly as `resolve_existing_prefix` does today.

---

### G6 — `clean_rel` normalizes the path twice

**Risk:** Medium · **Kind:** performance / clarity · **Files:**
`src/util/path.rs:62-128`

`clean_rel` calls `contained_rel`, which does `rel.replace('\\', "/")`, builds a
`PathBuf` from the components, and then `clean_rel` does
`to_string_lossy().replace('\\', "/")` again. For every manifest path this is
two full-string passes plus a `PathBuf` allocation.

**Suggested shape:** build the cleaned `String` directly while validating the
components:

```rust
let mut clean = String::new();
for c in Path::new(&normalized).components() {
    match c {
        Component::Normal(part) => {
            let part = part.to_str().with_context(...)?;
            // ... same validations ...
            if !clean.is_empty() { clean.push('/'); }
            clean.push_str(part);
        }
        Component::CurDir => {}
        _ => bail!(...),
    }
}
```

`contained_rel` can keep returning `PathBuf` for its public callers; `clean_rel`
can share the component-walk helper without the second replace. Also,
`is_windows_device_name` allocates with `to_ascii_uppercase()` per component;
`eq_ignore_ascii_case` against the fixed names (and a small COM/LPT check) avoids
that allocation.

**Contract notes:** the accepted/rejected spellings and the cleaned output are
pinned by `contained_rel_accepts_only_plain_relative_paths` and
`contained_rel_refuses_windows_device_and_alias_names`. Keep the
`/`-separated output exactly.

---

### G7 — `md5_file_slice` cleanup; `read_range` bounds

**Risk:** Low · **Kind:** cleanup / robustness · **Files:**
`src/util/md5.rs:219-266`

Three small things:

1. `md5_file_slice` has `use std::io::Seek; use std::io::SeekFrom;` inside the
   function, but the module already imports both. Remove the inner imports.
2. `let mut buf = vec![0u8; MD5_BUF.min(1 << 20)];` — `MD5_BUF` is 512 KiB and
   `1 << 20` is 1 MiB, so the `min` is always `MD5_BUF`. Use `MD5_BUF` directly,
   or raise the buffer intentionally and document why.
3. `read_range` does `vec![0u8; len as usize]`. `len` is server-controlled
   (`uncompressed_size`) and `as usize` truncates on 32-bit. Use
   `usize::try_from(len)` and `Vec::try_reserve_exact`, returning an error on
   failure, so a corrupt manifest cannot abort the process with an allocation
   failure. The RAM budget (`--max-ram-bytes`) is the intended guard, but it is
   optional; a `try_reserve` is the backstop.

**Contract notes:** `read_range` must still return the exact bytes or an error;
a short read is already an error via `read_exact`.

---

## H. Orchestration

### H1 — Split `repair::run` into plan and execute phases

**Risk:** High · **Kind:** structure / maintainability · **Files:**
`src/repair/mod.rs:75-530`

The module doc says `run` “holds no logic of its own beyond phase order”, but the
function is ~455 lines and contains:

- metadata + begin report;
- manifest fetch + local reuse map;
- plan build + purge plan + deprecated fetch + optional scan;
- estimate computation + `SUMMARY` flag setup + preflight ceilings;
- `--print-plan` and `--plan-out` rendering;
- `create_dir_all`, the single-writer lock, the audio scan write;
- `--purge-before`, `--check-only`, `--purge-only` early returns;
- the repair loop and the post-phase;
- the `SUMMARY` byte/stop flag assembly and exit-code mapping.

That is the whole pipeline in one function. It is hard to test in pieces and easy
to break an early-return ordering.

**Suggested shape:** extract three units with explicit outcomes:

```rust
struct PlanPhase { meta: Meta, plan: Arc<RepairPlan>, purge: Option<PurgePlan>, scan: Option<ScanReport>, deprecated: Vec<String>, est_by_file: Vec<u64>, ... }
async fn plan_phase(ctx: &RunCtx) -> Result<PlanPhase, RunFailure>;
async fn execute_phase(ctx: &RunCtx, plan: PlanPhase) -> Result<(Summary, i32), RunFailure>;
fn apply_summary_flags(ctx: &RunCtx, summary: &mut Summary);
```

Keep the phase order and the early returns exactly as they are; the existing
integration tests are the guard. Land this as a mechanical move (no logic
changes) so a bisect can distinguish “moved” from “changed”.

**Contract notes:** the plan half must still leave the disk untouched; the
execution half must still create `game_dir` only after every refusal. The
`--print-plan` return and the `--check-only` / `--purge-only` returns are part of
the exit-code contract.

---

### H2 — Move the `SUMMARY` flag assembly out of `run`

**Risk:** Medium · **Kind:** structure / maintainability · **Files:**
`src/repair/mod.rs:453-504`, `src/report/summary.rs`

The block from “The byte figures, when their ceilings were asked for” through the
`ram_bytes` / `stopped` handling is a sequence of `if ctx... { summary.flags... }`
checks. It is the one place that knows the conditional-key rules, and it sits in
the middle of the coordinator.

**Suggested shape:** `Summary::apply_run_flags(&mut self, ctx: &RunCtx)` (or a
free `fn finalize_flags(ctx, &mut summary)` in `report`). Keep the conditions
exactly as they are; the point is to make the “only when the flag was passed”
rule reviewable in one small function. This composes with F1/F2.

**Contract notes:** an unflagged run must still emit the same `SUMMARY` line; the
`stopped=` key must still name the flag that bound.

---

### H3 — One helper for `summary.flags.stopped`

**Risk:** Low · **Kind:** DRY · **Files:** `src/repair/mod.rs:493-503`,
`src/repair/mod.rs:539-550`

The runtime-stop branch sets `summary.flags.stopped = Some(flag)`, and `finish`
sets it to `"--max-delete-bytes"` for the post-phase ceiling. A single
`fn note_stopped(summary: &mut Summary, flag: &'static str)` would keep the key
and the spelling in one place, and make it harder to add a new ceiling without
the key.

**Contract notes:** the first ceiling to bind still wins; do not overwrite an
existing `stopped` value.

---

### H4 — Consider a `RunMode` enum for `RunCtx`

**Risk:** Medium · **Kind:** design / API · **Files:** `src/config.rs:101-192`

`RunCtx` carries `check_only`, `purge_only`, `dry_run`, `print_plan`, `scan`,
`fail_fast`, `purge_before`, `purge_after`, `audio_explicit`, and
`repair_mode_explicit`. Some combinations are mutually exclusive or imply others
(`purge_only` implies the cleanup; `readonly()` is `dry_run || check_only`;
`print_plan` is read-only by construction but is not in `readonly()`). The
current design works because every gate is explicit, but it is easy to add a new
flag and forget one of the gates.

**Suggested shape:** a primary `RunMode` enum
(`Repair`, `Check`, `PurgeOnly`, `PrintPlan`, `DryRun`) plus orthogonal booleans
for genuinely independent choices (`scan`, `fail_fast`, `purge_before`,
`purge_after`, explicit-flag markers). Keep `readonly()` as a derived method on
the mode. This is a broader API change; do it after H1, when the phase
boundaries are explicit.

**Contract notes:** the CLI mapping and every exit code must stay the same. This
is a design option, not a prerequisite for the other findings.

---

## Suggested implementation order

The order below is **low risk to high risk**, so each step lands with a green
suite before the next one starts.

### Phase 0 — pure cleanup (no behavior change)

1. **D1** — one `case_key` helper.
2. **D6** — one-pass empty-chunk count.
3. **G7** — remove dead imports / dead `min`; add `read_range` bounds.
4. **C3** — extract the in-place success block.
5. **C7** — extract the reserved-write helper.
6. **G3** — `OnceLock` the version regex.
7. **G4** — direct `writeln!` into the config buffer.
8. **F3** — allocation-free record rendering.
9. **A3** — count-then-materialize salvage order.
10. **H3** — one `stopped` setter.

### Phase 1 — low-risk performance

11. **B1** — `Arc<RepairPlan>` + index in `scan_plan`.
12. **C1** — per-file scratch buffers.
13. **C2** — one source handle in `try_reuse_slice`.
14. **B2 / C4** — borrowed chunks in `salvage_order`.
15. **G1** — byte-digest compare in `RangeState::is_ok`.
16. **D3** — cheap tiling check in `build_plan`.
17. **D4** — `Arc<str>` URL prefixes.
18. **E1** — one `is_visible` per purge file.
19. **D2** — key the expected set directly in `plan_purge`.

### Phase 2 — behavior-adjacent

20. **A1** — one chunk-identity predicate, normalized MD5.
21. **A2** — indexed per-file reuse entries.
22. **B3** — shared scan-range construction; fix the in-place offset-0 sources.
23. **D5** — deduplicate `verify_plan`'s branches.
24. **F1 / F2** — `Ceiling` newtype + `Extras` builder.
25. **G6** — single-pass `clean_rel`.
26. **G5** — `PathRoot` cached base resolution.
27. **C5** — `OsString` `_tmp` path.
28. **C6** — bounded `JoinSet` in `repair_all`.
29. **H2** — `apply_run_flags`.

### Phase 3 — structural / broad API

30. **H1** — split `repair::run` into plan/execute (mechanical move first).
31. **G2** — `Md5` digest newtype across the gates (separate series).
32. **H4** — `RunMode` enum (optional, after H1).

## What not to change

These are deliberate decisions documented in the code and README. A refactor
that “fixes” them is a regression:

- **S1** — manifests are streamed and parsed, not cached in `game_dir`.
- **S2** — reuse is same-file-only; cross-file dedup is not implemented.
- **Last-match-wins** `game_version` parsing (Starward parity).
- **`chuncks`** protobuf field spelling.
- **Audio filter parity** with Starward (broad `audio_lang_*` /
  `Audio_*_pkg_version` matching).
- **Purge’s missing union** with dispatcher/SDK/WPF manifests (documented v1
  gap); do not “complete” the expected set without reading the v1-limits
  section.
- **The temp path’s resume-by-length** and the final whole-file MD5 gate.
- **The in-place path’s “only write regions that were already wrong”** rule.
- **Path containment** checks, including the Windows device-name and
  trailing-dot/space rules.
- **Unflagged output** and conditional `SUMMARY` keys.
