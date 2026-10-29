# Adversarial review note — eleventh pass

**Tree:** `fix-gaps/girpr`, HEAD `ea2445b` (`docs(review): fold the tenth pass into the review`), clean.
**Scope:** `src/**`, `tests/**`, `docs/**`, root `README.md`, root `Cargo.toml` (root crate only; `tools/sync`, `tools/capture`, `crates/log`, `crates/tempdir` excluded except as dependencies).
**Baseline:** `main`; the pass re-read the whole tree, then concentrated on the tenth-pass fixes (`fe0934f`, `ec0a79d`, `3ce91a3`) and on the readers/estimators those fixes touched.

## Verdict

The tenth-pass fixes are correct and still wired:

- **V-1** (blacklist as a purge keep input) is passed to `plan_purge` at both call sites (`src/repair/mod.rs:177`, `src/repair/phases/post.rs:60-66`) and `classify_purge_path` keeps a blacklisted rel at rule 3 (`src/repair/purge.rs:381-384`). The integration test `blacklisted_files_survive_every_purge_spelling` pins all three purge timings and a stale orphan still purges.
- **V-2** (`read_blacklist` cleans entries with `util::clean_rel`, refusing uncleannable ones) is correct (`src/util.rs:546-559`).
- **V-3** (`build_local_chunk_map` keys on `util::clean_rel`) is correct (`src/sophon.rs:674-707`).

The binding decisions remain consistent with the code. The new findings are **not** regressions in those three fixes; they are gaps the tenth pass left in the documentation it did not update (V-1 changed behaviour that four documents still describe the old way), plus two independent estimator/reader gaps found while re-reading the touched areas.

## Findings

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| E-1 | Medium | fail-open / boundary validation | `src/util.rs:519-530`, `src/util.rs:550-554`, `src/util.rs:590-595` | A parseable JSON line whose entry field is missing or wrong-typed is dropped silently; an explicit blacklist/ignore entry can protect nothing. |
| E-2 | Low | wrong estimate / doc mismatch | `src/repair/scope/report.rs:94-100`, `src/repair/file/mod.rs:548-561` | `DiskPlan` models `--repair-mode discard` as in-place growth, so `PLAN est_extra_bytes` and `--plan-out extra_bytes` report a positive number where the documented peak is 0. |
| E-3 | Low | omitted error / stat state | `src/repair/scope/report.rs:92` | `metadata().ok()` treats an unreadable stat as an absent file, overstating the disk estimate and logging nothing. |
| E-4 | Low | stale documentation | `README.md:234-235`, `docs/01-repair-findings-and-verdict.md:140-142`, `docs/03-repair-rust-implementation-plan.md:98`, `src/repair/purge.rs:304-317` | The purge expected/keep-set docs still omit the blacklist keep added by V-1 (and `keep_set`'s doc says "config.ini ONLY" while returning the lock too). |
| E-5 | Low | stale documentation | `src/config.rs:182-186` | The `readonly()` doc says the metadata phase writes the audio-scan file; R-7 moved that write to execution. |

### E-1 — A parseable but malformed config-list line is a silent no-op

**Evidence.** `parse_config_list` splits three outcomes: a line that is not JSON is warned and recorded in `first_failure` (`src/util.rs:596-604`); a valid but entryless line (`{}`) is intentionally empty; a valid JSON object whose entry closure returns `None` is silently discarded (`src/util.rs:590-595`). Both readers use a closure that returns `None` for a missing/wrong-typed field:

- blacklist: `v.get("fileName").and_then(|n| n.as_str())` (`src/util.rs:550-554`);
- ignore categories: `v.get("is_delete").and_then(|b| b.as_bool()).unwrap_or(false)` followed by `v.get("category").and_then(|c| c.as_str())` (`src/util.rs:519-530`).

If every line is of that shape, `set` is empty and `first_failure` is `None`, so the reader returns `Ok(Some(empty))` — an empty configuration with no warning. This is the same fail-open T-1 fixed for **unparseable** lines, but T-1's documented "warning per bad line" does not hold for a **parseable** line with a malformed entry.

**Failure scenario.** A blacklist file contains `{"fileName": 123}` (or `{"FileName":"mods/x.pak"}`, or `{"fileName": null}`). `read_blacklist` returns an empty set. `build_plan` therefore includes the path if the live manifest names it, and the repair overwrites the file the user meant to exclude; if the path is not in the live manifest, `--purge-after` deletes it. An ignore file with `{"category":"10302","is_delete":"true"}` (string instead of bool) silently fails to ignore the category, so the repair rewrites a category the user excluded. The same shape applies to an all-malformed file: it reads as empty instead of exit `1`.

**Standard violated.** Strict validation at a boundary; an explicit configuration must not degrade silently; no omitted error that changes what the run repairs/keeps. T-1's OPEN item says a mixed file "warn[s] per bad line"; a parseable-but-wrong-typed line is a bad line and is not warned.

**Suggested fix.** Give `parse_config_list` a tri-state entry result (`Use(String)`, `Skip`, `Invalid(String)`) or make the closure return `Result<Option<String>>`, and warn/refuse on `Invalid` with the file and 1-based line. Per reader: blacklist requires a string `fileName`; ignore categories treats `is_delete: false`/absent as `Skip`, a non-bool `is_delete` as `Invalid`, and `is_delete: true` with a missing/non-string `category` as `Invalid`. Add one unit test per shape (wrong-typed blacklist field; wrong-typed `is_delete`) and keep the existing `{}`/blank-file cases empty.

### E-2 — `--repair-mode discard` is estimated as in-place growth

**Evidence.** `DiskPlan::new` matches only `Mode::Temp` and `Mode::Inplace` (`src/repair/scope/report.rs:94-100`). It obtains the mode from `resolve_mode(requested, &path)`, which special-cases only `RepairMode::Temp`; `RepairMode::Discard` falls through to the metadata check and returns `Mode::Inplace` (`src/repair/file/mod.rs:548-561`). The actual discard path deletes the target before writing (`src/repair/file/inplace.rs:151-157`), so its peak extra disk is 0, as the README table says (`README.md:292`: "`discard`: delete the target and write it from scratch (0 extra disk, a full download)").

**Failure scenario.** A planned file is 100 bytes on disk but 40 bytes present. `--print-plan --repair-mode discard` reports `est_extra_bytes=60` (and `--plan-out` says `extra_bytes=60`), while the run deletes the 40-byte target and writes 100 bytes, needing 0 extra bytes beyond the install. A user sizing a volume from the `PLAN` line is told they need 60 bytes they do not. The figure is conservative, so it cannot corrupt anything, but it is the wrong answer to the question the line exists to answer.

**Standard violated.** Decision 4 ("the scanned estimate models the write path and the resolved mode"); the README's documented discard disk figure.

**Suggested fix.** In `DiskPlan::new`, special-case `RepairMode::Discard` (or have `resolve_mode` return a discard-aware variant) so `extra` is 0; keep `est_write_bytes = size` because the whole file is rewritten. Add a unit test next to `the_disk_plan_says_what_each_file_costs` that checks `DiskPlan::new(..., RepairMode::Discard, ...)` on a partially present file.

### E-3 — `DiskPlan` reads a failed stat as an absent file

**Evidence.** `let on_disk = std::fs::metadata(&path).ok().map(|m| m.len());` (`src/repair/scope/report.rs:92`). `None` then means "the file is not there", and the in-place branch computes `size.saturating_sub(on_disk.unwrap_or(0))` (`src/repair/scope/report.rs:100`). There is no log and no distinction between `NotFound` and a permission/sharing error. This is the shape T-2 fixed for `--check-only` and the temp path (`util::file_len` keeps `Present`/`Absent`/`Unreadable` distinct, `src/util.rs:289-300`).

**Failure scenario.** A `--print-plan` (or a run with `--plan-out`) sees a file whose `metadata` call fails with an ACL or sharing violation. The plan treats it as absent and reports the file's full size as growth (`est_extra_bytes` and the document's per-file `extra_bytes`), with no warning. The repair later fails that file (exit `3`), but the preview claimed a disk requirement it never measured and gave the reader no reason to look at the file.

**Standard violated.** Decision 9 ("a stat failure is never a zero and never 'not a file'"; absent is `NotFound` only); no omitted errors at a boundary.

**Suggested fix.** Use `util::file_len(path)`; on `FileLen::Unreadable(e)` warn with the path and error and treat the length as unknown (the safe estimate is the full size, but the warning is the point), on `Absent` use 0, and on `Present(n)` use `n`. Pin the classifier branch with a unit test; a deny-ACL fixture is not required if the warning is tested through a small helper.

### E-4 — Purge expected/keep-set docs still describe the pre-V-1 rule

**Evidence.** V-1 made the cleaned blacklist a `plan_purge` keep input. `docs/02-repair-chunk-spec.md:347-348` was updated ("∪ `{blacklisted paths}`"), but four other places still say the old set:

- `README.md:234-235`: "expected set = live-manifest paths + `config.ini` + the single-writer lock + `audio_lang_*`/`Audio_*_pkg_version` patterns; **everything else purges**";
- `docs/01-repair-findings-and-verdict.md:140-142`: "expected set = `{latest chunk-manifest paths} ∪ {config.ini}` plus filename-pattern keeps ... Everything else not in the live manifest purges";
- `docs/03-repair-rust-implementation-plan.md:98`: "v1 scope: expected = Sophon paths + `config.ini` only";
- `src/repair/purge.rs:304-317`: `keep_set`'s doc says "v1: `config.ini` ONLY" and "Everything else ... is intentionally NOT kept", while the function returns `config.ini` **and** `LOCK_FILE` (`src/repair/purge.rs:324-332`).

**Failure scenario.** A user reads the README and expects a blacklisted file to be deleted by `--purge-after`; it is kept. A maintainer reads docs/01/docs/03 and "fixes" the cleanup back to the documented set, reintroducing V-1. The `keep_set` doc is internally contradictory on the lock, so a maintainer cannot tell whether the lock keep is deliberate (it is: `the_lock_is_kept_by_the_cleanup` pins it).

**Standard violated.** Docs that overclaim / do not match behaviour; a comment that contradicts its own function.

**Suggested fix.** Add "∪ the cleaned blacklist (`meta.blacklist`)" to the README expected set, docs/01 and docs/03; change README's "everything else purges" to "everything not kept by rules 0-4"; change `keep_set`'s doc to "`config.ini` and the single-writer lock" and note that blacklisted paths are a separate keep input, not part of this set.

### E-5 — `readonly()` still says the metadata phase writes the audio scan file

**Evidence.** `src/config.rs:182-186`: "it does reach the metadata phase, whose audio-scan write is therefore gated on `!readonly() && !print_plan`." Since R-7 the metadata phase only **validates** the scan path (`src/repair/phases/metadata.rs:190-217`); the write is in execution at `src/repair/mod.rs:384-388`, gated on `ctx.audio_explicit && !readonly && !meta.config.audio_pkg_scan_dir.is_empty()`. `--print-plan` returns before execution (`src/repair/mod.rs:346-348`), so the write is unreachable there.

**Failure scenario.** A maintainer adding a new write site reads this comment, puts the gate in `metadata`, and believes the read-only story is complete; the actual write in execution is the one that must be gated. The comment also sends a reader looking for a write that is not there.

**Standard violated.** Docs/comments must not overclaim; the R-7 decision ("the write moved to execution").

**Suggested fix.** Reword to: "`--print-plan` returns before the execution phase, where the audio-scan write lives; `metadata` only validates the scan path under `!readonly() && !print_plan`. A new write site must ask `!readonly()` (and must be unreachable from the plan-only returns)."

## Verification

Run on Windows, debug profile, this tree (`ea2445b`):

| Command | Result |
|---|---|
| `cargo build` | ok — `Finished dev profile ... in 0.49s` |
| `cargo test -p girpr` | **346 passed, 0 failed, 7 ignored** (203 lib + 7 `artifact_server` + 17 `cli_contract` + 25 `repair_machinery` + 94 `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0 `src/main.rs`; 0 doc-tests) |
| `cargo clippy -p girpr --all-targets` | clean, no warnings — `Finished dev profile ... in 0.49s` |
| `cargo fmt --check` | exit `0`, no output |

The tenth-pass tests are green, including `blacklisted_files_survive_every_purge_spelling`, `blacklist_entries_are_cleaned_and_bad_ones_are_refused` and `local_chunk_map_keys_are_the_cleaned_plan_spelling`.

## Not verified

- **E-1** is source-read, not executed end-to-end. The offline fixture's blacklist is well-formed and no test supplies a parseable-but-wrong-typed `fileName`/`is_delete`; the claim rests on the `parse_config_list` control flow. A unit test would reproduce it without a mock server.
- **E-2** is source-read plus arithmetic; no existing test passes `RepairMode::Discard` to `DiskPlan`, and the discard integration test does not assert `PLAN`/`--plan-out` figures.
- **E-3** is source-read; no deny-ACL/sharing-violation fixture was available on this host, so the failed-stat branch was not exercised through `DiskPlan`.
- **E-4/E-5** are documentation-only; verified by reading the current files, not by a runtime assertion.
- The `#[cfg(unix)]` test blocks (containment, mode, `file_len`, file-as-parent) were not run on this Windows host; the 7 ignored tests were not run; no release-profile run; no live CDN traffic (all executed tests use the in-process mock).
- The broader unverified areas from the first pass (live manifest shapes, exhaustive panic sweep, `--print-plan --audio` under a scope) remain as recorded in `OPEN_FOR_MAINTAINERS.md`; this pass adds nothing to them.
