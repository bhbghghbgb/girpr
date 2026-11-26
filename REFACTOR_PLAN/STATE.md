# Refactor state

Single source of truth for orchestration. The orchestrating session may be replaced at
any time; all durable knowledge lives here. See `ORCHESTRATION.md` for the procedure and
`CONVENTIONS.md` for the format.

States: `todo` → `in_progress` → `pending_review` → `completed` / `reverted` /
`skipped`. Every change to this file is its own commit. Commit refs are pointers for
reviewers only — no dates, session/tool IDs, machine data or change log.

## Tasks

| ID | Title | State | Commit ref(s) | Note |
|----|-------|-------|---------------|------|
| T01 | Purge parallel map → rayon | pending_review | aa091ace | Deviation: eight-thread ceiling dropped (unpinned perf heuristic); rayon global pool sizes itself. Follow-up suggestion held for reviewer: docs/04-scope-limitation-spec.md:727 "scoped worker" wording. |
| T02 | Repair futures combinators → futures-util | pending_review | 9af85115 | Deviation: added per-fan-out Semaphore (FuturesOrdered polls all; ManifestBudget caps at 4 only); both permits acquired inside each fetch. Follow-up held for reviewer: flaky logs_of/spawn_blocking_traced capture test (baseline-reproduced, unrelated). |
| T03 | Salvage order → petgraph | todo | | §R6 lines 212–227; high risk, §A1 lines 15–16 — stop if the safety argument cannot be re-established |
| T04 | Root test origin server → axum | todo | | §R2 lines 157–170, §X1 lines 56–89; large test blast radius |
| T05 | CLI help snapshots → trycmd + strip-ansi | todo | | §R3 lines 172–184, §LEAF-2 lines 432–439, §X3 lines 110–125 |
| T06 | Process assertions → assert_cmd | todo | | §R4 lines 186–196; keep the async process seam for mock-backed tests |
| T07 | Structured JSON log capture | todo | | §R8 lines 244–257, §LEAF-10 lines 530–534; adds a surface to shared girpr-log |
| T08 | INI parser/merger → rust-ini | todo | | §APP-1 lines 262–279; byte-exact pins need adjustment, policy layer stays |
| T09 | URL assembly & percent-encoding | todo | | §X2 lines 90–109, §APP-2 281–294, §APP-5 321–332, §CAP-3 689–700, §CAP-4 702–710; records the `game_ids[]` decision below |
| T10 | Small app wire helpers → serde-aux/quick-xml/base64/hex | todo | | §APP-3 296–306, §APP-4 308–319, §APP-9 371–378, §APP-10 380–389 |
| T11 | FNV-1a hashing → fnv | todo | | §APP-7 lines 347–357, §CAP-5 lines 712–723, §X4 lines 127–134 |
| T12 | Protobuf → prost decode + prost-build codegen | todo | | §APP-8 lines 359–369, §LEAF-4 lines 451–465; build-tooling and dump-key pins |
| T13 | Junction creation → junction crate | todo | | §APP-6 lines 334–345, §LEAF-3 lines 441–449; Windows only |
| T14 | Artifact-server fixtures → wiremock | todo | | §APP-11 lines 391–402, §X1 lines 56–89; large test blast radius |
| T15 | ScriptedServer → wiremock | todo | | §LEAF-1 lines 415–430, §X1 lines 56–89; large test blast radius |
| T16 | strum enum tables | todo | | §LEAF-7 lines 496–505, §SYNC-2 lines 551–561; shared crates, keep aliases/padding |
| T17 | Retry sleep hook → async-trait | todo | | §LEAF-6 lines 481–494; keep backoff as the schedule engine |
| T18 | tempfile scratch/case-tmp helpers | todo | | §LEAF-9 lines 520–528, §SYNC-4 lines 575–587; deterministic suffixes are not a contract |
| T19 | path-shape normalization → relative-path | todo | | §LEAF-8 lines 507–518; low confidence — stop if first-violation order cannot be kept |
| T20 | Filter glob matching → globset | todo | | §LEAF-5 lines 467–479; shared crate, keep separator/backslash policy |
| T21 | bitflags AlgoSet | todo | | §SYNC-1 lines 540–549; preserve two-byte layout, iteration order and Debug spelling |
| T22 | filetime mtime conversion | todo | | §SYNC-3 lines 563–573; keep `mtime_ns_of` as the single shim |
| T23 | path-clean link targets | todo | | §SYNC-5 lines 589–599; lexical normalization, keep escape refusal |
| T24 | cache-json NDJSON stream + direct hex | todo | | §SYNC-6 lines 601–612, §SYNC-7 lines 614–622; keep meta-first/duplicate rules and error wording where pinned |
| T25 | criterion benchmark harness | todo | | §SYNC-8 lines 624–634; dev tooling only |
| T26 | ignore walker for scan/filter | todo | | §SYNC-9 lines 636–649; low confidence — adopt only after a behavior diff |
| T27 | Capture mock server → axum | todo | | §CAP-1 lines 655–672, §X1 lines 56–89; large test blast radius; raw helper stays |
| T28 | Ordered JSON fixtures → serde_json | todo | | §CAP-2 lines 674–687; no `preserve_order` (§A7 lines 33–34) |
| T29 | Bounded ordered join → futures | todo | | §CAP-6 lines 725–736; keep panic/error vocabulary and retry loop |
| T30 | TSV writers → csv | todo | | §CAP-7 lines 738–750; keep sort/tie-break and LF-only newlines |
| T31 | Capture misc: markdown/semver/slug/differ | todo | | §CAP-8 752–761, §CAP-9 763–772, §CAP-10 774–781, §CAP-11 783–791; cosmetic/optional |
| T32 | Duplicated BOM stripping → unicode-bom | todo | | §APP-12 lines 404–409; optional, low payoff — skip if churn outweighs benefit |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| — | (none seeded) | — | Reviews are added as batches reach a checkpoint |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. Reviewers decide which are worth doing and may prune them to avoid run-away.
A follow-up planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| — | (none seeded) | — | Follow-ups are recorded after review sessions |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code records that in the
relevant task's `note` instead of silently proceeding):

- **The report is a static study of one tree (§A1, lines 15–16).** Every item is a
  finding to validate; line numbers and library versions may have drifted. Re-check
  versions/features at implementation time (§A2, lines 17–18). Trust the code.
- **Behavior changes are allowed, but intended behavior must be tested (§A4 lines 22–23,
  §A5 lines 26–28).** Output spelling, error text and golden bytes may change; regenerate
  rather than delete semantic assertions. Tests that only pin a hand-rolled
  implementation may go; tests of the function must be adapted.
- **Never enable `serde_json/preserve_order` (§A7, lines 33–34).** Ordered output is
  produced by a local `Serialize`/`Vec<(String, V)>` representation, not a global feature.
- **Project-local `girpr-*` leaves are deliberate (§A8, lines 35–36).** They hold policy,
  not boilerplate; the non-findings at report lines 795–855 are not tasks.
- **Library guarantees, not library quirks, are the contract (§A12, lines 48–50).** Do not
  pin internal formatting, header casing or iteration details beyond the documented API.
- **Cross-cutting themes are distributed, not standalone (§X1–X4, lines 56–134).** X1 →
  T04/T14/T15/T27; X2 → T09; X3 → T05/T07; X4 → T11/T16/T21.
- **`game_ids[]` spelling (§X2 lines 90–109, §APP-2 lines 281–294):** decide once in T09
  (literal brackets vs `%5B%5D`) and record the decision here before any other URL task
  (T27/T28 or follow-ups) is dispatched. Every URL task follows that pointer.
- **Pinned contracts that must survive unless a task says otherwise:** the two-byte
  `AlgoSet` layout, canonical hash order and Debug spelling (§SYNC-1 lines 540–549); the
  frozen protobuf dump keys `chuncks`/`unknown_field6` (§LEAF-4 lines 451–465); the
  fixed first-violation order in path-shape (§LEAF-8 lines 507–518); the postcard digest
  vocabulary (non-finding, lines 831–833).
- **Out of scope even though they resemble findings:** the deliberately malformed raw
  socket helper (§X1 line 84, §CAP-1 line 672), `File::lock`/`try_lock` (§A9 lines 40–41),
  `backoff`-based retry drivers (§LEAF-6 line 493, §CAP-6 lines 735–736), and every item
  in "Areas examined without a credible candidate" (lines 795–855).
- **No lingering references at cleanup.** No source, test, comment, manifest, doc table
  or commit message may reference `REFACTOR_PLAN/` or
  `docs/08-third-party-delegation-findings.md`.
