# ADVERSARIAL REVIEW — `girsync` (`tools/sync`)

Review of the `girsync` package in workspace `E:\hoccode\hocrust\girpr`:
source under `src/`, tests under `tests/`, shared leaf crates `crates/log` and
`crates/tempdir`, `docs/` claims, `Cargo.toml`/`Cargo.lock`, and the clap-derived
CLI surface.

**Method.** Read every source file, every test file, the README, and `docs/`;
ran `cargo build -p girsync` (clean) and `cargo nextest run -p girsync`
(326 passed, 0 failed, 0 ignored); probed several edge cases by running the
binary against scratch trees (case-only compare, `--hash-all-of none
--no-trust-size`, symlink/skeleton fixtures, `--max-depth` deep files).

Each finding is marked:

- **(a) bug** — incorrect or unexpected behavior, even if documented as
  intentional.
- **(b) intentional behavior lacking test coverage** — behaves as designed but
  not pinned by a test; recommend a pinning test (not written here, per the
  read-only constraint).

No code or tests were modified. The report is ready for human review; no PR or
commit was created.

---

## Summary table

| # | Class | Severity | Location | Finding (one line) |
|---|-------|----------|----------|--------------------|
| G-1 | (a) bug | medium | `src/scan.rs:74-94` vs README | Symlink handling is documented as "followed/materialized" but `walk_live` bails on anything that is not `is_dir`/`is_file`; unverifiable here (symlinks need admin), untested |
| G-2 | (b) intentional | medium | `src/commands/sync/apply/copy.rs:119-151` | `copy_one` follows dst symlinks — **decision: follow symlink per doc**; test may not be runnable where symlinks can't be created (environment-gated) |
| G-3 | (b) intentional | medium | `src/commands/sync/apply/phases.rs:16-50` | A single apply-phase failure (one bad file, one `unsupported type`) aborts the whole run with exit 3 and leaves a partially mirrored dst; fail-fast is documented only for input validation |
| G-4 | (b) intentional | low | `src/scan.rs:38`, README gotchas | `--max-depth` (default 10) silently skips deeper files; `sync` never mirrors or warns about them |
| G-5 | (b) docs | low | `src/diff/mod.rs:78-91,318-323` | `Diff::total()` excludes `case_mismatch` but `is_empty()` includes it: a case-only compare reports `total_diff=0` yet exits 4 |
| G-6 | (b) intentional | low | `src/commands/update.rs:73-94` | `update` warns and stores **all** algorithms for `--hash-any-of` (never picks one) |
| G-7 | (b) intentional | low | `src/effective/side.rs:143-173` | Non-dry-run `compare` backs up both caches (`backup_first=true`) as a side effect of opening them |
| G-8 | (b) intentional | low | `src/commands/compare_self.rs:122-128` | `compare-self --ignore-cache` bails; `--dry-run`/`--no-trust-cached-hashes` are accepted + warned |
| G-9 | (b) intentional | low | `src/config/common.rs:257-279` | `--hash-all-of none` + `--no-trust-size/--no-trust-mtime` is refused with exit 3 (verified); correct but only covered at unit level |
| G-10 | (a) bug | low | `src/commands/sync/apply/copy.rs:147-151` | `copy_one` preserves src mtime via `set_modified` after the write; if that fails the whole run aborts even though the bytes are already correct |
| G-11 | negative | — | — | Path traversal via `..`/absolute `rel` is **not** a girsync exposure: every `rel` is produced internally by disk walks (`scan.rs`) or cache keys, never supplied by the user |

---

## Detailed findings

---

### G-1 (a) bug, medium — Symlink behavior contradicts the README and is untested

**Behavior.** `docs`/README state: "Symlinks are followed … Link targets outside
the root are followed and materialized as plain files/dirs. Dirs compare by
presence only." (`README.md:1004-1006`). But `walk_live` only pushes entries
that are `ft.is_dir()` or `ft.is_file()` and otherwise executes
`bail!("unsupported file type …")` (`src/scan.rs:74-94`). walkdir's
`file_type()` does not follow symlinks, so on Windows a symlink (file or dir)
is neither `is_dir` nor `is_file` and the walk aborts the entire run. That is
the opposite of the documented "followed and materialized" behavior. A
dangling symlink *does* abort via walkdir's error arm (`scan.rs:43`), which is
consistent with the docs, but a *valid* symlink is handled neither as the docs
promise nor consistently.

Because symlink creation requires administrator privileges on this machine I
could not confirm the runtime behavior; the finding rests on code reading plus
the documented claim. Either the code is wrong relative to the docs, or the
docs are wrong relative to the code — in both cases the behavior is
uncovered by tests.

**Evidence.** `src/scan.rs:32-113` (walk), `src/scan.rs:74-94` (dir/file
branches + `bail!`), `README.md:1004-1006` (docs claim).

**Severity.** Medium: any symlink in `src` or `dst` can abort a `sync`/`compare`
run (exit 3) contrary to the documented feature.

**Recommended action.** Verify symlink behavior on a machine where symlinks can
be created, then either (a) implement per-docs handling (follow dir symlinks,
materialize file symlinks, abort on dangling) or (b) correct the README to
state symlinks abort the run. Add pinning tests for: file symlink, directory
symlink, dangling link (abort), and a symlinked subtree being walked.

---

### G-2 (b) intentional, medium — `copy_one` follows `dst` symlinks: write outside the mirror root

**Behavior.** `copy_one` opens `dst` with `std::fs::File::create(&d)`, restores
mtime via `OpenOptions::write(true).open(&d)`, and verifies via `hash_file(&d)`.
All three follow symlinks on Windows. If `dst` contains a symlink at a path
that `src` wants to copy to (a `MISSING`, `CHANGED`, or `type-conflict` copy),
the write lands on the symlink's target, which may live **outside** `dst`.
The README sanctions this ("link targets outside the root are followed and
materialized"), so it is intentional, but it is a footgun: a stale or hostile
symlink in the working dir causes `sync` to write outside the tree it is
supposed to mirror. `delete_extras`/`fix_dirs` use `remove_file`/`remove_dir_all`,
which remove the link itself (not the target), so the asymmetry is real: copies
follow, removals do not.

**Evidence.** `src/commands/sync/apply/copy.rs:119-151` (`copy_one`),
`src/commands/sync/apply/phases.rs:28-38` (delete/fix use remove_* which do not
follow for removal), `README.md:1004-1006`.

**Severity.** Medium: write outside `dst` driven by a symlink in `dst`.

**Recommended action.** Follow symlink per doc: `copy_one` follows `dst`
symlinks and writes to the link target (may be outside `dst`), matching the
README ("link targets outside the root are followed and materialized"). Note
that a pinning test may not be runnable in environments where symlinks cannot
be created (e.g. this machine without admin privileges), so the test should
be conditional or documented as environment-gated. Add a README note that
symlinks in `dst` are followed on write.

---

### G-3 (b) intentional, medium — One apply failure aborts the whole mirror (partial dst)

**Behavior.** The apply phases run in sequence; `delete_extras`,
`copy_files`, `remove_unknown_dirs`, and `predrop_cache_entries` all propagate
the first `Err` (`?`), so one unreadable source file, one permission failure,
or one `unsupported type` (`phases.rs:36-38`) aborts the run with exit 3
(`main.rs:36-39`) and leaves `dst` partially mirrored. Completed copies have
their cache rows written, so a *rerun* naturally resumes them (stat+digest
settles them), but there is no explicit resume and no per-failure isolation.
The README's "first error aborts the run" statement (`README.md:721`) only
covers input validation, not the apply phase.

**Evidence.** `src/commands/sync/apply/phases.rs:16-50`,
`src/commands/sync/apply/copy.rs:124-141` (copy error aborts),
`src/main.rs:34-42` (Err → exit 3).

**Severity.** Medium: a single bad file fails an entire mirror and leaves a
partially-applied dst.

**Recommended action.** Keep as intentional fail-fast (documented design
tension: crash-safety vs. all-or-nothing), but clarify in the README that a
single apply failure aborts with exit 3 and a rerun resumes completed copies.
Optionally recommend a pinning test: one unreadable src file among many → exit
3, the other files still copied, rerun completes the rest. Not written here.

---

### G-4 (b) intentional, low — `--max-depth` default 10 silently skips deeper files

**Behavior.** `walk_live` applies `max_depth` (default 10) to the whole walk
(`src/scan.rs:38`). Files and directories deeper than the cap are simply
absent from the effective map, so `sync` never mirrors them and `compare`
never reports them — no warning, no error. The README lists this as a gotcha
("`--max-depth 10` silently skips deeper files — set higher for real game
trees", `README.md:1095`) and there is a unit test for the depth cap
(`src/scan.rs:176-192`), but no end-to-end test asserts that a deep file is
skipped by `sync` rather than mirrored.

**Evidence.** `src/scan.rs:32-113`, `README.md:1095`.

**Severity.** Low: silent data loss for deep trees; documented and configurable.

**Recommended action.** Leave as intentional; recommend a pinning end-to-end
test: a file at depth > 10 is not mirrored by `sync` and the run exits 0
without error.

---

### G-5 (b) docs, low — `Diff::total()` vs `is_empty()` disagree on `case_mismatch`

**Behavior.** `Diff::total()` (the exit-4 input for `compare`) counts
`missing + extra + changed + type_conflict` and deliberately excludes
`identical` and `verdicts` (`src/diff/mod.rs:78-80`). But `Diff::is_empty()`
adds `case_mismatch.is_empty()` (`src/diff/mod.rs:89-91`). So a case-only
difference (same content, different casing) reports `total_diff=0` in the
`SUMMARY` yet `compare` exits 4 because `is_empty()` is false. That is
consistent with "exit 4 if any difference exists" (casing is a difference for
`compare`), but `total()`'s doc comment ("Number of differences that force a
nonzero exit") is inaccurate for `case_mismatch`, and a consumer keying off
`total_diff==0` to mean "nothing to do" would be wrong for `compare`.
`sync` is unaffected (it exits 0 always and renames before diffing).

**Evidence.** `src/diff/mod.rs:78-91,318-323`, `src/commands/compare.rs:169`.

**Severity.** Low: documentation/assertion asymmetry; no incorrect exit code.

**Recommended action.** Clarify the doc comment on `Diff::total()` (and the
README) that `case_mismatch` is a difference for exit-code purposes even though
it is excluded from `total()`. Optional pinning test: case-only `compare`
exits 4 with `total_diff=0` and `case_mismatch=1`.

---

### G-6 (b) intentional, low — `update` never honors `--hash-any-of`

**Behavior.** `update` accepts `--hash-any-of`, warns that it does not change
what `update` does, and stores **every** named algorithm (`src/commands/update.rs:73-94`).
This is documented and deliberate (there is no counterpart to choose between),
but a script that passes `--hash-any-of` to every subcommand might expect
`update` to store only one. The warning is the only signal.

**Evidence.** `src/commands/update.rs:73-94`, `README.md:194-196`.

**Severity.** Low.

**Recommended action.** Leave as intentional; recommend a pinning test that
`update --hash-any-of md5 sha256` stores both md5 and sha256 (warns, exit 0).

---

### G-7 (b) intentional, low — Non-dry-run `compare` backs up both caches

**Behavior.** `compare` opens each folder side through `open_folder_cache(…,
backup_first: true)` (`src/effective/side.rs:172-179`, called via
`src/commands/compare.rs:91-98`), so every non-dry-run `compare` creates
`girpr-cache-backup-<ts>` files for both sides before doing any comparison.
This is a filesystem side effect of a command whose README intro says it
"never modifies the two trees" (true for the trees, but not for the caches).
Dry-run avoids it (`src/effective/side.rs:181-186`).

**Evidence.** `src/effective/side.rs:143-187`, `src/commands/compare.rs:91-98`.

**Severity.** Low: backup files accumulate; documented only obliquely.

**Recommended action.** Keep as intentional; document in the README that a
non-dry-run `compare` writes (and backs up) both folder caches, so "audit and
leave both caches byte-identical" requires `--dry-run`.

---

### G-8 (b) intentional, low — `compare-self` rejects `--ignore-cache`; inert flags warned

**Behavior.** `compare-self --ignore-cache` bails ("it would delete the cache
being compared") (`src/commands/compare_self.rs:122-128`). `--dry-run` and
`--no-trust-cached-hashes` are accepted with a warning and do nothing
(`src/commands/compare_self.rs:102-120`). All documented, but only the bail has
a direct test path via the config/CLI; the warning behavior is not pinned at
the binary level.

**Evidence.** `src/commands/compare_self.rs:102-128`, `README.md:492-493`.

**Severity.** Low.

**Recommended action.** Leave as intentional; recommend pinning tests for the
two warnings and the `--ignore-cache` bail (exit 3).

---

### G-9 (b) intentional, low — `--hash-all-of none` + distrusted stat refused (exit 3), verified

**Behavior.** `CommonOpts::try_from` bails when no algorithm is requested and a
stat field is distrusted, because the pair would have nothing to fall back on
and could be reported equal without a read (`src/config/common.rs:257-279`).
Verified by running the binary: exit 3 with a clear message. Correct behavior,
but the refusal is only exercised at the unit/config level, not through the
binary in the integration suite.

**Evidence.** `src/config/common.rs:217-296`, verified exit 3.

**Severity.** Low: correct refusal; prevents the "changed file reported equal"
failure.

**Recommended action.** Leave as intentional; recommend a CLI-level pinning
test: `--hash-all-of none --no-trust-size` → exit 3, no stdout diff records.

---

### G-10 (b) docs, low — `copy_one` mtime restore failure aborts an otherwise-good copy

**Behavior.** `copy_one` takes src's mtime *before* copying, writes the bytes,
then calls `set_modified` on dst (`src/commands/sync/apply/copy.rs:147-151`).
If `set_modified` fails (permissions, read-only dst), `copy_one` errors and the
whole run aborts (G-3) even though the bytes were written correctly and the
verify would have passed. Documented as "preserve mtime" but the failure mode
is not called out.

**Evidence.** `src/commands/sync/apply/copy.rs:147-151`.

**Severity.** Low.

**Recommended action.** Document that mtime restoration failure aborts the copy;
optional pinning test not easily constructed without permission manipulation.

---

### G-11 (negative) — Path traversal via `..`/absolute paths is not a girsync exposure

**Behavior.** Every relative `rel` handled by girsync is produced internally:
`walk_live` derives it via `strip_prefix(root)` (`src/scan.rs:53-59`), cache
keys are stored relative paths, and `copy_one`/`delete_extras` join only
internally-produced `rel`s with `src`/`dst` roots. The user supplies only the
root directories (`--src`/`--dst`). There is no manifest-supplied path like
girsync's sister project `girpr`, so the `../../escape/evil.exe` class of
finding (girpr F-2) does not apply here. `validate_inputs` additionally rejects
record paths and non-directories and enforces one-cache-per-run
(`src/commands/sync/mod.rs:385-395`).

**Evidence.** `src/scan.rs:53-59`, `src/commands/sync/mod.rs:385-395`,
`src/util.rs:51-56` (`is_record_path`).

**Severity.** None — noted to rule the class out explicitly.

---

## Additional observations (not findings)

- **Exit-code semantics are consistent and documented.** `0` ok/equal, `2`
  clap parse error (clap self-exits via `Cli::parse()`, before `main`), `3`
  fatal at runtime/config (verified: `--hash-all-of none --no-trust-size`
  → exit 3), `4` compare found a diff. `sync` always exits `0`
  (`src/main.rs:13-42`, `src/commands/sync/mod.rs:381-382`).
- **`--dry-run` contract holds structurally.** `open_folder_cache` is the
  single place a folder cache is opened (`src/effective/side.rs:143-187`);
  under `dry_run` it opens existing caches read-only and serves missing ones
  from memory, so no `girpr-cache` file is left behind and no write occurs.
- **`compare-self` is a real audit.** The disk side is scanned against an
  empty in-memory cache (`src/commands/compare_self.rs:177`), the record is
  read-only, and `ensure_distinct_sides` blocks the two-sided spelling that
  would let the run repair the subject it audits.
- **Coverage check closes the "silent clean" gap.** `plan_pairs` fails
  (`Coverage`, exit 3) when a record side cannot supply a digest for an
  undecided pair, so `hashes_differ`'s silence on a missing digest can never
  be mistaken for agreement (`src/planner/pairs.rs:165-191`,
  `src/planner/coverage.rs:11-12`).
- **No resume beyond re-run.** `sync` has no checkpoint/resume; completed
  copies are cached so a rerun skips them, but an interrupted run leaves dst
  as-is (`README.md:735`). This is documented.
- **`crates/log` / `crates/tempdir`** are shared leaf crates; their behavior
  (log-file precedence, temp-dir naming/cleanup) is consistent with how girsync
  consumes them (`src/logging.rs`, `src/cache/db/backend.rs`). No findings in
  the copies themselves relevant to girsync beyond what is exercised here.

---

## Recommended actions (summary)

| Priority | Action |
|----------|--------|
| High | Verify and resolve the symlink discrepancy (G-1); decide documented vs actual behavior and add pinning tests once symlinks can be created in CI. |
| Medium | Document the whole-run abort / partial-mirror semantics (G-3). G-2 decided: follow symlink per doc (test environment-gated). |
| Low | Clarify `Diff::total()` vs `is_empty()` on `case_mismatch` (G-5); document `--max-depth` silent skip (G-4) and `compare` cache backups (G-7); add CLI-level pinning tests for G-6/G-8/G-9. |
| None | Path-traversal class (G-11) does not apply to girsync; no action. |

No code or tests were modified in this review. The report is ready for human
review; no PR or commit was created.
