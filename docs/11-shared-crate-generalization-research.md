# Shared-crate generalization research: assumptions and findings

Static research over the current tree at `91482f86`. No source was changed, no build or
test was run, no line was executed, and no commit was made before this report. Every item
below is a claim to be validated during implementation; file/line references are from this
tree and may drift. The three earlier research reports (`docs/08-shared-crate-refactor-research.md`,
`docs/09-crate-split-turnaround-research.md`, `docs/10-test-optimization-research.md`) and
their planning packages are deleted from the tree but recoverable from git; they are treated
as existing facts and prior art, not as instructions repeated here. The goal of this round is
**further generalization with high-risk changes**, so the lens applied throughout is: which
remaining duplication would justify a new shared crate or a deeper abstraction, and what
would make that change high-risk.

Items are labelled `A` (assumptions), `F` (findings) and `N` (negative findings: duplicate-looking
code that is not an extraction candidate). A "candidate boundary" describes where duplication
sits and what a shared abstraction would have to own; it is a structural description, not an
implementation ordering and not a plan. No task list, sequencing recommendation or
implementation step is included.

## 1. Assumptions

**A1 — Scope and drift.** The workspace has grown from the 13 shared crates described in
`git show e76c69c9^:docs/08-shared-crate-refactor-research.md` (`A1`, `F30`) to **27 crates
under `crates/` plus three `gircap-` mechanics crates** (`Cargo.toml:41`). Prior verdicts that
were premised on co-location, a 13-crate shape, or an unresolved inversion are stale by
construction and are revisited below. No wall-clock number from prior reports was re-measured
here; those numbers were host-specific (4-core, link-bound) and are not current budgets.

**A2 — Cargo rebuilds at crate granularity, and a facade defeats a split.** Editing any file
in a package invalidates that target and every downstream dependent; upstream dependencies
stay cached. Splitting only pays off when high-level consumers depend on the narrow crates
directly — if a facade still re-exports both halves, the change adds a compile edge without
removing one. This is the rationale the current tree documents for consuming `girpr-fetch`,
`girpr-report`, `girpr-sync-cache` and `girpr-sync-leaves` directly (`README.md:452-491`;
`Cargo.toml:40-44`, `:153-160`). Sources: `git show e64b4f64^:docs/09-crate-split-turnaround-research.md`
`A1`-`A3`.

**A3 — Two crate conventions, both deliberate.** Shared code lives in `crates/<policy>`, is
named `girpr-<policy>`, carries a one-line `description`, opts into `[lints] workspace = true`,
and holds mechanics rather than program policy (`crates/digest/src/lib.rs:1-16`,
`crates/report-core/src/lib.rs:1-22`, `crates/retry/src/lib.rs:1-16`,
`crates/loopback-mock/src/lib.rs:1-17`). `gircap-http`, `gircap-hash` and `gircap-test-support`
deliberately deviate: `gircap-` prefix, under `tools/capture/`, local tools-only lint policy
(`tools/capture/docs/00-spec.md:46-57`, `tools/capture/Cargo.toml:67-84`). The product denies
`unsafe_code` and warns on `unwrap_used`/`expect_used`; both dev tools deny `unsafe_code` only
(`Cargo.toml:46-62`, `tools/sync/Cargo.toml:8-17`). Code promoted from a tool into a shared
leaf must satisfy the stricter product policy.

**A4 — gircap independence is non-negotiable.** gircap has no `girpr` dependency, never invokes
its binary, and owns its exit codes, report format, flag set (apart from the log flags) and
fixtures. It may depend only on logic-free shared leaves; the §2 table now names thirteen of
them (`tools/capture/docs/00-spec.md:34-44`; `tools/capture/src/lib.rs:41-49`;
`README.md:400-450`). Adding a leaf requires synchronized updates to that table, the matching
manifest comment block (`tools/capture/Cargo.toml:1-44`), the README inventory and
`docs/06-artifact-bridge-spec.md:735-759`; the boundary is explicitly not mechanically
enforced. The bridge must not depend on gircap (`docs/06-artifact-bridge-spec.md:731-759`).

**A5 — Byte contracts are load-bearing.** Capture bundles, dump key sets and artifact bodies
are compared byte-for-byte after normalising only known-volatile fields
(`tools/capture/docs/04-bundle-differ.md:35`, `:293`), schema version 3 additions are required
and version 2 refused (`tools/capture/src/lib.rs:93-133`), protobuf field names `chuncks` /
opaque tag 6 are published bytes (`crates/proto/src/lib.rs:18-25`), the cache wire format is
pinned by literal-byte tests (`crates/sync-cache/src/wire.rs:118-140`), and stdout emits each
`SUMMARY` key only when its flag is active so an unflagged run stays byte-identical
(`crates/report/src/emit.rs:195-209`, `src/repair/mod.rs:1169-1177`). An extraction that changes
key order, casing, spacing, wording or flag conditions is a contract break even when tests
pass.

**A6 — Report planes cannot be merged verbatim; only mechanics are shared.**
`girpr-report-core` deliberately omits `preserve_order` and owns only ordered-JSON and
line-writing mechanics: girpr writes event-first declaration order with `1`/`0` flags,
girsync alphabetical order, gircap event-first with a real `event` field and percent-encoded
text values (`crates/report-core/src/lib.rs:1-22`, `:32-40`; `README.md:404-407`, `:443-450`).
The three renderers have converged on `serde::Serialize` + `serialize_entry`, so the mechanism
converged but the planes did not (`tools/capture/docs/03-design-notes.md:65`).

**A7 — Exit codes are separate contracts.** girpr's 0–5 table (`docs/03-repair-rust-implementation-plan.md:81-91`),
gircap's codes (`tools/capture/docs/00-spec.md:834-849`) and the tools' literals are independent;
`girpr-exit` and `girpr-cli` share only the enum/`ALL`/code/pin generation pattern and the
clap-error → usage-code mapping (`README.md:416-422`).

**A8 — There is no single "digest" semantics.** girpr parses a strict 16-byte `Md5` where
empty means "unknown, not a match" (`crates/digest/`), girsync stores raw bytes plus verbatim
foreign algorithm names with an 80-byte/no-allocation guarantee
(`tools/sync/DESIGN-NOTES.md:173-211`), and gircap publishes lowercase hex strings
(`tools/capture/hash/`). A shared type must not collapse these or absorb `DigestSet`/`AlgoSet`/
`HashAlg`. Sources: prior report 08 `A7`, `N5`.

**A9 — Divergent policies are intentional keepers, not bugs.** Retry jitter is a parameter
because girpr passes `0.5` and gircap passes `0.0` for byte-stable artifacts
(`crates/retry/src/lib.rs:12-18`); the INI readers take last-match/no-section versus first-match
in `[General]` (`crates/util-ini/`, `tools/capture/Cargo.toml:211-215`); `path_key` folds case
unconditionally in girsync but only on Windows in the product (`tools/sync/src/util.rs:376-406`,
`crates/util-path/src/path.rs:259-264`); cache open fails open for a missing cache but closed
for a corrupt one (`tools/sync/src/effective/side.rs:204-225`). The deleted `DO-NOT-TOUCH`
guidance (`git show bec516b6^:REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`) requires a session that
finds a keeper should be changed to stop and raise a follow-up.

**A10 — nextest shapes every test abstraction.** It runs one process per test and skips
doctests, so shared helpers must be public library surfaces reached through thin shims
(`tests/common/mod.rs:1-15`, `crates/log/src/test_support.rs:10-14`,
`crates/tempdir/src/lib.rs:62-65`); a process-global `OnceLock` fixture helps only within one
test. Overrides pin host-dependent skips by package + binary + fully-qualified test name
(`.config/nextest.toml:101-243`), so moving a test changes the filter. Thread-local decision
records are safe precisely because each test is its own process (`src/repair/diagnostics.rs:11-15`).

**A11 — Feature unification produces two cached builds of the same crate.** Edition 2024 implies
resolver 3: a package under test unifies its dev-dependency features into its own normal
dependencies while `cargo build` keeps a feature-reduced set (`Cargo.toml:205-214`). `log`'s
`test-support`/`test-support-async` split exists so sync-only consumers never compile tokio
(`crates/log/src/test_support.rs:16-24`). Consolidating test surfaces can therefore *increase*
compiled variants, not decrease them.

**A12 — Cycles constrain where a boundary can be cut.** A boundary must sit at an acyclic leaf
or own a whole strongly-connected group. The `report::summary ↔ repair::scope::ledger` cycle was
resolved by moving `Summary`/`Meter`/`StoppedBy` into one crate (`crates/report/src/lib.rs:10-12`,
`src/repair/scope/ledger.rs:1-9`). Remaining cycles: `effective ↔ planner`
(`tools/sync/src/planner/plan.rs:15`, `effective/phase_c.rs:16`), `config ↔ repair`,
`scope ↔ plan/limits`, `http ↔ biz` (`tools/capture/src/biz/mod.rs:26`, `http/probe.rs:32-33`),
`cli ↔ config` (`tools/capture/src/config/mod.rs:17-18`, `cli/mod.rs:135`), and `file ↔ scan`
(`src/repair/scan.rs:31,194-199`, `src/repair/file/mod.rs:85`).

**A13 — Verification must remain an independent re-derivation.** girsync requires two reads and
a failed flush because a loop over one buffer asserts `x == x` (`tools/sync/DESIGN-NOTES.md:21-44`);
gircap keeps pass 2 independent of pass 1 for the same reason
(`tools/capture/docs/03-design-notes.md:71-72`); girpr's hash gates and coarse-MTIME guard are a
correctness argument, not a cache (`src/repair/file/inplace.rs:789-918`). Any "generic engine"
that reuses an observation buffer turns "verified" into a tautology.

**A14 — New crates have a fixed set of doc and runner seams.** Workspace membership
(`Cargo.toml:40-44`), README crate table and prose (`README.md:38-87`, `:400-501`), the gircap §2
table (`tools/capture/docs/00-spec.md:34-57`), the long tool manifest comments
(`tools/capture/Cargo.toml:8-45`, `tools/sync/Cargo.toml:37-104`), and nextest filters
(`.config/nextest.toml:101-243`) must move together. Historical follow-up commits exist solely
to correct stale inventories (`f054b3c8`, `0051e2aa`, `ab98c622`).

**A15 — The prior exclusion list still holds unless explicitly revisited.** Prior report 08's
`N1`-`N10`, the `DO-NOT-TOUCH` §H items (rw gate, FNV lock key, fail-fast cache open, symlink
classification, bounded ordered runner, `_tmp` resume, parity rules, replay/bridge server), and
the cache/publish invariants at `tools/sync/DESIGN-NOTES.md:749-818` remain negative findings
here. A finding below can revisit their *premise*, but none proposes deleting them.

## 2. Findings

### 2.1 Baseline: what is already shared

**F1 — Sharing is still mechanics-only, and the mechanical layer is largely complete.**
The 27 crates carry log policy, digest spelling, fs-name and fs-identity facts, glob filtering,
retry schedules, exit-table generation, clap-error mapping, audio-language tokens, proto wire
types, region values, ordered-JSON writing, limit counting, INI reading, one-read MD5 scanning,
path containment/atomic publish, temp directories, the loopback server, the fetch path, the
report plane, the audio boundary, the run lock and the three test-support libraries
(`README.md:400-501`; crate `description` lines). Every finding below is duplication *above* that
line. Confidence: high.

**F2 — The tempdir→girsync inversion was relocated, not eliminated.** `girpr-tempdir` is now
explicitly girsync-free (`crates/tempdir/Cargo.toml:20-24`), but the same test-only-inversion-
as-normal-dependency pattern lives in `girpr-sync-test-support → girsync`
(`crates/sync-test-support/Cargo.toml:15-22`), `girpr-test-support → girpr`
(`crates/test-support/Cargo.toml:15-24`) and `gircap-test-support → gircap`
(`tools/capture/test-support/Cargo.toml:24-28`), and `sled2redb → girsync` is a normal dependency
(`tools/sled2redb/Cargo.toml:29`). Any cut that promises "facade-free test support" is a
high-risk test-graph change, not a new mechanical extraction. Confidence: high.

**F3 — Several previously skipped extraction tasks now have clean leaf boundaries.**
The prior plan skipped T18 (API DTO), T02 (chunk identity), T09 (bounded zstd) and T12 (full CLI
leaf) as stale-payoff or no-value. Their premises have changed: the DTO decoders now sit in two
isolated crates (`crates/fetch/src/hyp.rs:31`, `tools/capture/src/hyp/types.rs:14`), the identity
predicates in two small modules (`crates/fetch/src/sophon/map.rs:24-65`,
`tools/capture/src/blobs/chunks.rs:71-99`), zstd in `crates/fetch` and `tools/capture/hash/`, and
`girpr-cli`/`girpr-exit`/feature-gated `girpr-model` all exist. A re-assessment is warranted even
where the original "do not" verdict was correct at the time. Confidence: high on the premise
change, low-to-medium on any individual re-extraction.

### 2.2 Chunk-level domain mechanics (densest cross-tool duplication)

**F4 — Chunk identity `(md5, size)` is one rule in two representations.** The product's single
predicate is `ReuseEntry::matches`/`matches_identity` over a parsed `Md5` and `i64` size, used by
the reuse index, the plan estimate and both write paths (`crates/fetch/src/sophon/map.rs:21-116`,
`src/repair/reuse.rs:174-191`, `src/repair/phases/plan.rs:348-361`). gircap defines
`Identity<'a> { uncompressed_md5: &'a str, uncompressed_size: i64 }` with `Identity::of`, a
borrowed view introduced to avoid allocating per comparison, and states the same contract —
same MD5 with a different length is not the same bytes (`tools/capture/src/blobs/chunks.rs:6-15`,
`:71-99`); the identity is repeated in `tools/capture/src/manifest/proto.rs:111-155`. Candidate
boundary: a program-free `girpr-chunk-identity` leaf built on `girpr-digest`/`girpr-proto`, owning
the digest-bytes/length key, a borrowed view and the case-insensitive-hex parse rule; each caller
keeps selection and collision policy. High-risk because representation (parsed bytes vs string),
collision handling (`(hash,size) → Vec<usize>` with confirmation at `map.rs:72-112` vs gircap's
`BTreeMap` sets) and first-match ordering decide which bytes are fetched and therefore the
`PLAN`/`SUMMARY` bytes (A5, A4). Confidence: high on duplication, medium on a safe boundary.

**F5 — Chunk layout/range algebra is implemented twice with different verdict semantics.**
The product computes `PlannedFile::layout()`/`is_tiled()`, gaps, overlaps, checked ranges and
`validate_chunk_ranges` as a fatal plan error (`src/repair/phases/plan.rs:63-152`, `:375-407`,
`:487-502`), with saturated folds in `src/repair/file/inplace.rs:1027-1038` and
`src/repair/file/temp.rs:196-212`. gircap's `assemble::ranges::{Range, Layout}` uses the same
half-open model with saturating gap diagnostics and a "violations as data, never an error"
contract (`tools/capture/src/assemble/ranges.rs:1-28`, `:29-241`), plus arithmetic assertions in
`tools/capture/src/verify/checks.rs:295-320`, `:628-680`; product-side validation wording is
pinned by selfcheck-like output (`tools/capture/docs/03-design-notes.md:128-131`). Candidate
boundary: a `girpr-chunk-layout` leaf with `Range`/`tiled`/`gaps`/`overlaps` and checked, saturating
construction. High-risk because one caller treats a bad layout as a repaired-with-zero-fill file
and the other as an assertion finding that must still be described (A13). `assemble/ranges.rs`
imports only `serde`, making it the cleanest of the domain modules; `verify/checks.rs` is not
leaf-ready (F28). Confidence: high.

**F6 — The download-projection formula is literally documented as a cross-program copy.**
`est_file_download_bytes` says it is "character-for-character `gircap`'s `patch_fetch` formula"
so the `PLAN` line, `SUMMARY plan_download_bytes` and capture's `totals.download_bytes` are one
figure (`src/repair/phases/plan.rs:340-361`); the gircap mirror is
`projected_download(wants) = Σ compressed_size.max(0)` with the same saturating rationale
(`tools/capture/src/blobs/mod.rs:972-982`, `blobs/plan.rs:1-15`). The product derives the same
projection twice — no-scan (`plan.rs:321-337`) and scanned (`src/repair/scan.rs:145-281`). Candidate
boundary: a pure `girpr-chunk-cost` fold over `(identity, compressed_size) × local-reuse
membership`, with each program supplying the set. High-risk because the estimate is simultaneously
a ceiling input, a published `PLAN` key and the bundle ranking key that decides which files enter
the artifact (`src/repair/mod.rs:654-678`, `tools/capture/src/run/mod.rs:322-375`), and
`plan.rs:614-623` records it as a lower bound, not a promise. Confidence: high.

**F7 — Reuse/salvage ordering is a pure algorithm with a divergent twin.** `src/repair/reuse.rs:1-191`
is an entirely pure write-order decision (ascending survives iff `r_i >= o_i`, descending iff
`r_i <= o_i`, tie to ascending, conservative for holes), with the deferred optimum deliberately
left as a swappable function (`:39-56`). gircap's `retention`/`Set::{Reuse,PatchFetch,Materialize}`
makes a different decision — same-file content identity, not write-order survival
(`tools/capture/src/blobs/chunks.rs:3-40`, `:242-320`). Candidate boundary: a `girpr-reuse-decision`
leaf parameterized by source provider with identity-only and order-survival modes. High-risk
because the writer uses the real file length as `usable_len` while the estimate uses `u64::MAX`
because containment was already proved (`src/repair/file/inplace.rs:470-481`,
`src/repair/scan.rs:252-278`) — a load-bearing asymmetry a unified API must preserve. Confidence:
medium-high.

**F8 — Digest normalization placement is byte-load-bearing.** gircap lowers digest strings at
the single decode boundary specifically "to match girpr" and publishes the lowered forms in
manifests and every downstream key (`tools/capture/src/manifest/proto.rs:111-183`); the product
compares with `Digest::matches_hex` instead (`src/repair/file/temp.rs:374,645-646`,
`inplace.rs:419`). Candidate boundary belongs with F4, not in `girpr-proto`: moving normalization
into the wire leaf changes dump bytes and violates the logic-free rule (`tools/capture/Cargo.toml:184-187`).
Confidence: high.

**F9 — Manifest-stats reconciliation exists only as capture assertions.** getBuild `stats`
reconciliation (file count, chunk count, uncompressed size) and structural properties are
recorded, never fatal (`tools/capture/docs/00-spec.md:206-218`, `:643`,
`tools/capture/src/hyp/types.rs:713-745`, `verify/checks.rs:174-179`), while the product's
plan-time range validation is fatal (`src/repair/phases/plan.rs:375-385`). Candidate boundary: a
`girpr-manifest-stats` leaf with sum/compare primitives parameterized on fatal-versus-recorded
and preserving the string wire form. High-risk because the artifact records the violation as
evidence; normalizing a bad value destroys the finding. Confidence: low-to-medium.

**F10 — API DTO decoders are still duplicated; the earlier "shared DTO" premise is false in
this tree.** `girpr-model` exposes only `region` (`crates/model/src/lib.rs:19-21`), and the DTOs
remain private to `crates/fetch/src/hyp.rs:31,169,219-221` (`Wrapper<T>`, coercing
`num_from_string`) and `tools/capture/src/hyp/types.rs:14,38-106,108-122,529-745`
(`IntOrStr`, wire-preserving, forward-compatible). Candidate boundary: a `girpr-api-dto` leaf
holding only the shared wire shapes with both scalar adapters side by side — the boundary the
skipped T18 task itself specified. High-risk because capture re-serializes `IntOrStr` to the
same JSON form and captures bodies byte-exact (A5), URL-join strategies are a recorded
divergence, and a new gircap leaf dependency forces two doc-table updates (A4, A14). Confidence:
medium — the most concretely revisit-able of the skipped tasks.

### 2.3 Product repair internals

**F11 — A `file ↔ scan` cycle blocks extracting chunk verification.** `src/repair/scan.rs:31,194-199`
imports and calls `super::file::{Mode, resolve_mode}`, while `src/repair/file/mod.rs:85` and
`inplace.rs:22` import `ScanReport`, `Verdict`, `scan_ranges`, `source_ok_by_chunk` and
`source_ok_by_range`. Candidate boundary: a `girpr-chunk-verify` leaf owning those verdict and
remap types plus `verify_in_place`; `Mode`/`resolve_mode` must move first to a neutral leaf.
High-risk because `Verdict::modified` plus the coarse-MTIME guard is a correctness proof —
changing it can make the writer skip a file whose bytes changed (A13). Confidence: high.

**F12 — The plan/purge decision graph is concentrated in a 1,344-line `repair/mod.rs` and is
cyclic.** `plan_phase`, `build_and_assert_plan`, `plan_work_and_purge`, `estimate_download`,
`within_limits`, `publish_plan`, `fetch_deprecated`/`normalize_deprecated` and `apply_run_flags`
live at `src/repair/mod.rs:178-790`, `:1142-1281`, while the types they decide over point back:
`Limits::check_preflight` imports `RepairPlan`/`PurgePlan` (`src/repair/scope/limits.rs:26-45`,
`:98-135`), `RepairPlan` needs `LocalChunkMap` from `girpr-fetch` (`phases/plan.rs:314-337`), and
`PlanTotals` needs `file::Mode` (`scope/report.rs:28-33`, `:263-294`). Candidate boundary: a pure
`girpr-repair-plan` crate owning `PlanInputs`/`PlanWork`/`DownloadEstimate`/`PlanSummaryValues`
and the "scope selected something" rule. `RepairPlan`, `PlannedFile`, `Layout`, `PurgePlan`,
`Mode` and `PathScope` must become leaf-owned values first — the same move the ledger cycle
required (A12). Confidence: high on the structure, medium on a clean boundary.

**F13 — "A filtered-out path is nonexistent" is one rule at four-plus call sites, with an
opposite mechanic in girsync.** The invariant is declared at `src/repair/scope/mod.rs:15-19` and
re-enforced in `phases/plan.rs:417-449`, `purge.rs:188-215,470-519`, `phases/post.rs:604-666`,
`phases/metadata.rs:459-479` and `file/mod.rs:514-518`. girsync hides filtered entries from the
effective map but must still stat the disk for blockers before the first change
(`tools/sync/src/commands/sync/mod.rs:24-25`, `tools/sync/README.md:254-278`). Candidate boundary:
a `girpr-path-scope-semantics` leaf owning "apply scope to the cleaned `/` rel before any sink;
a hidden path is kept by purge and absent from the plan", with `girpr-filter` already shared
(`Cargo.toml:131-133`, `tools/sync/Cargo.toml:72-74`). High-risk because widening the girsync map
to retain hidden blockers would let the planner delete a path the filter promised to leave alone;
the rule also decides the `config.ini` bump (`src/repair/phases/post.rs:135-144`). Confidence:
medium-high.

**F14 — Byte-ceiling accounting names three roles without a shared type.** The product
distinguishes projected (`DownloadEstimate.bytes`, `src/repair/mod.rs:632-678`), live/spent
(`reserve_download_bytes`, `reserve_write_bytes`, `adjust_download_bytes` in
`src/repair/file/temp.rs:572-707` and `file/mod.rs:739-789`) and settled bytes
(`settle_download_bytes`, `note_write` in `file/mod.rs:725-737`, `:303-345`), with first-stop-wins
in `note_stopped` (`repair/mod.rs:1204-1213`). gircap has a projected/live split for two ceilings
(`tools/capture/src/blobs/plan.rs:24-54`, `run/mod.rs:322-375`) but deliberately no mid-flight
ledger. Candidate boundary: a `girpr-byte-accounting` leaf naming and relating
`Projected`/`Live`/`Settled`/`FirstStop` and the reservation arithmetic — explicitly *not* the
`>` predicate or counters that prior report 08's `N2` excluded (and `src/repair/scope/limits.rs:169-170`
says the `describe()` shape was copied for the habit, not shared code). High-risk because a wrong
settle changes published `SUMMARY` bytes and `stopped=` (A5). Confidence: medium.

**F15 — The write-mode decision is triplicated inside the facade.** `RepairMode → Mode` is decided
by `resolve_mode` (`src/repair/file/mod.rs:692-706`), re-derived by `DiskPlan::from_figures` for
test injection (`src/repair/scope/report.rs:71-124`, `:126-192`), and consulted by
`ScanReport::est_download_bytes_by_file` (`src/repair/scan.rs:177-209`). Candidate boundary: a
`girpr-repair-policy` leaf owning `RepairMode`, `Mode` and the single resolve rule; `RepairMode`
currently lives in `src/config.rs:22-62`. High-risk because the disk-plan duplication exists to
keep an injection seam and the estimate's temp/discard branches depend on the exact outcome
(`scan.rs:232-251`). Confidence: medium-high.

**F16 — Three very large files are the remaining cache-reuse bottleneck.** `src/repair/file/inplace.rs`
(2,219 lines), `src/repair/file/temp.rs` (1,455), `src/repair/phases/post.rs` (1,471) and
`src/repair/mod.rs` (1,344) hold the decision/execution graph; one edit recompiles the facade,
then `girpr-test-support` (F2), then all four integration roots (`Cargo.toml:20-38`). Candidate
boundary follows F11/F12: chunk-verify and plan leaves. High-risk because these files encode the
byte-safety proofs and their `#[cfg(test)]` halves are much of the suite (A10). Confidence: high
on the observation, medium on a clean boundary.

**F17 — Both programs keep a parallel machine-readable decision trace with incompatible
transports.** girpr records structured choices in a thread-local `RepairDiagnostics` that a
shipped run writes and never reads (`src/repair/diagnostics.rs:1-97`, `src/lib.rs:133-137`,
`:185-197`); gircap serializes one `Assertion` row per assertion instance into bundle bytes
(`tools/capture/src/emit/selfcheck.rs:1-60`, `:313-339`, `:412-427`). Candidate boundary: a
transport-free `girpr-decision-record` primitive for "record a typed decision, drain it", with
each program owning its fields. High-risk because one transport is thread-local +
current-thread runtime and the other is a published artifact (A5, A10). Confidence: low.

**F18 — Report provenance is a value-shape duplication that must not become a format crate.**
The begin report is built from `Meta` into `ReportProvenance` — local/latest version, biz, exe,
download mode, branch, package, build id, diff tags, audio (`src/repair/mod.rs:277-297`,
`crates/report/src/emit.rs:127-188`); gircap builds bundle provenance from the same API answers
(`tools/capture/src/hyp/mod.rs:174-226`, `emit/selfcheck.rs:176,464-469`). Candidate boundary: a
*format-free* `girpr-install-provenance` value crate holding only the scalar identity fields and
their extraction policy. High-risk/near-forbidden: the moment it emits a key or fixes a field
order it becomes a shared contract (A4, A6). Confidence: low-medium.

### 2.4 gircap-specific candidates

**F19 — The "legal relative path" rule is written twice with opposite normalization.** gircap's
`path_rule` rejects backslash, `.`, absolute paths, drives, devices and trailing dot/space and
preserves the game's spelling while escaping only the on-disk component
(`tools/capture/src/paths.rs:67-140`, `:181-234`); `crates/util-path/src/path.rs:55-139`
normalizes `\`→`/`, drops `.` components and accepts `:` except after the drive prefix, storing
the cleaned form in the plan. Candidate boundary: a parameterized path-shape validator in a
`girpr-path-shape` leaf with policy fields (normalize vs reject, reserved set, trim flag, colon
rule) and a caller-supplied stable-rule-ID mapping. High-risk because the policies are semantic —
a wrongly kept `./` would make purge delete the just-repaired file — and the rule IDs feed
`diagnostics/rejections.json` (`tools/capture/src/rejections.rs:63-69`, `src/paths.rs:69-137`).
Confidence: high.

**F20 — A bounded, deterministic refusal ledger exists only in gircap.** `RejectionsDocument`
caps retained value bytes (512) and rows (4,096), keeps the *smallest* rows by
`(source, value, rule)` so two captures of one input agree regardless of completion order, and
counts duplicates/omissions (`tools/capture/src/rejections.rs:40-52`, `:54-88`, `:119-191`); rule
IDs are minted in `src/paths.rs:69-137,277-291`. No analogue exists in girpr/girsync, whose
validators return `anyhow::Error` strings never collected into a sidecar. Candidate boundary: a
`girpr-refusal-ledger` leaf owning bounded order-independent retention and the JSON envelope.
High-risk because it creates a new shared *observation format* across programs, which A4 forbids,
and the sampling/`omitted` count is byte-visible (A5). Confidence: medium.

**F21 — Region resolution shares values but not request building.** `girpr-model::region` is shared
(`crates/model/src/lib.rs:19-21`), but capture's `Biz::resolve`, `Endpoints` URL builders,
`push_pair`/`enc` form encoding and `BranchParams` remain local
(`tools/capture/src/biz/mod.rs:96-141`, `:143-229`, `:231-351`); girpr keeps its resolve policy in
`crate::hyp` (`Cargo.toml:113-116`). Candidate boundary: a `girpr-request-url` leaf of pure
functions over the shared region plus a base/override seam. High-risk because query order and
escaping are load-bearing when a reader diffs a captured URL against a fresh request
(`tools/capture/src/biz/mod.rs:207-215`, `:300-302`) and `--hyp-base`/`--sophon-base` is capture's
testing seam. Confidence: medium.

**F22 — Schema-versioned ordered-JSON envelopes are repeated in five documents.** Capture stamps
`schema_version` and tool name/version in bundle manifests, selfcheck, pathmap and rejections
(`tools/capture/src/lib.rs:93-133`, `emit/index.rs:62-130`, `emit/selfcheck.rs:21-115`,
`pathmap.rs:44-67`, `rejections.rs:72-88`), borrowing `girpr-report-core` only as a writer.
Candidate boundary: deepen that leaf into a `girpr-document-envelope` owning
`{schema_version, tool, ordering, newline policy}` with per-program field lists. High-risk because
A4 reserves report format and A5 pins the key sets; the published wording table in
`emit/selfcheck.rs:118-135,1028-1058` must not be absorbed. Confidence: medium.

**F23 — Publish-target validation is duplicated while write/atomicity policy intentionally
diverges.** Capture validates trailing dot/space and device names before writing bundle paths and
writes small documents non-atomically (`tools/capture/src/paths.rs:110-132`, `emit/mod.rs:363-389`),
while `crates/util-path/src/atomic.rs:46-81` refuses a publish target with a Windows trailing
dot/space before a `NamedTempFile::persist`. Candidate boundary: extract only the predicate into
the fs-names leaf; leave each writer's atomicity policy local. High-risk because the two callers
use it at different times with different consequences and an over-eager merge would delete the
intentional non-atomic metadata writer or the blob rename (A4). Confidence: medium.

**F24 — Range-like CLI arguments are validated three ways with three error semantics.**
`tools/capture/src/config/mod.rs:439-483` maps out-of-range transport values to `UsageError` →
exit `1`; `src/cli.rs:23-50,298-301` uses `MAX_JOBS` and `bail!`; `tools/sync/src/config/common.rs:385-414`
uses an `nz` closure returning exit `3` deliberately instead of clap's `2`. Candidate boundary: a
`girpr-value-ranges` leaf of pure predicates returning facts, with each program mapping a fact to
its own error type and exit class. High-risk because the error strings and exit codes are
published contracts (A5, A7). Confidence: high on duplication, medium that it is worth coupling.

**F25 — Offline verification walkers duplicate a "re-derive from recorded bytes" discipline.**
Capture's `verify/checks.rs:19-97,139-200` and `verify/materialize.rs` rebuild both sides, with a
published split between re-derived and unrecorded findings (`tools/capture/docs/00-spec.md:752-770`,
`:772-779`); girpr's analogue is the check phase plus `src/repair/scan.rs:373-449` and
`phases/limits.rs:98-162`. Candidate boundary: a shared memoized blob→bytes→digest walker with a
recorded-versus-re-derived comparison shape. High-risk because A6 structural properties are
re-derived against `chunks/index.json`, not manifests, and verify reports coverage as findings
rather than failures (A13). Confidence: medium.

**F26 — The HTTP transport wrapper is duplicated, but its ownership choice is deliberate.**
`gircap-http` owns `ReqwestTransport` and `HttpTransport::send` with a 500 ms/10 s/zero-jitter
`RetryPolicy` (`tools/capture/http/src/transport.rs:14-59`, `:259-279`,
`tools/capture/http/src/lib.rs:1-19`), while `girpr-fetch` owns its own transport and a 1 s/30 s/
±50 % `BackoffPolicy` (`crates/fetch/src/retry.rs:19-64`) and the root takes `reqwest` directly
(`Cargo.toml:106`). Candidate boundary: a program-free `girpr-http-transport` leaf with
`HttpRequest`/`HttpResponse`/`HttpTransport` and the retry driver, classifiers local. High-risk
because it reverses the documented reason `gircap-http` exists — keeping reqwest's native tree
cached and no transport dependency crossing into gircap (`tools/capture/docs/00-spec.md:43`,
`tools/capture/Cargo.toml:47-56`, `:164-171`) — and would couple three programs' reqwest features
(A2, A4). Confidence: high on duplication, medium on extraction.

**F27 — Retry decisions share a mechanics layer but disagree on classifier and jitter.** The
shared `girpr-retry` owns only `Schedule`/`Sleep` drivers (`crates/retry/src/lib.rs:1-16`, `:27-104`);
`girpr-fetch` writes its own attempt driver with `Retry`/`RetryState` and a state slot
(`crates/fetch/src/retry.rs:1-13`, `:94-162`) and `gircap-http` writes a third loop with per-attempt
request logging and retained error bodies (`tools/capture/http/src/lib.rs:37-44`,
`src/http/fetcher.rs:789-862`). Candidate boundary: a shared `drive_attempts` in `girpr-retry`
(classification local) or, less invasively, a shared decision vocabulary
`RetryDecision::{Done,Fatal,Transient}` with a caller-supplied status set. High-risk because the
status set is itself published (`error_shapes.json`) and the jitter choices encode opposite goals
(A9). Confidence: high.

### 2.5 girsync and sled2redb

**F28 — A generic cache/DB abstraction would fight an intentionally missing product cache.**
The sync cache is one specialized schema (`entries`/`meta`, `FileRec`, version gate, positional
postcard codec: `crates/sync-cache/src/lib.rs:65-69`, `schema.rs:32-47`, `wire.rs:9-50`,
`open.rs:316-455`), while the product deliberately has no manifest cache
(`docs/02-repair-chunk-spec.md:385`, `docs/03-repair-rust-implementation-plan.md:184`). Candidate
boundary: a parameterized "verified KV cache" that both `girpr-sync-cache` and a future product
cache could implement. High-risk because the literal-byte contract (A5) and version gate would sit
behind a generic row trait and the abstraction would tempt reinstating a forbidden cache. redb is
pinned to `=5.0.0` (`crates/sync-cache/Cargo.toml:21`) and sealed its handle API, forcing a local
trait (`db/handle.rs:7-17`) that is not itself reusable. Confidence: medium.

**F29 — Plan/execute engines have converged in shape, but the order is not expressible as a
crate.** girsync documents eight phases whose order is the invariant and states phases are
*named rather than extracted* (`tools/sync/src/commands/sync/mod.rs:9-27`, `:29-50`); girpr's
`run` is "plan, then assert, then execute" (`src/repair/mod.rs:18-39`, `README.md:61`); gircap has
a verify/materialize half. Candidate boundary: a `girpr-engine` leaf with `Plan`/`Execute` and a
`Verify` trait requiring two independent observations (A13), with injected clock/sink/job
limiter. High-risk because it crosses three exit-code contracts (A7), three report planes (A6),
the open digest vocabulary (A8), the tool SCCs (A12) and the `Meter`/`Summary` constraint (F12).
Most plausible outcome is a shared trait shape only. Confidence: low.

**F30 — `path_key` has two deliberate fold rules and three deliberately disagreeing identity
questions.** girsync folds ASCII case on every host (`tools/sync/src/util.rs:376-406`, `:400-405`),
the product only on Windows (`crates/util-path/src/path.rs:259-264`), and `DESIGN-NOTES.md:102-145`
records three distinct rules (map key, same-open-place, on-disk entry spelling). Candidate
boundary: a `girpr-path-identity` crate with three named question types and parameterized folds.
High-risk because a single default fold silently changes which files a repair or mirror touches,
and the canonicalization spellings are already split across `girpr-fs-names`, `girpr-util-path`,
`girpr-lock` and girsync's direct `dunce` dependency (`crates/fs-names/src/lib.rs:20-39`,
`crates/util-path/src/path.rs:434-470`, `crates/lock/src/lib.rs:112`,
`tools/sync/Cargo.toml:61-64`). Confidence: high.

**F31 — Fail-open/fail-closed ("gray-box") policy is repeated as ad-hoc branching.** girsync
fails open for a missing or `--ignore-cache` cache but closed for a corrupt one
(`tools/sync/src/effective/side.rs:204-225`), fails closed on coverage before any read
(`planner/coverage.rs:1-30`, `planner/pairs.rs:185-223`), and marks "cannot tell" as
`Unverifiable`/`CHANGED` (`diff/verdict.rs:53-63,104-126`); the product has its own fail-closed
ceiling policy (`src/repair/scope/mod.rs:6-19`). Candidate boundary: a generic `EvidencePolicy`
with an explicit `CannotTell` arm and a "fail closed unless the missing input contributed
nothing" rule. High-risk because collapsing `CannotTell` into equality produces the false-clean
outcome both tools guard against (`tools/sync/DESIGN-NOTES.md:794-800`). Confidence: high.

**F32 — The relocated inversion is still a high-risk cut.** `girpr-sync-test-support` constructs
girsync's `CommonOpts`, `CacheDb`, `HashPlan`, `Report` and in-process runners
(`crates/sync-test-support/src/lib.rs:24-37`, `:549-612`, `:571-612`), and its seams call
`girsync::run_captured_with_hooks` and `subscriber_for` directly (`:592`, `:611`, `:819`). Candidate
boundary: split the fixtures into a generic CLI-runner/option-builder crate parameterized by
product types so neither test crate needs a normal edge on a program. High-risk because a
trait-based abstraction must reproduce env pinning (`GIRSYNC_TEST_CACHE=file/memory`) and nextest
isolation exactly or every in-process suite reverts to spawning children (A10, F2). Confidence:
high.

**F33 — Atomic publish exists in three forms with incompatible contracts.** `girpr-util-path::atomic`
uses `NamedTempFile` + `persist` with a `durable` flag (`crates/util-path/src/atomic.rs:1-30`,
`:62-114`); `girpr-sync-cache` uses working names, link-into-place and per-copy hold records,
explicitly accepting one-syscall gaps (`open.rs:9-95`, `:765-992`, `backup/publish.rs:1-13`);
sled2redb uses a random `girpr-cache-convert-*.redb` name with `persist`/`persist_noclobber` and
deliberately leaves leftovers (`tools/sled2redb/src/convert.rs:43-84`, `:264-330`); girpr renames
`_tmp` over a repaired file (`src/repair/file/temp.rs:748`). Candidate boundary: a generic
`AtomicPublish` with explicit durable/parent-creation/replace-vs-noclobber parameters. High-risk
because sync-cache's *bound* reads published names as whole copies and its hold records protect a
pre-lock gap, while sled2redb's namespace is deliberately unreclaimed (A13). Confidence:
medium-high.

**F34 — sled2redb's reuse of girsync's report plane constrains generalization.** The converter
builds its record with `girsync::report::{Record, Report, OutputFormat}`
(`tools/sled2redb/src/lib.rs:37`, `:67-89`) and `OutputFormat` is a clap `ValueEnum` in girsync
(`tools/sync/src/report/mod.rs:58-69`). Candidate boundary: one generalized report plane
parameterized by order, field vocabulary and renderer. High-risk because generalization would pull
clap into a shared crate and any default-order change flips pinned NDJSON bytes compared as
rendered text (`tools/sync/src/report/mod.rs:315-341`, `crates/sync-test-support/src/lib.rs:707-718`).
Confidence: high.

**F35 — Embedded test seams block extracting apply/copy and planner subsystems.** Production
files carry typed fault hooks and env reads: `TestHooks` (`tools/sync/src/config/common.rs:23-72`),
`flush_file` (`apply/copy.rs:80-98`), heartbeat (`tools/sync/src/scan.rs:18-49`), commit policy
(`crates/sync-cache/src/write.rs:60-102`), lock-failure injection (`sync-cache/src/open.rs:1000-1019`)
and `#[cfg(test)] pub(super) use` blocks (`open.rs:150-155`, `backup/mod.rs:19-24`). Candidate
boundary: a generic injected-fault seam crate. High-risk because hook fields name girsync paths
and stages (`fail_rename_step1/step2`, `gate_copy`, `heartbeat_entries`) and their env spellings
are pinned by `.config/nextest-test-env` (A10); generic names would silently disconnect tests
from production. Confidence: medium.

**F36 — Plan records and report records are two document shapes tempting a shared abstraction.**
girsync prints a diff plan then actions, with one terminal summary and `because=` never `why=`
(`tools/sync/src/commands/sync/mod.rs:64-88`, `report/mod.rs:192-280`); girpr emits a `PLAN` line
plus a JSON document (`src/repair/scope/report.rs:324`). Candidate boundary: a shared "plan action
record" crate. High-risk because girsync's stream is NDJSON and girpr's is a different document;
unifying the writer would change exit/summary parsing (`tools/sync/src/commands/mod.rs:114-162`)
and the artifact-side JSON contract. Confidence: medium.

**F37 — The in-memory cache backend blocks a generic backend cut.** `TestCacheBackend` is resolved
once in `CommonOpts::try_from` and is path-keyed/shared (`tools/sync/src/effective/side.rs:236-253`,
`crates/sync-cache/src/lib.rs:129-139`); row capture is process-global
(`crates/sync-test-support/src/lib.rs:293-366`). Candidate boundary: a pluggable KV backend with
an in-memory mirror and capture. High-risk because a shared global registry must preserve
"path-keyed, per-process, forgettable" semantics (`forget_memory_cache`, `with_observed_rows`);
generalizing the key or lifetime reintroduces cross-test leakage that nextest isolation prevents
(A10). Confidence: medium.

**F38 — Durability/commit policy is already parameterized inside one crate.** `CommitLimits`
batching by ops/bytes/time and a resolved `CommitPolicy` (`crates/sync-cache/src/write.rs:24-36`,
`:105-120`) pair with `write_durability()` (`sync-cache/src/lib.rs:71-112`); the thresholds are
test knobs (`:60-74`) and pre-drop commit ordering is a crash-safety invariant
(`tools/sync/src/commands/sync/mod.rs:52-62`). Candidate boundary: a generic batched durable
writer. High-risk because it would expose test knobs to consumers that should not have them and
change crash-safety semantics (A11, A13). Confidence: medium-low.

### 2.6 Shared crates and test infrastructure

**F39 — Two near-identical restartable byte-sink traits exist in two shared crates.**
`girpr-digest` defines `BodyFile: Write + Send` with `restart()` (`crates/digest/src/lib.rs:24-37`),
placed there to break a cycle; `girpr-fetch` defines `ChunkSink: Write` with the same method plus
`Vec<u8>`/`&mut T` impls (`crates/fetch/src/sophon/stream.rs:252-273`, used at `:110`, `:439`);
`gircap-http` re-exports `BodyFile` (`tools/capture/http/src/lib.rs:91-100`). Candidate boundary:
one generalized `RestartableSink` with blanket impls. High-risk because the traits differ in
`Send` bounds and in what `restart` means (truncate a file vs `clear()` a Vec), and unifying them
couples fetch's zstd path to the HTTP body seam that the digest leaf deliberately hosts.
Confidence: high.

**F40 — The log-capture buffer is implemented twice under different contracts.**
`girpr-log::test_support::Captured` is `Arc<Mutex<Vec<u8>>>` + `Write` + strict UTF-8 `.text()`
with debug-level scoped subscriber (`crates/log/src/test_support.rs:35-79`, `:99-112`);
`girpr-sync-test-support::SharedBuffer`/`BufferWriter` repeat the shape but use
`from_utf8_lossy`, implement `MakeWriter` and install girsync's full subscriber stack with a
non-blocking file guard and env mutation (`crates/sync-test-support/src/lib.rs:751-787`, `:789-827`).
Candidate boundary: a shared capture-buffer primitive, scoping/subscriber local. High-risk because
the captures are asserted byte-for-byte and strict-vs-lossy UTF-8 or `MakeWriter` differences can
silently alter lines; the sync stack would pull a program crate into the log leaf (A4). Confidence:
high.

**F41 — Three independent in-process command harnesses are test-only inversions.**
`girpr-test-support` builds `RunCtx` and drives `girpr::run` (`crates/test-support/src/lib.rs:39-106`);
`gircap-test-support` re-parses argv through gircap's clap surface and dispatches to four `run::*`
APIs with an injectable transport (`tools/capture/test-support/src/lib.rs:65-173`);
`girpr-sync-test-support` has `run_cli`/`run_cli_mem`/`run_args`/`run_*_text` over
`girsync::run_captured_with_hooks` (`crates/sync-test-support/src/lib.rs:549-749`, `:789-827`).
Candidate boundary: a generic parse→plan→dispatch→typed-outcome harness. High-risk because each
must name program-private construction and the outcome shapes differ (counts vs records vs typed
`Record`+`Failure`); forcing a common surface directly contradicts A4. Confidence: high.

**F42 — Two different "host refused a fixture" skip policies.** `girpr-util-path::test_support`
defines `FixtureRefusal { Host, Injected }`, `skip_fixture_refusal`, a `required_env` switch and
the load-bearing `SKIP precondition:` wording (`crates/util-path/src/lib.rs:33-102`);
`girpr-tempdir::test_support` defines `skip_if_no_link!`/`skip_if_case_insensitive!` with plain
`skip: this platform refused …` wording and no required-case switch
(`crates/tempdir/src/test_support.rs:176-223`). Candidate boundary: one shared precondition/skip
policy. High-risk because nextest overrides key on visible skip lines (A10) and an injected refusal
turned into a host skip would report coverage a passing test never had. Confidence: high.

**F43 — Scratch/fixture support is simultaneously too fine and too coarse.** `girpr-tempdir` owns
`TempDir` plus program-free file/link/mtime helpers (`crates/tempdir/src/lib.rs:70-103`,
`test_support.rs:64-93`, `:250-307`); `girpr-test-fixtures` adds a one-line `temp_dir` wrapper
(`:723-736`); `sync-test-support` re-exports the entire tempdir test surface
(`crates/sync-test-support/src/lib.rs:45-49`); `gircap-test-support` depends on tempdir directly.
`girpr-test-fixtures` itself owns four unrelated concerns — lock cleanup, a distinct artifact
replay server, fixture builders/digest, and scratch/log glue
(`crates/test-fixtures/src/lib.rs:44-57`, `:59-765`). Candidate boundary either way (one generic
fixture-builder crate, or splitting fixture-builder/lock-cleanup/replay-server) is high-risk: a
merge reintroduces girsync's rebuild cascade (F2) and violates A4; a split multiplies shims and
rebuild domains the dev-dependencies were created to reduce. Confidence: high.

**F44 — Test-support feature flags are a generalization trap.** Six crates expose test surfaces
(`crates/log/Cargo.toml:15-25`, `util-hash:15-18`, `util-path:15-20`, `fetch:15-20`,
`tools/capture/hash/Cargo.toml:12-16`, plus `report`'s tokio `test-util` at
`crates/report/Cargo.toml:43-48`). Candidate boundary: one umbrella `test-support` facade
re-exporting every surface. High-risk because features unify across the graph (A11), so enabling
async/test-util/clap-derived features in one facade makes tokio timers/runtime and clap compile
into consumers that currently avoid them. Confidence: high.

**F45 — Timestamp formatting has one shared compact stamp and two RFC 3339 spellings.**
`girpr-log` owns the compact `%Y%m%dT%H%M%S%.6fZ` stamp (`crates/log/src/plan.rs:320-341`, pinned at
`lib.rs:91-111`) used by capture ids (`tools/capture/src/config/mod.rs:781-787`,
`emit/index.rs:74`) and sync-cache backup names (`crates/sync-cache/Cargo.toml:33-34`). But
`gircap-http` formats request-log time with `to_rfc3339_opts(Millis, true)`
(`tools/capture/http/src/log.rs:251`) and `girpr-lock` writes `to_rfc3339()` into the lock body
(`crates/lock/src/lib.rs:82-84`, read at `:353-358`). Candidate boundary: a time/stamp leaf with
named formats. High-risk because all three strings are on-disk or artifact byte contracts (A5).
Confidence: medium-high.

**F46 — "Write one rendered line + newline" is implemented in two shared leaves.**
`girpr-report-core::write_line` writes a string plus exactly one `\n`
(`crates/report-core/src/lib.rs:96-105`, tested at `:199-227`); `girpr-log::install` hand-writes
warnings with `writeln!` (`crates/log/src/install.rs:62-105`). Candidate boundary: promote a tiny
shared line-sink primitive. High-risk because making log depend on report-core pulls `serde` into
every log consumer and blurs the "no report format is shared" rule (A6), and the log call sites
need a caller-chosen target/prefix that `write_line` does not model. Confidence: medium.

**F47 — The 512 KiB hash read chunk is duplicated in two shared crates.** `girpr-util-hash`
declares `MD5_BUF = 512 * 1024` (`crates/util-hash/src/md5.rs:8`, `:125-127`, `:286-299`) and
`girpr-sync-cache` declares `CHUNK = 512 * 1024` (`crates/sync-cache/src/hash/stream.rs:15`, `:83`),
while `girpr-digest::hash_reader` deliberately takes a caller-supplied buffer
(`crates/digest/src/stream.rs:199-219`). Candidate boundary: a shared IO read-buffer policy.
High-risk because it contradicts the caller-owned-buffer seam and changes throughput-sensitive
read boundaries pinned by a small-buffer equivalence test (`util-hash/src/md5.rs:561-577`).
Confidence: medium.

**F48 — Several dependency-free leaves are fragments whose merge is a rebuild-domain trap.**
`girpr-cli` is one function (`crates/cli/src/lib.rs:1-21`), `girpr-exit` one macro
(`crates/exit/src/lib.rs:1-75`), `girpr-util-counter` one function
(`crates/util-counter/src/lib.rs:1-11`), `girpr-audio-language` has no dependencies, and
`girpr-loopback-mock` declares it must never gain any (`crates/loopback-mock/Cargo.toml:15-18`).
Candidate boundary: a `girpr-primitives` consolidation. High-risk because their value is the
rebuild domain (A2), not code volume — merging loopback-mock with cli pulls clap into every
fetch/test-fixture/gircap test build, and merging exit/audio-language couples unrelated
artifact-byte contracts. Confidence: medium.

**F49 — The shared loopback server is only the mechanical layer; scripting is repeated.** The
dependency-free `ScriptedServer` in `girpr-loopback-mock` (`crates/loopback-mock/src/lib.rs:1-17`)
has been adopted across product, fetch and capture, but the bind→script→journal→hit-count layer
is repeated in `crates/test-fixtures`, `crates/test-support`, `crates/sync-test-support`,
`crates/test-fixtures/src/artifact_server.rs` and `tools/capture/test-support`. Candidate boundary:
a `ScriptedOrigin`/`ReplayHarness` extension of the loopback leaf owning journal assertions,
with each program supplying its route table. High-risk because domain servers encode different
contracts (N16) and nextest process isolation makes cross-test fixture state useless (A10).
Confidence: low for domain servers, medium for a narrow hit-count/assertion extension.

**F50 — The three report field models now have a much smaller common target.** `girpr-report-core`
already owns the renderer (`crates/report-core/src/lib.rs:28-94`), so the remaining duplication is
the `Field`/`Value` vocabulary and event placement: `girpr-report` owns counters and 1/0 flags,
gircap's `Val::{Str,Int,Bool}` and event-first order (`tools/capture/src/report/mod.rs:1-21`,
`:69-115`), girsync's alphabetical NDJSON. Candidate boundary: a `girpr-report-model` leaf with
`Field`, `Value`, `EventPlacement` and per-tool schema tables, leaving `report-core` as writer.
High-risk because booleans versus `1`/`0`, percent-encoding and pinned key orders are contracts
(A5, A6), and `girpr-report` owns the recursive `Summary`/`Meter`/`StoppedBy` group (A12).
Confidence: medium-low.

**F51 — A narrow CLI leaf extension is plausible; a unified flag bundle is not.** `girpr-cli`
currently owns only the clap-error mapper (`crates/cli/src/lib.rs:1-21`), while `OutputFormat` is
declared in all three program report planes and log flags are declared three times. The
mechanical prerequisites now exist (`girpr-exit`, feature-gated `girpr-model/clap`). Candidate
boundary: add feature-gated `OutputFormat` and a `LogArgs` builder parameterised by project name
and usage code to `girpr-cli`. High-risk because `--version` presence differs, flag help text is
byte-identical and pinned, exit codes stay separate (A7), and `.config/nextest.toml:101-120` pins
the CLI-contract binary and paths (A10). Confidence: medium for the narrow extension, low for a
single bundle.

**F52 — The bounded-decode/skip verdict has a new boundary but the same risk.** The prior T09
task was skipped with "a forced shared wrapper would add no value", and the
`zstd_measure`/`zstd_measure_from` pair stays deliberately duplicated for distinct error text
(`tools/capture/docs/00-spec.md:44`, `tools/capture/Cargo.toml:58-65`). The two shapes now live in
clean leaves (`crates/fetch/src/sophon/stream.rs`, `tools/capture/hash/src/`). Candidate boundary:
a caller-supplied-ceiling, caller-supplied-sink `bounded_decode` in `girpr-digest`. High-risk
because the streaming invariant is an always-on test that fails if a buffered decode writes the
whole body at once (`docs/03-repair-rust-implementation-plan.md:148-153`) and native `zstd-sys`
rebuild propagation dominates cost (A2). Confidence: low; likely re-confirms the skip.

**F53 — A cross-program fs composition leaf would sit on top of already-shared predicates.**
The predicates are leaves (`girpr-fs-names`, `girpr-fs-identity`, `crates/util-path`,
`crates/sync-leaves`), but their higher-level combinations remain local: girpr's lock no-follow
body guard (`crates/lock/`), girsync's hard-link refusal and reparse classification
(`tools/sync/DESIGN-NOTES.md:280-306`), gircap's canonical visited set in `verify/list.rs`.
Candidate boundary: a `girpr-fs` leaf exposing `canonical_no_verbatim`, identity from path/handle
and `LinkClass`, each with an explicit host/fail-closed policy parameter. High-risk because
reparse classification is domain logic, the one deliberate `unsafe` Windows handle call lives
here, and the device-name drift between girpr and gircap remains an open defect candidate
(prior report 08 `A10`, `F6`). Confidence: medium-low.

## 3. Negative findings

**N1 — INI parsers stay divergent.** girpr's last-match/no-section reader (`crates/util-ini/`) and
gircap's first-match `[General]`-only streaming reader (`tools/capture/Cargo.toml:211-215`,
`emit/selfcheck.rs:176,464-469`) encode different capture and bump behavior. Prior report 08 `N1`.

**N2 — `_tmp` resume/rollback and the audio move are not generic temp files.**
`src/repair/file/temp.rs:214-296`, `:327-467` and `docs/03:186-198` make them durable resume
artifacts, not disposable temps; they must not be absorbed by F33.

**N3 — The FNV lock key, OS advisory locks and sidecar cache lock stay.** `crates/lock/` and
`crates/sync-cache/src/lock.rs:1-22` — the sidecar exists because redb's byte-range lock can block
on Windows, and changing the FNV key orphans existing files. `DO-NOT-TOUCH` §H.

**N4 — `DigestSet`/`AlgoSet`/`HashAlg` stay raw-byte and allocation-free.**
`tools/sync/DESIGN-NOTES.md:173-211,787-792`, `tools/sync/README.md:10`; directly blocks F4/F28
from absorbing girsync's digest vocabulary (A8).

**N5 — The rw gate, lock ordering, bounded ordered runner, symlink classification and
`select/budget.rs` stay local.** `tools/sync/DESIGN-NOTES.md:46-62,765-768`,
`tools/capture/docs/03-design-notes.md:65-73`; prior report 08 `N6`, `DO-NOT-TOUCH` §H. F29 must
not absorb them.

**N6 — The replay/bridge server and all domain fake servers stay independent.**
`docs/06-artifact-bridge-spec.md:731-759`, `tools/capture/docs/00-spec.md:41`, prior report 08
`N7`. Only the mechanism in `girpr-loopback-mock` is shareable (F49).

**N7 — The `zstd_measure`/`zstd_measure_from` pair stays duplicated.** Distinct error text is the
reason (`tools/capture/docs/00-spec.md:44`, `tools/capture/docs/03-design-notes.md:56-73`).

**N8 — Exit-code values, URL/status/retcode helpers and flag sets stay local.** Only the
mechanisms in `girpr-exit` and `girpr-cli` are shared (A7); capture's `validate_base` uses `url`
directly with no transport dependency crossing (`tools/capture/Cargo.toml:164-171`).

**N9 — Capture's artifact writer and its selfcheck wording table stay local.**
`tools/capture/src/emit/mod.rs:363-389` deliberately uses non-atomic documents with one blob
rename; `emit/selfcheck.rs:118-135,1028-1058` pins published wording. F22's envelope must not
absorb either.

**N10 — The four gircap-owned convention deviations must not be "corrected".**
`tools/capture/docs/00-spec.md:46-57`: the `gircap-` prefix, location under `tools/capture/`, and
local lint policy are intentional and block the naive "promote the mechanics crates to shared
leaves" move in F26/F52.

**N11 — Hex, ordered-JSON writing, sleep drivers and clap-error mapping have no remaining
duplication.** `gircap-hash` delegates spelling to `girpr-digest`
(`tools/capture/hash/src/lib.rs:39-45`), `test-fixtures::md5hex` delegates similarly
(`crates/test-fixtures/src/digest.rs:8-11`); `girpr-report-core` is the sole ordered-JSON emitter;
`girpr-retry` owns all sleep drivers and `gircap-http` only re-exports them; `girpr-cli` has one
consumer set. Do not re-propose these extractions.

**N12 — `logs_of` is not duplicated; the separate capture buffers are (F40).** The one
implementation is `crates/log/src/test_support.rs:75-112`, with one-line delegates elsewhere
(`crates/test-fixtures/src/lib.rs:763-764`).

**N13 — Path spelling/case predicates, digest spelling and glob-compile options are already
leaves.** `girpr-util-path`, `girpr-filter`, `girpr-digest`, `gircap-hash`; F13 concerns only the
two-sided existence rule (prior report 08 `N7`-`N10` remain in force).

**N14 — The product's `RepairDiagnostics` field set and capture's matrix/budget/require-class
semantics stay local.** `src/lib.rs:133-137,185-197`; `tools/capture/src/select/classes.rs:1-41`,
`select/budget.rs:1-12`, `docs/00-spec.md:275-278,781-796`.

**N15 — Cache codec/schema/publisher and `FileRec.kind` are closed decisions.** Literal-byte pin,
`FMT_V2`, `CACHE_VERSION`, closed `kind` registry (`crates/sync-cache/src/wire.rs:28,118-140`,
`schema.rs:22,41-63`), one-syscall gaps and `same_spelling` (`tools/sync/DESIGN-NOTES.md:426-448`,
`:749-818`). F28 does not reopen them.

**N16 — The offline mock, artifact replay server, fetch scripting helper and gircap's server
encode different contracts.** `crates/test-fixtures/src/lib.rs:3-13,53-57`,
`crates/fetch/src/test_http.rs:1-12`, `tools/capture/test-support/src/lib.rs:11-18`. Only the
shared listener/journal leaf exists; F49 is a narrow extension of that mechanism, not a merge.
