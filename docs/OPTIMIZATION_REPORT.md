# Optimization & correctness research report

**Scope.** The whole `girpr` cargo workspace: the shipped `girpr` repair binary
(`src/`, `app/`, `crates/`), the two dev tools (`tools/sync` = girsync,
`tools/capture` = gircap, `tools/sled2redb`), the workspace manifests and the
test/build configuration. Roughly 135 k lines of Rust across 299 source files.

**Method.** Static reading of the production paths (plan → manifest → scan →
repair → post), the fetch/hashing layers, the report plane, the two tools, and
the build/test configuration. Findings were cross-checked against their call
sites; the few upstream facts that matter (e.g. `tracing` field evaluation,
`tracing-appender` buffering) were checked against the vendored registry source.

**How to read.** Every finding has a stable ID, one or more `path:line`
locations, an observation, an impact, and a concrete change. Findings are
*research* — nothing was implemented and no tests were changed. A few are
deliberate design trade-offs that are still worth revisiting; those say so.
"Assumptions" at the end lists what must be confirmed before implementing.

**Priorities:** **H** = large wall-clock / memory / correctness effect on a real
install; **M** = measurable; **L** = micro-optimization, only worth it in a hot
loop. Categories: **perf**, **mem**, **io**, **par**, **correct**, **fail**,
**diag**, **build**.

---

## 1. Summary

| ID | Area | Finding | Pri | Cat |
|---|---|---|---|---|
| S-01 | runtime | `--jobs 1024` builds 1025 Tokio OS threads | H | par/mem |
| S-02 | repair | file tasks do blocking disk I/O on async workers | H | par |
| S-03 | repair | chunks inside one large file are fetched strictly serially | H | par/perf |
| S-04 | fetch | three independent network phases run serially | H | par/perf |
| S-05 | purge | walk, stat and delete are single-threaded | H | par/io |
| S-06 | plan | `DiskPlan` and scan repeat the same per-file stat pass | M | io/par |
| IO-01 | scan | temp path ignores the plan-scan verdict and re-reads intact files | H | io |
| IO-02 | reuse | reuse slice is read twice (hash pass, then copy pass) | H | io |
| IO-03 | stage | staged source bytes are hashed twice | M | perf |
| IO-04 | layout | `file.layout()` is recomputed (sorting all chunks) up to 3× per file | M | perf |
| IO-05 | path | every planned file is re-validated and re-canonicalized per consumer | H | io/perf |
| IO-06 | config | `game_dir` is canonicalized 3–4× per run | L | io |
| IO-07 | post | `create_dir_all` called for every audio-cache file | M | io |
| IO-08 | post | audio-cache move collects all paths before deciding | M | mem |
| MEM-01 | fetch | 4 × 64 MiB decoder buffers allocated up front from untrusted hints | H | mem |
| MEM-02 | purge | `path_key` allocates a `String` per walked file and 3 keyed copies of the plan | M | mem |
| MEM-03 | hash | a fresh 512 KiB buffer is allocated and zeroed per file/pass | M | mem/perf |
| MEM-04 | hash | read buffers are zero-filled immediately before being overwritten | M | perf |
| MEM-05 | fetch | JSON node is deserialized, deep-cloned, then deserialized again | M | mem/perf |
| NET-01 | fetch | no HTTP timeout or connect timeout is configured | H | fail |
| NET-02 | fetch | `get_node` never checks HTTP status | H | correct |
| NET-03 | fetch | body-read errors are fatal instead of retryable | M | correct |
| NET-04 | fetch | deterministic JSON parse failures are retried 5× (~15 s) | M | perf/fail |
| NET-05 | fetch | manifest decode has no decoded-size ceiling | H | correct/mem |
| NET-06 | fetch | server-supplied blob IDs are joined into URLs unvalidated | H | correct |
| NET-07 | fetch | blocking file writes inside the async streaming path | H | par |
| COR-01 | exit | lock contention and `create_dir_all` failures mapped to usage (exit 1) | H | fail |
| COR-02 | check | `--check-only --scan` trusts a stale scan verdict after in-run modification | M | correct |
| COR-03 | reuse | negative/oversized local-manifest offsets silently clamped | M | fail |
| COR-04 | temp | reuse-path I/O errors swallowed into debug logs | M | fail |
| COR-05 | scan | scan panics/unreadable files degrade silently | M | fail |
| COR-06 | plan | scope-typo refusal happens after a full purge walk | M | fail/perf |
| COR-07 | purge | unreadable purge candidates counted as 0 bytes and never surfaced | M | correct/fail |
| COR-08 | temp | `promote_tmp` stats/hashes by path then renames by path (TOCTOU) | M | correct |
| COR-09 | purge | legacy lock keep-rule is root-only but undocumented | L | correct |
| LOG-01 | logging | per-chunk `debug!`/`trace!` events are enabled in release by default | H | diag/perf |
| LOG-02 | logging | `tracing` `%` fields eagerly evaluate expressions even when disabled | M | diag/perf |
| LOG-03 | logging | file writer is lossy and never reports dropped lines | M | correct/diag |
| MIC-01 | digest | `Md5::parse_hex` redone per chunk lookup and per verification | M | perf |
| MIC-02 | path | glob visibility re-matched per file instead of inherited per directory | M | perf |
| MIC-03 | sort | deep directory comparator recomputes `components().count()` per comparison | M | perf |
| MIC-04 | fmt | `urlencode` allocates a temporary `String` per escaped byte | L | perf |
| MIC-05 | fmt | `hyp_base`/`sophon_base` clone `String` per request | L | mem |
| MIC-06 | retry | `Arc<Mutex<Option<S>>>` in the retry loop is uncontended | L | perf |
| MIC-07 | io | `write_line` issues two `write_all` calls per record | L | io |
| MIC-08 | digest | `Md5::parse_hex` allocates an error for a normal miss | L | perf |
| MIC-09 | hash | range interning allocates and sorts a `Vec` per scan | L | perf |
| MIC-10 | hash | `RangeState::is_ok` clones the whole MD5 context per verdict | L | perf |
| MIC-11 | reuse | `FileReuse::key` builds a fresh `DefaultHasher` per lookup | L | perf |
| MIC-12 | fetch | deprecated list collected twice | L | mem |
| B-01 | build | `chrono` pulled in for one log-filename stamp | L | build |
| B-02 | build | duplicate transitive crate versions in the lockfile | L | build |
| TOOL-01 | gircap http | every response body chunk is copied (`Bytes` → `Vec`) | H | perf/mem |
| TOOL-02 | girsync | per-file `stat` despite `DirEntry` metadata | H | io |
| TOOL-03 | gircap verify | whole-blob buffering + double zstd decode per blob | H | mem/perf |
| TOOL-04 | gircap | capture decodes every blob twice (measure, then assemble) | H | perf |
| TOOL-05 | gircap | repeated linear scans for build ids per admitted path | M | perf |
| TOOL-06 | gircap | `build_manifest` matrix rescans admitted list per row | M | perf |
| TOOL-07 | gircap http | Content-Type parsed/allocated twice per request | M | perf |
| TOOL-08 | gircap http | `header_map` allocates a `Vec` per header name | M | mem |
| TOOL-09 | gircap http | base request headers deep-cloned per logged request | M | mem |
| TOOL-10 | gircap http | `Fetcher` clone deep-copies header maps per chunk task | M | mem |
| TOOL-11 | gircap | redundant `Option<String>` clones per measured blob | L | mem |
| TOOL-12 | gircap | full manifest decode serialized twice | M | perf |
| TOOL-13 | gircap | every file canonicalized 4× per close-out | H | io |
| TOOL-14 | gircap verify | per-blob A6 work fully serial | M | par |
| TOOL-15 | gircap | capture assembly of independent captures is serial | M | par |
| TOOL-16 | girsync | fresh 512 KiB buffer per hashed file | M | mem/perf |
| TOOL-17 | gircap | `rel()` allocates `Vec<Cow>` + String per entry per pass | M | mem |
| TOOL-18 | sled2redb | whole migration staged in one giant redb transaction | M | mem/correct |
| TOOL-19 | gircap | canonical dedup can drop the real file for a symlink alias | H | correct |
| TOOL-20 | gircap | debug-only path round-trip assertion per file write | L | perf |
| TOOL-21 | gircap hash | declared-size mismatch reported as a decode-limit error | M | fail |

---

## 2. Scheduling & parallelism

### S-01 — `--jobs 1024` builds 1025 Tokio worker threads
**Where:** `src/main.rs:91-98`, `src/cli.rs:26-38`.
`worker_threads(available, jobs) = available.max(jobs.saturating_add(1))`, and
`MAX_JOBS = 1024`, so the largest accepted flag creates a runtime with 1025 OS
threads. The comment at `cli.rs:31-34` already calls this "a real startup and
stack cost".
**Impact:** ~1000 thread stacks and scheduler churn for a file window whose
concurrency is already bounded by `buffer_unordered(jobs)`. Several phases
(check, scan, purge) do their work on `spawn_blocking`, so those workers are
almost idle; only the repair loop (S-02) uses them for file work.
**Change:** size the runtime independently of the file window (e.g.
`available.max(2) + 1`, or even `available.max(2)`), keep `jobs` as the
`buffer_unordered` bound, and update the test at `main.rs:190-195` that asserts
`worker_threads(1, jobs) > jobs`.
**Assumption:** the repair tasks switched to `spawn_blocking` (S-02) need no
worker thread per file; verify no async future inside a repair task needs a
worker while all file tasks are blocked.

### S-02 — Repair file tasks do synchronous disk I/O on async workers
**Where:** `src/repair/file/mod.rs:418-420` (`tokio::spawn(spawn(idx))`),
`:477-534` (`repair_one_file`), `inplace.rs:237,562,606`, `temp.rs:96-184`.
The scan phase correctly uses `spawn_blocking` (`scan.rs:292-301`), but the
repair phase spawns each file task with `tokio::spawn` and performs long,
synchronous `std::fs` reads/writes/seeks on the runtime worker.
**Impact:** a multi-second whole-file scan or verify blocks a runtime worker;
with `--jobs` near the worker count, network polling and progress ticks starve.
This is why S-01's oversized runtime was chosen, so the two findings should be
fixed together.
**Change:** offload the synchronous scan/stage/verify/copy portions to
`spawn_blocking` (or `tokio::fs`), keeping only `stream_chunk_to` async. Same
issue in `app/fetch/src/sophon/stream.rs:505,535-540` (NET/S-09 below).
**Assumption:** `reqwest` futures are `Send` and can be awaited outside a
blocking task; the async fetch can be split from the blocking decode/write.

### S-03 — Chunks inside one file are fetched strictly serially
**Where:** `src/repair/file/mod.rs:56-57` (explicit design note),
`inplace.rs:498-531`, `temp.rs:126-174`.
Chunks write to disjoint offsets, yet fetch + zstd decode + write are awaited
one at a time. `--jobs` only parallelizes across files.
**Impact:** for one large file with many small chunks — an important case for a
low-disk patcher — network latency and decompression cannot overlap.
**Change:** for non-overlapping layouts and the staged/discard tiers, fetch and
decode a bounded window with `buffer_unordered` and write at precomputed
offsets using positional I/O. Keep the strict ordered tier where salvage
dependencies require it. The write-ceiling reservations are already per-chunk
and atomic, so they remain exact.
**Assumption:** chunk writes to disjoint ranges are independent; the ceiling
ledger tolerates out-of-order settlement.

### S-04 — Three independent network phases run serially
**Where:** `src/repair/mod.rs:209-212`, `:305-332`, `:400`, `:1172-1173`.
`load_manifests_and_reuse` awaits `manifest::fetch_latest` (up to 4 concurrent
fetches) and *then* `manifest::local_reuse_map` (another up to 4).
`fetch_deprecated` is a third independent request issued later in
`build_and_assert_plan`. None depends on another's output.
**Impact:** wall-clock latency is the sum of three network phases instead of
roughly their max — the largest avoidable delay in the plan half on a
high-latency link.
**Change:** start all three together with `tokio::join!`/`try_join!` in
`plan_phase`, sharing one client and one combined concurrency budget (total
4–8 in flight) rather than two independent caps of 4.
**Assumption:** `deprecated_files` is safe to fetch before the local-reuse map
is built (it is currently best-effort and independent).

### S-05 — Purge walk, per-candidate stat, and delete are single-threaded
**Where:** `src/repair/purge.rs:82` (one `WalkDir`), `:137-161` (classify +
`e.metadata()` per candidate), `:254-291` (delete loop), `:299-313`
(empty-dir removal). All run on one thread inside `spawn_blocking_traced`.
**Impact:** a purge of tens of thousands of orphan files is dominated by
serialized `metadata()` and `remove_file()` syscalls.
**Change:** parallelize the per-candidate stat and delete with bounded scoped
threads (or `rayon`, already in the tree). To keep the documented deterministic
log order, collect `(candidate, Result<...>)` into index slots and emit lines in
plan order after the parallel work.
**Assumption:** `remove_file` on distinct paths is safe concurrently; the
empty-directory pass must run after all deletes.

### S-06 — `DiskPlan` and scan repeat the same per-file stat pass
**Where:** `src/repair/scope/report.rs:83-123`, `src/repair/scan.rs:382-384`,
`src/repair/mod.rs:245,400-407`.
`DiskPlan::new` sequentially `symlink_metadata`s every planned file on the
blocking pool. When `--scan` is set, `scan_one` already stats the same file.
The two blocking tasks are awaited one after the other.
**Impact:** a serial O(n) syscall pass that can overlap with the scan or be
eliminated.
**Change:** feed the scan's length/mtime/is-file into `DiskFigure::Measured`,
or hold both `JoinHandle`s and await them together; otherwise parallelize by
index shards like check/scan.

---

## 3. Disk I/O, hashing & path handling

### IO-01 — Temp path ignores the plan-scan verdict and re-reads intact files
**Where:** `src/repair/file/temp.rs:35-55`; `FileCtx.scan` at `file/mod.rs:459`;
in-place consumer at `inplace.rs:213-239`.
`attempt_temp` does its own `file_len` + full `md5_file_digest` skip check even
when a plan scan already produced a whole-file digest.
**Impact:** with `--scan` (or on fallback to temp mode) every intact file is
read twice — doubling scan-phase I/O for no benefit.
**Change:** share the mtime/length-guarded `reusable_verdict` from `inplace.rs`
with the temp path and use its digest for the skip check; read only when no
verdict is reusable.
**Assumption:** the verdict's whole-file digest is computed over the same bytes
the temp skip check would hash; the guard is safe under the same conditions.

### IO-02 — Reuse slice is read from disk twice
**Where:** `src/repair/file/temp.rs:362-394,436-467`.
`md5_reader_slice_digest` reads the source bytes to hash them; on match the code
seeks back (`sf.seek(SeekFrom::Start(ro as u64))`, line 382) and
`copy_reuse_pieces` reads the same bytes again.
**Impact:** every reused chunk in temp mode costs 2× source reads — for a
dedup-heavy repair this is a large fraction of total I/O.
**Change:** read once into a staging buffer, hash, then write from the buffer;
for large chunks use a single read loop that hashes and spools into a buffered
sink, committing on digest match (the download path already accepts
"write before digest known" with a post-hoc gate).
**Assumption:** a bounded staging buffer is acceptable under `--max-ram-bytes`/
`--max-write-bytes`.

### IO-03 — Staged source bytes are hashed twice
**Where:** `src/repair/file/inplace.rs:400-429` (esp. `419-426`).
The scan already hashed `[sources[i], sources[i]+size)` against
`c.uncompressed_md5` (`app/chunk-verify/src/lib.rs:80-84`). `stage_sources`
reads those bytes again for staging and re-computes `md5_digest(&bytes)` before
accepting them.
**Impact:** redundant CPU hashing proportional to the staged payload, which is
deliberately bounded by the RAM budget and can be large.
**Change:** trust the scan per-range verdict and drop the second hash; if a
TOCTOU re-check is required, fold hashing into the single capture read and
document the deliberate double verification.

### IO-04 — `file.layout()` recomputes and sorts all chunks up to 3× per file
**Where:** `src/repair/file/inplace.rs:537,668,711`; implementation
`app/repair-plan/src/plan.rs:62-107`.
`patch_chunks`, `overlapping_ranges` and `redo_mismatch_is_terminal` each call
`file.layout()`, which allocates a `Vec<(u64,u64)>` and `sort_unstable`s it.
The third call is on the failure path only.
**Impact:** O(n log n) × up to 3 per file with n chunks — pure repeated work in
the hot path.
**Change:** compute `let layout = file.layout();` once in
`attempt_inplace_inner` and pass `&Layout` down.

### IO-05 — Planned rels are re-validated and re-canonicalized per consumer
**Where:** `app/util-path/src/path.rs:310-326,398-425`;
`crates/path-shape/src/lib.rs:136-182`; callers at `check.rs:110-113`,
`scope/report.rs:91-94`, `scan.rs:165-168,352-355`.
`PathRoot::join` re-runs `contained_rel` → `validate_rel` →
`girpr_path_shape::validate` (allocating `parts`, `checked`, `clean`) and
`resolve_existing_prefix` → `std::fs::canonicalize` on the joined path. The rel
was already cleaned and validated in `build_plan`
(`app/repair-plan/src/plan.rs:434-435`).
**Impact:** several syscalls and ~3 heap allocations per file *before* the
file is even stat'd, repeated in every consumer.
**Change:** add `PathRoot::join_validated(&self, clean_rel)` for plan-derived
rels that skips lexical revalidation; cache resolved parent directories in a
`HashMap<PathBuf,PathBuf>` so files in one directory pay one canonicalize.

### IO-06 — `game_dir` canonicalized 3–4× per run
**Where:** `src/repair/mod.rs:190-193,838-841`, `app/lock/src/lib.rs:110-114`.
Plan phase canonicalizes; `prepare_install` canonicalizes again; `PathRoot::new`
resolves again; `lock_path` canonicalizes a fourth time even though
`Lock::acquire` already receives the canonical path.
**Impact:** small but pure waste, and a filesystem touch before the lock is
taken.
**Change:** add a `lock_path_for_canonical` or a canonical-form flag; skip the
second `PathRoot::new` when `create_dir_all` did not change the path.

### IO-07 — `create_dir_all` per audio-cache file
**Where:** `src/repair/phases/post.rs:684` (and `check_publish_target` at
`:477-511` calling `any_component_has_windows_trailing_dot_or_space`,
`app/util-path/src/path.rs:264-269`).
**Impact:** thousands of redundant mkdir syscalls and `to_string_lossy()`
allocations when many files share a destination directory.
**Change:** keep a `HashSet<PathBuf>` of parents already created; add a
byte-based component scan for the common ASCII case.

### IO-08 — Audio-cache move materializes the whole file list first
**Where:** `src/repair/phases/post.rs:629-657`.
WalkDir results are `filter().map().collect::<Vec<PathBuf>>()` then iterated
twice. Nothing requires the full list up front.
**Impact:** memory proportional to cache size and one `PathBuf` per file in a
best-effort phase. **Change:** stream the walk into the per-file decision.

---

## 4. Memory & allocation

### MEM-01 — 4 × 64 MiB decoder buffers allocated from untrusted hints
**Where:** `app/fetch/src/sophon/stream.rs:38-39,76-85`;
`src/repair/phases/manifest.rs:42` (`MAX_MANIFEST_CONCURRENCY = 4`).
Every manifest fetch reserves the full clamped `uncompressed_size` before the
request, even for a tiny or failed blob. Four concurrent fetches reserve
256 MiB.
**Impact:** unnecessary peak memory and allocation time driven by an unchecked
server field the code itself calls "a hint, not a promise". Related: NET-05 —
the hint is also not used as a decode ceiling.
**Change:** start with a modest capacity (e.g. 64 KiB or the first streamed
chunk) and grow geometrically; keep the clamp only as a hard ceiling (NET-05).

### MEM-02 — `path_key` allocates per walked file; three full keyed copies
**Where:** `src/repair/purge.rs:426,438-443`;
`app/repair-plan/src/purge_values.rs:84-86`; `app/util-path/src/path.rs:226-232`.
`classify_purge_path_visible` allocates a `String` per walked path to probe
three `HashSet<String>`s; `PurgeKeys::new` materializes three complete keyed
copies of plan rels, keep sets and blacklist.
**Impact:** 100k throwaway allocations per purge pass plus 3× resident key
strings. **Change:** `HashSet<String>` supports borrowed lookup via
`Borrow<str>` — on non-Windows use `set.contains(rel)` with no allocation; only
fold on Windows. Consider a cheap 128-bit membership hash for the sets.

### MEM-03 — A fresh 512 KiB buffer per scan/digest call
**Where:** `app/util-hash/src/md5.rs:124-127,286-289,336-338`.
Each call does `vec![0u8; MD5_BUF]`; private `*_with_buf` helpers already exist.
Under `verify_in_place` a mismatching file allocates the buffer 2–3 times.
**Impact:** thousands of 512 KiB allocations and zero-fills across a run.
**Change:** expose the `*_with_buf` variants and keep one reusable buffer per
worker via thread-local or task-owned storage.

### MEM-04 — Read buffers are zeroed immediately before being overwritten
**Where:** `app/util-hash/src/md5.rs:251-255`, `src/repair/file/mod.rs:114-119`,
`md5.rs:336-338`. `resize(len, 0)` (or `try_reserve_exact` + `resize`) is
followed by a full `read`/`read_exact`.
**Impact:** an extra `memset` of up to 512 KiB per scratch buffer and of the
entire length of every staged chunk — 1 GiB of useless writes for a 1 GiB
staged payload. **Change:** reserve capacity and read into spare capacity
(`spare_capacity_mut` + `MaybeUninit` + `set_len`, or `Read::read_buf`).

### MEM-05 — JSON node deserialized, deep-cloned, deserialized again
**Where:** `app/fetch/src/hyp.rs:383-388`.
`w.data.get(node).cloned()` deep-clones a potentially large
`Vec<GameConfig>`/`Vec<GameBranch>` before `serde_json::from_value`.
**Impact:** two full deserializations plus allocation-heavy clone per call.
**Change:** consume owned `data` (`as_object_mut().and_then(|o| o.remove(node))`)
or deserialize into a typed wrapper directly from the body.

---

## 5. Network correctness

### NET-01 — No HTTP timeout anywhere in the fetch path
**Where:** `app/fetch/src/hyp.rs:298-302` (builder sets only `user_agent` and
`pool_max_idle_per_host(16)`).
No `.timeout()` / `.connect_timeout()` on the one client used for every JSON
call and blob stream. The retry loop (`app/fetch/src/retry.rs:105-181`) cannot
fire if a single `send()`/`bytes_stream()` never completes; the test at
`manifest.rs:519` (`Reply::NeverAfterHit`) demonstrates a hanging request.
**Impact:** a stalled CDN connection hangs the run until the OS TCP timeout
(minutes), defeating fail-fast and retry. **Change:** add connect and overall
timeouts (or a per-stream idle timeout in `stream_body`).

### NET-02 — `get_node` never checks HTTP status
**Where:** `app/fetch/src/hyp.rs:354,361`.
Unlike `get_direct` (`:412-426`) and the blob path, `get_node` reads the body
and trusts `retcode` regardless of status. A `503` with `{"retcode":0}` is
accepted; a `500`/`404` with HTML takes the fatal parse path and is never
retried. **Change:** `resp.error_for_status_ref()` before reading the body.
**Assumption:** no production endpoint legitimately returns a non-2xx body with
a usable `retcode` — verify against captured responses.

### NET-03 — Body-read failures are fatal instead of retryable
**Where:** `app/fetch/src/hyp.rs:355-359,415-419`.
`resp.bytes().await` errors return `Retry::Fatal("read body")`. A truncated
body from a dropped connection is a *transport* failure; the blob path correctly
treats it as retryable (`stream.rs:513-519`). **Change:** return `Retry::Retry`
for body-read errors in both JSON paths.

### NET-04 — Deterministic JSON parse failures retried 5× (~15 s)
**Where:** `app/fetch/src/hyp.rs:362-368` (`Fatal`) vs `:422-427` (`Retry`), and
`:450-451` (`data: null`).
Both functions fully buffer the body, so a parse error or `retcode:0,data:null`
answer is deterministic for the same URL. `get_direct` retries 5× with
exponential backoff (1+2+4+8 ≈ 15 s) for identical bytes, then reports generic
exhaustion instead of "malformed response" / `SophonNotFound`.
**Change:** make wrapper-parse errors `Fatal` in `get_direct`; map `data: null`
to a clean not-found.

### NET-05 — Manifest decode has no decoded-size ceiling
**Where:** `app/fetch/src/sophon/stream.rs:38-39,117,505` —
`stream_body(resp.bytes_stream(), None, None, decoded)` gives `expected = None`.
`meta.manifest.uncompressed_size` only sizes the initial `Vec`; it is never
passed as the decode ceiling, and there is no `max_compressed` cap. The chunk
path caps both. A compromised origin can stream an arbitrarily large zstd frame
into the `Vec` until OOM. **Change:** pass a hard `expected` cap and a
compressed-byte cap; let the existing `HashingWriter`/`stream_body` limits apply.

### NET-06 — Server-supplied blob IDs joined into URLs unvalidated
**Where:** `app/fetch/src/sophon/stream.rs:33-34,415-419,579-587`.
`base.join(&meta.manifest.id)` / `base.join(id)` takes raw manifest strings.
`Url::join` replaces the whole URL for an absolute `http://…` id and escapes a
leading `/` or `..`. No shape check exists (`path-shape` validates *file* paths,
not chunk ids). **Impact:** a corrupt/hostile manifest can redirect fetches to
another host/path. **Change:** reject empty, absolute, `://`, leading `/`,
`..`, `?`, `#`; assert the resulting URL's origin and path prefix before sending.

### NET-07 — Blocking file writes inside the async streaming path
**Where:** `app/fetch/src/sophon/stream.rs:505,535-540` — `stream_body` writes
through `HashingWriter<&mut ChunkFileSink<W>>` where `W` is `std::fs::File`;
call sites in `temp.rs:96-102,510-536,147-155` await it directly.
**Impact:** each in-flight chunk blocks a Tokio worker on disk I/O (same class
as S-02). **Change:** move decode+write to `spawn_blocking`, bridge only the
network stream into it; or document and enforce that `stream_chunk_to` must be
called from a blocking task.

---

## 6. Correctness, fail-fast & reason-for-failure

### COR-01 — Lock contention and `create_dir_all` failures reported as usage errors
**Where:** `src/repair/mod.rs:832-834,859-860`; code meanings at
`src/config.rs:40-42`.
`create_dir_all` failure (usually permission-denied or disk-full) and
`Lock::acquire` failure (commonly "another run holds the install", per
`app/lock/src/lib.rs:68-73`) both map to `RunFailure::usage` = exit 1, whose
documented meaning is "the request or the configuration is wrong".
**Impact:** automation classifies a locked install or full disk as a bad
invocation, steering retries/alarms wrongly. **Change:** map `create_dir_all` to
`RunFailure::write` (exit 3) with path and error; map lock contention to a
non-usage class whose message names the holder (`held()` already provides it).

### COR-02 — `--check-only --scan` trusts a stale scan verdict
**Where:** `src/repair/phases/check.rs:339-389`; scan at `scan.rs:379-384`;
trust documented at `scan.rs:18-23`.
The scan records `modified` from `std::fs::metadata`; `check_one` re-stats with
`util::file_len` to detect post-scan changes, but `ScanReport::get` does not
re-validate that the file's length/mtime still match. A file whose content
changes with the same length after the plan-phase scan is trusted to the stale
digest, and a `--check-only --scan` run can return `Success` for it.
**Impact:** the check answers the plan's question, not the operator's.
**Change:** document this prominently, or re-validate `modified` against the
fresh stat that `file_len` already performs and fall back to a read when it
differs.

### COR-03 — Reuse offsets from local manifest never validated
**Where:** `app/fetch/src/sophon/map.rs:176-180`, `src/repair/reuse.rs:182-191`
(`entry.offset.max(0) as u64`).
A negative offset becomes 0; a huge one becomes a range past EOF. `RangeState::is_ok`
(`md5.rs:85-90`) makes these "not intact", so the chunk is downloaded — no
corruption — but a corrupt local manifest is accepted without a diagnostic and
a bogus scan pass is paid.
**Change:** validate `offset >= 0`, `size >= 0`, `offset.checked_add(size) <=`
local file length at map-build time; warn and skip the entry. Then `.max(0)` can
be dropped.

### COR-04 — Reuse-path I/O errors swallowed into debug logs
**Where:** `src/repair/file/temp.rs:350-390,448-457`.
`File::open` failure, slice digest failure, seek failure and reuse-copy
read/truncation are all logged at trace/debug and converted to `Download`.
The concrete `io::Error` is dropped.
**Impact:** an unreadable source or failed seek is indistinguishable from
"no candidate"; downloads inflate but the report never says why.
**Change:** keep the fallback but record unexpected I/O failures in
`diagnostics` and at `info`/`warn`; reserve `debug` for the expected MD5
mismatch. Carry the error text in the fallback enum if the caller wants it.

### COR-05 — Scan failures degrade with no user-visible reason
**Where:** `src/repair/scan.rs:316-327` (panicked/cancelled task → warn +
continue), `:387-392` (unreadable file → `Scan::absent` at debug).
**Impact:** a panic leaves the file without a verdict, so it is estimated in
full and repaired without reuse; the plan/estimate silently deviates from what
`--scan` promised, with the cause only in trace logs.
**Change:** add a per-file `scan_failed` count / collected reasons to
`ScanReport` and surface them in the plan line, keeping the conservative
"estimate in full" behavior.

### COR-06 — Scope-typo refusal happens after a full purge walk
**Where:** `src/repair/mod.rs:385-398,473-506`, `app/repair-plan/src/lib.rs:100-124`.
`build_and_assert_plan` walks the entire install to fold `PurgeKeys` *before*
`check_scope_selection`. When no purge flag is set, `purge_wanted` is false,
`purge_plan` is `None` and `purge_candidates` is always 0 — so the
"--include matched nothing" error is fully decidable immediately after
`build_plan`.
**Impact:** a typo'd `--include` still pays a full install walk before exit 1.
**Change:** evaluate the early-exit case right after `build_plan` when
`!purge_wanted`; keep the full check after the walk only for purge-enabled runs
where a purge candidate can rescue the selection.

### COR-07 — Unreadable purge candidates counted as 0 bytes and never surfaced
**Where:** `src/repair/purge.rs:144-155,205-210`;
`src/repair/scope/limits.rs:121-133,150-156`.
A candidate that cannot be `stat`ed increments `errors`, logs a warn and records
`bytes = 0`. Without `--max-delete-bytes` the limit checks are no-ops, so the
run deletes a set whose `total_bytes` is a documented lower bound;
`PurgePlan::errors` never reaches `SUMMARY`.
**Change:** surface `purge_errors=N` (and any hidden-byte count) in `SUMMARY`
whenever non-zero, independent of ceilings; consider strict refusal.

### COR-08 — `promote_tmp` stats and hashes by path, then renames by path
**Where:** `src/repair/file/temp.rs:716-748` (`metadata(tmp_path)` at 723,
`md5_file_digest(tmp_path)` at 729, `rename(tmp_path, final_path)` at 748).
Three independent path lookups create a TOCTOU window: a swap between digest
and rename promotes a file that was never verified. The in-place path guards
the equivalent case by operating on an open handle (`inplace.rs:786-790`).
**Change:** hold one handle for the tmp file, stat/hash through it, then rename
(OS handle-relative rename where available).

### COR-09 — Legacy lock keep-rule is root-only but undocumented
**Where:** `src/repair/purge.rs:348-360,415-444`.
`keep_set()` stores `"girpr.lock"` as a bare rel and the classifier matches
exactly, so `sub/girpr.lock` is purged. Probably intended (the legacy file
lived at the root), but neither docs nor a test pin it.
**Change:** add a test pinning root-only matching and say so in the `keep_set`
docs, or widen the rule if subdirectories are possible.

---

## 7. Logging & diagnostic output

### LOG-01 — Per-chunk debug/trace events are enabled in release by default
**Where:** `src/repair/file/temp.rs:336-456,627,649`,
`src/repair/file/inplace.rs:202,423`, `src/repair/scan.rs:390`,
`app/fetch/src/hyp.rs:350,361,371,389,399,421`; default file level at
`src/logging.rs:12-21` and `crates/log/src/plan.rs`.
A release build installs a log file at `debug` by default. Every chunk emits a
`debug!`/`trace!` event with formatted fields (MD5 hex, paths) that is rendered
and pushed through the non-blocking worker. On a large repair with millions of
chunks this is a significant share of CPU and queue traffic — and diagnostic
output may legitimately be reduced if it skips work.
**Change:** default the file layer to `info` in release, or move per-chunk lines
to `trace` (off by default) and keep per-file lines at `debug`. Keep the data
plane (`REPORT`/`PLAN`/`PROGRESS`/`SUMMARY`) unchanged.
**Assumption:** operators who currently rely on per-chunk debug logs can opt in
via `--log-file-level debug`; the JSON/text stdout contract is unaffected.

### LOG-02 — `tracing` `%` fields eagerly evaluate expressions when disabled
**Where:** `app/fetch/src/hyp.rs:361` and `:421` build
`String::from_utf8_lossy(&body)` for **every** response in a `trace!` event;
`src/repair/file/inplace.rs:202` and `src/repair/scan.rs:390` call
`format!("{e:#}")` in disabled `debug!` events.
`tracing_core::field::display(t: T)` takes `t` by value (verified in the
vendored `tracing-core-0.1.36/src/field.rs:368-373`), so the expression is
evaluated before the callsite's level check. The lossy UTF-8 conversion of a
full JSON body therefore happens even with `--no-log-file` and console at
`error`.
**Impact:** avoidable full-body scans and `String` allocations per request,
and error-chain formatting per failure, regardless of log configuration.
**Change:** gate expensive fields behind `tracing::enabled!(Level::TRACE)` or
use a lazy wrapper, e.g. `tracing::field::debug` with a type whose `fmt` does
the conversion, or simply drop the body field.

### LOG-03 — File writer is lossy and never reports dropped lines
**Where:** `crates/log/src/install.rs:57` calls `tracing_appender::non_blocking(file)`.
The default `NonBlockingBuilder` is lossy with a 128,000-line bounded channel
(verified in `tracing-appender-0.2.5/src/non_blocking.rs:60-67,229-234`); at
capacity, events are silently dropped and only an internal counter increments.
`flush_log` (`src/logging.rs:229-235`) drops the guard without reading it.
**Impact:** under the per-chunk debug volume of LOG-01 a large run can lose log
records, contradicting "the file is the record of the run", with no signal.
**Change:** use `NonBlockingBuilder::default().lossy(false)`, or report
`dropped_lines()` on flush. Either trades a little throughput for honesty.

---

## 8. Micro-optimizations

### MIC-01 — `Md5::parse_hex` redone per lookup and per verification
**Where:** `crates/digest/src/lib.rs:101-108,124-126`; callers
`src/repair/reuse.rs:182-191`, `app/fetch/src/sophon/map.rs:112-123`,
`app/util-hash/src/md5.rs:222`, `app/fetch/src/sophon/stream.rs:129`,
`src/repair/file/temp.rs:645`.
Every reuse lookup re-parses the 32-char expected MD5 (and builds an
`anyhow::Error` on the common "unknown digest" path).
**Change:** add a non-allocating `try_parse_hex(&str) -> Option<Md5>` and parse
once at decode time, storing a parsed `Md5` on the chunk or in the reuse map.

### MIC-02 — Glob visibility re-matched per file instead of inherited
**Where:** `app/repair-plan/src/path_scope.rs:84-89`;
`src/repair/purge.rs:120,136`.
`is_visible` linearly scans compiled globs (`require_literal_separator: false`,
so `*` scans the whole string) for every planned rel and every walked path,
including directories. **Change:** carry the parent directory's visibility down
the `WalkDir` traversal and skip `is_visible` for files in a visible parent;
add a fast suffix check for the common `--include "*.ext"` shape.

### MIC-03 — Deep directory comparator recomputes `components().count()`
**Where:** `src/repair/purge.rs:189-194`, `app/repair-plan/src/plan.rs:500`.
`dirs.sort_by_key(|a| Reverse(a.components().count()))` counts components on
every comparison (O(n log n × depth) lexical walks). The closures also use
stable sorts on unique paths.
**Change:** map to `(depth, PathBuf)` once, `sort_unstable_by_key`, strip depth;
use `sort_unstable_by` in both places.

### MIC-04 — `urlencode` allocates a temporary `String` per escaped byte
**Where:** `app/fetch/src/hyp.rs:547-557`. Use `write!(o, "%{b:02X}")`
with `std::fmt::Write` into the preallocated buffer, or a maintained
percent-encoding crate.

### MIC-05 — Base URL accessors clone `String` per request
**Where:** `app/fetch/src/hyp.rs:315-321,333-345,482,536`. Return `&str` or
precompute the formatted URLs in the constructor.

### MIC-06 — Uncontended `Arc<Mutex<Option<S>>>` in the retry loop
**Where:** `app/fetch/src/retry.rs:159-170`. The state slot is locked twice per
attempt in a single sequential async loop. **Change:** keep a local
`Option<S>` and pass state by value through `attempt`/`RetryState`.

### MIC-07 — `write_line` issues two `write_all` calls per record
**Where:** `crates/report-core/src/lib.rs:105-108`. Combine line and newline
into one preallocated buffer (or use `write!`) to halve writer calls; stdout is
locked so the two writes are already atomic relative to other records.

### MIC-08 — `Md5::parse_hex` allocates an error for a normal miss
**Where:** `crates/digest/src/lib.rs:101-108`. Covered by MIC-01; the
`anyhow::Error` is immediately discarded by callers using `.ok()`.

### MIC-09 — Range interning allocates and sorts a `Vec<usize>` per scan
**Where:** `app/util-hash/src/md5.rs:140-157,209-237`. `scan_reader_with_buf`
interns ranges into a `HashMap` and sorts a fresh `Vec` of all interned ranges
every call. **Change:** pass pre-sorted, deduplicated ranges from `scan_ranges`
(chunk-order invariant) so the sweep skips the sort, or intern once at plan time.

### MIC-10 — `RangeState::is_ok` clones the whole MD5 context per verdict
**Where:** `app/util-hash/src/md5.rs:85-90` (`self.ctx.clone().finalize()`).
Finalize once when a range is last touched and compare stored bytes.

### MIC-11 — `FileReuse::key` builds a fresh `DefaultHasher` per lookup
**Where:** `app/fetch/src/sophon/map.rs:141-145`. Build a `u64` from the two
8-byte halves of `md5.as_bytes()`, or key `by_key` directly on
`IdentityKey { digest, size }` (already `Hash + Eq`) and drop the custom bucket
hashing.

### MIC-12 — Deprecated list collected twice
**Where:** `app/fetch/src/hyp.rs:486-493`, `src/repair/mod.rs:1172-1181`.
`deprecated_files()` collects a `Vec<String>`, `fetch_deprecated` normalizes in
place, then `list.into_iter().collect()` copies into `Arc<[String]>`. The
comment already acknowledges it; minor because the list is usually small.
Normalize directly into the final buffer if it can be large.

---

## 9. Dev tools — girsync, gircap, sled2redb

### P1 — Network / CPU / memory / disk I/O

**TOOL-01 — Every HTTP body chunk is copied.**
`tools/capture/http/src/transport.rs:236-243`: `HttpResponse::chunk()` maps
reqwest's `chunk()` through `|bytes| bytes.to_vec()`. reqwest 0.13 returns a
refcounted, zero-copy `bytes::Bytes`, so this allocates and copies every body
byte, only for `drain_capped` (`fetcher.rs:188-199`) to write it straight to a
sink. On a capture of ~113 k chunks plus ~9 MB of manifest archives this doubles
per-byte CPU and transient memory.
**Change:** return `bytes::Bytes` from `chunk()` (the `bytes` crate is already in
the lockfile) and write `&chunk` directly; wrap `Memory` chunks with
`Bytes::from(vec)`.

**TOOL-02 — Redundant per-file `stat` in girsync's live walk.**
`tools/sync/src/scan.rs:200-202` calls `std::fs::metadata(ent.path())` even
though `walkdir::DirEntry` already carries metadata; on Windows
`DirEntry::metadata()` reuses the metadata cached during enumeration. A
200 k-file tree pays 200 k avoidable stats. **Change:** use `ent.metadata()?`;
keep `path_is_symlink()` for the link flag.

**TOOL-03 — `verify` buffers whole blobs and decompresses each twice.**
`tools/capture/src/verify/checks.rs:79-93,119-137`;
`tools/capture/src/assemble/source.rs:201-232`. `check_blobs` does
`std::fs::read` (whole blob) then `zstd_measure`; `rebuild_all_with` later reads
and decompresses the same blob again. A streaming API already exists
(`hash::zstd_measure_from`, `hash/src/lib.rs:543-588`).
**Change:** open with `BufReader` and call `zstd_measure_from`; better, populate
a shared `BTreeMap<String, OnceLock<Vec<u8>>>` memo that `ChunkSource::memoizing`
accepts so the rebuild reuses decoded bytes.

**TOOL-04 — Capture decompresses each blob twice (measure then assemble).**
`tools/capture/src/blobs/props.rs:338-375`, `emit/fixture.rs:156-168`,
`assemble/source.rs:147-233`. `measure_file` decompresses to compute md5/len;
`fixture::build` decompresses again through the non-memoizing `ChunkSource`.
Documented as a deliberate memory trade-off, but it doubles zstd CPU across
the bundle. **Change:** memoize decoded bytes for the short window between
measurement and the last assembly, with an explicit bound to keep low RSS.

**TOOL-05 — Repeated linear scans for build ids.**
`tools/capture/src/blobs/mod.rs:924-933` (called at 948-949).
The `build_id` closure scans manifests × fields × entries for every admitted
path, twice, even though the `(field, tag) → chunk map` is already built once
at 912-923. **Change:** store `(map, build_id)` in that index or add a
`HashMap<(&str,&str),&str>`; lookups become O(1).

**TOOL-06 — `build_manifest` matrix is O(rows × admitted).**
`tools/capture/src/emit/index.rs:86-112` (`find` at 91-94) rescans `admitted`
for every row. **Change:** build a `HashMap<&str,&Admission>` by `capture_id`
once and use `.get()`.

**TOOL-07 — Content-Type parsed and allocated twice per request.**
`tools/capture/http/src/fetcher.rs:651-656,969-973`. Both sites build
`content_type: Option<String>` from headers and then call `header_map`, which
copies the same header again. **Change:** compute `resp_headers` first and read
`content_type` from it.

**TOOL-08 — `header_map` allocates a `Vec` per header name.**
`tools/capture/http/src/record.rs:176-198`: `collect::<Vec<_>>().join("\n")`
per header name per response. At ~114 k requests this is hundreds of thousands
of small allocations. **Change:** build the `String` directly with `push_str`
and `'\n'` separators.

**TOOL-09 — Base request headers deep-cloned per logged request.**
`tools/capture/http/src/record.rs:133-145`, `fetcher.rs:1134`. Each
`RequestRecord::req_headers` clones a `BTreeMap` of three owned `String`s
(~340 k identical `String` allocations at 114 k requests).
**Change:** store `Arc<BTreeMap<...>>` (serde serializes through `Arc`); only
the `Range` case needs a fresh map.

**TOOL-10 — `Fetcher` clone deep-copies header maps per chunk task.**
`tools/capture/http/src/fetcher.rs:476-491` clones the whole `Fetcher` once per
item (113 k chunks), copying two `BTreeMap`s and several `String`s despite the
comment calling clones cheap. **Change:** wrap in `Arc<Fetcher>` once and pass
`Arc` to the task closure.

**TOOL-11 — Redundant `Option<String>` clones per measured blob.**
`tools/capture/src/blobs/props.rs:486-505`: `m.md5.clone()` then another clone;
`decode_error` cloned even though `measured` is owned. **Change:** consume
`measured` with `.ok()` and move `m.md5`.

**TOOL-12 — Full manifest decode serialized twice.**
`tools/capture/src/manifest/dump.rs:137,150`; `emit/mod.rs:444-449,481-532`.
`json_len()` walks the entire ~27 MB decode to pick the gzip filename, then
`write_json_gz` serializes the same document again.
**Change:** serialize once into a temporary plain file with a counting writer,
then gzip that file to its final name and delete the temp — one extra disk pass
instead of one CPU-heavy serialization.

**TOOL-13 — Every file canonicalized four times per close-out.**
`tools/capture/src/emit/index.rs:275-337,353-356`: `walk()`'s `filter_entry`
canonicalizes every entry, and close-out walks at least four times
(`write_manifest_files`, two `scan_reusing` passes, `a16`/`a16_scoped`
membership). **Change:** share one canonical walk result per close-out, or
only canonicalize entries containing a symlink component (cheap
`file_type()` check first). Directly related to the artifact-bundle rewrite
work the brief permits skipping.

### P2 — Parallelization

**TOOL-14 — `verify`'s per-blob A6 work is serial.**
`tools/capture/src/verify/checks.rs:30-96`: MD5 + SHA-256 + zstd measure per
blob, all independent and CPU-bound. **Change:** `rayon::par_iter()` over
`index.blobs`, collecting rows and sorting by id for determinism; pair with
TOOL-03 so each worker streams its own file handle.

**TOOL-15 — Capture assembly is serial.**
`tools/capture/src/emit/fixture.rs:156-186`: each admitted capture is assembled
(zstd decode + MD5 + patch flow) in a plain `for` loop, writing disjoint
`patchmaps/<id>.json`, `game/…`, `expected/…` paths.
**Change:** parallelize with rayon, appending `(rows, patchmap, bytes, digests)`
in index order to keep artifact bytes deterministic. Verify `path_map`'s
interior mutability is thread-safe before sharing it.

### P3 — Buffers & strings

**TOOL-16 — Fresh 512 KiB buffer per girsync hashed file.**
`tools/sync/cache/src/hash/stream.rs:74-91` (`vec![0u8; CHUNK]` at 83), called
per file from `tools/sync/src/effective/phase_c.rs:139-140`. The copy path
already takes a caller-owned buffer (`commands/sync/apply/copy.rs:33-54`).
**Change:** add `hash_file_with_buf(path, algos, &mut [u8])` and give each rayon
worker a reusable buffer via `for_each_init`.

**TOOL-17 — `rel()` allocates `Vec<Cow>` + `String` per entry per walk pass.**
`tools/capture/src/emit/index.rs:363-370`, used by every entry of every walk
(TOOL-13). **Change:** iterate `components()` and `push_str` into a pre-sized
`String` with `'/'` separators.

### P4 — redb / migration

**TOOL-18 — `sled2redb` stages the whole migration in one giant transaction.**
`tools/sled2redb/src/convert.rs:239-252`: one `WriteTransaction`, `put` for
every record, one commit. The rest of the cache layer batches via `CacheWrite`
(`COMMIT_BATCH`/`COMMIT_BYTES`/`COMMIT_INTERVAL`). The destination is a
temporary file published only at the end, so intermediate commits are safe.
**Change:** use `cache.begin_write()` and its standard batching, keeping a
final commit — bounds peak transaction memory without changing atomic publish.

### P5 — Correctness / fail-fast

**TOOL-19 — Canonical dedup can drop the real file for a symlink alias.**
`tools/capture/src/emit/index.rs:284-302` (`visited.insert(canonical)`) and
`:353-356`: `filter_entry` inserts the canonical path of *every* entry and
filters repeats. If a symlink to a file sorts before the real file, the link is
inserted first and the real path is filtered out. `checksums.json` then names
the link instead of the real file, so A14's judgement depends on directory-sort
order. **Change:** only dedup directories in `filter_entry` (`is_dir()`); files
cannot create a traversal cycle.

**TOOL-20 — Debug-only round-trip assertion runs per document write.**
`tools/capture/src/emit/mod.rs:414-419,526-531`:
`debug_assert_eq!(rel_path_string(root, &path), rel, …)` allocates on every
write, including under `cargo nextest` debug builds. **Change:** move the
invariant check into a test over representative paths, or gate it behind an
explicit test-only switch.

**TOOL-21 — Declared-size mismatch reported as a decode-limit error.**
`tools/capture/hash/src/lib.rs:354-356,421-435`: when the index declares a
decompressed length smaller than the actual frame, `zstd_decompress_from`
limits decoding to the declaration and reports "decoded output exceeds the N B
decode limit", conflating a lying index with a decompression bomb.
`zstd_measure_from` uses the absolute ceiling.
**Change:** distinguish "expected < decoded" from "decoded > ceiling" in the
error text, naming both values; or measure first when the declaration is
suspicious.

---

## 10. Build & toolchain

**Already tuned — no change proposed.** `Cargo.toml` sets
`profile.dev.codegen-units = 4`, `profile.test.{debug=0, codegen-units=1,
incremental=false}`, `profile.release.{lto="thin", codegen-units=1,
strip=true}`, and `.cargo/config.toml` selects `rust-lld` on MSVC. `.config/nextest.toml`
uses a `cmd.exe` setup script specifically to avoid PowerShell startup (~1.7 s
per run) and sets test-only fsync/durability hooks. These are deliberate,
measured choices; re-visiting them needs host measurements, not static analysis.

**B-01 — `chrono` for a single log filename stamp.** `crates/time/Cargo.toml`
enables `chrono` with `clock` (pulling `iana-time-zone`, windows shims) only to
produce a UTC timestamp for the default log filename. A lighter time source
could reduce build/dependency surface, but hand-rolling calendar conversion is
error-prone and hand-rolling is discouraged here. **Assumption:** no other
crate needs `chrono`; verify with `cargo tree -i chrono` before changing.

**B-02 — Duplicate crate versions.** `Cargo.lock` has two versions each of
`bitflags`, `core-foundation`, `getrandom`, `rand`, `rand_core`, `syn` and
`windows-sys`. All are ordinary transitive duplicates (notably `syn` 1/2 from
proc-macro dependencies); none is an obvious deduplication win.

---

## 11. Assumptions to validate before implementing

These are the parts of the findings that depend on runtime behavior, workload
shape or external contracts and were not fully provable from source alone.

1. **S-01/S-02 pairing.** Repair file tasks must be moved off async workers
   before shrinking the runtime; otherwise `jobs` file tasks could starve
   timers/network. Measure with `--jobs 1/4/64` on both SSD and HDD.
2. **S-03 write independence.** Chunk ranges are disjoint for the tiers where
   concurrent fetch is proposed; overlapping/salvage layouts must stay
   ordered. Confirm the ceiling ledger accepts out-of-order chunk settlement.
3. **IO-01 verdict reuse.** The scan's whole-file digest is valid for the temp
   skip check under the same mtime/length guard used by the in-place path; a
   file scanned before a write in the same run is not incorrectly skipped.
4. **IO-02/IO-03 double-read removal.** Holding reused bytes in memory (or
   spooling them) fits the `--max-ram-bytes` contract; dropping the second hash
   does not weaken a required TOCTOU guard. If a re-check is required, keep it
   but fold it into the single read.
5. **IO-05 `join_validated`.** No caller of `PathRoot::join` passes an
   untrusted rel that happens to look plan-derived; the new fast path must be
   used only from `build_plan` consumers.
6. **NET-01 timeouts.** A 5–10 s connect timeout and a generous overall
   timeout do not break large chunk streams on slow links — a *per-stream idle*
   timeout is safer than a blanket total timeout.
7. **NET-02 status check.** No production HYP endpoint returns a non-2xx body
   with a usable `retcode`; check captured bundles before enforcing
   `error_for_status`.
8. **NET-05/NET-06 caps.** The chosen manifest decode cap is above the real
   largest manifest (16.8 MB for build 7.1.0) and the blob-id shape check does
   not reject any id currently returned by production.
9. **COR-02 stale verdict.** Whether `--check-only --scan` is intended to be a
   fresh check; if it is, re-validate mtime and fall back to a read.
10. **COR-01 exit class.** Changing lock/dir failures from exit 1 to exit 3 (or
    a new code) is acceptable to downstream automation — the task allows
    observable behavior changes, but consumers may pin exit 1.
11. **LOG-01 default level.** Operators can opt back into per-chunk logs via
    `--log-file-level debug`; no bundled artifact depends on those lines.
12. **LOG-03 lossy writer.** Switching to `lossy(false)` could backpressure a
    very verbose run; consider a larger bounded channel plus a periodic
    dropped-line report instead.
13. **B-01 chrono.** Confirm with `cargo tree -i chrono` that only the timestamp
    leaf uses it, and that a replacement preserves every pinned spelling in
    `crates/time`.
14. **TOOL-01/07/08/09/10.** Changing the HTTP `chunk()` return type and header
    plumbing touches several `Fetcher` call sites; confirm the recorded
    request/response JSON is byte-identical (the artifact bundle pins it).
15. **TOOL-03/04/12.** Memoizing decoded blobs and serializing the manifest once
    via a temp file changes peak RSS vs. disk-I/O trade-offs; measure on a real
    capture bundle before choosing.
16. **TOOL-19.** Symlink entries that are also listed as real files may be
    *intentionally* deduplicated; confirm the bundle contract before changing
    `visited` to directories-only, and add a regression test with a link that
    sorts first.
17. **TOOL-13/15.** Sharing a canonical walk across close-out and parallelizing
    capture assembly must preserve the documented write order and deterministic
    artifact bytes; check the close-out's fixpoint logic (two/three passes) is
    unaffected.

## 12. Checked and deliberately not reported

- **Integer overflow in range arithmetic.** `r.offset + r.len` in
  `scan_reader_with_buf` and `offset + len` in `layout()` cannot overflow `u64`:
  both operands derive from `i64`-bounded manifest fields, so the sum is at
  most `2*i64::MAX = u64::MAX - 1`.
- **`RepairPlan::fingerprint()`.** The expensive file-list hash is cached at
  construction (`app/repair-plan/src/plan.rs:211-227,281-286`); not a hot cost.
- **Verify-path multiple reads / temp-mode resume.** The extra full-file reads
  on digest mismatch and the length-based resume with rollback are documented
  and internally consistent; no off-by-one was found in their ranges.
- **`write_config_ini` / `read_game_version`.** Already avoid the
  `exists()`-then-read TOCTOU and pin the last-match-wins rule; no safe
  simplification found (the per-line `String` allocation is on a tiny file).
- **Cargo profile settings.** Already tuned with measured host data; changing
  them without the same measurements would be guesswork.
