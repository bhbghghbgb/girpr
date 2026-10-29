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

1. **Bytes that can be deleted** — §1 is resolved (case-sensitive purge
   membership, an incomplete delete plan behind a ceiling, the empty-directory
   sweep); see the resolved decisions log at the end. These acted on the
   irreversible half and had concrete failure modes.
2. **What a read-only run promises** — §2 is resolved: `build_plan` validates
   chunk ranges, so a malformed range is exit `2` before any mode runs and a
   printed plan is always runnable; see the resolved decisions log at the end.
3. **Exit-code strictness of local state** — §4 is resolved: every item keeps
   the strict local-state rule (R-2, T-1/E-1, V-2, T-2/E-3, U-1); see the
   resolved decisions log at the end.
4. **Counters and the disk model** — §3 is resolved: `kept_bytes` keeps its
   documented meaning and `discard` keeps its growth cost (F-9, E-2); see the
   resolved decisions log at the end.
5. **Evidence depth** — §5 is resolved: P-2's two in-place stat sites share a
   private classifier so the error arm is unit-tested, and F-6/S-3/R-5 are
   accepted as-is; see the resolved decisions log at the end.
6. **Unverified areas** — §6. Probe or accept; do not decide them from the
   current mock/reference corpus.

---

## 1. Plan identity and irreversible deletion

**Resolved (2026-10-30).** N-5, N-19 and S-1 are all resolved; see the
[`Resolved decisions`](#resolved-decisions) log at the end of this file.

---

## 2. Read-only runs and the plan boundary

**Resolved (2026-10-30).** `build_plan` now validates every non-folder entry's
chunk ranges after the blacklist/scope/dedup filters, so a malformed range is a
plan/metadata error (exit `2`) before any mode runs and `--print-plan` cannot
print an unrunnable plan. See the [`Resolved decisions`](#resolved-decisions)
log at the end of this file.

---

## 3. Counters and the disk model

**Resolved (2026-10-30).** F-9 and E-2 both keep the documented behaviour:
`kept_bytes` stays "bytes this run did not have to write" and `discard` keeps
its growth cost. See the [`Resolved decisions`](#resolved-decisions) log at the
end of this file.

---

## 4. Boundary strictness and stat state

**Resolved (2026-10-30).** R-2, T-1/E-1, V-2, T-2/E-3 and U-1 all keep the
strict local-state rule: an explicit configuration does not degrade silently.
See the [`Resolved decisions`](#resolved-decisions) log at the end of this file.

---

## 5. Stat-failure and panic evidence seams

**Resolved (2026-10-30).** The evidence depth stays as accepted: these paths
have no public injection seam and none is added. The code is source-read at the
failure arm except where a cheap classifier exists. F-6 (a panicked scan task),
S-3 (the deprecated-stat failure arm) and R-5 (`write_config_ini`'s `sync_all`)
are accepted as-is. P-2's two in-place `metadata()` sites now share a private
two-state classifier over `io::Result<Metadata>`, so the error arm is executed
in a unit test; production still fails the file (exit `3`) with the same path
context. See the [`Resolved decisions`](#resolved-decisions) log at the end of
this file.

**Cross-project impact.** `.config/nextest.toml` and
`.config/nextest-test-env{,.cmd}` are workspace-wide and carry both
`GIRPR_TEST_NO_FSYNC` and `GIRSYNC_TEST_NO_FSYNC`; no new hook was added, so
they do not change. `crates/log` and `crates/tempdir` are unaffected (the
latter is dev-only scratch, `girpr_tempdir::TempDir`, with no API change).
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

---

## Resolved decisions

- **N-5** — option (b), fold case on Windows in the purge: mirrors the plan's dedup and the case-insensitive scope, so `--purge-*` cannot delete a just-verified file under a differently-cased on-disk name. (2026-10-30)
- **N-19** — option (b), refuse the delete pass when a ceiling is set and the plan is incomplete: a lower bound cannot enforce an exact-set ceiling, so exit `5` names `purge_errors` instead of deleting against it; a real counted walk failure is driven through both checks in `purge::tests`. (2026-10-30)
- **S-1** — keep (a), document the directory-sweep consequence: the keep rule is the safer irreversible outcome, and a content glob does not hide its directory. (2026-10-30)
- **2.1** — option (b), plan-time chunk-range validation (exit `2`) so every read-only mode agrees; docs/02 copies on the tool branches need the same statement at merge. (2026-10-30)
- **F-9** — keep (a): `kept_bytes` is the complement of settled writes and counts a resumed `_tmp` prefix, matching the field's documented meaning; a verified number would be a new counter, not a redefinition. (2026-10-30)
- **E-2** — keep (a): `discard` reports the target's growth (`size - on_disk`), which is what a truncated target needs; the old `0` was a documentation error. (2026-10-30)
- **R-2** — keep (a): an unreadable audio scan file is exit `1`; only `NotFound` is absent, so a transient ACL failure cannot silently replace the user's selection or steer a purge. (2026-10-30)
- **T-1/E-1** — keep (a): a mixed ignore/blacklist file warns per bad line and is refused only when it yields no usable entry; typed-field failures are bad lines, not silent skips. (2026-10-30)
- **V-2** — keep (a): an uncleannable blacklist entry is fatal; protection must not degrade silently, unlike the optimisation-shaped reuse map. (2026-10-30)
- **T-2/E-3** — keep (a): `FileLen` keeps present, absent and unreadable distinct and a directory at a file path is still reported damaged; no new enum arm for a diagnostic improvement. (2026-10-30)
- **U-1** — keep (a): normalisation happens once at each boundary, and the two read-only directory inputs are validated at `contained_join`; no defensive second clean. (2026-10-30)
- **F-6** — accept (a): the scan `JoinError` warning is not load-bearing for correctness and the estimate is conservative; no test-only panic sentinel in the hot loop. (2026-10-30)
- **P-2** — option (b): the two in-place stat sites classify `io::Result<Metadata>` so the error arm is unit-tested; production still fails the file (exit `3`) with path context. (2026-10-30)
- **S-3** — no change: the four-state `classify_deprecated` is already the seam and is executed; the syscall wrapper needs none. (2026-10-30)
- **R-5** — accept (a): the linear temp-write/sync/rename path already shares the write-error cleanup; a fault-injection hook costs more than the risk. (2026-10-30)
