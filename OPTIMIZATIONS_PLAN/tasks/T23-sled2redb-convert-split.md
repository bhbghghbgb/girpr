# T23 — Split `convert_into` at the sled-read / redb-write boundary

Priority P2 · `tools/sled2redb` / production code · checkpoint CP11 ·
report finding D2

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L505–L527` (§D2).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L797–L799`
  (pattern 2), `L818–L820` (Wave 3).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T23-sled2redb-convert-split.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~5 of 7 sled fixture builds (including 5 fsync-bearing conversions) by
extracting the pure redb-write half of `convert_into`.

`src/convert.rs:191–264` does sled open/parse (:192–226), `drop(old)` (:229),
redb write/count/commit (:231–254) and cleanup (:260–262). The `build_sled_db`
fixture (`tests/convert.rs:78–141`: sled open, meta + 3 rows, one with 3
algorithms, `flush()`/fsync, sidecars) is rebuilt 7 times (:200, :266, :335,
:380, :420, :442, :500); `sled_to_redb` is called 11 times. Only the round-trip
test needs real hex/JSON decoding and the 5-hash count.

## Implementation plan

Code anchors (validate before editing): `src/convert.rs:191–264`,
`tests/convert.rs:78–141`, `:200`, `:266`, `:270`, `:335`, `:341`, `:380`,
`:384`, `:420`, `:424`, `:442`, `:500`.

Steps (one or two commits):

1. Extract
   `pub fn write_records(temp: &Path, case_sensitive: bool,
   recs: &[(String, FileRec)]) -> Result<Converted>` from :231–262;
   `convert_into` becomes read+parse then delegate.
2. Keep one real sled DB for the round-trip and one bad-row DB for the error path.
3. Rewrite the count/force/atomicity tests (:270, :341, :384, :424) to call
   `write_records` with 1–2 synthetic rows and assert the same counters/file
   outcomes (adjust expected `Converted` counts).

## Acceptance criteria

* Decoding still pinned by round-trip + bad-row cases; counters, case routing,
  unknown-algorithm passthrough and cleanup still asserted.
* ~5 of 7 sled fixture builds removed, including 5 fsync-bearing conversions.
* Preserve case mode from meta, foreign-algorithm routing and exact counters
  (the report's stated risks). Affected suites pass.

## Guardrails

* Preserve `drop(old)` ordering and the temp-sibling cleanup semantics when
  extracting `write_records`; if the boundary cannot be drawn cleanly, stop and
  report `skipped` rather than changing conversion behavior.
* Do not re-touch the spawns T22 keeps, and do not implement follow-up F02.
  Report follow-up ideas without implementing them.
