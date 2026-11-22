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
| T01 | `girpr-chunk-identity` leaf (+ digest normalization placement) | completed | f831b5af, 950fb9cf, d9a49602, 401fb580 | F4+F8; leaf `girpr-chunk-identity` added, digest normalization stays at gircap decode boundary; borrowed view keeps spelling-exact Eq/Hash (case-insensitive via `matches`/`parse`); no byte/first-match/collision drift; gircap leaf inventory 12→13 |
| T02 | `girpr-chunk-layout` leaf | skipped | | F5 validated skip: the two layouts are different data models (i64 string-violations vs u64 tiled/gaps/overlaps), algorithms and verdicts, each with one consumer; unifying risks published `Range` bytes and collapses §A13 verdicts. Reviewer endorsed follow-up FU01 |
| T03 | `girpr-chunk-cost` download-projection fold | completed | cdc28f44, 5fb0508e, cbdfb574 | F6; leaf folds `(compressed_size, downloads)` entries (identity kept out to stay dependency-free), membership still caller-supplied; migrated no-scan + 3 scanned derivations + `scope/report.rs` `--plan-out` fallback + gircap; lower-bound framing and published figures unchanged; reviewer: keep `scan.rs`/capture test-side re-derivations independent, never replace with the shared fold (§A13) |
| T04 | `girpr-reuse-decision` leaf | skipped | | F7 validated skip: identity-only mode already owned by `girpr-chunk-identity` (T01, task forbids reabsorbing it); order-survival mode has one consumer and incomparable input domain, so a both-mode leaf is a single-consumer mode-dispatch facade (§A2) coupling gircap to girpr repair policy (§A4). Reviewer endorsed follow-up FU02 |
| T05 | `girpr-manifest-stats` leaf | skipped | | F9 validated skip (premise stale): `verify()` in `tools/capture/src/manifest/mod.rs` turns reconciliation mismatches into a fatal exit-3 assertion in the production fetch path, so "recorded, never fatal" is contradicted; product does no declared-stats reconciliation, leaving only a trivial inequality (thin leaf, §A2). Do not retry on the old reading; sums sit upstream in the decoder and touching them risks the §A5 string wire form and §N9 wording |
| T06 | `girpr-api-dto` leaf (revisit skipped T18) | skipped | | F10 validated skip: the two isolated decoders diverge in envelope requiredness, field sets, scalar adapter (coercing vs wire-preserving), traits and error semantics; only trivial shapes are identical, so a shared hierarchy would change accepted bodies/bytes and two parallel hierarchies are a §A2 facade. Reviewer endorsed follow-up FU04 |
| T07 | Lift `Mode`/`resolve_mode`/`RepairMode` to a policy leaf | completed | 267c9d17, 692e8350 | F15; **for T08:** \`Mode\`/\`resolve_mode\` now live in acyclic leaf \`girpr-repair-policy\` (\`crates/repair-policy/\`), with \`resolve_mode\` (probe) and \`resolve_mode_from_target\` (pure, for the \`DiskPlan::from_figures\` seam); resolve rule \`Temp → temp; non-file → temp; else in-place\`; \`RepairMode\` removed from the facade (incl. \`test_support\` re-export); remaining file→scan edge to move in T08 |
| T08 | `girpr-chunk-verify` leaf | completed | 767babe35fdd8884193acfc3fa93acfb94caa7ac, d474b787669642405639d164493b209024905c3b | F11+F16; leaf `girpr-chunk-verify` owns `Verdict`, `scan_ranges`, `source_ok_by_chunk`/`range`, `verify_in_place`; `file ↔ scan` cycle gone; `Verdict::modified`/coarse-MTIME proof and fresh-read verification intact; no nextest filter moves. **For T09:** `ScanReport`/`reusable_verdict` deliberately left in `scan.rs`/`file` because they need `RepairPlan`/`FileId`/`LocalChunkMap`/`reuse`; the residual `file → scan::ScanReport` edge is a benign one-way producer→consumer dependency, revisit only if T09 lifts those values |
| T09 | Lift value types + `girpr-repair-plan` crate | in_progress | | F12+F16; ledger-cycle style value move first |
| T10 | `girpr-path-scope-semantics` leaf | todo | | F13; "filtered path is nonexistent" two-sided rule; girsync blocker stat |
| T11 | `girpr-byte-accounting` leaf | todo | | F14; Projected/Live/Settled/FirstStop; not `>` predicate or counters |
| T12 | `girpr-decision-record` transport-free primitive | todo | | F17; low confidence, published artifact vs thread-local — likely skip |
| T13 | `girpr-install-provenance` value crate | todo | | F18; near-forbidden if it emits keys/fixes order — likely skip |
| T14 | `girpr-path-shape` validator leaf | todo | | F19; normalize-vs-reject policies and stable rule IDs are semantic |
| T15 | `girpr-refusal-ledger` assessment | todo | | F20; A4-forbidden shared observation format — likely validated skip |
| T16 | `girpr-request-url` leaf | todo | | F21; query order/escaping and `--hyp-base`/`--sophon-base` seam |
| T17 | Deepen `girpr-document-envelope` from report-core | todo | | F22; A4/A5 key sets; selfcheck wording table stays local (N9) |
| T18 | Publish-target predicate into fs-names leaf | todo | | F23; atomicity policy stays local; different call consequences |
| T19 | `girpr-value-ranges` leaf | todo | | F24; program-specific error strings and exit codes stay local |
| T20 | Shared blob→bytes→digest verify walker | todo | | F25; re-derive against chunks/index.json; findings not failures (A13) |
| T21 | `girpr-http-transport` assessment | todo | | F26; reverses gircap-http rationale, couples reqwest — likely skip |
| T22 | Shared retry decision vocabulary / `drive_attempts` | todo | | F27; jitter keepers (A9), published status set |
| T23 | Generic verified KV cache assessment | todo | | F28; forbidden product cache + literal-byte pin — likely skip |
| T24 | Generic engine `Plan`/`Execute`/`Verify` trait shape | todo | | F29; crosses exit/report/digest/SCC contracts — trait shape at most |
| T25 | `girpr-path-identity` crate | todo | | F30; three named questions, two fold rules; canonicalization split |
| T26 | Generic `EvidencePolicy` with `CannotTell` | todo | | F31; collapsing CannotTell into equality is the false-clean bug |
| T27 | Split sync-test-support fixtures into generic runner | todo | | F32; env pinning + nextest isolation must survive (A10, F2) |
| T28 | Generic `AtomicPublish` leaf | todo | | F33; one-syscall gaps + hold records; `_tmp` resume excluded (N2) |
| T29 | Generalized report plane | todo | | F34; pulls clap, pinned NDJSON byte order (A5) |
| T30 | Generic injected-fault seam | todo | | F35; env spellings pinned by nextest env (A10) |
| T31 | Shared plan-action record crate | todo | | F36; NDJSON stream vs JSON document; exit/summary parsing |
| T32 | Pluggable KV backend + in-memory mirror | todo | | F37; path-keyed, per-process, forgettable; cross-test leakage risk |
| T33 | Generic batched durable writer | todo | | F38; test knobs + crash-safety invariants; medium-low value |
| T34 | Generalized `RestartableSink` with blanket impls | todo | | F39; Send bound + `restart()` meaning differ between traits |
| T35 | Shared capture-buffer primitive | todo | | F40; strict-vs-lossy UTF-8 and MakeWriter must not alter lines |
| T36 | Generic parse→plan→dispatch command harness | todo | | F41; typed outcomes differ, contradicts A4 — likely skip |
| T37 | Shared precondition/skip policy | todo | | F42; pinned `SKIP precondition:` wording and nextest overrides |
| T38 | Scratch/fixture support (merge vs split assessment) | todo | | F43; both directions high-risk — likely validated skip |
| T39 | Umbrella `test-support` facade assessment | todo | | F44; feature-unification trap (A11) — likely skip |
| T40 | Time/stamp leaf with named formats | todo | | F45; three on-disk/artifact byte strings |
| T41 | Shared line-sink primitive | todo | | F46; must not pull serde into every log consumer (A6) |
| T42 | Shared IO read-buffer policy | todo | | F47; caller-owned-buffer seam + small-buffer equivalence test |
| T43 | `girpr-primitives` consolidation assessment | todo | | F48; rebuild domain is the value — likely skip |
| T44 | Narrow loopback hit-count/assertion extension | todo | | F49; domain servers stay independent (N6) |
| T45 | `girpr-report-model` leaf | todo | | F50; bool vs 1/0, percent-encoding, pinned key order (A5/A6) |
| T46 | Narrow CLI leaf extension (OutputFormat + LogArgs) | todo | | F51; pinned help/exit codes, `--version` presence differs |
| T47 | `bounded_decode` in girpr-digest assessment | todo | | F52; streaming always-on test, F52 likely re-confirms N7 — likely skip |
| T48 | `girpr-fs` composition leaf | todo | | F53; reparse classification is domain logic, one deliberate unsafe call |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | done | T01 completed (no fixes needed); T02 skip endorsed |
| B | T03, T04, T05 | done | T03 completed (no fixes); T04 skip + follow-up endorsed; T05 note confirmed accurate |
| C | T06 | pending | |
| D | T07, T08 | done | Both completed (no fixes); T06 follow-up endorsed; gircap seams correctly untouched (leaves are girpr-only) |
| E | T09, T10 | pending | |
| F | T11, T12, T13 | pending | |
| G | T14, T15, T16 | pending | |
| H | T17, T18, T19 | pending | |
| I | T20, T21, T22 | pending | |
| J | T23, T24, T25 | pending | |
| K | T26, T27, T28 | pending | |
| L | T29, T30, T31 | pending | |
| M | T32, T33, T34 | pending | |
| N | T35, T36, T37 | pending | |
| O | T38, T39, T40 | pending | |
| P | T41, T42, T43 | pending | |
| Q | T44, T45, T46 | pending | |
| R | T47, T48 | pending | |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. Reviewers decide which are worth doing and may prune them to avoid run-away.
A planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU01 | Revisit extracting gircap's published `Range` into a logic-free leaf only if a second consumer appears (from T02) | open | Reviewer-endorsed: a second consumer is the §A2 condition that makes the leaf pay off; skip holds until then |
| FU02 | Close §F7 as satisfied-by-T01 for the identity half and not-applicable for the order half; revisit a standalone order fold only if a second consumer appears (from T04) | open | Reviewer-endorsed: the second-consumer trigger is §A2's payoff condition; do not reopen T04 |
| FU03 | Reword the capture manifest comment's "not a fourteenth shared leaf" ordinal now that a real fourteenth leaf (`girpr-chunk-cost`) exists (from review B) | open | Non-blocking cosmetic suggestion; not implemented |
| FU04 | Record §F10 as structural resemblance only; a shared DTO leaf would add edges without removing any (§A2) plus gircap doc-table cost (§A4), so keep URL-join and both scalar adapters local (from T06) | open | Reviewer-endorsed; endorsed at checkpoint D |
| FU05 | Update the stale comment in `src/repair/file/inplace.rs:92` that still names `scan::scan_ranges` (now in `girpr-chunk-verify`) (from review D) | open | Non-blocking cosmetic; no behaviour impact |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code records that in the
relevant task's `note` instead of silently proceeding):

- **Reports are static claims about one tree (intro lines 3–18; §A1, lines 22–28).** Every
  finding is a hypothesis to validate against the current code; line numbers may have
  drifted; trust the code. No wall-clock number from prior reports is a current budget.
- **Rebuild granularity and facade rule (§A2, lines 29–36).** Editing a file invalidates its
  target and downstream dependents. A split pays off only when high-level consumers depend
  on the narrow crate directly; a re-exporting facade is not a win.
- **Two deliberate crate conventions (§A3, lines 37–47).** Shared code is `girpr-<policy>`
  under `crates/` with product lints; `gircap-*` deviates under `tools/capture/` with local
  tools-only lints. Promoted code must satisfy the stricter product policy.
- **gircap independence (§A4, lines 48–56).** No `girpr` dep, no binary invocation, owns its
  codes/format/flags/fixtures; only logic-free leaves; synchronized doc-table updates for
  any new leaf; the bridge must not depend on gircap.
- **Byte contracts are load-bearing (§A5, lines 57–67).** Bundle, dump and artifact bytes,
  schema version 3 / refusal of v2, published protobuf field names, cache wire format and
  `SUMMARY` key/flag conditions must not drift even when tests pass.
- **Report planes cannot be merged (§A6, lines 68–75).** Only mechanics are shared; girpr
  event-first `1`/`0`, girsync alphabetical, gircap event-first with real `event` and
  percent-encoded values.
- **Exit codes are separate contracts (§A7, lines 76–80).** Only enum/pin/clap-mapping
  patterns are shared.
- **No single digest semantics (§A8, lines 81–87).** girpr parsed `Md5`, girsync raw bytes +
  foreign names, gircap lowercase hex; `DigestSet`/`AlgoSet`/`HashAlg` stay raw and
  allocation-free (N4).
- **Divergent policies are keepers (§A9, lines 88–97).** Retry jitter (0.5 vs 0.0), INI
  last-vs-first match, `path_key` fold, cache fail-open-vs-closed. A session that finds a
  keeper should change stops and raises a follow-up.
- **nextest shapes every test abstraction (§A10, lines 98–105).** One process per test;
  process-global fixtures help only within a test; moving a test changes its pinned filter
  in `.config/nextest.toml`.
- **Feature unification (§A11, lines 106–113).** Resolver 3 unifies dev-dependencies for a
  package under test; consolidating test surfaces can increase compiled variants.
- **Cycles constrain boundaries (§A12, lines 114–121).** A cut sits at an acyclic leaf or
  owns a whole SCC. Known cycles: effective↔planner, config↔repair, scope↔plan/limits,
  http↔biz, cli↔config, file↔scan. Value types must often be lifted first.
- **Verification is independent re-derivation (§A13, lines 122–128).** Two reads + failed
  flush, independent pass 2, hash gates and coarse-MTIME guard are correctness arguments,
  not caches. No generic engine may reuse an observation buffer.
- **New crate doc seams (§A14, lines 129–135).** Workspace, README table/prose, gircap §2
  table, tool manifest comments and nextest filters move together; stale inventories have
  needed follow-up commits before.
- **Prior exclusion list (§A15, lines 136–141) and negative findings §N1–N16 stay.** See
  `guidance/DO-NOT-TOUCH.md`. A finding may revisit a premise but never deletes a keeper.
- **Low-confidence / forbidden candidates default to a validated skip** (§F17, §F18, §F20,
  §F26, §F28, §F29, §F41, §F44, §F48, §F52 and `DO-NOT-TOUCH.md`). Implement only on hard
  evidence of a safe, worthwhile boundary.
