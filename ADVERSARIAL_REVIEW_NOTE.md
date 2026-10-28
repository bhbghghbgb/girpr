# ADVERSARIAL REVIEW NOTE — fifth pass

**Reviewed tree:** branch `fix-gaps/girpr`, HEAD `e88b62a`
(`docs(review): fold the fourth-pass verdicts into the review`).
**Baseline:** `main` (`git diff main..HEAD`).
**Scope:** `src/**`, `tests/**`, `docs/**`, root `README.md`, root `Cargo.toml`.
`tools/sync`, `tools/capture`, `crates/log`, and `crates/tempdir` were read
only where a `girpr` change depends on them.
**Standard:** strict validation, fail fast, and no errors omitted from the
report. A swallowed failure or a documented contract the code does not honour
is a finding, even when the happy path is correct.

This pass re-checked the four fixes recorded by the fourth pass (P-1 … P-4) and
then looked for the same classes of defect elsewhere. P-1 … P-4 still hold;
the residuals below are new or previously unrecorded.

---

## 1. Verdict

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| R-1 | Medium | bug | `src/repair/mod.rs:311`, `:327-336`; `src/cli.rs:177-181`; `README.md:301` | `--plan-out` without `--print-plan` is silently ignored, contradicting both the flag help and the README. |
| R-2 | Medium | bug | `src/repair/audio.rs:46-53`, `:61-91`; `src/repair/phases/metadata.rs:165` | Audio scan-file read conflates *absent*, *unreadable*, and *empty*; an empty file (written by `--audio none`) becomes `en-us` on the next unflagged run, and an unreadable file does so with no warning. |
| R-3 | Low | swallowed error | `src/repair/file/temp.rs:118-130` | A failed rollback replaces the original chunk error, so the run reports the wrong failure. |
| R-4 | Low | swallowed error | `src/repair/file/inplace.rs:155-158` | `discard` mode ignores the delete failure, then reports a misleading "create …" error (or, on some filesystems, keeps writing into the file it meant to discard). |
| R-5 | Low | swallowed error | `src/util.rs:688-696` | `config.ini` durability: `f.sync_all().ok()` drops the fsync failure before the rename that publishes the file. |
| R-6 | Low | swallowed error / test gap | `src/util.rs:444-450`, `:453-473`, `:476-493` | `read_game_version`, `read_ignore_categories`, and `read_blacklist` treat every read error as "absent" without a log; an unreadable blacklist/ignore file silently changes what the run does. |

Severity caps: R-2 needs an I/O error (or a prior `--audio none`) to mis-select;
the data-loss path runs through `--purge-after`, whose expected set comes from
the repair plan (`src/repair/purge.rs:88`, `:348`). R-3 … R-6 are diagnostics /
robustness issues that do not by themselves produce a wrong repair, so they are
Low under the scale in `ADVERSARIAL_REVIEW.md`.

---

## 2. Fourth-pass fixes re-checked

| Fix | Still fixed? | Current-tree evidence |
|---|---|---|
| P-1 single scan estimate rule | yes | `RepairPlan::est_download_bytes` / `est_download_bytes_by_file` (`src/repair/phases/plan.rs:139-152`); no second derivation in `--plan-out`. |
| P-2 stat failures propagate | yes | `src/repair/file/temp.rs:104-111`, `src/repair/file/inplace.rs`, `delete_deprecated` all `?` the stat. |
| P-3 derived audio-move log | yes | `src/repair/phases/post.rs:354` logs the derived move. |
| P-4 `remove_dir` failure logged | yes | `src/repair/purge.rs:264` logs why an emptied directory was not removed. |

---

## 3. Findings

### R-1 — Medium — `--plan-out` is silently dropped unless `--print-plan` is also set

`src/cli.rs:177-181` promises the flag works on its own:

```rust
/// Also write the per-file plan detail to this file as JSON
/// `{repair:[{rel,size,chunks_total,est_download_bytes}],purge:[{rel,size}]}`.
/// Implies nothing: use it with `--print-plan`, or on its own to dump the
/// plan of a run that does the work.
#[arg(long, value_name = "FILE")]
pub plan_out: Option<PathBuf>,
```

`README.md:301` says the same (`--plan-out <FILE>` | `(none)` | "Write the
per-file plan as JSON …"). But the only write site is inside the `print_plan`
early return, `src/repair/mod.rs:311` … `:327-336`:

```rust
if ctx.print_plan {
    let disk = scope::report::DiskPlan::new(&game_dir, &plan, ctx.repair_mode, ctx.jobs);
    let totals = /* … */;
    report::emit(&scope::report::plan_record(&totals), ctx.output);
    if let Some(out) = ctx.plan_out.as_deref() {
        let doc = scope::report::PlanDocument::new(/* … */);
        doc.write_to(std::path::Path::new(out))
            .map_err(|e| RunFailure::write(e.context("write --plan-out")))?;
    }
    return Ok((summary, 0));
}
```

`git grep plan_out src/` finds no other write site (only the field definitions
and a test helper name). So `girpr --plan-out plan.json` (no `--print-plan`)
runs the repair, exits `0`, and never creates `plan.json`; a caller polling the
file cannot tell the run ignored the flag.

Why this fails the standard: a documented output is silently omitted, and a
missing artifact is exactly the kind of error the run must not hide. It is a
bug in the documented contract, not a documentation typo, because the help text
explicitly names the standalone use.

Test gap: every `--plan-out` test also passes `--print-plan`
(`tests/repair_offline.rs:1047-1096`, `:2454-2495`; `tests/cli_contract.rs`), so
the standalone path has no coverage.

Suggested fix: move the `PlanDocument` write out of the `if ctx.print_plan`
block — `DiskPlan`, `PlanDocument`, and `est_download_by_file` are already in
scope at `:311` — and write it before the early return when `print_plan` is set
and at the end of the execution path otherwise (or once before the branch,
accepting one extra `stat` pass). Add a test that passes `--plan-out` alone and
asserts the file exists with the documented shape.

### R-2 — Medium — audio scan-file read conflates absent, unreadable, and empty

`src/repair/audio.rs:46-53`:

```rust
None => {
    let mut langs = read_scan_file(game_dir, scan_dir);
    if langs.is_empty() {
        langs.insert("en-us".to_string());
    }
    tracing::info!("keeping current audio langs: {langs:?}");
    langs
}
```

and `read_scan_file` (`:61-91`) returns an empty set for three different
conditions: an empty `scan_dir` (`:63-65`), a path that escapes the game dir
(`:66-72`), and any read error (`:73-75`, `let Ok(text) = ... else { return set; }`).
An existing-but-empty file also parses to an empty set.

Concrete failure: `--audio none` writes an empty file on purpose
(`write_scan_file`, `:93-107`, "An empty set writes an empty file, which is how
the game is told to launch game-only"), and `docs/02-repair-chunk-spec.md:55-61`
documents empty = game-only. The next invocation *without* `--audio` calls
`resolve_effective_audio` with `explicit = None`, reads the empty file, and
defaults to `en-us` — reversing the user's game-only choice and re-planning the
`en-us` audio manifests. The same happens silently when the file is present but
unreadable (permissions, a directory in its place, a transient sharing
violation): README:244 says "default `en-us` if unreadable", but nothing warns,
so the user cannot tell a deliberate default from a failed read.

The consequence is not only extra work. `purge_plan` is built from
`plan.expected_paths()` (`src/repair/purge.rs:88`), and the membership test is
`if expected.contains(rel) { PurgeVerdict::Keep }` (`:348`). Selecting the
wrong audio set changes what the purge considers expected, so under
`--purge-after` the audio packages belonging to the real selection can be
classified as purge candidates and deleted, while the wrongly selected ones are
re-downloaded. That needs the read to fail (or a prior `--audio none`), which is
why the severity is Medium rather than High.

`write_scan_file` is also a plain `std::fs::write` (`:105`) with no temp+rename,
so a crash or full disk mid-write can leave a truncated file that reads as
game-only.

Test gap: `resolve_effective_audio` and `read_scan_file` have no unit tests that
distinguish absent / unreadable / empty; the only end-to-end audio test
(`tests/repair_offline.rs:1428-1468`) covers `--audio none` itself, not the
following unflagged run.

Suggested fix: make the read three-valued — `Ok(Some(set))` for a successful
read (including empty = game-only), `Ok(None)` for "not configured/absent"
(→ `en-us`), and `Err` for unreadable (propagate, or warn and keep the previous
selection). Write the scan file atomically (temp in the same directory +
rename). Add tests for the empty-then-unflagged sequence and for an unreadable
file.

### R-3 — Low — rollback error masks the chunk error it was cleaning up after

`src/repair/file/temp.rs:118-130`:

```rust
ChunkOutcome::Download => {
    if let Err(e) = write_downloaded_chunk(ctx, prefix, c, &mut f, stats, writes).await
    {
        // … truncating back to the chunk's offset makes the resume honest …
        rollback_tmp(&mut f, tmp_path, c.offset.max(0) as u64)?;
        return Err(e);
    }
}
```

If `rollback_tmp` fails (its own `stat` at `:174-177` or `set_len` at `:181`),
the `?` returns the rollback error and `e` — the actual download failure — is
dropped. The user sees "stat tmp … before rollback" or a `set_len` error and
never learns which chunk failed or why. The rollback failure is the more urgent
condition (the `_tmp` may now be left over-long and will be trusted by a
length-based resume), but both facts belong in the report.

Fails the standard: an error is omitted, and the reported cause is the
cleanup, not the fault.

Suggested fix: wrap the rollback error and keep the original as the source,
e.g. `rollback_tmp(...).map_err(|re| re.context(format!("roll back tmp after chunk failure ({e})")))?`,
or log the rollback failure with `e` attached and return `e`.

### R-4 — Low — `discard` mode hides the delete failure

`src/repair/file/inplace.rs:155-158`:

```rust
if ctx.repair_mode == RepairMode::Discard {
    drop(handle);
    std::fs::remove_file(final_path).ok();
    handle = open_rw(final_path).with_context(|| format!("create {}", final_path.display()))?;
```

A read-only target, an ACL denial, or a sharing violation makes `remove_file`
fail, and `.ok()` discards it. The following `open_rw` then fails and the run
reports `create <path>: access denied` — pointing at the wrong syscall — or,
where the open succeeds (e.g. the file is writable but not deletable), the
"discard" contract is silently broken and the old bytes stay in place while the
code sets `scan = util::Scan::absent(...)` (`:161`) and treats every chunk as
missing. The repair still converges (every chunk is rewritten), but the mode's
documented zero-reuse guarantee is not what happened.

Fails the standard: the failure that matters (the delete) is dropped, and the
diagnostic names a later step.

Suggested fix: propagate with context,
`std::fs::remove_file(final_path).with_context(|| format!("discard {}", final_path.display()))?`,
and only fall back to truncation if that is an intentional, logged behaviour.

### R-5 — Low — `config.ini` fsync failure is swallowed before publish

`src/util.rs:688-696`:

```rust
let tmp = path.with_extension("ini.girpr_tmp");
{
    let mut f = File::create(&tmp)?;
    f.write_all(out.as_bytes())?;
    if !test_no_fsync() {
        f.sync_all().ok();
    }
}
std::fs::rename(&tmp, &path)?;
```

`write_all` returning `Ok` means the bytes reached the page cache, not the
disk. If `sync_all` fails (I/O error, full device), the code still renames the
temp over `config.ini`, so a power loss can publish a `config.ini` whose
`game_version=` line is absent or torn — and the version bump is the record that
the install is up to date. The failure is exactly the durability condition the
call exists to establish, and it is discarded.

Fails the standard: an error is dropped on a path whose whole purpose is
durability.

Suggested fix: `f.sync_all().with_context(|| format!("sync {}", tmp.display()))?;`
(keeping the test-only `test_no_fsync()` bypass), and remove the temp on the
error path if the caller can do so.

### R-6 — Low — sibling readers treat every read error as "absent", unlogged

`src/util.rs:444-450`:

```rust
pub fn read_game_version(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("config.ini")).ok()?;
```

`:453-457` (`read_ignore_categories`) and `:476-480` (`read_blacklist`) repeat
the pattern (`let Ok(text) = ... else { return set; }`). A permissions error,
a directory where a file is expected, or a transient sharing violation is
indistinguishable from "the file is not there".

Concrete failure: an unreadable `blacklist` (or `res_category` ignore file)
returns an empty set, so the run repairs files the user excluded — extra
download and write work, not data loss, because the blacklist only narrows what
is repaired. An unreadable `config.ini` makes `read_game_version` return
`None`, which the version logic treats as a fresh install; the following bump
rewrites the version. The happy path is correct; the failure path silently
changes behaviour with no warning.

Fails the standard: a real error is reported as absence. The docs do not promise
a warning here, but "no omitted errors" means an unreadable configuration file
should at least be logged, and the three readers should agree with the audio
reader (R-2) on what "absent" means.

Suggested fix: return `Result<Option<T>>` (or log at `warn` with the path and
`io::ErrorKind` before returning the empty/default value). Add one test per
reader for `NotFound` vs. an injected read error (a directory at the path is the
portable way to force `InvalidInput`/`IsADirectory`).

---

## 4. Verification

All commands were run in `E:\hoccode\hocrust\girpr` on the reviewed tree
(`fix-gaps/girpr`, HEAD `e88b62a`), Windows host, default (dev) profile.

| Command | Result |
|---|---|
| `cargo build` | ok — `Finished dev profile … in 40.08s`, no warnings |
| `cargo test -p girpr` | **311 passed, 0 failed, 7 ignored**. 178 lib + 7 `artifact_server` + 16 `cli_contract` + 25 `repair_machinery` + 85 `repair_offline`; 0 `main`, 0 doc-tests. Ignored: 6 `artifact_bridge`, 1 `peak_heap` |
| `cargo clippy -p girpr --all-targets` | clean — no warnings, `Finished dev profile … in 14.57s` |
| `cargo fmt --check` | clean — exit `0`, no diff |

The total of 311 passing tests matches the fourth pass, so this tree introduces
no test regression.

## 5. Not verified

- **R-1 and R-2 were not executed end to end.** Both are source-level claims:
  R-1 from the single `plan_out` write site inside `if ctx.print_plan`, R-2 from
  the three-way collapse in `read_scan_file` plus the `purge` membership rule.
  No binary run created a plan file without `--print-plan`, and no run wrote an
  empty audio file and then re-ran without `--audio`.
- **No live CDN / API run.** No manifest, chunk, or audio package was fetched;
  the audio and purge claims are about selection, not about downloaded bytes.
- **No release-profile run and no Linux run.** `#[cfg(unix)]` tests were skipped
  on this Windows host; the `cfg(unix)` durability paths (R-5) were not
  exercised.
- **7 tests ignored** by the suite (6 `artifact_bridge`, 1 `peak_heap`) were not
  run here; they need a live artifact server or a heap-measuring target.
- **R-3, R-4, R-6 were not fault-injected.** The failure scenarios are read
  from the control flow; no read-only target, failing `sync_all`, or unreadable
  config file was constructed to observe the reported symptom.
- The known fourth-pass residuals are unchanged and not re-litigated here:
  `expected_paths()` / `classify_purge_path` case sensitivity on Windows,
  F-6 (log-only scan panic), the dry-run `validate_chunk_ranges` skip, and the
  absence of a release-profile run.
