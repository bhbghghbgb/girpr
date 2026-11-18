# T07 — Capture: downscale `m7_assembly` megabyte bodies

Priority P0 · `tools/capture` (gircap) · checkpoint CP2 · report finding B3

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L253–L279` (§B3).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L797–L799`
  (pattern 2), `L812–L814` (Wave 1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T07-capture-m7-downscale.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Replace ~28 MB of synthetic bodies/hashing with <10 KB without losing the
structural identity that gives the file its ground-truth value.

`tests/measured_fixture.rs` helpers `measured()` (:1277–1288), `seeded()`
(:1292–1297), `hoyo_pair()` (:1321–1352) and `assemble_measured()` (:1354–1382)
are used by four tests (:1385, :1411, :1440, :1463). Each calls
`assemble_measured(&hoyo_pair())`, building `690_719 + 1_626_019 + 23_526` bytes
per side (~4.7 MB total, ~28 MB allocated + hashed across four tests) only to
pin literal totals. None of the assembly decisions (reuse, A7–A12,
patch-vs-direct byte equality, `must_not_fetch`) depend on the magnitudes.

## Implementation plan

Code anchors (validate before editing): `tests/measured_fixture.rs:1277–1288`,
`:1292–1297`, `:1321–1352`, `:1354–1382`, and the four tests at :1385/:1411/
:1440/:1463.

Steps (one commit, or two: helper parameterization then assertions):

1. Parameterize `hoyo_pair` with the three lengths.
2. Keep **one** test on the measured lengths to preserve the ground-truth pin
   (literal totals such as `download_bytes == 274_426`,
   `materialize_old/new_bytes == 2_340_264`, `n.bytes.len() == 2_340_264`,
   `uncompressed_size == 690_719`).
3. Run the other three with tiny lengths (e.g. 64/128/16) and express their size
   assertions as scale-free relations (`download_bytes == sum(fetch chunk
   sizes)`, `uncompressed_size == chunk.uncompressed_size`, …).

No production change needed — `assemble::assemble` already accepts
`ChunkSource::from_memory`.

## Acceptance criteria

* All four tests kept; one retains the exact measured-value ground truth.
* The structural identity is untouched: real id prefixes, one chunk shared
  across both builds, two different chunks at the same offset.
* For the three downscaled tests, verify the new scale-free assertions evaluate
  to the old literals when fed the measured lengths (report's stated risk).
* ~28 MB → <10 KB of bodies/hashing. Affected suite passes.

## Guardrails

* Do not downscale all four tests — the measured-length pin must survive.
* Do not touch the distinct-state blob measurement tests (report §B non-findings,
  L332–L340) or `assemble/tests.rs`'s 35 distinct-assertion tests.
* Test-only change. Report follow-up ideas without implementing them.
