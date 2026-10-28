# Fourth adversarial pass — findings

Reviewed tree: `fix-gaps/girpr` at `8fdfc55` (`docs(review): fold the
third-pass verdicts into the review`), baseline `main`.
Host: Windows, debug profile.

Scope: the third-pass fixes N-15…N-20 (`cbfaf4e`, `d88c76a`, `0b01501`,
`d5f84b3`, `408b239`, `c04da2e`, plus `5560e0d`) and the places they touch,
then a fresh sweep of the same classes (swallowed errors, unvalidated server
data, ceilings enforced on a number that is not the cost, logs that claim an
outcome the code did not reach).

Four items are actionable. One contradicts a third-pass claim, one is a
contract gap in the scanned estimate that the last pass only half closed, and
two are the residual "stat failure becomes 0 / failure is not logged" class.
Nothing found is data-losing or corrupting; the severities reflect that.

## 1. Verdict table

| ID | Severity | Class | Site | Status |
|---|---|---|---|---|
| P-1 | **Medium** | bug — estimate/contract (under-estimate), test gap | `src/repair/mod.rs:273` (with `src/repair/scan.rs:156-165`) | **Fixed** `4d2b2d9` |
| P-2 | **Low** | bug — swallowed errors (residual N-20 class) | `src/repair/file/temp.rs:220`, `src/repair/file/temp.rs:488`, `src/repair/file/inplace.rs:182`, `src/repair/file/inplace.rs:271`, `src/repair/phases/post.rs:163` | **Fixed** `edb2eb4`, `63a53de` |
| P-3 | **Low** | bug — log claims an outcome that did not happen | `src/repair/phases/post.rs:318` | **Fixed** `25ce46f` |
| P-4 | **Low** | bug — swallowed error in a delete sink | `src/repair/purge.rs:258` | **Fixed** `d331174` |

## 2. Findings

### P-1 — Medium — `--scan --repair-mode temp` still reports the *unscanned* estimate, so `PLAN est_download_bytes` can be 0 for bytes the same run downloads

`src/repair/mod.rs:270-276`:

```rust
let est_download_by_file: Vec<u64> = match (&scan, ctx.repair_mode) {
    // The temp path writes every chunk of a file it repairs, so its
    // figure is the reuse map's one.
    (Some(_), crate::config::RepairMode::Temp) => plan.est_download_bytes_by_file(&local_map),
    (Some(s), _) => s.est_download_bytes_by_file(&plan, &game_dir, ctx.repair_mode, &local_map),
    (None, _) => plan.est_download_bytes_by_file(&local_map),
};
```

The first arm ignores the scan it was handed. `plan.est_download_bytes_by_file`
is the pre-scan rule (`src/repair/phases/plan.rs:166-180`): a chunk is free when
a `(md5, size)` entry exists in the local reuse map, with no check that the
bytes at that offset are actually that chunk. The scan arm below it is the one
that knows: `ScanReport::est_download_bytes_by_file` resolves the mode and,
for temp, counts exactly the chunks whose *verified* source is not intact
(`src/repair/scan.rs:156-165`):

```rust
// The temp path writes every chunk; only the reuse sources save a
// download, and `source_ok` is exactly the scan's verdict on them.
Mode::Temp => chunks.iter().zip(v.source_ok.iter())
    .filter(|(_, ok)| !**ok)
```

So under `--scan --repair-mode temp` the figure falls back to "a candidate
entry exists", which is the one thing the scan was bought to check.

**Failure scenario (reachable with an existing fixture).** Take
`tests/repair_offline.rs:337-371` `verified_local_slice_is_reused_without_redownload`
unchanged: `config.ini` names the installed build, so the local reuse map has a
candidate entry for **both** of `game/foo.dat`'s chunks, and chunk 1's 64 bytes
are flipped on disk. That test proves the writer ignores the corrupt candidate
and fetches `chunk_foo1` (`:364-368`): the temp path's `try_reuse_slice`
(`src/repair/file/temp.rs:243-275`) re-hashes the candidate slice and answers
`ChunkOutcome::Download` on a mismatch or an unreadable slice
(`temp.rs:272-275`). The map rule never looks: an entry exists, so the chunk is
free (`plan.rs:166-174`). The same is true of a *missing* or *truncated* target:
the map is built from the manifest (`src/sophon.rs:661-683`), not from the
disk, so its entries still match and every chunk still downloads.

The numbers the same run then prints:

- `--scan --repair-mode temp --print-plan` → `PLAN est_download_bytes=0`
  for `game/foo.dat` (both candidates exist), and the same `0` per file in
  `--plan-out`;
- `--scan --repair-mode temp` → `SUMMARY download_bytes` = the compressed size
  of `chunk_foo1`, non-zero — the same chunk the existing test sees fetched
  (`tests/repair_offline.rs:368`).

A pre-flight `--max-download-bytes` compares against the `0`, so the ceiling
that exists to refuse before the work is admitted on a figure the run then
exceeds. The runtime hard stop still bounds the spend, which is why this is
Medium and not High.

**Why it fails the standard.** N-17's recorded verdict is that "the scanned
estimate resolves the file's mode and models discard, temp and in-place with
the writer's `salvage_order`" (`ADVERSARIAL_REVIEW.md` §9.1). That is true of
the resolved mode, but not of the *requested* one: the `(Some(_), Temp)` arm
predates N-17 and still answers with the pre-scan rule, which makes
`est_file_download_bytes`'s `Mode::Temp` arm unreachable in exactly the spelling
that names temp. The comment it carries ("its figure is the reuse map's one")
restates the rule N-17 replaced; `mod.rs:261-265` and `scan.rs:156-165` now
contradict each other about which temp figure is the honest one. And the
`--scan` contract (`scan.rs:1-16`: "the download estimate can be the figure the
repair will really spend, instead of an upper bound") is not met.

**Fix.** Delete the `(Some(_), RepairMode::Temp)` arm — `est_file_download_bytes`
already handles `Resolved == Temp`, and `resolve_mode` returns `Temp` for
`requested == Temp` — or make the arm call
`s.est_download_bytes_by_file(&plan, &game_dir, ctx.repair_mode, &local_map)`.
One line, one behaviour.

**Test gap.** No test runs `--scan` with `--repair-mode temp`; the estimate
tests all use the default (resolved) mode. The regression test is the on-disk
shape above (installed-version `config.ini`, one chunk corrupted) with
`--repair-mode temp` added to the `--print-plan --scan` invocation and to the
repair invocation, asserting `PLAN est_download_bytes == SUMMARY
download_bytes` — the same assertion `repair_offline.rs:1196-1291` makes for the
default mode.

### P-2 — Low — N-20's "no more length 0" rule is still broken at five sites in the same two paths

`c04da2e` fixed three stats in `temp.rs` (`:96-99`, `:104-111`, `:174-177`) and
justified the change in the strongest terms (`temp.rs:170-173`): "Treating it
as 0 would ... with nothing logged." Five of the same idiom remain:

| Site | Code | What a stat failure becomes | Consequence |
|---|---|---|---|
| `src/repair/file/temp.rs:220` | `let cur_len = out.metadata().map(\|m\| m.len()).unwrap_or(0);` | "nothing in the `_tmp`" | the chunk is re-copied/re-downloaded; safe direction, silent extra work |
| `src/repair/file/temp.rs:488` | `let len = util::file_len(tmp_path).unwrap_or(0);` | "the tmp is empty" | reports `tmp length 0 != expected N` for a tmp that is not empty; the real `io::Error` is discarded by `util::file_len` (`src/util.rs:268-270`) |
| `src/repair/file/inplace.rs:182` | `let have = handle.metadata().map(\|m\| m.len()).unwrap_or(0);` | "no tail" | the eager trim is skipped silently |
| `src/repair/file/inplace.rs:271` | `let usable = handle.metadata().map(\|m\| m.len()).unwrap_or(0);` | "the file is 0 bytes" | `salvage_order` marks **no** source live; every bad chunk is downloaded; no log line says why |
| `src/repair/phases/post.rs:163` | `let sz = util::file_len(&p).unwrap_or(0);` | "0 bytes" | `deleted_extra_bytes` under-reports, and a dry run prints `would delete deprecated X (0 bytes)` for a non-empty file |

`temp.rs:220` is the sharpest: it is the fourth stat of the already-open `_tmp`
handle in the very attempt loop `c04da2e` edited, one call below the propagated
ones. Its own comment at `temp.rs:221-226` documents the resume rule but not
the failure mode, and `temp.rs:167-173` argues the opposite policy three
functions earlier.

**Why it fails the standard.** "No omitted errors, fail closed" — the run hides
an I/O failure on a handle it already holds, and the operator sees a misleading
number or an unexplained bandwidth spike. `promote_tmp`'s message at `:490`
(`tmp length 0 != expected N`) is actively wrong: the tmp was not empty, and a
retry will hit the same error with no cause attached.

**Fix.** Same shape as `c04da2e`: `f.metadata().with_context(|| format!("stat
tmp {}", ...))?` / `std::fs::metadata(tmp_path).with_context(...)?.len()`. For
`post.rs:163`, a `match` that logs and skips the counter add on failure (the
file stays deleted, the number stays honest) is enough; do not turn a housekeeping
stat into a fatal error.

### P-3 — Low — the audio relocation logs success unconditionally

`src/repair/phases/post.rs:318`:

```rust
tracing::info!("moved audio cache -> res");
```

This is the last statement of `move_audio_cache`, reached on every non-dry-run
path that got past `:235`, including:

- an empty `audio_pkg_cache_dir` (`files` empty at `:238-254`) — nothing was
  moved;
- every move refused: `:277-284` (destination dir cannot be created) and
  `:302-313` ("audio cache move failed; source kept") `continue`/warn and then
  fall through to the unconditional line;
- a partial move: some files copied and their sources left behind because
  `remove_file` failed (`:294-300`).

The per-file warnings exist and are correct; the one line that summarises the
relocation is the one that can lie, and it is the line an operator greps for.
`move_audio_cache` returns `()`, so nothing downstream can correct it.

**Fix.** Count `moved` / `kept` and log them (`tracing::info!(moved, kept,
"audio cache -> res")`), or emit the info only when `moved > 0`. A count also
makes the "copied but source not removed" case visible in one place.

### P-4 — Low — the emptied-directory sweep discards its error with no log

`src/repair/purge.rs:256-260`:

```rust
fn remove_emptied_dirs(dirs: &[PathBuf]) {
    for d in dirs {
        let _ = std::fs::remove_dir(d);
    }
}
```

The doc comment covers the *expected* failure ("still fail harmlessly if
something appeared since the plan was built"), but `remove_dir` has other
failure modes — a sharing violation, an ACL, a read-only parent — and those are
indistinguishable from the expected one after `let _`. Every other delete in
this module is warned: `apply_purge`'s `remove_file` (`:229-238`), the walk
(`:111-119`) and the candidate `metadata()` (`:138-149`), which is what N-11 and
N-19 established. The sweep is the remaining silent one, and the directory it
failed to remove is state the plan said it would remove.

**Fix.** `if let Err(e) = std::fs::remove_dir(d) { tracing::debug!(dir =
%d.display(), error = %e, "the emptied dir was not removed"); }` — debug level
is enough; the point is that the reason is recoverable.

## 3. Checked and found sound (this pass)

Not re-litigated, but re-read at the current lines because the third pass
touched them:

- `est_download_bytes_by_file` ↔ `PLAN` ↔ `--plan-out`: one vector, one fold
  (`mod.rs:270-279`, `report.rs:253-304`); N-16 holds.
- N-17's ordered-tier model vs `inplace.rs:263-282`: same `bad` list, same
  `source_ok` filter, same `salvage_order`; `usable_len = u64::MAX` cannot
  over-credit a source because `source_ok` implies the range was read inside
  the file (`scan.rs:175-176`). The fixture test
  (`repair_offline.rs:876-944`) does discriminate: the pre-N-17 rule would
  estimate one chunk, the tree downloads two.
- N-18's source-owner mapping (`scan.rs:304-326`): a chunk with no source is
  `false`, never verified against offset 0.
- N-19's counted walk (`purge.rs:97-160`): `NotFound` is the empty plan, every
  other walk/stat error is warned and counted; `total_bytes` saturates.
- `join_file_tasks` counts every join error; `settle_reservation` stops on the
  settled figure (`temp.rs:453-472`); the `--print-plan` path returns before
  `create_dir_all`, the lock and the purge (`mod.rs:313-340`).
- `contained_rel` / `contained_join` / `resolve_existing_prefix`: drive prefix,
  root, UNC, `..`, device names, trailing dot/space and ADS refused; non-
  `NotFound` probes fail closed (`util.rs:272-420`).

Already recorded as open, so not counted as findings here: `PurgePlan::errors`
is diagnostic only and the delete ceilings cannot see candidates under an
unreadable directory (`ADVERSARIAL_REVIEW.md` §9.4, first bullet);
`inplace.rs:182`/`report.rs:92` conflating "missing" with "unreadable" only
over-approximates; `DiskPlan::for_file`'s `inplace/0` default is unreachable
after `build_plan` (the escaping rel is refused there), and N-8 named it.

## 4. Verification

Run on this tree, in this order, from `E:\hoccode\hocrust\girpr`:

| Command | Result |
|---|---|
| `cargo build` | ok, debug profile |
| `cargo test -p girpr` | **306 passed, 0 failed, 7 ignored** — 174 lib + 0 `src/main.rs` + 7 `artifact_server` + 16 `cli_contract` + 25 `repair_machinery` + 84 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap` |
| `cargo clippy -p girpr --all-targets` | clean, no warnings (`Finished dev profile ... in 18.06s`) |
| `cargo fmt --check` | clean, exit `0` |

The suite is green **with all four findings present**, which is the point: none
of them is caught today. P-1's check exists only in the default mode
(`repair_offline.rs:1196-1291`, and the reuse half at `:337-371`); P-2/P-3/P-4
have no seam (`temp.rs:488`, `inplace.rs:271` and the sweep need an injected
I/O failure; `post.rs:318` is only asserted positively, in the moves that
succeed).

**Not verified (no deterministic seam, or out of environment):**

- P-1's exact two-spelling run (`--scan --repair-mode temp` plan vs repair):
  argued from `mod.rs:273`, `scan.rs:156-165`, `temp.rs:243-275` and the
  existing default-mode tests (`repair_offline.rs:337-371`, `:1196-1291`); no
  new run was made, because adding the test would modify the tree under review.
- P-2's stat failures: as §9.3 already recorded for N-20, `metadata()` on an
  open handle cannot be made to fail through a public entry point on Windows.
- P-3/P-4: the failing branches (copy/rename refused, `remove_dir` refused)
  have no injection seam on a normal Windows account.
- The `#[cfg(unix)]` blocks of
  `contained_join_resolves_a_symlink_and_refuses_the_escape`,
  `contained_join_fails_closed_when_a_component_cannot_be_resolved` and
  `the_mode_is_part_of_the_file_stats`; the 7 ignored tests; no release-profile
  run; no Linux/macOS run; no live CDN traffic.

## 5. Progress — validation and fixes (this tree)

Tree `fix-gaps/girpr`, started at `b99ddbb`, closed at `d331174`. Every finding
was re-read at the cited lines and, where a real seam exists, executed.

### 5.1 P-1 — **Valid, reachable; fixed `4d2b2d9`**

- Re-read: `src/repair/mod.rs:270-276` took the `(Some(_), RepairMode::Temp)`
  arm whenever `--scan` and a requested temp met, so the scan was ignored.
  `resolve_mode` (`src/repair/file/mod.rs:548-551`) returns `Mode::Temp` for a
  requested `Temp`, which made `scan.rs:159-165` (`Mode::Temp`) unreachable in
  the one spelling that names temp.
- **Executed on the unfixed tree**: the new binary-level test failed with
  `PLAN files=2 est_download_bytes=0 ... scan_bytes=176`, and the same
  fixture's repair fetched `chunk_foo1` — exactly the note's scenario.
- Fix: the temp arm is gone; `mod.rs:268-272` now branches on `scan` alone, so
  a scanned run always asks `ScanReport::est_download_bytes_by_file`, which
  resolves the requested mode. The comment at `mod.rs:261-266` no longer
  restates the pre-N-17 temp rule.
- Regression test: `tests/repair_offline.rs:1293-1385`
  `the_scanned_temp_estimate_is_the_bytes_the_repair_really_downloads`
  (installed-version `config.ini`, chunk 1 corrupted, `--scan --repair-mode
  temp --print-plan --plan-out`, then the same mode for the repair). Asserts
  `est_download_bytes > 0`, that the document's per-file halves sum to the
  total, `PLAN est_download_bytes == SUMMARY download_bytes`, and that only
  `chunk_foo1` was fetched. Fails on the old arm at the first assertion.

### 5.2 P-2 — **Valid at all five sites; fixed `edb2eb4` / `63a53de`**

- `temp.rs:221` (resume stat) and `temp.rs:492` (`promote_tmp`) propagate
  `metadata(...)` with the `_tmp` path as context; `try_reuse_slice` takes the
  path so the message can name it.
- `inplace.rs:184` (eager trim) and `inplace.rs:276` (salvage order) propagate
  with the target path. The second is the sharp one: a zero marked no source
  live, downloading every bad chunk with no explanation.
- `post.rs:167-181` keeps the size as `Option<u64>`: a stat failure is warned
  with its `io::Error`, the delete still runs, the counter omits the unknown
  size instead of adding a false zero, and the dry-run line says "size
  unavailable" rather than "0 bytes".
- Regression test: `src/repair/file/temp.rs:544`
  `a_tmp_that_cannot_be_statted_is_reported_as_a_stat_failure` stats a `_tmp`
  under a regular file (`ENOTDIR`) and requires the error chain to name the
  stat and the path. On the old `util::file_len(...).unwrap_or(0)` it fails
  with `tmp length 0 != expected 64`.
- **No test** for the two `File::metadata()` sites (`temp.rs:221`,
  `inplace.rs:184`/`:276`): the handle is already open, so an `fstat` failure
  has no public seam on this host — the same limitation §9.3 records for
  N-20. The `post.rs` site needs `metadata` to fail after `is_file()`
  succeeded (a race on a static tree), so it has none either.
- **No shared crate changed.**

### 5.3 P-3 — **Valid; fixed `25ce46f`**

- `post.rs:354-356`: the summary is now
  `tracing::info!(moved, kept, "audio cache -> res")`, emitted only when a
  relocation was attempted. `moved` counts sources gone from the cache;
  `kept` counts refused moves and copies whose source could not be removed.
  An empty cache and every-refused move no longer claim success.
- Tests (`post.rs:391`, `:430`, `:463`, using the shared capture helper in
  `src/repair/test_support.rs`): the refused copy asserts `moved=0 kept=1`,
  the completed move `moved=1 kept=0`, and an empty cache emits no relocation
  line. The refused-copy assertion fails on the old bare line.

### 5.4 P-4 — **Valid; fixed `d331174`**

- `purge.rs:258-270`: `remove_dir`'s error is a `debug!` naming the directory
  and the `io::Error`; the sweep stays best effort (nothing escalated). The
  doc comment now says the reason is logged.
- Test `src/repair/purge.rs:753`
  `a_dir_the_sweep_could_not_remove_is_logged_with_the_reason` makes the
  failure real (a file appears in the directory after the plan) and asserts
  the captured log names both. It sees nothing on the old `let _ =`.

### 5.5 Verification after the fixes

| Command | Result |
|---|---|
| `cargo build` | ok, debug profile |
| `cargo test -p girpr` | **311 passed, 0 failed, 7 ignored** — 178 lib + 0 `src/main.rs` + 7 `artifact_server` + 16 `cli_contract` + 25 `repair_machinery` + 85 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap` |
| `cargo clippy -p girpr --all-targets` | clean, 0 warnings |
| `cargo fmt --check` | clean, exit `0` |

The §4 table records the pre-fix baseline (306 passed); the five new tests are
the difference. Not run, unchanged from §4: the `#[cfg(unix)]` blocks, the 7
ignored tests, no release profile, no Linux/macOS host, no live CDN.

*End of fourth-pass note.*
