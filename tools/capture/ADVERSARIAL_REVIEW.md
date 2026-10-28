# ADVERSARIAL REVIEW — `gircap` (`tools/capture`)

**Baseline:** HEAD `64dc69d` (`revert: drop the adversarial review series`), tree
`66985753edf618610a3df629bacdf2b6b70609fa`, identical to the tree of `7f2d69c`
(`git diff --stat 7f2d69 HEAD` is empty). Reviewed 2026-10-27.

This is a review of **HEAD**, not of the reverted `f9352f7` fix series. The old
review describes `f9352f7`; several of its claims do not hold at HEAD. Where the
old series matters, the difference is called out explicitly.

## Scope & method

**Files read:** `tools/capture/src/**`, `tools/capture/tests/**`,
`tools/capture/Cargo.toml`, `tools/capture/docs/00-spec.md`,
`docs/01-artifact-schema.md`, `docs/03-design-notes.md`,
`docs/04-bundle-differ.md`, and `git show f9352f7:...` for the reverted review
and fixes.

**Method:** `git show`, `git grep`/`rg`, and file reads. No `cargo build`, no
`cargo test`, no binary run, no network. **No tests were run.** Every claim below
is **source-verified** unless it is explicitly marked **not verified**. Line
numbers are current-tree (`64dc69d`). The old review's “all tests pass, ≈622”
result is not treated as evidence for HEAD.

No source, test, Cargo, or documentation file was modified. Only this file was
created.

## Findings

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| F1 | **high** | bug | `src/select/mod.rs:511` | Forced `--files` costs are checked one-by-one against the unspent caller budget, so the sum can exceed `--max-download-bytes`. |
| F2 | medium | bug | `src/select/mod.rs:512` | A budget-refused forced path `break`s the loop; later forced paths are silently dropped and recorded nowhere. |
| F3 | medium | bug | `src/select/mod.rs:507` | A forced path whose candidate answers no matrix class is silently skipped, contradicting “still scored and still recorded”. |
| F4 | medium | bug | `src/blobs/mod.rs:272` | A failed chunk fetch loses its task index, is counted twice in `fetches_failed`, and its index row says only “not stored”. |
| F5 | medium | bug | `src/http/fetcher.rs:739` | Non-2xx bodies are drained with no `max_response_bytes` cap, contradicting the `drain_capped` and `Transport` docs. |
| F6 | medium | bug | `src/config/mod.rs:535` | Tag validation allows `\`, `:`, and backslash-separated `..` components that are interpolated raw into bundle paths. |
| F7 | low | bug | `src/run/mod.rs:222` | `--no-chunk-blobs` still fails the chunk-byte ceiling even though it fetches no chunk bytes. |
| F8 | low | bug | `src/main.rs:58` | A non-Unicode `--out`/`--log-file` reaches `std::env::args()` and panics instead of printing a summary. |
| F9 | low | bug | `src/select/mod.rs:498` | Duplicate `--files` entries are admitted twice with the same `capture_id`, producing duplicate rows and one overwritten patchmap. |
| F10 | low | doc error | `src/cli/mod.rs:161`, `:187` | `--require-class` says “Default: all” and `--no-chunk-blobs` says “A7-A10”; both contradict code and spec. |
| F11 | low | intentional-but-uncovered | `src/blobs/mod.rs:432` | A failed fetch leaves `.<id>.<slot>.tmp`; “the next run truncates it” holds only for the same id **and** slot. |
| F12 | low | test gap | `src/emit/closeout.rs:637` | The closeout test checks 8 of the 11 skipped ids and uses containment, so A21–A23 can regress silently. |

All twelve findings were fixed on `fix-gaps/gircap`; the Pass log records the commits.

---

### F1 — Forced `--files` admissions bypass the cumulative budget

**Severity: high. Class: bug. Location:** `src/select/mod.rs:511`, `:514`,
`:526`, `:652`, `:749`.

Phase 0 creates a local `Budget::unlimited()` at `:496` and then, for each forced
path, checks the **caller's** budget:

```rust
// select/mod.rs:510-514
let cost = c.budget_cost();
if !budget.admits(cost) {
    break;
}
spent.charge(cost);
```

`Budget` is `Copy` (`src/select/budget.rs:17-21`); `budget` arrives with
`spent == 0` and is never charged here. The local `spent` ledger is charged, but
`admits` is not asked about it. Consequently two forced paths costing 60 B each
are both admitted under a 100 B cap: each individually satisfies
`0 + 60 <= 100`.

Phase 2 then rebuilds the budget with the forced total:
`Budget::with_spent(budget.limit(), initial)` at `:652`. `with_spent` does not
clamp (`budget.rs:45-47`), and `admits` uses `saturating_add`
(`budget.rs:60-64`), so once `initial > limit` every want is rejected while the
forced admissions stay in `admitted`. `Selection::bytes_admitted` at `:749` is
the sum of all admitted costs and can exceed the limit. Nothing turns that into a
red row: `emit/mod.rs:235-238` writes `spent` and `limit` but no selfcheck
compares them.

`blobs::plan`’s `check_byte_ceiling` (`src/blobs/plan.rs:115-117`) is a second,
different number (`Σ compressed_size` over chunk wants, `plan.rs:24-35`), so it
may catch some cases but is not a guarantee.

**Concrete fix.** Use one ledger. Either make the parameter `mut` and charge it
in phase 0 (`if budget.admits(cost) { budget.charge(cost); ... }`) while dropping
the separate `spent`, or make the local ledger carry `budget.limit()` and check
it. Pass the
resulting total to phase 2. Add a test with two forced paths of 60 B under a
100 B cap: exactly one is admitted and `bytes_admitted <= 100`; with F2’s
`continue`, the other is in `rejected`. Consider a debug assertion or selfcheck
row for `bytes_admitted <= limit`.

---

### F2 — `break` silently drops later forced paths

**Severity: medium. Class: bug. Location:** `src/select/mod.rs:512`.

The forced loop is `for want in forced` (`:498`). On the first individually
over-cap path it executes `break`, so every later forced path is never examined.
It is not added to `forced_skipped` (that vector is only filled for paths absent
from the pool at `:504`) and it is not added to `rejected` (that vector is only
created in phase 2 at `:653`). The summary therefore reports
`forced_files_requested` from `plan.files.len()` (`src/run/mod.rs:396`) but
`forced_files_skipped` from `selection.forced_skipped.len()` (`:397`); a caller
who named `a` then `b` sees requested 2, skipped 0, one capture.

This is independent of F1: even after the cumulative ledger is fixed, `break`
still stops at the first refusal instead of letting a later, cheaper path fit.

**Concrete fix.** Replace `break` with a recorded refusal and `continue`, as
`f9352f7` attempted, **but** use the fixed cumulative ledger from F1. Add the
test `--files a --files b` under a 200 B cap with `a = 500 B` and `b = 100 B`:
`a` is in `rejected`, `b` is admitted, and neither is silently dropped. Add the
cumulative case from F1 separately; the old fix’s test only covered the first
case.

---

### F3 — A forced path with an empty class set is silently dropped

**Severity: medium. Class: bug. Location:** `src/select/mod.rs:507-509`;
supporting: `src/select/classes.rs:398`, `tests/measured_fixture.rs:855`.

The forced loop does:

```rust
// select/mod.rs:507-509
let Some(primary) = c.candidate.classes.first().copied() else {
    continue;
};
```

`Candidate.classes` is `Class::all_satisfied_by(shape)` at `:389`. That function
legitimately returns an empty vector for a degenerate patch shape; the unit test
at `classes.rs:394-399` asserts exactly this for a one-chunk file with no reuse.
The frozen corpus has a real path with that shape:
`GenshinImpact_Data/AssetBundles/data_revision` at
`tests/measured_fixture.rs:855-857`.

For such a path, `--files` produces neither an admission nor a
`forced_skipped` entry. The CLI help says `--files` paths are “Still scored and
still recorded” (`src/cli/mod.rs:165-166`). `forced_files_requested` counts the
path; `forced_files_skipped` does not. That is the same “requested but invisible”
failure F2 describes, reached through a different branch.

**Concrete fix.** Record it. Either push the path to `forced_skipped` with a
reason (“answers no matrix class”), or introduce a `forced_unscored` list with
that reason and surface it in the summary. Do not invent a synthetic primary
class: a capture with no class would make the matrix claim something false. Add a
test using the degenerate corpus path.

---

### F4 — A failed chunk fetch loses its index, is double-counted, and hides its cause

**Severity: medium. Class: bug. Location:** `src/blobs/mod.rs:272`; supporting:
`:293-298`, `:306`, `:331`, `src/blobs/fill.rs:25-32`, `src/run/mod.rs:413`.

`Store::one` returns `Result<(usize, (IndexedBlob, Vec<Set>)), String>`
(`blobs/mod.rs:326-331`). On failure the slot index is gone. `fill` handles it:

```rust
// blobs/mod.rs:269-272
match joined {
    Ok(Ok((i, entry))) => out[i] = Some(entry),
    Ok(Err(e)) => errors.push(e),
```

`out[i]` stays `None`. The `missing` scan at `:293-298` then collects **every**
`None` slot, including the one that just reported an error, and `:306` returns
`[errors, missing].concat()`. One failed chunk therefore appears twice in the
same `Vec<String>`: once as the real error text, once as its bare chunk id.
`run/mod.rs:413` sets `fetches_failed = fetch_errors.len()`, so one failure is
reported as two. The `Filled.errors` doc says “Fetches that failed”
(`fill.rs:25-32`), but the vector now also contains missing ids.

The index row is also weakened: `unfilled` sets
`decode_error = Some("not stored")` (`blobs/mod.rs:379-382`), so the bundle
records that the chunk is absent but not why the fetch failed.

**Concrete fix.** Preserve the index on error: return
`(usize, Result<(IndexedBlob, Vec<Set>), String>)` from `Store::one`, or
`Result<(usize, entry), (usize, String)>`. Record the error per slot, and build
`missing` only for slots with no result at all (panics), not for slots that
already have a real error. Put the actual fetch error into the unfilled row (a
`fetch_error` field, or `decode_error` with the real text) instead of
`"not stored"`. Add a test with one failing chunk: `fetches_failed == 1` and the
index row carries the fetch reason.

---

### F5 — Non-2xx bodies are drained without the response ceiling

**Severity: medium. Class: bug. Location:** `src/http/fetcher.rs:739-747`;
contradicted docs: `fetcher.rs:106-121`, `src/http/transport.rs:145-157`.

The non-success branch drains the body with a bare loop:

```rust
// fetcher.rs:739-747
let mut resp = resp;
let mut bytes_in: u64 = 0;
loop {
    match resp.chunk().await {
        Ok(Some(c)) => bytes_in += c.len() as u64,
        Ok(None) => break,
        Err(_) => break,
    }
}
```

There is no `Content-Length` precheck and no running-total check against
`max_response_bytes`. The `drain_capped` doc at `fetcher.rs:106-121` says it is
“The one place a body is read, for every destination — the buffered path, the
file path, `observe`, and the drain of an *error* body.” That is false at HEAD:
the error body does not go through `drain_capped`. `Transport::max_response_bytes`
at `transport.rs:145-157` likewise says the ceiling is checked in two places and
is “the only bound on the path between a socket and this tool’s buffers”; the
error-body path is outside it.

Memory is bounded because only the count is kept, but time and bandwidth are not:
a hostile or broken 5xx can stream until the per-request timeout, once per retry.
`bytes_in` then over-reports the cost of the failed request.

**Concrete fix.** Cap the error drain with the same declared/running checks, but
preserve the primary fact, which is the status. Return the `Status` error with
the partial `bytes_in`; if the body exceeds the cap, stop reading and record the
cap event (a warning, or a separate field) rather than replacing a 404/503 with
an unrelated `Shape`. Do **not** copy the reverted fix: at
`f9352f7:tools/capture/src/http/fetcher.rs:760-785`, a mid-read failure goes
through `drain_capped`, becomes `ApiError::Transport`, is retried, and records
`bytes_in: 0`; a 404 with a truncated body would be retried. Add tests for:
503 with a declared huge body is not fully read; 404 with a truncated body is
not retried; partial `bytes_in` is preserved.

---

### F6 — Tag validation does not reject Windows separators or path prefixes

**Severity: medium. Class: bug. Location:** `src/config/mod.rs:535`; sinks:
`:585-588`, `:600-607`, `src/run/mod.rs:239`; tests: `:880-886`.

`validate_tag_shape` rejects only empty, longer than 32, `/`, and space:

```rust
// config/mod.rs:535-539
if tag.contains('/') || tag.contains(' ') {
    return Err(UsageError::new(format!(
        "{flag} {tag:?} contains a separator or a space; a tag is a bare version string"
    )));
}
```

It accepts `\`, `:`, and therefore tags such as `..\..` (a `..` component once
backslashes are treated as separators). `pair_dir` interpolates raw tags:

```rust
// config/mod.rs:585-588
pub fn pair_dir(root: &Path, biz: Biz, old_tag: &str, new_tag: &str) -> PathBuf {
    root.join(biz.as_str())
        .join(format!("{old_tag}__{new_tag}"))
}
```

`artifact_dir` (`:600-607`) appends the capture id and `run/mod.rs:239` uses it.
On Windows, `\` is a separator; `C:` is a path prefix (`Path::join` replaces the
base when the joined path has a prefix but no root); and `..\..` is traversal.
`--files` already has the right checks at `config/mod.rs:457-490` — backslash,
colon, absolute, `..` component, empty component — but tags do not. The resolved
`main.tag`/`diff_tags[0]` values are not re-validated either, so a hostile API or
local proxy can supply the same string even when the flags are clean.

The tests at `:880-886` only exercise empty, `/`, a space, and length. A tag like
`C:` or `..\..` is accepted by the validator.

**Concrete fix.** Share the component checks from `validate_paths`: reject `\`,
`:`, any `..` component after splitting on both `/` and `\`, and control
characters in `validate_tag_shape`, so the same rule covers `--old-tag`,
`--new-tag`, and `--probe-tags`. Add tests for `C:`, `..\..`, `a\b`, and
`a:b`. The live-API question
(would the service echo such a tag?) is **not verified**; the local sink is
still worth hardening, especially when `--hyp-base`/`--sophon-base` point at a
test or proxy server.

---

### F7 — `--no-chunk-blobs` still enforces the chunk-byte ceiling

**Severity: low. Class: bug. Location:** `src/run/mod.rs:222-229`;
`src/blobs/plan.rs:115-117`; `src/blobs/fill.rs:48-57`.

`run::capture` always calls `blobs::plan::plan(..., plan.max_download_bytes)`
before it knows whether blobs will be fetched. `plan` calls
`check_byte_ceiling(projected, cap)` at `plan.rs:115-117`. Only later does
`fill_or_plan` see `no_chunk_blobs` and return the plan-only index without
fetching anything (`fill.rs:48-57`).

So a metadata-only run is refused when the **would-be** chunk store exceeds the
cap, even though no chunk bytes will cross the wire. The flag is documented as
“Capture metadata only” (`src/cli/mod.rs:187`), and the spec’s
`--max-download-bytes` is a ceiling on `Σ total_cost` over admitted captures
(`docs/00-spec.md:572`); the plan’s check is a second wire-byte ceiling
(`plan.rs:24-35`). With no chunk wire bytes, the second ceiling is vacuous.

**Concrete fix.** Pass `None` to `plan` when `plan.no_chunk_blobs` is true, or
add a `no_chunk_blobs` parameter and skip `check_byte_ceiling` while keeping the
A12 and total-cost checks. Add a test: `--no-chunk-blobs` with a
`--max-download-bytes` below the projected chunk bytes succeeds; the same run
without `--no-chunk-blobs` fails. If the current behavior is intentional, the
help and spec must say that the ceiling applies to the hypothetical store, not
the wire.

---

### F8 — Non-Unicode `--out`/`--log-file` panics in `main`

**Severity: low. Class: bug. Location:** `src/main.rs:58`; `:15`;
`src/cli/mod.rs:95`, `:140`.

`main` parses with `std::env::args_os()` at `:15` (safe), then records argv with
`std::env::args().collect()` at `:58`:

```rust
// main.rs:58
Cmd::Capture(a) => match capture_plan(a, std::env::args().collect()) {
```

`std::env::args()` panics on any argument that is not valid Unicode. The
realistic carriers are `PathBuf`-typed values: `--out` (`cli/mod.rs:140`) and
the global `--log-file` (`cli/mod.rs:95`). `String`-typed values are rejected by
clap’s value parser first; `argv[0]` itself is the other theoretical carrier if
the executable path is non-Unicode. The panic exits 101 and
prints no `SUMMARY`, violating the “every run prints exactly one” promise at
`main.rs:34-36`.

This is a real but narrow bug: it needs an unpaired surrogate or otherwise
non-Unicode path on Windows. It is **not** “any argument”, and the old review’s
“same as `girpr` and `girsync`” justification is false: the root `girpr` and
`girsync` binaries do not call `std::env::args()` (only test scripts and
`tools/sync/src/bin/sled2redb.rs` do).

**Concrete fix.** Use
`std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect()` for the
recorded `argv`, or change `CapturePlan::argv` to `Vec<OsString>` and
lossy-serialize at emit. Add a unit test of the lossy helper with an unpaired
surrogate. The exact clap behavior per argument type is **not verified** by a
run.

---

### F9 — Duplicate `--files` entries produce duplicate ids and rows

**Severity: low. Class: bug. Location:** `src/select/mod.rs:498-525`;
`src/emit/fixture.rs:248`, `:259`; `src/emit/index.rs:165`.

Phase 0 has no `seen` set. Each occurrence of the same path finds the same
candidate and is admitted with `capture_id(primary, 0)` (`select/mod.rs:516`),
so `--files a --files a` creates two admissions with the same id. Downstream:

- `emit/fixture.rs:248` pushes the id twice into `assembled_ids`;
- `:259` writes the same patchmap twice, the second overwriting the first;
- `emit/index.rs:165` reports `files_captured` as `admitted.len()`, i.e. 2,
  while only one patchmap exists.

`--audio` rejects duplicates explicitly (`config/mod.rs:435-443`); `--files`
does not.

**Concrete fix.** Reject duplicates in `validate_paths` (or deduplicate in
`capture_plan`, preserving first occurrence and recording the dedup count) with
a usage error naming the repeated path. Add a test that `--files a --files a`
produces one admission, one row, and one patchmap. If dedup is preferred, make
the summary say how many were dropped.

---

### F10 — CLI help contradicts the code and the spec

**Severity: low. Class: doc error. Location:** `src/cli/mod.rs:161`, `:187`;
`src/emit/selfcheck.rs:707-709`; `docs/00-spec.md:730-759`, `:766-767`.

Two clap doc comments are stale:

1. `--require-class`: “Fail with exit 4 unless these matrix classes are
   satisfied. Default: all.” (`cli/mod.rs:161`). The implementation returns
   `Ok(())` for an empty list (`src/select/require.rs:84-86`), and the spec says
   “defaults to **nothing**” (`docs/00-spec.md:730-739`).
2. `--no-chunk-blobs`: “Breaks assertions A7-A10 by design.”
   (`cli/mod.rs:187`). The actual skip set is
   `["A6","A7","A8","A9","A10","A11","A12","A19","A21","A22","A23"]`
   (`selfcheck.rs:707-709`), and the spec says “A6–A12, A19 and A21–A23”
   (`docs/00-spec.md:766-767`).

**Concrete fix.** Change the first to “Default: none (nothing is required unless
named)” and the second to “Breaks assertions A6–A12, A19, A21–A23 by design.”
Add a help-text assertion if the CLI contract tests have a place for it; none of
the current tests pin these strings. The closeout test that should pin the
second is itself incomplete (F12).

---

### F11 — Failed fetches can orphan `.tmp` files; the comment overstates cleanup

**Severity: low. Class: intentional-but-uncovered. Location:**
`src/blobs/mod.rs:432-436`, `:484-491`.

`stream_from` writes to `.<name>.<slot>.tmp` and renames on success. On a fetch
error it returns without removing the temp file, with this justification:

```rust
// blobs/mod.rs:432-435
// The tmp file is left behind rather than removed: it is
// dot-prefixed inside `chunks/blobs/`, nothing reads it, and the
// next run truncates it.
```

`paths_for` (`:484-491`) includes the slot, which is the index in `wants`. The
next run truncates the file only if the same chunk id lands in the same slot.
Selection order, `--files`, budget changes, or a different chunk set can move it
to another slot, leaving `.<name>.<old_slot>.tmp` behind forever. Nothing sweeps
`chunks/blobs/` for orphans. The leak is bounded per failed want per run, but it
is not self-healing in general.

**Concrete fix.** Either remove the temp file on the error path (best-effort
`let _ = std::fs::remove_file(&tmp_path);`, after logging the original error), or
sweep `.*.tmp` at store creation with an age guard. Correct the comment to state
the exact invariant (“same id and slot”). Add a test: a failed fetch leaves no
`.tmp` if removing; or a later run with a different slot leaves the old file if
documenting. The current comment is **source-verified** as incomplete; the
real-world accumulation rate is **not verified**.

---

### F12 — The closeout skip-set test checks only 8 of 11 ids

**Severity: low. Class: test gap. Location:** `src/emit/closeout.rs:637`;
`src/emit/selfcheck.rs:707-709`.

The test loops over eight ids and asserts containment:

```rust
// closeout.rs:637-639
for id in ["A6", "A7", "A8", "A9", "A10", "A11", "A12", "A19"] {
    assert!(skipped.contains(&id), "{id} should be declared skipped");
}
```

`SKIPPED_WITHOUT_CHUNK_BLOBS` has eleven ids, adding `A21`, `A22`, `A23`
(`selfcheck.rs:707-709`). The test does not assert equality or length, so a
regression that drops A21–A23 from the skip set is not directly detected; the
separate `failing.is_empty()` assertion may or may not catch it depending on
which rows the close-out generates. This is the test the old review described as
asserting “exactly those eight ids”; it does not.

**Concrete fix.** Assert set equality against `SKIPPED_WITHOUT_CHUNK_BLOBS`
(sort both if order is not meaningful), and add a negative case that an extra id
fails. This also pins the F10 doc fix.

---

## Verified-correct behavior

These were checked at HEAD and hold as described. They are **source-verified**,
not test-run.

- **`too_big` is a `Shape`, not a retried `Transport`.** `drain_capped` returns
  `too_big` on the declared-length and running-total checks
  (`src/http/fetcher.rs:129-152`), and `too_big` builds
  `ApiError::shape(...)` (`:168-182`). The retry match treats `Shape` as
  non-retryable (`:603-609`). This is correct for 2xx and `observe`; F5 is the
  error-body exception.
- **`--per-class 0` is floored, not rejected.** `capture_plan` applies
  `.max(1)` at `src/config/mod.rs:336`, and `select::decide` re-applies it at
  `src/select/decide.rs:54` as defence for direct callers. Tests exist at
  `config/mod.rs:686-696` and `decide.rs:221`.
- **`--require-class` defaults to nothing.** `require::check` returns `Ok(())`
  for an empty list (`src/select/require.rs:84-86`), matching
  `docs/00-spec.md:730-739`. F10 is only the help-text drift.
- **A9 is byte-compared internally, digest-recorded in the artifact, and
  intentionally uncovered.** `assemble/mod.rs:856-881` compares
  `patch_bytes == new.bytes` for `equals_new`; the artifact’s A9 row at
  `:395-401` compares `p.md5` and `n.md5`. The comment at `:872-875` states that
  replacing the byte comparison with `true` passes the suite (mutation-tested
  twice). This is **intentional-but-uncovered**, not “well-covered”; the
  observable digest row is covered, the internal byte comparison is not.
- **The crate’s dependency independence holds.** `Cargo.toml:24-58` has only
  `girpr-log` and `girpr-tempdir` as path dependencies; there is no `girpr`
  dependency. `std::process::Command` appears in tests (e.g.
  `tests/case_coverage.rs:62`, `:102`, `:145`) but not in `src/`, and none of
  those invocations is `girpr`. The old review’s “no `Command` anywhere in the
  tree” is overstated, but the independence conclusion is correct.
- **The manifest-id prefix is recorded, never asserted.** Every construction
  site sets `id_prefix_verified: false` (e.g. `src/manifest/mod.rs:323`, `:435`),
  and `Verified::all_green` ignores it (`:94-98`). This matches spec §3.7.
- **A missing `--files` path is recorded, not fatal.** `forced_skipped` is
  populated at `src/select/mod.rs:499-505`; the integration test at
  `tests/matrix.rs:695-707` asserts requested 1 / skipped 1. F2 and F3 are the
  two other “requested but invisible” branches.
- **Phase 2’s want admission is smallest-first and charges only admitted
  costs.** `budget::admit_in_order` checks before charging and leaves rejected
  candidates uncharged (`src/select/budget.rs:114-128`); `select` sorts by cost
  before reading fates (`src/select/mod.rs:648-652`). This is correct for wants;
  F1/F2 are the forced-phase exceptions.
- **`--files` path validation is strict.** `validate_paths` rejects empty,
  backslash, absolute, colon, `..` components, and empty components
  (`src/config/mod.rs:457-490`). F6 is the missing counterpart for tags.
- **`--no-chunk-blobs` stores nothing.** `fill_or_plan` returns the plan-only
  index without calling `store.fill` (`src/blobs/fill.rs:48-57`). F7 is only the
  ceiling check that runs before this branch.
- **A12 is checked on the plan, before any blob is fetched.**
  `blobs::plan` iterates the admitted pairs and rejects a retention plan whose
  `patch_fetch` is not a subset of `materialize_new`
  (`src/blobs/plan.rs:86-103`).

## Corrections to the reverted review series

The reverted review and its fixes are in `f9352f7`, not HEAD. These are the
claims that need correction; the consequence column says what to do instead of
resurrecting the old text.

| # | Old claim (in `f9352f7` review) | Actual truth at HEAD `64dc69d` | Consequence |
|---|--------------------------------|--------------------------------|-------------|
| C1 | F1: non-Unicode argv crashes; **medium**; “any argument”; “let it panic, same as `girpr` and `girsync`.” | Only `Cmd::Capture` calls `std::env::args()` (`main.rs:58`); the realistic carriers are `PathBuf`-typed values (`--out`, `--log-file`), plus `argv[0]` in principle. The root `girpr`/`girsync` binaries do not call `std::env::args()`; only test scripts and `tools/sync/src/bin/sled2redb.rs` do. A panic exits 101, not “abort.” | Downgrade to **low** (F8); the consistency justification is false; use `args_os` + lossy rather than “let it panic.” |
| C2 | F2: change `break` to `continue`, record the refused path as `over_budget`; test `a=500, b=100, cap=200`. | `f9352f7` kept `budget.admits(cost)` on the **unspent** caller budget, so two forced paths each ≤ cap but summing > cap are both admitted. Its test only covers one path that is individually over the cap. | The fix does not close over-admission; HEAD still has F1 and F2. Add a cumulative test, not just the single-over-cap test. |
| C3 | F6: drain the error body through `drain_capped`; split `max_response_bytes` / `max_success_response_bytes`; severity low. | At `f9352f7:tools/capture/src/http/fetcher.rs:760-785`, a mid-read failure becomes `ApiError::Transport` (retried) and records `bytes_in: 0`; a 404 with a truncated body is retried, and a too-big error body becomes `Shape` instead of `Status`. | Do not resurrect that patch. F5’s fix must preserve the status and the partial byte count, and must not turn a non-retryable 4xx into a retry. |
| C4 | F4: “the closeout test at `:637` asserts exactly those eight ids are declared skipped.” | The test loops over 8 ids with `contains`; it does not assert length or equality and omits A21–A23. The constant has 11 (`selfcheck.rs:707-709`). | The test does not pin the full skip set; F12 adds the equality assertion. The old sentence is misleading. |
| C5 | F5: “clarify the docs — leaving the `.tmp` behind is intentional; the next run truncates it.” | Docs cannot fix the leak. `paths_for` includes the slot (`blobs/mod.rs:484-491`); “next run truncates it” is true only for the same id **and** slot. | The recommendation is a no-op for the leak; either remove the temp on failure or state the exact invariant and add a test (F11). |
| C6 | F7: A9 is intentional and “well-covered”; a disagreement always turns the row red. | The observable digest row is covered, but the internal byte comparison is not: the code comment at `assemble/mod.rs:872-875` says replacing it with `true` passes the entire suite (mutation-tested twice). | Label A9 **intentional-but-uncovered**, not well-covered; do not cite it as a fully pinned behavior. |
| C7 | F10: “no `Command` anywhere in the tree.” | `std::process::Command` is used in tests (e.g. `tests/case_coverage.rs:62`, `:102`, `:145`); there is none in `src/`, and none invokes `girpr`. | The independence conclusion still holds, but the evidence sentence is false as written. Say “no `Command` in `src/` and no `girpr` invocation.” |
| C8 | “`cargo build -p gircap` — clean; `cargo test -p gircap` — all tests pass (≈622).” | **Not verified** at HEAD. HEAD is a revert of the reviewed series; no build or test was run for this review. | Do not cite the old pass count as evidence for HEAD. This review is source-level only. |
| C9 | F3/F4: the `--require-class` and `--no-chunk-blobs` help-text fixes were applied. | HEAD is pre-fix: `cli/mod.rs:161` still says “Default: all” and `:187` still says “A7-A10.” | Re-report as F10; do not assume the help text is fixed at HEAD. |

## Recommended landing order

All nine steps landed on `fix-gaps/gircap`; the Pass log records the commits.

1. **F1 + F2 + F9** — one selection/budget change. F1 is the high-severity
   ceiling violation; F2 and F9 are the same forced-loop code. Add the
   cumulative-forced test, the break/continue test, and the duplicate-`--files`
   test in the same change.
2. **F6** — tag/path validation. Small and isolated; land before any release
   where `--old-tag`/`--new-tag` can come from an untrusted wrapper or a local
   mock.
3. **F3** — record the empty-class forced path. Small, but it needs a decision
   on the summary field (`forced_skipped` vs a new `forced_unscored`).
4. **F5** — cap the error body without the `f9352f7` regression. Add the
   truncated-404 and huge-503 tests first.
5. **F4** — preserve the task index and the real fetch error. This touches
   `Store::one` and `fill`, so keep it separate from the selection change.
6. **F10 + F12** — fix the two help strings and make the closeout test assert the
   full 11-id set. Low risk; can land with the docs.
7. **F7** — decide the `--no-chunk-blobs` semantics, then either skip the
   ceiling or document it. Do not change it silently.
8. **F8** — replace `args()` with `args_os` + lossy recording. Low risk.
9. **F11** — choose cleanup or a precise comment, and add the corresponding
   test. Lowest priority because the leak is bounded.

---

## Pass log

> Detailed evidence, failure scenarios and per-pass verification are in the git
> history: `git log --oneline main..HEAD`, `git show <commit>`. The commit hashes
> in the tables are the index. No item in any pass was rejected as invalid or out
> of scope.

### First pass — F1–F12 (all fixed)

| ID | Finding | Status | Commit |
|---|---|---|---|
| F1 | Forced `--files` admissions bypass the cumulative budget. | fixed | `39304be`, `abe7bdc`, `998649c` |
| F2 | A budget-refused forced path `break`s and silently drops later paths. | fixed | `39304be` |
| F3 | A forced path that answers no matrix class is silently dropped. | fixed | `83eac6d`, `d8514c7` |
| F4 | A failed chunk fetch loses its index, is double-counted and hides its cause. | fixed | `f31850b`, `583aa79` |
| F5 | Non-2xx bodies are drained without the response ceiling. | fixed | `71a1321`, `0259220` |
| F6 | Tag validation accepts Windows separators and path prefixes. | fixed | `efc5322`, `de85df2` |
| F7 | `--no-chunk-blobs` still enforces the chunk-byte ceiling. | fixed | `801d91d` |
| F8 | Non-Unicode `--out`/`--log-file` panics in `main`. | fixed | `df366ee` |
| F9 | Duplicate `--files` entries produce duplicate ids and rows. | fixed | `39304be` |
| F10 | CLI help contradicts the code and the spec. | fixed | `4dd1c05` |
| F11 | A failed fetch can orphan `.tmp` files; the comment overstates cleanup. | fixed | `882a53b`, `7c64557` |
| F12 | The closeout skip-set test checks only 8 of 11 ids. | fixed | `4dd1c05`, `d538d4d` |

### Second pass — residuals against F1–F12

Re-derived at `b726054`; all items accepted, and F1, F2, F7, F8, F9 and F10 needed no change.

| ID | Finding | Status | Commit |
|---|---|---|---|
| F6 residual | API-resolved tags reach `pair_dir` unvalidated (blocking). | fixed | `de85df2` |
| F6 reserved | Windows-reserved `* ? " < > \|` accepted in tags. | fixed | `de85df2` |
| F6 dots | A leading or trailing `.` collides on Windows. | fixed | `de85df2` |
| F4 order | `fill.rs` says errors are in `wants` order. | fixed (doc) | `583aa79` |
| F4 panic | A panicking chunk's `JoinError` payload is discarded. | fixed | `583aa79` |
| F5 log | A mid-read error-body failure is silent. | fixed | `0259220` |
| F1 budget | `Budget::with_spent` is dead and unclamped. | fixed | `abe7bdc` |
| F1 matrix | `matrix.json` reports `bytes_admitted`, not the charged ledger. | fixed | `998649c` |
| F3 test | Missing end-to-end test for a nonzero `forced_files_unscored`. | fixed (test) | `d8514c7` |
| F12 test | The expectation is derived from the constant under test. | fixed (test) | `d538d4d` |
| F11 doc | The doc overstates the cleanup guarantee. | fixed (doc) | `7c64557` |

### Third pass — G1–G3

Re-derived at `577c399`; all three confirmed valid.

| ID | Finding | Status | Commit |
|---|---|---|---|
| G1 | Document-supplied paths reach filesystem joins unvalidated (blocking). | fixed | `88d5413` |
| G2 | `tmp/` staging and universal-atomicity claims. | fixed | `b1b035b` |
| G3 | The "three causes" of a null `stored_path` omit `--no-chunk-blobs`. | fixed | `b1b035b` |

Two test-harness defects were fixed alongside: the `hyp` fixture lacked the now-required `audio_pkg_scan_dir` (`6003958`), and the mock's armed `503` sent `keep-alive` then closed (`66bc6f0`).

### Fourth pass — H1–H4

Re-derived at `1cbf2f1`; all four confirmed valid; H1 was reproduced end to end before the fix.

| ID | Finding | Status | Commit |
|---|---|---|---|
| H1 | `verify`/`materialize` still join document paths unvalidated (**high**). | fixed | `0a21ebe` |
| H2 | `--audio` becomes `matching_field` and reaches manifest filenames raw. | fixed | `0b55a40` |
| H3 | The path rule accepts names Windows folds together. | fixed | `c9caa02`, `b6aeb6d` |
| H4 | The `--no-chunk-blobs` schema row contradicts `unmeasured`. | fixed (doc) | `c9bf161` |

### Fifth pass — I1–I5

Re-derived at `9b60455`; all five confirmed valid; I1 and I2 were reproduced at the real seam before the fix.

| ID | Finding | Status | Commit |
|---|---|---|---|
| I1 | Two distinct forced paths of one primary class share a `capture_id`. | fixed | `afc08bd` |
| I2 | A failed `2xx` body is recorded with `bytes_in: 0`. | fixed | `f631c5c` |
| I3 | Duplicate patchmap ids are silently collapsed. | fixed | `34de040` |
| I4 | `list` silently skips unreadable directories and failed entries. | fixed | `5d24494` |
| I5 | `dump::write` validates the field but not the tag. | fixed | `f20ee4c` |

### Sixth pass — J1–J3

Re-derived at `ecc81c5`; all three confirmed valid; J1 was reproduced with a share-locked reference manifest (exit `0` before, usage error after).

| ID | Finding | Status | Commit |
|---|---|---|---|
| J1 | `list` treats a `MANIFEST.json` it cannot read as "not a bundle", so a real bundle vanishes with exit `0`. | fixed | `0706984`, `43d5977`, `3f042a1` |
| J2 | The `list` doc promises "deepest first"; the code sorts by relative-path string. | fixed (doc) | `28c3c18`, `d74e40b` |
| J3 | The `list` command doc calls a refused bundle "a skip" and omits I4's fail-fast usage error. | fixed (doc) | `73d0cb5` |

### Seventh pass — K1–K2

Re-derived at `8d25b06`; both confirmed valid and both reproduced outside the repository before the fix. K1's `Layout::of([(i64::MAX, 1)], 1)` aborted at `ranges.rs:48`, a manifest with `compressed_size: -1` produced `download_bytes: -1`, and three maximum `projected_download` wants aborted. K2's `Store::fill` left an unframed 58-byte XML body on disk while the row said `bytes: 0`, `stored_path: null` and the error list said "not a zstd frame".

| ID | Finding | Status | Commit |
|---|---|---|---|
| K1 | Arithmetic on untrusted offsets, lengths and costs is unchecked across the range algebra, plan totals, index decode, manifest decode, A20 and the diff totals. | fixed | `06bd550`, `e371ee9`, `37b7b2a`, `fc71387`, `ab560d8` |
| K2 | An undecodable fetched body is published on disk but recorded as an unfilled fetch, losing the evidence the spec says to keep. | fixed | `f7a8da2`, `41fdef9` |

K1 follows `decode_counts`: *measurements* saturate, while address-like sums use `checked_add` and return a typed error naming the chunk and values. `Range::at`/`len`/`to` are total; `Layout::of` records an overflowing end or total as a violation; `materialize::rebuild` checks offsets, lengths and `declared_size`; `ChunkIndex::validate_arithmetic` runs in `Bundle::open`; `SophonChunkFile::validate_arithmetic` runs at manifest decode and again in `assemble`; `totals_for` rejects a negative `compressed_size` and checks the sum; A20 rejects a negative cost and folds with `checked_add`; `projected_download` and `diff.rs` saturate. Every fixed site has a regression test, including a crafted index refused at `Bundle::open`.

K2 takes the spec-supported row: an unframed body is a successful fetch with `stored_path`, `bytes`, both compressed digests, `decode_error`, and `verified` carrying the two decode-independent answers and two `false` decode-dependent keys. Because the fetch succeeded, the store stops rather than asking a second source and renaming a good body over the evidence; a real I/O error after the rename removes the published blob so disk and index agree. Both docs now state this. A loopback `Store::fill` test drives the real store path.

### Eighth pass — L1–L2

Re-derived at `b9c1682`; both confirmed valid and both reproduced before the fix. L1 failed as `--old-tag "CON" must be refused`; L2's `select` fixture panicked in debug at `accum.rs:206:1` (`attempt to add with overflow`) and the release budget test reported `18446744073709551612` against the ledger's `u64::MAX`.

| ID | Finding | Status | Commit |
|---|---|---|---|
| L1 | A tag may name a Windows DOS device, so `manifests/<tag>` fails late or names a directory Windows cannot open. | fixed | `63ba129`, `f049c12` |
| L2 | `bytes_admitted` and `total_admitted` use plain `.sum()` while the budget saturates. | fixed | `06c650b` |

L1 makes `paths::is_windows_device_name` `pub(crate)` and calls it from `tag_shape_problem`, so the flag path, the API-resolved tags and the manifest writer share one rule; the extension is ignored, and `7.1.0`/`CONSOLE`/`COM0`/`LPT10` stay legal. L2 folds both totals with `saturating_add`, so the summary and `diff/matrix.json` report the same saturated number; a `select` fixture with two `u64::MAX - 1` costs pins it in debug and release.

### Decisions that still constrain current behaviour

- **F1/F2/F9.** Forced paths share one cumulative budget; a refusal is recorded and the loop continues; duplicate `--files` paths are a usage error.
- **F1 matrix.** `matrix.json` reports the charged ledger, with a debug assertion tying it to `bytes_admitted`.
- **F5.** An error body is drained through a counting cap, but the status is always preserved, partial bytes are recorded, and a 404 is never retried.
- **F6/G1.** One shared validator covers every document-supplied path: no `\`, absolute path, `:`, `..`, empty component or control character on any host; tags also refuse Windows-reserved characters and leading/trailing dots. A chunk id must match `<16 hex>_<32 hex>` (case-insensitive), not merely be one component.
- **L1.** A tag also refuses DOS device names through the same `paths::is_windows_device_name` helper the path rule uses, extension ignored (`CON`, `NUL.txt`, `COM1`, `CONIN$`), on the flag path, the API-resolved tags and the manifest writer.
- **L2.** `Selection::bytes_admitted` and `budget::total_admitted` saturate with the charged ledger, so a crafted maximum cost cannot panic in debug or wrap in release.
- **F3.** A classless forced path is reported in `forced_unscored`/`forced_files_unscored`, not overloaded onto `forced_skipped`.
- **F4.** The fetch reason goes in `decode_error`; a dedicated `fetch_error` remains open.
- **F7.** A metadata-only run does not enforce a chunk wire-byte ceiling it pays nothing towards.
- **F8.** Recorded argv is lossy, not byte-exact; replay would need a raw-bytes companion.
- **F11.** A handled fetch error removes the temp file best-effort after the handle is dropped; a panic unwinds past cleanup.
- **F12.** The closeout test pins an independent eleven-id literal and asserts it equals the constant.
- **H1.** `Bundle::open` validates document paths once; the older sink checks remain as defence in depth.
- **H2/H3.** `--audio` is one file-name component; names Windows folds together (trailing dot/space) and DOS device names are refused on every host.
- **I1.** Forced admissions are numbered per primary class; phase 1's `taken` still starts at zero so a budget-refused class stays over-budget, not absent; the matrix resolves a class to its first admission.
- **I3.** A patchmap is keyed by its declared `capture_id`; its filename is not authoritative.
- **I4/J1.** `list` fails fast on an unreadable directory, entry or manifest; a non-bundle is stepped over and a refused bundle is named and counted.
- **I5.** The manifest writer repeats the CLI's tag validation by design.
- **J2.** `list` orders by relative path, not deepest-first; deepest-first would be a new contract.
- **K1.** Measurement totals saturate; address-like sums and plan costs use `checked_add` with an error naming the offending chunk and values. No negative `offset`, `compressed_size` or `uncompressed_size` is accepted at a decode boundary, and no `uncompressed_size` above `i64::MAX` survives index validation.
- **K2.** An unframed fetched body is a successful fetch and an `undecodable` row, not a failed fetch; it is not retried from another source, because replacing it would destroy the evidence. A post-rename I/O error removes the published blob.

### Verification (latest: eighth pass, `f049c12`)

| Command | Result |
|---|---|
| `cargo build -p gircap` | finished, no warnings |
| `cargo test -p gircap` | **693 passed, 0 failed** (549 lib + 144 integration) |
| `cargo clippy -p gircap --all-targets` | finished, no diagnostics |
| `cargo fmt -p gircap --check` | clean |

Integration total 3 + 8 + 32 + 2 + 30 + 10 + 29 + 15 + 15; no test is `#[cfg(unix)]`, so none was skipped and the `#[cfg(windows)]` tests ran. The eighth-pass reproductions are now in-tree unit/fixture tests; `cargo test -p gircap --release --lib maximum_costs` also passes (2 passed).

## Open for maintainers

- **F4 dedicated field.** A `fetch_error` field would describe a failed fetch more precisely but needs an artifact-schema change; a panicking chunk still gets the stable generic row text.
- **F7 ceiling intent.** If `--max-download-bytes` was meant as an input-independent policy check, the metadata-only skip should be reverted and the help changed instead.
- **F8 replay.** The recorded argv is lossy; replaying a bundle would need a raw-bytes companion.
- **F11 older orphans.** Cleanup covers handled errors only; a panic unwinds past it, orphans from older binaries are not swept, and the historical accumulation rate is unmeasured.
- **F6/G1 live inputs.** Whether a live API echoes a hostile tag, serves a hostile document or audio path, or uses an unusual but legitimate chunk-id spelling is unverified; the local sinks and rejection paths are tested.
- **L1 end to end / cross-host.** No capture was run with a device-name tag — it is now refused at argument resolution, before any request — and a Linux-produced `manifests/CON/` bundle was not replayed through `verify` on Windows; A16's missing-file consequence is reasoned, not executed.
- **L2 through the command.** The near-`u64::MAX` cost is driven through `select` with a `PathScore` fixture, not through a hand-edited manifest read from disk; no live manifest declaring extreme sizes was sampled.
- **G2 crash tear.** Whether a metadata document can be left torn in a way `checksums.json` misses was not stress-tested.
- **H3 live manifests.** Whether a live manifest contains names differing only by a trailing dot or space was not sampled.
- **H4 on-disk bundle.** No real `--no-chunk-blobs` bundle was inspected; the doc matches the constructor and the tests.
- **I4/J1 ACLs.** No real Windows ACL-denied directory or `MANIFEST.json` was set up; the read arms are injected, the J1 share lock was reproduced end to end, and the stat/read race was reasoned, not timed.
- **I5 CLI precondition.** A hostile tag reaches the writer only through a hand-built manifest; the CLI's earlier validation is not re-driven, though the writer's refusal is unit-tested.
- **F5 production bodies.** Real non-2xx body sizes are unknown, so the practical effect of the cap is unverified.
- **F1 real corpus.** Whether `blobs::plan`'s projected check also fires on a real corpus is unverified; the selection-level test is the guarantee.
- **F8 clap typing.** Which non-`PathBuf` argument types clap accepts non-Unicode values for is unverified; only `--out` is exercised end to end.
- **K1 A20 through the command.** A20's negative and overflowing cases are unit-tested against a crafted `PatchMap`, but no full bundle with a hand-edited patchmap was run through `gircap verify`.
- **K2 removal arm.** The published blob is removed on a post-rename measurement I/O error, but no test forces that I/O failure; the undecodable path is the tested post-rename path and deliberately keeps the blob.
- **K1/K2 live inputs.** Whether a live CDN serves a non-frame chunk, or a live manifest declares extreme or negative numbers, remains unmeasured.
