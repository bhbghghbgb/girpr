# T15 — gircap: emit + verify subsystem crates

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F25 (lines 310–320) — `emit/*`
  ~6,015 lines (owns `flate2`, `walkdir`, `chrono`, `ini_core`) and `verify/*`
  ~4,365 lines (2,974 production; top-level, no network).
- §F27 (327–333) — single lib facade defeats isolation (§A2).
- Assumptions to validate: §A2 (27–33), §A5 (48–53), §A4 (40–46), §A6 (55–61).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/emit/**`, `tools/capture/src/verify/**`.
- Test-only `#[cfg(test)] use` names in `verify/mod.rs:62-76` that support
  `use super::*` (§F28).
- The `flate2`/`walkdir`/`chrono`/`ini_core` deps.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate that emit and verify are acyclic (or find their SCC) and have direct
   high-level consumers (§A2). They may be separate crates or one group —
   decide by the dependency edges, not by size.
2. Move each subsystem behind a crate boundary and make gircap depend on it
   directly, so changes in http/blobs/manifest leave emit/verify cached.
3. Promote test-only seams via the §A7 pattern (public lib + thin shim) so
   `use super::*` tests keep working (§F28).
4. Honor the gircap boundary (§A5) and update §A5/§F39 doc tables in the same
   work; update nextest filters (§F31) if paths move.

## Constraints / validation
- §A2 payoff must be real (no facade-only split).
- Preserve artifact/dump bytes and INI parsing behaviour — INI parsers are
   deliberate keepers (§A6, §A13).
- No girpr dependency or binary invocation (§A5).

## Done when
Emit/verify are logic-free crates consumed directly by gircap, heavy deps stay
cached, test seams work, and all emit/verify tests pass with unchanged bytes.
