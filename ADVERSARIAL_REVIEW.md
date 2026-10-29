# ADVERSARIAL REVIEW — `girpr` (root crate)

**Reviewed tree:** branch `all/fix-gaps`, HEAD `64dc69d` (`revert: drop the
adversarial review series (tree back to 7f2d69)`).
**Baseline:** `7f2d69c`. `git diff --stat 7f2d69c HEAD` is empty, so this tree is
byte-identical to the pre-fix baseline. The prior review/fix series
`0e4f011..f9352f7` was reverted by `64dc69d`; its review at
`f9352f7:ADVERSARIAL_REVIEW.md` is historical input, not evidence.

This document is the pass log of a fresh source-level review of the current
tree. It does not assume any sentence of the reverted review is true. The fixes
landed on `fix-gaps/girpr`; the pass log opens with the first pass, and every
later pass carries its findings to a final verdict. Deliberate trade-offs,
unverified areas and product decisions that remain open are listed in
[`OPEN_FOR_MAINTAINERS.md`](OPEN_FOR_MAINTAINERS.md).

---

## Pass log

Detailed evidence, failure scenarios and per-pass verification are in the git
history — `git log --oneline main..HEAD`, `git show <commit>` — and the commit
hashes in the tables are the index. The uncompacted first pass, its corrections
to the reverted series and its recommended landing order are at
`git show 57178e8:ADVERSARIAL_REVIEW.md`; their line numbers are from `64dc69d`
and have moved since. `util::normalize_rel` (F-2) no longer exists: N-9 replaced
it with `contained_rel`/`contained_join`. This compaction changed no source,
test or behaviour.

### First pass — F-1…F-9

All nine findings were fixed on this branch; the detailed evidence, failure
scenarios and corrections to the reverted series are in git history
(`git show 57178e8:ADVERSARIAL_REVIEW.md`) and in the fix commits below.

| ID | Finding | Status | Commit |
|---|---|---|---|
| F-1 | Panicked file tasks exit `0` | fixed | `a3e5caf` |
| F-2 | Uncontained server paths | fixed | `5b9159e` |
| F-3 | Failed fetch refunds bytes already sent | fixed | `82a6d85` |
| F-4 | `--print-plan --audio` writes the scan file | fixed | `183f316` |
| F-5 | `--check-only --scan` calls a missing empty file clean | fixed | `778ae1d` |
| F-6 | Panicked scan tasks are dropped silently | fixed | `64c80d6` |
| F-7 | `--max-write-bytes` documented as `*_tmp`-only | fixed | `0b0aec8` |
| F-8 | Regression coverage gaps | fixed | `3546902` |
| F-9 | `kept_bytes` comment says the temp path keeps nothing | fixed | `64e32ac` |

**Verified correct (do not re-fix):**

- `layout`'s `offset + len` cannot overflow (non-negative `i64` → `u64`, sum at
  most `u64::MAX - 1`); the reverted `checked_add` there was dead defensive
  code. The overflow that mattered was `temp.rs`'s `i64` add (F-1).
- `reuse.rs` and `inplace.rs` already use `checked_add` for source-range ends;
  no change.
- Purge skips symlinks (`follow_links(false)`, regular files only); the reverted
  "metadata follows symlinks" claim was false.
- A corrupt `_tmp` cannot be promoted: `promote_tmp` checks whole-file length
  and MD5 before the rename and deletes on mismatch; resume-by-length may waste
  work, not corrupt.
- Non-scan `--check-only` catches absence (`actual_len != Some(size)`); F-5 was
  specific to the scan branch.
- The post-repair `--max-delete-bytes` re-check matches the docs (independent,
  can only shrink).
- `--plan-out` write failure → exit `3` is defensible (before mutation,
  write/verify class); untested, but not a bug to "fix".
- `build_plan` is pure and runs before `create_dir_all`; the ordering keeps
  refusal paths from leaving a directory behind.
- `plan_purge` on a missing `game_dir` is empty by design; do not add a
  `create_dir_all` before it.
- `--audio` under `--dry-run`/`--check-only` does not write; only
  `--print-plan` was missing from `readonly()` (F-4).
- The scan fallback estimate is a safe overestimate (a missing file is
  estimated in full).
- Real file failures already produced exit `3`; the panic path was the only
  hole (F-1).

### Second pass — N-1…N-14

| ID | Finding | Status | Commit |
|---|---|---|---|
| N-1 | A containment probe failing with anything but `NotFound` fell through to a lexical compare. | Fixed: `resolve_existing_prefix` propagates every other error. | `ddb4371` |
| N-2 | A settled download could cross `--max-download-bytes` and the run still exited `0`. | Fixed: `settle_reservation` trips `StoppedBy::Download`. | `0bcd6a6` |
| N-3 | The audio cache source was removed without a successful copy. | Fixed: source kept, refused moves/deletes warned, housekeeping best effort. | `dafed9b` |
| N-4 | The audio cache/res dirs were validated only in the post phase. | Fixed: validated at metadata, exit `2`. | `c999b5f` |
| N-5 | Plan dedup folded case on every host, merging distinct Unix files. | Fixed: case folding on Windows only. | `56a8a45` |
| N-6 | `./game/foo.dat` was not cleaned, so the purge keep set missed it. | Fixed: stored as the cleaned component list. | `df52195` |
| N-7 | Handles were awaited in spawn order, so a panic was seen late. | Fixed: `FuturesUnordered`, completion-order join. | `9d4e420` |
| N-8 | A containment failure logged as a size mismatch; the disk estimator dropped a file silently. | Fixed: both check branches log "escapes the game dir"; the estimator names the omitted file. | `f9a6967` |
| N-9 | `util::normalize_rel` was dead code with stale doc sites. | Removed; README, `lib.rs`, `docs/03`, `docs/04` and `filter.rs` name `contained_rel`/`contained_join`. | `2dfc1b5` |
| N-10 | The `kept_bytes` field doc described only in-place correctness. | Fixed with F-9: a resumed `_tmp` prefix counts, unverified until the MD5 gate. | `ce45a2f` |
| N-11 | Delete, copy and walk failures were swallowed. | Fixed: warned and counted where a counter exists. | `dafed9b` |
| N-12 | The file semaphore `unwrap` relied on an unnamed invariant. | Fixed: named `expect`; the join loop still counts a panic. | `9d4e420` |
| N-13 | Scan and plan estimates could wrap on server-controlled sizes. | Fixed: saturating adds, both branches unit-tested. | `682058e` |
| N-14 | Windows naming rules were not applied to manifest rels. | Fixed: `contained_rel` refuses trailing separators, device names, trailing dot/space and ADS colons on every host. | `f7449f5` |

Earlier findings re-validated: F-1/F-8 panic wiring is now exercised through `repair_all` (`d5ef6fa`); F-2's helper fails closed with N-1 (`ddb4371`); F-3/N-2 refuses the overrun (`0bcd6a6`); F-9/N-10 field docs (`ce45a2f`); Decision 7, `readonly()` is not the whole read-only gate (`d6e2d0c`).

### Third pass — N-15…N-20

| ID | Finding | Status | Commit |
|---|---|---|---|
| N-15 | The disk plan and document totals could wrap. | Fixed: `DiskPlan`, the short-vector fallback and the document totals saturate; `Val::UInt` renders unsigned. | `cbfaf4e` |
| N-16 | The `--plan-out` document re-derived `Σ compressed_size` and reconciled only upward. | Fixed: `PlanDocument::new` takes the per-file vector `PLAN` folds. | `d88c76a` |
| N-17 | The scanned estimate ignored the file's mode, under-counting a source the write overwrites. | Fixed: models discard/temp/in-place with `salvage_order`; a missing verdict still means the whole file. | `0b01501` |
| N-18 | A chunk with no source was verified at offset 0, faking reuse. | Fixed: ranges only for `Some(source)`; no-source chunks are `source_ok = false`. | `d5f84b3` |
| N-19 | A purge walk or candidate stat failure was dropped. | Fixed: non-`NotFound` errors warned and counted in `PurgePlan.errors`; a root `NotFound` stays the empty plan. | `408b239` |
| N-20 | Temp resume/debug stats and `rollback_tmp` substituted length `0` on error. | Fixed: `File::metadata()` errors propagate with the temp path as context. | `c04da2e` |

### Fourth pass — P-1…P-4

| ID | Finding | Status | Commit |
|---|---|---|---|
| P-1 | Under `--scan --repair-mode temp` the estimate took the pre-scan reuse rule and answered `0`. | Fixed: a scanned run always uses the scan's per-file vector. | `4d2b2d9` |
| P-2 | Five stat failures were still read as zero (temp resume/promote, in-place trim/salvage order, deprecated size). | Fixed: propagate or count honestly; a failed deprecated stat warns, deletes, and omits the size. | `edb2eb4`, `63a53de` |
| P-3 | `moved audio cache -> res` was logged unconditionally, claiming success for an empty or refused move. | Fixed: `info!(moved, kept, …)` only when a relocation was attempted. | `25ce46f` |
| P-4 | The emptied-directory sweep dropped `remove_dir`'s error. | Fixed: debug log with the directory and the `io::Error`; still best effort. | `d331174` |

### Fifth pass — R-1…R-6

| ID | Finding | Status | Commit |
|---|---|---|---|
| R-1 | `--plan-out` without `--print-plan` wrote nothing; the only write site sat in the print-plan branch. | Fixed: the document is written before execution; a failure is exit `3` with nothing mutated. | `867ed7f` |
| R-2 | Unreadable, absent and empty scan files were one state; `--audio none` fell back to `en-us` and, under `--purge-after`, deleted the real selection. | Fixed: `Ok(Some)`/`Ok(None)`/`Err`; an empty file means game-only; atomic write. | `a812fce` |
| R-3 | A failed rollback replaced the chunk error instead of annotating it. | Fixed: the rollback error is context; the chunk stays the cause. | `86f1fe3` |
| R-4 | `discard` mode's `remove_file(...).ok()` hid a refused delete. | Fixed: `discard_target` propagates every error except `NotFound`. | `646b9c9` |
| R-5 | `config.ini`'s `sync_all().ok()` published a rename after a failed sync. | Fixed: a failed sync aborts, and the temp is removed on the failed-write path. | `a812fce` |
| R-6 | The version/ignore-categories/blacklist readers treated every read error as absent. | Fixed: `Result<Option<T>>`; only `NotFound` is `None`, anything else is exit `1`. | `a812fce` |

### Sixth pass — R-7…R-9

| ID | Finding | Status | Commit |
|---|---|---|---|
| R-7 | `metadata::fetch` wrote the explicit-`--audio` scan file before the run's refusals. | Fixed: the write moved to execution, after every refusal; the repair uses in-memory `Meta::audio`. | `be6d83f` |
| R-8 | A scan file naming no recognized language was read as game-only, silently changing the purge keep set. | Fixed: one leading BOM is stripped, non-blank text with no known language is refused, a BOM-only file is refused; a mixed file keeps the known set and warns per unknown line. | `a44c666`, `16e4573` |
| R-9 | `--plan-out` help omitted `mode` and `extra_bytes`. | Fixed: help lists both; `docs/04` already had the full shape. | `c9fb86f` |

### Seventh pass — S-1…S-3

| ID | Finding | Status | Commit |
|---|---|---|---|
| S-1 | A scoped purge swept an **empty out-of-scope directory**: `mark_kept_ancestors` only marks directories reached from a kept file. | Valid, fixed: the walk adds only directories the scope admits to the sweep set; a hidden directory is kept and counted in `hidden` (so the scope warning names it). Reproduced with the binary before the fix (exit `0`, no warning, `mods/` and `mods/nested/` deleted) and confirmed after (dirs kept, warning, exit `0`). | `3c156c8` |
| S-2 | The staged-tier RAM payload used a non-saturating `.sum()` over server `uncompressed_size`. | Valid, fixed: `staged_payload` folds with `saturating_add`; three chunks at `i64::MAX` reach `u64::MAX` instead of wrapping to the stage-nothing sentinel (release) or panicking (debug). | `96b7ca9` |
| S-3 | `delete_deprecated`'s `is_file()` read every stat error as "not a file" and skipped the entry silently. | Valid, fixed: one `metadata` call is classified file / not-a-file / already-gone / failed; a failed stat warns with the path and deletes without counting an unknown size. | `e2040cd` |

### Eighth pass — T-1…T-2

| ID | Finding | Status | Commit |
|---|---|---|---|
| T-1 | An unparseable line in the launcher's `res_category`/blacklist JSON-lines was dropped by an `if let Ok`, so a BOM-prefixed or crash-truncated line silently ignored an explicit configuration; an all-malformed file degraded to the empty set. | Valid, fixed: one leading UTF-8 BOM is stripped; a mixed file warns per unparseable line with the path and 1-based line and keeps the well-formed entries; a file whose non-blank text produced no entry **and** had a parse failure is refused (exit `1`), matching R-8's rule for the audio scan file. A valid entryless `{}` and a blank file stay empty. Both readers share `parse_config_list`. Reproduced before the fix (BOM plus truncated line → `Ok(Some({}))`, all-malformed → `Ok(Some({}))`, no log) and pinned by two unit tests. | `ad0a09c` |
| T-2 | `util::file_len` mapped every `metadata` error to `None`, so an unreadable file was logged as missing by the temp path and reported as `CHECK fail: size` by `--check-only`. | Valid, fixed: `file_len` returns a three-state `FileLen` through `classify_len`; only `NotFound` is `Absent`, every other error is `Unreadable`. `check.rs` logs `CHECK fail: unreadable` with the file and the `io::Error` and still counts the file damaged; `temp.rs` warns with the path and error and still repairs. Pinned by an injected `PermissionDenied`, a real NUL path, and (Unix) a file-as-parent `NotADirectory`; Windows maps file-as-parent to `NotFound`, so that shape is not used for the unreadable arm there. | `baf6498` |

### Ninth pass — U-1…U-3

| ID | Finding | Status | Commit |
|---|---|---|---|
| U-1 | The delete/move/`--audio` gates scoped the raw server/config spelling while the filesystem sink cleaned it, so a `./`-spelled path was deleted or moved through an `--exclude`, and an in-scope `./`-spelled scan dir was refused with exit `1`. | Valid, fixed: `util::clean_rel` normalises the manifest rel, the deprecated list and the audio dirs once at their boundary; every gate and sink uses that string. Tests pin the excluded deprecated path, the excluded audio cache and the in-scope scan file. The audio-dir normalisation sits after the unsupported-download-mode check, so a run that is wrong in both ways still reports the documented usage error first (`27e7a18`). | `3123d96` |
| U-2 | A stale lock that could not be removed (an open handle without `FILE_SHARE_DELETE`, a read-only file, a delete-denying ACL) recursed in `Lock::acquire` until the stack overflowed: exit `-1073741571`, no `SUMMARY`. | Valid, fixed: one bounded takeover attempt. A failed removal is a contextual error naming the path and the I/O error; a successful removal is followed by one `create_new`, and a raced fresh lock is refused by name. The removal is injected so the arm is pinned on every host; the Windows sharing-violation probe now returns that error instead of aborting. | `d6bf9b3` |
| U-3 | README's module map and remaining module references, docs/03's crate layout and docs/04's module/touch-point tables still named the pre-split `src/repair/{metadata,manifest,check,post}.rs`, `src/repair/plan/...` and `src/repair/file.rs`. | Valid, fixed: `phases/…`, `purge.rs` and `file/mod.rs`; every written path was checked against `git ls-files`. | `1ff9e90` |

### Tenth pass — V-1…V-3

| ID | Finding | Status | Commit |
|---|---|---|---|
| V-1 | A correctly spelled blacklist entry dropped the file from the work list **and** from `expected_paths()`, so `--purge-before`/`--purge-after`/`--purge-only` deleted a file docs/01 says is never deleted. | Valid, fixed: the cleaned blacklist is a separate `plan_purge` keep input, so a blacklisted path survives every purge timing even after it has left the manifest. Reproduced with the fixture before the fix (the `--purge-after` case failed; the other two share the same `plan_purge`); unit and integration tests now pin all three flags while a stale file still purges. | `fe0934f` |
| V-2 | Blacklist entries were compared raw against the cleaned plan rel, so `./`- or `//`-spelled entries protected nothing (fail-open, decision 14). | Valid, fixed: `read_blacklist` cleans each entry through `util::clean_rel` at the reader; an entry that cannot be a contained rel is refused with the entry in the error rather than dropped. Unit test failed pre-fix. | `ec0a79d` |
| V-3 | The local reuse map was keyed on the raw local-manifest spelling while every consumer looked up the cleaned `PlannedFile::rel`, losing all reuse for a non-canonical manifest (and risking a false `--max-download-bytes` exit `5`). | Valid, fixed: the map key goes through `util::clean_rel`; an uncleannable entry is warned and skipped, matching the local-reuse map's best-effort contract. Unit test failed pre-fix. | `3ce91a3` |

### Eleventh pass — E-1…E-5

| ID | Finding | Status | Commit |
|---|---|---|---|
| E-1 | A parseable JSON line with a missing/wrong-typed required field (`{"fileName":123}`, `{"category":"10302","is_delete":"true"}`) was dropped silently, so an explicit blacklist/ignore entry protected nothing and an all-malformed file read as empty. | Valid, fixed: `parse_config_list` takes a `Result<Option<_>>` entry closure; a non-object line, a non-string `fileName`, a non-bool `is_delete`, or `is_delete: true` without a string `category` warns with the path and 1-based line and is kept out; a list whose non-blank text produced no entry and had a bad line is refused. `{}`, an absent/false `is_delete`, a blank file and a BOM-only file keep their old meanings. Pinned by `malformed_config_list_entries_are_refused_not_dropped`. | `6cb0cdf` |
| E-2 | `DiskPlan` modelled `--repair-mode discard` as in-place growth, so `est_extra_bytes`/`extra_bytes` reported `size - on_disk` where README/docs said 0. | Partially valid: the mismatch is real, but the growth figure is the correct one — discard deletes the target first (no second copy), yet a truncated target still grows to its manifest size; 0 is only the full-size case. Kept the code and corrected README/docs/02; pinned by `discard_needs_the_growth_but_never_a_second_copy`. | `56236a5`, `8a1618f` |
| E-3 | `DiskPlan` read a failed `metadata` as an absent file, overstating the estimate and logging nothing. | Valid, fixed: `planned_on_disk` uses `util::file_len`; only `NotFound` is zero, an unreadable stat warns with the path and I/O error and is charged as absent (the safe full-size estimate). Pinned with an injected `PermissionDenied`. | `56236a5` |
| E-4 | Purge expected/keep-set docs still omitted the V-1 blacklist keep; `keep_set` said "config.ini ONLY" while returning the lock too. | Valid, fixed: README/docs/01/docs/02/docs/03 add the blacklist (and the lock); `keep_set` names both and calls the blacklist a separate `plan_purge` input; rule 2 names both. | `8a1618f` |
| E-5 | `readonly()` said the metadata phase writes the audio-scan file; R-7 moved that write to execution. | Valid, fixed: metadata only validates the scan path under `!readonly() && !print_plan`; the write lives in execution, which `--print-plan` returns before. | `8a1618f` |

### Decisions that still bind

1. **Saturate, never wrap**, in every server-size estimate (N-13, N-15, N-19, S-2).
2. **Containment is a strict lexical gate**: fail closed on an unresolvable probe (N-1); refuse drive/UNC, `..`, trailing separators, Windows device names, trailing dot/space and ADS colons on every host (N-14); clean `./` rather than refuse it (N-6); fold case for plan dedup on Windows only (N-5); an outside path is exit `2` even if a scope filter would drop it (F-2).
3. **Downloads stop at settle time**: the overrun bound is the body already in flight; partial bodies charge `download_bytes_spent`, settled bodies charge `download_bytes` (F-3, N-2). A body past its reservation is refused at the piece that crosses it rather than streamed, stricter than capture invariant A6 (F-3).
4. **The scanned estimate models the write path** and the resolved mode; one per-file vector feeds both `PLAN` and `--plan-out` (N-17, N-16, P-1). The disk plan charges `discard` as the in-place growth: deleting the target removes the second copy, but a truncated target still has to grow to its manifest size, so 0 is the full-size case (E-2).
5. **Absent is `NotFound` only**; unreadable local state is exit `1` (R-2, R-6). An empty audio scan file is game-only; non-blank text with no known language and a BOM-only file are refused; one leading BOM is stripped (R-8). The launcher's ignore/blacklist JSON-lines follow the same split: one leading BOM is stripped, a mixed file warns per bad line with path and line — an unparseable line and a parseable line with a missing/wrong-typed required field are the same malformed-entry state — and a list whose non-blank text produced no entry and had a bad line is refused rather than read as empty (T-1, E-1). `--check-only --scan` checks presence and length with one `stat` per planned file, matching the non-scan path (F-5).
6. **The deferred audio write keeps the plan half mutation-free**: the scan file is written only in execution, after every refusal, so `--print-plan`/`--dry-run`/`--check-only` write nothing (R-7); `--plan-out` is a requested artifact, written before mutation, failure exit `3` (R-1).
7. **Publishing is crash-safe**: scan file via same-directory temp plus rename (R-2); `config.ini` syncs before its rename and removes the temp on a failed write (R-5).
8. **The post phase is best-effort housekeeping**: only the `config.ini` bump and the delete ceiling are fatal; a refused move/delete, an unreadable deprecated size, an unremovable directory and an unreadable purge subtree are warned, counted where possible, and left in place (N-3, N-11, P-2, P-4, N-19).
9. **A stat failure is never a zero and never "not a file"**: repair paths fail the file with path context; the post-phase delete warns with the entry and error and omits an unknown size, while a directory or an absent path stays a distinct, logged state (P-2, N-20, S-3). `util::file_len` keeps `Absent` (`NotFound`) distinct from `Unreadable`; `--check-only` logs the latter as `CHECK fail: unreadable`, not size, and the temp path warns with the path before repairing (T-2). The disk plan uses the same split: an unreadable stat warns with the path and I/O error and is charged as absent (full-size growth), never as a zero-length file (E-3).
10. **Log lines derive from outcomes**: relocation logs only when a move was attempted, with `moved`/`kept`; a containment failure logs as an escape (P-3, N-8).
11. **`kept_bytes` counts a resumed `_tmp` prefix**, unverified until the MD5 gate (F-9/N-10).
12. **File tasks join in completion order**, and the file-semaphore invariant is a named `expect` (N-7, N-12).
13. **The path scope bounds every deletion site, including the directory sweep**: a directory the scope hides is kept out of `PurgePlan::dirs` and counted in `hidden`, so an empty out-of-scope placeholder is not removed (S-1).
14. **A server/config spelling is normalised once, at its boundary**: `util::clean_rel` returns the `/`-separated string the gate and the sink share, so `./a/b` cannot pass a scope that hides `a/b` (U-1). The `blacklist` reader and the local reuse-map key go through the same clean (V-2, V-3); an uncleannable blacklist entry is refused by name, while the best-effort reuse map warns and skips it. A stale lock that cannot be removed is refused with the path and the I/O error, never retried; a successful takeover is one bounded attempt (U-2).
15. **The purge keep set is not the filtered repair plan alone**: the cleaned blacklist is a separate `plan_purge` keep input (V-1), so `--purge-before`/`--purge-after`/`--purge-only` never delete a path the blacklist excluded from repair, even after it has left the manifest (docs/01 "never deleted"). The scope remains rule 0, so it can still narrow both the plan and the cleanup deliberately. README, docs/01–03 and `keep_set`'s doc say the blacklist (and the single-writer lock) are part of the expected/kept set (E-4).

### Latest verification

Full run on Windows, debug profile, eleventh-pass final tree:

- `cargo build` — ok (10.28s).
- `cargo test -p girpr` — **349 passed, 0 failed, 7 ignored** (206 lib + 7 `artifact_server` + 17 `cli_contract` + 25 `repair_machinery` + 94 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0 `src/main.rs`, 0 doc-tests).
- `cargo clippy -p girpr --all-targets` — clean, no warnings (5.49s).
- `cargo fmt --check` — clean (exit `0`).

The eleventh-pass tests are `malformed_config_list_entries_are_refused_not_dropped` (both review shapes, a mixed file and an all-malformed file), `discard_needs_the_growth_but_never_a_second_copy` (a full-size and a truncated target) and `an_unreadable_stat_is_charged_as_the_whole_file_and_warned` (an injected `PermissionDenied`, with the real `NotFound` split covered by `file_len_separates_present_absent_and_unreadable`). E-4/E-5 are documentation/comments checked against the current call sites. The tenth-pass tests are the blacklist survival integration case (all three purge flags), the blacklist-reader cleaning/refusal unit case and the reuse-map key unit case; the first failed on the pre-fix tree and the last two failed before their fixes, so all three findings were executed rather than only source-read. The four ninth-pass tests are the three `./`-spelling cases and the undeletable-lock unit case.

**Not run:** the `#[cfg(unix)]` blocks of the containment, mode, `file_len` and file-as-parent tests (this host is Windows; the three path-spelling tests are host-independent); the 7 ignored tests; no release-profile run; no Linux/macOS host; no live CDN traffic (all tests use the in-process mock). V-3's reuse *hit* is unit-tested at the map key, not end-to-end (the mock serves one manifest spelling per build). U-2's real undeletable-file shape was exercised only with the Windows sharing-violation probe; the in-repo test injects the failed removal, so the arm is pinned on every host. S-1 was reproduced with the real binary over the mock. R-8's purge-deletion consequence remains a code-reading claim: the mock fixture has a single `game` manifest and no per-language package to delete. T-1/E-1 are pinned at the unit reader, not through the binary. T-2's root cause was reproduced with a real non-`NotFound` stat error, but the full `check.rs` log path is pinned at the classifier and syscall boundary, not through a deny-ACL fixture; E-3's disk-plan arm is pinned through an injected `FileLen::Unreadable`, not a real deny ACL. E-2's discard figures are pinned at `DiskPlan`, not end-to-end through `PLAN`/`--plan-out`.

*End of review. The pass log records the first pass, the decisions that still bind and the latest verification, and the commits it cites are the source, test and doc changes. The open items are in [`OPEN_FOR_MAINTAINERS.md`](OPEN_FOR_MAINTAINERS.md).*
