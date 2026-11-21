# T14 — gircap: extract the http subsystem crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F25 (lines 310–320) — `http/*`
  ~5,376 lines (3,462 production; owns `reqwest`, `wiremock` in tests,
  `thiserror`); cycle `http/probe.rs:32` ↔ `biz/mod.rs:26`; `config/mod.rs:286`
  uses `reqwest::Url`, so an http split alone does not remove reqwest from the
  whole tool graph.
- §F27 (327–333) — heavy deps group by subsystem but the lib facade defeats
  isolation (§A2).
- Assumptions to validate: §A2 (27–33), §A5 (48–53), §A10 (86–94), §A4 (40–46).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (four fake servers §F40).

## Scope
- `tools/capture/src/http/**` and `tools/capture/src/biz/**` (cycle partners),
  the `reqwest`/`wiremock`/`thiserror` deps, and `tools/capture/src/config/mod.rs:286`.
- Locate the full SCC and all edges in the current tree before editing.

## Implementation plan
1. Validate §F25: confirm the `http ↔ biz` cycle and that `config` uses
   `reqwest::Url`. Decide whether the group moves whole (§A10) or a narrow
   interface seam removes the cycle.
2. Create the http crate owning reqwest and the http mechanics and make gircap
   depend on it directly (§A2) so a change in emit/blobs/manifest leaves the
   heavy reqwest/wiremock code cached.
3. Honor the gircap boundary (§A5): the crate is logic-free, no girpr dep, no
   invoking girpr's binary. Update the §A5/§F39 doc tables in the same work
   (`tools/capture/docs/00-spec.md:42`, `tools/capture/Cargo.toml:1-44`, plus
   the README/member-list seams in §F39).
4. Keep the four fake HTTP servers distinct (§F40) and the test-only seams
   promoted via §A7; update nextest filters (§F31).

## Constraints / validation
- §A2 payoff must be real: a facade that re-exports both halves is not a win.
- Because `config` still uses `reqwest::Url`, verify the split still removes a
   rebuild edge; if not, stop with a reason.
- Do not couple gircap to girpr (§A5) or change any artifact bytes (§A6).

## Done when
The http subsystem is a logic-free crate consumed directly by gircap, the
§A5/§F39 tables are updated, reqwest/wiremock stay cached across unrelated
edits, and all http tests pass with unchanged bytes.
