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
| T09 | Lift value types + `girpr-repair-plan` crate | completed | c898b105, 75f5acd3, 92cab060 | F12+F16; **lifted values (for later tasks):** in \`girpr_repair_plan\` — \`RepairPlan\`/\`PlannedFile\`/\`Layout\`/\`FileId\`/\`build_plan\`/\`validate_chunk_ranges\`/\`est_file_download_bytes\` (plan.rs), \`PurgePlan\`/\`PurgeCandidate\`/\`PurgeKeys\` (purge_values.rs), \`PathScope\`/\`compile_patterns\` (path_scope.rs), \`LocalChunkMap\`/\`DownloadEstimate\`/\`PlanSummaryValues\`/\`check_scope_selection\`/\`ScopeSelectionError\` (lib.rs); \`Mode\` stays in \`girpr-repair-policy\`; \`ScanReport\` stays in \`src/repair/scan.rs\`; \`PlanInputs\`/\`PlanWork\` kept local (name facade types). **For T10:** build on \`girpr_repair_plan::PathScope\`, do not re-lift it. Docs-drift follow-up suggested (historical docs name old paths/test names) — needs reviewer endorsement |
| T10 | `girpr-path-scope-semantics` leaf | skipped | | F13 validated skip (pre-satisfied by T09): \`PathScope::is_visible\`/`is_active`/`is_open` already in acyclic leaf \`girpr-repair-plan\` and all product callers use it; a new leaf is a §A2 facade or a forbidden re-lift. girsync's opposite mechanic (hide from map but still stat blockers via \`SystemProbes\`) stays local (§A9/§A13). Report's \`file/mod.rs:514-518\` pointer is stale |
| T11 | `girpr-byte-accounting` leaf | skipped | | F14 validated skip (pre-satisfied): "Projected" is already \`girpr-repair-plan::DownloadEstimate\` + \`girpr-chunk-cost\` (both programs); Live/Settled/FirstStop + reservation arithmetic already in \`girpr-report\`'s \`ledger.rs\` (\`Meter\`, \`StoppedBy\`). A new crate would be a single-consumer re-exporting facade with a \`StoppedBy::count(&Summary)\` back-edge (§A2/§A12); gircap's remnants are the excluded \`>\` predicate/counters. Do not reopen |
| T12 | `girpr-decision-record` transport-free primitive | skipped | | F17 validated skip (premise stale on gircap side): product side is a test-only thread-local observation seam (§A10); gircap has no record/drain transport — pure functions returning \`Vec<Assertion>\` feed byte-published \`verify/selfcheck.json\` (§A5, N9). No shared core; do not re-propose extracting from a non-existent common mechanism |
| T13 | `girpr-install-provenance` value crate | skipped | | F18 validated skip (premise stale): product \`ReportProvenance\` is a single-consumer borrowed view whose field order is the report's contract; gircap's provenance is HTTP request/manifest identity with a different field set. No shared value shape; a format-free crate would be a §A2 no-op. Report's gircap pointers are stale. Implementer suggests revisiting only if a second real consumer appears — needs reviewer endorsement |
| T14 | `girpr-path-shape` validator leaf | completed | cc56fda6, c7bab952, d7989557 | F19; leaf `girpr-path-shape` with \`ShapePolicy\`/\`Normalization::{Strict,Normalizing}\`/\`ShapeProblem\`; rule IDs+messages stay caller-local; product Normalizing and gircap Strict policies intact, rejection JSON/rule-ID bytes unchanged; **gircap leaf inventory now fifteen**; suggested follow-up: replace moving ordinal in gircap manifest comment |
| T15 | `girpr-refusal-ledger` assessment | skipped | | F20 validated skip: \`RejectionsDocument\`/\`rejections.json\` is a gircap-only published observation document with no second consumer (§A4 forbids a shared observation format) and its \`omitted\`/sampling fields are byte-visible and round-trip-compared (§A5); T14's rule IDs stay caller-local |
| T16 | `girpr-request-url` leaf | skipped | | F21 validated skip (pre-satisfied/keeper): shared values already in \`girpr-model::region\`; remaining URL-join/escaping strategies are a recorded divergence (§A9/§N8) and program-local override seams (\`--hyp-base\`/\`--sophon-base\`, gircap \`Box::leak\`). Do not re-dispatch §F21; a shared builder would change pinned URL bytes (§A5) or collapse the keeper |
| T17 | Deepen `girpr-document-envelope` from report-core | skipped | | F22 validated skip (premise stale): \`girpr-report-core\` serves only the stdout NDJSON plane (A6), not the five durable documents, which use the local \`emit::write_doc\` helpers; no common envelope shape (\`tool\` only on MANIFEST.json, key order is load-bearing serde order, N11 says no duplication). Deepening would absorb the published format (A4/A5/N9). Do not re-dispatch |
| T18 | Publish-target predicate into fs-names leaf | skipped | | F23 validated skip (pre-satisfied): \`girpr-fs-names\` already owns \`has_trailing_dot_or_space\` and \`is_reserved_device_name\`; T14's \`girpr-path-shape::validate\` composes them; both writers consume the leaves directly with atomicity intact (capture non-atomic docs + one blob rename, util-path \`NamedTempFile::persist\`). Do not re-dispatch F23 |
| T19 | `girpr-value-ranges` leaf | completed | 92f7964305273f41754b974caab8791ccfbf992f | F24; dependency-free leaf with \`check_in_range\`/\`RangeFact\`/\`RangeBound\`, no messages/exit codes/constants; all three callers migrated directly with error strings and exit classes unchanged; **gircap leaf inventory now sixteen**; suggested follow-up: girsync \`backup_policy\` fourth NonZeroUsize check not migrated |
| T20 | Shared blob→bytes→digest verify walker | skipped | | F25 validated skip: premise stale (product side is \`check_preflight\`, not a walker; product verify already in \`girpr-chunk-verify\`); \`girpr-digest\`/`gircap-hash` already own the mechanics; a memoized walker spans incompatible digest domains (A8), is not a logic-free leaf gircap may use (A4), publishes byte-visible recorded/re-derived counts (A5) and risks tautological verification (A13). Do not reopen |
| T21 | `girpr-http-transport` assessment | skipped | | F26 validated skip (premise stale): no transport abstraction in \`girpr-fetch\` (uses \`reqwest::Client\` directly); shared retry mechanics already in \`girpr-retry\` (\`Schedule\`/\`Sleep\`, jitter as caller parameter). A leaf would unify reqwest features (json/stream vs bare) (§A2), put a transport dep into gircap reversing \`gircap-http\`'s reason (§A4/N10). Do not re-propose F26/F27 shared retry drivers |
| T22 | Shared retry decision vocabulary / `drive_attempts` | skipped | | F27 validated skip: shared vocabulary is a §A2 thin enum (girpr already has local \`Retry<T>\`, gircap an inline bool); \`drive_attempts\` would lose gircap's per-attempt logging, body retention budget and exhaustion text (§A5). Keep classifiers/jitter local. Only a third loss-free driver consumer would justify revisiting |
| T23 | Generic verified KV cache assessment | skipped | | F28 validated skip: one specialized redb schema; product deliberately has no manifest cache. A generic row trait would hide the literal-byte pin / \`FMT_V2\`/\`CACHE_VERSION\`/closed \`kind\` registry (§A5/N15), tempt the forbidden cache (§A13/N5), and be a single-implementer facade blocked by redb's sealed handle API. No reopening warranted |
| T24 | Generic engine `Plan`/`Execute`/`Verify` trait shape | skipped | | F29 validated skip: no engines trait exists; an order/executor/transport-free \`Plan\`/\`Execute\` trait is an unconsumed §A2 dead facade, adoption would regress girsync's named-phase invariant and A6/A7/A8/A12 boundaries, and a generic \`Verify\` cannot enforce A13's two-independent-observations without collapsing to a tautology. Reopen only for a genuinely new fourth consumer |
| T25 | `girpr-path-identity` crate | skipped | | F30 validated skip (pre-satisfied): \`girpr-fs-names::fold_ascii_case(value, fold_case)\` already Parameterized with no default; both \`path_key\` rules (girsync \`true\`, product \`cfg!(windows)\`) consume it directly; the three identity questions are already named at their real callers, two girsync-only. Do not re-dispatch F30 |
| T26 | Generic `EvidencePolicy` with `CannotTell` | skipped | | F31 validated skip: four sites share only a conceptual fail-closed shape, no duplicated code/data type; a generic \`EvidencePolicy\`/\`CannotTell\` is a content-free §A2 facade with all decision logic left in callers. \`Verdict::Unverifiable\`/\`CHANGED\` bytes and cache fail-open-vs-closed keeper (§A9) stay local. Documentation-only principle note suggested |
| T27 | Split sync-test-support fixtures into generic runner | skipped | | F32 validated skip: \`sync-test-support → girsync\` normal edge is a deliberate keeper (F2), the runner has one consumer, and a generic shape would be the near-forbidden F29/F41 engine/harness trait while absorbing T30 fault seams and T32 memory backend (out of scope). Env pinning (\`GIRSYNC_TEST_CACHE\`) and nextest isolation untouched. Revisit only for a genuine second in-process CLI product |
| T28 | Generic `AtomicPublish` leaf | skipped | | F33 validated skip: bytes-publish family already \`girpr-util-path::atomic\` (directly consumed); sync-cache's link-into-place + hold-record/bound/one-syscall-gap protocol and sled2redb's deliberately-unreclaimed namespace cannot be expressed by durable/noclobber parameters without collapsing keepers (§A5/A9/A13); \`_tmp\` resume excluded (N2). Do not re-propose F33 |
| T29 | Generalized report plane | skipped | | F34 validated skip (pre-satisfied): \`girpr-report-core\` already parameterizes order/field vocabulary/renderer and is consumed directly by all three planes; further extraction would pull clap into a shared crate (forbidden), flip girsync's pinned alphabetical NDJSON bytes (§A5) or be a §A2 facade. Remaining field-model work is T45/F50 only. Stale premise: code uses \`why=\`, not \`because=\` |
| T30 | Generic injected-fault seam | skipped | | F35 validated skip (pre-satisfied/keeper): typed \`TestHooks\` already a de-embedded local seam with one consumer; generic names would disconnect tests from production (task's HIGH-RISK blocker) while keeping domain names leaves a §A2 facade. Field names and env spellings (\`GIRSYNC_TEST_*\`, nextest-pinned) untouched. Do not force a shared fault crate |
| T31 | Shared plan-action record crate | skipped | | F36 validated skip: the only shared mechanic is \`girpr-report-core\`'s ordered-JSON/line writer, already consumed directly by both planes; a value-only record type would be a §A2 facade (no overlapping field set) or force one format onto the other (§A5/A6). Stale pointers: code uses \`why=\` (not \`because=\`), report module is \`tools/sync/src/report/mod.rs\` |
| T32 | Pluggable KV backend + in-memory mirror | skipped | | F37 validated skip: restatement of closed F28/T23 - one KV implementation (redb file vs in-memory), sealed redb handle forces a local trait, single-implementer §A2 facade otherwise. Registry already path-keyed/per-process/forgettable (\`OnceLock\`); generalizing key/lifetime is the cross-test leakage to stop (§A10). Stale premise: row capture is thread-local, not process-global. Do not re-dispatch |
| T33 | Generic batched durable writer | skipped | | F38 validated skip: only writer is \`girpr-sync-cache::CacheWrite\` (single consumer; §A2 facade); a generic shape must expose \`GIRSYNC_TEST_*\` durability knobs (§A11) or leave all policy local, while pre-drop commit ordering is a call-site crash-safety invariant (§A13/N15). Revisit only if a second batched writer appears |
| T34 | Generalized `RestartableSink` with blanket impls | skipped | | F39 validated skip: only acyclic common home is \`girpr-digest\`, which deliberately hosts \`BodyFile\` to break a cycle; unifying couples fetch's zstd path to gircap's HTTP body seam. \`Send\` bounds and \`restart()\` responsibilities (truncate+re-digest vs seek-only vs clear) diverge by contract (§A5/A12). No safe narrower boundary |
| T35 | Shared capture-buffer primitive | completed | ace02d2a | F40; \`girpr_log::test_support::CaptureBuffer\` (storage + Write + snapshot only); \`logs_of\` strict UTF-8 and sync \`SharedBuffer\` lossy text + MakeWriter stay local; no new crate/leaf so no gircap inventory or A14 seam change; suggested follow-up: third test-local copy in girsync \`apply/phases.rs\` \`capture_log\` not migrated |
| T36 | Generic parse→plan→dispatch command harness | skipped | | F41 validated skip: each harness names program-private construction (gircap independence §A4 blocker), three incompatible typed outcomes, and a common surface would be a §A2 facade breaking \`GIRSYNC_TEST_CACHE\`/nextest isolation (§A10/F2/F32). T27 split/T30 fault seams not absorbed |
| T37 | Shared precondition/skip policy | skipped | | F42 validated skip: only a thin assert/eprintln shell is shared; wording, required-case switch, Injected/Host distinction and macro-vs-function control flow ARE the policy and must stay local (§A5/A10). A shared core is a parameter-bag facade (§A2) with cycle/placement cost (§A12). Wording and nextest filters untouched |
| T38 | Scratch/fixture support (merge vs split assessment) | skipped | | F43 validated skip (both directions): merge reintroduces girsync rebuild cascade into girpr/gircap test builds and breaks gircap independence (§F2/§A4/A11/N6); split adds three dev-dep edges/shims/seams for concerns consumed only by root girpr test binaries, multiplying rebuild domains without removing edges (§A2). Note: fixtures now module-split; revalidate against individual files if ever reopened |
| T39 | Umbrella `test-support` facade assessment | skipped | | F44 validated skip (measured, A11): an umbrella would force all consumers onto \`girpr-log/test-support-async\` (tokio rt) and \`girpr-report/clap\` plus tokio \`test-util\`, collapsing the deliberate log feature split (§A2). Two \`girpr-log\` variants (tokio-free / tokio-bearing) exist today and must stay; \`girpr-test-support\` is run glue, not the pre-satisfied umbrella |
| T40 | Time/stamp leaf with named formats | completed | d445fee0c2e69c5db735bfa25e6bf84f2b8b0cb1, c7322ec6 | F45; new acyclic \`girpr-time\` leaf owns four distinct named formats (compact, RFC3339-millis-Z, RFC3339-Secs-Z found in \`manifest.captured_at\`, RFC3339/lock body); removed \`girpr-log\` wrappers to avoid a facade; **gircap leaf inventory now seventeen**; all format bytes unchanged |
| T41 | Shared line-sink primitive | skipped | | F46 validated skip: emission is intentionally divergent (report-core propagates errors/ordered plane vs log/gircap/product ignore errors, different writer abstractions and prefixes); a shared sink collapses a §A9 keeper or is a thin §A2 facade over \`writeln!\`, and moving \`write_line\` reduces report-core cohesion. Line emission stays divergent across planes |
| T42 | Shared IO read-buffer policy | skipped | | F47 validated skip (premise incomplete): at least three independent \`512 * 1024\` call-site constants (util-hash, sync-cache, \`src/repair/file/mod.rs\`), not one policy; \`girpr-digest::hash_reader\` deliberately takes a caller-owned buffer. A shared constant pollutes a mechanics-only leaf (§A2) or needs a new low-value crate with full A14 seams; independent tuning is a keeper (§A9) |
| T43 | `girpr-primitives` consolidation assessment | skipped | | F48 validated skip: merged leaves have disjoint consumer sets, so any single-part edit rebuilds a strict superset (never fewer variants, §A2); folding loopback-mock into cli pulls clap into every consumer and exit+audio-language couples exit-code vs artifact-byte contracts. No co-change coupling in history |
| T44 | Narrow loopback hit-count/assertion extension | skipped | | F49 validated skip (premise stale/pre-satisfied): \`girpr-loopback-mock\` already owns \`hits()\`/\`requests()\`/\`start_expecting()\` drop-time count assertion; the named "repeated layer" is actually divergent domain fake servers (N6/N16 keepers) and glue crates with no HTTP server. Leaf stays dependency-free; do not merge domain servers |
| T45 | `girpr-report-model` leaf | in_progress | | F50; bool vs 1/0, percent-encoding, pinned key order (A5/A6) |
| T46 | Narrow CLI leaf extension (OutputFormat + LogArgs) | todo | | F51; pinned help/exit codes, `--version` presence differs |
| T47 | `bounded_decode` in girpr-digest assessment | todo | | F52; streaming always-on test, F52 likely re-confirms N7 — likely skip |
| T48 | `girpr-fs` composition leaf | todo | | F53; reparse classification is domain logic, one deliberate unsafe call |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | done | T01 completed (no fixes needed); T02 skip endorsed |
| B | T03, T04, T05 | done | T03 completed (no fixes); T04 skip + follow-up endorsed; T05 note confirmed accurate |
| C | T06 | done | No reviewable work — T06 validated skip; follow-up endorsed at checkpoint D |
| D | T07, T08 | done | Both completed (no fixes); T06 follow-up endorsed; gircap seams correctly untouched (leaves are girpr-only) |
| E | T09, T10 | done | T09 completed (no fixes); T10 skip endorsed; report-only drift suggestion rejected (report deleted at cleanup) |
| F | T11, T12, T13 | done | No reviewable work — all three validated skips (T11/T12 pre-satisfied or stale premise, T13 near-forbidden) |
| G | T14, T15, T16 | done | T14 completed (no fixes); T15/T16 skips confirmed; three follow-ups endorsed |
| H | T17, T18, T19 | done | T19 completed (no fixes); T17/T18 skips confirmed; ordinal drift covered by FU03 |
| I | T20, T21, T22 | done | No reviewable work - all three validated skips; F26/F27 shared retry drivers should not be re-dispatched |
| J | T23, T24, T25 | done | No reviewable work - all three validated skips (forbidden cache, dead trait, pre-satisfied fold) |
| K | T26, T27, T28 | done | No reviewable work - all three validated skips (content-free policy facade, single-consumer test runner, incompatible publish contracts) |
| L | T29, T30, T31 | done | No reviewable work - all three validated skips (pre-satisfied report core, local typed fault seam, no shared plan-action value) |
| M | T32, T33, T34 | done | No reviewable work - all three validated skips (single-implementer KV facade, single-consumer batched writer, divergent sink contracts) |
| N | T35, T36, T37 | done | T35 completed (no fixes); T36/T37 skips confirmed |
| O | T38, T39, T40 | done | T40 completed with corrective commit c7322ec6 (stale gircap spec dependency descriptions); T38/T39 skips confirmed |
| P | T41, T42, T43 | done | No reviewable work - all three validated skips (divergent line emission, per-call-site buffer tuning, rebuild-domain trap) |
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
| FU03 | Replace the moving ordinal in gircap's manifest comment (e.g. "not a fifteenth shared leaf") with a non-numeric phrase (from review B, recurred at T14) | open | Non-blocking cosmetic; the ordinal has drifted repeatedly — a non-numeric phrase removes recurring doc churn |
| FU04 | Record §F10 as structural resemblance only; a shared DTO leaf would add edges without removing any (§A2) plus gircap doc-table cost (§A4), so keep URL-join and both scalar adapters local (from T06) | open | Reviewer-endorsed; endorsed at checkpoint D |
| FU05 | Update the stale comment in `src/repair/file/inplace.rs:92` that still names `scan::scan_ranges` (now in `girpr-chunk-verify`) (from review D) | open | Non-blocking cosmetic; no behaviour impact |
| FU06 | Docs-consistency pass for surviving docs naming old paths/test FQNs (`docs/03-repair-rust-implementation-plan.md`, `docs/04-scope-limitation-spec.md`, `docs/07-panic-audit.md`) (from T09) | open | Reviewer-endorsed; report-based drift is moot (deleted at cleanup) |
| FU07 | Reword the task-tracking reference in `crates/repair-plan/src/lib.rs:8` to name `girpr-repair-policy` without the task ID, so shipped source carries no orchestration identifier (from review E) | open | Non-blocking; matters for final cleanup |
| FU08 | Revisit provenance extraction only if a second real consumer appears (from T13) | open | Reviewer-endorsed: a second consumer is §A2's payoff condition; with one consumer F18 stays near-forbidden (§A4) |
| FU09 | Record gircap URL construction as a deliberate keeper so §F21 is not re-dispatched; weigh any shared uppercase percent-encoder against §N8 and §A14 cost (from T16) | open | Reviewer-endorsed: F21's remainder is the recorded URL-join keeper (§A9/§N8) |
| FU10 | Consider the shared validator's whole-string colon check vs the old product per-component check: for a value violating both an earlier trailing-dot rule and a later colon rule, product error wording changed (from review G) | open | Low priority; caller-local, non-published text; no test regressed |
| FU11 | Optionally migrate girsync's fourth hand-written NonZeroUsize check in `backup_policy` (`--backup-retain`) to `girpr-value-ranges` (from T19) | open | Reviewer-endorsed, non-blocking; preserve its distinct wording/exit-3 class |
| FU12 | Migrate the third test-local capture-buffer copy in girsync `capture_log` (`tools/sync/src/commands/sync/apply/phases.rs`) to `CaptureBuffer` via a dev-dependency (from T35) | open | Reviewer-endorsed, non-blocking; keep its local subscriber/bytes |
| FU13 | Add a direct unit test in `crates/log` for `CaptureBuffer`'s Write/snapshot byte fidelity (from review N) | open | Non-blocking; currently covered only indirectly |
| FU14 | Add a brief comment at the `unwrap_or_else(into_inner)` call sites noting intentional parity with the sync consumer (from review N) | open | Non-blocking cosmetic |
| FU15 | Remove the now-unused direct `chrono = "0.4.45"` dependency from the root `girpr` package (from review O) | open | Non-blocking optional cleanup; pre-existing before T40, no source usage |

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
