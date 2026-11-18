# T22 — Expose `Args` + conversion record from sled2redb lib

Priority P2 · `tools/sled2redb` / production code · checkpoint CP11 ·
report finding D1

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L473–L503` (§D1).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L790–L795`
  (pattern 1), `L818–L820` (Wave 3).
* Report note distinguishing it from the earlier deleted F03 proposal:
  `docs/09-test-scale-reduction-research.md:L501–L503`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T22-sled2redb-args.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Cut sled2redb spawns 10 → ~3–4 and binary conversions 5 → 2 by moving the record
render seam into the library so tests need not spawn a process to inspect it.

`tools/sled2redb/src/main.rs:34–50` holds `struct Args` and :81–88 inlines
`Record::keyed("converted")…emit(args.output)`. Tests (`tests/convert.rs:438–481`
3 spawns + 3 full conversions, :496–586 2 spawns, :604–661 4 spawns, :768–796
1 spawn) spawn only to inspect a rendered record that is a pure function of
`(Converted, src, dst, OutputFormat)`. Enabling APIs already exist in
`tools/sync/src/report/mod.rs:91–96,136–153` and `record.rs:86,100`.

## Implementation plan

Code anchors (validate before editing): `src/main.rs:34–50`, `:81–88`,
`tests/convert.rs:438–481`, `:496–586`, `:604–661`, `:768–796`,
`tools/sync/src/report/mod.rs:91–96,136–153`, `record.rs:86,100`.

Steps (one or two commits):

1. Move `Args` and a
   `pub fn conversion_record(&Converted, &Path, &Path) -> Record` plus a thin
   `conversion_output(...)` built on `Report::render` into
   `sled2redb/src/lib.rs`; `main.rs` calls them.
2. Assert JSON/text/default rendering in-process.
3. Keep **one** spawn for the flag→record→exit-0 path; replace the byte-equality
   spawn with in-process parse+render equality (both spellings already parse
   identically in the 6-row table at :525–532); assert 3 of 4 usage spellings via
   `Args::try_parse_from`.

## Acceptance criteria

* Pure render contract, all parse spellings and the binary's exit-code/`--help`
  wiring all remain.
* spawns 10 → ~3–4, binary conversions 5 → 2, two `build_sled_db` fixtures
  removed.
* Keep `.bin_name(...)` in the help pin. Affected suites pass.

## Guardrails

* `clap` derive moves into lib — it is already a normal dependency
  (`Cargo.toml:32`). Do not delete the two conversions the earlier F03 proposal
  removed: here a new seam makes the properties assertable without converting,
  but the retained spawns still pin distinct flag-spelling/byte-equality facts
  (validate before removing).
* This task unblocks follow-up F02 (D8 — render the real `Args::command()` for
  the help pin). Do **not** implement F02 here; report it as a follow-up.
* Report follow-up ideas without implementing them.
