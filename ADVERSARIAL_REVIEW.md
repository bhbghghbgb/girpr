# ADVERSARIAL REVIEW — `girpr` (root crate)

**Reviewed tree:** branch `all/fix-gaps`, HEAD `64dc69d` (`revert: drop the
adversarial review series (tree back to 7f2d69)`).
**Baseline:** `7f2d69c`. `git diff --stat 7f2d69c HEAD` is empty, so this tree is
byte-identical to the pre-fix baseline. The prior review/fix series
`0e4f011..f9352f7` was reverted by `64dc69d`; its review at
`f9352f7:ADVERSARIAL_REVIEW.md` is historical input, not evidence.

This document is a fresh source-level review of the current tree. It does not
assume any sentence of the reverted review is true. The fixes landed on
`fix-gaps/girpr`; §7 is the first-pass implementation table and §8 is the pass
log that carries every later finding to its final verdict.

---

## 1. Scope & method

- **Scope:** the root `girpr` crate — `src/**`, `tests/**`, `README.md`,
  `docs/04-scope-limitation-spec.md`, `Cargo.toml`. The sibling crates
  (`crates/log`, `crates/tempdir`) and the two tools (`tools/capture`,
  `tools/sync`) were not reviewed for this report.
- **Method:** read the current source and tests; use `git show`, `git diff`,
  `git grep`, and file reads to establish what the current tree actually does.
  Every finding cites a current-tree `file:line`.
- **No tests were run.** No `cargo build`, `cargo test`, `cargo nextest`,
  `cargo clippy`, or binary execution was performed. There are no test results
  in this report; statements about behavior are code-path analysis. Where a
  claim depends on runtime behavior that was not exercised, it is marked
  **not verified**.
- **Severity scale:**
  - **High** — silent wrong result, or a write/delete outside the install, or
    an exit code that tells the caller the install is healthy when it is not.
  - **Medium** — a documented contract or enforcement ceiling is violated, but
    the damage is bounded or requires a narrower precondition.
  - **Low** — diagnostics, docs, or coverage; no wrong repair result by itself.
- **Classification vocabulary:** `bug`, `doc error`, `dead code`, `test gap`,
  `intentional-but-uncovered`.
- **Source-verified** below means the cited lines exist and the control/data
  path was read end to end. It does not mean the scenario was executed.

---

## 2. Findings

### 2.1 Index

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| F-1 | High | bug | `src/repair/file/mod.rs:292-294` | Per-file task panics are discarded; the run can exit `0` and bump `config.ini` with an unrepaired file. |
| F-2 | High | bug | `src/repair/phases/plan.rs:187`; `src/repair/phases/post.rs:142`, `:185-186` | Server-controlled path strings are joined without containment: manifest rels can write outside `game_dir`; deprecated names can delete outside it; audio cache/res dirs can move files outside it. |
| F-3 | Medium | bug | `src/repair/file/temp.rs:354-372`; `src/sophon.rs:385-399`, `:500` | A failed chunk fetch refunds the whole download reservation, although up to 5 partial bodies per call (25 per chunk with file retries) may have crossed the wire; the ceiling is under-enforced. |
| F-4 | Medium | bug | `src/repair/phases/metadata.rs:136-153` | `--print-plan` plus an explicit `--audio` writes the scan file and creates `game_dir`, contradicting the documented read-only contract. |
| F-5 | Medium | bug | `src/repair/phases/check.rs:38-41` | `--check-only --scan` compares only the whole-file MD5, not length/presence, so an absent zero-length file can be reported clean while the non-scan path reports damage. |
| F-6 | Low | bug | `src/repair/scan.rs:132` | A panicked scan task is dropped with no warning; `PLAN` scan bytes/damaged counts silently undercount. |
| F-7 | Low | doc error | `src/cli.rs:89-95`; `src/repair/scope/limits.rs:20`, `:55`; `src/report.rs:390`; `src/repair/scope/ledger.rs:394-395`; `docs/04-scope-limitation-spec.md:168`, `:291` | `--max-write-bytes` is documented as bytes written into `*_tmp`; the default in-place path writes the target itself. |
| F-8 | Medium | test gap | `tests/repair_offline.rs:336-365`, `:2813-2866`, `:1757-1799`; `tests/cli_contract.rs:216-252` | No regression tests for panic→exit code, drive-relative/deprecated traversal, partial-body refund, full-length corrupt `_tmp`, `--print-plan --audio`, or `--check-only --scan` zero-length absence. |
| F-9 | Low | doc error | `src/repair/file/mod.rs:261-264` | The comment says the temp path “rewrites the whole file and keeps nothing”, but resume-by-length keeps a `_tmp` prefix and `kept_bytes` counts it. |

---

### F-1 — High — Per-file task panics are silently swallowed

**Location:** `src/repair/file/mod.rs:292-294`

```rust
for h in handles {
    let _ = h.await;
}
```

**What the code does.** Each file task is spawned at `src/repair/file/mod.rs:211`
and its body returns `()` (the async block ends at `:289-290`). Therefore
`h.await` has type `Result<(), JoinError>`; an `Err` can only be a panic or a
cancellation, never an `anyhow::Error`. `let _ =` drops that `Err` without a log
and without touching `summary.files_failed`, which is only incremented for a
normal `Err(e)` returned by `repair_one_file` at `:275-276`.

**Why it matters.** `files_failed` is the only gate for exit `3`:

```rust
if summary.files_failed.load(Ordering::Relaxed) > 0 {
    return Ok((summary, 3));
}
```

`src/repair/mod.rs:480-482`. With `files_failed == 0`, the run reaches the
post-phase (`src/repair/mod.rs:487-491`) and `post::run` writes `config.ini`
(`src/repair/phases/post.rs:96-105`). The result is the exact launcher-desync
failure the post-phase ordering exists to prevent: a planned file was never
repaired, the version was advanced, and the process exits `0`.

**Concrete panic trigger.** `src/repair/file/temp.rs:193`:

```rust
let end = (c.offset + c.uncompressed_size) as u64;
```

`c.offset` and `c.uncompressed_size` are `i64` fields deserialized from the
manifest (`src/hyp.rs:154-156`, parsed by `num_from_string` at `:120-145`).
With e.g. `offset = i64::MAX` and `uncompressed_size = 16`, the `i64` addition
overflows: debug builds (the normal test/CI profile; `Cargo.toml` has no
`[profile.*] panic = "abort"`) panic, release builds wrap. A panic kills the
task, and the `let _ = h.await` above converts it into exit `0`. This is
source-verified as an unchecked expression on manifest-controlled values; the
panic itself was **not executed** in this review.

**Verification:** source-verified control flow and trigger expression; the
panic/exit-0 scenario is **not verified** by a test (none exists — see F-8).

**Classification/severity:** `bug`, High.

**Concrete fix.**
1. Count every join error, including panics, as a file failure and log it:

```rust
match h.await {
    Ok(()) => {}
    Err(e) => {
        summary.files_failed.fetch_add(1, Ordering::Relaxed);
        tracing::error!(error = %e, "repair task failed");
        if summary.fail_fast() {
            summary.abort();
        }
    }
}
```

   This makes the run exit `3` and skips the post-phase/config bump. (Do not
   use the reverted `Err(e) if e.is_panic() => {}` arm: it is precisely the
   silent path.)
2. Remove the unchecked arithmetic in `temp.rs:193` with
   `u64::try_from` + `checked_add`, so a corrupt manifest fails the file
   normally instead of panicking the task. The reverted commit `474f3f9` did
   this part correctly and it should be re-landed (see §4).
3. Add the regression test described in F-8 (huge offset → exit `3`,
   `files_failed=1`, no `config.ini`).

---

### F-2 — High — Server-controlled paths are joined without containment

**Locations:**
- Manifest rel: `src/repair/phases/plan.rs:187`
  ```rust
  let rel = f.file.replace('\\', "/");
  ```
  There is no validation of `..`, absolute paths, drive prefixes, or UNC paths
  in the current tree. `build_plan` returns a `RepairPlan`, not a `Result`
  (`src/repair/phases/plan.rs:172-177`, `:237-242`), so it cannot refuse.
- Repair target: `src/repair/file/mod.rs:329-334` joins
  `game_dir.join(util::normalize_rel(&file.rel))` and creates the parent.
- Temp reuse source and promote target: `src/repair/file/temp.rs:217`,
  `:434-435`.
- Scan read: `src/repair/scan.rs:148`.
- Deprecated-file delete: `src/repair/phases/post.rs:142-149` joins the
  server's deprecated name and calls `remove_file`.
- Audio cache move: `src/repair/phases/post.rs:185-186` joins the server's
  `audio_pkg_cache_dir`/`audio_pkg_res_dir`; `:208-222` walks the cache and
  renames/copies/removes into the destination.
- Audio scan read/write: `src/repair/audio.rs:66`, `:95-99`;
  `src/repair/phases/metadata.rs:123-126`, `:151`.
- `util::normalize_rel` only swaps separators (`src/util.rs:277-279`); it is
  not a containment check.
- `PathScope` is not a containment check: with the default empty scope,
  `is_visible` returns `true` for every string (`src/repair/scope/filter.rs:79-85`).

**Why it matters.** A hostile or corrupted server response can make the tool
operate outside the install:

| Input | Result of `game_dir.join(normalize_rel(input))` | Sink |
|---|---|---|
| `../outside.dat` | `game_dir/../outside.dat` (resolves outside) | scan read, repair write/promote |
| `/abs/evil.dat` | `E:\abs\evil.dat` (root replaces the base on Windows) | same |
| `C:/evil.dat` | `C:\evil.dat` (absolute, replaces the base) | same |
| `C:evil.dat` | drive-relative; `PathBuf::push` replaces the base for a prefixed path | same |
| `\\server\share\evil` | UNC absolute, replaces the base | same |

The escape itself follows `std::path::PathBuf::push` semantics; it was **not
executed** in this review, but the absence of any validation is
source-verified. The manifest is fetched over the network and this crate does
not authenticate it beyond a checksum supplied by the same API; a compromised
CDN, a hostile `--hyp-base`, or a corrupt mock is enough. The deprecated and
audio sinks are worse than the repair sink because they can delete or move
files, not just write them.

**Verification:** source-verified that every cited join has no containment
check. Real manifests are believed to contain only relative `game/...` paths;
that was **not verified** against the live CDN.

**Classification/severity:** `bug`, High (integrity; requires hostile/corrupt
server data).

**Concrete fix.**
1. Add one helper, e.g. `fn contained_rel(rel: &str) -> Result<PathBuf>`:
   - normalize `\` to `/`;
   - reject empty, leading `/`, `Path::has_root()`, and any
     `Component::Prefix(_)` or `Component::ParentDir` (the prefix check is what
     catches `C:evil`, which `is_absolute()` does **not** catch);
   - join, then verify the result is under `game_dir` after resolving the
     nearest existing ancestor (lexical `starts_with` alone is not enough for
     `..`).
2. Apply it at every server-path boundary: `build_plan` (manifest rels),
   `delete_deprecated` (deprecated names), `move_audio_cache`
   (cache/res dirs), `metadata::fetch` (`res_category_dir`,
   `blacklist_dir`, `audio_pkg_scan_dir`), and `audio::read_scan_file` /
   `write_scan_file`.
3. Fail the plan with exit `2` (metadata/server data) before any mutation for
   manifest rels and config dirs. For the deprecated/audio post-phase lists,
   validate them in the plan phase too; if that is not possible, skip the
   invalid entry with a warning rather than deleting/moving outside.
4. Tests: `C:evil`, `C:/evil`, `../x`, `/abs`, `\\server\share`, a legitimate
   `a/b.dat`, plus a deprecated `../x` and an audio cache dir of `..`.

---

### F-3 — Medium — Failed fetches refund bytes that may already have crossed the wire

**Locations:** `src/repair/file/temp.rs:342-377`; `src/sophon.rs:385-399`,
`:500`; `src/repair/file/mod.rs:86`, `:375-419`;
`src/repair/scope/ledger.rs:214-222`, `:301-326`.

**What the code does.** Before a chunk fetch, `write_downloaded_chunk` reserves
the manifest's `compressed_size` (`temp.rs:342-351`). On any error it refunds
the **whole** reservation:

```rust
Err(e) => {
    ctx.summary.adjust_download_bytes(-(reserved as i64));
    return Err(e).with_context(|| format!("chunk {}", c.id));
}
```

`temp.rs:369-372`. The comment at `:354-357` says “A failed fetch delivers
nothing, so the reservation goes straight back.” That is false for a
mid-stream transport failure: `stream_body` writes decoded pieces to the sink
as they arrive (`src/sophon.rs:476-490`) and can return `Transport` after some
pieces. The compressed byte count accumulated at `:481` is discarded on error.

**Bound.** `stream_chunk_to` retries `ATTEMPTS = 5` times
(`src/sophon.rs:385-399`, `:500`); each retry calls `sink.restart()` and can
deliver another partial body. A file is attempted up to `MAX_ATTEMPTS = 5`
times (`src/repair/file/mod.rs:86`, `:365-369`, `:375-419`). So a single chunk
can have up to **5 partial bodies per `stream_chunk_to` call**, and up to
**25 partial bodies per chunk per run** if the same chunk keeps failing. Each
one is charged to the write live counter per piece (`temp.rs:285-299`) but not
to the download live counter. The reverted review's “at most one partial chunk”
was also wrong; see §4.

**Second direction (over-run).** On success the reservation is corrected after
the fetch:

```rust
let actual = digest.compressed_bytes;
ctx.summary.adjust_download_bytes(actual as i64 - reserved as i64);
```

`temp.rs:374-376`. `adjust_live` (`ledger.rs:224-237`) does not check the
ceiling. If a lying server sends more compressed bytes than the manifest's
`compressed_size`, the live figure can exceed `--max-download-bytes` by the
delta (bounded by the in-flight tasks). The code comment at `temp.rs:336-341`
claims exactness under capture invariant A6 (`len(blob) == compressed_size`);
a malicious CDN breaks A6. This direction is source-verified; it was **not
executed**.

**Verification:** source-verified reserve/refund/retry paths; no test delivers a
partial body then drops (the mock only has `corrupt_first_chunks`, a complete
body with a bad digest — `tests/common/mod.rs:521`).

**Classification/severity:** `bug`, Medium (resource-ceiling under-enforcement;
requires a flaky/malicious transport and an active `--max-download-bytes`).

**Concrete fix.**
1. Make `stream_chunk_to` report consumed compressed bytes on failure (e.g.
   return a `StreamFailure { error, compressed_bytes }`, or take a
   `&mut u64` consumed counter), accumulating across retries. On failure,
   reconcile with `consumed` rather than refunding `reserved` blindly:
   `adjust_download_bytes(consumed - reserved)`. This charges exactly what
   crossed the wire and refunds only what did not.
2. Either enforce `compressed_size` as a hard cap while streaming (reject a
   body that exceeds the reservation) or explicitly document that the live
   ceiling is approximate when A6 is violated. The current “exact” comment is
   too strong.
3. Correct the comment at `temp.rs:354-357` to state the real bound: up to
   `ATTEMPTS` partial bodies per call, up to `MAX_ATTEMPTS` calls per file.
4. Add a mock test that streams a prefix then drops, with
   `--max-download-bytes` active; assert the live/spent figure and that the
   ceiling is not materially under-enforced.

---

### F-4 — Medium — `--print-plan` + explicit `--audio` writes the scan file

**Locations:** `src/repair/phases/metadata.rs:134-153`;
`src/config.rs:181-183`; `src/repair/mod.rs:93`, `:301-335`;
`src/repair/audio.rs:95-99`; `README.md:227-231`;
`docs/04-scope-limitation-spec.md:386-389`.

**What the code does.** `metadata::fetch` runs at `src/repair/mod.rs:93`,
before the `--print-plan` early return at `:308-335`. The audio write is gated
only on `!ctx.readonly()`:

```rust
if ctx.audio_explicit && !ctx.readonly() && !config.audio_pkg_scan_dir.is_empty() {
    ...
    audio::write_scan_file(game_dir, &config.audio_pkg_scan_dir, &audio)
```

`metadata.rs:136-153`. `RunCtx::readonly()` is `dry_run || check_only`
(`config.rs:181-183`); `print_plan` is not included. `write_scan_file` joins
the configured scan dir, calls `create_dir_all(parent)`, and writes the file
(`audio.rs:95-99`). So:

```
girpr --game-path <missing-dir> --biz hk4e_global --print-plan --audio en-us
```

creates `<missing-dir>/<scan_dir>` and the scan file, then returns `0` from the
plan branch. This contradicts the code's own comment at
`src/repair/mod.rs:301-304` (“returns before `create_dir_all`, so no
`game_dir`”), README:227-231 (“read-only by construction”), and
`docs/04:386-389`. Under an active path scope it can instead exit `1` because
the scan file is outside the scope (`metadata.rs:143-149`), even though a
`--print-plan` run should not be writing it at all.

**Verification:** source-verified control flow. The command was **not
executed**. The existing `print_plan_reports_the_plan_and_writes_nothing` test
(`tests/repair_offline.rs:1757-1799`) does not set `audio_explicit`, so it does
not exercise this path.

**Classification/severity:** `bug`, Medium (documented read-only contract
violated; creates directories/files in a mode that promises not to).

**Concrete fix.**
1. Gate the write (and the scope refusal that exists only because of the
   write) on `!ctx.readonly() && !ctx.print_plan`; still resolve `audio` for
   the report.
2. Add a test: `ctx.print_plan = true; ctx.audio_explicit = true;` with a
   missing `game_dir`; assert exit `0`, no `Audio/` directory, no scan file,
   and no `game_dir` creation. Also assert the report names the requested
   language.
3. If the maintainers instead decide `--print-plan` may write, then the
   README/docs/comment must be changed to say so; the current three say it
   does not.

---

### F-5 — Medium — `--check-only --scan` can report an absent zero-length file as clean

**Locations:** `src/repair/phases/check.rs:38-41`, `:54-72`;
`src/repair/scan.rs:174-179`, `:39-40`, `:185`; `src/util.rs:9-13`,
`:52-59`, `:100-103`.

**What the code does.** With a scan report, the verdict branch compares only
the whole-file MD5:

```rust
Some(v) => {
    if v.file_md5.eq_ignore_ascii_case(&f.md5) {
        // intact
    } else { ... }
}
```

`check.rs:38-41`. It never checks `v.len` (the `Verdict` field exists at
`scan.rs:39-40` and is set at `:185`, but is not read in production code) and
never checks whether the file is present. An absent or unreadable file becomes
`Scan::absent`, whose `file_md5` is `EMPTY_MD5` and whose `file_len` is `0`
(`src/util.rs:9-13`, `:52-59`; `scan.rs:174-179`; NotFound at
`util.rs:100-103`).

Therefore a manifest entry with `size == 0` and `md5 == EMPTY_MD5` (a legitimate
empty file) plus a missing/unreadable file is judged **intact** by
`--check-only --scan` and exits `0`. The non-scan fallback checks length first:

```rust
if actual_len != Some(f.size as u64) {
    bad += 1;
```

`check.rs:54-59`; `None != Some(0)` catches the absent file and exits `4`.
So the two spellings of `--check-only` disagree on the same install.

**Verification:** source-verified code paths; the scenario depends on a
zero-length manifest entry and was **not executed**. `--check-only --scan` has
no test at all in the current tree (grep finds `--scan` only with
`--print-plan` and a normal repair in `tests/repair_offline.rs:918`, `:937`).

**Classification/severity:** `bug`, Medium (false “clean” in the verification
mode; narrow trigger).

**Concrete fix.**
1. In the scan branch, require presence and length:
   `util::file_len(&p) == Some(f.size as u64) && v.file_md5.eq_ignore_ascii_case(&f.md5)`.
   A `stat` per file is cheap and matches the non-scan path. Alternatively add
   a `present: bool` to `Verdict`/`Scan` and use `v.len == f.size as u64 &&
   v.present`.
2. Remove or use `Verdict.len`; it is currently written and never read.
3. Add a test with a zero-length manifest entry: file absent →
   `--check-only` and `--check-only --scan` both exit `4`; file present empty →
   both exit `0`.

---

### F-6 — Low — Panicked scan tasks are dropped without a warning

**Location:** `src/repair/scan.rs:132`

```rust
let Ok(scanned) = res else { continue };
```

**What the code does.** `scan_one` itself catches unreadable files and returns
`Scan::absent` (`scan.rs:174-179`), so the only `Err` from `spawn_blocking` is
a panic/cancellation. The `continue` drops it silently: the file is absent
from `ScanReport.files`, `out.bytes` and `out.damaged_chunks` undercount, and
the `scan` log at `:137-142` reports a smaller file count. The consequences are
bounded: `ScanReport::est_download_bytes` estimates an unreached file in full
(`scan.rs:77-96`), which is a safe overestimate, and `--check-only` falls back
to re-reading the file (`check.rs:54-72`). The defect is diagnosability, not a
wrong repair.

**Verification:** source-verified. No test injects a scan-task panic.

**Classification/severity:** `bug`, Low (observability/undercount).

**Concrete fix.** Re-land the reverted warning (the reverted commit `474f3f9`
added it):

```rust
Err(e) => tracing::warn!(
    error = %format!("{e:#}"),
    "scan task failed; unreached files are estimated in full"
),
```

Optionally add a `scan_failures` counter to `ScanReport` and surface it in
`SUMMARY`. This can be folded into the F-1 panic-handling change.

---

### F-7 — Low — `--max-write-bytes` is documented as `*_tmp` only

**Locations:** `src/cli.rs:89-95`; `src/repair/scope/limits.rs:20`, `:55`;
`src/report.rs:390`; `src/repair/scope/ledger.rs:394-395`;
`docs/04-scope-limitation-spec.md:168`, `:291`.

**What the code does.** The help text says “once this many bytes have been
written into `*_tmp`” (`cli.rs:89-95`), the limits table and field doc say the
same (`limits.rs:20`, `:55`), `report.rs:390` says “Bytes written into
`*_tmp`”, `ledger.rs:394-395` says every write “lands in `*_tmp`”, and
`docs/04:168`, `:291` repeat it. But the default in-place path writes the
target itself through `write_at` (`src/repair/file/mod.rs:531-543`),
`copy_within` (`:551-575`), and `zero_fill` (`:581-604`), called from
`src/repair/file/inplace.rs:298`, `:305`, `:324`. The temp path is the one that
writes `*_tmp` (`temp.rs:285-299`). The counter is correct; only the prose is
wrong. `README.md:290` already has the accurate wording (“written into the
install”).

**Verification:** source-verified at all listed lines.

**Classification/severity:** `doc error`, Low.

**Concrete fix.** Replace “into `*_tmp`” with “into the install — the target for
`inplace`/`discard`, `{path}_tmp` for `temp`” at all six sites. Do not change
the counter or the ceiling.

---

### F-8 — Medium — Regression coverage gaps for the high/medium findings

**Locations / existing coverage:**
- Resume test: `tests/repair_offline.rs:336-365` (`partial_tmp_prefix_is_resumed`).
- Mid-file rollback test: `tests/repair_offline.rs:2813-2866`
  (`a_corrupt_chunk_is_not_resumed_as_already_complete`).
- Print-plan “writes nothing” test: `tests/repair_offline.rs:1757-1799`
  (does not set `audio_explicit`).
- Read-only deprecated-call test: `tests/repair_offline.rs:2682-2719`
  (does not set `audio_explicit`).
- CLI help test: `tests/cli_contract.rs:216-252`.
- Mock only has `corrupt_first_chunks` (a complete corrupt body):
  `tests/common/mod.rs:521`.

**Missing tests (all source-verified as absent by grep):**
1. **Panic → exit code.** No test uses an `i64::MAX` offset or otherwise
   injects a task panic. Without one, F-1 can regress silently. Re-land the
   corrected version of the reverted test: huge offset, `--repair-mode temp`,
   assert exit `3`, `files_failed=1`, no `config.ini`, no repaired file.
2. **Drive-relative and post-phase traversal.** No test uses `C:`, `C:/`,
   `../`, `/abs`, or a UNC path; no test puts `../x` in the deprecated list or
   `..` in the audio cache/res dirs.
3. **Partial-body refund.** No mock streams a prefix and then drops; only
   complete corrupt bodies are tested. Add a `--max-download-bytes` test and
   assert the live/spent figure.
4. **Full-length corrupt `_tmp`.** The rollback test covers a mid-file digest
   failure, not a pre-seeded full-length `_tmp` with wrong bytes. The behavior
   is safe (the final MD5 gate deletes it — `temp.rs:419-433`), but the
   “delete → retry → success” path is not pinned.
5. **`--print-plan --audio`.** No test combines the two; the existing
   print-plan test leaves `audio_explicit = false`.
6. **`--check-only --scan` zero-length absence.** No test combines
   `check_only` and `scan` at all; add the absent-empty-file case from F-5.

**Verification:** source-verified absence of the tests; no tests were run.

**Classification/severity:** `test gap`, Medium (the missing tests cover the
High/Medium behaviors above).

**Concrete fix.** Add the six tests alongside the corresponding fixes; do not
treat the existing resume/rollback tests as covering the panic, drive-relative,
partial-body, or print-plan cases.

---

### F-9 — Low — `kept_bytes` comment is wrong under temp resume

**Location:** `src/repair/file/mod.rs:261-264`

```rust
// The bytes that did not have to be written: the file's
// size minus what was written. The in-place path
// leaves the correct chunks alone; the temp path
// rewrites the whole file and keeps nothing.
```

**What the code does.** The temp path resumes by length: if the `_tmp` already
extends past a chunk's end, that chunk is `Resumed` and not written
(`temp.rs:194-204`). A `_tmp` prefix left by an earlier run is therefore kept,
and `kept = file.size - settled_writes` (`mod.rs:270-272`) counts it as kept.
The counter's own definition — “bytes already correct where they belonged and
were not written” (`report.rs:297-308`) — supports that; only the “keeps
nothing” sentence is false. The existing resume test
(`tests/repair_offline.rs:336-365`) does not assert `kept_bytes`.

**Verification:** source-verified; the value was not computed by a test.

**Classification/severity:** `doc error`, Low.

**Concrete fix.** Change the sentence to “the temp path rewrites every chunk it
does not resume; a `_tmp` prefix left by an earlier run is kept and counts
here.” Add an assertion to `partial_tmp_prefix_is_resumed` (with
`repair_mode_explicit = true`) that `kept_bytes == prefix_len`.

---

## 3. Verified-correct behavior (do not “fix”)

These are source-verified and should not be changed in response to the reverted
review:

1. **`layout`’s `offset + len` cannot overflow.** `offset` and `len` are
   non-negative `i64` values cast to `u64` (`src/repair/phases/plan.rs:63-70`);
   each is at most `i64::MAX`, so their sum is at most `u64::MAX - 1`. The
   reverted `checked_add` in `layout` (`474f3f9`) was dead defensive code; do
   not re-land it as a “fix”. The unchecked addition that *can* overflow is the
   `i64` one at `temp.rs:193`, which F-1 covers.
2. **`reuse.rs` and `inplace.rs` already use checked addition.**
   `src/repair/reuse.rs:128` and `src/repair/file/inplace.rs:426` both use
   `checked_add` for source-range ends; no change is needed there.
3. **Purge skips symlinks.** `plan_purge` walks with `follow_links(false)` and
   skips anything that is not a regular file (`src/repair/purge.rs:88-99`).
   The reverted review’s claim that `e.metadata()` follows symlinks and makes
   the delete ceiling disagree with freed bytes is false for the current tree.
4. **A corrupt `_tmp` cannot be promoted.** `promote_tmp` checks the whole-file
   length and MD5 before the rename (`temp.rs:419-433`); on mismatch it deletes
   the `_tmp`. Resume-by-length may keep a bad prefix, but the final gate
   prevents a bad install. The only issue is wasted work/test coverage, not
   corruption.
5. **The non-scan `--check-only` path catches absence.** It checks
   `actual_len != Some(f.size as u64)` before hashing (`check.rs:54-59`); the
   F-5 bug is specific to the scan branch.
6. **The post-repair `--max-delete-bytes` re-check matches the docs.**
   `post.rs:54-70` re-plans and re-checks; `docs/04:310-313` documents that
   the post pass can only be smaller and is checked independently. This is not
   a defect.
7. **`--plan-out` write failure → exit `3` is defensible.**
   `src/repair/mod.rs:331-332` maps it to `RunFailure::write`; it happens
   before any install mutation and is the write/verify class. It is untested,
   but it is not a bug to “fix” without a classification decision.
8. **`build_plan` is pure and runs before `create_dir_all`.**
   `src/repair/mod.rs:77-80` deliberately does not create `game_dir` early;
   `plan.rs:172-242` is a pure function over manifest data. This ordering is
   intentional and protects refusal paths from leaving a directory behind.
9. **`plan_purge` on a missing `game_dir` is empty by design.**
   `src/repair/mod.rs:162-176` documents that a missing directory yields an
   empty plan, which is the same result as planning a freshly created empty
   directory. Do not add a `create_dir_all` before it.
10. **`--audio` under `--dry-run`/`--check-only` does not write.**
    `metadata.rs:136` plus `config.rs:181-183` correctly suppress the write;
    only `--print-plan` is missing from `readonly()` (F-4).
11. **The scan fallback estimate is a safe overestimate.** A file missing from
    `ScanReport.files` is estimated in full (`scan.rs:77-96`); that is the safe
    direction and is not a correctness bug.
12. **Real file failures already produce exit `3`.** `files_failed` is
    incremented at `file/mod.rs:276`, and `repair/mod.rs:480-482` returns `3`;
    runtime hard stops return `5` at `:468-479`. The panic path is the only
    hole (F-1).

---

## 4. Corrections to the reverted review series

Every claim below is from `git show f9352f7:ADVERSARIAL_REVIEW.md` or the
reverted fix commit `474f3f9`. The “actual” column is the current tree.

| # | Old claim | Actual in `64dc69d` / `7f2d69` | Consequence / re-landing |
|---|---|---|---|
| 1 | F-1 was fixed by counting a non-panic task `Err` as `files_failed`; panics stay swallowed “matching sister projects”. | The spawned task returns `()` (`file/mod.rs:211-290`), so the only `Err` is a panic/cancel. The reverted `Err(e) if e.is_panic() => {}` arm swallows exactly the case that matters; the `Err(e)` arm is unreachable. Current tree has neither arm. | **Not fixed.** Re-land by counting *every* join error (including panics) as `files_failed` and logging it, as in F-1. Do not preserve the “panic stays swallowed” design. |
| 2 | F-1 recommended `checked_add` in `PlannedFile::layout`. | `layout`’s inputs are non-negative `i64` cast to `u64`; the sum cannot overflow (`plan.rs:63-70`, `:85-86`). | That part of the old fix was **dead defensive code**. The real overflow is the `i64` add at `temp.rs:193`; fix that instead. |
| 3 | F-2’s `validate_rel` “covers repair targets, scan reads, reuse slices and purge candidates”. | It only validated manifest rels. Deprecated names (`post.rs:142`) and audio cache/res dirs (`post.rs:185-186`, `:208-222`) still join server strings; `metadata.rs:151` still writes the scan file. `C:evil` also slips: it is not `is_absolute()` and does not start with `/`. | **Partial fix only.** Re-land a containment helper at every server-path boundary, with a `Component::Prefix`/`has_root`/`ParentDir` check and tests for `C:`. |
| 4 | F-2 claimed purge’s `e.metadata()` follows symlinks, so the delete ceiling and freed bytes disagree. | `plan_purge` uses `follow_links(false)` and skips non-regular files (`purge.rs:88-99`); `metadata()` only runs for regular files. | **False.** Remove the symlink-size claim; no purge change is needed. |
| 5 | F-3 said a failed fetch under-enforces by “at most one partial chunk”. | `stream_chunk_to` retries `ATTEMPTS = 5` (`sophon.rs:385-399`, `:500`), and a file is attempted `MAX_ATTEMPTS = 5` times (`file/mod.rs:86`). | Bound is up to **5 partial bodies per call, 25 per chunk per run**. The reverted comment fix repeated the wrong “one partial chunk” bound; correct it. |
| 6 | F-3 said the over-count direction (manifest understating `compressed_size`) lets the live figure exceed the ceiling. | True: `adjust_download_bytes(actual - reserved)` happens after the fetch (`temp.rs:374-376`) and `adjust_live` does not check the ceiling (`ledger.rs:224-237`). | Keep this as a caveat; either hard-cap the body at `reserved` or document that A6 is trusted. |
| 7 | F-4 said a full-length corrupt `_tmp` “is deleted, and the file is retried from scratch (eventually exit 3 after retries)”. | `promote_tmp` deletes the `_tmp` on MD5 mismatch (`temp.rs:425-433`); the retry starts clean and can **succeed**. Exit `3` only if all retries fail. | Behavior is safe; correct the expected outcome. The full-length case is still a test gap (F-8). |
| 8 | F-5 said only `cli.rs:89-95` was imprecise. | Six more sites still say `*_tmp`: `limits.rs:20`, `:55`; `report.rs:390`; `ledger.rs:394-395`; `docs/04:168`, `:291`. | The old fix was **incomplete**. Re-land all doc corrections; `README.md:290` is already correct. |
| 9 | F-7 said “read-only modes never write the scan file”. | True for `--dry-run`/`--check-only` (`metadata.rs:136`, `config.rs:181-183`), false for `--print-plan` because `readonly()` excludes it. | This is F-4, not a docs-only asymmetry. Gate the write on `print_plan` too. |
| 10 | F-8 said `--max-delete-bytes` post-pass behavior is operator-facing confusion. | `post.rs:54-70` re-checks the post-repair re-plan; `docs/04:310-313` already documents that the two passes are checked independently. | No defect. The old recommendation was already implemented; do not “fix” it. |
| 11 | F-9 said `--plan-out` write failure is exit `3`, untested. | Still true at `src/repair/mod.rs:331-332`. | Defensible; leave the code, optionally add the pinning test. |
| 12 | F-6 said the `#[ignore]`d bridge/peak_heap suites never run in the standard suite. | Still true: `tests/artifact_bridge.rs:718`, `tests/peak_heap.rs:261`. | Process note, not a code defect. Keep them opt-in; run them on repair-spec changes. |
| 13 | F-10 said metadata/manifest traffic is not bounded by any ceiling. | Still true and documented (`README.md:290` says the download ceiling does not bound metadata/manifest traffic; docs/04 §2.2). | Intentional; no change. |
| 14 | F-1 said `scan_plan`’s `let Ok(...) else continue` makes the file estimated in full, “silently”. | True, but `scan_one` catches unreadable files (`scan.rs:174-179`), so `Err` is a panic; the estimate is a safe overestimate and `--check-only` re-reads. | Downgrade to Low (F-6): the defect is the missing warning/undercount, not a wrong repair. |

---

## 5. Open questions / unverified

- **No runtime verification.** The reverted review claimed “270 passed, 0
  failed”; that claim is **not verified** for this tree. No tests were run here.
- **Real manifest contents.** Whether the live CDN ever serves a zero-length
  entry with `EMPTY_MD5`, a `..` segment, a drive-relative `C:` path, or an
  `offset + size > i64::MAX` is **not verified**. The findings are about what
  the code does if such data arrives, not a claim that it does.
- **`PathBuf::push` semantics.** The `C:evil` escape relies on the documented
  `std::path::PathBuf::push` rule that a prefixed-but-rootless path replaces
  the base on Windows. This was **not executed** on the review host; the
  absence of validation is source-verified.
- **Panic sites.** `temp.rs:193` is a concrete unchecked expression; other
  panic sites may exist (e.g. the semaphore `unwrap` at
  `file/mod.rs:212`). The review did not enumerate every possible panic.
- **Partial-body delivery.** The claim that `stream_body` can deliver decoded
  pieces before returning `Transport` follows from the loop at
  `sophon.rs:476-490`; no test exercises it. The exact number of bytes a
  dropped connection delivers is transport-dependent.
- **`--print-plan --audio` under a scope.** The exit-`1` path
  (`metadata.rs:143-149`) is source-verified but was **not executed**.
- **`--check-only --scan` false clean.** The trigger requires a zero-length
  manifest entry; whether such an entry exists in a real Genshin manifest is
  **not verified**.
- **`kept_bytes` on resume.** The metric is defensible as “bytes not written”;
  whether maintainers intend a resumed `_tmp` prefix to count as “kept” is a
  product decision. The comment is wrong either way.

---

## 6. Recommended landing order

1. **F-1 (High) + the panic regression test.** Smallest diff, highest
   consequence: stop the silent exit `0`/`config.ini` bump, and replace the
   unchecked `temp.rs:193` addition with `u64::try_from` + `checked_add`.
   Re-land the arithmetic part of `474f3f9`; do **not** re-land its
   `is_panic` arm.
2. **F-2 (High) + containment tests.** One helper, applied to manifest rels,
   deprecated names, audio cache/res dirs, and scan/blacklist dirs. This is the
   largest behavioral change; do it before any further post-phase work. Do
   not re-land `474f3f9`’s `validate_rel` verbatim: add the prefix check and
   the non-manifest sinks.
3. **F-4 (Medium).** Gate the audio scan write on `print_plan`; add the
   missing test. Small and self-contained.
4. **F-3 (Medium).** Make `stream_chunk_to` report consumed bytes on failure,
   reconcile the reservation, and correct the comment/bound. This may need a
   small API change; land after F-1/F-2 so the retry/exit behavior is already
   correct.
5. **F-5 (Medium).** Add the presence/length check to the scan branch; use or
   remove `Verdict.len`; add the zero-length test.
6. **F-6 (Low).** Re-land the scan-task warning (can be part of F-1’s
   join-error handling).
7. **F-7 and F-9 (Low).** Batch the doc corrections; no code behavior changes.
8. **F-8 (Medium test gap).** Add the remaining tests as each fix lands; the
   full-length corrupt `_tmp` test is optional but cheap, and the
   `--check-only --scan` / `--print-plan --audio` tests are not.

**Do not re-land the reverted series as-is.** Specifically:
- F-1’s `Err(e) if e.is_panic() => {}` does not fix F-1.
- F-2’s `validate_rel` misses `C:evil` and all post-phase sinks.
- F-3’s “at most one partial chunk” comment is factually wrong.
- F-5’s one-file doc fix leaves six sites wrong.
- `layout`’s `checked_add` is dead code and should not be presented as a fix.

---

## 7. Implementation progress

Landed on branch `fix-gaps/girpr` in the §6 order, one commit per finding
(F-7 and F-9 are separate commits; F-8's tests live in the commits whose fixes
they pin, plus one test-only commit). This is the **first-pass** table: the
pass log in §8 re-validated the residual claims, fixed them, and records the
final verdict for every finding. No commit re-lands the reverted series
verbatim: the join-error arm counts panics (there is no `is_panic => {}`),
`layout`'s dead `checked_add` is not reintroduced, `validate_rel` is replaced
by `util::contained_rel` / `contained_join` applied to every server-path sink,
and all six `*_tmp` doc sites plus the `kept_bytes` comment are corrected.

| # | Finding | Status | Regression tests | Commit |
|---|---|---|---|---|
| F-1 | Panicked file tasks exit `0` | **Fixed.** `join_file_tasks` awaits every handle and counts *any* join error as `files_failed`, logs it, and aborts under `--fail-fast`. The `temp.rs` range end is `u64::try_from` + `checked_add`; a new `validate_chunk_ranges` also refuses negative/overflowing/out-of-file ranges before a seek. | Unit: `a_panicked_file_task_is_counted_as_a_failure`, `a_chunk_past_the_files_end_is_refused`. Integration: `a_manifest_offset_overflow_fails_the_file_instead_of_panicking` (exit `3`, `files_failed=1`, no `config.ini`, no target). | `a3e5caf` |
| F-2 | Uncontained server paths | **Fixed.** `util::contained_rel` / `contained_join` reject empty, drive prefix (`C:`, `C:/`), root/absolute, UNC, `ParentDir` and non-`Normal` components, then resolve the nearest existing ancestor and require `starts_with(base)` (catches symlink escapes). Applied to the plan, repair target, `_tmp` reuse/promote, scan read, check, config/blacklist dirs, audio read/write, deprecated deletes, audio cache/res moves and the scope report. `build_plan` is now fallible: a bad rel is metadata, exit `2`, before any mutation. | Unit spellings incl. `C:`, `C:/`, `../`, `/abs`, UNC, a legitimate `a/b.dat`, a missing base, and `#[cfg(unix)]` symlink escape. Integration: `a_manifest_path_that_escapes_the_game_dir_is_refused_before_any_write`, `a_deprecated_path_that_escapes_the_game_dir_is_kept`, `an_audio_cache_dir_that_escapes_the_game_dir_is_not_moved`; unit `a_scan_path_that_escapes_the_game_dir_is_not_read_or_written`. | `5b9159e` |
| F-4 | `--print-plan --audio` writes the scan file | **Fixed.** The scan write and the scope refusal that exists for it are gated on `!ctx.readonly() && !ctx.print_plan`; the audio set is still resolved for the report. | `print_plan_with_audio_writes_no_scan_file_and_names_the_language` (exit `0`, no game dir, no `Audio/` dir, `audio_langs=en-us`). | `183f316` |
| F-3 | Failed fetch refunds bytes already sent | **Fixed.** `stream_chunk_to` takes `&mut u64` consumed bytes, accumulates them across every attempt (including the successful one), and `temp.rs` reconciles the reservation with `consumed - reserved` on both success and failure. Each body is capped at its reservation; the body of an attempt that fails is charged. The bound is now stated correctly: up to `ATTEMPTS` partial bodies per call, `ATTEMPTS × MAX_ATTEMPTS` per chunk per run. | Unit: `a_mid_stream_transport_failure_reports_what_arrived`, `a_body_past_its_reservation_is_refused`; integration: `a_partial_body_then_a_drop_is_charged_to_the_download_meter` (live = full + partial, settled = full, retried once). | `82a6d85` |
| F-5 | `--check-only --scan` calls a missing empty file clean | **Fixed.** The scan branch now checks `util::file_len == f.size` *and* the digest, matching the non-scan path; the never-read `Verdict.len` is gone. | `check_only_scan_treats_an_absent_empty_file_as_damage` (absent → exit `4` in both spellings, present-empty → `0`). Verified the test fails without the fix. | `778ae1d` |
| F-6 | Panicked scan tasks are dropped silently | **Fixed.** The `let Ok(...) else continue` is a `warn!` naming the error and the estimate's safety; the file stays absent from the report and is estimated in full. | None added: `scan_one` catches unreadable files itself, and a `spawn_blocking` panic is not reachable through a public seam; the change is a log line only. | `64c80d6` |
| F-7 | `--max-write-bytes` documented as `*_tmp`-only | **Fixed.** All six sites (`cli.rs`, `limits.rs:20`/`:55`, `report.rs`, `ledger.rs`, `docs/04:168`/`:291`) now say the install — the target for the in-place path, `{path}_tmp` for the temp path. No counter or ceiling changed. | None needed (prose only; the CLI-contract test already pins the flag itself). | `0b0aec8` |
| F-8 | Regression coverage gaps | **Fixed.** All six listed gaps are covered: panic → `3`, the drive-relative/absolute/UNC/traversal spellings and the two post-phase sinks, the partial body, `--print-plan --audio`, `--check-only --scan` zero-length absence, and the full-length corrupt `_tmp`. | The first five are in the F-1/F-2/F-3/F-4/F-5 commits above; the last is `a_full_length_corrupt_tmp_is_deleted_and_rebuilt` (delete → retry → rebuild, `_tmp` not reused). | `3546902` |
| F-9 | `kept_bytes` comment says the temp path keeps nothing | **Fixed.** The comment now says the temp path rewrites every chunk it does not resume and that an earlier `_tmp` prefix counts as kept; the resume test sets `repair_mode_explicit` and pins `kept_bytes == 64`. | `partial_tmp_prefix_is_resumed` (extended). | `64e32ac` |

### 7.1 Behavioural notes and pending maintainer decisions

1. **F-3 hard cap vs. documenting A6 (decided: cap).** A body that sends more
   than the manifest's `compressed_size` is now refused at the piece that
   crosses the line instead of being streamed. That is stricter than capture
   invariant A6 and means a *well-formed but different-length* frame is refused
   before its digest can be checked; the write-meter test pins its corrupt body
   by giving the manifest a few bytes of `compressed_size` slack. The review
   allows the alternative (trust A6 and document the approximation). Keep the
   cap: it bounds the traffic, and a body past its claim is not the blob the
   manifest describes.
2. **F-3 reported vs. live figures (decided: unchanged split).** `download_bytes`
   (settled) still counts only the successful body of each chunk, because it is
   a function of the promoted install; `download_bytes_spent` (live) now counts
   every partial body. A chunk can charge up to `ATTEMPTS × reserved` per call,
   so a flaky origin can pass the ceiling by the bytes it actually delivered
   before the next reservation stops the run. That is the honest number; the
   alternative (refuse the retry once the reservation is spent) would reject
   recoverable transfers.
3. **F-2 plan validation runs before the blacklist/scope/dedup filters
   (decided: strict).** Any escaping manifest entry fails the plan with exit
   `2`, even one that would have been filtered out. The alternative is to
   validate after filtering and silently ignore a hostile entry that is out of
   scope. Keep strict: a manifest that names a path outside the install is not
   trustworthy for the parts that happen to be in scope.
4. **F-5 checks presence with one `stat` per planned file under
   `--check-only --scan` (decided: stat).** It matches the non-scan path's
   answer and avoids a second notion of presence. A `Verdict.present` flag
   would avoid the syscall but add state that can drift from the disk between
   the scan and the check.
5. **F-9 `kept_bytes` on resume (product decision, implemented as kept).** A
   resumed `_tmp` prefix is bytes the repair did not write, so it counts as
   kept, consistent with the field's definition. If maintainers prefer "kept"
   to mean only correct-where-it-belongs in-place bytes, the counter needs a
   different definition and the new assertion changes with it.
6. **F-1 dry runs skip `validate_chunk_ranges`.** A range that would fail a
   write is still planned under `--dry-run`/`--check-only`, because those modes
   never seek; a corrupt range is reported by the repair paths that would use
   it. If the plan should refuse such a manifest instead, the validation moves
   into `build_plan` (exit `2`) rather than the per-file path (exit `3`).

### 7.2 Verification

Run on the first-pass final tree on Windows (`cargo` 1.x, debug profile). The
latest full-suite numbers are in §8.7.

- `cargo build` — ok.
- `cargo test -p girpr` — **286 passed, 0 failed, 7 ignored** (156 lib + 7 binary + 16 + 25 + 82 integration). The 7 ignored are the 6 `artifact_bridge` tests and the 1 `peak_heap` measurement.
- `cargo clippy --all-targets` — clean (no warnings).
- `cargo fmt --check` — clean.
- No shared crate changed, so no workspace-wide run was needed.

**Not verified here:** the live CDN (all tests use the in-process mock; the
manifest shapes are the captured ones, not a fresh fetch), the number of bytes a
real transport delivers before dropping (the mock truncates deterministically),
the `#[cfg(unix)]` symlink test (this host is Windows, so it compiled but did
not run), and `--print-plan --audio` under an active path scope (the fix gates
the write; the scope-refusal interaction is covered by the existing
`explicit_audio_under_a_scope_...` test, not by a print-plan variant).

## 8. Pass log

Detailed evidence, failure scenarios and per-pass verification are in the git history — `git log --oneline main..HEAD`, `git show <commit>` — and the commit hashes in the tables are the index. §1–§7 above are the first-pass record; their line numbers are from `64dc69d` and have moved since. `util::normalize_rel` (F-2) no longer exists: N-9 replaced it with `contained_rel`/`contained_join`. This compaction changed no source, test or behaviour.

### 8.1 Second pass — N-1…N-14

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

### 8.2 Third pass — N-15…N-20

| ID | Finding | Status | Commit |
|---|---|---|---|
| N-15 | The disk plan and document totals could wrap. | Fixed: `DiskPlan`, the short-vector fallback and the document totals saturate; `Val::UInt` renders unsigned. | `cbfaf4e` |
| N-16 | The `--plan-out` document re-derived `Σ compressed_size` and reconciled only upward. | Fixed: `PlanDocument::new` takes the per-file vector `PLAN` folds. | `d88c76a` |
| N-17 | The scanned estimate ignored the file's mode, under-counting a source the write overwrites. | Fixed: models discard/temp/in-place with `salvage_order`; a missing verdict still means the whole file. | `0b01501` |
| N-18 | A chunk with no source was verified at offset 0, faking reuse. | Fixed: ranges only for `Some(source)`; no-source chunks are `source_ok = false`. | `d5f84b3` |
| N-19 | A purge walk or candidate stat failure was dropped. | Fixed: non-`NotFound` errors warned and counted in `PurgePlan.errors`; a root `NotFound` stays the empty plan. | `408b239` |
| N-20 | Temp resume/debug stats and `rollback_tmp` substituted length `0` on error. | Fixed: `File::metadata()` errors propagate with the temp path as context. | `c04da2e` |

### 8.3 Fourth pass — P-1…P-4

| ID | Finding | Status | Commit |
|---|---|---|---|
| P-1 | Under `--scan --repair-mode temp` the estimate took the pre-scan reuse rule and answered `0`. | Fixed: a scanned run always uses the scan's per-file vector. | `4d2b2d9` |
| P-2 | Five stat failures were still read as zero (temp resume/promote, in-place trim/salvage order, deprecated size). | Fixed: propagate or count honestly; a failed deprecated stat warns, deletes, and omits the size. | `edb2eb4`, `63a53de` |
| P-3 | `moved audio cache -> res` was logged unconditionally, claiming success for an empty or refused move. | Fixed: `info!(moved, kept, …)` only when a relocation was attempted. | `25ce46f` |
| P-4 | The emptied-directory sweep dropped `remove_dir`'s error. | Fixed: debug log with the directory and the `io::Error`; still best effort. | `d331174` |

### 8.4 Fifth pass — R-1…R-6

| ID | Finding | Status | Commit |
|---|---|---|---|
| R-1 | `--plan-out` without `--print-plan` wrote nothing; the only write site sat in the print-plan branch. | Fixed: the document is written before execution; a failure is exit `3` with nothing mutated. | `867ed7f` |
| R-2 | Unreadable, absent and empty scan files were one state; `--audio none` fell back to `en-us` and, under `--purge-after`, deleted the real selection. | Fixed: `Ok(Some)`/`Ok(None)`/`Err`; an empty file means game-only; atomic write. | `a812fce` |
| R-3 | A failed rollback replaced the chunk error instead of annotating it. | Fixed: the rollback error is context; the chunk stays the cause. | `86f1fe3` |
| R-4 | `discard` mode's `remove_file(...).ok()` hid a refused delete. | Fixed: `discard_target` propagates every error except `NotFound`. | `646b9c9` |
| R-5 | `config.ini`'s `sync_all().ok()` published a rename after a failed sync. | Fixed: a failed sync aborts, and the temp is removed on the failed-write path. | `a812fce` |
| R-6 | The version/ignore-categories/blacklist readers treated every read error as absent. | Fixed: `Result<Option<T>>`; only `NotFound` is `None`, anything else is exit `1`. | `a812fce` |

### 8.5 Sixth pass — R-7…R-9

| ID | Finding | Status | Commit |
|---|---|---|---|
| R-7 | `metadata::fetch` wrote the explicit-`--audio` scan file before the run's refusals. | Fixed: the write moved to execution, after every refusal; the repair uses in-memory `Meta::audio`. | `be6d83f` |
| R-8 | A scan file naming no recognized language was read as game-only, silently changing the purge keep set. | Fixed: one leading BOM is stripped, non-blank text with no known language is refused, a BOM-only file is refused; a mixed file keeps the known set and warns per unknown line. | `a44c666`, `16e4573` |
| R-9 | `--plan-out` help omitted `mode` and `extra_bytes`. | Fixed: help lists both; `docs/04` already had the full shape. | `c9fb86f` |

### 8.6 Seventh pass — S-1…S-3

| ID | Finding | Status | Commit |
|---|---|---|---|
| S-1 | A scoped purge swept an **empty out-of-scope directory**: `mark_kept_ancestors` only marks directories reached from a kept file. | Valid, fixed: the walk adds only directories the scope admits to the sweep set; a hidden directory is kept and counted in `hidden` (so the scope warning names it). Reproduced with the binary before the fix (exit `0`, no warning, `mods/` and `mods/nested/` deleted) and confirmed after (dirs kept, warning, exit `0`). | `3c156c8` |
| S-2 | The staged-tier RAM payload used a non-saturating `.sum()` over server `uncompressed_size`. | Valid, fixed: `staged_payload` folds with `saturating_add`; three chunks at `i64::MAX` reach `u64::MAX` instead of wrapping to the stage-nothing sentinel (release) or panicking (debug). | `96b7ca9` |
| S-3 | `delete_deprecated`'s `is_file()` read every stat error as "not a file" and skipped the entry silently. | Valid, fixed: one `metadata` call is classified file / not-a-file / already-gone / failed; a failed stat warns with the path and deletes without counting an unknown size. | `e2040cd` |

### 8.7 Decisions that still bind

1. **Saturate, never wrap**, in every server-size estimate (N-13, N-15, N-19, S-2).
2. **Containment is a strict lexical gate**: fail closed on an unresolvable probe (N-1); refuse drive/UNC, `..`, trailing separators, Windows device names, trailing dot/space and ADS colons on every host (N-14); clean `./` rather than refuse it (N-6); fold case for plan dedup on Windows only (N-5); an outside path is exit `2` even if a scope filter would drop it (F-2).
3. **Downloads stop at settle time**: the overrun bound is the body already in flight; partial bodies charge `download_bytes_spent`, settled bodies charge `download_bytes` (F-3, N-2).
4. **The scanned estimate models the write path** and the resolved mode; one per-file vector feeds both `PLAN` and `--plan-out` (N-17, N-16, P-1).
5. **Absent is `NotFound` only**; unreadable local state is exit `1` (R-2, R-6). An empty audio scan file is game-only; non-blank text with no known language and a BOM-only file are refused; one leading BOM is stripped (R-8).
6. **The deferred audio write keeps the plan half mutation-free**: the scan file is written only in execution, after every refusal, so `--print-plan`/`--dry-run`/`--check-only` write nothing (R-7); `--plan-out` is a requested artifact, written before mutation, failure exit `3` (R-1).
7. **Publishing is crash-safe**: scan file via same-directory temp plus rename (R-2); `config.ini` syncs before its rename and removes the temp on a failed write (R-5).
8. **The post phase is best-effort housekeeping**: only the `config.ini` bump and the delete ceiling are fatal; a refused move/delete, an unreadable deprecated size, an unremovable directory and an unreadable purge subtree are warned, counted where possible, and left in place (N-3, N-11, P-2, P-4, N-19).
9. **A stat failure is never a zero and never "not a file"**: repair paths fail the file with path context; the post-phase delete warns with the entry and error and omits an unknown size, while a directory or an absent path stays a distinct, logged state (P-2, N-20, S-3).
10. **Log lines derive from outcomes**: relocation logs only when a move was attempted, with `moved`/`kept`; a containment failure logs as an escape (P-3, N-8).
11. **`kept_bytes` counts a resumed `_tmp` prefix**, unverified until the MD5 gate (F-9/N-10).
12. **File tasks join in completion order**, and the file-semaphore invariant is a named `expect` (N-7, N-12).
13. **The path scope bounds every deletion site, including the directory sweep**: a directory the scope hides is kept out of `PurgePlan::dirs` and counted in `hidden`, so an empty out-of-scope placeholder is not removed (S-1).

### 8.8 Latest verification

Full run on Windows, debug profile, seventh-pass final tree:

- `cargo build` — ok.
- `cargo test -p girpr` — **335 passed, 0 failed, 7 ignored** (196 lib + 7 `artifact_server` + 17 `cli_contract` + 25 `repair_machinery` + 90 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0 `src/main.rs`, 0 doc-tests).
- `cargo clippy -p girpr --all-targets` — clean, no warnings.
- `cargo fmt --check` — clean (exit `0`).

**Not run:** the `#[cfg(unix)]` blocks of the containment and mode tests (this host is Windows); the 7 ignored tests; no release-profile run; no Linux/macOS host; no live CDN traffic (all tests use the in-process mock). S-1 was reproduced with the real binary over the mock. R-8's purge-deletion consequence and the mixed-line warning capture remain code-reading claims: the mock fixture has a single `game` manifest and no per-language package to delete.

### 8.9 Open for maintainers

- `expected_paths()` and `classify_purge_path` still compare case-sensitively on Windows. N-5 closed duplicate plan entries, but an existing file whose case differs from the manifest can still be an orphan under `--purge-after`.
- F-6's `spawn_blocking` panic, P-2's two in-place `File::metadata()` sites, S-3's deprecated-stat failure arm, and R-5's `sync_all` failure have no public injection seam; S-3's arm is pinned at its classifier, the rest remain code-reading claims.
- S-1 keeps an empty out-of-scope directory rather than warning and deleting it; the choice is deliberate (the scope says the path does not exist) and pinned by a unit and an integration test. A directory hidden only by an `--exclude` of its *contents* (`--exclude "mods/*"`) is still swept when empty, because the exclude pattern does not name the directory itself.
- `--dry-run`/`--check-only` still skip `validate_chunk_ranges` (§7.1 item 6); a corrupt range is refused by the repair paths (exit `3`), not by the plan (exit `2`).
- N-19's `errors` counter is diagnostic only; the delete ceilings still see the candidates the walk could reach, not the ones under an unreadable directory.
- R-2's strictness is a behaviour change: an unreadable audio scan file now fails the run (exit `1`) instead of falling back to `en-us`.

*End of review. §1–§7 record the first pass; §8 is the pass log, and the commits it cites are the source, test and doc changes.*
