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
| T19 | Cut allocations in gircap's HTTP recording pipeline | pending_review | 54d186e4 | §9 TOOL-01/07/08/09/10 (643–652, 691–715); assumption 14 (875–877) validated with a byte-level golden fixture (passed pre- and post-change). API: HttpResponse::chunk() -> Result<Option<bytes::Bytes>, String>; RequestRecord::req_headers is Arc<BTreeMap<...>> (serde rc); Fetcher::map_concurrently closure takes Arc<Fetcher> first. TOOL-07 edge: content_type now from joined resp_headers (differs for repeated/non-UTF-8; pinned bundle uses single ASCII). |
| T20 | Memoize decoded blobs and serialize the manifest once | pending_review | 6d4e3888, 12e73ec4 | §9 TOOL-03/04/11/12 (661–678, 716–728); assumption 15 (878–880). Implemented TOOL-11 (move measured md5/error into rows) and TOOL-12 (new public write_json_gz_counted: serialize once to a counted temp with RAII cleanup, then copy/gzip; byte-identical test). SKIPPED TOOL-03/04 memoization: ChunkSource::memoizing already exists in verify but is unbounded and deliberately not used in capture (+39.5 MB RSS trade-off); no real bundle to measure (only 44 KB synthetic), so assumption 15 cannot be validated. "Each blob decoded once" does NOT hold across A6->rebuild or measure->assemble. Reviewer to decide final state. |
| T21 | Fix gircap indexing and canonicalization correctness | todo | | §9 TOOL-05/06/13/17/19/20 (679–690, 729–737, 763–768, 780–795); assumptions 16–17 (881–888) |
| T22 | Parallelize gircap verification and capture assembly | todo | | §9 TOOL-14/15 (740–752); assumption 17 (885–888) |
| T23 | Make gircap declared-size mismatch errors honest | todo | | §9 TOOL-21 (796–805) |
| T24 | Cut girsync and sled2redb I/O and transaction cost | todo | | §9 TOOL-02/16/18 (654–659, 756–762, 770–777) |
| T25 | Validate and, if safe, shrink the build-time dependency surface | todo | | §10 B-01 (818–823); assumption 13 (872–874) |

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

## Follow-ups

Non-blocking suggestions from review sessions; the orchestrator records them here.
Reviewers decide which are worth doing and may prune them to avoid run-away. A
planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Stress-test a plan with `--jobs` above Tokio's default blocking-pool cap (512) to confirm queuing, no deadlock/starvation, that the file-window bound holds, and that chunk streaming is polled off runtime workers | open | Endorsed by reviews A and F; ties to assumption 1 (report lines 837–840), T01 and the T10 documented-contract check. |
| F02 | Validate a full repair at `--jobs 1/4/64` (and on HDD where possible), not just the host-independent worker arithmetic | open | Non-blocking validation follow-up from review A; assumption 1 (report lines 837–840). |
| F03 | Add a many-small-chunks end-to-end stress case with per-chunk timing to confirm real network/decompression overlap from the T02 window, not just structural correctness | open | Endorsed by review B; temp-mode windowing extension (its resume-by-length/rollback coupling) dropped. |
| F04 | Conditionally defer the deprecated prefetch when `path_scope.has_includes()` is active, restoring the "no advisory request on scope typo" property while keeping overlap in the common unscoped case | open | Endorsed by review B; low priority — the extra advisory request is harmless, usage error/message unchanged. |
| F05 | Validate peak RSS under high `--jobs` with the 4x per-file decoder window to confirm tight `--max-ram-bytes` ceilings are not pressured by transient decoder scratch | open | Non-blocking validation follow-up from review B. |
| F06 | Measure purge wall time at scale (10k+ orphans, SSD/HDD, both platforms) before claiming a cross-platform speedup | open | Endorsed by review C; on Windows the stat stage issues no syscall (cached DirEntry metadata), so only delete benefits there. |
| F07 | Let `scan_one` capture a `symlink_metadata`-derived `is_file`/`length` so one syscall serves plan and scan | dropped | T06 investigation confirmed guard-equivalence fails (scan needs follow-symlink modified + bytes-read file_len; plan needs symlink_metadata is_file/length); review D closed it. No code change. |
| F08 | Measure peak temp-mode RSS with large reuse slices under tight `--max-ram-bytes`/`--max-write-bytes` to decide whether a bounded spool fallback is worth adding while keeping the single read | open | Endorsed by review D; one-chunk transient RAM is bounded but uncharged to the budget and unmeasured. |
| F09 | Add a `CleanRel` newtype returned by `clean_rel` to enforce the `join_validated` precondition structurally instead of by debug assertion | open | Endorsed by review E, low priority; all five current call sites are correct and test-covered. |
| F10 | Measure the path syscall reduction from T07 on a large install before claiming a speedup | open | Endorsed by review E; report figures are static reasoning. |
| F11 | Measure the audio-cache move at scale (many files per destination directory) to quantify the mkdir/syscall reduction | open | Endorsed by review E; report figures are static reasoning. |
| F12 | Revisit the fixed 64 MiB manifest ceilings if a future manifest grows past them, and share one id-shape definition between fetch and capture tools | open | Endorsed by review F; keep the conservative fetch check — the strict capture regex would reject real short ids. |
| F13 | Add a fetch-side test for the JSON body idle/deadline timeout using a stalled loopback origin with an injectable short duration; clarify that JSON_BODY_IDLE is a bounded deadline on small buffered bodies, not a resetting timer | open | Endorsed by review F; the timeout path is currently untested. |
| F14 | Add a resetting per-piece idle timeout inside `sophon/stream.rs::stream_body` for mid-stream blob stalls (never a total deadline) | open | Endorsed by review F, lower priority; touches the file shared with T09/T10 — schedule after those in-flight tasks. |
| F15 | Replace `by_key` buckets with `HashMap<IdentityKey, usize>` to drop the Vec/collision-confirmation machinery | open | Endorsed by review G, low priority; MIC-11 alternative, cleanup only (current code is correct). |
| F16 | Add a log-capture assertion that the T12 invalid-range warning is actually emitted | open | Endorsed by review G, low priority; app/fetch has no log-capture dev-dependency yet. |
| F17 | Convert `file/mod.rs` `scratch_slice`/piece-copy loops to spare-capacity reads | open | Endorsed by review G conditionally (low priority) — only if the one-time-per-file 512 KiB fill proves measurable. |
| F18 | Consider task-owned rather than thread-local hash buffers if 512 KiB per blocking-pool thread becomes a concern | open | Endorsed by review G conditionally (low priority); revisit with RSS evidence. |
| F19 | Add a unit test that a lying declared `f.size` (larger than the real file) cannot turn a bad reuse offset into an intact verdict | open | Non-blocking follow-up from review G; existing tests cover negative/overflow/past-declared-EOF, not declared-vs-real mismatch. |
| F20 | Add a binary-level e2e case asserting process status 3 plus FATAL text for a held lock and failed game-dir creation | open | Endorsed by review H, low priority; API-level tests already assert RunExit::Write. |
| F21 | Revisit COR-08 if a safe portable handle-relative rename primitive appears, or evaluate post-rename identity verification | open | Endorsed by review H; tracks the validated skip in T15. |
| F22 | Surface the post-repair re-plan errors/hidden counters by threading `&mut Summary` through the post phase | open | Endorsed by review H, low priority; only the up-front purge plan's counters are currently surfaced. |
| F23 | Consider strict refusal on purge errors as a separate, explicitly opted-in change | open | Endorsed by review H; must stay opt-in and separate, never a silent change. |
| F24 | Add an end-to-end test injecting an unreadable purge candidate to assert the new SUMMARY keys | open | Endorsed by review H; current coverage of the new keys is unit-level. |
| F25 | Make the order-sensitive chunk-hit assertions (e.g. tests/repair_writer.rs) compare as a set to remove the pre-existing flake under T02's concurrent fetch window | open | Endorsed by review I, low priority; a BTreeSet comparison is already used in the windowed test. |
| F26 | Revisit MEM-02's three keyed PurgeKeys copies via a borrowing-friendly type or a compact membership hash | open | Endorsed by review I conditionally — only if profiling shows the resident strings matter. |
| F27 | Validate lossless-appender backpressure under a trace-level large repair (--log-file-level trace with the 128k channel) | open | Non-blocking validation follow-up from review I; assumption 12. |

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
