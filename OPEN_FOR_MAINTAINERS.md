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
6. **Unverified areas** — §6 is resolved: the zero-length manifest shapes are
   probed with crafted fixtures, the panic sweep is written up in
   [`docs/07-panic-audit.md`](docs/07-panic-audit.md) with no unguarded
   server-derived panic found, and `--print-plan --audio` under an active scope
   is executed in both the out-of-scope and in-scope cases; see the resolved
   decisions log at the end.

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

**Resolved (2026-10-30).** All three items are closed by evidence in this tree;
see the [`Resolved decisions`](#resolved-decisions) log at the end and
[`docs/07-panic-audit.md`](docs/07-panic-audit.md).

- **6.1 zero-length manifest shapes** — probed with crafted fixtures through
  the real binary. A legitimate empty file (`size = 0`, `EMPTY_MD5`) passes
  `--check-only`, `--dry-run`, `--print-plan` (a `PLAN` record) and a repair,
  with the file left empty and no `_tmp`. A zero-length entry with a non-empty
  digest is refused by the MD5 gate: `--check-only` exit `4`, `--dry-run` exit
  `0`, `--print-plan` exit `0` with the `PLAN` record, and a repair exit `3`
  that publishes no file. A zero-length entry that declares a chunk is refused
  at plan time by the `end > size` arm (exit `2`). The `..`, `C:` and
  `offset + size > i64::MAX` shapes keep their existing refusing gates and are
  pinned by `a_manifest_path_that_escapes_the_game_dir_is_refused` and
  `a_hostile_chunk_range_refuses_every_read_only_mode_before_it_runs`. **No live
  CDN zero-length entry was available**, so the crafted fixture is the probe;
  the ignored `artifact_bridge` suite can be run separately as extra evidence.
- **6.2 panic sweep** — source audit of `unwrap`/`expect`/direct indexing/
  integer arithmetic on server-derived values in the manifest, parse, plan,
  scan and write paths. No unguarded server-derived panic was found; every
  candidate is checked, saturated or an invariant. The full table and the one
  non-panic observation (a `u64 as i64` wrap in `hyp.rs`) are in
  [`docs/07-panic-audit.md`](docs/07-panic-audit.md).
- **6.3 `--print-plan --audio` under an active scope** — executed in both
  directions. With the scan file out of scope the run exits `0` (the
  print-plan exemption), prints a `PLAN` record and writes no scan file; with
  the scan file in scope it still exits `0`, prints the `PLAN` record and
  defers the write. The two cases pin the independent guards rather than only
  the exit code.

**Cross-project impact (whole §6).** `docs/06` (the crafted-bundle contract)
and `docs/02` (the documented refusals) stay aligned with the probe; `docs/07`
is the new audit record. `crates/log`, `crates/tempdir`, `.config/nextest*` and
the workspace `Cargo.toml`/`Cargo.lock` do not change, and no `tools/capture` or
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
- **6.1** — probe with crafted fixtures: a legitimate `size = 0`/`EMPTY_MD5` file passes `--check-only`, `--dry-run`, `--print-plan` and repair untouched; a zero-length entry with a non-empty digest is refused by the MD5 gate (`--check-only` `4`, `--dry-run` `0`, `--print-plan` `0` + `PLAN`, repair `3`, no file published); a zero-length entry declaring a chunk is refused at plan time (exit `2`). No live CDN zero-length entry was available, so this is fixture evidence; pinned by `a_legitimate_empty_file_passes_every_mode_untouched`, `an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate` and `a_zero_length_entry_declaring_a_chunk_is_refused_at_plan_time`, with `..`/`C:`/overflow still pinned by `a_manifest_path_that_escapes_the_game_dir_is_refused` and `a_hostile_chunk_range_refuses_every_read_only_mode_before_it_runs`. (2026-10-30)
- **6.2** — probe, not `catch_unwind`: the source audit in `docs/07-panic-audit.md` enumerates every `unwrap`/`expect`/index/arithmetic site on server-derived manifest/plan/scan/write values; no unguarded server-derived panic remains (checked, saturated or invariant), and the only non-panic note is the `u64 as i64` wrap in `hyp.rs`. (2026-10-30)
- **6.3** — probe both guards: with the scan file out of scope `--print-plan --audio` exits `0`, prints `PLAN` and writes no scan file; with it in scope the run still exits `0`, prints `PLAN` and defers the write. Pinned by `print_plan_audio_outside_the_scope_is_not_refused_and_writes_nothing` and `print_plan_audio_inside_the_scope_still_defers_the_write`. (2026-10-30)
