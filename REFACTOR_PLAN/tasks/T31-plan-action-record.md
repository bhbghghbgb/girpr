# T31 — Shared plan-action record crate

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F36 (lines 520–526) — plan records and
  report records are two document shapes tempting a shared abstraction; candidate boundary
  is a shared "plan action record" crate. **High-risk** because girsync's stream is NDJSON
  and girpr's is a different document; unifying the writer changes exit/summary parsing and
  the artifact JSON contract. Confidence medium.
- Assumptions to validate: §A5 (57–67), §A6 (68–75), §A7 (76–80), §A2 (29–36), §A14 (129–135), §N5 (713–716).

## Scope
- girsync `tools/sync/src/commands/sync/mod.rs` (diff plan then actions, one terminal
  summary, `because=` never `why=`), `report/mod.rs`, `commands/mod.rs` (exit/summary parsing);
  product `src/repair/scope/report.rs` (`PLAN` line + JSON document). Locate before editing.

## Implementation plan
1. Validate §F36: confirm the two document shapes and that exit/summary parsing depends on
   them.
2. If attempted, share only a value-level "plan action record" type; keep each program's
   writer/format local. Do not unify the writer.
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If a shared writer is required to get value, stop — it changes pinned contracts.

## Constraints / validation
- High-risk (§F36): girsync NDJSON vs girpr JSON; unifying changes exit/summary parsing
   and artifact JSON contract (§A5, §A7). `because=` wording must stay (§F36).
- Keep the rw gate / bounded runner local (N5). Report planes stay distinct (§A6).

## Done when
A value-only record is shared — or a validated skip with the one-line reason is recorded.
