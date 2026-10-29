# OPEN FOR MAINTAINERS — `girpr` (root crate)

`girpr` (root crate), branch `fix-gaps/girpr`. These are deliberate trade-offs,
unverified areas and product decisions that remain open after the adversarial
review — not open defects. Each item below gives the current and historical
behaviour, who depends on it, the real options, the code that would move, a
recommendation where one is genuinely inferable, and what was not verified.
Claims are grounded in `file:line`, a test name, a doc section or a commit hash;
anything reasoned rather than run is labelled. The review and its binding
decisions are in [`ADVERSARIAL_REVIEW.md`](ADVERSARIAL_REVIEW.md).

Verification baseline: `ADVERSARIAL_REVIEW.md` records **349 passed, 0 failed,
7 ignored** on Windows debug for the eleventh-pass tree. The seven ignored are
the six `artifact_bridge` cases and `peak_heap`; that run has no live CDN
traffic, no release profile and no Linux/macOS host.

## Suggested decision order

1. **Bytes that can be deleted** — §1 (case-sensitive purge membership, an
   incomplete delete plan behind a ceiling, the empty-directory sweep). These
   act on the irreversible half and have concrete failure modes.
2. **What a read-only run promises** — §2 (`--dry-run`/`--check-only` skip the
   range validation). This decides exit `2` vs `3` and whether a printed plan
   can be trusted.
3. **Exit-code strictness of local state** — §4 (R-2, T-1/E-1, V-2, T-2/E-3,
   U-1). These share the "an explicit configuration must not degrade silently"
   rule and are what a user sees at exit `1`/`4`; decide them together.
4. **Counters and the disk model** — §3 (F-9, E-2). No data path; fix the
   definition and both documents together.
5. **Evidence depth** — §5 (stat/panic injection seams). No product decision;
   only add a seam where a cheap classifier refactor exists.
6. **Unverified areas** — §6. Probe or accept; do not decide them from the
   current mock/reference corpus.

---

## 1. Plan identity and irreversible deletion

### 1.1 N-5 — the purge's expected set is still case-sensitive on Windows

**Current.** `RepairPlan::expected_paths()` returns the plan's `rel` strings
verbatim (`src/repair/phases/plan.rs:128-130`), and `classify_purge_path` keeps
a path only on exact `expected.contains(rel)` (`src/repair/purge.rs:384`). The
same exact comparison applies to `server_keep` (`purge.rs:388`) and `blacklist`
(`purge.rs:392`). On Windows, `Game/Foo.dat` and `game/foo.dat` name one file:
the plan's dedup key folds case on Windows only (`plan.rs:188-194`, first
spelling wins, `the_plan_dedups_case_insensitively_on_windows`,
`plan.rs:540`), but the purge walk reports the **on-disk** spelling. A manifest
`Game/Foo.dat` repairing an existing `game/foo.dat` leaves the directory entry
named `game/foo.dat`; `--purge-after` then sees that name, misses the expected
set and deletes the file the run just verified.

**History.** N-5 (`56a8a45`) fixed only the plan's `seen` set. Before it, two
case variants were planned as two files writing the same target concurrently.
The commit message states the rule: "Windows paths and the path scope are not
[case-sensitive]", so the plan must fold; it did not touch `expected_paths` or
the purge. The scope glob is documented case-insensitive (`README.md:285`), so
the plan, the scope and the purge now disagree. *(The deletion is reasoned
from the three call sites; it was not reproduced on this host.)*

**Usage and dependents.** `expected_paths()` is consumed only by `plan_purge`
(`purge.rs:95`). `classify_purge_path` is the single keep/purge predicate
(`purge.rs:372-...`), reached by `--purge-before`, `--purge-after` and
`--purge-only` through `apply_purge`. No test builds a case-differing on-disk
file; the only case test is the plan dedup.

**Options.**
- **(a) Keep exact matching** — no code change; on Windows a server-side case
  change can delete a just-verified file.
- **(b) Fold case on Windows in the purge** — a `purge_key` mirroring
  `dedup_key`, applied to `rel`, `expected`, `server_keep` and `blacklist`.
  Matches N-5 and `README.md:285`; on Unix the key is identity, so distinct
  files stay distinct.
- **(c) Canonicalise the manifest rel to the on-disk case at plan time** — one
  comparison rule, but needs a `readdir` per component and cannot canonicalise
  a path that does not exist yet.

Failure mode for (b): none on Unix (no folding); on Windows the two spellings
cannot be distinct files. Failure mode for (c): a fresh install has nothing to
canonicalise.

**Code/tests/docs.** `src/repair/phases/plan.rs:128-130`,
`src/repair/purge.rs:384-392`. Add a purge case test beside
`the_plan_dedups_case_insensitively_on_windows` and one integration case with a
lower-case on-disk file. No doc change beyond the existing scope/case statement.

**Recommendation.** **(b).** It closes a delete path, mirrors the already
binding N-5/decision 2 rule, and is Windows-only, so it cannot merge two real
Unix files. *(Inferred from N-5 and the scope contract; the deletion itself is
source-reasoned, not run.)*

**Verification limits.** No case-differing on-disk fixture; no live CDN. The
claim that Windows preserves the existing name when opened under another case
is reasoned from `contained_join` + `metadata`, not observed on a mounted NTFS
volume with a case-differing manifest.

**Cross-project impact.** none. The expected set and purge are `girpr`-only;
`gircap` records manifest spellings verbatim and the bridge replays them, so a
bundle can expose the case only if the capture itself carries both spellings.

### 1.2 N-19 — `errors` is diagnostic; the delete ceiling sees a lower bound

**Current.** `plan_purge` counts a non-`NotFound` walk error (`purge.rs:119-125`)
and a failed candidate `metadata` (`purge.rs:162-170`) in `PurgePlan::errors`,
warns once (`purge.rs:201-206`) and still returns the reachable candidates.
`errors`'s own doc says `total_bytes` is then a lower bound and "the caller must
not treat it as the whole delete set" (`purge.rs:52-58`). The ceiling does
exactly that: `check_preflight` uses `pf.purge.map_or(0, |p| p.total_bytes)`
(`src/repair/scope/limits.rs:122-127`) and `check_delete` compares
`purge.total_bytes > limit` (`limits.rs:141-144`); neither reads `errors`. The
post-repair recheck is the same (`src/repair/phases/post.rs:75`).

Two distinct consequences:
- a **walk** error hides a subtree; those files are not candidates and are not
  deleted, so the run silently under-cleans and `total_bytes` is short;
- a **candidate stat** error still pushes the candidate with `bytes: 0`
  (`purge.rs:162-176`) and `apply_purge` deletes it, because deletion does not
  need the size — an uncounted file can carry the run past the ceiling.

**History.** N-19 (`408b239`) added the counter and the warning after a
walk/stat failure was dropped; it did not change the ceiling. The review records
the decision as "warn and count", not "refuse".

**Usage and dependents.** `--max-delete-bytes` is the only consumer of
`total_bytes`; `errors` is otherwise unread (construction and the warning
only). `README.md:288` says the ceiling is checked against the candidates
"computed before any work starts, and again against the post-repair pass's own
candidates", without saying the list may be short.

**Options.**
- **(a) Keep diagnostic only** — no code change; a stat-failed candidate can be
  deleted uncounted, and an unreadable subtree makes the cleanup silently
  narrower.
- **(b) Refuse the delete pass when a ceiling is set and `errors > 0`** — a
  ceiling is a promise about an exact set, and a lower bound cannot enforce it.
  Simple: an extra hit in `check_preflight`/`check_delete`. Cons: one ACL
  failure refuses the whole cleanup.
- **(c) Mark an unmeasurable candidate and skip it under a ceiling** — precise,
  leaves the file in place and names it; needs `PurgeCandidate::bytes` to become
  `Option<u64>` and every consumer to handle it.
- **(d) Use a conservative size when a candidate cannot be stat'd** — no
  uncounted delete, but there is no safe substitute for an arbitrary file size.

Failure mode for (b): a permission hiccup blocks a cleanup that would have been
safe. Failure mode for (c): a larger struct/API change for a rare shape.

**Code/tests/docs.** `limits.rs:98-128, 141-144`, `post.rs:75`,
`purge.rs:162-176`; `PurgePlan::errors` is constructed as `0` in tests
(`limits.rs:218`, `scope/report.rs:407`), so the non-zero arm needs a new test
that injects a walk/stat failure.

**Recommendation.** **(b) for the ceiling case only.** When
`--max-delete-bytes` is set and `errors > 0`, refuse (exit `5`) before any
delete and name the count; without a ceiling, keep today's warn-and-delete.
This respects the `errors` doc without changing the no-ceiling cleanup.
*(Inferable from the field's own contract and the ceiling's purpose; the
per-candidate `Option<u64>` option is more precise but not required.)*

**Verification limits.** No test drives a non-zero `errors` into either ceiling;
the walk/stat failure arms are source-read only.

**Cross-project impact.** none. `PurgePlan` and both ceilings are `girpr`-only.

### 1.3 S-1 — an empty out-of-scope directory is kept

**Current.** In the purge walk a directory is added to `all_dirs` only if
`scope.is_visible(rel)`; a hidden directory increments `hidden` and is skipped
(`purge.rs:132-144`). The sweep filters `all_dirs` by `kept_dirs` and `game_dir`
(`purge.rs:186-190`), so an empty directory the scope hides is never removed,
and `hidden` drives the "outside the active scope" warning (`purge.rs:196-200`).
An `--exclude "mods/*"` hides the **files** under `mods` but not `mods` itself
(`*` crosses `/`, `README.md:285`), so `mods` is visible and is swept when
empty; only the matched children are hidden.

**History.** S-1 (`3c156c8`) added the `is_visible` gate after the review
reproduced the binary deleting `mods/` and `mods/nested/` under
`--include game/foo.dat`. Before it, `mark_kept_ancestors` walked up only from
kept **files**, and an empty directory has none.

**Usage and evidence.** Unit
`a_scope_keeps_an_empty_out_of_scope_directory_out_of_the_sweep`
(`purge.rs:758`), control `a_scope_still_sweeps_a_directory_it_admits`
(`purge.rs:793`), integration
`a_scoped_purge_keeps_an_empty_out_of_scope_directory`
(`tests/repair_offline.rs:1738`). `docs/04-scope-limitation-spec.md:96-97`
states the rule.

**Options.**
- **(a) Keep** — the scope says the path does not exist; removing the
  placeholder would be a deletion the scope did not admit. Cons: a stale empty
  dir survives a scoped cleanup, and `--exclude "mods/*"` still sweeps `mods`.
- **(b) Warn and delete hidden empty dirs** — matches "cleanup everything", but
  violates the stated scope contract and the unit/integration tests.
- **(c) Treat a directory as hidden when every path under it is hidden** —
  matches the likely intent of `--exclude "mods/*"`, but needs a decision for
  an empty directory and changes the "path exists" model.

Failure mode for (a): surprise deletion of `mods` under a content-only exclude;
for (b): deletes a path the user excluded.

**Code change.** None under (a). Under (c), `purge.rs:132-144` and
`PathScope::is_visible` need a directory-aware predicate, and
`docs/04-scope-limitation-spec.md:96-97` / `README.md:285` should say which
spelling hides a directory.

**Recommendation.** **Keep (a)**, and add one sentence to
`README.md:285`/`docs/04:96-97` that a content glob does not hide the
directory itself. The keep rule is the safer of the two irreversible outcomes.
*(The `mods` sweep is reasoned from `*` crossing `/`; it was not run as its own
case.)*

**Verification limits.** The hidden-dir case was reproduced with the binary
before the fix (review, S-1); the `--exclude "mods/*"` sweep was not run as a
dedicated case.

**Cross-project impact.** `docs/04` is duplicated on `fix-gaps/gircap` and
`fix-gaps/girsync` with the same rule but older module paths
(`repair::plan::purge::…`); a sentence added here must be applied there or left
to the `all/fix-gaps` merge. No code is shared.

---

## 2. Read-only runs and the plan boundary

### 2.1 `validate_chunk_ranges` is only called on the write path

**Current.** `validate_chunk_ranges` (`src/repair/file/mod.rs:314-345`) rejects
a negative file size, a negative chunk offset/size, an `offset + size` overflow
and an end past `file.size`. It is called only in `repair_one_file` and only
when `!ctx.dry_run` (`file/mod.rs:412-414`). `--check-only` has its own loop
(size + MD5, `src/repair/phases/check.rs:61-118`) and never calls it;
`--print-plan` returns at `src/repair/mod.rs:346` before the repair loop. All
three read-only modes therefore accept a manifest the write path would refuse
per file, and `--print-plan` can print a plan that cannot run. A file whose
size and MD5 happen to match but whose chunk arithmetic is corrupt is reported
**clean** by `--check-only`, for the wrong reason.

**History.** `a3e5caf` added the validation together with the panicked-file
accounting; before it the end was computed as `c.offset + c.uncompressed_size`
in `i64`, which panicked a debug build and left `files_failed == 0`. The helper's
doc states the deliberate choice: "Both are per-file failures (exit `3`), not
plan failures: the rest of the install can still be repaired"
(`file/mod.rs:308-313`). No commit ever put the check in `build_plan`.

**Usage and dependents.** The write path and
`a_manifest_offset_overflow_fails_the_file_instead_of_panicking`
(`tests/repair_offline.rs:641`, asserts exit `3`, one failed file, no
`config.ini` bump). `--check-only` and `--dry-run` are documented as writing
nothing and keeping the counters at `0` (`README.md:220-221`).

**Options.**
- **(a) Keep per-file only** — no code change; the manifest is data, and one
  corrupt file should not block the other 100k. Cost: read-only modes give a
  false green, and `--print-plan` can show an unrunnable plan.
- **(b) Validate in `build_plan` (exit `2`) before any mode** — one gate, all
  modes agree, matches the containment/F-2 rule that a plan that cannot be
  executed is a metadata error. Cost: one corrupt range refuses the whole run
  and the helper's stated per-file rationale/test would change.
- **(c) Add the check to `check.rs` only (exit `4`)** — keeps per-file
  semantics and makes `--check-only` honest; `--print-plan` is still a preview
  and `--dry-run` would still not validate.

Failure mode for (b): a single hostile/corrupt manifest entry blocks every
repair, including the files that are fine. Failure mode for (c): a user who
trusts `--print-plan` still sees an unrunnable plan.

**Code/tests/docs.** `src/repair/file/mod.rs:314-345, 412-414`,
`src/repair/phases/check.rs:61-118`, `src/repair/mod.rs:305-346`. A (b) change
also moves/duplicates the `a_manifest_offset_overflow…` expectation to exit
`2`; a (c) change needs a check-only case with a hostile range.

**Recommendation.** **No recommendation on the exit code**; both contracts are
defensible, and the helper's comment makes (a) the current deliberate choice.
Whichever is chosen, add the same range check to the `check.rs` loop so
`--check-only` cannot call a corrupt-range file "intact" for the wrong reason,
and state the chosen exit in `README.md:220-221`. If the plan must be
executable before it is printed, (b) is the coherent choice.

**Verification limits.** No read-only run was driven against a hostile range;
the claim that `--check-only` reports it clean is source-reasoned from the
absence of a call in `check.rs`.

**Cross-project impact.** `docs/02-repair-chunk-spec.md` is the repair contract
the artifact bridge replays; a plan-time refusal changes the documented exit
for a malformed manifest, so `docs/02` and the bridge's expectation would need
the same statement. `gircap` captures the manifest, not the refusal, so no
`tools/capture` code changes. `crates/log`, `crates/tempdir`, workspace
`Cargo.toml`/`Cargo.lock` and `.config/nextest*` are unaffected.

---

## 3. Counters and the disk model

### 3.1 F-9 — `kept_bytes` counts a resumed `_tmp` prefix

**Current.** `kept = file.size - stats.settled_writes` (`file/mod.rs:273-277`).
The temp path resumes a partial `_tmp` by length, so the bytes already in that
prefix are not written by this run and count as kept. `settled_writes` is
accumulated across attempts, not taken from the last one, so a chunk an
abandoned attempt wrote is not claimed as kept a second time. The count is made
**before** the final MD5 gate; the file may still fail and be re-done, which is
why the review calls the figure "unverified until the MD5 gate".

**History.** `ef5d69f` changed the counter from the last attempt's writes to
the settled ones; `64e32ac` documented that a resumed `_tmp` prefix counts;
`ce45a2f` aligned the field doc. `partial_tmp_prefix_is_resumed`
(`tests/repair_offline.rs:435`) pins the resumed-prefix figure and
`kept_bytes_is_the_complement_of_the_files_settled_writes`
(`tests/repair_offline.rs:3358`) pins the settled-write definition.

**Usage and dependents.** `SUMMARY kept_bytes=` (`README.md:147-150`),
`docs/02-repair-chunk-spec.md:405-411`. No consumer reads it as a verified
claim; it is the run's disk claim for operators. It is emitted only when
`--repair-mode` was passed.

**Options.**
- **(a) Keep** — the field means "bytes this run did not have to write", and a
  resumed prefix qualifies; it is the complement of the settled writes, which
  is the only figure the repair actually knows.
- **(b) Count only in-place correct bytes** — "kept" becomes "correct where it
  belongs", excluding a resumed `_tmp` prefix; a different, narrower claim, and
  the temp path's headline number drops.
- **(c) Split the counter** (`kept_inplace` / `kept_resumed`) — both claims,
  no redefinition; adds a `SUMMARY` field and a consumer contract.

Failure mode for (a): an operator reads "kept" as "verified correct", but the
MD5 gate may later reject the file. Failure mode for (b): the temp path's disk
saving disappears from the summary even though the bytes were not written.

**Code change.** None under (a). (b) changes `file/mod.rs:273-277` and both
documents; (c) adds a counter to `Summary` and the `SUMMARY` line.

**Recommendation.** **Keep (a).** The field is documented as "bytes not
written this run" in `docs/02:405-411` and `README.md:147-150`; redefining it
to mean verified correctness would make it a different metric, not a bug fix.
If a verified number is wanted, add (c) rather than change (a).
*(Inferred from the documented definition; the pre-MD5 timing is stated in the
code and the review.)*

**Verification limits.** The resumed-prefix figure is unit/integration tested;
no test covers a resumed prefix that later fails the MD5 gate and is re-done.

**Cross-project impact.** `docs/02` is shared (the tool branches carry an older
copy); a redefinition would need the same edit there and would change the
`SUMMARY` field the bridge records but does not compare. No code in
`crates/log`, `crates/tempdir`, the workspace `Cargo.*` or `.config/nextest*`.

### 3.2 E-2 — `discard`'s `extra_bytes` is growth, not 0

**Current.** `DiskPlan` maps `discard` to `Mode::Inplace` with
`extra_bytes = size - on_disk` (`src/repair/scope/report.rs:100-108`). Deleting
the target removes the second copy, but a truncated target still has to grow to
its manifest size, so the run needs that growth before it starts. `0` is
correct only when the target is already full size. `README.md:293` and
`:500-506` and `docs/02:394-403` now say "growth (0 when already full size)".
`discard_needs_the_growth_but_never_a_second_copy` (`report.rs:535`) pins a
full-size and a truncated target.

**History.** The code was always growth; the docs said `0` for every discard.
E-2 (`56236a5`) kept the code and corrected `README.md`/`docs/02`, and
`8a1618f` aligned the surrounding text.

**Usage and dependents.** `DiskPlan::est_extra_bytes` feeds the `PLAN` line and
`--plan-out`'s per-file `extra_bytes`; nothing branches on it at runtime. The
bridge replays a repair and does not compare these figures.

**Options.**
- **(a) Keep growth** — truthful free-space estimate; a truncated discard needs
  the growth.
- **(b) Report 0** — matches the old headline "0 extra disk", but understates
  what a truncated target needs and can fail a run on a full volume.
- **(c) Report `size`** — conservative like `temp`, but wrong: discard never
  holds a second full copy.

Failure mode for (b): the run starts without the free space it needs. Failure
mode for (c): overstates the cost and may refuse a run that fits.

**Code change.** None. If (b) were chosen, change `report.rs:100-108`,
`README.md:293,500-506` and `docs/02:394-403` together.

**Recommendation.** **Keep (a).** The growth figure is the only one that
matches what the write path does; the old `0` was a documentation error.
*(Inferred from the code path and the pinned test.)*

**Verification limits.** Pinned at `DiskPlan`, not end-to-end through
`PLAN`/`--plan-out`; no run was executed against a truncated discard target on
a constrained volume.

**Cross-project impact.** `docs/02-repair-chunk-spec.md` on
`fix-gaps/gircap` and `fix-gaps/girsync` still has `discard … 0` at line 375
(verified with `git show`), while `fix-gaps/girpr` has the growth row. The
`all/fix-gaps` merge must take this branch's `docs/02` for all three tools, or
the shared contract will disagree with the code. No code in `crates/log`,
`crates/tempdir`, workspace `Cargo.*` or `.config/nextest*`.

---

## 4. Boundary strictness and stat state

### 4.1 R-2 — an unreadable audio scan file is exit `1`, not `en-us`

**Current.** `read_scan_file` maps only `NotFound` to `Ok(None)`; every other
read error is `Err` (`src/repair/audio.rs:77-87`). `resolve_effective_audio`
propagates it (`audio.rs:48`), and `metadata::fetch` maps it to a usage failure
(`src/repair/phases/metadata.rs:186-190`), so the run exits `1` naming the file.
An **absent** file is `Ok(None)` and defaults to `en-us`; an **empty** file is
`Ok(Some(empty))` and means game-only; a non-empty file naming no known language
is an error (`audio.rs:105`).

**History.** Before `a812fce`, unreadable, absent and empty were one state;
`--audio none` fell back to `en-us` and, under `--purge-after`, deleted the real
selection. The fix split the states and moved the explicit write to the
execution phase.

**Usage and evidence.** `an_unreadable_audio_scan_file_fails_before_any_repair`
(`tests/repair_offline.rs:1532`, uses a directory where the file belongs, so it
is exercised on this host, not just reasoned). `README.md:244` and
`docs/02:66` document the split.

**Options.**
- **(a) Keep exit `1`** — local state is authoritative; replacing it silently is
  the old data-loss path.
- **(b) Warn and default to `en-us`** — a run still starts, but a transient ACL
  error rewrites the selection and may make `--purge-after` delete packages.
- **(c) Treat unreadable as game-only** — no `en-us` substitution, but it lies
  about the user's saved selection.

Failure mode for (a): a permission problem stops the run; the error names the
file and leaves the install untouched. Failure mode for (b)/(c): the run mutates
or purges against a selection the user did not choose.

**Code change.** None under (a). (b) changes `audio.rs:48,77-87`; (c) changes
the `None` arm.

**Recommendation.** **Keep (a).** It matches R-6 and the "absent is `NotFound`
only" rule (decision 5), and the failure is recoverable without touching the
install. *(Inferred from the binding decision and the executed test.)*

**Verification limits.** The unreadable case is a directory-as-file, not a real
deny ACL; the `--purge-after` deletion consequence is a code-reading claim (the
mock has one language fixture).

**Cross-project impact.** none. The bridge does not exercise audio; `gircap`
does not read the scan file.

### 4.2 T-1/E-1 — a mixed ignore/blacklist file warns per bad line

**Current.** `parse_config_list` strips one leading BOM (`src/util.rs:604`),
warns for each bad line with the path and 1-based line (`util.rs:620-629`) and
refuses only when the file produced **no** entry **and** had at least one
failure (`util.rs:634-641`). The entry closure returns
`Result<Option<String>>`: a non-object line, a non-string `fileName`, a
non-bool `is_delete`, or `is_delete: true` without a string `category` is a bad
line, not a silent skip (E-1, `6cb0cdf`). `read_ignore_categories` and
`read_blacklist` share the parser (`util.rs:520-576`).

**History.** T-1 (`ad0a09c`) replaced the `if let Ok` drop and the
all-malformed `Ok(Some({}))`; E-1 (`6cb0cdf`) added the typed-field failures.

**Usage and evidence.**
`config_lists_strip_a_bom_and_warn_per_unparseable_line` (`util.rs:1418`),
`malformed_config_list_entries_are_refused_not_dropped` (`util.rs:1463`),
`an_all_malformed_config_list_is_refused` (`util.rs:1569`). The ignore set
filters the manifest plan; the blacklist filters the plan **and** the purge.

**Options.**
- **(a) Keep mixed tolerance** — one typo does not disable the other entries;
  the warning names the line. Cons: a corrupt line is fail-open for that one
  entry, and a file with one valid plus many bad lines runs with one entry.
- **(b) Any bad line is fatal** — an explicit file is all-or-nothing; cons: a
  single typo stops a repair that would otherwise be correct.
- **(c) Fatal only for the blacklist** — protection is strict, filtering is
  tolerant; cons: two rules for two files that look alike.

Failure mode for (a): an entry the user believed was protecting a file is
ignored. Failure mode for (b): a cosmetic JSON error blocks the run.

**Code/tests/docs.** `src/util.rs:598-642`; a (b) change drops the
`set.is_empty()` condition (one line), not the readers. `README.md:250-251`
states the cleaned blacklist is a separate keep input.

**Recommendation.** **Keep (a).** The warning is per line and includes the path,
so the failure is visible, and the all-malformed file is still refused. This is
the same shape as R-8 (a scan file with no known language is refused, a mixed
one is not). *(Inferred from the existing decision and the three tests; the
one-valid-plus-many-bad case was not run.)*

**Verification limits.** Pinned at the unit reader, not through the binary; the
purge consequence of a dropped blacklist entry is covered by the tenth-pass
integration case, not by a mixed-file fixture.

**Cross-project impact.** none. The parser is `girpr`-only; `girsync` has its
own config formats.

### 4.3 V-2 — a malformed blacklist entry is fatal

**Current.** `read_blacklist` cleans every entry through `util::clean_rel` and
collects into `anyhow::Result<HashSet<_>>` (`util.rs:573-576`), so one entry
that cannot be a contained rel aborts the read with the entry named, and the
run exits `1`. The comparison against the plan and the purge then uses the
cleaned spelling.

**History.** Before `ec0a79d`, entries were compared raw, so `./`- or
`//`-spelled entries protected nothing (fail-open). V-2 fixed the cleaning and
made an uncleannable entry a refusal rather than a drop.

**Usage and evidence.** The blacklist is a plan filter (`plan.rs:233`) and a
purge keep rule (`purge.rs:392`); losing an entry either repairs a file the user
excluded or deletes a file they meant to keep. V-3's reuse map is the deliberate
opposite: it warns and skips, because it is an optimisation.

**Options.**
- **(a) Keep fatal** — an explicit protection must not degrade silently; the
  error names the entry. Cons: a corrupt line stops the run.
- **(b) Warn and skip** — matches V-3 and keeps the run alive; cons: the skipped
  entry protects nothing, which is the fail-open V-2 fixed.
- **(c) Clean-or-drop only for the blacklist** — a third rule; not obviously
  better than (a).

Failure mode for (a): a hand-edited entry blocks a repair. Failure mode for (b):
a file the user blacklisted is repaired or purged.

**Code change.** None under (a). (b) changes the `collect` at `util.rs:575` to a
filter with a warning.

**Recommendation.** **Keep (a).** The blacklist is protection, the reuse map is
an optimisation; the asymmetry is deliberate (decision 14). *(Inferred from the
binding decision and the fail-open history.)*

**Verification limits.** Pinned at the reader unit test; no end-to-end run with
a malformed entry and a matching on-disk file.

**Cross-project impact.** none. `read_blacklist` is `girpr`-only.

### 4.4 T-2/E-3 — `file_len` keeps absent, unreadable and directory apart

**Current.** `FileLen` is `Present(u64) | Absent | Unreadable(io::Error)`
(`util.rs:270-282`); `classify_len` maps only `NotFound` to `Absent` and every
other error to `Unreadable` (`util.rs:289-296`). `check.rs` logs
`CHECK fail: unreadable` with the file and error and counts the file damaged
(`check.rs:61-76, 101-118`); the temp path warns and still repairs
(`baf6498`). The disk plan's `planned_on_disk` warns and charges an unreadable
file as 0 on disk, i.e. the whole file as growth (`report.rs:148-160`).
A **directory** at a planned file path is `Present(dir_len)` because
`metadata.len()` succeeds for a directory; it is reported as a size mismatch
(or, if the length coincides, `md5_file` fails → "unreadable"). That is safe
but is a separate state from a failed stat.

**History.** T-2 (`baf6498`) split the three states; E-3 (`56236a5`) made the
disk plan use the same split. Before, every non-`NotFound` error read as
"missing/zero".

**Usage and evidence.** `file_len_separates_present_absent_and_unreadable`
(`util.rs:1378`), `an_unreadable_stat_is_charged_as_the_whole_file_and_warned`
(`report.rs:564`, injected `PermissionDenied`), `discard_needs_the_growth…`
(`report.rs:535`). `README.md:244` and decision 9 state the rule.

**Options.**
- **(a) Keep** — the states are safe: a directory is damaged, an unreadable
  file is damaged, only `NotFound` is absent.
- **(b) Add a `NotAFile`/directory variant** — the log says "is a directory"
  rather than "size 0 vs N"; touches every `match FileLen` arm and every
  caller, for a diagnostic improvement.
- **(c) `is_file()` check at each sink** — same information, scattered, and a
  second stat on some paths.

Failure mode for (a): an operator sees "size" for a directory and has to infer
the cause. Failure mode for (b): a wider diff and a new enum arm to keep
exhaustive.

**Code change.** None under (a). (b) changes `util.rs:270-298` and the three
match sites (`check.rs`, `temp.rs`, `report.rs`); `inplace.rs` reads
`scan.file_len` but never matches `FileLen`.

**Recommendation.** **Keep (a).** The current classification is safe and the
log already carries `actual_size`/`error`; a directory at a file path is rare
and is still reported as damaged. *(Inferred from the executed tests; the
directory-length coincidence is reasoned, not run.)*

**Verification limits.** No real deny-ACL fixture; the `DiskPlan` unreadable
arm is pinned through an injected `FileLen::Unreadable`, and the directory case
is not a dedicated test.

**Cross-project impact.** none. `FileLen` is `girpr`-only; `docs/02` does not
describe the stat states.

### 4.5 U-1 — normalisation happens once, at each boundary

**Current.** `util::clean_rel` returns the `/`-separated, `./`-free,
validated form. It is applied to the manifest rel (`plan.rs:228-229`), the
deprecated list (`repair/mod.rs:560-575`; an uncleannable name is dropped with
a warning because it is a delete sink), the three audio dirs at the
`getGameConfigs` boundary (`metadata.rs:66-79, 110-115`, `3123d96`) and
blacklist entries (`util.rs:573`). `res_category_dir` and `blacklist_dir` are read-only
**directory** inputs validated at `contained_join` (`metadata.rs:156-171`),
not normalised by `clean_rel`; `res_category_dir` entries are category ids, not
paths, so they are deliberately not cleaned. V-3's local reuse map cleans its
key and warns/skips an uncleannable local-manifest path (`sophon.rs`,
`3ce91a3`).

**History.** U-1 (`3123d96`) moved the normalisation to the boundary after the
gates scoped the raw spelling while the sink cleaned it, so a `./`-spelled path
could be deleted or moved through an `--exclude`, and an in-scope `./`-spelled
scan dir was refused. `27e7a18` kept the download-mode check ahead of the audio
dir normalisation so the documented usage error wins.

**Usage and evidence.** Ninth-pass tests: the excluded deprecated path, the
excluded audio cache and the in-scope scan file (`tests/repair_offline.rs`,
review pass 9). The gates and sinks share the cleaned string.

**Options.**
- **(a) Keep boundary normalisation** — one spelling per value, gates and sinks
  agree; the two directory inputs rely on `contained_join` because they are
  validated where they are used.
- **(b) Clean every sink defensively** — a second clean cannot hurt but hides a
  missing boundary and makes the "one spelling" rule unenforceable.
- **(c) Clean `res_category_dir`/`blacklist_dir` too** — only needed if a future
  gate matches those directory fields against the scope; today they are
  read-only inputs and `contained_join` is the sink.

Failure mode for (a): a future scope gate on a directory field would compare
the raw spelling; the comment at `metadata.rs:156-171` is the reminder.

**Code change.** None under (a). Under (c), route both fields through
`clean_rel` in `metadata.rs` and add the same scope test shape as the audio
dirs.

**Recommendation.** **Keep (a).** The boundary is the right place, and the two
directory fields have no scope gate to disagree with. If one is added, apply
`clean_rel` there first. *(Inferred from the ninth-pass fix and the call
sites.)*

**Verification limits.** The audio/deprecated/scan-file spellings are tested;
`res_category_dir`/`blacklist_dir` are not exercised with a `./` prefix because
they have no scope gate.

**Cross-project impact.** none. `clean_rel` is `girpr`-only; `docs/04` is
duplicated on the tool branches but this rule is not restated there.

---

## 5. Stat-failure and panic evidence seams

These four paths have no public injection seam. The code is source-read at the
failure arm; only S-3 has a classifier that can be unit-tested with an injected
`io::Error`. None is a known defect.

### 5.1 F-6 — a panicked scan task

**Current.** `scan_plan` runs `scan_one` on the blocking pool and treats a
`JoinError` as a warning; the estimate stays safe because an unreached file is
estimated in full (`src/repair/scan.rs:213-245`, `64c80d6`). No `#[cfg(test)]`
sentinel and no test raises a panic inside `scan_one`; the branch is
code-reading. By contrast, the repair task has `TEST_PANIC_REL`
(`file/mod.rs:390-410`) and `a_panic_inside_a_file_task_fails_the_run`
(`file/mod.rs:794`).

**Options.** (a) Accept — the warning is not load-bearing for correctness. (b)
Add a `TEST_PANIC_REL`-style sentinel in `scan_one` plus a test — pins the
warning and the safe estimate. Failure mode for (a): a future refactor could
drop the `Err` arm silently again. (b) adds test-only code in a hot loop.

**Recommendation.** **Accept (a)** unless the warning text is a contract; the
estimate is already conservative. *(The panic is reasoned from the
`JoinError` arm, not injected.)*

### 5.2 P-2 — the two in-place `File::metadata()` sites

**Current.** `inplace.rs:183` (before the trim) and `inplace.rs:271` (salvage
order) call `handle.metadata()?`; the error propagates to `repair_one_file` and
becomes a failed file (exit `3`). `temp.rs` was moved to `file_len`/`classify_len`
(`edb2eb4`), but the in-place sites still call the handle directly, so there is
no classifier to unit-test.

**Options.** (a) Accept — the `?` already fails the file with path context.
(b) Extract a `classify_len`-style helper over `io::Result<Metadata>` (the same
shape as `classify_deprecated`) and unit-test the error arm; production
behaviour is unchanged. (c) Add a trait/injection seam — overkill for two call
sites.

**Recommendation.** **(b).** It is a small refactor with no public seam and
makes the error arm executable, matching S-3. *(Inferred from the existing
`classify_deprecated` pattern.)*

### 5.3 S-3 — the deprecated-stat failure arm

**Current.** `classify_deprecated` (`post.rs:246-255`) splits
`File | NotAFile | AlreadyGone | Failed`; the failure arm warns with the path
and deletes without counting an unknown size (`post.rs:185-200`). It is pinned
by `classify_deprecated(Err(denied))` (`post.rs:545`) and
`stat_deprecated_separates_file_non_file_and_absent` (`post.rs:557`). History
`e2040cd`/`63a53de`.

**Recommendation.** **No change.** The four-state classifier is the seam; the
syscall wrapper needs none. *(The classifier is executed; the real ACL shape is
not.)*

### 5.4 R-5 — `write_config_ini`'s `sync_all`

**Current.** The temp is written, `sync_all` runs unless
`GIRPR_TEST_NO_FSYNC` is truthy, any write error removes the temp and aborts,
and only then is the rename done (`src/util.rs:855-890`). A failed `sync_all` is
fatal and removes the temp, but no test forces the syscall to fail: the hook
only **skips** it (`util.rs:666-683`; `.config/nextest.toml:38-40`). History
`a812fce`, hook `24c5d0c`.

**Options.** (a) Accept — the path is linear and the cleanup is the same one
the write-error branch exercises. (b) Add a test-only `sync` wrapper selected by
an env var (like the fsync hook) that returns `Err`, and a test asserting no
rename and no temp. Failure mode for (a): a future refactor could publish the
rename after a failed sync unnoticed. (b) adds a production-adjacent env var.

**Recommendation.** **Accept (a).** The cost of a fault-injection hook is
higher than the risk in a three-line path; if a seam is wanted, the `#[cfg(test)]`
wrapper is the smallest one. *(Inferred from the code and the existing hook.)*

**Cross-project impact (whole §5).** `.config/nextest.toml` and
`.config/nextest-test-env{,.cmd}` are workspace-wide and already carry both
`GIRPR_TEST_NO_FSYNC` and `GIRSYNC_TEST_NO_FSYNC`; any new hook must be added
there and affects both tools. `crates/log` and `crates/tempdir` are unaffected
(the latter is dev-only scratch, `girpr_tempdir::TempDir`, with no API change).
No `docs/02`, workspace `Cargo.toml` or `Cargo.lock` change.

---

## 6. Unverified areas carried from the first pass

### 6.1 Live manifest shapes

**What is unverified.** Whether the CDN ever serves a zero-length entry with
`EMPTY_MD5` (`util.rs:13`), a `..` segment, a drive-relative `C:` path, or an
`offset + size > i64::MAX`. The findings describe what the code does **if** such
data arrives, not a claim that it does.

**What the code does now.** `contained_rel` refuses `..`, `C:` and roots before
any sink (`util.rs:306-340`); `validate_chunk_ranges` refuses the overflow per
file (`file/mod.rs:314-345`); a legitimate empty file matches `EMPTY_MD5` in
the MD5 gate and `scan.rs:278` uses the same constant for an absent/unreadable
scan verdict.

**Decision that follows.** **Accept risk** for `..`, `C:` and the overflow:
each has a refusing gate, so the worst case is a refused file or run, not a
write outside the game dir. **Probe** the zero-length shape, because its
handling is spread across the scan verdict, the range validator (no chunks) and
the MD5 gate.

**What closes it.** One live capture (or a crafted bundle through
`tests/artifact_bridge`) containing each shape, run through `--check-only`,
`--dry-run`, `--print-plan` and a repair; for `EMPTY_MD5`, assert an empty file
is intact and a zero-length entry with a non-empty digest is refused. This also
supplies the evidence for §2.1's exit-code choice.

### 6.2 No exhaustive panic sweep

**What is unverified.** The first pass did not enumerate every panic site. The
known server-data panic `temp.rs:193` (F-1) and the file-semaphore `unwrap`
(now a named `expect`, N-12) were handled, but there was no systematic sweep of
`unwrap`/`expect`/slice indexing on server-derived values.

**Decision that follows.** **Probe, not defensive code.** Do not wrap the
pipeline in `catch_unwind`; enumerate the sites and classify each as
server-derived (fix/refuse) or invariant (keep). The existing pattern is to
convert a server-data panic into a per-file `anyhow::Error` (F-1,
`validate_chunk_ranges`).

**What closes it.** A source audit (or a small `rg` checklist) of `unwrap`,
`expect`, direct indexing and integer arithmetic in the manifest/plan/scan/write
paths on values that come from the server, plus one hostile-manifest case per
newly found site. A fuzz target over manifest parsing/planning would cover the
arithmetic half.

### 6.3 `--print-plan --audio` under an active path scope

**What is unverified.** The combination is source-verified but was not
executed. `metadata.rs:203-212` excludes `print_plan` from the refusal, and
`mod.rs:346` returns before the audio write at `mod.rs:384`, so the run exits
`0` and writes nothing; the existing
`explicit_audio_under_a_scope_is_refused_unless_the_scan_file_is_in_scope`
(`tests/repair_offline.rs:1840`) covers the refusal, not the print-plan
variant.

**Decision that follows.** **Accept risk** for the write (it cannot happen:
two independent guards), but **probe** the contract, because a user may expect
the same exit `1` refusal from `--print-plan` and instead gets a plan.

**What closes it.** One integration case with `print_plan = true`,
`audio_explicit = true`, an active scope and the scan file out of scope,
asserting exit `0`, a `PLAN` record, and no scan file on disk; a second case
with the scan file in scope asserting the `PLAN` record still appears and the
write is deferred. If the expected behaviour is refusal, change the
`!ctx.print_plan` condition and the test together.

**Cross-project impact (whole §6).** The evidence for §6.1 runs through
`tests/artifact_bridge*` and `docs/06` (a crafted bundle) and `docs/02` (the
documented refusals), so those two contracts must stay aligned with whatever
the probe finds. `crates/log`, `crates/tempdir`, `.config/nextest*` and the
workspace `Cargo.toml`/`Cargo.lock` do not change, and no `tools/capture` or
`tools/sync` code moves.
