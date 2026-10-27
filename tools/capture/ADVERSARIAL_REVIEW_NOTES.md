# gircap remediation review — reviewer's notes

- **Project:** `gircap` (`tools/capture`)
- **Branch:** `fix-gaps/gircap`
- **Reviewed tip:** `b726054`
- **Baseline:** `64dc69d`
- **Status:** reviewer's notes, committed later with the branch-reference correction (no tracked source files changed during the audit).

---

# Adversarial review — `gircap` remediation of F1–F12

**Review target:** `tools/capture/ADVERSARIAL_REVIEW.md` findings F1-F12 and the fix series in `git diff 64dc69d..HEAD -- tools/capture` (reviewed tip `b726054`).
**Committed:** with the branch-reference correction on this branch (the reviewed tip above predates this commit).
**Method:** read the review document, the full diff, every touched production site and regression test; re-derived each bug rather than trusting the progress section; ran the crate test suite. No tracked files were modified.

---

## Verdict summary

| ID | Finding (abridged) | Verdict | Primary evidence |
|----|-------------------|---------|------------------|
| F1 | Forced captures checked against a per-path budget, not the cumulative ledger | **Fixed** | `src/select/mod.rs:505-511`, `:515-561`, `:689`; test `:1201-1225` |
| F2 | First forced refusal `break`s the list, hiding later cheaper paths | **Fixed** | `src/select/mod.rs:534-549`; test `:1233-1258` |
| F3 | Forced path present but answering no class is silently dropped | **Fixed** | `src/select/mod.rs:524-531`, `:797`; `src/run/mod.rs:399`; test `:1268-1294` |
| F4 | A failed chunk is counted twice and its index row cannot say why | **Fixed** (doc/panic caveats) | `src/blobs/mod.rs:269-306`, `:374-392`; test `:1117-1144` |
| F5 | Error bodies are read unbounded / status is lost on a body read failure | **Fixed** (silent-swallow caveat) | `src/http/fetcher.rs:188-224`; tests `src/http/mod.rs:1372-1490` |
| F6 | Tag shape validation only rejected `/` and space | **Partially fixed — blocking** | `src/config/mod.rs:553-584`; callers only `:530`, `:538`; API tags `:96`, `:100-107` → `src/run/mod.rs:124`, `:240` → `src/config/mod.rs:628-631` |
| F7 | Wire-byte ceiling applied to a metadata-only run | **Fixed** | `src/blobs/plan.rs:122-140`; tests `:208-263` |
| F8 | `std::env::args()` panics after a successful parse on non-UTF-8 argv | **Fixed** | `src/main.rs:59`; `src/cli/mod.rs:409-416`; `tests/cli_contract.rs:112-200` |
| F9 | Duplicate `--files` silently produces two admissions/writes | **Fixed** | `src/config/mod.rs:498-503`; `tests/cli_contract.rs` |
| F10 | `--require-class` / `--no-chunk-blobs` help lies about defaults/scope | **Fixed** | `src/cli/mod.rs:161-162`, `:188`; `tests/cli_contract.rs:133` |
| F11 | Temp file orphaned on Windows because the handle is still open | **Fixed** (panic caveat) | `src/blobs/mod.rs:445-473`, `:492-502` |
| F12 | Close-out regression test iterated a hand-written 8-id subset | **Partially fixed** (test claim overstated) | `src/emit/closeout.rs:623-670`; real pinning at `src/emit/selfcheck.rs:1012-1016`, `tests/capture_layout.rs:977-985` |

**Overall:** the batch fixes the accounting, reporting and I/O defects correctly and the suite is substantially stronger. It is **not** ready to be described as “F1–F12 closed”, because F6 is only half-fixed and the residual is a path-escape sink. Ship after adding validation of API-resolved tags; everything else is follow-up material.

---

## Per-finding analysis

### F1 — Cumulative forced budget — **Fixed**

`select()` now carries one `Budget` through phase 0 and phase 2. Forced admissions call `budget.admits(cost)` then `budget.charge(cost)` on the same ledger (`src/select/mod.rs:534-550`), and the post-forced budget is handed to `admit_in_order` at `:689`. `bytes_admitted` is then recomputed from the admitted set at `:793`, so the matrix and summary agree with the ledger.

The regression test `forced_costs_are_checked_against_the_cumulative_budget_not_one_by_one` (`:1201-1225`) is genuine: two 60 B forced paths under a 100 B cap. The old per-path check admitted both; the test asserts exactly one admission, `bytes_admitted == 60`, and that the returned budget has `spent() == 60`. It would fail against the pre-fix code.

Residual: `Budget::with_spent` (`src/select/budget.rs:45-47`) is now dead — referenced only in the review document, not in `src/`. It is also unclamped, so a caller passing `spent > limit` silently makes every admission fail. Since it is `pub`, remove it or document the precondition; dead API that encodes the old model is a trap for the next change.

### F2 — Forced refusal must not stop the list — **Fixed**

The refusal branch now records a `Rejection` and `continue`s (`src/select/mod.rs:534-548`). The test `a_forced_path_over_the_ceiling_does_not_hide_a_cheaper_one_after_it` (`:1233-1258`) uses 500 B then 100 B under a 200 B cap and asserts the second is admitted and the first is in `rejected`. The old `break` would have admitted neither, so this is a true regression test.

Phase 2 de-duplicates by path (`:695-701`), so the same forced path cannot appear twice in `rejected` when phase 1 also wants it. That is the right interaction: the matrix can still say `covered_by`/`over_budget` without a duplicate finding.

### F3 — Forced path with no class — **Fixed**

A pool entry whose `classes` set is empty is now pushed to `forced_unscored` (`src/select/mod.rs:524-531`) and surfaced as `forced_files_unscored` (`src/run/mod.rs:399`, `src/report/mod.rs:474`, `:647`). The unit test `a_forced_path_that_answers_no_class_is_recorded_as_unscored` (`:1268-1294`) constructs a degenerate patch, asserts no admission, asserts the path is in `forced_unscored`, and asserts it is *not* in `forced_skipped`. That distinguishes “absent from the manifests” from “present but unscorable”, which is the point of the finding.

Test gap: the only integration assertion is `tests/matrix.rs:707-710`, which asserts `forced_files_unscored == 0`. There is no end-to-end test with a nonzero value. `tests/measured_fixture.rs:846` covers the selection layer, so the wiring from `Selection` → summary is untested. Add a capture-level fixture with a degenerate forced path and assert `forced_files_unscored == 1` plus the JSON field.

### F4 — One failed chunk is one failure, with its reason — **Fixed**

`Store::one` now returns `(slot, Result<…>)` (`src/blobs/mod.rs:331-372`), so the error and the index row are produced together. `fill` stores the unfilled row and pushes the error exactly once on `Ok((i, Err(e)))` (`:275-278`), and the hole-fill loop at `:299-305` only handles slots that genuinely never reported. `unfilled` puts the reason into `decode_error` (`:388-392`), which is what A6 reads.

The test `a_chunk_the_cdn_will_not_serve_is_one_failure_with_its_reason_in_the_row` (`:1117-1144`) uses an unbindable port so both sources fail, and asserts `errors.len() == wants.len()` and every want still has a row. That directly pins the double-count.

Two low-severity defects remain:

- **Doc is wrong.** `src/blobs/fill.rs:25` says the errors are “in `wants` order”, but `fill` drains via `JoinSet::join_next`, so they are in completion order. `run` prices only `errors.len()`, so this is documentation-only, but it is exactly the kind of comment a future index writer would trust.
- **Panic cause is discarded.** `Err(_) => {}` at `src/blobs/mod.rs:282` drops the `JoinError` (including the panic payload). The hole-fill then records the generic `"the chunk fetch task panicked while storing it"` (`:301`). The failure count is right, but the operator loses the actual panic message. Log the `JoinError` before discarding it; keep the generic row text if the row must stay stable.

### F5 — Bounded error bodies without losing the status — **Fixed**

`drain_error_body` (`src/http/fetcher.rs:188-224`) refuses to read a body whose declared length is over the cap (`:189-198`), reads at most until the running total crosses it (`:200-215`), and returns the partial count. The call site (`:792-832`) keeps the response status as the failure kind, so a truncated `503` stays `Status` and is not retried as a `Transport`. The three tests at `src/http/mod.rs:1372`, `:1407`, `:1449` cover declared-over-cap, truncated-body-keeps-status, and counted-as-far-as-it-arrived respectively. These are real behavioral tests, not mocks of the branch.

Residual (Low): a mid-read error hits `Err(_) => break` (`:220`) with no log. The status is deliberately preserved, which is correct, but a transport failure while draining is silently erased. A `tracing::debug!`/`warn!` with the URL and the error costs nothing and would explain a suspiciously short `bytes_in`. Also, the declared-over-cap path returns `bytes_in = 0` even though the headers did cross the wire; that is defensible and documented at `:190`, but the field is named `bytes_in`, so a reader may expect header bytes. Either rename or document it in the schema.

### F6 — Tag validation — **Partially fixed; blocking**

The flag path is now correct. `validate_tag_shape` (`src/config/mod.rs:553-584`) rejects empty, >32 chars, control characters, `/`, `\`, space, `:`, and a `..` component, and `validate_tag`/`validate_new_tag` (`:523-539`) call it. `--probe-tags` goes through `validate_probe_tags` (`:508-513`). The error messages name the offending flag. That closes the original reproduction for `--new-tag`, `--old-tag` and `--probe-tags`.

The hole is the **resolved** tags. `VersionPair::select` takes `params.tag` and `params.diff_tags[0]` straight from `getGameBranches` without validation (`src/config/mod.rs:96`, `:100-107`). `run` clones them at `src/run/mod.rs:124` and passes them to `artifact_dir` at `:240`, which calls `pair_dir` (`src/config/mod.rs:628-631`) and joins the raw string as a path component. A hostile or compromised API/proxy can therefore return `..\..\..\outside` or `C:evil` as the branch tag and the bundle is written outside the chosen `--out` root. The same string also feeds `capture_id`/report fields, but the path sink is the security-relevant one.

This is not theoretical hardening: the review’s own F6 rationale is that “a tag is a path component on every host”. That statement is only true for flags today. The fix must validate the resolved pair too — the cheapest correct change is to call `validate_tag_shape` on `params.tag` and `params.diff_tags[0]` in `select()` (or reject in `attach_build_ids` before any directory is created), and to return a clear API-shape error rather than a usage error. A test should feed a mock `BranchParams` with `tag = r"..\..\escape"` and assert `select()` rejects it before `artifact_dir` is reachable.

Two further gaps in the same validator:

- **Windows-reserved characters.** `*`, `?`, `"`, `<`, `>`, `|` are accepted. They are not traversal, but a hostile API tag like `7.1.0?` will fail at `create_dir_all` on Windows after the network work is done — a late, confusing failure. Reject them with the same message class.
- **Trailing dot/space collision.** `7.1.0.` is accepted; Windows normalizes a trailing dot on a path component, so `7.1.0.` and `7.1.0` can land in the same pair directory. Space is already rejected, so only the dot remains. Reject a trailing `.` (and arguably a leading `.`) to keep the directory name injective. Device names are not reachable because `pair_dir` always inserts `__` between the tags, so `CON` becomes `CON__7.1.0`; no action needed there.

### F7 — Metadata-only runs and the wire ceiling — **Fixed**

`plan` now applies `check_byte_ceiling` only when `!no_chunk_blobs` (`src/blobs/plan.rs:132-140`), while A12 and admission accounting still run. The comment at `:122-131` correctly explains that the projection is a distinct-chunk subset of already-charged admission bytes, so the ceiling is a guard rather than a live limit. The test `a_metadata_only_plan_is_not_refused_by_the_wire_ceiling` (`:208-244`) asserts both directions: a fetching run with `max_download_bytes = 1` still fails and names `1000 B`, and the same plan with `no_chunk_blobs = true` succeeds while still recording `projected == 1000`. `a_metadata_only_plan_still_lists_every_want` (`:246-263`) guards against “skip the ceiling” becoming “skip the plan”. Strong.

### F8 — Non-UTF-8 argv — **Fixed**

`main` now calls `recorded_argv(std::env::args_os())` (`src/main.rs:59`), and `recorded_argv` maps each `OsString` through `to_string_lossy` (`src/cli/mod.rs:409-416`). `std::env::args()` no longer appears in `src/`. The comment at `:403-408` states the trade-off honestly: a replacement character is a worse record than the real bytes, but the alternative is a panic after a successful parse. The Windows-only test in `tests/cli_contract.rs:112-200` uses an unpaired surrogate; that is the right platform-specific case.

One reproducibility note (not a defect): the recorded argv is no longer byte-exact on Unix either. If the bundle is meant to be a replayable record, consider also recording the raw bytes (e.g. base64) alongside the lossy display string. That is a schema change, not a fix for F8.

### F9 — Duplicate `--files` — **Fixed**

`validate_paths` now keeps a `seen` list and rejects a repeat with “was given twice; a repeated path adds no capture” (`src/config/mod.rs:498-503`). The comment at `:461-465` correctly explains the failure mode: two admissions, two patchmap writes (second overwrites first), and `files_captured == 2` for one file. The check runs in the same pass as the traversal/absolute/drive checks, so a duplicate is rejected before any network or directory work. `tests/cli_contract.rs` pins the usage error.

Minor: `seen` is a `Vec<&str>` with `contains`, so duplicate detection is O(n²). The list is operator-supplied and small, so this is fine; a `HashSet` would be a readability improvement, not a correctness one.

### F10 — Help text — **Fixed**

`--require-class` now says “Default: none (nothing is required unless named)” (`src/cli/mod.rs:161-162`), and `--no-chunk-blobs` says “Breaks assertions A6-A12, A19, A21-A23 by design” (`:188`). Both match the actual behavior: `require_class` is an empty `Vec` with no default, and the skip constant is exactly those 11 ids (`src/emit/selfcheck.rs:707-709`). `tests/cli_contract.rs:133` pins the `--no-chunk-blobs` sentence verbatim, which is the right kind of test for a help contract.

### F11 — Temp-file lifecycle — **Fixed**

The failure paths now `drop(out)` before `discard_tmp` on a write/seq failure (`src/blobs/mod.rs:445-454`), and `finish()`/`rename()` failures call `discard_tmp` after the handle is consumed (`:456-473`). `discard_tmp` treats `NotFound` as success and logs other failures rather than replacing the fetch error (`:492-502`). The comment at `:485-491` states the Windows rule correctly.

Residual (Low): a panic inside `get_to_file` (or inside the `HashingFile` write path) still unwinds past the cleanup, leaving `.tmp` behind. The doc says “never orphaned” (`:485`), which is stronger than the guarantee. Either soften the doc to “best-effort on every handled error” or wrap the body in a guard that removes the temp on unwind. The hole-fill already records the panic for the index, so this is hygiene, not accounting.

### F12 — Close-out test — **Partially fixed**

The old test looped over a hand-written set of eight ids, so dropping A21–A23 from the declared-skip constant was invisible. The new test (`src/emit/closeout.rs:623-670`) builds `declared` by filtering `skipped()` with `SKIPPED_WITHOUT_CHUNK_BLOBS` and compares it to the constant (`:647-658`). That removes the hard-coded subset, but the comparison is still self-referential: if the constant loses an id, both sides lose it and `assert_eq!` passes. The comment at `:638-640` (“pins each id, once”) overstates what that assertion proves.

The behavior is nevertheless covered elsewhere, so this is not a release blocker:

- Dropping an id from the constant but not from `WORDING` makes it an asserted row in the metadata-only fixture; `assert!(failing.is_empty())` at `:660-667` then fails (assuming the minimal fixture cannot satisfy A21–A23, which it cannot).
- Dropping it from both is caught by `src/emit/selfcheck.rs:1012-1016` (`the_three_coverage_assertions_are_present_and_named`) and by `tests/capture_layout.rs:977-985`, which pins the exact 21-id set for a real capture.

Recommendation: replace the filter with an explicit expected literal (or assert `SKIPPED_WITHOUT_CHUNK_BLOBS.len() == 11` plus `skipped()` set-equality), and rewrite the comment to say what the assertion actually proves. That turns the test into the pin its comment claims.

---

## Decision review

The decisions are mostly sound, and the comments document the rejected alternatives. Points worth challenging:

1. **“Record and continue” for forced refusals (F2)** — correct. A refusal is a per-path fact. The only thing to watch is that phase 2 can re-refuse the same path; the path-based dedup at `:695-701` handles it. Do not change this to “stop at first refusal” for fail-fast aesthetics: the caller explicitly named the paths, and the report is the product.

2. **`forced_unscored` as a separate field (F3)** — correct, and better than folding it into `forced_skipped`. “Absent from the manifests” and “present but answers no class” are different facts, and `tests/matrix.rs:707-710` explicitly asserts they stay different. Keep it.

3. **Skipping the wire ceiling for `--no-chunk-blobs` (F7)** — correct, with one caveat the code already acknowledges: admission still charges the same bytes, so the ceiling is not bypassed, only not double-applied to the projection. If admission charging ever changes, this skip could become a real hole; the comment at `:127-131` is the guard. Add a test that ties the two numbers together if the charging model changes.

4. **Capping error bodies while preserving status (F5)** — correct. The alternative (turning an over-cap error body into `Shape`) would hide the status; the alternative (turning a truncated read into `Transport`) would retry a deterministic 404. The chosen split is the right one.

5. **Lossy argv recording (F8)** — acceptable, but it is a data-fidelity regression for reproducibility. The comment is honest. If argv is ever used to replay a capture, this needs a raw-bytes companion field.

6. **Rejecting duplicate `--files` (F9)** — correct and consistent with `--audio`. The failure mode it prevents (double write, double count) is worse than the inconvenience.

7. **The F12 test rewrite** — directionally right, but the assertion should be literal. A test whose expected value is derived from the same constant it is testing is a tautology; the current suite only works because a *different* assertion (the failing-row check) catches the mutation. Make the intent explicit.

---

## New and remaining findings

Ordered by severity. Only the first is release-blocking.

### 1. High — API-resolved tags still reach the path sink unvalidated (F6 residual)

`src/config/mod.rs:96` and `:100-107` accept `params.tag` / `params.diff_tags[0]` verbatim; `src/run/mod.rs:124` clones them; `:240` passes them to `artifact_dir`; `pair_dir` (`src/config/mod.rs:628-631`) joins them as a path component. A malicious API response can escape the artifact root on Windows (`..\..\..`, `C:`), and on Unix a `..` component would do the same if separators were ever allowed. The flag path is validated; the API path is not. **Fix before shipping**, with a unit test on `VersionPair::select` using hostile `BranchParams`.

### 2. Medium — Windows-reserved characters in tags fail late

`validate_tag_shape` accepts `* ? " < > |`. These are not traversal, but they make `create_dir_all` fail on Windows after the network work. Reject them in the same validator, or document that tags are assumed to be POSIX-safe and the run will fail at directory creation.

### 3. Low/Medium — Trailing-dot tag collision on Windows

`7.1.0.` and `7.1.0` are distinct strings but the same directory on Windows (trailing dots are stripped). Two captures can collide in one pair directory. Reject a trailing `.` (and consider a leading `.`).

### 4. Low — `fill.rs` documents the wrong error order

`src/blobs/fill.rs:25` says “in `wants` order”; `fill` pushes errors as `join_next` completes (`src/blobs/mod.rs:269-278`), i.e. completion order. Fix the doc or sort before returning. If any consumer ever zips `errors` with `wants`, this becomes a real bug.

### 5. Low — Panic payload discarded in `fill`

`Err(_) => {}` (`src/blobs/mod.rs:282`) throws away the `JoinError`. The count is correct, but the operator cannot see why the task panicked. Log it (debug/warn) before falling through to the generic hole-fill message.

### 6. Low — `drain_error_body` swallows mid-read failures

`Err(_) => break` (`src/http/fetcher.rs:220`) keeps the status but records nothing about the read failure. Add a debug/warn log with the URL and error. Also consider whether `bytes_in = 0` on the declared-over-cap path (`:197`) is the right schema value; it is documented but surprising.

### 7. Low — `Budget::with_spent` is dead and unclamped

`src/select/budget.rs:45-47` is no longer called anywhere in `src/` (only the review document mentions it). It also allows `spent > limit`, which silently refuses every admission. Delete it, or make it `debug_assert!(spent <= limit.unwrap_or(u64::MAX))` and document it as test-only.

### 8. Low — `matrix.json` uses `bytes_admitted`, not `budget.spent()`

`src/emit/mod.rs:237` writes `"spent": selection.bytes_admitted`, while `Selection` also carries `budget: final_budget` (`src/select/mod.rs:799`). Today they are equal by construction (`:793` sums the admitted costs, and `admit_in_order` charged the same costs), but there are now two representations of one fact. Prefer `selection.budget.spent()` for the document, or add an assertion/test that the two are equal, so a future admission path cannot make them diverge silently.

### 9. Test gap — no end-to-end nonzero `forced_files_unscored`

`tests/matrix.rs:707-710` asserts only `== 0`. `tests/measured_fixture.rs:846` covers the selector. The run/report wiring (`src/run/mod.rs:399`, `src/report/mod.rs:647`) has no integration test with a nonzero value, and there is no forced + budget integration test that checks `forced_files_unscored` alongside `forced_files_skipped`/`forced_files_requested`. Add one capture fixture with a degenerate forced path.

---

## Test adequacy

**Ran:** `cargo test -p gircap` → **637 passed, 0 failed** (504 lib + 133 integration: 3 `bundle_consistency`, 8 `bundle_diff`, 29 `capture_layout`, 2 `case_coverage`, 30 `cli_contract`, 8 `matrix`, 29 `measured_fixture`, 9 `offline_bundle`, 15 `schema`).

Assessment of the regression tests that matter:

- **F1, F2, F3, F4, F5, F7, F8, F9, F10** each have a test that constructs the failure mode and would fail on the pre-fix code. F7 is the strongest (asserts both the fetching-run refusal and the metadata-only success). F4’s test uses a real unbindable socket, not a mock of the branch, so it exercises the actual `fill` loop.
- **F6** has flag-path tests but no test for API-resolved tags. That is the gap that lets the High finding through. A test at the `VersionPair::select` boundary is cheap and should be added with the fix.
- **F11** has no direct test that the temp file is gone after a failed fetch; the fix is verified by inspection and by the Windows handle rule. A test that asserts `!tmp_path.exists()` after a simulated write/rename failure would be worth adding, especially since the panic path is still uncovered.
- **F12** is only partly self-verifying, as described above. The suite-level pinning exists (`selfcheck.rs:1012-1016`, `capture_layout.rs:977-985`), so the risk is a misleading test comment rather than an unguarded constant.
- **Vacuous-test check:** no other new test appears to assert a value derived from the code under test without an independent oracle. The F7 tests use explicit `1`/`1000` numbers; the F1/F2 tests use explicit 60/500/100/200 numbers; the F5 tests use explicit caps and status codes. Good.

---

## Commands run

```text
cargo test -p gircap
# 637 passed; 0 failed; 0 ignored
```

No other build/lint commands were run. The working tree was left untouched; no branch checkout, worktree, or subagent was used.

---

## Ship verdict

**Conditional ship — do not close F6 yet.**

The accounting/reporting half of the series (F1–F5, F7–F11) is implemented correctly, the fixes are cumulative where they need to be, and the regression tests are real. The remaining defects are mostly documentation, logging and dead-code hygiene, plus one integration-test gap.

The blocker is **F6b**: `VersionPair::select` still takes API-supplied tags into `artifact_dir`/`pair_dir` without the validation the flag path now has. Until that is closed, a hostile or compromised API response can write outside `--out`, and the review cannot claim F6 is fixed. The fix is small (validate `params.tag` and `params.diff_tags[0]` in `select`, reject reserved characters/trailing dot, add a hostile-`BranchParams` test), and it should land before this batch is described as complete.

After that, land the Low items opportunistically: fix the `fill.rs` order comment, log the `JoinError`, log the `drain_error_body` read error, delete `Budget::with_spent`, and make the F12 assertion literal.
