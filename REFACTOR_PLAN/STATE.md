# Refactor checklist state

Single source of truth for orchestration. Updated by the orchestrator in commits
separate from implementation commits. See `ORCHESTRATION.md` for the protocol and
`README.md` for ordering, checkpoints and coverage.

Initial state: no task started, no reviews done, no follow-ups.

## Tasks

Status values: `todo`, `in progress`, `pending review`, `completed`, `reverted`, `skipped`.

| ID | Task file | Status | Commits | Notes |
| --- | --- | --- | --- | --- |
| T01 | `tasks/T01-chunk-cost-arithmetic.md` | completed | 4 commits: chunk-cost primitives; repair-plan/chunk-verify adoption; gircap manifest totals; repair scan/plan | CP1 kept; unused import fixed in review |
| T02 | `tasks/T02-chunk-layout.md` | completed | 4 commits: chunk-layout leaf; repair-plan tiling adoption; gircap range/tiling adoption; repair/fetch reuse checks | CP1 kept |
| T03 | `tasks/T03-xxh64-hex-boundary.md` | completed | 4 commits: digest XXH64 conversions; persistent-plan/fetch adoption; cache-json hex wrappers; chunk-identity normalize_hex re-export | CP1 kept |
| T04 | `tasks/T04-digest-internals.md` | completed | 3 commits: restartable sink unification; Md5/Sha256 templating; shared buffered hash/copy loop | CP1 kept |
| T05 | `tasks/T05-retryable-status.md` | completed | 3 commits: shared retry predicate; fetch adoption; capture-http adoption | CP2 kept |
| T06 | `tasks/T06-async-retry-driver.md` | completed | 3 commits: retry-driver leaf; fetch loop adoption; capture-http loop adoption | CP2 kept |
| T07 | `tasks/T07-bounded-join.md` | completed | 4 commits: async-join leaf; capture-http ordered join; repair adoptions; test-helper downcast | CP2 kept |
| T08 | `tasks/T08-ledger-cas-helpers.md` | completed | 3 commits: counter CAS primitives; meter charge/refund/settle; RAM reserve/release | CP3 kept |
| T09 | `tasks/T09-whole-file-verify.md` | completed | 3 commits: shared whole-file gate; repair check/temp adoption; persistent-plan/fetch/sophon adoption | CP3 kept |
| T10 | `tasks/T10-bounded-byte-io.md` | completed | 3 commits: util-hash bounded read promotion; util-io leaf; repair file writer adoption | CP3 kept; clippy drop fix in review |
| T11 | `tasks/T11-clear-readonly.md` | completed | 4 commits: util-path readonly leaf; fetch/persistent-exec adoption; file/mode propagation; leaf test formatting | CP3 kept |
| T12 | `tasks/T12-request-url.md` | completed | 5 commits: request-url grammar leaf; fetch Sophon URLs; capture biz/probes; dispatch/persistent-plan path append; fixture query lookups | CP4 kept |
| T13 | `tasks/T13-line-and-bom-readers.md` | completed | 1 commit: line-reader leaf and parameterized audio/ini renderings | CP4 kept |
| T14 | `tasks/T14-claim-publish.md` | completed | 3 commits: util-path claim primitive; cache-json import adoption; sync backup/case-rename adoption | CP4 kept |
| T15 | `tasks/T15-fs-path-helpers.md` | completed | 7 commits: metadata classification; repair stat-site adoption; fs-names device/separator helpers; persistent-plan scope/Md5; girsync prefix helpers; clippy cleanup; disk-question seam | CP5 kept |
| T16 | `tasks/T16-global-cli-and-format.md` | completed | 2 commits: report-core OutputFormat; cli global flags and process shims | CP5 kept; clippy bound fix in review; before T18 |
| T17 | `tasks/T17-subscriber-and-log-internals.md` | completed | 7 commits: log subscriber_for; sync/capture/girpr adoption; log internals; ANSI default fix; docs | CP6 kept |
| T18 | `tasks/T18-report-core.md` | completed | 4 commits: report-core record/sink; sync/report/capture plane adoption | CP6 kept |
| T19 | `tasks/T19-test-fixture-helpers.md` | completed | 1 commit: fixture helper consolidation and cache-names leaf | CP7 kept; comment fix in review |
| T20 | `tasks/T20-test-http-servers.md` | completed | 4 commits: test-http axum mechanics; cli argv helper; integration binary runner; junction fallback helper | CP7 kept; deferrals F50/F51 accepted; comment fix in review |
| T21 | `tasks/T21-hyp-wire.md` | completed | 3 commits: hyp-wire leaf; fetch envelope/identity adoption; capture identity/envelope adoption | CP8 kept |
| T22 | `tasks/T22-capture-emit-stores.md` | completed | 4 commits: capture serialize/gzip bodies; bounded-store mechanics; schema-version read; zstd error formatter | CP8 kept |
| T23 | `tasks/T23-girsync-heartbeat.md` | completed | 3 commits: heartbeat leaf; scan adoption; effective phase adoption | CP9 kept |
| T24 | `tasks/T24-workspace-inheritance.md` | completed | 1 commit: workspace package/dependency inheritance across 53 members | CP9 kept |

## Reviews

| Checkpoint | Reviews | Status | Outcome |
| --- | --- | --- | --- |
| CP1 | T01–T04 | done | all kept; T01 unused-import defect fixed in review |
| CP2 | T05–T07 | done | all kept; no fixes or reverts |
| CP3 | T08–T11 | done | all kept; T10 clippy drop fix in review |
| CP4 | T12–T14 | done | all kept; no fixes or reverts |
| CP5 | T15–T16 | done | T15 kept; T16 kept with clippy bound fix |
| CP6 | T17–T18 | done | all kept; no fixes or reverts |
| CP7 | T19–T20 | done | both kept; stale comments fixed; T20 deferrals accepted |
| CP8 | T21–T22 | done | all kept; retcode default concern verified behavior-equal |
| CP9 | T23–T24 | done | all kept; no fixes or reverts |

## Follow-ups

Status values: `proposed`, `accepted`, `declined`, `done`.

Status values: `proposed`, `accepted`, `declined`, `done`. Accepted items are grouped into batches B1–B7 below.

| ID | Source task | Recommendation | Status |
| --- | --- | --- | --- |
| F01 | T01 | Fold remaining i64→u64 clamps in `src/repair/reuse.rs`, `src/repair/file/{mod,temp,inplace}.rs` (incl. `staged_payload` total) and `tools/capture/src/{select/mod,select/tests,blobs/props,assemble/assembly}.rs` through chunk-cost primitives; overlaps other task boundaries. | done |
| F03 | T02 | Give capture's A11 `Layout::of` a typed-problem adapter: adjacent-previous diagnostic semantics differ from the coverage cursor and its strings are published in `verify/selfcheck.json`, so it stayed local by contract. | declined |
| F04 | T02 | Decide explicitly what an unrepresentable signed chunk end means for tiling; the fold clamps at `i64::MAX` (matching capture's old behaviour) and plan-time validation rejects those ranges before layout. | declined |
| F05 | T04 | Evaluate composing `tools/capture/hash`'s `HashingFile::write` with the shared sink while keeping write-before-hash ordering at the call site. | declined |
| F06 | T05 | Optionally collapse the thin delegating wrappers (`fetch::retryable`, `RetryPolicy::should_retry_status`) into direct `girpr_retry::should_retry_status` calls; kept local to preserve the published `http` module surface and test seams. | declined |
| F07 | T06 | Optionally collapse `RetryPolicy::backoff()` / `BackoffPolicy::build()` schedule wrappers into direct `girpr_retry::Schedule::exponential(...)` calls once test seams are reviewed; kept local to preserve published module surfaces. | declined |
| F08 | T06 | If in-place (not by-value) state carry is ever needed in capture, consider an HRTB/boxed-future variant of the driver; not worth the per-attempt allocation now. | declined |
| F09 | T07 | Repair phases still render `Failed(JoinError)` with their own `Display` templates; switching to the payload-aware `join_failure` would change published error text and needs an explicit wording decision. | declined |
| F10 | T07 | `inplace.rs::bounded_window` stays local: an ordered buffered window with no index or panic recovery and file-local reservation semantics. | declined |
| F11 | T07 | Collapsing `spawn_blocking_traced` and the file driver's setup wrappers further would bury the load-bearing "capture at spawn time" documentation and the diagnostics-owner hook. | declined |
| F12 | T07 | `LocalRead::join_failure` in `src/repair/phases/metadata.rs` changed its cancellation wording to "the {label} task was cancelled: {JoinError}"; no test/golden asserts the old text — decide explicitly if stderr wording is a published contract (with F09). | declined |
| F13 | T07 | `src/repair/persistent_exec.rs` now gives a lost task a policy (`files_failed += 1`, fail-fast abort) where the old loop discarded results; confirm this deliberate behavior addition, covered by tests. | declined |
| F14 | T08 | Consider an explicit leaf primitive returning the "within ceiling" verdict so `settle` need not pass the `u64::MAX` sentinel; current form is deliberate to keep the settled-limit rule in `Meter`. | declined |
| F15 | T09 | `src/repair/file/temp.rs::promote_tmp` still has a hand-written length+MD5 final gate with its own delete-and-bail policy; could be routed through the shared gate if a single implementation is wanted there. | done |
| F16 | T09 | Consider relocating the pure whole-file gate to a lower crate; `app/persistent-plan` now depends on `girpr-chunk-verify` (transitively proto/util-hash) for a gate that needs only `std`. | declined |
| F17 | T10 | `ChunkFileSink::write` in `temp.rs` is a decoder-fed sink, not a loop; its per-piece ceiling charge stayed local — a shared charge helper would touch the `sophon::ChunkSink` seam and retry/refund tests. | declined |
| F18 | T10 | Consider relocating `read_up_to`/`reserve_exact` into `util-io` to make the leaf dependency-free (currently `util-io` depends on `util-hash` for one read); moves a public primitive between crates. | declined |
| F19 | T10 | `capture_bounded` downcasts `read_up_to`'s `anyhow::Error` back to `io::Error`; an `io::Result`-typed read primitive would remove the downcast but needs an explicit decision on reserve-failure wording. | declined |
| F20 | T11 | Add an injectable seam to force a `set_permissions` failure so the `SetFailed` warning branches can be covered directly. | accepted (B5) |
| F21 | T11 | Consider relocating the `clear_readonly` leaf to a `std`-only crate; `girpr-util-path` carries dependencies the mutation needs none of. | declined |
| F22 | CP3 | Update the README shared-leaf table for `app/util-io`, `util-path::readonly` and the whole-file gate (the util-counter row was updated in T08). | accepted (B1) |
| F23 | T10 | A truncated in-file copy now reports the leaf wording ("in-file copy ended after X of Y bytes") instead of `read_exact`'s message; still a hard error and no contract pins the string. | declined |
| F24 | T12 | Add a `girpr-request-url` row to the README shared-leaf table (same class as F22). | accepted (B1) |
| F25 | T12 | Consider a policy-parameterized HYP URL seam to unify fetch's and capture's HYP builders; deliberately not done because girpr's hardcoded/unencoded language spelling and capture's `needs_channel`/override policy are call-site contracts. | declined |
| F26 | T13 | Add a `girpr-line-reader` row (and note the new audio-language/util-ini helpers) to the README shared-leaf table, alongside F22/F24. | accepted (B1) |
| F27 | T13 | Consider relocating the pure `game_version_from_text` scan into a logic-free leaf so gircap no longer depends on an `app/` crate that also owns the repair config writer. | declined |
| F28 | T13 | Consider an explicit policy type for the two alias spellings if a third scan-file reader appears; today `app/audio` owns them as a local const. | declined |
| F29 | T14 | Consider relocating the claim primitive from `app/util-path` to a lower-level `crates/` leaf; three `tools/sync` crates now depend on it (and its Windows `junction` dependency) for a small wrapper. | declined |
| F30 | T14 | `girsync-cache`'s `#[cfg(test)] claim_sibling` still spells the published `_2`/`_3` name family by hand — a deliberately different family from the temp sibling; decide explicitly whether consolidation is wanted. | declined |
| F31 | T14 | `tools/sync/Cargo.toml`'s updated `tempfile` comment says it is only for unit tests, but the dependency stays in `[dependencies]` (pre-existing placement); consider moving it to `[dev-dependencies]`. | done |
| F32 | T15 | Consider a std-only home for `native_separator`/`forward_slash`: `girpr-tempdir` and `girpr-filter` now take a `girpr-fs-names` (dunce-backed) edge for a pure string rewrite. | declined |
| F33 | T15 | Decide whether `util-path` should keep re-exporting `fold_ascii_case`/`forward_slash`, or `persistent-plan` should depend on `girpr-fs-names` directly. | declined |
| F34 | T15 | `MetadataFact::is_file`'s "stat failed → in-place capable" default restates `girpr-repair-policy::resolve_mode`; consider one home for that rule if it ever changes. | declined |
| F35 | T16 | `global_args!` has two arms duplicating the five field declarations; a tt-muncher could reduce this to one textual declaration if strictly required. | declined |
| F36 | T16 | Capture's `FromStr` wording is now byte-identical but defined in `girpr-report-core`; if strict locality is wanted, replace it with a capture-local parse helper. | declined |
| F37 | T16 | `girpr-cli` now links `girpr-log` and `girpr-report-core`; consider whether re-exporting `Level`/`OutputFormat` from it is the preferred seam versus direct imports. | declined |
| F38 | T16 | The `clap` feature on `girpr-report-core` (and forwarding features on app/report, tools/sync, tools/capture) is now unused since `--output` uses `PossibleValuesParser`; could be retired later. | declined |
| F39 | T16 | girpr's `--output` was previously `global = true`; the shared macro emits `global = false` — behavior-neutral today (no subcommands) but note if subcommands are added. | declined |
| F40 | T17 | Consider an explicit `AnsiChoice` enum instead of `Option<bool>` if another program needs "formatter default" semantics. | declined |
| F41 | T17 | If a third file format appears, a caller-supplied formatter factory would avoid another cfg-gated enum arm in the shared function. | declined |
| F42 | T17 | `CaptureBuffer` and internal `SharedBytes` remain two byte sinks by design (public feature surface vs crate-private helper); revisit only if the feature surface is intentionally changed. | declined |
| F43 | T18 | Update README.md:469-471 and :564 plus the report-core crate's Cargo.toml description — the crate now owns the record container and emit sink, not just ordered-JSON/line-writing mechanics. | accepted (B1) |
| F44 | T18 | If a fourth report plane appears, decide whether an explicit policy enum in the core is preferable to returning `io::Result`; today's three broken-pipe policies are deliberately unshared. | declined |
| F45 | T18 | Consider promoting the core `Record`'s "skip a field named like the event key" rule to a named helper if more than capture relies on it. | declined |
| F46 | T19 | Production still spells `CACHE_PREFIX`/`has_cache_prefix`/`is_cache_name` itself; adopting the new leaf there was deferred under the tests-only guardrail — worth a dedicated task so the prefix has one home. | accepted (B6) |
| F47 | T19 | `app/lock::field` stays local (scans whole `key=value` lines, unlike whitespace-token `field_token`); consolidation needs an explicit decision on values containing spaces. | declined |
| F48 | T19 | Capture-side `read_json`/`write_json` helpers use generic deserialization and different error policies; left local by contract. | declined |
| F49 | T19 | `ctx_default` bakes `jobs = 1` and `backoff = none()`; a parameterized variant is only worth adding if a third fixture needs different defaults. | declined |
| F50 | T20 | CAP-15 raw-scripted reply mode deferred: the three raw-socket servers have deliberately different policies; a dedicated task could add a parameterized raw-reply helper and migrate only the malformed shape. | declined |
| F51 | T20 | CAP-16/test-fixture axum scaffolds deferred: capture's synchronous `Mock::start` and the artifact replay's custom accept loop need a base-aware router factory and blocking-serve variant; byte-exact body/length pins at risk. | declined |
| F52 | T20 | Consider re-exporting `girpr-test-http` from a shared test-support surface if a third axum server adopts it. | declined |
| F53 | CP7 | Add README shared-leaf table rows for `girpr-cache-names`, `girpr-test-http` and report-core's `test_support` scanner (same class as F22/F24/F26). | accepted (B1) |
| F54 | T19 | `entry_names_with` uses `filter_map(e.ok())` where the retired local copies used `e.unwrap()`; unreadable entries are now silently skipped — decide whether strictness should be preserved. | accepted (B7) |
| F55 | T21 | Add a `girpr-hyp-wire` row to the README shared-leaf table (same class as F22/F24/F26/F53). | accepted (B1) |
| F56 | T21 | Consider routing capture's `Fetcher::get_json` retcode/message extraction through a shared head helper; it reads `serde_json::Value` directly because returning the full `Value` is part of the published contract. | declined |
| F57 | T21 | Consider moving fetch's `ManifestFile` sizes to the leaf's `IntOrStr` if a preserve-form consumer appears; deliberately not done because downstream sizes are `i64`. | declined |
| F58 | T22 | `bounded_store` is a public module of `gircap-http` solely because two crates need it; reconsider a dedicated gircap-owned mechanics crate if a fourth store appears. | declined |
| F59 | T22 | The count-first driver still walks the document twice (`json_len` then write); unifying it with the counted driver needs fresh performance and byte-equality validation. | declined |
| F60 | T22 | `gzip_from_reader` has one production caller; leave it until a second file-backed gzip path appears rather than changing a driver's measured behaviour. | declined |
| F61 | T21 | The shared number decoder trims whitespace on string-encoded numbers whereas serde-aux did not — a relaxation; no current field exercises it. Decide whether to pin the stricter behavior. | declined |
| F62 | T21 | Capture's `Wrapper<T>` alias field `message` changed from `String` to `Option<String>`; no consumer reads it, but it is a public surface change worth an explicit decision. | declined |
| F63 | T23 | Add a `girsync-leaves::heartbeat` row to the README shared-leaf table (same class as F22/F24/F26/F53/F55). | accepted (B1) |
| F64 | T23 | Re-export `heartbeat` from the girsync lib facade only if an external consumer needs it; today all three callers import the leaf directly. | declined |
| F65 | T24 | Decide whether the two `tests/scripts/io-profile/*` helper crates (edition 2021, outside the workspace, own `redb` pin) should join inheritance; deliberately left untouched. | declined |
| F66 | T24 | Path dependencies remain local (versionless, with call-site comments); centralizing them in `[workspace.dependencies]` is a separate decision. | declined |
| F67 | T23 | Progress events' `elapsed_s` now originates at `Heartbeat` construction rather than the earlier `t_eff`/`t0`, making it marginally smaller and not sharing the phase's "done" zero; no test/golden asserts the value — consider a caller-supplied start `Instant` if strict parity is wanted. | declined |

## Accepted batches

| Batch | Follow-ups | Scope | Ordering |
| --- | --- | --- | --- |
| B1 | F22, F24, F26, F43, F53, F55, F63 | Backfill the README shared-leaf table and fix the report-core description/Cargo.toml. | Do last, after B2–B7. |
| B2 | F31 | Move `tempfile` to `[dev-dependencies]` in `tools/sync/Cargo.toml`. | Independent. |
| B3 | F01 | Fold remaining i64→u64 clamps through chunk-cost primitives. | Before B4. |
| B4 | F15 | Route `promote_tmp`'s final length+MD5 gate through the shared gate. | After B3. |
| B5 | F20 | Add an injectable `set_permissions` failure seam for `SetFailed` tests. | Independent. |
| B6 | F46 | Adopt `girpr-cache-names` in production call sites. | Independent. |
| B7 | F54 | Restore strict handling of unreadable directory entries (defect). | Independent. |

## Persistent context

- Report: `docs/09-shared-code-refactor-findings.md` (temporary, deleted at finish).
- Every finding is mapped to a task in `README.md`; non-candidates are listed there.
- LF-14's cross-reference to X14 is a pointer, not a grouping; LF-14 is in T17.
- Findings are static readings; implementers must validate against current code.
- These plan files and the report must not be referenced from code or permanent docs.
- Report line numbers are shifted relative to the current tree; implementers validate named sites against code, not cited lines.
- F01's remaining single i64→u64 clamps overlap other task boundaries; they were left local in T01.
- T01 and T02 implemented and validated (see task table); awaiting CP1 review.
- CP1 note: T02 reports a leftover unused `saturating_nonneg_sum` import in `src/repair/scan.rs` from T01; reviewer should fix it as a T01 defect.
- T02's tiling leaf consumes T01's `checked_end` (F02 done) and clamps unrepresentable signed ends at `i64::MAX`, matching capture's prior behaviour.
- T03 implemented and validated; no follow-ups proposed. Remaining `{:016x}` sites in gircap are opaque manifest-id prefixes, not hashes.
- T04 implemented and validated; `Sha256::try_parse_hex` is additive and public paths are unchanged.
- CP1 done: T01–T04 kept; the T01 unused `saturating_nonneg_sum` import was scoped to the test module in review. Follow-ups F01/F03/F04/F05 remain open.
- CP1 minor observations, no action taken: a redundant `RangeProblem::NegativeOffset` match arm in `manifest/proto.rs`; `hash_file_with_buf` opens the file before the leaf's empty-buffer check, so an empty-buffer-plus-missing-file error reports the open failure.
- T05 finding: the report's out-of-range-u16 discrepancy claim does not hold in this tree (http 1.5.0 `from_u16` accepts 100..=999 and `is_server_error` is 500..600); the shared 500..=599 rule preserves prior behavior exactly.
- T06 note: the report's 897-898 "retry drivers stay local" note does not block the task — the new leaf holds no classification or side effects, and jitter/logging/error types stayed in the products.
- T06 observation: clippy reports one warning in `crates/chunk-layout` (T02's crate, already reviewed at CP1); not re-opened.
- T07 guardrails held: capture's buffered-future panics and repair's `Handle::block_on` cancellation remain distinct; dispatcher and `DiagnosticsOwner` survive via the thread-setup hook; published fallback strings unchanged.
- CP2 done: T05–T07 kept; no fixes or reverts; workspace 2297 passed / 8 skipped; only the out-of-batch chunk-layout clippy warning remains.
- T08 keeper: the remaining `stopped_by` CAS in `app/report/src/ledger.rs` is an idempotent first-wins flag, not a saturating tally; routed through no leaf deliberately.
- T09 keeper: `src/repair/file/inplace.rs` scanned-verdict reuse digest comparisons remain local `matches_hex` calls by the reuse-guard contract.
- T10 boundary: `inplace.rs` was not edited — it already calls the `mod.rs` wrappers that delegate to the leaf and downloads through `temp.rs`, so it consumes both mechanics transitively.
- CP3 done: T08/T09/T10/T11 kept; T10's `drop(rw)` calls removed for clippy; only the out-of-batch chunk-layout `collapsible_if` warning remains when clippy runs with -D warnings.
- T12 keeper: CAP-06 blob-id shape helpers left local by guardrail — tightening girpr to capture's strict rule risks rejecting real ids.
- T13: BOM-only behavior and trailing-newline contracts remain opposite by explicit parameters; capture's Cargo.toml now states the newline distinction; `game_version_from_text` takes a `value_of` closure to keep anchoring/empty-value/error wording at call sites.
- CP4 done: T12/T13/T14 kept; no fixes or reverts; workspace 2332 passed / 8 skipped; only the out-of-batch chunk-layout clippy warning remains.
- T15: `Absent` carries the original `NotFound` error; `util-hash`'s `Md5` re-export left intact; prefix helpers imported directly from `girsync_leaves::link` so the disk-question scanner still sees only real disk calls.
- T16: shared CLI uses an item-level `global_args!` macro (two arms) rather than a flattened struct to preserve each binary's help order and public struct literals; --help/exit bytes verified identical against the pre-T16 tree.
- CP5 done: T15 kept; T16 kept after removing redundant parser bounds; workspace 2339 passed / 8 skipped; only one trycmd help golden (`tests/cmd/help.trycmd`, girpr --help) exists; sync/capture assert help semantically.
- T17: `subscriber_for` takes `ConsoleSpec<W>`, `FileFormat`, `SpanFields` and `open_file`; `FileFormat::Json` behind a new girpr-log `json` feature used by the tools; girpr passes `ansi: None` to preserve its NO_COLOR behavior.
- T18: broken-pipe policy stays parameterized by the caller via `io::Result`; each plane keeps its own `Val` vocabulary, layout, field order and event derivation.
- CP6 done: T17/T18 kept; no fixes or reverts; workspace 2345 passed / 8 skipped; clippy clean when allowing the out-of-batch chunk-layout warning.
- T19: `girpr-cache-names` owns the Windows ASCII-case-fold rule; `entry_names_with` parameterizes the `girpr-cache*` predicate; `CARGO_BIN_EXE_girpr` remains integration-only; test roots unchanged.
- T20 boundary: only axum socket/stop/gate mechanics moved; the three server contracts and per-test reply policies stayed distinct; `argv_with_program` takes the sentinel as a parameter and the binary runner stayed in integration roots.
- CP7 done: T19/T20 kept; two stale comments fixed; workspace 2348 passed / 8 skipped; T20 deferrals F50/F51 accepted as justified follow-ups.
- T21: chose a new `girpr-hyp-wire` leaf over extending `crates/proto` (protobuf tags and JSON envelopes are different wire contracts); shared `Envelope.retcode` is required as fetch's was — capture's former `#[serde(default)]` was unreachable because `get_json` rejects a body with no retcode first, and no test/golden covers it (flag for CP8).
- T22: `bounded_store` lives in `gircap-http` (shared parent dependency) because `error_bodies.rs` is there and `gircap-http` cannot depend on `gircap`; BufWriter construction stayed at call sites.
- CP8 done: T21/T22 kept; the flagged `Envelope.retcode` default concern was verified unreachable and behavior-equal; workspace 2361 passed / 8 skipped.
- T23: `Heartbeat<F>` owns only the clock/trigger; `elapsed_s`, wording, fields and levels stay at the three call sites.
- T24: report's "39 members" is stale — the tree has 53 workspace members; all 58 external dependency versions inherited, lockfile and resolved graph byte-identical; tools' reduced lint tables and `[lib]` sections stayed local.
- CP9 done: T23/T24 kept; no fixes or reverts; all 24 tasks are now terminal; workspace 2366 passed / 8 skipped.
- Follow-up triage done: 13 accepted (B1–B7), 53 declined, F02 removed as stale/no remaining work (it merely recorded that T02 consumed checked_end); no new follow-ups.
- F54 is the only triaged defect: `entry_names_with` silently skips unreadable directory entries where the retired local copies failed loud; accepted as B7. All other accepted items are enhancements/hygiene.
- F59 was omitted from the triage's declined list; declined by the orchestrator by analogy with F60 (unifying the count-first driver needs fresh performance/byte-equality validation, no current need).
- B2 done: F31 kept in review; `tempfile` moved to dev-dependencies, lockfile unchanged, 464 tests passed.
- B3 done: F01 kept in review; all named clamp sites now use chunk-cost primitives except those already folded or deliberately not clamps; workspace 2366 passed / 8 skipped.
- B4 done: F15 kept in review; `promote_tmp` routes through `verify_whole_file` with delete-and-bail and all published strings unchanged; workspace 2366 passed / 8 skipped.
