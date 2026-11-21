# Refactor state

Single source of truth for orchestration. The orchestrating session may be
replaced at any time; all durable knowledge lives here. See `ORCHESTRATION.md`
for the procedure and `CONVENTIONS.md` for the format.

States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
`skipped`. Every change to this file is its own commit. Commit refs are pointers
for reviewers only — no dates, session/tool IDs, machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Break the `girpr-tempdir → girsync` inversion | completed | 220ad882 | Dedicated `girpr-sync-test-support` leaf (deviation over feature/move options); §A2 payoff achieved — tempdir is tempfile-only, girpr/gircap dev graphs no longer pull girsync; review A clean, no fixes |
| T02 | `girpr-util` leaf crate | pending_review | 1665a69f | Facade uses `use girpr_util as util` direct dep; §A2 payoff verified empirically (repair edit leaves util Fresh). Deviation: leaf also needs fs-names/tracing + dev log/tempdir beyond §F14; §F31 filters updated by content (stale line numbers) |
| T03 | Cheapest product leaves batch | todo | | |
| T04 | Writer / file-repair crate | todo | | |
| T05 | Plan / purge / scope decision crate | todo | | |
| T06 | HTTP + Sophon fetch crate | todo | | |
| T07 | Report plane crate | todo | | |
| T08 | CLI / config crate for narrow tests | todo | | |
| T09 | De-duplicate root `tests/common/mod.rs` inlining | todo | | |
| T10 | Consolidate shared test seams and includes | todo | | |
| T11 | girsync: extract the cache crate | todo | | |
| T12 | girsync: extract the acyclic leaves | todo | | |
| T13 | girsync: planner + effective cycle crate | todo | | |
| T14 | gircap: extract the http subsystem crate | todo | | |
| T15 | gircap: emit + verify subsystem crates | todo | | |
| T16 | gircap: blobs + manifest crates | todo | | |
| T17 | gircap: small production leaves | todo | | |
| T18 | Split the disjoint halves of `girpr-digest` | todo | | |
| T19 | `girpr-log` split payoff assessment | todo | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01 | done | Checkpoint A (solo). Review clean, no fix/revert commits; §A2 payoff confirmed, all 2167 tests pass. 2 follow-ups raised |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU-01 | Update `tools/sync/README.md:10` shared-crate inventory to include `girpr-sync-test-support` (fixtures no longer from `girpr-tempdir`) | open | From review A; §F39 doc-seam consistency |
| FU-02 | Add `crates/sync-test-support/` to crate-layout tree in `docs/03-repair-rust-implementation-plan.md:25` | open | From review A; §F39 shared-crate inventory consistency |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code should record
that in the relevant task's `note` instead of silently proceeding):

- **A split must shrink the rebuild set (§A2, lines 27–33).** Editing any file
  invalidates a target and all downstream dependents. A split pays off only if
  high-level consumers (binaries, integration tests) depend on the smaller
  crate directly; if the facade still depends on and re-exports both halves,
  the change adds a crate without removing a rebuild edge. Every task is
  validated against this before it is accepted.
- **The `girpr-tempdir → girsync` inversion is the dominant fan-out fact
  (§F3, lines 140–152).** `crates/tempdir` has normal deps on girsync and
  serde_json for `pub mod test_support`; it is a dev-dep of all four programs.
  Until this edge changes, splitting the six leaves named there cannot shrink
  their rebuild scope. T01 is the enabler for several later tasks.
- **Thirteen shared crates are existing facts (§A3, lines 35–38; §F37,
  lines 424–430).** The `crates/*` leaves from the earlier extraction work are
  not new candidates; §F37 names the ones where further splitting would not
  help (`exit`, `cli`, `audio-language`, `proto`, `filter`, `report-core`,
  `retry`, `model`, `fs-identity`, `fs-names`). Raise a follow-up rather than
  expanding a task into those.
- **gircap independence is a hard boundary (§A5, lines 48–53; doc seams §F39,
  lines 447–454).** gircap may not depend on girpr, may not invoke its binary,
  and may depend only on leaf crates holding no program logic. Adding a leaf
  requires updating the independence table at `tools/capture/docs/00-spec.md:42`
  and the matching manifest comment at `tools/capture/Cargo.toml:1-44` in the
  same work. Do not couple programs to prove one another's tests.
- **Byte contracts are load-bearing (§A6, lines 55–61).** Bundle, report and
  dump bytes are pinned by tests: proto field names `chuncks` /
  `unknown_field6`, `girpr-report-core` deliberately omits `preserve_order`,
  `girpr_log::format_timestamp` pins its format. A split must not change key
  order, casing, spacing or flag conditions.
- **nextest shapes helpers and names (§A7, lines 63–70; §F31, lines 377–383).**
  nextest runs one process per test and no doctests, so shared test helpers must
  be a public library surface reached through thin per-consumer shims, with
  doctest examples mirrored as unit tests. `.config/nextest.toml` pins filters
  to exact binary names and fully-qualified test paths; moving a crate or test
  changes those filters and must be done in the same work.
- **Cycles constrain boundaries (§A10, lines 86–94; §F13, lines 228–232).** A
  boundary must sit at an acyclic leaf or around a whole strongly-connected
  group, with shared types moved down first. Product cycles: report/summary ↔
  repair/scope/ledger, config ↔ repair, scope ↔ phases; tool cycles: util ↔
  cache, effective ↔ planner, cli ↔ config (both tools), http ↔ biz.
- **Divergent policies are intentional keepers (§A13, lines 108–114; §F40,
  lines 456–466; §F38, lines 432–445).** The prior report's N1–N10 and the
  findings in `DO-NOT-TOUCH.md` are duplicate-looking code deliberately left
  local; they are not split candidates. A split that "fixes" them is out of
  scope — raise a follow-up instead.
- **Keep the goal framing (§F7, lines 183–189; §A9, lines 79–84).** The
  bottleneck is link-bound turnaround on low-core hosts; profile settings such
  as `codegen-units` and `debug` affect object count and link size, not which
  packages rebuild. A task that only reduces build size without shrinking the
  rebuild set does not meet the goal.
- **All report items are static, unvalidated claims (§F41, lines 468–470).**
  No item was implemented, measured or tested when the report was written.
  Implementing sessions validate against the current code and may stop.
