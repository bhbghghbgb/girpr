# T31 — `JoinSet` window → `buffer_unordered`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B3 (lines 401–409).

## Scope
- Package: root `girpr`, `src/repair/file/mod.rs` (lines 207–267, 384–447).
  Reference implementations: `src/repair/scan.rs` (313–341),
  `src/repair/phases/check.rs` (88–122), `src/repair/phases/manifest.rs`
  (150–157).

## Implementation plan
1. Consolidate the ~65 LOC of `JoinSet` window management on
   `futures_util::stream::iter(..).buffer_unordered(jobs)`, matching the other
   modules, or extract one bounded-executor helper.
2. If panic reporting must be preserved, consider `futures-concurrency`
   (`Tasks`/`Group`) (§B3).

## Constraints / validation
- `buffer_unordered` detaches on drop, while `file/mod.rs` (lines 59–63) treats
   abort-on-drop as intentional — preserve that behavior or explicitly accept
   and record the change.
- Tests named in §B3 (line 407): `file/mod.rs` and `tests/repair_machinery.rs`.

## Done when
The `JoinSet` window is consolidated and abort-on-drop behavior is resolved.
