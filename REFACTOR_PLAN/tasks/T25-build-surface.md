# T25 — Validate and, if safe, shrink the build-time dependency surface

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §10 **B-01** (lines 818–823) — `crates/time/Cargo.toml`
  enables `chrono` with `clock` (pulling `iana-time-zone`, Windows shims) only to make
  a UTC timestamp for the default log filename. A lighter source could reduce the
  dependency surface, but hand-rolling calendar conversion is error-prone and
  discouraged here.
- §11 **assumption 13** (lines 872–874) — confirm with `cargo tree -i chrono` that only
  the timestamp leaf uses it, and that a replacement preserves every pinned spelling
  in `crates/time`.
- §10 "Already tuned — no change proposed" (lines 810–816) and B-02 (825–829) are
  explicitly out of scope; do not touch Cargo profiles or duplicate transitive
  versions.
- Keep items: pinned timestamp spellings, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/time/Cargo.toml` and the source that formats the default log filename.
- Any tests in `crates/time` that pin timestamp spellings.
- Locate current usage before editing; the list above is a hint.

## Implementation plan
1. Run `cargo tree -i chrono` and confirm only the timestamp leaf uses it. If another
   crate needs it, stop — there is no surface win here.
2. Confirm every pinned spelling in `crates/time` and decide whether a lighter source
   can reproduce them exactly without hand-rolling calendar conversion.
3. If (and only if) both checks pass, replace the dependency and keep all spellings
   and tests unchanged.
4. Deliver in one commit; if the validation fails, this task is a clean `skipped`.

## Constraints / validation
- Do not hand-roll calendar conversion; prefer a maintained lighter crate.
- Every pinned timestamp spelling must be preserved.
- Do not touch Cargo profiles or any §12 non-finding.

## Done when
The `chrono` dependency is removed only if validation proves it is safe and all
spellings are preserved; otherwise the task is `skipped` with a one-line reason. A
successful change keeps `crates/time` tests passing.
