# T12 — Pure `DiskPlan::from_figures` seam

Priority P1 · root product unit tests / production code · checkpoint CP4 ·
report finding C5

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L425–L445` (§C5).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L797–L799`
  (pattern 2), `L815–L816` (Wave 2).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T12-disk-plan-figures.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove four temp dirs and six files by driving `DiskPlan` arithmetic through a
pure injected-figures seam.

`src/repair/scope/report.rs` tests at :557–589, :632–654, :698–709 and :747–770
create four temp dirs and six files solely to drive `DiskPlan::new` (:80–143),
which does two stats per file (`util::file_len` :112 and `resolve_mode`'s
`symlink_metadata` at `file/mod.rs:688`). The pure single-file seam
`planned_on_disk` (:161–175) already exists.

## Implementation plan

Code anchors (validate before editing): `src/repair/scope/report.rs:80–143`
(`DiskPlan::new`), `:161–175` (`planned_on_disk`), the four tests at :557–589 /
:632–654 / :698–709 / :747–770, `util::file_len` (:112) and `resolve_mode`
(`file/mod.rs:688`).

Steps (one or two commits):

1. Extract
   `DiskPlan::from_figures(plan, impl Iterator<Item=(FileLen, bool /* regular */)>,
   requested, jobs)`.
2. `new()` becomes a thin I/O wrapper supplying the two facts (file length and
   regular-file bool) from its two stats.
3. Rewrite the four tests to feed `FileLen::Present(128)` / `Absent` /
   `Unreadable` and a bool, keeping the same assertions.

## Acceptance criteria

* Identical arithmetic and saturation; only the measurement source is injected.
* Symlink/dir → `temp` semantics preserved (the report's stated risk).
* −4 temp dirs, −6 files, ~8 fewer stats; production simplification (two stats
  → one per file).
* **Do not** change `resolve_mode` itself — it is shared with the repair path;
  move only its decision into the injected input.
* Affected suite passes.

## Guardrails

* Preserve the `resolve_mode` decision as injected data rather than duplicating
  its logic in tests. If the seam cannot preserve the symlink/dir distinctions,
  skip and report why.
* Report follow-up ideas without implementing them.
