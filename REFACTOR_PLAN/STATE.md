# Refactor state

Single source of truth for orchestration. The orchestrating session may be replaced at
any time; all durable knowledge lives here. See `ORCHESTRATION.md` for the procedure and
`CONVENTIONS.md` for the format.

Report = `docs/OPTIMIZATION_REPORT.md`. States: `todo` → `in_progress` →
`pending_review` → `completed` / `reverted` / `skipped`. Every change to this file is its
own commit. Commit refs are pointers for reviewers only — no dates, session/tool IDs,
machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Right-size the Tokio runtime and move repair file I/O off async workers | completed | 3fd12e3c, 7c488216 | §2 S-01/S-02 (109–141); assumption 1 (837–840); pair with T10. Deviation: whole file task runs on the blocking pool via Handle::block_on instead of per-call-site spawn_blocking; diagnostics keyed by owner thread (src/repair/diagnostics.rs). T10 offload must match (see persistent context). Reviewed A: no fix/revert needed. |
| T02 | Fetch and decode chunks within one file in a bounded window | completed | e385b8dd | §2 S-03 (143–157); assumption 2 (841–842). Scoped to in-place Staged tier with disjoint layout; salvage/Ordered, overlapping layouts and temp stay sequential (temp resume-by-length/rollback couples writes). Ceiling ledger CAS settles out of order exactly; per-chunk wire counts change on failure (siblings still fetched); RAM contract unchanged. |
| T03 | Run the three independent plan-phase network fetches concurrently | completed | 3ca00e39 | §2 S-04 (158–172); early-deprecated assumption (170–171) holds. Deviation: deprecated fetch now runs before the step 4.2 scope guard, so a scope typo may issue one extra best-effort advisory request before the same usage error. New public `ManifestBudget`; `fetch_latest`/`local_reuse_map` take a budget; `PlanInputs` gained `deprecated` — relevant to T11/T12. |
| T04 | Parallelize purge walk/stat/delete while keeping deterministic log order | completed | ff48e189 | §2 S-05 (173–185); assumptions (183–184) hold. Walk stays serial; stat/delete parallel via bounded scoped threads (no new dependency); results in index slots, log lines emitted on caller thread in plan order; empty-dir sweep after all deletes. DirEntry carried into stat stage to keep Windows cached metadata (no extra syscall); tracing dispatcher is thread-local so scoped workers cannot log. |
| T05 | Eliminate the duplicate plan/scan per-file stat pass | completed | 0b4031c5 | §2 S-06 (186–198). Reuse of scan data into DiskFigure::Measured rejected: scan uses follow-symlink `metadata` + bytes-read length, plan needs `symlink_metadata` is_file/length (classification + verdict mtime guard). Chose genuine overlap: plan stat task launched before scan awaited, JoinHandle held in PlanWork and awaited after limits gate; aborted on limit refusal. Relevant to T06. |
| T06 | Reuse scan verdicts and remove double reads in the repair path | pending_review | bcd10762, d45c3c49, 10394ad7 | §3 IO-01..IO-04 (202–251); §6 COR-04 (453–462); assumptions 3–4 (843–849). IO-01 guarded verdict shared with temp (byte-identical conditions); IO-02 single capture read into one reusable per-file chunk buffer; IO-04 layout() once, passed as &Layout; COR-04 reuse I/O fallback counter + info/warn (MD5 mismatch stays debug). IO-03 premise false: staged-source hash is a required TOCTOU re-check, kept and documented. Temp staging buffer holds one chunk, not charged to --max-ram-bytes (temp mode never used it); peak temp RSS rises by one chunk. Guard helpers moved to file/mod.rs; attempt_temp gained FileId; added RepairDiagnostics.reuse_io_fallbacks. F07 guard-equivalence fails — no code change. |
| T07 | Add validated path joins and cache canonicalization | completed | 36883118 | §3 IO-05/IO-06 (252–276); assumption 5 (850–852) holds — only five plan-derived join callers. Deviation: parent cache keeps leaf canonicalization (dropping it would allow a file-level symlink escape); game_dir canonicalized once per phase (twice per run), second PathRoot via syscall-free new_canonical. New API: PathRoot::join_validated/new_canonical, Lock::acquire_canonical, lock_path_for_canonical. |
| T08 | Remove redundant post-phase mkdir syscalls and materialization | completed | c24b6f71 | §3 IO-07/IO-08 (277–293). Audio-cache walk streamed (no Vec); mkdir dedup via HashSet of successful creates only (failures retried, phase stays best-effort); ASCII byte fast path for trailing-dot/space check with lossy fallback for non-ASCII. New private `move_audio_cache_with_mkdir` test seam; `rel_from_base` no longer imported in post.rs. |
| T09 | Bound streaming decode output and validate blob IDs | completed | 9b914411 | §4 MEM-01 (297–308); §5 NET-05/NET-06 (386–403); assumption 8 (859–861) validated on captured 7.1.0 bundle (16.8 MB manifest, 9 MB compressed) — both 64 MiB caps exceed it. Deviation: conservative unsafe-shape id check (not strict capture regex) so short hex/test ids still pass; origin/path-prefix assertion is the backstop. MAX_MANIFEST_HINT removed; new MANIFEST_INITIAL_CAPACITY/MAX_MANIFEST_BYTES/MAX_MANIFEST_COMPRESSED; join_url now rejects hostile ids. |
| T10 | Move streaming decode/write off async workers | completed | ec5b52ab | §5 NET-07 (404–413); same class as S-02/T01. Report premise false in current tree: sole production caller write_downloaded_chunk runs inside T01's Handle::block_on blocking-pool file tasks, so a spawn_blocking wrapper would be redundant and is forbidden by T01 policy. Took the sanctioned document-and-enforce outcome (rustdoc contract on stream_chunk_to/write_downloaded_chunk); no executable code change. No runtime assertion (Tokio 1.54 exposes none); enforcement by documented contract + Send bound. |
| T11 | Add HTTP timeouts, status checks and fetch-layer micro-optimizations | completed | 76daf1ce | §5 NET-01..04 (349–385); §4 MEM-05 (337–345); §8 MIC-04/05/12 (591–599, 630–636); assumptions 6–7 (853–858) validated on capture (9 MB GETs take 11 s; all JSON responses 200). 10 s connect + 30 s idle on JSON reads (no blanket total); status check in both JSON paths, non-2xx classified Fatal (pinned 500 test); body-read Retry, wrapper-parse Fatal, data:null -> SophonNotFound. MIC-12 not needed (already in-place retain_mut); stream.rs untouched (T09-owned); tokio `time` feature added as normal dep. |
| T12 | Validate reuse offsets and parse digests once | completed | 65ef8db6, 9b5eabdf | §6 COR-03 (442–451); §8 MIC-01/08/11 (564–573, 610–613, 624–629). COR-03: offset/size >= 0 and checked_add against manifest-declared `f.size`, warn+skip; `.max(0)` clamp removed. MIC-01/08: new `Md5::try_parse_hex -> Option` (parse_hex keeps identical errors); "parse once at decode" half already satisfied. MIC-11: XOR of two digest halves replaces DefaultHasher; bucket/collision-confirmation unchanged. stream.rs untouched (T09/T11). |
| T13 | Reuse hash scratch buffers and tighten range handling | completed | 3e301c80, 4b3998ae | §4 MEM-03/04 (320–336); §8 MIC-09/10 (614–623). Thread-local reusable 512 KiB hash scratch; read_to_end into reserved spare capacity (no unsafe — workspace denies unsafe_code; read_buf unstable on pinned 1.100). MIC-10 finalizes range Md5 once; MIC-09 already-sorted fast path (not cross-crate pre-sort — public scan API accepts arbitrary order). Report's mod.rs zero-fill pointer now resolves to temp.rs::capture_slice; scratch_slice in file/mod.rs left unchanged (already reused). 4 equivalence tests added. |
| T14 | Classify lock and directory failures correctly | completed | e604bb45, dc3f9cfd | §6 COR-01 (417–428); assumption 10 (864–866) validated in-repo (two lock test assertions + 3 docs updated; no CLI-contract pin; external automation unverifiable). Deviation: lock contention uses existing `RunFailure::write` (exit 3), not a new code, preserving the pinned exit-code table; held() chain names the holder. create_dir_all failure -> write (exit 3) with `ctx.game_dir` path. app/lock untouched. |
| T15 | Fix stale scan/check verdicts, surface scan failures, close tmp TOCTOU | completed | 56a1c625, c4f21a4c | §6 COR-02/COR-05/COR-08 (429–441, 464–472, 496–504); assumption 9 (862–863). Partially implemented: COR-02 documented, not changed — --check-only --scan is a deliberate plan-answer check (verified by commit d046eb54 and a_scanned_check_uses_the_scan_digest_but_still_checks_the_length); COR-05 adds ScanReport::failures + scan_failed(), surfaced on the scan-phase tracing line (not PLAN stdout) with estimates unchanged. COR-08 SKIPPED: no portable safe handle-relative rename (std has no File::rename; Linux no rename-by-fd; Windows needs FFI but workspace denies unsafe_code) — promote_tmp left path-based. Reviewed H: completed (partial with one validated skip — do not re-validate mtime/re-hash or use unsafe FFI). |
| T16 | Move scope-typo refusal earlier and surface purge diagnostics | completed | 9923b4c9 | §6 COR-06/COR-07/COR-09 (474–512). Early check_scope_selection after build_plan for !purge_wanted (before walk); full post-walk check retained for purge runs. SUMMARY gains conditional numeric purge_errors=N / purge_hidden=N (only when non-zero) via existing Extra::Num path — T16's sole data-plane exception. Deviation: no hidden-byte counter exists, so purge_hidden is a path count, documented as such. plan_work_and_purge gained private purge_wanted: bool. Strict refusal not added. |
| T17 | Reduce logging cost without changing the data plane | completed | 39fa35a0, 8fc260ba, 76bc4ecf | §7 LOG-01..03 (517–559); assumptions 11–12 (867–871). Chose release default file level Info over moving per-chunk lines to trace (assumption 11 pins the --log-file-level debug opt-in spelling); new public girpr_log::default_file_level() — shared crate, so gircap/girsync release builds also default Info. Expensive body/error fields gated behind enabled!(TRACE)/(DEBUG). Appender uses lossy(false) with stock 128k channel (backpressure deliberate, no dropped_lines report). Data plane untouched. |
| T18 | Low-risk micro-optimization sweep | completed | 97b9f6e5, 19e89da4, 5b01ea28 | §8 MIC-02/03/06/07 (574–589, 600–609); §4 MEM-02 (309–319). Implemented MIC-06 (local Option state), MIC-07 (single write_all), MIC-03 purge.rs (stable sort_by_cached_key — unstable would reorder same-depth dirs), MEM-02 (borrowed &str keep-set probes off Windows, fold unchanged). SKIPPED: MIC-02 whole (glob require_literal_separator:false means * crosses /, so visibility is not inherited — fast path would change purge decisions); MIC-03 plan.rs site stale (plain rel sort); MEM-02 PurgeKeys three copies/membership hash (optional + needs lifetime redesign). |
| T19 | Cut allocations in gircap's HTTP recording pipeline | completed | 54d186e4 | §9 TOOL-01/07/08/09/10 (643–652, 691–715); assumption 14 (875–877) validated with a byte-level golden fixture (passed pre- and post-change). API: HttpResponse::chunk() -> Result<Option<bytes::Bytes>, String>; RequestRecord::req_headers is Arc<BTreeMap<...>> (serde rc); Fetcher::map_concurrently closure takes Arc<Fetcher> first. TOOL-07 edge: content_type now from joined resp_headers (differs for repeated/non-UTF-8; pinned bundle uses single ASCII). |
| T20 | Memoize decoded blobs and serialize the manifest once | completed | 6d4e3888, 12e73ec4 | §9 TOOL-03/04/11/12 (661–678, 716–728); assumption 15 (878–880). Implemented TOOL-11 (move measured md5/error into rows) and TOOL-12 (new public write_json_gz_counted: serialize once to a counted temp with RAII cleanup, then copy/gzip; byte-identical test). SKIPPED TOOL-03/04 memoization: ChunkSource::memoizing already exists in verify but is unbounded and deliberately not used in capture (+39.5 MB RSS trade-off); no real bundle to measure (only 44 KB synthetic), so assumption 15 cannot be validated. "Each blob decoded once" does NOT hold across A6->rebuild or measure->assemble. Reviewed J: completed (partial with validated skip — do not enable capture memoization without a total-byte bound and a real-bundle measurement). |
| T21 | Fix gircap indexing and canonicalization correctness | completed | af8a2890 | §9 TOOL-05/06/13/17/19/20 (679–690, 729–737, 763–768, 780–795); assumptions 16–17 (881–888) validated. O(1) build-id/admission indexes; TOOL-13 uses path_is_symlink() + ancestor stack (file_type() returns target under follow_links(true)); shared canonical walk rejected — tree changes between fixpoint passes. Dedup directory-only with first-sorting-link regression test. TOOL-20 removed three per-write debug asserts (incl. T20's in write_json_gz_counted) into a test. No production symlink creation; first-match semantics kept. |
| T22 | Parallelize gircap verification and capture assembly | completed | fbc134ba, b4bade9a | §9 TOOL-14/15 (740–752); assumption 17 (885–888) validated. rayon added to gircap. Verify A6 per-blob over rayon, collect (id,row) sorted by id; assembly CPU parallel but writes/rows/digests/path_map/tracing stay serial in index order (path_map proven Send+Sync but not actually shared — preserves write order, abort semantics and .json.tmp/close-out safety). New trade-off: all assembled results briefly resident. A6 row order relies on ChunkIndex::blobs being id-ordered (hand-edited unsorted index could change first-red-A6 headline). |
| T23 | Make gircap declared-size mismatch errors honest | completed | 68e83ac5 | §9 TOOL-21 (796–805). Text-only fix: private zstd_over_limit_from; mismatch says "declared size mismatch ... declared expected size of N B", bomb says "decode ceiling exceeded ... C B hard ceiling"; both keep the "decode limit" token. Ceiling/limit enforcement code unchanged; zstd_measure_from untouched so verify A6 report strings/artifact bytes unaffected. Chose text distinction over measure-first (would change decode behavior/RSS). |
| T24 | Cut girsync and sled2redb I/O and transaction cost | completed | b8d34961, 01f717e1 | §9 TOOL-02/16/18 (654–659, 756–762, 770–777). TOOL-02: ent.metadata() in live walk (walkdir 2.5.1 caches for regular files under follow_links(true)), path_is_symlink kept. TOOL-16: new public girsync_cache::hash::hash_file_with_buf; phase C uses rayon try_for_each_init (Result-returning closure) for per-worker buffers. TOOL-18 SKIPPED (stale): write_records already batches via cache.begin_write() + COMMIT_* + final commit, publishes via NamedTempFile; sled2redb untouched. |
| T25 | Validate and, if safe, shrink the build-time dependency surface | skipped | | §10 B-01 (818–823); assumption 13 (872–874) FAILS workspace-wide: cargo tree -i chrono --workspace shows gircap is a second direct chrono consumer with chrono-typed public API feeding girpr_time; removing chrono cannot preserve the four pinned spellings/tests or the API without an out-of-scope rewrite. No code change, no commit. |

## Reviews

A review-and-fix session reviews one `pending_review` batch at a checkpoint sized by
amount of work (see `REVIEW-CHECKPOINTS.md`).

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01 | done | No fix/revert needed; offload committed before runtime shrink and tests pass. Endorsed follow-up: stress test above blocking-pool cap; dropped: --jobs help clarification. |
| B | T02, T03 | done | No fix/revert needed. Endorsed stress case with per-chunk timing and conditional deprecated prefetch deferral; dropped temp-mode windowing. |
| C | T04, T05 | done | No fix/revert needed. Endorsed purge-scale measurement; endorsed scan-probe sharing only if folded into T06. |
| D | T06 | done | No fix/revert needed. F07 closed (guard-equivalence fails); endorsed temp-RSS measurement follow-up. |
| E | T07, T08 | done | No fix/revert needed. Endorsed CleanRel newtype, install-scale syscall measurement, and audio-cache scale measurement. |
| F | T09, T10, T11 | done | No fix/revert needed. All four suggested follow-ups endorsed; the high-jobs poll check folded into F01/F02. |
| G | T12, T13 | done | No fix/revert needed. Dropped Sha256 symmetry; endorsed five other follow-ups (mostly low/conditional priority). |
| H | T14, T15, T16 | done | No fix/revert needed. T15 resolved completed with COR-08 a validated skip; dedicated exit code dropped; five other follow-ups endorsed. |
| I | T17, T18 | done | No fix/revert needed. Endorsed order-insensitive chunk-hit test, conditional PurgeKeys revisit, and lossless-appender backpressure validation; duplicate suggestion dropped. |
| J | T19, T20 | done | No fix/revert needed. T20 resolved completed with memoization a validated skip; all three suggested follow-ups endorsed plus a content_type edge test. |
| K | T21, T22, T23 | done | No fix/revert needed. All six suggested follow-ups endorsed. |
| L | T24 | done | No fix/revert needed. T25 was validated-skipped and not reviewed. Endorsed scale measurement and a walk_live mtime-equivalence regression test. |

## Follow-ups

Non-blocking suggestions from review sessions; the orchestrator records them here.
Reviewers decide which are worth doing and may prune them to avoid run-away. A
planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Stress-test a plan with `--jobs` above Tokio's default blocking-pool cap (512) to confirm queuing, no deadlock/starvation, that the file-window bound holds, and that chunk streaming is polled off runtime workers | dropped | Endorsed by reviews A and F; ties to assumption 1 (report lines 837–840), T01 and the T10 documented-contract check. Dropped by follow-up planning: validation-only; runtime already decoupled from --jobs. |
| F02 | Validate a full repair at `--jobs 1/4/64` (and on HDD where possible), not just the host-independent worker arithmetic | dropped | Non-blocking validation follow-up from review A; assumption 1 (report lines 837–840). Dropped by follow-up planning: needs a real install plus SSD/HDD control. |
| F03 | Add a many-small-chunks end-to-end stress case with per-chunk timing to confirm real network/decompression overlap from the T02 window, not just structural correctness | dropped | Endorsed by review B; temp-mode windowing extension (its resume-by-length/rollback coupling) dropped. Dropped by follow-up planning: needs a large file and real network/decompression timing. |
| F04 | Conditionally defer the deprecated prefetch when `path_scope.has_includes()` is active, restoring the "no advisory request on scope typo" property while keeping overlap in the common unscoped case | dropped | Endorsed by review B; low priority — the extra advisory request is harmless, usage error/message unchanged. Dropped by follow-up planning: one harmless advisory request; pure churn. |
| F05 | Validate peak RSS under high `--jobs` with the 4x per-file decoder window to confirm tight `--max-ram-bytes` ceilings are not pressured by transient decoder scratch | dropped | Non-blocking validation follow-up from review B. Dropped by follow-up planning: peak-RSS measurement; no shipped change. |
| F06 | Measure purge wall time at scale (10k+ orphans, SSD/HDD, both platforms) before claiming a cross-platform speedup | dropped | Endorsed by review C; on Windows the stat stage issues no syscall (cached DirEntry metadata), so only delete benefits there. Dropped by follow-up planning: scale timing validates a report claim deleted at cleanup. |
| F07 | Let `scan_one` capture a `symlink_metadata`-derived `is_file`/`length` so one syscall serves plan and scan | dropped | T06 investigation confirmed guard-equivalence fails (scan needs follow-symlink modified + bytes-read file_len; plan needs symlink_metadata is_file/length); review D closed it. No code change. |
| F08 | Measure peak temp-mode RSS with large reuse slices under tight `--max-ram-bytes`/`--max-write-bytes` to decide whether a bounded spool fallback is worth adding while keeping the single read | dropped | Endorsed by review D; one-chunk transient RAM is bounded but uncharged to the budget and unmeasured. Dropped by follow-up planning: needs scale; no action here. |
| F09 | Add a `CleanRel` newtype returned by `clean_rel` to enforce the `join_validated` precondition structurally instead of by debug assertion | dropped | Endorsed by review E, low priority; all five current call sites are correct and test-covered. Dropped by follow-up planning: cross-cutting path churn; call sites already correct and covered. |
| F10 | Measure the path syscall reduction from T07 on a large install before claiming a speedup | dropped | Endorsed by review E; report figures are static reasoning. Dropped by follow-up planning: large-install measurement; validates a deleted report claim. |
| F11 | Measure the audio-cache move at scale (many files per destination directory) to quantify the mkdir/syscall reduction | dropped | Endorsed by review E; report figures are static reasoning. Dropped by follow-up planning: audio-cache scale measurement; validates a deleted report claim. |
| F12 | Revisit the fixed 64 MiB manifest ceilings if a future manifest grows past them, and share one id-shape definition between fetch and capture tools | dropped | Endorsed by review F; keep the conservative fetch check — the strict capture regex would reject real short ids. Dropped by follow-up planning: conditional; sharing id-shape risks the assumption-8 violation. |
| F13 | Add a fetch-side test for the JSON body idle/deadline timeout using a stalled loopback origin with an injectable short duration; clarify that JSON_BODY_IDLE is a bounded deadline on small buffered bodies, not a resetting timer | done | Endorsed by review F; the timeout path is currently untested. Batch 1 (test hardening). Batch 1 reviewed; fix commit 62e6be61 (comment-only). |
| F14 | Add a resetting per-piece idle timeout inside `sophon/stream.rs::stream_body` for mid-stream blob stalls (never a total deadline) | dropped | Endorsed by review F, lower priority; touches the file shared with T09/T10 — schedule after those in-flight tasks. Dropped by follow-up planning: new timeout needs slow-link validation unavailable here. |
| F15 | Replace `by_key` buckets with `HashMap<IdentityKey, usize>` to drop the Vec/collision-confirmation machinery | dropped | Endorsed by review G, low priority; MIC-11 alternative, cleanup only (current code is correct). Dropped by follow-up planning: cleanup of already-correct code. |
| F16 | Add a log-capture assertion that the T12 invalid-range warning is actually emitted | dropped | Endorsed by review G, low priority; app/fetch has no log-capture dev-dependency yet. Dropped by follow-up planning: new dev-dependency for one low-priority assertion. |
| F17 | Convert `file/mod.rs` `scratch_slice`/piece-copy loops to spare-capacity reads | dropped | Endorsed by review G conditionally (low priority) — only if the one-time-per-file 512 KiB fill proves measurable. Dropped by follow-up planning: conditional on an unmeasured one-time 512 KiB fill. |
| F18 | Consider task-owned rather than thread-local hash buffers if 512 KiB per blocking-pool thread becomes a concern | dropped | Endorsed by review G conditionally (low priority); revisit with RSS evidence. Dropped by follow-up planning: conditional on RSS evidence. |
| F19 | Add a unit test that a lying declared `f.size` (larger than the real file) cannot turn a bad reuse offset into an intact verdict | done | Non-blocking follow-up from review G; existing tests cover negative/overflow/past-declared-EOF, not declared-vs-real mismatch. Batch 1 (test hardening). Batch 1 reviewed; fix commit 62e6be61 (comment-only). |
| F20 | Add a binary-level e2e case asserting process status 3 plus FATAL text for a held lock and failed game-dir creation | dropped | Endorsed by review H, low priority; API-level tests already assert RunExit::Write. Dropped by follow-up planning: duplicates existing API-level exit-code assertions. |
| F21 | Revisit COR-08 if a safe portable handle-relative rename primitive appears, or evaluate post-rename identity verification | dropped | Endorsed by review H; tracks the validated skip in T15. Dropped by follow-up planning: no portable safe handle-relative rename exists; nothing actionable. |
| F22 | Surface the post-repair re-plan errors/hidden counters by threading `&mut Summary` through the post phase | dropped | Endorsed by review H, low priority; only the up-front purge plan's counters are currently surfaced. Dropped by follow-up planning: would re-open the pinned SUMMARY data plane. |
| F23 | Consider strict refusal on purge errors as a separate, explicitly opted-in change | dropped | Endorsed by review H; must stay opt-in and separate, never a silent change. Dropped by follow-up planning: low value and refusal-semantics risk. |
| F24 | Add an end-to-end test injecting an unreadable purge candidate to assert the new SUMMARY keys | dropped | Endorsed by review H; current coverage of the new keys is unit-level. Dropped by follow-up planning: platform-fragile; new SUMMARY keys already unit-covered. |
| F25 | Make the order-sensitive chunk-hit assertions (e.g. tests/repair_writer.rs) compare as a set to remove the pre-existing flake under T02's concurrent fetch window | done | Endorsed by review I, low priority; a BTreeSet comparison is already used in the windowed test. Batch 1 (test hardening). Batch 1 reviewed; fix commit 62e6be61 (comment-only). |
| F26 | Revisit MEM-02's three keyed PurgeKeys copies via a borrowing-friendly type or a compact membership hash | dropped | Endorsed by review I conditionally — only if profiling shows the resident strings matter. Dropped by follow-up planning: conditional on profiling evidence. |
| F27 | Validate lossless-appender backpressure under a trace-level large repair (--log-file-level trace with the 128k channel) | dropped | Non-blocking validation follow-up from review I; assumption 12. Dropped by follow-up planning: needs a trace-level large repair; unactionable here. |
| F28 | Extend the recorded-JSON golden fixture with a retained 503-retry error-body capture so the attempt() error-branch content_type and error_responses.json body bytes are byte-golden-covered | dropped | Endorsed by review J, low priority; coverage strengthening (value already unit-asserted). Dropped by follow-up planning: golden-fixture churn; value already unit-asserted. |
| F29 | Revisit T20 memoization with an explicit total-memo byte cap and a wall-clock measurement on a real capture bundle | dropped | Endorsed by review J; the missing preconditions for the validated skip — do not enable capture memoization without both. Dropped by follow-up planning: needs a total-byte cap and a real bundle, neither available. |
| F30 | Measure the counted-temp serialize-once path on a ~27 MB manifest on HDD to confirm the extra disk pass beats the second serialization | dropped | Endorsed by review J, low priority (validation only). Dropped by follow-up planning: validation-only HDD timing. |
| F31 | Pin the deliberate content_type edge with a test asserting joined/encoded value for repeated or non-UTF-8 Content-Type headers | done | Non-blocking follow-up from review J; documents an intentional difference so it is not "fixed" back. Batch 2 (gircap artifact e2e). Batch 2 reviewed; no fix needed. |
| F32 | End-to-end capture test planting a file link sorting before its real target and asserting published `checksums.json` lists both paths | done | Endorsed by review K; current T21 regression test is at walk() level. Batch 2 (gircap artifact e2e). Batch 2 reviewed; no fix needed. |
| F33 | Measure gircap walk syscalls on a large bundle to validate the TOOL-13/17 reduction | dropped | Endorsed by review K; current evidence is structural, not measured. Dropped by follow-up planning: needs a large bundle; validates a deleted report claim. |
| F34 | Consider bounded assembly batching if peak RSS shows the all-results-resident trade-off is a concern, keeping ordered writes | dropped | Endorsed by review K conditionally; measure first. Dropped by follow-up planning: conditional on RSS evidence. |
| F35 | Consider streaming per-worker A6 reads to cut transient memory while preserving exact row text (MD5+SHA-256+zstd measure) | dropped | Endorsed by review K conditionally; non-trivial. Dropped by follow-up planning: conditional and non-trivial; needs memory evidence. |
| F36 | Wall-clock timing on a real capture bundle to validate the gircap parallel speedup claim | dropped | Endorsed by review K; report figures are static reasoning. Dropped by follow-up planning: needs a real bundle; validates a deleted report claim. |
| F37 | End-to-end materialize assertion that an under-declared blob surfaces the new declared-size-mismatch text | done | Endorsed by review K; validates T23's user-visible contract. Batch 2 (gircap artifact e2e). Batch 2 reviewed; no fix needed. |
| F38 | Measure girsync's walk-stat and per-worker-allocation reductions at scale (large tree, both platforms) | dropped | Endorsed by review L; current gains are structural, not measured. Dropped by follow-up planning: scale/two-platform measurement; validates a deleted report claim. |
| F39 | Add a direct regression test that walk_live reports the same size/mtime_ns as std::fs::metadata for a regular file | done | Non-blocking follow-up from review L; locks TOOL-02's byte-equivalence assumption against a future walkdir upgrade. Batch 1 (test hardening). Batch 1 reviewed; fix commit 62e6be61 (comment-only). |
| F40 | Add a byte-dribble loopback test with a short injected deadline to pin that JSON_BODY_IDLE is a fixed window, not a per-byte resetting idle timer | done | Proposed by batch-1 review; only fixture that distinguishes the two semantics. Batch 3 (timeout & repair-path test hardening). Batch 3 reviewed; no fix needed. |
| F41 | In the stalled-body test, assert the retry budget is actually consumed (one request per attempt) rather than only hits >= 1 | dropped | Non-blocking suggestion from batch-1 review; low priority. Dropped by final pruning: low-priority assertion strengthening; pins no new contract. |
| F42 | Extend the F19 lying-declared-size scenario to the temp path's real-length re-check (capture_slice) | done | Non-blocking suggestion from batch-1 review; pins both downstream guards. Batch 3 (timeout & repair-path test hardening). Batch 3 reviewed; no fix needed. |
| F43 | Add a link counterpart to F39 asserting is_link and the target's size/mtime, gated for platform-specific junction behavior | dropped | Non-blocking suggestion from batch-1 review. Dropped by final pruning: conditional, platform-gated, low value. |
| F44 | Add an e2e test for directory-symlink dedup through the published-checksums path, asserting the exact path set and no double-counted file_count/total_bytes | dropped | Endorsed by batch-2 review, low priority; complements F32, soft-skips without symlink privilege. Dropped by final pruning: low priority, soft-skips, complements done F32. |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code records that in the
relevant task's `note` instead of silently proceeding):

- **The report is a static study of one tree (lines 8–18).** Every finding is a claim
  to validate against the current code; line numbers may have drifted; trust the code.
  Priorities are H/M/L and categories are perf/mem/io/par/correct/fail/diag/build
  (lines 20–23). The summary table at lines 27–105 indexes every finding ID.
- **Assumptions are hard gates (§11, lines 832–888).** A finding whose listed
  assumption does not hold in the current tree is skipped, not forced. Record the
  failed assumption in the task's `note`.
  1. (837–840) S-01/S-02 pair — move repair work off async workers before shrinking
     the runtime; measure with `--jobs 1/4/64` on SSD and HDD.
  2. (841–842) S-03 — chunk ranges must be disjoint for the tiers made concurrent;
     the ceiling ledger must accept out-of-order settlement.
  3. (843–845) IO-01 — the scan's whole-file digest is valid for temp's skip check
     under the same mtime/length guard; a file written earlier in the run must not be
     skipped.
  4. (846–849) IO-02/IO-03 — bounded staging bytes must fit `--max-ram-bytes` /
     `--max-write-bytes`; dropping the second hash must not weaken a required TOCTOU
     guard (fold it into the single read if required).
  5. (850–852) IO-05 — `join_validated` fast path only for plan-derived clean rels;
     never for untrusted input.
  6. (853–855) NET-01 — prefer a per-stream idle timeout over a blanket total
     timeout so large streams on slow links keep working.
  7. (856–858) NET-02 — no production HYP endpoint returns a non-2xx body with a
     usable `retcode`; check captured bundles before enforcing `error_for_status`.
  8. (859–861) NET-05/NET-06 — the decode cap must exceed the real largest manifest
     (16.8 MB for build 7.1.0) and the blob-id shape check must not reject any
     production id.
  9. (862–863) COR-02 — confirm whether `--check-only --scan` is meant to be a fresh
     check before re-validating mtime / falling back to a read.
  10. (864–866) COR-01 — downstream automation may pin exit 1; confirm a new exit
     class is acceptable.
  11. (867–868) LOG-01 — operators can opt back into per-chunk logs with
      `--log-file-level debug`; no bundled artifact depends on those lines.
  12. (869–871) LOG-03 — `lossy(false)` can backpressure a verbose run; consider a
      larger channel plus a periodic dropped-line report.
  13. (872–874) B-01 — confirm with `cargo tree -i chrono` that only the timestamp
      leaf uses it and that a replacement preserves every pinned spelling in
      `crates/time`.
  14. (875–877) TOOL-01/07/08/09/10 — recorded request/response JSON is pinned by
      the artifact bundle; confirm it stays byte-identical.
  15. (878–880) TOOL-03/04/12 — memoizing decoded blobs and the temp-file serialize
      change peak RSS vs. disk I/O; measure on a real capture bundle.
  16. (881–884) TOOL-19 — symlink entries listed as real files may be intentionally
      deduplicated; confirm the bundle contract and add a regression test where the
      link sorts first.
  17. (885–888) TOOL-13/15 — shared canonical walks and parallel assembly must
      preserve documented write order and deterministic artifact bytes; the
      close-out fixpoint passes must be unaffected.
- **Non-findings are keepers (§12, lines 890–905).** The five "checked and
  deliberately not reported" items, the already-tuned Cargo profiles (lines
  810–816) and B-02 (lines 825–829) are not tasks; do not change them.
- **Data plane is stable.** The `REPORT`/`PLAN`/`PROGRESS`/`SUMMARY` stdout contract
  must not change (LOG-01, lines 527–530). T16 is the only task deliberately
  extending `SUMMARY`, and must keep its machine-readable shape stable.
- **Deterministic output and pinned formats survive.** Purge log order in plan order
  (S-05, lines 179–183); girsync's pinned `postcard` digest vocabulary (T24); gircap's
  recorded JSON and artifact bytes (assumptions 14–17).
- **Pairings.** S-01/S-02 land together (assumption 1); NET-07 is the same class as
  S-02; IO-01/02/03 share assumptions 3–4. If one of a pair is skipped, record the
  reason and pass it to the other's session as a pointer.
- **T01 offload pattern (feeds T10).** T01 drives each repair file task's future on
  Tokio's blocking pool with `Handle::block_on`, so `stream_chunk_to` is already polled
  from a blocking context on the repair path; diagnostics are keyed by owner thread
  (`src/repair/diagnostics.rs`). T10 (NET-07) must keep its fetch-stream offload
  consistent with this and must not add a redundant `spawn_blocking` around
  `stream_chunk_to` on that path, nor move it back onto runtime workers. See T01 note.
- **T02 window gating (feeds T10).** The bounded per-chunk fetch window applies only
  where `Tier::Staged && layout.overlaps.is_empty()`; disjointness rests on
  `validate_chunk_ranges` at plan time and defensively in `repair_one_file`. The ledger
  settles out of order exactly; per-chunk wire counts rise on failure (siblings still
  fetched). T02 added no `spawn_blocking` — T10 must preserve the blocking context.
  See T02 note.
- **Runtime / `--jobs` after T01.** Worker count is host-derived
  (`available.max(2).saturating_add(1)`) and independent of `--jobs`; `--jobs` only
  bounds the in-flight file window. Values above the blocking pool's default 512 cap
  queue rather than add concurrency.
- **girsync APIs after T24.** `girsync_cache::hash::hash_file_with_buf(path,
  &AlgoSet, &mut [u8])` is public (empty buffer rejected for non-empty algos; empty
  algos is a no-op); effective phase C hashes via per-worker rayon buffers
  (`try_for_each_init`, abort-on-first-error preserved). The live walk uses
  `ent.metadata()`; under `follow_links(true)` walkdir 2.5.1 caches Windows metadata
  for regular files but re-stats symlinks, with `path_is_symlink()` still carrying the
  link flag. sled2redb is untouched: TOOL-18 is a validated stale skip (`write_records`
  already batches via `CacheWrite::maybe_commit` with a final commit and atomic
  NamedTempFile publish). See T24 note.
- **Final follow-up pruning.** F41, F43 and F44 were dropped (low-priority,
  redundant or platform-conditional). F40 and F42 survived as one bounded Batch 3
  of test-only work; no new follow-ups were proposed. Batch 3 (11feedfb) is done and
  reviewed (no fix); its new suggestion was dropped. All tasks and follow-ups are now
  settled — proceed to Phase 4 cleanup. Plan-ID-shaped comment tokens such as
  (T12/COR-03) and (COR-04) are not path references and are pre-existing style.
- **Follow-up batch 2 outcome.** F31, F32 and F37 are done (implementation
  9d79c13f, no fix needed). The F32 symlink e2e test soft-skips on hosts where
  `symlink_file` is denied (verified on the Windows runner), so a green run there is
  NOT evidence of symlink coverage — the assertions execute only on symlink-capable
  hosts. Batch 2 was test-only: no production code, golden fixture, artifact bytes,
  data-plane string, keeper or pinned contract changed.
- **Follow-up batch 1 outcome.** F13, F19, F25 and F39 are done (implementation
  604390b8, comment-only fix 62e6be61). `HypClient::json_body_idle` is private,
  defaults to `JSON_BODY_IDLE` (30 s), and is settable only from hyp.rs's test module;
  the stalled-body test proves the buffered read is bounded, NOT that the deadline is
  non-resetting — the fixed-window contract lives on JSON_BODY_IDLE's rustdoc.
  `build_local_chunk_map` validates against declared `f.size` with warn+skip and no
  clamp; real-length enforcement is downstream (`salvage_order_refs`, scan `source_ok`,
  temp `capture_slice`) — do not make the map clamp. Chunk-hit assertions are set-based
  because of T02's concurrent window; the duplicate-fetch test is intentionally sorted
  to preserve multiplicity — do not convert it.
- **Follow-up planning outcome.** The guard pruned 31 of the 38 open follow-ups
  (mostly validation-only work that would validate a report claim deleted at cleanup,
  conditional/measurement-gated items, or churn on already-correct code). Survivors:
  Batch 1 (test hardening) = F13, F19, F25, F39; Batch 2 (gircap artifact e2e) = F31,
  F32, F37. No new follow-ups were proposed. Each batch gets one implementation
  session and one review session.
- **chrono build surface (T25 skip).** `cargo tree -i chrono --workspace` shows two
  direct consumers: `girpr-time` and `gircap`. gircap's public API (`ManifestInput.started`,
  `capture_id`, `artifact_dir`) passes chrono `DateTime<Utc>` into `girpr_time`, so the
  four pinned spellings in `crates/time` (compact_stamp, rfc3339_millis_z,
  rfc3339_secs_z, rfc3339) and their chrono-typed tests/signatures are a coupled
  contract. Any future replacement must be coordinated across both crates; trimming
  chrono's `clock` feature from `girpr-time` alone would not remove the workspace-wide
  shims because gircap independently enables `clock`. No code change was made.
- **gircap concurrency / errors after T21-T23.** gircap depends on `rayon` 1.12.0
  (already in the lockfile via girsync). Verify A6 order relies on
  `ChunkIndex::blobs` being id-ordered by contract (test-pinned), not by `build()`; a
  hand-edited unsorted index could shift the first-red A6 headline but cannot affect
  artifact bytes or write order. T21's link-only canonicalization relies on walkdir
  semantics (`file_type()` follows links; `path_is_symlink()` reports links) and dedups
  directories only. T22 runs only CPU work on rayon — all writes, selfcheck rows,
  digests, path_map recording and tracing stay serial in `record()`, so close-out runs
  after build returns (no .json.tmp hazard); the all-results-resident trade-off is
  documented. T23 gives distinct declared-size-mismatch vs ceiling error strings while
  keeping the `decode limit` token; `zstd_measure_from` is unchanged.
- **gircap APIs after T19-T20 (feeds T21/T22).** `HttpResponse::chunk()` returns
  `bytes::Bytes` (not `Vec<u8>`); sinks write `&chunk`. `RequestRecord::req_headers`
  is `Arc<BTreeMap<String,String>>` (serde `rc`); `Fetcher::map_concurrently`'s
  closure takes `Arc<Fetcher>` first. `content_type` is derived from the joined
  `resp_headers` at both fetcher sites (differs from old first-value behavior only for
  repeated/non-UTF-8 headers — recorded-data-only, accepted). The byte-level golden
  fixture (`recorded_json_fixture.rs` + `recorded_json_golden.rs`) is the assumption-14
  gate and passes on pre- and post-change trees. `write_json_gz_counted` is the
  serialize-once path (RAII removes the temp); a crash mid-serialize leaves a
  `.json.tmp` instead of a partial final file — T22's parallel close-out walk must not
  pick up `.tmp` files (currently safe because assembly is serial). TOOL-03/04
  memoization is a validated skip: `ChunkSource::memoizing` is opt-in, unbounded,
  verify-only, with a ~39.5 MB RSS trade-off; "each blob decoded once" does NOT hold
  across A6->rebuild or measure->assemble. See T19/T20 notes.
- **Logging / purge internals after T17-T18.** `girpr_log::default_file_level()` is
  public (Debug in debug builds, Info in release); release file logging defaults to
  Info across girpr, gircap and girsync via the shared crate, with
  `--log-file-level debug|trace` still opting in. The file appender is lossless
  (`lossy(false)`), so verbose opt-in deliberately backpressures rather than drops.
  Purge keep-set probes use borrowed `&str` off Windows and a folded owned key on
  Windows; the depth sort is a stable `sort_by_cached_key` (same-depth walk order
  preserved); MIC-02 is deliberately skipped because `*` crosses `/`. `retry_state`
  threads state by value with no lock; `report_core::write_line` emits a record in
  one `write_all`. See T17/T18 notes.
- **Fail-fast / diagnostics after T14-T16.** Live-lock and game-dir-creation failures
  now exit 3 (`RunFailure::write`) with FATAL; the pinned exit-code table is unchanged
  (external automation pinning exit 1 is unverifiable). `--check-only --scan` is
  deliberately a plan-answer check (`d046eb54`) — do not re-validate mtime or re-hash;
  `ScanReport::failures` / `scan_failed()` surface on the scan tracing line only.
  `promote_tmp` stays path-based (no portable safe handle-relative rename;
  `unsafe_code = "deny"`) — do not "fix" via mtime revalidation or unsafe FFI.
  `plan_work_and_purge` takes a private `purge_wanted: bool`; SUMMARY gained conditional
  numeric `purge_errors`/`purge_hidden` (path count, not bytes) via `Extra::Num`, the
  sole data-plane exception; strict refusal not added. Post-repair re-plan counters
  remain unsurfaced. See T14/T15/T16 notes.
- **Digest / hash internals after T12-T13.** `girpr_digest::Md5::try_parse_hex` is
  public; `parse_hex` keeps identical diagnostics. Reuse keys are `(XOR of the two
  8-byte digest halves, size)` buckets with full `IdentityKey` confirmation. Local
  ranges are validated against the declared `f.size` (warn + skip, no clamp); real
  length/byte re-checks live in `salvage_order_refs`, scan `source_ok`, and temp
  `capture_slice`. `app/util-hash::md5` has a lazy thread-local 512 KiB `MD5_SCRATCH`
  and public `scan_reader_with_buf` / `md5_reader_digest_with_buf`; spare-capacity
  reads use `read_to_end` over `Take` (no unsafe); `RangeState` stores a finalized
  digest and sorts the intern sweep only when order is not already sorted.
  `stream.rs` remains T09/T11-owned. See T12/T13 notes.
- **Fetch client policy after T11 (feeds T19/T20).** `HypClient` has a 10 s connect
  timeout and a 30 s bounded deadline only on the two buffered JSON paths (no blanket
  total on streamed bodies); non-2xx in JSON paths is Fatal, body-read is Retry,
  `get_direct` wrapper-parse is Fatal, and `data:null` maps to `SophonNotFound`.
  `hyp_base()` / `sophon_base()` return `&str`; `app/fetch` declares tokio with the
  `time` feature. MIC-12 was intentionally unchanged (deprecated list already
  normalized in place). See T11 note.
- **Fetch manifest caps / ID validation after T09 (feeds T11, T19/T20).**
  `app/fetch/src/sophon/stream.rs` no longer has `MAX_MANIFEST_HINT`; it now has
  `MANIFEST_INITIAL_CAPACITY` (64 KiB), `MAX_MANIFEST_BYTES` and
  `MAX_MANIFEST_COMPRESSED` (both 64 MiB), with geometric buffer growth. `join_url`
  (and `fetch_manifest` / `try_stream_chunk`) rejects hostile blob-id shapes — any
  tool/test feeding a non-alphanumeric id must use a simple one. The conservative
  unsafe-shape check is deliberate: the strict capture regex would reject real short
  ids and violate assumption 8. See T09 note.
- **Path APIs after T07 (feeds T14/T16/T17).** `PathRoot::join_validated` and
  `PathRoot::new_canonical`, `Lock::acquire_canonical` and `lock_path_for_canonical` are
  public. The resolved-parent cache (`Arc<Mutex<HashMap>>`) is shared only by clones of
  a phase-bound root; clones must not outlive the phase. Leaf canonicalization is
  deliberately retained — do not remove it. Five plan-derived join call sites use the
  fast path; each `contained_join` fallback remains full validation. A parent cached
  while missing and swapped for an outside symlink mid-phase could bypass containment
  via a missing leaf — no in-tree code does this today, but any future mid-run symlink
  creation or longer-lived roots must re-validate the full joined path. See T07 note.
- **Post-phase seam after T08 (feeds T16/T17).** `move_audio_cache_with_mkdir` is the
  private test seam; `rel_from_base` is no longer imported in `post.rs`. The audio-cache
  walk is streamed and dry-run shares that single pass, so audio-cache logging/scope
  changes must preserve the single-pass structure. Unreadable-entry warnings now
  interleave with per-file logs (not a pinned contract). See T08 note.
- **Repair-file API after T06.** Guarded verdict helpers live in `file/mod.rs` and take
  a stat from the caller; `attempt_temp` takes a `FileId`; `RepairDiagnostics` gained a
  public `reuse_io_fallbacks` counter. `file.layout()` is computed once per file and
  passed as `&Layout`. The temp reuse staging buffer holds one chunk at a time and is
  not charged to `--max-ram-bytes` (temp mode never used that budget), so peak temp RSS
  rises by one chunk. The staged-source hash is a required TOCTOU re-check, not a
  redundant double hash — do not remove it.
- **Plan stat vs scan stat are not interchangeable (feeds T06).** `DiskPlan::new`
  needs `symlink_metadata` for is_file()/length; `scan_one` uses follow-symlink
  `metadata` plus a bytes-read length for the mtime verdict guard. Sharing one stat
  would change symlink/dir → temp classification and verdict-trust semantics. T05
  therefore overlaps the two blocking tasks rather than merging them; T06 must keep
  the `symlink_metadata` source for the mode decision. See T05 note.
- **Plan-work API after T05 (feeds T06).** `publish_plan(ctx, &work)` no longer takes
  `game_dir`/`root` nor computes figures; it consumes `work.disk`, computed and awaited
  in `build_and_assert_plan` (`PlanWork` gained `disk: Option<DiskPlan>`). Later tasks
  touching that call site must preserve this.
- **Purge threads after T04 (feeds T16).** Purge stat/delete run on scoped threads with
  a thread-local tracing dispatcher; new purge diagnostics must be emitted on the caller
  thread via index slots, not from workers. On Windows the stat stage reads cached
  `DirEntry` metadata (no syscall), so any speedup there comes only from deletes.
  See T04 note.
- **Plan-phase fetch API after T03.** `src/repair/phases/manifest.rs` exports
  `ManifestBudget` (shared 4-permit semaphore); `fetch_latest` / `local_reuse_map` take
  it, and `PlanInputs` has a `deprecated` field. T11/T12 fetch work must build on this.
  See T03 note for the scope-typo advisory-request deviation.
- **No lingering references at cleanup.** No source, test, comment, manifest, doc
  table or commit message may reference `REFACTOR_PLAN/` or
  `docs/OPTIMIZATION_REPORT.md`.
