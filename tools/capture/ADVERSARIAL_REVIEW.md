# ADVERSARIAL REVIEW — `gircap` (`tools/capture`)

Reviewed against the source under `src/`, the tests under `tests/`, `Cargo.toml`,
the shared leaf crates `girpr-log` / `girpr-tempdir`, and `docs/` (notably
`docs/00-spec.md` §2 "no shared contracts" and §3.7 "16-hex manifest-id prefix
is not a hash").

## Result of the test suite

`cargo build -p gircap` — clean.
`cargo test -p gircap` — **all tests pass** (unit, integration, doc-adjacent;
nextest-equivalent count ≈ 622; 0 ignored, 0 failed).

The crate is genuinely well engineered: nearly every behavioral claim in the
docs is pinned by a test, and several tests exist expressly to catch regressions
the authors already found (mutation-tested A9, the `restart` digest test, the
`is_moved`/diff agreement test, the `skip()`/`is_acceptable()` agreement test,
etc.). Most of this review is therefore **doc drift and edge cases**, not
broken code.

No code or tests were modified.

---

## Findings

### F1 — Crash bug: non-Unicode argv aborts the process (no clean exit code)

**Severity: medium — actual crash, not a wrong answer.**

**Behavior / gap.** `main.rs` parses the command line with
`std::env::args_os()` (line 15), which is safe on Windows, but then resolves the
capture plan with `std::env::args().collect()` (line 58). `std::env::args()`
panics when any argument is not valid Unicode. On Windows arguments are arbitrary
`WCHAR` sequences, so a path or value containing non-UTF-16-valid data causes an
abort instead of a clean usage error (exit 1) — and no `SUMMARY` is printed, so
a harness reading stdout gets nothing.

**Evidence.**
- `src/main.rs:15`: `parse_from(std::env::args_os())`
- `src/main.rs:58`: `capture_plan(a, std::env::args().collect())`

`parse_from` already succeeded with an `OsString`; the second collection is
redundant and is the only path that can panic.

**Recommended action:** fix — collect `argv` from `std::env::args_os()` and
convert losslessly (e.g. `to_string_lossy()`), or wrap the collection so a
non-Unicode argument becomes a usage error rather than a panic.

---

### F2 — Silent drop of later `--files` entries under a budget

**Severity: medium — user-requested files can disappear without any record.**

**Behavior / gap.** In `select::select`, forced captures (`--files`) are
processed in caller order (select/mod.rs:498–525). When one forced capture is
refused by the budget, the loop executes `break` and stops considering **all**
remaining forced paths — even cheaper ones that would fit. Nothing is recorded
for them: they are neither admitted, nor `forced_skipped`, nor `over_budget`.
So `--files a --files b` with a ceiling that refuses `a` silently drops `b`.
This is inconsistent with the admission phase, which admits smallest-first
across the whole wanted set, and it is a hidden assumption that `--files` order
plus "first unfundable stops everything" is acceptable.

**Evidence.**
- `src/select/mod.rs:508–513`: `if !budget.admits(cost) { break; }` inside the
  `for want in forced` loop.
- `src/select/mod.rs:526`: `let initial = spent.spent();` — the forced loop's
  `spent` is carried into `select`, so a refused forced path genuinely consumes
  nothing, yet later forced paths are never reached.

**Recommended action:** pin with a test (`--files a --files b` under a tight
ceiling where `b` fits but `a` does not), and decide whether later forced paths
should still be evaluated (and recorded as `forced_skipped`/`over_budget`) rather
than abandoned by `break`.

---

### F3 — Doc bug: `--require-class` help text says "Default: all"

**Severity: low — documentation contradicts code and spec.**

**Behavior / gap.** The clap doc comment on `--require-class` reads
`"Fail with exit 4 unless these matrix classes are satisfied. Default: all."`
but the implementation does not default the set at all: an empty list means
"nothing is required", and `--require-class` is opt-in. A run whose matrix
leaves a class `unavailable` still exits 0.

**Evidence.**
- `src/cli/mod.rs:161–163`: doc comment `"Default: all."`
- `src/config/mod.rs:190–197`: "Defaulting does *not* happen here ... An empty
  list therefore means 'nothing is required'"; `parse_classes` returns an empty
  `Vec` when no flag is given.
- `src/select/require.rs:84–86`: `if required.is_empty() { return Ok(()); }`.
- `docs/00-spec.md §6` and §7: "`--require-class` defaults to **nothing**";
  exit 4 only for a class the caller named.

**Recommended action:** fix the clap doc comment to `"Default: none (nothing is
required unless named)."` No code change.

---

### F4 — Doc bug: `--no-chunk-blobs` understates which assertions it breaks

**Severity: low — documentation contradicts code.**

**Behavior / gap.** The clap doc comment on `--no-chunk-blobs` reads
`"Capture metadata only. Breaks assertions A7-A10 by design."` but the actual
skipped set is A6, A7, A8, A9, A10, A11, A12, A19, A21, A22, A23 — eleven ids,
not four.

**Evidence.**
- `src/cli/mod.rs:187–189`: `"Breaks assertions A7-A10 by design."`
- `src/emit/selfcheck.rs:707–709`: `SKIPPED_WITHOUT_CHUNK_BLOBS` lists exactly
  `["A6", "A7", "A8", "A9", "A10", "A11", "A12", "A19", "A21", "A22", "A23"]`.
- `src/emit/closeout.rs:180–184` and `:637–638` apply the same list; the
  closeout test at `:637` asserts exactly those eight ids are declared skipped.

**Recommended action:** fix the clap doc comment to name the full skipped set
(`A6–A12, A19, A21–A23`). No code change.

---

### F5 — Temp files left behind after a failed chunk fetch

**Severity: low — gradual temp-file accumulation in `chunks/blobs/`.**

**Behavior / gap.** `Store::stream_from` writes each blob to a dot-prefixed temp
file (`.<id>.<slot>.tmp`) and renames it into place. When the fetch fails, the
temp file is deliberately **not** removed (blobs/mod.rs:432–436): "the next run
truncates it". So every failed chunk fetch leaves a hidden `.zst.N.tmp` file in
`chunks/blobs/`, accumulating across failed runs until a successful run
truncates/overwrites one with the same id+slot. Nothing cleans orphaned temp
files whose chunk id is never retried.

**Evidence.**
- `src/blobs/mod.rs:432–436`: "The tmp file is left behind rather than removed...
  the next run truncates it."
- `src/blobs/mod.rs:484–492` (`paths_for`): temp names live beside the blob,
  slot-suffixed; orphans are never scanned for.

**Recommended action:** clarify the docs (it is intentional) or add startup
cleanup of stale `.*.tmp` files under `chunks/blobs/`; recommend a pinning test
that a failed fetch leaves no orphan when the same id is never retried, if the
behavior is meant to stay.

---

### F6 — `max_response_bytes` ceiling not applied to non-2xx response bodies

**Severity: low — asymmetry in body-size protection.**

**Behavior / gap.** The response-body ceiling (`--max-response-bytes`) is
enforced on successful 2xx bodies (via `drain_capped`, both pre-check on
`Content-Length` and running total) and on probe bodies (`observe`), but in
`Fetcher::attempt` the body of a non-2xx response is drained and **only
counted**, never capped against `max_response_bytes` (fetcher.rs:741–747).
A hostile origin returning a 5xx with a huge body forces the client to read it
all (memory is bounded because only the count is kept, but time is unbounded,
and `bytes_in` then over-reports the "cost" of a failed request). The ceiling's
documented purpose is to bound "any single response".

**Evidence.**
- `src/http/fetcher.rs:721–767`: non-success branch drains the body in a loop
  that only accumulates `bytes_in`; no `max_response_bytes` check.
- Contrast `drain_capped` (fetcher.rs:122–153), which does cap, used for 2xx and
  for `observe`.

**Recommended action:** either apply `drain_capped`-style capping to the
error-body drain, or document the asymmetry explicitly (and note that
`bytes_in` for a failed request reflects the full un-capped body). If left as
is, recommend a pinning test.

---

### F7 — Intentional, well-covered: A9 is byte-compared internally but digest-recorded in the artifact

**Severity: n/a — intentional; no action.**

**Behavior.** `assemble::assemble` compares `patch_bytes == new.bytes` (a byte
comparison) for `p.equals_new` (assemble/mod.rs:880), but the selfcheck A9 row
written to `verify/selfcheck.json` compares `p.md5` against `n.md5` (a digest
comparison) (assemble/mod.rs:395–401). The comments at assemble/mod.rs:860–877
explicitly acknowledge this gap and state it is not closed because "every
reachable disagreement also changes the digests".

**Evidence.**
- `src/assemble/mod.rs:860–877` (comment on the byte-equality vs digest gap).
- Tests `a9_is_red_when_the_patch_flow_and_the_direct_assembly_disagree` and
  `the_patch_flow_byte_equals_the_direct_new_assembly` cover the observable
  side (the row goes red on disagreement).

**Recommended action:** leave as intentional; it is pinned at the observable
level (a disagreement always turns the A9 row red). If the byte-vs-digest gap
ever matters (only via an MD5 collision), it would need a spec change.

---

### F8 — Intentional, well-covered: `--per-class 0` is silently floored to 1

**Severity: n/a — intentional; no action.**

**Behavior.** `--per-class 0` is not a usage error; `capture_plan` floors it to
1 (config/mod.rs:336) and `select::decide` re-floors (decide.rs:54). Asking for
zero captures of a class means "capture nothing but still charge admission",
not "an empty bundle".

**Evidence.**
- `src/config/mod.rs:328–336`, `src/select/decide.rs:54`.
- Tests in `config/mod.rs` (`a_zero_per_class_is_floored_once_in_the_plan`) and
  `decide.rs` (`per_class_is_floored_at_one`).

**Recommended action:** leave as intentional; already pinned by tests.

---

### F9 — Intentional, well-covered: `--require-class` defaults to nothing; over-budget is recorded, not fatal

**Severity: n/a — intentional; no action.**

**Behavior.** An empty `--require-class` requires nothing (require/mod.rs:84–86).
A class the budget refuses is `over_budget`, exits 0, and is accepted by A17;
only a class named by `--require-class` that is unmet is exit 4 (require/mod.rs).

**Evidence.**
- `docs/00-spec.md §6` and §7 (the deliberate reversal of "all required
  classes"; exit 4 is `--require-class`'s job).
- `src/select/require.rs`, `src/select/mod.rs` (`is_satisfied` excludes
  `over_budget`).

**Recommended action:** leave as intentional; extensively tested in
`tests/matrix.rs`, `tests/cli_contract.rs`, and unit tests in `select/`.

---

### F10 — Verified consistent (no finding): independence / "no shared contracts"

**Behavior.** `gircap` declares no dependency on the `girpr` crate, never
invokes the `girpr` binary, shares only `girpr-log` (log-file/log-level policy)
and `girpr-tempdir` (scratch-directory policy, dev-dependency), and the flag
names/defaults are shared only for logging. The `girpr-log` crate is a pure
policy crate with no program logic; `girpr-tempdir` is a pure naming/cleanup
helper.

**Evidence.**
- `Cargo.toml:24–58`: only `girpr-log` and `girpr-tempdir` as path deps; no
  `girpr`, no `Command` anywhere in the tree.
- `docs/00-spec.md §2`; `src/lib.rs:40–47`.
- `crates/log/` and `crates/tempdir/`: both are leaf crates (depend on
  `chrono`/`tracing-subscriber` and nothing else / `std` only).

**Recommended action:** none. The one concrete doc lapse in this area is F3
(the `--require-class` help text).

---

### F11 — Verified consistent (no finding): §3.7 manifest-id prefix

**Behavior.** The 16-hex prefix of `manifest.id` is recorded as `opaque` and
never verified: `id_prefix_verified` is always `false`
(manifest/mod.rs:706–708, hyp/types.rs:632–635), and `Verified::all_green`
ignores it (manifest/mod.rs:96–98). The captured `getBuild` body and the
measured values confirm it is not a hash of anything observable.

**Evidence.**
- `docs/00-spec.md §3.7`, `docs/01-artifact-schema.md` (implied).
- `src/hyp/types.rs:614` ("Recorded, never asserted").
- Test `the_manifest_id_splits_into_an_opaque_prefix_and_the_checksum_suffix`
  (`hyp/types.rs:802–816`).

**Recommended action:** none.

---

## Summary table

| ID | Kind | Severity | Location | One-line summary |
|----|------|----------|----------|------------------|
| F1 | Bug (crash) | medium | `src/main.rs:58` | non-Unicode `argv` panics instead of a clean exit-1 usage error |
| F2 | Behavioral gap | medium | `src/select/mod.rs:508–513` | `--files` entries after the first budget-refused one are silently dropped |
| F3 | Doc bug | low | `src/cli/mod.rs:161` | `--require-class` help says "Default: all", actual default is nothing |
| F4 | Doc bug | low | `src/cli/mod.rs:187` | `--no-chunk-blobs` doc says A7-A10, actual skip set is A6–A12 + A19 + A21–A23 |
| F5 | Temp-file hygiene | low | `src/blobs/mod.rs:432–436` | failed chunk fetches leave orphan `.tmp` files in `chunks/blobs/` |
| F6 | Ceiling asymmetry | low | `src/http/fetcher.rs:741–747` | non-2xx response bodies are not subject to `max_response_bytes` |
| F7–F9 | Intentional | n/a | — | A9 byte-vs-digest; `--per-class 0` floored; `--require-class` defaults to nothing |
| F10–F11 | Verified | n/a | — | independence and manifest-id-prefix claims hold |

## Recommended next steps (in priority order)

1. Fix F1 (crash on non-Unicode args) — collect `argv` from `args_os()`.
2. Re-evaluate F2 (budget `break` drops later `--files`) and either change it or pin it with a test.
3. Fix F3 and F4 doc comments.
4. Decide on F5/F6 (cleanup or document; pin with tests if kept).

No PR or commit was made; this report is for human review.
