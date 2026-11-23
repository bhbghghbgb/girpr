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
| T07 | Add validated path joins and cache canonicalization | in_progress | | §3 IO-05/IO-06 (252–276); assumption 5 (850–852) |
| T08 | Remove redundant post-phase mkdir syscalls and materialization | todo | | §3 IO-07/IO-08 (277–293) |
| T09 | Bound streaming decode output and validate blob IDs | todo | | §4 MEM-01 (297–308); §5 NET-05/NET-06 (386–403); assumption 8 (859–861) |
| T10 | Move streaming decode/write off async workers | todo | | §5 NET-07 (404–413); same class as S-02/T01 |
| T11 | Add HTTP timeouts, status checks and fetch-layer micro-optimizations | todo | | §5 NET-01..04 (349–385); §4 MEM-05 (337–345); §8 MIC-04/05/12 (591–599, 630–636); assumptions 6–7 (853–858) |
| T12 | Validate reuse offsets and parse digests once | todo | | §6 COR-03 (442–451); §8 MIC-01/08/11 (564–573, 610–613, 624–629) |
| T13 | Reuse hash scratch buffers and tighten range handling | todo | | §4 MEM-03/04 (320–336); §8 MIC-09/10 (614–623) |
| T14 | Classify lock and directory failures correctly | todo | | §6 COR-01 (417–428); assumption 10 (864–866) |
| T15 | Fix stale scan/check verdicts, surface scan failures, close tmp TOCTOU | todo | | §6 COR-02/COR-05/COR-08 (429–441, 464–472, 496–504); assumption 9 (862–863) |
| T16 | Move scope-typo refusal earlier and surface purge diagnostics | todo | | §6 COR-06/COR-07/COR-09 (474–512) |
| T17 | Reduce logging cost without changing the data plane | todo | | §7 LOG-01..03 (517–559); assumptions 11–12 (867–871) |
| T18 | Low-risk micro-optimization sweep | todo | | §8 MIC-02/03/06/07 (574–589, 600–609); §4 MEM-02 (309–319) |
| T19 | Cut allocations in gircap's HTTP recording pipeline | todo | | §9 TOOL-01/07/08/09/10 (643–652, 691–715); assumption 14 (875–877) |
| T20 | Memoize decoded blobs and serialize the manifest once | todo | | §9 TOOL-03/04/11/12 (661–678, 716–728); assumption 15 (878–880) |
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

## Follow-ups

Non-blocking suggestions from review sessions; the orchestrator records them here.
Reviewers decide which are worth doing and may prune them to avoid run-away. A
planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Stress-test a plan with `--jobs` above Tokio's default blocking-pool cap (512) to confirm queuing, no deadlock/starvation, and that the file-window bound holds | open | Endorsed by review A; ties to assumption 1 (report lines 837–840) and T01. |
| F02 | Validate a full repair at `--jobs 1/4/64` (and on HDD where possible), not just the host-independent worker arithmetic | open | Non-blocking validation follow-up from review A; assumption 1 (report lines 837–840). |
| F03 | Add a many-small-chunks end-to-end stress case with per-chunk timing to confirm real network/decompression overlap from the T02 window, not just structural correctness | open | Endorsed by review B; temp-mode windowing extension (its resume-by-length/rollback coupling) dropped. |
| F04 | Conditionally defer the deprecated prefetch when `path_scope.has_includes()` is active, restoring the "no advisory request on scope typo" property while keeping overlap in the common unscoped case | open | Endorsed by review B; low priority — the extra advisory request is harmless, usage error/message unchanged. |
| F05 | Validate peak RSS under high `--jobs` with the 4x per-file decoder window to confirm tight `--max-ram-bytes` ceilings are not pressured by transient decoder scratch | open | Non-blocking validation follow-up from review B. |
| F06 | Measure purge wall time at scale (10k+ orphans, SSD/HDD, both platforms) before claiming a cross-platform speedup | open | Endorsed by review C; on Windows the stat stage issues no syscall (cached DirEntry metadata), so only delete benefits there. |
| F07 | Let `scan_one` capture a `symlink_metadata`-derived `is_file`/`length` so one syscall serves plan and scan | dropped | T06 investigation confirmed guard-equivalence fails (scan needs follow-symlink modified + bytes-read file_len; plan needs symlink_metadata is_file/length); review D closed it. No code change. |
| F08 | Measure peak temp-mode RSS with large reuse slices under tight `--max-ram-bytes`/`--max-write-bytes` to decide whether a bounded spool fallback is worth adding while keeping the single read | open | Endorsed by review D; one-chunk transient RAM is bounded but uncharged to the budget and unmeasured. |

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
