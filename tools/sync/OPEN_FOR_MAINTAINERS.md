# Open for maintainers — `girsync`

`girsync` (`tools/sync`), branch `fix-gaps/girsync`. These are deliberate trade-offs,
unverified areas and product decisions, not open defects; findings and their fixes are
in [ADVERSARIAL_REVIEW.md](ADVERSARIAL_REVIEW.md). Each item below gives the current and
historical behaviour, who depends on it, the real options, the code that would move, a
recommendation where one is genuinely inferable, and what was not verified. Claims are
grounded in `file:line`, a test name, a doc section or a commit hash; anything reasoned
rather than run is labelled.

Verification baseline: [ADVERSARIAL_REVIEW.md](ADVERSARIAL_REVIEW.md) records **428 passed,
1 ignored** (suite 429) on the eleventh pass. [README.md](README.md) still says `421 test
functions … 420 run` at line 1143, so treat the review as the current count and the README
line as stale documentation, not a behaviour claim.

## Suggested decision order

1. **Hard links** (§1) — the only item that can destroy data; confirm the plan-time
   refusal and the POSIX gap first.
2. **R11-1 cache prefix** (§4) and **R10-1 spelling lookup** (§3) — same platform rule;
   decide them together because a change to one invites the same change to the other.
3. **P7-1/P8-4 rename** (§6) — decide whether the two-step rename stays before touching
   the case-fix temporary.
4. **File links** (§5) and **end-to-end skips** (§2) — both are CI-host coverage, not
   product choices; settle with a job, not code.
5. **T-3/T-4** (§8), **failure injectors** (§7), **F-6/P6-1** (§9) — test depth; lower risk.
6. **Other gaps** (§10), **Windows junction mechanics** (§11), **`compare-self` warning**
   (§12) — documentation/observability, no data path.

---

## 1. Hard links

**Current behaviour.** A destination file with more than one link is refused at plan time,
in both apply modes, and re-checked at the copy site. `src/commands/sync/plan.rs:1025-1081`
computes the preflight over `LinkInfo { links, id, regular_file }`; `plan.rs:1063` gates it
on `regular_file`, so directories are never over-refused for their `nlink`; names the plan
deletes are credited first so a link the run itself removes does not block the copy.
`src/commands/sync/apply/copy.rs:252-269` repeats the check immediately before truncation.
Identity is `(dev, ino)` on Unix and `(volume serial, file index)` on Windows
(`src/hardlink.rs:67-147`), with `count()` as the single reader (`hardlink.rs:149-156`).

**History.** P6-2 closed a dst-hard-link write-through by refusing any destination with
more than one link (`b2b1445`, `53d113f`). P7-4 moved the decision to plan time (`53d113f`,
`374453d`). P8-1 narrowed it to regular files (`4dfb046`) because a directory's `nlink`
counts subdirectories, which had over-refused directories on POSIX and was invisible on
NTFS (a directory there reports one link).

**Usage and evidence.** `tests/sync.rs:403` (P6-2) and `:454`/`:512` cover the refuse and
the credited-name paths; `src/hardlink.rs:162-237` unit-tests the platform readers. The
user-facing promise is [README.md](README.md):1089-1096 ("refused at plan time in both
modes") and [DESIGN-NOTES.md](DESIGN-NOTES.md):208-233. No external consumer reads the
refusal; it is an internal safety gate.

**Options.**
- **Keep the plan-time refusal + regular-file gate** — pros: fails before any write,
  covers both apply modes, one identity source; cons: a legitimate multi-link destination
  must be broken by the user first; an over-broad `regular_file` definition could still
  over-refuse.
- **Refuse only at the copy site** — pros: less plan work; cons: a failure after other
  writes have landed, and no dry-run warning; rejected by P7-4.
- **Warn and proceed** — pros: no hard stop; cons: writes through to every other name,
  which is the P6-2 data loss; rejected.

**Code change.** None recommended. The POSIX `nlink`/identity path is already written
(`hardlink.rs:67-77`) but unrun here.

**Recommendation.** Keep. The remaining work is evidence, not design: add a POSIX run of
`tests/sync.rs:403`/`:454`/`:512` and a fixture for a hard link to a file **outside** both
roots, which is the case the current suite cannot build on NTFS.

**Verification limits.** No POSIX `nlink`/identity run on this host; no external-link
fixture; the pre-P8-1 directory over-refusal is invisible on NTFS because a directory
reports one link. `regular_file` on Windows is `attributes & FILE_ATTRIBUTE_DIRECTORY == 0`
(`hardlink.rs:134`), which is reasoned, not exercised against a reparse-point directory.

Cross-project impact: none — the gate is internal to `tools/sync`; `girpr`/`gircap` share
no artifact or flag with it.

---

## 2. P7-2 runtime and P8-2/P8-3/R9/R10-1 end-to-end

**Current behaviour.** Several end-to-end cases are present but self-skip on this host:
the dst-file-link-at-a-src-directory case (`tests/sync.rs:896`, P7-2), the link-target and
link-chain spare cases and the shared-storage refuse cases (P8-2/P8-3/R9), and the
case-only parity cases (R10-1). `tests/symlink.rs` self-skips file-link bodies when the
host refuses file symlinks (`os error 1314`); `tests/sync.rs` case-only parity self-skips
when the filesystem pairs `CaseProbe` with `caseprobe`. Their unit-level halves do run:
junction-backed full spare passes for R9-1/R9-2/R9-3, and the case-differing
resolver/spare cases for R10-1.

**History.** These were the seventh-to-ninth pass gaps: `7e8809b` (claim a unique case-fix
temporary), `e93b17f` (spare every link on a kept link's chain), `d9aa88e` (cover link-chain
spares and shared storage), `7e11e12` (resolve a link target's final component), `7aca7e2`
(match an alias to the walk's key). The tests were added with the fixes; only the host
coverage is missing.

**Usage and evidence.** Skip macros in `tests/symlink.rs` and the `#[cfg(windows)]` /
case-probe guards in `tests/sync.rs`; the promises they cover are
[README.md](README.md):1053-1096 and [DESIGN-NOTES.md](DESIGN-NOTES.md):235-292.

**Options.**
- **Keep the self-skips and add a CI job** on developer-mode Windows or POSIX — pros: the
  suite stays honest on any host, and the gap is closed where the capability exists; cons:
  a green local run still does not mean these cases ran.
- **Force the cases and fail on an incapable host** — pros: no silent skip; cons: every
  Windows host without developer mode goes red for an environment limit, not a defect;
  rejected by the existing design.

**Code change.** None. This is a CI configuration decision, not a `tools/sync` change.

**Recommendation.** Keep the skips and add one POSIX (or developer-mode Windows) job that
asserts the cases actually ran (e.g. a non-skipped marker), so a future all-skip job cannot
look green.

**Verification limits.** The host refuses file symlinks (`os error 1314`) and pairs
`CaseProbe`/`caseprobe`; junction-backed passes are the only directory-link coverage here.

Cross-project impact: none — CI job choice only; no shared code or artifact.

---

## 3. R10-1's spelling lookup

**Current behaviour.** `on_disk_name` (`src/commands/sync/plan.rs:297-324`) returns the
listing's exact spelling on a case-sensitive directory and, on Windows, folds case to find
a unique match; an ambiguous match returns `None` (`plan.rs:297-319`). POSIX returns the
name unchanged (`plan.rs:321-324`). The rule is "exact entry wins, otherwise a unique
case-insensitive match".

**History.** Introduced by R10-1 (`7e11e12`, `7aca7e2`); the POSIX branch and the
per-directory case-sensitive Windows mode were never run.

**Usage and evidence.** `src/commands/sync/plan.rs:297-324`; the case-differing
resolver/spare unit cases pass, the end-to-end parity cases self-skip (§2). No doc promises
the fold; [DESIGN-NOTES.md](DESIGN-NOTES.md):362-388 lists the platform rule as a limit.

**Options.**
- **Keep the Windows-only fold** — pros: matches the platform's own case semantics, one
  rule shared with R11-1; cons: POSIX is reasoned, not run, and a case-sensitive Windows
  directory would take the fold where an exact match was intended.
- **Fold on every platform** — pros: uniform code; cons: wrong on case-sensitive POSIX,
  where `A` and `a` are distinct files; rejected.
- **Never fold; require exact spelling** — pros: no ambiguity; cons: breaks the
  case-only rename the walk legitimately produces on Windows; rejected.

**Code change.** None recommended; the branch is already platform-gated.

**Recommendation.** Keep. Settle it with a POSIX run and, if the host allows it, a
`fsutil file setCaseSensitiveInfo` directory test; the ambiguous branch is unreachable on
an ordinary case-insensitive directory and stays unexercised until then.

**Verification limits.** `fsutil file setCaseSensitiveInfo` is denied here; ordinary NTFS
is the only Windows filesystem tested; POSIX is reasoned only.

Cross-project impact: none — the resolver is `tools/sync`-local.

---

## 4. R11-1's cache-prefix fold

**Current behaviour.** `has_cache_prefix` (`src/util.rs:58-69`) folds case on Windows
(`eq_ignore_ascii_case`) and uses `starts_with` on POSIX. On POSIX, `GIRPR-CACHE` is a
distinct file and correctly stays content; on macOS's default case-insensitive APFS/HFS+
volumes the POSIX branch runs, so a case-only re-spelling of the live cache is still not
reserved. A Windows directory made case-sensitive per directory would over-reserve a
genuinely distinct `GIRPR-CACHE`.

**History.** R11-1 (`40102cd`, `d8672ae`) added the fold after a case-renamed cache prefix
could be copied as content; the POSIX/macOS trade-off is documented, not measured.

**Usage and evidence.** `src/util.rs:58-69`; `is_record_path`/`is_cache_rel`
(`util.rs:73-92`); unit tests `util.rs:202-239`; end-to-end `tests/sync.rs:1603`
(`#[cfg(windows)]`, self-skips on a case-sensitive filesystem). `CACHE_PREFIX =
"girpr-cache"` and `CACHE_VERSION = 2` in `src/cache/schema.rs`. The only cross-project
coupling is `.gitignore:6` (`girpr-cache*`).

**Options.**
- **Keep the Windows-only fold** — pros: mirrors R10-1, reserves the case-only re-spelling
  on the platform where it was observed; cons: macOS default volumes take the POSIX branch
  and remain exposed.
- **Fold on macOS too (case-insensitive-volume probe)** — pros: closes the macOS gap; cons:
  needs a per-volume case probe, and folding on a case-sensitive APFS volume would
  over-reserve; more platform code for an edge case.
- **Rename the prefix to something collision-proof** — pros: removes the class; cons:
  invalidates every existing cache and needs a migration plus a `.gitignore` change;
  rejected for the cost.

**Code change.** None recommended now. If the macOS gap is accepted as real, the probe
belongs next to `has_cache_prefix` (`util.rs:58-69`); a prefix rename would also edit
`.gitignore:6`.

**Recommendation.** Keep the current platform rule and record the macOS exposure as a
known limit; only add a volume-case probe if a macOS user reports the case-only re-spelling.
Settling evidence: a macOS APFS run of `tests/sync.rs:1603` (or a case-only cache rename).

**Verification limits.** `fsutil file setCaseSensitiveInfo` is denied here; macOS is not
run at all; the over-reserve trade-off on a case-sensitive Windows directory is unmeasured.

Cross-project impact: the cache prefix is mirrored by `.gitignore:6` (`girpr-cache*`); a
rename would need that file updated, but no `girpr`/`gircap` code reads it.

---

## 5. File links

**Current behaviour.** File-link cases exist in `tests/symlink.rs` (32 link cases) but their
file-link bodies self-skip when the host refuses file symlinks (`os error 1314`). Junctions
stand in for directory links only; native POSIX symlinks and Unix link behaviour are
unexercised.

**History.** The link fixes (`f4dd3c0`, `e93b17f`, `0385eee`, `1543725`, `7e11e12`) were
verified through junctions and unit cases; the file-link end-to-end path never ran here.

**Usage and evidence.** `tests/symlink.rs` skip macros; [README.md](README.md):1053-1096
promises both link kinds; [DESIGN-NOTES.md](DESIGN-NOTES.md):235-292 describes kept-link
sparing.

**Options.**
- **Keep the self-skip and rely on CI** — pros: no false red on locked-down hosts; cons:
  local runs never cover file links.
- **Require developer mode for the suite** — pros: forces coverage; cons: excludes the
  default Windows developer environment; rejected.

**Code change.** None; `src/link.rs:53-89` (`try_is_link`) already handles the fallback.

**Recommendation.** Keep; close it with a POSIX CI job and one developer-mode Windows run.
This is the same host-coverage decision as §2 and should be settled there.

**Verification limits.** No native POSIX symlink run; no file symlink on this host;
junctions are not file links and do not exercise the file-link branch.

Cross-project impact: none.

---

## 6. P7-1/P8-4 on POSIX and a crash between the renames

**Current behaviour.** A case-only rename uses a fixed short temporary
`.girsync_case_tmp.<n>` claimed via `symlink_metadata` + `create_new`/`create_dir`
(`src/commands/sync/rename.rs:198-244`), then a two-step rename
(`rename.rs:422-466`). The temporary was exercised on Windows, including a 244-byte source
name (`tests/sync.rs:710`, P8-4) and an occupied temporary (`:634`, P7-1); P8-5/P8-6
(`:793`, `:845`) cover cleanup/rollback. A
process killed between step 1 and step 2 was not produced; the next run steps over such a
leftover by construction.

**History.** `7e8809b` claimed a unique temporary; `e5df3d4` bounded its name; `0459a68`
reported a failed placeholder cleanup. Whether a direct case-only `rename` is reliable
enough to drop the two-step was not tested.

**Usage and evidence.** `src/commands/sync/rename.rs:37-47` (failure hooks), `:255-284`
(`fail_step1`/`fail_step2`), `:422-466` (two-step). No external consumer.

**Options.**
- **Keep the two-step rename** — pros: works where a direct case-only rename is unreliable,
  and the leftover is recoverable; cons: two syscalls and a temporary that a crash can
  leave behind.
- **Use a direct case-only rename** — pros: one syscall, no temporary; cons: unproven on
  Windows/POSIX filesystems, and a failure mid-way has no recovery; not recommended without
  a matrix run.
- **Keep two-step but make the temporary process-unique** — pros: removes the claim race;
  cons: longer names and a new cleanup contract; the current claim already excludes
  concurrent writers.

**Code change.** None recommended. If the direct rename is ever adopted, it replaces
`rename.rs:422-466`; the temporary and hooks would go with it.

**Recommendation.** Keep the two-step. Settle it with (a) a POSIX `NAME_MAX` run of the
244-byte case and (b) a kill-between-renames test that asserts the next run recovers.

**Verification limits.** Windows-only exercise; POSIX `NAME_MAX` reasoned, not run; no
crash injection between the renames; direct-rename reliability untested.

Cross-project impact: none — the temporary name is `tools/sync`-local and matched by the
tool's own cleanup.

---

## 7. The failure injectors

**Current behaviour.** Cleanup and rollback tests use the test-only
`GIRSYNC_TEST_FAIL_RENAME{,_STEP1,_STEP2}` hooks because a real `fs::rename` failure cannot
be arranged portably. Copy-side hooks (`GIRSYNC_TEST_FAIL_COPY`, `_FLUSH`, `_NOFSYNC`) live
at `src/commands/sync/apply/copy.rs:69-134`. P8-5's cleanup-error wording is not forced: a
failing `remove_file` on the tool's own placeholder could not be arranged, so only the
cleanup path, not its error text, runs.

**History.** Hooks were added with the rollback work (`7e8809b`, `0459a68`, `f60e151`);
they are compiled in all profiles but only read when the env var is set.

**Usage and evidence.** `copy.rs:69-134`; `rename.rs:37-47`; tests `tests/sync.rs:793`
(P8-5), `:845` (P8-6), `:1478` (T-2 sets `GIRSYNC_TEST_NOFSYNC=0`).

**Options.**
- **Keep the test-only env hooks** — pros: deterministic failure injection without a
  platform matrix; cons: a shipped env-var surface and one unforced error-text path.
- **Move hooks behind `#[cfg(test)]`** — pros: no production surface; cons: integration
  tests run the binary, so they cannot see a `cfg(test)` build; rejected for the current
  test shape.
- **Drop the hooks and rely on real faults** — pros: no extra surface; cons: the rollback
  paths become untestable; rejected.

**Code change.** None recommended. If the shipped surface matters, gate the reads behind a
`debug_assertions` check rather than `cfg(test)`.

**Recommendation.** Keep; accept the unforced cleanup-error wording as a test-depth limit
and note it in the test, rather than adding an artificial fault.

**Verification limits.** Real `fs::rename`/`remove_file` failures are not produced; P8-5
error text is not asserted; the hooks are not compiled out of release builds.

Cross-project impact: the hooks are `tools/sync`-local env vars; `.config/nextest.toml`
and `nextest-test-env{,.cmd}` are shared test plumbing but carry no girsync-specific hook.

---

## 8. T-3 / T-4

**Current behaviour.** T-3 has no failing-first regression on a case-insensitive host; the
only observable regression is on a case-sensitive host (`tests/sync.rs:1531`). T-4's
non-link reparse classification is source-verified only: `src/link.rs:53-89` starts from
std's `FileType::is_symlink` and falls back to `read_link`; no OneDrive install exists here
and tagging needs elevation.

**History.** T-3/T-4 came from the third/fourth passes; both were accepted as
source-verified at the time.

**Usage and evidence.** `tests/sync.rs:1531` (T-3); `src/link.rs:53-89` (T-4);
[DESIGN-NOTES.md](DESIGN-NOTES.md):362-388 lists the reparse limits.

**Options.**
- **Keep as-is and watch Linux CI** — pros: no code churn; cons: T-3 stays unproven until
  a case-sensitive host runs it.
- **Add a reparse-point fixture for T-4** — pros: exercises the fallback; cons: needs
  elevation or a real cloud-placeholder file, neither available here.
- **Treat `FILE_ATTRIBUTE_REPARSE_POINT` alone as a link** — pros: simple; cons: cloud
  placeholders, dedup and AppExecLink set it too, so it would misclassify real files;
  rejected.

**Code change.** None recommended.

**Recommendation.** Keep; T-3 is settled by the Linux job in §2, and T-4 by a
reparse-tag fixture on a host that can create one. Until then the current
`is_symlink`-then-`read_link` fallback is the safer default.

**Verification limits.** `fsutil file setCaseSensitiveInfo` is denied here; no OneDrive or
elevation; the `FileType` bits for a junction are not portable across toolchains (§11).

Cross-project impact: none.

---

## 9. F-6 and P6-1

**Current behaviour.** F-6's mtime-restore failure still has no injected test; the doc note
is the practical fix (`src/commands/sync/apply/copy.rs:293-306` aborts on a failed restore).
P6-1's `--keep-extra`/`--missing-only` variants are unit-covered, not all run end-to-end.

**History.** F-6 was a sixth-pass finding; P6-1 is the sixth-pass extra-file policy. Both
were fixed and documented; only the test depth is short.

**Usage and evidence.** `copy.rs:293-306`; P6-1 at `tests/sync.rs:351`; the flags are
documented in [README.md](README.md) (sync options) and [DESIGN-NOTES.md](DESIGN-NOTES.md).

**Options.**
- **Keep the doc note for F-6** — pros: no artificial fault; cons: the abort path is only
  reasoned.
- **Add an mtime-injection hook** — pros: covers the abort; cons: another shipped test hook
  and a platform-specific `utimensat`/`SetFileTime` failure to fake.
- **Run every P6-1 variant end-to-end** — pros: closes the policy gap; cons: more suite
  time for variants the unit tests already cover.

**Code change.** None required for either; a new injector would sit beside the existing
ones in `copy.rs:69-134`.

**Recommendation.** Keep the F-6 doc note (the abort is a safety stop, and faking an mtime
failure adds surface for little gain). Add end-to-end coverage for the P6-1 variants only if
a variant's interaction with the delete plan is in doubt.

**Verification limits.** No injected mtime-restore failure; not every P6-1 variant runs
end-to-end.

Cross-project impact: none.

---

## 10. Other gaps

**Current behaviour.** `remove_dir_all` on a genuine directory containing a junction is not
in the suite; an eighth-pass probe outside the repo removed such a directory and the
junction's target outside the tree survived, closing the Windows half. T-2's flush injector
is test-only (`copy.rs:85-114`; `tests/sync.rs:1478` sets `GIRSYNC_TEST_NOFSYNC=0`). T-6's
binary test is Windows-only (`tests/sync.rs:1563`).

**History.** The probe is recorded in `1a5c829`/`ff4e1ac`; T-2 and T-6 came from earlier passes and
were left as test-only/platform-only by design.

**Usage and evidence.** `src/commands/sync/apply/phases.rs:24-301` (delete/rmdir passes,
`remove_dir_all`), `:345-384` (cache prune), `:399-415`
(`guard_removal_outside_src`); `copy.rs:85-114`; `tests/sync.rs:1478`, `:1563`.

**Options.**
- **Add the junction-in-directory case to the suite** — pros: pins the Windows half; cons:
  the probe needed a junction target outside the repo, so the fixture is fiddly.
- **Keep the probe result as documentation** — pros: no fixture; cons: a regression would
  not be caught.
- **Add a POSIX T-6 equivalent** — pros: covers the binary path on Linux; cons: the
  Windows-only test may already be the only place the binary path differs.

**Code change.** None required for the current behaviour; a suite case would call
`remove_dir_all` through the existing phases (`phases.rs:24-301`).

**Recommendation.** Add the junction-in-directory case to the suite if a stable fixture can
be built; otherwise keep the probe note. Leave T-2 test-only and add a POSIX T-6 equivalent
when a POSIX job exists (§2).

**Verification limits.** The junction-in-directory case is outside the suite; T-2's flush is
injected, not real; T-6 is Windows-only.

Cross-project impact: none.

---

## 11. Windows junction and reparse mechanics

**Current behaviour.** From the original audit (archived at `94f1c3a`): `remove_dir_all` on a
junction or directory link was not run — F-1 does not depend on it, because the data loss
was the `remove_file` of a descendant — and how far it recurses into the target remains
unverified. On rustc 1.98.1, `symlink_metadata` on a junction reports `is_symlink = true`,
`is_dir = false`, `is_file = false`, `attrs = 0x410`, while `path_is_symlink()` is true and
the followed `metadata()` is a directory. Older toolchains differ, so the `FileType` bits
are not a portable test on their own, and `FILE_ATTRIBUTE_REPARSE_POINT` is necessary but
not sufficient (cloud placeholders, dedup and AppExecLink set it too). `src/link.rs:53-89`
starts from std's `FileType::is_symlink` and falls back to `read_link` (third pass, T-4).

**History.** This is the oldest open item, carried from the first pass at `94f1c3a`; later
passes narrowed it (P8-1 for directories, T-4 for the fallback) but did not close the
recursion question.

**Usage and evidence.** `src/link.rs:53-89`; `src/scan.rs:40-140` (`walk_live` follows
links) and `:144-161` (`check_mixed_case`); [DESIGN-NOTES.md](DESIGN-NOTES.md):362-388.

**Options.**
- **Keep the std `is_symlink` + `read_link` fallback** — pros: portable across toolchains,
  and the fallback catches a reparse point std does not call a symlink; cons: does not
  answer how far `remove_dir_all` recurses into a junction target.
- **Classify on `FILE_ATTRIBUTE_REPARSE_POINT` alone** — pros: one attribute check; cons:
  misclassifies cloud placeholders, dedup and AppExecLink as links; rejected.
- **Add a recursive-removal guard that refuses any reparse point below a directory** —
  pros: makes the unverified recursion moot; cons: changes removal semantics for a case no
  current fix depends on.

**Code change.** None recommended for the classification; a recursion guard would live in
`src/commands/sync/apply/phases.rs:24-301` next to `remove_dir_all`.

**Recommendation.** Keep the fallback. The recursion question is only worth a guard if a
future fix starts removing through a junction; today F-1's data loss is the `remove_file`,
which is already covered. Settle it with a junction-recursion probe if that changes.

**Verification limits.** `remove_dir_all` on a junction/link was not run; `FileType` bits
are toolchain-dependent; reparse-tag classification is source-only.

Cross-project impact: none — the link classifier is `tools/sync`-local.

---

## 12. `compare-self` warning text

**Current behaviour.** `warn!` is emitted at `src/commands/compare_self.rs:102-120`, but no
test captures stderr; whether the default console level always shows it is not verified. The
behaviour (flag accepted, no writes) is tested.

**History.** The warning was added with the `compare-self` guard; the test asserts the
no-write behaviour, not the message.

**Usage and evidence.** `src/commands/compare_self.rs:102-120`; `tests/compare_self.rs`
(behaviour only). The command is user-facing, so the message is part of the contract.

**Options.**
- **Keep the `warn!` and add a stderr capture test** — pros: pins the user-visible text;
  cons: needs a logging-capture harness or a subprocess assertion.
- **Raise it to `error!`** — pros: more likely shown at the default level; cons: changes
  severity for a non-fatal condition.
- **Leave it unasserted** — pros: no harness; cons: the message can regress silently.

**Code change.** None required for behaviour; a test would assert the stderr of a
subprocess run, and any level change is a one-line edit at `compare_self.rs:102-120`.

**Recommendation.** Keep `warn!` and add a subprocess stderr assertion; that is the only
way to settle "always shown at the default level". Do not raise the level without a
user report.

**Verification limits.** No stderr capture; the default console level's filtering is
unverified.

Cross-project impact: none — the warning is `tools/sync`-local; `crates/log` supplies the
shared logger but this item does not change its policy.
