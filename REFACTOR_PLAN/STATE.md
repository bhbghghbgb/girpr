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
| T01 | Purge parallel map → rayon | completed | aa091ace | Deviation: eight-thread ceiling dropped (unpinned perf heuristic); rayon global pool sizes itself. |
| T02 | Repair futures combinators → futures-util | completed | 9af85115 | Deviation: added per-fan-out Semaphore (FuturesOrdered polls all; ManifestBudget caps at 4 only); both permits acquired inside each fetch. |
| T03 | Salvage order → petgraph | skipped | | Byte-level safety proof tied to one global write direction + disjoint destinations; a graph order cannot re-establish it without changing correctness-critical src/repair/file/** byte movement (non-finding). Durable: temp.rs has no salvage-order consumption (R6 consumer list stale); disjointness premise fails in Tier::Ordered — latent risk if revisited. |
| T04 | Root test origin server → axum | completed | e51d485c | Stale hints: manifest setup ~388–401 (not 554–865); metadata.rs tests are a second server user but needed no edits (API kept). Drop now sends a watch stop signal to cancel detached axum handlers. Removed only the accept-loop lifecycle test + wait_until_idle. |
| T05 | CLI help snapshots → trycmd + strip-ansi | completed | c990d1bf | trycmd 1.2.1 inline snapshot at tests/cmd/help.trycmd (no .stdout sidecar needed); harness uses default_bin_path; --jobs test adapted to clap introspection. T06 seam untouched. |
| T06 | Process assertions → assert_cmd | completed | e0e8aa96 | Async seams intentionally stay tuple-based (run_binary, run_binary_with_closed_stdout, run_data_plane_as, run_audio_refusal, repair_machinery inline spawn) — they share a mock-server runtime or need closed stdout. |
| T07 | Structured JSON log capture | completed | e990678b | Deliberate skip of repair_machinery.rs:318–342 — it reads the shipped file log (no level/token workaround); that report pointer is stale for this finding. |
| T08 | INI parser/merger → rust-ini | completed | ff782018, 8578f13e | Decision: read_game_version stays a Starward-parity last-match raw line scan (Ini::get is first-match + trims whitespace — would break pinned behavior); later version-parsing work must keep it. rust-ini load_from_str_noescape (default escape corrupts Windows paths); policy layer does quote/comment pre-pass, last-wins duplicate keys, verbatim render. |
| T09 | URL assembly & percent-encoding | completed | 22e7f073 | Decision recorded in Persistent context below: literal brackets + %20, exact AsciiSet. Deviation: query_pairs_mut().append_pair not used for the literal key; push_pair/with_param remain (now append library-encoded values). Report lines 822–823 pointer is stale (permissive versions, not host checks). |
| T10 | Small app wire helpers → serde-aux/quick-xml/base64/hex | completed | d2e830d6 | Durable: both padded and unpadded base64 occur in checked-in key XML — decode_base64 keeps STANDARD as fallback to STANDARD_NO_PAD; later base64 delegation must retain it. serde-aux pinned default-features=false; quick-xml needs features=["serialize"]; xxhash-rust direct dep for finish(); parse_hash_hex error text kept. |
| T11 | FNV-1a hashing → fnv | completed | 7a3d9385 | FnvHasher constants/byte order identical to local loops: lock filenames preserved (no invalidation). Shared bundle FORMAT bumped 3→4 once because bundle_source_key hashes this edited src/lib.rs. |
| T12 | Protobuf → prost decode + prost-build codegen | pending_review | 7e9ec7b3 | §APP-8 lines 359–369, §LEAF-4 lines 451–465. Deviation: upstream field 6 is `unknown` but the frozen dump key is `unknown_field6`; vendored pinned upstream proto carries one documented rename, tag 6 (0x30, int64) and misspelled `chuncks` byte-unchanged. protoc from protoc-bin-vendored 3.3.0 via Config::protoc_executable (no env::set_var); generated types flat re-exported with Eq/Hash additive and serde via type_attribute; no serde_json/preserve_order. Frozen keys/wire layout held by retained tests + new dump-key regression. |
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
| A | T01, T02 | done | Both completed; no fix or revert; endorsed F01, F02. |
| C | T04 | done | Completed; no fix or revert; endorsed F03. Confirmed F02 race is pre-existing, not a T04 regression. |
| D | T05, T06 | done | Both completed; no fix or revert; endorsed F04, dropped the async predicate-wrapper suggestion. |
| E1 | T07, T08 | done | Both completed; one comment-only fix (8578f13e); endorsed F05, dropped the conditional downloaded_chunks and single-quote suggestions. |
| G | T09 | done | Completed; no fix or revert; spelling decision verified consistent in every product. Endorsed F06. |
| H1 | T10, T11 | done | Both completed; no fix or revert; endorsed F07, dropped the fixture-module suggestion. |

## Follow-ups

Follow-ups are non-blocking suggestions from review sessions; the orchestrator records
them here. Reviewers decide which are worth doing and may prune them to avoid run-away.
A follow-up planning review batches the survivors only after every task is settled.

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Update docs/04-scope-limitation-spec.md:727 wording from "scoped worker" to "rayon worker" after purge delegate | open | Docs-only; endorsed by review A. |
| F02 | Investigate intermittent missing blocking log line in a_local_build_tag_mismatch_degrades_to_the_fallback; logs_of / spawn_blocking_traced dispatcher capture | open | Low priority; pre-existing, reproduced on pre-T04 parent and single-thread-clean; T04's extra tasks raise its frequency. Endorsed by reviews A and C. |
| F03 | Type scripted statuses as axum::http::StatusCode instead of Reply::Status(&'static str) + local string-to-code conversion | open | Purely type-safety cleanup; behavior correct. Endorsed by review C. |
| F04 | Add a trycmd case for short -h output (distinct clap rendering path from --help) | open | Low priority, non-blocking. Endorsed by review D. |
| F05 | Add a typed CapturedLogRecord wrapper over the JSON capture Value surface | open | Low priority, non-blocking; removes stringly-typed field assertions. Endorsed by review E1. |
| F06 | Consolidate duplicated QUERY_VALUE_SET (fetch/hyp.rs, capture/biz/mod.rs) into a shared leaf | open | Non-blocking; prevents fetch/capture drift against the standing game_ids[]/%20 decision. Endorsed by review G. |
| F07 | Add a focused crypto.rs unit test for decode_base64 covering padded, unpadded and whitespace-containing inputs | open | Non-blocking; directly pins the CRITICAL STANDARD fallback and whitespace filter. Endorsed by review H1. |

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
- **`game_ids[]` spelling — DECIDED IN T09 (§X2 lines 90–109, §APP-2 lines 281–294):**
  send `game_ids[]` literally (NOT `%5B%5D`) and spaces as `%20` (NOT `+`). Literal
  spelling is byte-stable and observed by mock routing and recorded request fixtures;
  use `percent-encoding` with an exact `AsciiSet`, not `Url`'s form serializer. Every
  later URL task (T14/T15/T27/T28 and follow-ups) must follow this; fetch and capture
  must not diverge.
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
