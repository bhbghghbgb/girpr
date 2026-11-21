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
| T02 | `girpr-util` leaf crate | completed | 1665a69f, dd8c6d13 | Facade uses `use girpr_util as util` direct dep; §A2 payoff verified empirically (repair edit leaves util Fresh). Deviation: leaf also needs fs-names/tracing + dev log/tempdir beyond §F14; §F31 filters updated by content. Review B clean; dd8c6d13 fixed doc seams only |
| T03 | Cheapest product leaves batch | completed | 46ede904, dd8c6d13 | Split only `girpr-lock` (true leaf, direct consumers); other 6 left local. Review B: biz/phase/filter reasons sound; audio reason no longer holds now util is external (see FU-03); logging/retry reasons cite wrong constraints but low-payoff outcome defensible (FU-04). §A2 payoff verified empirically; dd8c6d13 fixed doc seams only |
| T04 | Writer / file-repair crate | skipped | | Boundary crossed by three §A10/F13 cycles (report/summary↔ledger, config↔repair, scope↔phases) + wide interface needs most of repair moved down (§F15 stop); §F27 heavy-dep claim stale (zstd→sophon/stream.rs, tempfile→phases/post.rs only) |
| T05 | Plan / purge / scope decision crate | skipped | | Cycle-bound like T04: plan/purge/scope in config↔repair↔file↔phases↔report SCC; moving Meter/Summary down still leaves OUT-edges to sophon, manifest::LocalChunkMap, config::RepairMode, file::Mode, Record — no clean leaf, §A2 fails |
| T06 | HTTP + Sophon fetch crate | completed | aff092c2 | New `girpr-fetch` leaf (hyp/sophon/retry); ReportProvenance severed via borrowed-scalar interface seam, no report move; new narrow `tests/fetch_direct.rs` direct consumer. §A2 verified empirically (repair edit leaves fetch Fresh). Dev: resolved earlier blocker claim — fetch not in SCC, provenance seam works, gircap unchanged. Review D clean |
| T07 | Report plane crate | completed | 89091f68 | New `girpr-report` reuses report-core; resolved §F13 summary↔ledger cycle by moving whole 2-node group (§A10), facade alias + binary direct consumer; §A2 achieved, report byte tests pass. Dev: §F18 line count stale; no nextest filter edits needed; gircap unchanged. Review D clean |
| T08 | CLI / config crate for narrow tests | skipped | | Narrow tests still link facade via `mod common;` (§A2 fails); cli/config crate would cut config↔repair SCC for Limits (§A10); §F22 premise "needs only girpr::Args" false. Revisit after T09/T10 de-dup the shared harness |
| T09 | De-duplicate root `tests/common/mod.rs` inlining | completed | b6fe1c4d | New facade-free `girpr-test-fixtures` lib (~1400 lines); `tests/common/mod.rs` shrank 1521→~145-line shim; all 11 inline + 3 #[path] consumers unchanged; §A2 verified empirically (fixtures Fresh after repair edit). Dev: small glue (RunCtx/Limits/PathScope/RepairMode) still needs facade SCC — left for T10; §F29 counts stale. Review E clean |
| T10 | Consolidate shared test seams and includes | completed | ee2066fd | Root facade-glue → `girpr-test-support` lib (compile-once, not rebuild-edge; T08 still SCC-blocked); artifact replay server → `girpr-test-fixtures`; gircap mock → `gircap-test-support` (independence table + manifest comment updated); girsync runner glue → sync-test-support. Six-harness merge skipped per §F32 (unfavourable trade-off). nextest filters unchanged; all suites pass. Review E clean |
| T11 | girsync: extract the cache crate | completed | 87ce6cf6 | New upstream `girpr-sync-cache` leaf owns whole cache subtree + redb/postcard/tempfile and hash vocabulary (wire format); util↔cache cycle broken by moving ts_now/copy_dir_all/identity helpers down; direct consumers girsync (re-export alias) + sled2redb. §A2 verified empirically (cache/redb/postcard/tempfile Fresh after girsync edit). Dev: T12 must NOT re-extract hash from girsync — it now lives in sync-cache; no nextest filter changes needed. Review F clean |
| T12 | girsync: extract the acyclic leaves | completed | a6503f7e | New `girpr-sync-leaves` holds rw/filter/link/hardlink verbatim; girsync direct dep + old-name re-exports; direct consumer `crates/sync-leaves/tests/direct.rs`; §A2 verified empirically (cache+leaves Fresh after planner edit). Dev: hash not extracted (T11 already moved to sync-cache); scan.rs left local — imports crate::util + crate::link, extracting would invert dep via util→cache (§A10/§A2); gate/policy unchanged. Review F clean |
| T13 | girsync: planner + effective cycle crate | skipped | | planner/effective not a standalone 2-node SCC — sit in 6-module girsync SCC {cli,config,diff,effective,planner,report}; a legal whole-group move needs scan/util/hash too (broad, not narrow interface), §A2 fails; stop per task instruction |
| T14 | gircap: extract the http subsystem crate | completed | b8b31045, 0670b7c9 | New logic-free `gircap-http` (tools/capture/http) owns reqwest/thiserror mechanics; gircap direct dep, src/http/mod.rs thin facade (probe/negative/tests kept local as policy). §A2 verified empirically (emit edit leaves gircap-http/reqwest/retry Fresh). Dev: §F25 SCC actually 11 modules (http↔manifest via negative, http↔failure, biz↔cli↔config) — carved acyclic leaf instead; doc seams updated in same work; wiremock stays gircap dev-dep. Review G1 clean; 0670b7c9 removed 6 superseded dead-duplicate mechanics files |
| T15 | gircap: emit + verify subsystem crates | skipped | | emit/verify not logic-free leaves: emit in SCC with manifest/assemble/http (manifest→emit, assemble→emit back-edges); verify depends on emit policy types; no direct consumer, no heavy dep isolatable (flate2/ini_core stay, ini=§A13 keeper). Legal extraction needs whole-SCC move + byte-contract rewrite (§A6) |
| T16 | gircap: blobs + manifest crates | skipped | | §F25/F27 premise stale: wire types already in girpr-proto, zstd helpers in acyclic `hash` leaf; blobs/manifest deep in gircap SCC (blobs→manifest→emit→blobs, manifest↔emit, http↔manifest) with no T14-style acyclic logic-free leaf; §A2 fails |
| T17 | gircap: small production leaves | pending_review | 07dcd08e | New logic-free `gircap-hash` (tools/capture/hash) owns zstd/flate2/md-5 mechanics; gircap direct dep + `pub use gircap_hash as hash`; zstd_measure pair stays duplicated (§A13); gzip test seam behind test-support feature. §A2 verified empirically (emit edit leaves hash+http Fresh). Other candidates skipped (tasks=shim, exit/log/report=§F37 keepers, encoding/paths no heavy dep, pathmap/rejections use SCHEMA_VERSION). §A5/§F39 seams updated; no nextest filter changes |
| T18 | Split the disjoint halves of `girpr-digest` | todo | | |
| T19 | `girpr-log` split payoff assessment | todo | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01 | done | Checkpoint A (solo). Review clean, no fix/revert commits; §A2 payoff confirmed, all 2167 tests pass. 2 follow-ups raised |
| B | T02, T03 | done | Checkpoint B. Both completed; §A2 payoff confirmed empirically for util+lock; one doc-seam fix (dd8c6d13). 3 follow-ups raised |
| D | T06, T07 | done | Checkpoint D (T08 skipped, not reviewed). Both completed, no fix/revert; §A2 payoff confirmed empirically for fetch+report, bytes/SCC/fake-servers checks pass. 4 follow-ups raised |
| E | T09, T10 | done | Checkpoint E. Both completed, no fix/revert; public-lib + thin-shim, nextest filters, cross-tool independence, four-fake-server and §F32 skip all verified. 3 follow-ups raised |
| F | T11, T12 | done | Checkpoint F (T13 skipped, not reviewed). Both completed, no fix/revert; byte contracts, redb pin, util↔cache SCC, policies, direct consumers all verified; §A2 confirmed empirically. 2 follow-ups raised |
| G1 | T14 | done | Checkpoint G1 (T15 skipped, not reviewed). T14 completed; §A2/§A5/§F40/§F39 verified; one cleanup fix (0670b7c9 removed dead duplicate mechanics files). 3 follow-ups raised |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU-01 | Update `tools/sync/README.md:10` shared-crate inventory to include `girpr-sync-test-support` (fixtures no longer from `girpr-tempdir`) | open | From review A; §F39 doc-seam consistency |
| FU-02 | Add `crates/sync-test-support/` to crate-layout tree in `docs/03-repair-rust-implementation-plan.md:25` | open | From review A; §F39 shared-crate inventory consistency |
| FU-03 | Extract `repair/audio.rs` into a leaf crate — now a true leaf since util is external; same §A2 argument as `girpr-lock` | open | From review B; T03's "uses crate::util" reason no longer holds |
| FU-04 | Re-assess `logging.rs` / `retry.rs` with correct reasoning (cited §A5/§F37 do not apply to product-only files); decide with a concrete rebuild-edge argument or drop | open | From review B |
| FU-05 | Consider a narrow audio/lock-only consumer test target so the §A2 win shows in link count, not just compile caching (current `tests/run_lock.rs` links the full facade) | open | From review B |
| FU-06 | Relocate `tests/fetch_direct.rs` into `crates/fetch/tests/` so the §A2 win shows in link count (currently a girpr-package target re-linked on repair edits) | open | From review D |
| FU-07 | Add `girpr-report` to the nextest crate-list comment in `.config/nextest.toml` (T06 added girpr-fetch; T07 did not add girpr-report) | open | From review D; docs-only, no filter affected |
| FU-08 | Restore the README paragraph break around the crate inventory (report paragraph runs into the fetch sentence) | open | From review D; docs-only |
| FU-09 | Narrow `ReuseEntry`'s unconditional `pub` in shipped `girpr-fetch` (was `#[cfg(test)]` in facade; now public only for facade unit tests) | open | From review D |
| FU-10 | Refresh `tools/capture/docs/02-plan.md:124` layout tree — labels `common/mod.rs` as local mock; mock now in gircap-only `test-support/`, common is a thin shim | open | From review E; remaining §F39 doc-seam gap |
| FU-11 | Update `tools/sync/README.md:1754` — still says shared fixtures live in `tests/common/mod.rs`; now from `girpr-sync-test-support` via shim (see FU-01) | open | From review E; §F39 inventory consistency |
| FU-12 | Revisit narrowing facade-coupled `girpr-test-support` glue — still normal-depends on full `girpr` facade; compile-once win, no rebuild edge; T08 narrow CLI tests remain blocked | open | From review E; §A2, tied to config↔repair SCC |
| FU-13 | Add `girpr-sync-leaves` to §F39 doc seams — `README.md:48` crate table, `README.md:443` prose inventory, `docs/03-repair-rust-implementation-plan.md:28` layout tree | open | From review F; docs-only inventory consistency |
| FU-14 | Drop now-unused `girpr-digest` direct dependency from `tools/sync/Cargo.toml` (vocabulary moved to `girpr-sync-cache`, reached transitively) | open | From review F; dependency hygiene, §A2 cleanup |
| FU-15 | Drop gircap's direct `reqwest` dep by replacing `config::validate_base`'s `reqwest::Url` with the `url` crate, leaving reqwest owned solely by `gircap-http` | open | From review G1; §F25 http split alone cannot remove reqwest |
| FU-16 | Relocate mechanics-only tests from `tools/capture/src/http/tests.rs` into `tools/capture/http/tests/` so HTTP edits re-link only the gircap-http test binary (keep probe/negative policy tests local) | open | From review G1; §A2 test-link payoff |
| FU-17 | Document deliberate gircap-owned deviations (gircap- prefix, tools-only lint policy instead of girpr-<policy>/[lints] workspace = true) in crate/independence-table notes so a future session does not "correct" them | open | From review G1; §A4 deliberate deviation |

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
- **Writer/file boundary is cycle-bound (T04 validation, §A10/F13, §F15).**
  `repair/file` consumes `Summary`, `StoppedBy`, `RepairMode`, plan types and
  `ScanReport` across three cycles: report/summary↔scope/ledger, config↔repair,
  scope↔phases. Its interface is 12 fields used program-wide, so a leaf would
  require moving most of repair down. Later cycle-sensitive splits (T05, T13)
  must account for these cycles; do not re-attempt T04 as a leaf without first
  breaking them.
- **§F27 heavy-dep claim is stale (T04 validation).** In the current tree
  `repair/file` contains no zstd (real code in `sophon/stream.rs`) and no
  tempfile (only `phases/post.rs`, a DO-NOT-TOUCH atomic-publish item);
  reqwest/tokio are spread broadly, not concentrated in the writer.
- **Product mid-level SCC blocks T04 and T05 (T05 validation, §A10/F13).**
  `config`, `repair/file`, `repair/phases` (plan + manifest), `scope`
  (ledger/report/limits/filter), `report` (summary/emit) and `sophon` chunk
  types form one strongly-connected group. Moving `Meter`/`Summary` down alone
  leaves out-edges to `sophon::{FileReuse,SophonChunk}`, `manifest::LocalChunkMap`,
  `config::RepairMode`, `file::Mode` and `report::Record`; a boundary around the
  whole SCC would be most of the product, so §A2 fails. Revisit only after the
  needed subsystems are extracted as leaves and the out-edges are severed.
  Correction from T06 validation: fetch/Sophon is NOT part of this SCC (no
  back-edge to report/config/repair), and the `report/emit` hyp-type edge is
  severable with a borrowed-scalar `ReportProvenance` seam (T06) — T07 is not
  gated by it. T07 then resolved the summary↔ledger cycle by moving the whole
  2-node group into `girpr-report` (§A10 boundary around the group); the
  `report::Record` out-edge now points at `girpr-report` (direction repair→report)
  and no longer gates report splits. Remaining out-edges for a T04/T05 revisit
  are `manifest::LocalChunkMap`, `config::RepairMode` and `file::Mode`. Do not
  re-attempt T04/T05 as leaf splits before those are addressed.
- **Narrow CLI tests are blocked by the shared harness (T08 validation, §F22).**
  Both `tests/cli_contract.rs` and `tests/process.rs` declare `mod common;`, whose
  fixture imports many `girpr::*` types and calls `girpr::run`, so a cli/config
  crate would not remove a rebuild edge (§A2). `RunCtx` embeds `Limits`/`PathScope`
  from the config↔repair SCC (§A10). Revisit T08 only after T09/T10 make the
  shared test harness facade-free and that SCC is addressed. The §F22 claim that
  cli_contract needs only `girpr::Args` is contradicted by the current code.
- **gircap SCC is larger than §F25 claims (T14 validation, §A10/§F41).**
  The report's `http↔biz` cycle is actually an ~11-module SCC including
  `http↔manifest` (via `negative.rs`), `http↔failure`, and `biz↔cli↔config`.
  T14 extracted only the acyclic mechanics leaf (`gircap-http`); `probe`/
  `negative` and their manifest/failure edges stay inside gircap's SCC.
  T15–T17 must account for this rather than assuming a 2-node cycle. A whole-SCC
  move would leave gircap a re-exporting facade (§A2 fail). T15 found emit is
  also in this SCC (manifest→emit via dump.rs, assemble→emit via types.rs, plus
  emit→http), and verify depends one-way on emit's policy types (selfcheck/
  index). Any legal emit/verify extraction needs those back-edges severed by
  moving shared schema/policy types down — a whole-SCC move that rewrites §A6
  byte contracts. T16/T17 should expect the same SCC unless they find a
  T14-style acyclic mechanics leaf. T16 found blobs/manifest deep in that SCC
  (blobs→manifest→emit→blobs, manifest↔emit, http↔manifest) with no carvable
  leaf. T16 also established the heavy-dep premise is stale: wire types are
  already in `girpr-proto` (`manifest/proto.rs:17` re-exports them) and zstd
  helpers (`is_zstd`, `zstd_decompress`, `zstd_measure`/`_from`, `HashingFile`)
  live in the acyclic production leaf `tools/capture/src/hash/` — T17 territory,
  not T16's. The `zstd_measure` pair stays duplicated (§A13).
- **girsync planner/effective sit in a 6-module SCC (T13 validation, §A10).**
  Production `use crate::` edges put `{planner, effective, config, cli, report,
  diff}` in one strongly-connected group (§A10 also names `cli↔config` and
  `report↔diff`). `commands` and `cache` are NOT in it, but moving the whole
  group still leaves out-edges through `effective → scan`/`util` and `crate::hash`
  plus `diff → util`, so a legal boundary would be nearly the whole lib — the
  interface to commands would not be narrow (§A2 fails). Revisit only after
  shared types (`HashMode`/`StatTrust` for `config → planner`,
  `CommonOpts`/`ScanMode` for `effective → config`) are moved down to sever those
  edges; note `CommonOpts` depends on report/cache, cascading toward the product
  SCC that blocked T04/T05. Do not re-attempt T13 as a partial split.
- **T09 moved the bulk harness, not the facade glue (T09 validation).** The
  ~1400-line mock/fixture mechanics now live in facade-free `girpr-test-fixtures`;
  `tests/common/mod.rs` is a ~145-line shim whose remaining glue (`scope`,
  `ctx_for`, `load_counters`, `run`, `run_binary`, `stamp_ago/ahead`) still needs
  `RunCtx`/`Limits`/`PathScope`/`RepairMode` from the config↔repair SCC. T10 is
  the place to finish that seam; only then would T08's narrow CLI tests be
  unblocked (and still only with the SCC addressed).
