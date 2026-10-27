# ADVERSARIAL REVIEW NOTE — third pass

**Reviewed tree:** branch `fix-gaps/girpr`, HEAD `fa89641`
(`docs(review): fold the second-pass verdicts into the review`). The working
tree was clean at the start and is clean apart from this note.

**Baseline:** `ADVERSARIAL_REVIEW.md` — §2 (F-1…F-9), §7 (fix record) and §8
(second pass, N-1…N-14). Its sentences were re-checked against this tree, not
trusted.

**Method:** every finding cites the current tree and the control/data path was
read end to end. The arithmetic/contract findings below were also **executed**
with a throwaway crate outside the tree (`E:\Symlinks\Temp\opencode\girpr_probe`,
not committed) that links `girpr` and calls the public functions; §4 records the
exact commands and output. No source, test, doc, README or Cargo file was
changed.

---

## 1. Verdict

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| N-15 | Medium | bug | `src/repair/scope/report.rs:102`, `:261-271` | `DiskPlan`/`PlanDocument` add server-controlled sizes without saturation: a debug-build panic and a release-build wrap in `--print-plan`/`--plan-out`. |
| N-16 | Medium | bug | `src/repair/scope/report.rs:271-276` | `--plan-out` per-file estimates stop adding up to `PLAN est_download_bytes` whenever the estimate is smaller (the `--scan` case); the reconciliation is a no-op in exactly that direction. |
| N-17 | Medium | bug | `src/repair/scan.rs:75-98` | The scanned estimate ignores the ordered tier's salvage liveness (and the in-place→temp fallback), so it understates the bytes the repair downloads. |
| N-18 | Low | bug | `src/repair/scan.rs:198-209` | A chunk with **no** reuse source is scanned at offset 0, so unrelated bytes at the file head can mark its source intact and hide a download from the estimate. |
| N-19 | Low | bug | `src/repair/purge.rs:91`, `:107` | Walk errors and candidate metadata errors are swallowed: orphans under an unreadable directory are never planned, the byte total understates, and nothing is logged. |
| N-20 | Low | bug | `src/repair/file/temp.rs:96`, `:164` | `File::metadata()` failures become length 0, skipping the oversize truncate and the failed-chunk rollback; a recoverable `_tmp` can be retried into failure with no diagnostic. |

Severity follows `ADVERSARIAL_REVIEW.md` §1: Medium = a documented contract or
enforcement ceiling is violated, damage bounded or precondition narrower; Low =
diagnostics or coverage, no wrong repair result alone.

---

## 2. Prior fixes re-checked

| Item | Verdict | Evidence in this tree |
|---|---|---|
| F-1 per-file panics | **Fixed** | `join_file_tasks` counts every join error into `files_failed` and trips `--fail-fast` (`src/repair/file/mod.rs:345-368`). Tests `a_panic_inside_a_file_task_fails_the_run` (`:794`) and `a_panic_is_observed_before_a_slower_earlier_task` (`:858`) pass. |
| F-2 uncontained server paths | **Fixed** | `contained_rel`/`contained_join` at `src/util.rs`; applied at `phases/plan.rs:210`, `scope/report.rs:78`, scan/check/`_tmp`/config/audio/deprecated sinks. The Windows-runnable regression tests named in §7 pass; the `#[cfg(unix)]` symlink test is listed in §5. |
| F-3 partial-body refund | **Fixed** | `settle_reservation` settles the real figure and stops on `StoppedBy::Download` when it crosses the limit (`src/repair/file/temp.rs:423-459`); `a_partial_body_then_a_drop_is_charged_to_the_download_meter` (`tests/repair_offline.rs:1917`) passes and asserts exit `5`. |
| F-4 `--print-plan --audio` write | **Fixed** | `RunCtx::readonly` documents the two-gate rule (`src/config.rs:177-189`); the audio-scan write is gated on both. `print_plan_with_audio_writes_no_scan_file_and_names_the_language` (`tests/repair_offline.rs:2143`) passes. |
| F-5 `--check-only --scan` length | **Fixed** | The scan branch compares presence/length as well as the whole-file MD5 (`src/repair/phases/check.rs:38-79`); `check_only_scan_treats_an_absent_empty_file_as_damage` (`tests/repair_offline.rs:195`) passes. |
| F-6 dropped scan panic | **Fixed** | A failed scan task is warned and the file is estimated in full (`src/repair/scan.rs:133-146`). |
| F-7 `--max-write-bytes` prose | **Fixed** | All six sites (`src/cli.rs:89-96`, `scope/limits.rs:20`/`:55`, `report.rs`, `ledger.rs`, `docs/04:168`/`:291`) now say install/target vs `_tmp`. |
| F-8 regression coverage | **Fixed** | Present and passing: `partial_tmp_prefix_is_resumed` (`:374`), `a_full_length_corrupt_tmp_is_deleted_and_rebuilt` (`:415`), the three escape tests (`:458`, `:485`, `:523`), the offset-overflow test (`:580`), the partial-body test (`:1917`), the print-plan-audio test (`:2143`). |
| F-9 `kept_bytes` doc | **Fixed** | `src/repair/file/mod.rs:264-269` now says a resumed `_tmp` prefix is kept and counted, unverified until the final MD5. |
| N-1…N-14 | **Spot-checked, no contradiction found** | The gates named in §8 are present at their cited sites (fail-closed resolution, settle-time stop, audio source retained, config-dir validation, Windows-only dedup, `./` cleaning, completion-order join, saturation, device names); they were not re-audited one by one. One wording caveat is folded into N-19. |

Residual, not a finding: the second pass's N-11 says “walk failures are
warned”, which is true for the audio walk (`phases/post.rs:238-251`) and not for
the purge walk (N-19).

---

## 3. Findings

### N-15 — Medium — unchecked `u64` aggregation of server sizes in the plan report

**Location:** `src/repair/scope/report.rs:102`; `:261-271`.

```rust
// DiskPlan::new, :102
out.est_write_bytes += size;
```

```rust
// PlanDocument::new, :261-271
let mut per_file: Vec<u64> = plan.files.iter()
    .map(|f| f.chunks.iter()
        .map(|c| c.compressed_size.max(0) as u64)
        .sum())
    .collect();
let summed: u64 = per_file.iter().sum();
```

**Evidence.** `size` (`f.size`, `plan.rs:243-248`) and `compressed_size` are
server manifest data with no cap; the tree already treats this exact data class
as hostile and saturates it in both estimates (`phases/plan.rs:163-168`,
`scan.rs:92-96`, test `an_overflowing_estimate_saturates_instead_of_panicking`
at `scan.rs:281-311`). The report aggregations were missed.

Executed with the probe:

```
probe doc   -> panicked at core/src/iter/traits/accum.rs:206: attempt to add with overflow
               (backtrace frames: src/repair/scope/report.rs:268 and :270)
probe disk  -> panicked at src/repair/scope/report.rs:102:13: attempt to add with overflow
```

**Failure scenario.** A manifest with three files each declaring
`size = i64::MAX` reaches `DiskPlan::new` from the `--print-plan` branch
(`src/repair/mod.rs:310`); `3 × (2^63−1)` exceeds `u64::MAX` and the debug build
panics. A file with three chunks each declaring `compressed_size = i64::MAX`
does the same in `PlanDocument::new` (frame `:268`/`:270`), reachable through
`--plan-out` (`mod.rs:323-328`). In a release build the same input wraps:
`est_write_bytes` becomes `9223372036854775805` and the `PLAN` line reports
that as the disk the repair needs. `build_plan` only warns about a layout that
does not tile (`plan.rs:253-261`); no preflight ceiling covers write bytes
(`scope/limits.rs:98-128`), so neither path is refused earlier.

**Standard violated.** The tree's own invariant for server-controlled sizes:
saturate and over-approximate, never panic in the plan phase (N-13); plus
fail-fast/no-panic on metadata. A wrapped disk estimate is a `PLAN` line that
lies.

**Suggested fix.** Use `saturating_add` at `:102` and a saturating fold at
`:261-271`; add one unit test per site mirroring
`an_overflowing_estimate_saturates_instead_of_panicking` (three files at
`i64::MAX`; three chunks at `i64::MAX`).

---

### N-16 — Medium — `--plan-out` disagrees with `PLAN` when the estimate is smaller

**Location:** `src/repair/scope/report.rs:271-276`; caller
`src/repair/mod.rs:266-270`, `:323-328`.

```rust
let summed: u64 = per_file.iter().sum();
if summed != est_download_bytes
    && let Some(first) = per_file.first_mut()
{
    *first = first.saturating_add(est_download_bytes.saturating_sub(summed));
}
```

**Evidence.** The document starts from `Σ compressed_size` over **all** chunks.
With `--scan` and the in-place path, the caller passes
`s.est_download_bytes(&plan)` (`mod.rs:270`), which counts only chunks that are
wrong where they belong **and** have no intact source (`scan.rs:81-85`) — so it
is normally smaller than the naive sum. In that direction
`est_download_bytes.saturating_sub(summed)` is `0`, the first entry is
unchanged, and the document total stays larger than the `PLAN` figure the
comment at `:257-260` promises it equals. The doc on `RepairEntry::est_download_bytes`
(`:235-236`) makes the same promise (“this file's half of the plan's
`est_download_bytes`”), and the test at `:530-538` only covers `summed < est`.

Executed with the probe (two 10-byte chunks, one already reusable, `est = 10`):

```
plan est=10 document sum=20
per-file=[20]
```

**Failure scenario.** `girpr --scan --print-plan --plan-out plan.json` on an
install where one chunk is already correct prints `PLAN … est_download_bytes=10`
and writes a document whose entries sum to `20`; a consumer that adds the
per-file halves (the documented meaning) budgets twice the scan's figure and
disagrees with the same run's `PLAN` line.

**Standard violated.** Documented contract (“the document must agree with the
`PLAN` line”, pinned by `the_document_lists_every_file_and_the_estimates_sum_to_the_plan`);
test gap for the downward direction.

**Suggested fix.** Build the per-file split from the same source as the total
(pass the `ScanReport` or a per-file `Vec<u64>` from `mod.rs`), instead of
reconciling a naive sum. If reconciliation is kept, handle `summed > est` too
(e.g. recompute from the scan) rather than silently skipping it; add a test
with `est < summed`.

---

### N-17 — Medium — the scanned estimate ignores the write path the run takes

**Location:** `src/repair/scan.rs:75-98`; `src/repair/file/inplace.rs:210-237`,
`:263-282`, `:300-315`; fallback `src/repair/file/mod.rs:480-485`; estimate
choice `src/repair/mod.rs:266-270`.

```rust
// scan.rs:81-85
.filter(|(i, _)| !v.dest_ok[*i] && !v.source_ok[*i])
.fold(0u64, |n, (_, c)| n.saturating_add(c.compressed_size.max(0) as u64)),
```

```rust
// inplace.rs:300-303 — the ordered tier reuses only `live` sources
} else if let Some(o) = &order
    && o.live[k]
    && let Some(src) = sources[i]
```

**Evidence.** The scan calls a chunk reusable whenever its source bytes verify
(`source_ok`). The writer does not: the ordered tier reuses only the sources the
chosen direction keeps alive (`inplace.rs:263-282`, gate at `:300-315`), and
the staged tier reuses only sources whose MD5 still verifies at capture
(`:240-257`). The ordered tier is selected whenever the RAM budget cannot hold
the payload (`:215-237`), i.e. for `--max-ram-bytes 0` or any budget below the
payload.

Executed with the probe — two damaged chunks whose intact sources point at each
other (swapped halves):

```
scan est_download_bytes=0
ordered tier: ascending=true live=[true, false] salvaged=1 of 2
```

With `--max-ram-bytes 0` the repair takes the ordered tier and downloads the
dead source's chunk (20 bytes) while the estimate said 0. The same shape occurs
when an in-place open fails and the file permanently falls back to temp
(`mod.rs:480-485`): the temp path downloads every chunk without a source,
including `dest_ok` chunks the scan excludes.

**Failure scenario.** `girpr --scan --max-ram-bytes 0` (or a reflowed file over
the budget): `PLAN est_download_bytes` is below the bytes the run downloads.
The preflight `--max-download-bytes` (`scope/limits.rs:111-121`) passes on that
number, and the run then stops mid-flight on the runtime ceiling with exit `5` —
the estimate's job was to refuse before doing any work.

**Standard violated.** `ScanReport::est_download_bytes` is documented as “the
bytes the repair will really download”, and N-13's comment states the estimate
must stay an over-approximation. This is an under-approximation with a
supported flag combination.

**Suggested fix.** Compute the estimate with the same tier/order decision the
writer makes (the scan already has `dest_ok`), or make it conservative by
counting any chunk with a source that is dead in either direction and any file
that may fall back to temp. Add an integration test with conflicting sources
and `--scan --max-ram-bytes 0` asserting `PLAN est_download_bytes == SUMMARY
download_bytes`.

---

### N-18 — Low — “no source” is scanned as “source at offset 0”

**Location:** `src/repair/scan.rs:198-209`.

```rust
.map(|(c, src)| {
    (
        src.unwrap_or(0),
        c.uncompressed_size.max(0) as u64,
        c.uncompressed_md5.as_str(),
    )
})
```

**Evidence.** `sources_for` returns `None` when no `(md5, size)` entry matches
(`src/repair/reuse.rs:148-160`). `unwrap_or(0)` then asks `util::scan_file` to
verify the chunk's length and MD5 **at file offset 0**. If those bytes match
(e.g. a manifest/map state where the head of the file duplicates the chunk's
content but is not recorded as a chunk pair — a longer first chunk containing a
copy, or a map built before a concurrent game write), `source_ok[i]` is `true`
although there is no source to copy from. The writer checks
`sources[i].is_some()` (`inplace.rs:212`, `:245`, `:302`; `temp.rs:220-228`), so
it downloads the chunk anyway while `est_download_bytes` skips it.

**Failure scenario.** A reflowed file where the chunk's content exists only as
a prefix of a longer first chunk: `--scan --print-plan` reports a download
estimate smaller than the repair's actual `download_bytes`; with a ceiling the
preflight can pass and the run stop later.

**Standard violated.** Absent must not be coerced into a valid value; the
estimate must not mark a non-existent source intact.

**Suggested fix.** Only push a source range when `src` is `Some`, and force
`source_ok[i] = false` for the rest (or hand `scan_file` a range that cannot
match). Add a unit test with duplicate content and no matching map entry.

---

### N-19 — Low — the purge walk and candidate sizes swallow errors

**Location:** `src/repair/purge.rs:88-92`, `:107`; total at `:127`.

```rust
for e in WalkDir::new(game_dir)
    .follow_links(false)
    .into_iter()
    .filter_map(|e| e.ok())
...
let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
```

**Evidence.** An unreadable subdirectory (permissions, a lock, an I/O error)
produces `Err`, which `filter_map` drops: the walk does not descend, so the
orphans underneath are never candidates, `hidden` is not incremented, and no
line records why. A candidate whose `metadata()` fails is counted as `0` bytes.
`total_bytes` (`:127`) therefore understates the delete set, and the preflight
and re-plan delete ceilings (`scope/limits.rs:122-127`, `:141-144`) compare
against the understated number. The parallel audio walk does warn on the same
condition (`phases/post.rs:238-251`), and §8.2's N-11 records “walk failures are
warned”; the purge walk is the outlier.

**Failure scenario.** A directory under the install that the current user cannot
list: the run reports `purged_bytes` from the visible subset and exits `0` while
orphans remain; with `--max-delete-bytes` the ceiling may also pass on a total
that omits them.

**Standard violated.** No swallowed errors — `filter_map(|e| e.ok())` is named
in the review standard — and the delete ceiling must see the whole candidate
set.

**Suggested fix.** Match the walk item, warn with the error and keep a count (or
refuse the purge plan when the walk cannot be trusted); log/propagate the
metadata failure instead of substituting `0`; saturate the total. Add an
unreadable-directory test (unix permissions or a Windows ACL).

---

### N-20 — Low — temp resume/rollback treat a metadata failure as length 0

**Location:** `src/repair/file/temp.rs:96`, `:164`; `:207` fails safe.

```rust
let have = f.metadata().map(|m| m.len()).unwrap_or(0);   // :96
...
let len = f.metadata().map(|m| m.len()).unwrap_or(0);    // :164
```

**Evidence.** `f` is an already-open handle, so a metadata failure is a real I/O
error, not “empty”. At `:96` a failure skips the oversize truncate. At `:164`
(`rollback_tmp`) a failure makes `len = 0 <= offset` and returns **without**
truncating the failed chunk's partial bytes. The next attempt's resume check
(`try_reuse_slice`, `:207-217`) then sees `cur_len >= end` and returns `Resumed`
for those bytes without hashing them. The final MD5 gate (`promote_tmp`,
`:475-489`) refuses promotion and deletes the `_tmp`, so no corrupt file lands,
but a file that a truncate would have recovered burns its attempts and can be
counted failed; nothing is logged. (`:207`'s own fallback is safe: a zero
resume length means “download”.)

**Failure scenario.** A transient stat failure after a failed chunk: the tmp is
treated as complete from that chunk on, the whole-file MD5 fails, the tmp is
deleted, and the retry loop consumes `MAX_ATTEMPTS` — the file fails while the
recoverable path (truncate and continue) was skipped silently.

**Standard violated.** No swallowed errors; fail-fast over “default to zero”.

**Suggested fix.** Propagate `f.metadata()` with context (`?`) at `:96` and
`:164`; the handle is open, so an error here should fail the attempt with a
diagnostic rather than invent a length.

---

## 4. Verification

All commands run in `E:\hoccode\hocrust\girpr` on `fa89641`.

```
$ cargo build -p girpr
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.42s

$ cargo test -p girpr
    unittests src/lib.rs        166 passed; 0 failed; 0 ignored
    unittests src/main.rs         0 passed; 0 failed; 0 ignored
    tests/artifact_bridge.rs      0 passed; 0 failed; 6 ignored
    tests/artifact_server.rs      7 passed; 0 failed; 0 ignored
    tests/cli_contract.rs        16 passed; 0 failed; 0 ignored
    tests/peak_heap.rs            0 passed; 0 failed; 1 ignored
    tests/repair_machinery.rs    25 passed; 0 failed; 0 ignored
    tests/repair_offline.rs      82 passed; 0 failed; 0 ignored
    Doc-tests girpr               0 passed; 0 failed; 0 ignored
    total                       296 passed; 0 failed; 7 ignored; exit 0

$ cargo clippy -p girpr --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 32.33s
    (no warnings)

$ cargo fmt --check
    (no output, exit 0)
```

Probe (scratch crate outside the tree, `CARGO_TARGET_DIR` pointed at the tree's
`target`; not part of the repository):

```
$ cargo build                                    # in girpr_probe
    Finished `dev` profile ... in 2.52s

$ girpr_probe.exe disk
    panicked at src/repair/scope/report.rs:102:13: attempt to add with overflow
$ girpr_probe.exe doc
    panicked at core/src/iter/traits/accum.rs:206: attempt to add with overflow
    (backtrace frames: src/repair/scope/report.rs:268, :270)
$ girpr_probe.exe reconcile
    plan est=10 document sum=20
    per-file=[20]
$ girpr_probe.exe scan
    scan est_download_bytes=0
    ordered tier: ascending=true live=[true, false] salvaged=1 of 2
```

`git status --porcelain` before this note: empty. After: only
`ADVERSARIAL_REVIEW_NOTE.md` is added.

---

## 5. Not verified

- `#[cfg(unix)]` code paths: `contained_join_resolves_a_symlink_and_refuses_the_escape`
  (`src/util.rs:852-864`), the unix block of
  `contained_join_fails_closed_when_a_component_cannot_be_resolved` (`:891-899`)
  and of `the_mode_is_part_of_the_file_stats` (`src/repair/file/mod.rs:724-729`).
  The host is Windows.
- The 7 ignored tests were not run: 6 in `tests/artifact_bridge.rs` (expensive,
  real-bundle runs) and 1 in `tests/peak_heap.rs` (a measurement).
- N-15/N-16/N-17 were executed at the function level through the probe, not
  through a full `girpr` run with a hostile manifest or mock server; their
  end-to-end exit codes and output lines were not observed.
- N-18/N-19/N-20 are source-verified only: the private `scan_one`, an
  unreadable-directory walk and a failing `File::metadata()` were not
  exercised.
- No release-profile run, so the wrap half of N-15 is arithmetic, not observed.
- No Linux/macOS run, no Miri/fuzz/loom, no network or live CDN traffic.
