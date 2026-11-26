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
| T12 | Protobuf → prost decode + prost-build codegen | completed | 7e9ec7b3 | §APP-8 lines 359–369, §LEAF-4 lines 451–465. Deviation: upstream field 6 is `unknown` but the frozen dump key is `unknown_field6`; vendored pinned upstream proto carries one documented rename, tag 6 (0x30, int64) and misspelled `chuncks` byte-unchanged. protoc from protoc-bin-vendored 3.3.0 via Config::protoc_executable (no env::set_var); generated types flat re-exported with Eq/Hash additive and serde via type_attribute; no serde_json/preserve_order. Frozen keys/wire layout held by retained tests + new dump-key regression. |
| T13 | Junction creation → junction crate | completed | e7160155 | §APP-6/§LEAF-3. Actual crate signature is `junction::create(target, link)`; §LEAF-3 line 447 hint (`link, target`) was wrong and not copied — new test pins order via `junction::get_target`. junction 2.1.0 newly locked as `[target.'cfg(windows)'.dependencies]` with default-features=false in both manifests; no non-Windows import; default feature `unstable_admin` disabled to keep ordinary privilege path. Classifier/fixture policy byte-unchanged; §APP-6 hint that path.rs:1218 / ini.rs:1188 need new error kinds did not hold (public seam unchanged; both pass unchanged). Dangling-target premise held. Windows 66/66 + symlink 34/34 pass; Linux-target check fails only at pre-existing atomic.rs:156 `tracing::warn!` unrelated to this change. |
| T14 | Artifact-server fixtures → wiremock | skipped | | Validated skip, no code changed. T09 already removed query_value/urldecode in favour of url::form_urlencoded::parse (artifact_server.rs:984; report :1063–1098 and lib.rs:70–76 stale). Real journal consumers are 55 synchronous call sites in 10 root test files, not app/fetch/tests*/app/audio/tests*. Blocker outside the authorized truncation/on_first_chunk carve-out: config_after_branches (lib.rs:1356–1361) is a deterministic cross-request gate and wiremock's Respond is synchronous on a single-thread runtime (blocking deadlocks; fixed delay weakens pinned behavior in repair_offline.rs, §A5). Chunk route's lying-length truncation is unexpressible as Response<Full<Bytes>>; received_requests() is async (would force all 55 call sites to rewrite); bridge replay server is a separate larger rewrite. Follow-up held for reviewer: drop as pre-satisfied or re-scope to bridge replay routes only. |
| T15 | ScriptedServer → wiremock | completed | 07812e2e | §LEAF-1/§X1. wiremock 0.6.5 now locked (9 new lock packages) + direct tokio 1.54.1 rt/net/time. Deviation: wiremock is a regular [dependencies] entry, not dev-dep — consumers declare the leaf as a dev-dependency, so a leaf dev-dep would be invisible to them. Last-reply-repeats via one any() mock per reply with up_to_n_times(1) on the first N-1, relying on wiremock's insertion-order priority tie-break; exact-total drop hit assertion stays in the leaf's own Drop with unchanged message (wiremock expect(n) is per-mock, not total). Journal via custom Respond pushing into Arc<Mutex<Vec<RecordedRequest>>> to keep hits()/requests() synchronous — no async received_requests() in callers. API change: RecordedRequest.headers is now wiremock::http::HeaderMap; custom Headers deleted; no downstream edits needed (get/contains_key/to_str semantics preserved). Synchronous start uses a thread::scope helper driving a fresh new_current_thread runtime. Obsolete lifecycle test removed; two leaf contract tests added. |
| T16 | strum enum tables | pending_review | 6e1ae85f | §LEAF-7/§SYNC-2. strum 0.28.0 (features=["derive"]) added to girpr-model, girpr-log, girsync-cache; exactly two new lock packages; no serde/preserve_order. #[strum(const_into_str)] keeps public const as_str() wrappers and function-pointer call sites. HashAlg manual Display ({:<12}) and Debug ("md5"-style) deliberately kept; ALL, const from_name, compile-time assertion block, names() kept. Level manual FromStr kept (trim, warning alias, exact error text) but iterates strum EnumIter. ParseError mapped via parse_err_ty/parse_err_fn = UnknownHash. Region/Level derive Display+EnumIter+IntoStaticStr; HashAlg derives EnumIter+EnumString+IntoStaticStr. Region gained no EnumString (no parse table existed). Wire bytes pinned by DigestSet literal-byte test; 154+927+679 tests passed. |
| T17 | Retry sleep hook → async-trait | in_progress | | §LEAF-6 lines 481–494; keep backoff as the schedule engine |
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
| H2 | T12, T13 | done | Both completed; no fix or revert; endorsed F08, F09; dropped the optional/feature-gated junction dependency suggestion. Verified the Linux atomic.rs:156 failure is pre-existing at the parent commit. |
| I | T15 | done | Completed; no fix or revert; endorsed F10 (widened site list). Verified the insertion-order tie-break, synchronous Respond journal and regular-dependency deviation against wiremock 0.6.5 source and 20x repeat runs. |

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
| F08 | Add a serde JSON key-set golden for every generated girpr-proto message, not only chuncks/unknown_field6 | open | Low priority, non-blocking; key-name-only golden closes frozen-dump key drift without pinning formatting (§A12). Endorsed by review H2. |
| F09 | Update tools/sync/tests/symlink.rs:12-16 doc wording from `mklink /J` to the junction::create mechanism | open | Doc-only; the common::dir_link re-export is now junction::create-driven. Other tools/sync mklink /J helpers are accurate and out of scope. Endorsed by review H2. |
| F10 | Update stale "dependency-free loopback server" wording after T15 | open | Doc-only, zero-risk; sites: docs/06-artifact-bridge-spec.md:767, tools/capture/docs/00-spec.md:42, app/fetch/Cargo.toml:71, tools/capture/Cargo.toml:286, plus root Cargo.toml:265, tools/capture/Cargo.toml:52, tools/capture/src/http/tests.rs:41,47. Endorsed by review I (widened site list). |

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
- **wiremock facts from the validated T14 skip (bind T15):** wiremock 0.6.5 is cached in the registry but absent from Cargo.lock/all manifests; its `Respond::respond` is synchronous and the server runs on a `new_current_thread()` Tokio runtime, so a blocking cross-request gate deadlocks; `received_requests()` is async; `Response<Full<Bytes>>` derives Content-Length, so a lying-length truncated body is unexpressible. T09 already replaced the artifact-server percent decoder with `url::form_urlencoded::parse`. T15 must validate its own premise against these facts, not T14's stale pointers.
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
