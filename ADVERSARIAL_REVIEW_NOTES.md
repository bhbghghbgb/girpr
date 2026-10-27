# Reviewer's notes — `girpr` fixes for F‑1…F‑9

- **Project:** `girpr`
- **Branch:** `fix-gaps/girpr`
- **Reviewed tip:** `48e7dd2`
- **Baseline:** `64dc69d`
- **Committed:** with the branch-reference correction on this branch (the reviewed tip above predates this commit).
- **Document type:** reviewer's notes — adversarial review of the F‑1…F‑9 fixes

---

# Adversarial review — `girpr` fixes for F‑1…F‑9

**Scope.** Reviewed `ADVERSARIAL_REVIEW.md` in full and the actual change set `git diff 64dc69d..HEAD -- src tests docs Cargo.toml` on branch `fix-gaps/girpr` (reviewed tip `48e7dd2`, baseline `64dc69d`). Every claimed fix was checked at the source site, not just in the §7 progress table. No tracked files were modified.

---

## 1. Verdict summary

| # | Sev. | Claimed | Verdict | Primary evidence | Would the named regression test fail without the fix? |
|---|---|---|---|---|---|
| **F‑1** | High | Fixed | **Fixed** — core bug closed; end‑to‑end test does not actually pin it | `src/repair/file/mod.rs:294`, `:349-359`, `:311-340`; `src/repair/file/temp.rs:197-205`; `src/repair/file/mod.rs:769-783`, `:788-804`; `tests/repair_offline.rs:562-604` | Unit test: **yes**. Integration test: **no** — it passes via `validate_chunk_ranges` returning `Err`, not via a panic (see F‑1). |
| **F‑2** | High | Fixed | **Partially fixed** — all listed sinks are contained, but the containment helper fails open on non‑`NotFound` canonicalize errors | `src/util.rs:298-343`, `:351-373` (esp. `:362`); sinks at `plan.rs:194`, `file/mod.rs:390`, `scan.rs:160`, `check.rs:47/67`, `metadata.rs:123/129/140`, `post.rs:142/197-199`, `scope/report.rs:78`; tests `tests/repair_offline.rs:458-559`, `src/util.rs:693-745` | Sink tests: **yes** (escape would write/delete/move outside). Fail‑open branch: **untested**. |
| **F‑3** | Medium | Fixed | **Partially fixed** — consumed bytes are now charged, but the “ceiling” can still be exceeded and the run exits `0` | `src/repair/file/temp.rs:354-362`, `:370-392`, `:433-442`; `src/sophon.rs:527-590`, `:566-575`; `tests/repair_offline.rs:1928-1944`; `README.md:289`; `docs/04-scope-limitation-spec.md:163-170` | **Yes** for the accounting half (live would be only `compressed`). The test also **pins the overrun** (`live() > compressed`, `code == 0`). |
| **F‑4** | Medium | Fixed | **Fixed** | `src/repair/phases/metadata.rs:150-176`; `tests/repair_offline.rs:2113-2165` | **Yes** — without the gate the game/scan dirs are created. |
| **F‑5** | Medium | Fixed | **Fixed** | `src/repair/phases/check.rs:37-64`; `tests/repair_offline.rs:195-…` | **Yes** — absent empty file would be reported clean (exit `0` vs expected `4`). |
| **F‑6** | Low | Fixed | **Fixed** (log‑only, no test) | `src/repair/scan.rs:130-142`, `:86-93` | **N/A** — prose/log change; no regression test possible through a public seam. |
| **F‑7** | Low | Fixed | **Fixed** | `README.md:289-290`; `docs/04-scope-limitation-spec.md:163-170`, `:286-294`; `docs/03-repair-rust-implementation-plan.md:169-176`; CLI/limits/report/ledger sites inspected | **N/A** — prose only. |
| **F‑8** | Medium | Fixed | **Partially fixed** — five of six gaps are genuinely covered; the panic→exit‑3 gap is not, and Windows‑specific containment is still unix‑only | `tests/repair_offline.rs:458-559`, `:562-604`, `:1896-1949`, `:2113-2165`, `:374-454`; `src/util.rs:733-745` | Mixed — see F‑8. |
| **F‑9** | Low | Fixed | **Partially fixed** — the cited comment and the `kept_bytes == 64` assertion are correct, but the canonical field doc still describes the old semantics | `src/repair/file/mod.rs:261-271`; `tests/repair_offline.rs:373-412`; `src/report.rs:297-299` | The assertion pins behaviour, not the comment; the stale field doc is untested and wrong. |

---

## 2. Per‑finding analysis

### F‑1 — Panicked file tasks exit `0` — **Fixed, with a test that proves less than claimed**

**What changed.**
- `repair_all` now calls `join_file_tasks(handles, &summary)` (`src/repair/file/mod.rs:294`).
- `join_file_tasks` awaits every handle and turns any `JoinError` (panic or cancellation) into `files_failed += 1`, logs it, and aborts under `--fail-fast` (`:349-359`).
- The old `i64` end computation is gone; `validate_chunk_ranges` rejects negative, overflowing, and out‑of‑file ranges before any seek (`:311-340`), and `temp.rs:197-205` uses `u64::try_from(...).checked_add(...)`.
- Unit coverage: `a_panicked_file_task_is_counted_as_a_failure` (`:769-783`) and `a_chunk_past_the_files_end_is_refused` (`:788-804`).

**Why the core fix is real.** Before the fix, `let _ = h.await` dropped a panic; `files_failed` stayed `0`, the post phase bumped `config.ini`, and the process exited `0`. The new join loop makes a panic observable and feeds the existing “failed file → exit 3 / no config bump” path. The unit test directly fails if `join_file_tasks` is reverted to a discard.

**The test gap.** `tests/repair_offline.rs:562-604` is named “fails the file instead of panicking”, but with the current code it never reaches a panic: `validate_chunk_ranges` returns `Err` first. Worse, it would still pass if **either** half were reverted in isolation:
- revert only the join fix → `validate_chunk_ranges` still returns `Err` → exit 3, failed 1 → test passes;
- revert only the range fix → the task panics → the new join counts it → exit 3, failed 1 → test passes.

The test only pins the *pair*. It also never asserts that `game/foo.dat_tmp` is absent or that stderr says “range overflows”, so a panic path that leaves a `_tmp` behind would still pass. The unit test pins the helper, not its wiring into `repair_all`; changing line 294 back to `let _ = h.await` breaks no test.

**Verdict:** Fixed. The implementation is correct; the regression claim is overstated. Add an end‑to‑end assertion that distinguishes the range error from a panic (e.g. stderr contains `chunk ... range overflows`, no `_tmp`), or a `#[cfg(test)]` panic injection seam in `repair_one_file`.

---

### F‑2 — Uncontained server paths — **Partially fixed: sinks closed, helper fail‑open**

**What changed.** `contained_rel` (`src/util.rs:298-321`) rejects empty, drive‑prefixed (`C:`, `C:/`), absolute/root/UNC, `ParentDir`, and non‑`Normal` components. `contained_join` (`:331-343`) joins and then compares the resolved nearest existing ancestors with `starts_with`. It is applied at every sink the finding names:
- plan: `plan.rs:194-195` (now fallible; bad rel = metadata, exit 2);
- repair target and `_tmp`: `file/mod.rs:390`, `:412`;
- scan read: `scan.rs:160-179`;
- check: `check.rs:47`, `:67`;
- config/blacklist/audio‑scan dirs: `metadata.rs:123`, `:129`, `:140`;
- audio read/write: `audio.rs:66`, `:101`;
- deprecated deletes: `post.rs:142-154`;
- audio cache/res move: `post.rs:197-215`;
- scope report: `scope/report.rs:78`.

The three integration tests (`tests/repair_offline.rs:458-482`, `:485-520`, `:523-559`) assert the outside file survives / exit 2 / no game dir, and they would fail without containment.

**Residual issue — the helper fails open.** `resolve_existing_prefix` (`src/util.rs:351-373`) catches **every** `canonicalize` error:

```rust
Err(_) => match (cur.parent(), cur.file_name()) {
    (Some(parent), Some(name)) => { missing.push(name.to_os_string()); cur = parent; }
    _ => return path.to_path_buf(),
},
```

Only `NotFound` means “this component does not exist yet”. `PermissionDenied`, sharing violations, and other errors mean “I could not resolve this component”, and the code silently treats the component as a literal name. The result is appended lexically and compared with `starts_with`, so a symlink/junction whose target cannot be canonicalized (e.g. an ACL that denies the attributes read but permits the write) can pass the containment check. A dangling link is likewise misclassified, though the subsequent `create_dir_all`/open usually fails rather than escaping. This is exactly the “lexical check pretending to be a filesystem check” failure the fix was meant to remove.

**Residual issue — sibling validation timing.** `metadata::fetch` validates `res_category_dir`, `blacklist_dir`, and `audio_pkg_scan_dir` (`metadata.rs:120-143`), but **not** `audio_pkg_cache_dir` / `audio_pkg_res_dir`. The post phase does contain them (`post.rs:197-215`) and skips with a warning, so there is no escape — but a hostile value is accepted through metadata, `config.ini` is bumped, and the run exits `0` after skipping the move. That is inconsistent with the “server data is metadata, refused before mutation” rule applied to the other dirs.

**Residual issue — no Windows test.** The only symlink test is `#[cfg(unix)]` (`src/util.rs:733-745`). The threat model is Windows game installs; a junction/reparse‑point test is the missing one. The temp‑directory probes done during this review confirmed junctions are resolved and refused, but that is not a committed regression test.

**Verdict:** Partially fixed. The original escape paths are closed; the guarantee is not yet fail‑closed. Make `resolve_existing_prefix` return `Result<PathBuf>` and propagate any error other than `NotFound`; validate cache/res at metadata time; add a Windows junction test (and a denied‑ACL or non‑`NotFound` test).

---

### F‑3 — Failed fetches refund bytes that crossed the wire — **Partially fixed: accounting yes, ceiling no**

**What changed.** `write_downloaded_chunk` reserves `compressed_size` before fetching (`temp.rs:354-362`), passes `&mut consumed` into `stream_chunk_to`, and calls `settle_reservation(ctx, reserved, consumed)` on both success and failure (`:370-392`). `stream_body` caps each body at the reservation (`sophon.rs:566-575`), and `settle_reservation` folds `consumed - reserved` into the live meter (`temp.rs:433-442`). The integration test asserts `live == compressed + cut` and `settled == compressed`, and that the chunk was retried (`tests/repair_offline.rs:1936-1949`).

**Why it is only partial.** The reservation is taken **once per call** (`temp.rs:354-355`), and each retry inside `stream_chunk_to` can deliver a partial body up to the cap. The module comment now admits the bound: up to `ATTEMPTS` bodies per call, `ATTEMPTS × MAX_ATTEMPTS` per chunk (`temp.rs:422-432`). There is no check after `settle_reservation` that `live` is still at or below the limit. Consequently:

- `--max-download-bytes N` can be exceeded by up to one partial body (and, in principle, much more across retries);
- if the overrun happens on the last chunk, the run **exits `0`**.

The new test pins this: `assert!(summary.download.live() > compressed)` and `assert_eq!(code, 0, ...)` (`tests/repair_offline.rs:1941-1944`). That directly contradicts `README.md:289` (“hard-stop mid-flight (exit `5`, `SUMMARY stopped=--max-download-bytes`)”), `ledger.rs:301-308` (“a ceiling, not a guideline”), and `docs/04:163-170` (“Both produce exit `5`”).

**Verdict:** Partially fixed. The accounting bug (free partial bodies) is fixed; the enforcement contract is now knowingly broken and blessed by a test. Either check `live` after settling and set `StoppedBy::Download` (exit 5), or rewrite the docs and the test to describe a “soft ceiling with an in‑flight overrun bound” — do not leave the contradiction.

---

### F‑4 — `--print-plan` + explicit `--audio` writes the scan file — **Fixed**

`metadata.rs:150-153` gates the write and the scope refusal on `!ctx.readonly() && !ctx.print_plan`. The audio set is still resolved for the report (`:148-149`). `tests/repair_offline.rs:2113-2165` asserts exit `0`, no game dir, no `Audio/` dir, and `audio_langs=en-us` in stdout. Without the `!ctx.print_plan` term, `readonly()` (`config.rs:181-183`) is false for print‑plan, so the scan write and `create_dir_all` would run and the test would fail.

**Residual design smell:** `readonly()`’s doc says “Every delete/write in the pipeline is gated on this” (`config.rs:177-180`), but print‑plan is read‑only by construction and is not in `readonly()`. The code now gates on `readonly() || print_plan` in three places (`metadata.rs:151-152`, `repair/mod.rs:223`, `:354`). That is correct today, but the next writer who trusts the doc will reintroduce F‑4. Update the doc or add `print_plan` to the predicate.

**Verdict:** Fixed.

---

### F‑5 — `--check-only --scan` can report an absent zero-length file as clean — **Fixed**

The scan branch now checks presence/length before trusting the scan digest (`check.rs:47-51`), matching the fallback branch (`:67-72`), and `Verdict.len` is gone. `tests/repair_offline.rs:195-…` asserts absent → exit `4` in both spellings and present‑empty → `0`; without the length check the absent file would compare its empty digest to the manifest’s empty digest and be called clean.

**Residual (low):** `contained_join(...).ok()` at `check.rs:47` means a containment failure in the scan branch is reported as `CHECK fail: size` (`:51`), not as an escape. The fallback branch distinguishes it (`:83-87`). Unreachable after `build_plan`, but it is a misleading diagnostic in a defence‑in‑depth path.

**Verdict:** Fixed.

---

### F‑6 — Panicked scan tasks dropped silently — **Fixed**

`scan_plan` now logs a warning with the join error and states that unreached files are estimated in full (`scan.rs:130-142`); `est_download_bytes` treats a missing verdict as “download everything” (`:86-93`). No test was added, and the claim says so. The warning does not name the file — the panic means the `Scanned.rel` is unavailable — but the finding only asked for a trace. A counter (`scan_tasks_failed`) would make it machine‑visible; optional.

**Verdict:** Fixed.

---

### F‑7 — `--max-write-bytes` documented as `*_tmp` only — **Fixed**

The six prose sites named in the finding now say “the install — the target for the in‑place path, `{path}_tmp` for the temp path”. `README.md:289-290`, `docs/04:163-170`/`:286-294`, and `docs/03:169-176` are consistent. `docs/03:176` is a column‑specific table row (in‑place vs temp), not a residual error. No behaviour changed; no test needed.

**Verdict:** Fixed.

---

### F‑8 — Regression coverage gaps — **Partially fixed**

What is genuinely covered:
- manifest escape (`tests/repair_offline.rs:458-482`) — `../`, `C:evil.dat`, `/abs/evil.dat`, exit 2, no game dir;
- deprecated escape (`:485-520`) — outside file survives, `deleted_extra_bytes == 0`;
- audio cache escape (`:523-559`) — nothing moved into the install;
- partial body (`:1896-1949`) — live includes the partial, settled does not, one retry;
- print‑plan audio (`:2113-2165`) — no game/scan dir, language named;
- full‑length corrupt `_tmp` (`:415-454`) — deleted, retried, rebuilt, both chunks fetched.

What is not:
1. **Panic → exit 3 is not end‑to‑end.** As in F‑1, the integration test exercises `validate_chunk_ranges`, not a panic; the unit test exercises `join_file_tasks` directly, not `repair_all`’s call site. The claim “panic → `3`” is not pinned.
2. **Windows‑specific containment is unix‑only.** `src/util.rs:733` is `#[cfg(unix)]`; no junction/reparse‑point test. The unit list covers `C:`, `C:/`, UNC, `../`, `/abs` (`:698-714`), but those are lexical spellings, not filesystem resolution.
3. **No test for the fail‑open canonicalize branch** (F‑2 residual).
4. **No test for audio cache/res metadata validation** (because the validation is missing).
5. **The partial‑body test pins the overrun** rather than the intended “ceiling” contract (F‑3).

**Verdict:** Partially fixed. Five of six gaps are real; the most important one (the original High‑severity panic path) is still not proven through the production wiring.

---

### F‑9 — `kept_bytes` comment wrong under temp resume — **Partially fixed**

The cited comment in `src/repair/file/mod.rs:261-271` now correctly says the temp path rewrites every chunk it does not resume and that an earlier `_tmp` prefix counts as kept. `partial_tmp_prefix_is_resumed` sets `repair_mode_explicit` and asserts `kept_bytes == 64` (`tests/repair_offline.rs:373-412`), and `a_full_length_corrupt_tmp_is_deleted_and_rebuilt` pins the final‑MD5 gate (`:415-454`).

But the canonical field doc still says:

```rust
/// Bytes that were already correct where they belonged and were **not**
/// written. ...
pub kept_bytes: AtomicU64,          // src/report.rs:297-299
```

A resumed `_tmp` prefix is **not** “already correct where it belonged” — it is unverified bytes that the repair chose not to rewrite, and the test itself says “the prefix was kept, not rewritten” (`tests/repair_offline.rs:393-399`). The field doc is now the stale sibling of the comment that was fixed. Since F‑7’s lesson was “fix all sites”, this is a partial fix.

**Verdict:** Partially fixed. Update `report.rs:297-299` to say “bytes not rewritten, including a resumed `_tmp` prefix that the final MD5 gate has not yet verified”.

---

## 3. Decision review

1. **Rejecting the `Err(e) if e.is_panic() => {}` arm — sound.** Counting every `JoinError` is the only way to guarantee a panicked file is not a silent success. Counting cancellation as failure is acceptable (a cancelled file task means the run did not repair it). Keep it.
2. **Replacing `validate_rel` with `contained_rel` / `contained_join` — sound in shape, incomplete in error handling.** Applying the check at the plan **and** every sink is the right defence‑in‑depth choice, and making `build_plan` fallible (metadata, exit 2) is correct. But the helper’s `Err(_)` branch undermines the “filesystem check” claim. The decision is right; the implementation of one branch is not.
3. **F‑3’s “correct the bound, don’t enforce it” decision — questionable.** The comment now tells the truth about the bound, but the product contract still says “ceiling” and “hard stop / exit 5”. A test that asserts `live > limit` and `code == 0` converts a bug into a specification. Either enforce the ceiling after settling or re‑document it as a soft budget. This is the main decision I would send back.
4. **F‑5’s “use or remove `Verdict.len`” — sound.** Removing the dead field and checking length + digest in both branches is the right minimal fix; it also removes a source of “the scan knows a length the checker ignores”.
5. **Not re‑landing `layout`’s dead `checked_add` — sound.** Presenting unreachable arithmetic as a fix would have been worse than the bug. The real range check is in `validate_chunk_ranges` and `temp.rs`.
6. **Commit‑per‑finding with tests in the fix commit — mostly sound, not uniformly true.** F‑6 has no test (acceptable for a log line), F‑9’s test pins behaviour but not the doc, F‑8’s panic test does not exercise the wiring, and F‑3’s test pins the overrun. The process is good; the “all six gaps covered” claim is the part that needs correction.
7. **F‑4’s minimal gate vs. changing `readonly()` — sound, but document it.** Adding `!ctx.print_plan` at the write site is the smallest safe change. The alternative (put `print_plan` into `readonly()`) would touch more call sites and could change lock/purge behaviour. Keep the minimal gate; fix the `readonly()` doc so the next writer does not trust a false invariant.
8. **F‑2’s post‑phase skip for audio cache/res — safe but inconsistent.** Skipping an escaping move is the right fail‑safe behaviour at the sink. But the same strings are validated as metadata for `res_category_dir`, `blacklist_dir`, and `audio_pkg_scan_dir`; cache/res should be too, so the run refuses before bumping `config.ini` instead of reporting success after a warning.

---

## 4. New and remaining findings

| ID | Sev. | Location | Issue | Suggested fix |
|---|---|---|---|---|
| **N‑1** | Medium (security) | `src/util.rs:351-373` | `resolve_existing_prefix` treats every `canonicalize` error as “missing”, so a symlink/junction whose target cannot be resolved is compared lexically; containment can fail open. | Only `ErrorKind::NotFound` walks up; propagate other errors from `contained_join`. Add a non‑`NotFound` test. |
| **N‑2** | Medium (contract) | `src/repair/file/temp.rs:354-362`, `:433-442`; `tests/repair_offline.rs:1928-1944` | `--max-download-bytes` can be exceeded by in‑flight partial bodies and the run exits `0`; the test asserts the overrun. Contradicts `README.md:289` and `docs/04:163-170`. | After `settle_reservation`, if `live > limit`, set `StoppedBy::Download` and stop (exit 5); or re‑document as a soft ceiling and rename the test. |
| **N‑3** | Medium (data loss) | `src/repair/phases/post.rs:247-252` | If `rename` fails and `copy` fails, the code still `remove_file(&src)` — the cache file is deleted without a destination. Both errors are swallowed (`let _`). | Only remove after a successful copy; log/count failures. The module doc says “Every failure here is fatal” (`post.rs` header), so either honour that or soften the doc. |
| **N‑4** | Medium (consistency) | `src/repair/phases/metadata.rs:120-143` vs `post.rs:186-215` | `audio_pkg_cache_dir` / `audio_pkg_res_dir` are not validated at metadata time. A hostile value mutates `config.ini`/scan, then the post phase skips with a warning and exit `0`. | Validate both at metadata like `audio_pkg_scan_dir`; add an integration test (exit 2, no config bump). |
| **N‑5** | Medium (concurrency) | `src/repair/phases/plan.rs:183`, `:197` | `seen: HashSet<String>` is case‑sensitive, but Windows paths are not. `game/Foo.dat` and `game/foo.dat` are two plan entries writing the same file concurrently. The scope is case‑insensitive, so this is inconsistent with the rest of the tool. | Dedup on a case‑folded key (keep the original rel for the path); add a two‑entry fixture test. |
| **N‑6** | Low (data loss) | `src/util.rs:314-315`; `plan.rs:196`; `purge.rs:80`, `:128-130`, `:273` | `contained_rel` accepts `./a/b` and `a/./b`. `build_plan` stores the rel verbatim, so `expected_paths()` contains `./game/foo.dat` while the purge walk produces `game/foo.dat`; `--purge-after` can classify the just‑repaired file as an orphan and delete it. Also creates duplicate plan entries. | Strip `CurDir` components (or reject them) before storing; add a purge test with a `./` manifest path. |
| **N‑7** | Low (fail‑fast) | `src/repair/file/mod.rs:349-359` | For a panic, `abort()` is only set when that task’s handle is reached in spawn order. If an earlier handle is slow, queued files can still start; fail‑fast is not prompt for panics (ordinary `Err` paths abort inside the task, so they are fine). | Use `JoinSet` / `FuturesUnordered`, or wrap the task body in `catch_unwind` and abort there. |
| **N‑8** | Low (diagnostic) | `src/repair/phases/check.rs:47`; `src/repair/scope/report.rs:78` | `.ok()` mislabels a containment failure as `CHECK fail: size`; `DiskPlan::new` silently drops the file. Both are unreachable after `build_plan`, but they are defence‑in‑depth paths. | Match the error explicitly and log “escapes the game dir”, as the fallback branch does. |
| **N‑9** | Low (dead code/doc) | `src/util.rs:277-279`; `src/repair/scope/filter.rs:10-12`; `docs/04:191-193` | `normalize_rel` now has no callers (only its definition and a doc reference). The docs still say it “converts at the filesystem boundary”, but the boundary now uses `contained_rel`/`contained_join`. | Remove `normalize_rel` or use it; update the two docs. |
| **N‑10** | Low (doc) | `src/report.rs:297-299` | `kept_bytes` is still documented as “already correct where they belonged”, which is false for a resumed unverified `_tmp` prefix. | Reword per F‑9. |
| **N‑11** | Low (swallowed errors) | `src/repair/phases/post.rs:158`, `:161`, `:221`, `:250-251`; `src/repair/file/temp.rs:465`; `src/repair/purge.rs:174`, `:195` | Delete/copy/walk failures are swallowed (`is_ok()`, `unwrap_or(0)`, `let _`, `filter_map(|e| e.ok())`). `post.rs`’s module doc claims every failure is fatal; it is not. | Log at `warn!`, count failures, and at minimum fix N‑3’s delete‑after‑failed‑copy. |
| **N‑12** | Low (panic path) | `src/repair/file/mod.rs:212` | `sem.acquire_owned().await.unwrap()` panics if the semaphore is ever closed. The new join handler counts it, so it is no longer silent, but a panic is a poor control path. | `expect("file semaphore is never closed")` or handle the `Err` as a stopped task. |
| **N‑13** | Low (overflow) | `src/repair/scan.rs:75-95`; `src/repair/scope/limits.rs` estimates | `u64` `.sum()` over `compressed_size.max(0) as u64` can overflow with three or more chunks at `i64::MAX` (two fit, three do not): debug panic, release wrap. Server‑controlled manifest. | `checked_add`/`saturating_add` with a metadata error. |
| **N‑14** | Low (Windows names) | `src/util.rs:298-320` | `contained_rel` accepts a trailing separator (`"game/"`) and Windows device names (`NUL`, `CON`, `COM1`) and trailing dot/space final components. No escape, but writes can target a device or alias another file. | Reject a trailing separator and reserved device names, or document the behaviour. |

**Top three to fix first:** N‑1 (fail‑open), N‑2 (ceiling contract), N‑3 (delete after failed copy). N‑4 and N‑5 are the next tier.

---

## 5. Test adequacy

**Strong.**
- The F‑2 sink tests assert the *effect* (outside file survives, no game dir, `deleted_extra_bytes == 0`), not just the return code. These would fail on the old code.
- F‑4 asserts the absence of both the game dir and the scan dir, plus the report still names the language — a good “no write, but no information loss” pair.
- F‑5 tests both spellings (`--scan` and fallback) and both states (absent vs present‑empty), which is exactly the asymmetry the bug was about.
- F‑9’s resume test asserts the actual counter (`kept_bytes == 64`) and the promoted bytes, not just exit `0`.

**Weak / vacuous.**
- **F‑1’s integration test is mislabelled.** It proves the range error path, not the panic path. The unit test proves the helper, not the wiring. A mutation that reverts line 294 to `let _ = h.await` is invisible to the suite. This is the most important test gap because F‑1 was High.
- **F‑3’s test asserts the bug.** `live() > compressed` plus `code == 0` is a specification of the overrun, not a regression test for a ceiling.
- **F‑6 has no test.** Acceptable for a log line, but it means the “Fixed” status is a code‑reading claim.
- **F‑2’s filesystem test is unix‑only.** The product is Windows‑first; the junction case is the one that matters and is not committed.
- **No test for the fail‑open branch.** A test that makes `canonicalize` fail with a non‑`NotFound` error (e.g. a file used as a parent component, or a denied directory on Unix) would at least pin the fail‑closed behaviour once N‑1 is fixed.
- **No test for `./` plan rels** (N‑6) or case‑insensitive duplicates (N‑5).

The suite is green, but green here means “the happy paths and the named escapes are covered”, not “the security property is mutation‑proof”.

---

## 6. Commands run and results

| Command | Result |
|---|---|
| `cargo test -p girpr` | **286 passed, 0 failed, 7 ignored** (156 lib + 7 artifact_server + 16 cli_contract + 25 repair_machinery + 82 repair_offline) |
| Focused lib tests for the new units | **9 passed** |
| Focused integration tests for the eight new regressions | **8 passed** |
| `cargo clippy -p girpr --all-targets` | **clean** |
| `cargo fmt -p girpr --check` | **clean** |
| Windows filesystem probes in `E:\Symlinks\Temp\opencode\contain_exp` | `.. ` trailing‑space components are **not** an escape via Rust `std` (write fails or stays inside); a junction inside the base **is** resolved and refused by `contained_join`; trailing dot/space final components alias inside the base only |

The probes were ad hoc and are not part of the repo. The test/clippy/fmt results above are the full run for the branch, not a subset.

---

## 7. Overall verdict

**Ship with follow-ups.** The High‑severity issues are directionally fixed: panic joins are counted, every listed server‑path sink is contained, the print‑plan audio write is gated, and the check‑only length bug is closed. The suite is green and the fixes are mostly minimal and well‑placed.

But I would not call the work “done” yet. The containment helper fails open on non‑`NotFound` errors, the download “ceiling” can be exceeded with exit `0`, and the audio move can delete a source after a failed copy. The F‑8 claim is also overstated: the panic→exit‑3 regression is not proven through the production wiring.

**Minimum blocking items before merge:**

1. **N‑1 — make `resolve_existing_prefix` fail closed.** Only `NotFound` may walk up; propagate every other `canonicalize` error from `contained_join`. This is a small, localized change and the one I would insist on for a High‑severity containment fix.
2. **N‑2 — resolve the `--max-download-bytes` contradiction.** Either stop the run with `StoppedBy::Download` (exit 5) after a settle that crosses the limit, or rewrite `README.md:289`, `docs/04:163-170`, `ledger.rs:301-308`, and the test so it no longer claims a hard ceiling. Do not leave a test that asserts `live > limit` and `code == 0`.
3. **N‑3 — do not delete the audio cache source when the copy fails.** Check the copy result before `remove_file`; log/count failures, or make the post phase fatal as its module doc claims.

4. **F‑1/F‑8 test wiring — pin the panic path through `repair_all`.** Assert the non‑panic diagnostic (stderr “range overflows”, no `_tmp`) or add a test‑only panic injection. Otherwise a future `let _ = h.await` silently restores the original High bug, and F‑8 should be marked Partially fixed rather than Fixed.

**Recommended but non‑blocking:** N‑4 (validate audio cache/res at metadata), N‑5 (case‑insensitive dedup), N‑6 (`./` rel normalization), N‑7 (JoinSet for prompt panic abort), N‑8–N‑14 (diagnostics, dead code, docs, swallowed errors). Add the Windows junction test with N‑1; it is the missing platform‑specific proof for the fix that matters most.
