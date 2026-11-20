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
| T01 | Digest newtypes + hex sweep (`girpr-digest`) | completed | e3a1bc08 | F2, F4, F5 (F5 stale: girsync raw bytes, no hex). Review A: mechanics-only leaf, §A7 preserved, §A2 tables updated, no fixes |
| T02 | `ChunkIdentity` (md5, size) value type | skipped | | F27; follows T01. F27 premise contradicted by code: two distinct §A7 case-insensitive-MD5 mechanisms (capture raw normalised hex `&str` vs product parsed 16-byte `Md5` + invalid-skipped). A shared type collapses §A7 and changes behavior. Details in Persistent context |
| T03 | Windows device-name predicate leaf (`girpr-fs-names`) | completed | db8c5e37 | F6, A10; parameterized predicate, both rules unchanged; §A2 tables updated (4 leaf deps); §A10 divergence → FU-03. Review B: no changes |
| T04 | Trailing-dot/space and path-key predicates | completed | 0651c39b | F7; host policy explicit, both path_key kept; extended girpr-fs-names, girsync gained dep (no new gircap leaf); FU-04, FU-05. Review B: no code change |
| T05 | Canonicalize + `\\?\` stripping helpers | completed | ebd758b9 | F8; canonical_no_verbatim in girpr-fs-names, all girsync dunce sites migrated; capture std::canonicalize sites + girpr fail-closed routine untouched; §F8 capture/dunce pointer stale; FU-06, FU-07. Review B: no code change |
| T06 | File identity + link-count leaf | completed | 57877cac | F9, A4; girpr-fs-identity leaf, no unsafe exists (A4/F9 claim stale), path-read policy stays local per N6; no gircap dep; FU-08, FU-09. Review C: no changes |
| T07 | Atomic temp-then-publish leaf | skipped | | F10 premise contradicted by code: four sites share only tempfile; write mechanism (handle vs path), publish (persist vs claim+rename), durability, error type and lifecycle all differ; a single parameterized leaf would change behavior. girsync has no persist step. Details in Persistent context; FU-10..12 |
| T08 | Streaming hash-sink leaf | completed | f84ec7b9 | F3; girpr-digest gains Sink/Algorithms/DigestBytes/hash_reader; ordering traps kept local; girsync now depends on girpr-digest (mechanics only); no gircap dep change; FU-13. Review D: no changes |
| T09 | Bounded zstd decode primitive | skipped | | F28 medium-confidence claim contradicted by code: product decode is async write-side, capture sync read-side; ceilings/error text/result types deliberately distinct; only shared piece (hashing) already in girpr-digest. zstd_measure loops stay duplicated (N10). FU-14 |
| T10 | Retry policy/schedule leaf (`girpr-retry`) | completed | 217477de | F15, A6; jitter stays local (girpr 0.5 / gircap 0.0); backoff now leaf-only; gircap now 5 leaf deps, all five A2 doc tables updated; Review D: no changes |
| T11 | Log-install helpers + timestamp reuse | completed | 7b4ff3be | F18, F19; girpr-log gains install mechanics + format_timestamp; guard/layer stay local; DateTime seam preserved; bare --log-file reviewed acceptable (FU-16); no new gircap dep. Review E: no changes |
| T12 | CLI leaf (`OutputFormat`, `LogArgs`, clap-error mapper) | skipped | | F20 premise contradicted by code + live --help: help text and OutputFormat variant docs differ substantively across the three tools, and variant docs render into --help Possible values — sharing changes CLI bytes (A3). Macros cannot expand to struct fields. Only byte-neutral piece is the clap-error mapper (FU-19). Details in Persistent context |
| T13 | Exit-table mechanism macro | completed | 3c691e58 | F22, A9; girpr-exit leaf define_exit_table! generates mechanism only, tables/codes/strings unchanged; A2 tables updated (6 deps then, 7 after T14); FU-22. Review E: no changes |
| T14 | Audio-language normalisation primitive | completed | e67cf272 | F23; girpr-audio-language leaf, normalize_audio_token + CaseFold policy + AUDIO_LANGUAGES; alias/sentinel/validation stay local; gircap 7 leaf deps, §A2 updated; FU-23. Review E: no changes |
| T15 | Ordered-JSON / `write_line` report core | completed | bcdbfa02 | F21, A3, A8; girpr-report-core leaf (write_json_object + write_line), three value vocabularies kept distinct, no preserve_order; gircap 8 leaf deps, A2 updated; FU-24, FU-25. Solo review: no changes |
| T16 | Sophon protobuf wire types leaf (`girpr-proto`) | completed | e377f50c | F24; three wire types in leaf, capture methods kept as local extension traits, chuncks/tag6/Serialize keys byte-identical; field-6 source name collapsed to unknown_field6 (source-only); gircap 9 leaf deps, A2 updated; FU-26. Review F: no fixes — wire tags/misspelling/field 6/Serialize keys byte-identical to pre-extraction, no program coupling |
| T17 | Region model leaf (`girpr-model::region`) | completed | 92292793, 042683eb | F25, A9; leaf holds three ids + launcher/game ids + channel tuples + Host families + four base URLs + DOWNLOAD_MODE_CHUNK/CHANNEL_ENDPOINT; clap ValueEnum feature-gated (zero deps off); override seam + resolve policies stay local; needs_channel now path()==CHANNEL_ENDPOINT (was const matches!), pinned by both suites; gircap 10 leaf deps, all five A2 tables + manifest header updated; test oracles keep literal region strings (A3). Review F: fixed --help byte regression (042683eb removed leaf variant docs, restored capture's local byte-pinned clap Biz enum); region values/URLs/channel rule unchanged; FU-27..29 |
| T18 | API DTO leaf (keep both decoding strategies) | skipped | | F26 premise contradicted by code: DTO field sets do NOT match (different JSON subsets, field types, serde strictness); decoding/URL strategies deliberately distinct; capture DTOs carry program logic + byte-critical re-serialization. A logic-free leaf forces a byte/validation change or couples the programs. Details in Persistent context |
| T19 | Shared log-capture test helper | completed | 57b1bd0e | F16, A5; one public girpr-log::test_support helper (sync logs_of + async logs_of_async with CurrentThread debug assert), three copies now shims; tokio feature-gated dev-only (no A11 inversion); no new gircap leaf dep so no A2 table change; girsync SharedBuffer left local (different semantics); 638 passed/8 skipped. Review G: no fixes, contracts preserved; FU-30 |
| T20 | Shared generic test helpers | completed | ba57a704 | F17, A5, A11; temp_dir + write/read_game_file consolidated in girpr-tempdir::test_support with product shims; md5hex already a shim over girpr-digest (T01); fs2 lock judged non-duplicate (product CLI lock cleanup, no fs2) and left local; fake HTTP servers + tool fixtures excluded (N7); no new gircap dep (no A2 churn); wfile/rfile left local (Windows-separator/panic differences); 2162 passed/11 skipped. Review G: no fixes, contracts preserved; FU-31 |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | done | T01 completed (no fixes); T02 skipped — not reviewed. FU-01, FU-02 raised |
| B | T03, T04, T05 | done | All completed, no reverts/skips; fix 44dcae95 (girsync README); FU-03..07 unchanged |
| C | T06, T07 | done | T06 completed, no fixes; T07 skipped — not reviewed. FU-08/09 kept open |
| D | T08, T09, T10 | done | T08 + T10 completed, no fixes; T09 skipped — not reviewed. FU-13 kept; FU-14/15 dropped |
| E | T11, T12, T13, T14 | done | T11/T13/T14 completed, no fixes; T12 skipped — not reviewed. FU-16 assessed acceptable; all follow-ups kept |
| solo | T15 | done | Completed, no fixes; all byte oracles pass, no preserve_order; FU-24/25 kept open |
| F | T16, T17 | done | T16 completed (no fixes); T17 completed with fix 042683eb (--help byte regression); byte/wire contracts, gircap boundary and DO-NOT-TOUCH preserved; FU-27..29 raised |
| solo | T18 | done | T18 skipped — not reviewed; §F26 premise refuted by code (see Finding from T18) |
| G | T19, T20 | done | Both completed, no fixes; test independence (gircap/§A11), §N7 exclusion and byte/debug-assert/panic-message contracts preserved; FU-30, FU-31 raised |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU-01 | Add girpr-filter to cross-program "what ships" tables | batched (Batch D) | Review A §F30 doc gap (README.md:44-45,62; tools/capture/docs/00-spec.md:40-42); real gap in product docs that survives cleanup. Planning: kept, doc-only |
| FU-02 | Investigate intermittent `repair_machinery the_three_tiers_produce_the_same_file` flake under full parallel nextest | dropped | Review A: failed once, passed in isolation + clean rerun; unrelated. Planning: one-off non-reproducible flake, no repro to act on |
| FU-03 | Resolve girpr/gircap Windows device-name divergence (CLOCK$ reservation + trailing-space trim) | dropped | Raised by T03 per §A10 — defect candidate, not verdict. Planning: needs human verdict; any fix may move §A3 bundle bytes — unsafe to batch |
| FU-04 | Generalise girpr-fs-names one-line description in the five §A2 doc locations | batched (Batch D) | Raised by T04/T05; doc-only, no behaviour change. Planning: kept, keeps docs accurate |
| FU-05 | Consider documenting sync's local ascii_case_eq/ascii_lower/FoldedKey as intentionally unshared | dropped | Raised by T04; deliberately local. Planning: doc-only nicety with no value |
| FU-06 | Decide whether gircap's std::canonicalize containment sites should consolidate onto the shared helper | dropped | Raised by T05; coordinated root+child rewrite, byte-stability risk. Planning: "decide whether" with no safe implementation |
| FU-07 | Migrate girsync test oracles off direct dunce so its direct dunce dep can be dropped | dropped | Raised by T05; optional cleanup. Planning: circular oracle risk (tests helper with itself), marginal value |
| FU-08 | Correct README sentence claiming a deliberate unsafe Windows handle call in girsync's hardlink module | batched (Batch D) | Raised by T06; doc-only, unsafe claim stale. Planning: kept, cheap factual fix |
| FU-09 | Add Windows regression test for lock path swapped to symlink after open | batched (Batch T) | Raised by T06; existing swap test cfg(unix). Planning: kept, test-only, no byte risk |
| FU-10 | Re-scope atomic publish as a parameterized temp-file publish guard leaf | dropped | Raised by T07 (F07-a). Planning: expands skipped T07; note itself questions thin-wrapper value |
| FU-11 | Decide whether girsync's kept-path working-name creation should use any shared temp helper | dropped | Raised by T07 (F07-b). Planning: different primitive explicitly excluded from skipped T07 |
| FU-12 | Re-validate F10 against code before opening any atomic-publish follow-up | dropped | Raised by T07 (F07-c). Planning: meta "re-validate report" task, moot at report deletion |
| FU-13 | Consider using girpr-digest Sink/hash_reader as the digest core of capture's zstd_measure / zstd_measure_from loops | dropped | Raised by T08. Planning: §N10 keeps the loops duplicated for distinct error text; changing risks §A3 error bytes |
| FU-14 | Consider using the shared digest sink in capture's measure path | dropped | Review D: duplicate of FU-13 (same N10 constraint) |
| FU-15 | Correct report A2 claim that the capture spec table names two leaf crates | dropped | Review D: moot at final cleanup when report deleted |
| FU-16 | Consider a cheap CLI regression test for bare --log-file <name> | batched (Batch T) | Raised by T11, Review E accepted; no byte impact. Planning: kept, cheap regression test locking in accepted behavior |
| FU-17 | Consider splitting log-install mechanics into a separate leaf if girpr-log must stay minimal | dropped | Raised by T11. Planning: speculative "split if we must" with no trigger or concrete benefit |
| FU-18 | Consider normalizing capture's env-warning prefix (gircap: vs WARN) only if uniform stderr wording desired | dropped | Raised by T11. Planning: changes observable stderr bytes (§A3) |
| FU-19 | Extract only the clap-error -> usage-code mapper into a byte-neutral leaf parameterised by caller usage code (no shared exit table) | batched (Batch C) | Raised by T12 (F-T12A): preserve exit mechanics, verify codes 0/1/3 identical. Planning: kept, only byte-neutral piece of skipped T12 |
| FU-20 | If a single CLI leaf is still wanted, first get explicit decision to unify the three --output/log-flag help text | dropped | Raised by T12 (F-T12B). Planning: user-visible --help change (§A3), not a pure extraction |
| FU-21 | Reconcile the three OutputFormat types only after confirming error string and per-variant help are not contracts | dropped | Raised by T12 (F-T12C). Planning: changes --help/error bytes (§A3); cited test is not a byte guard |
| FU-22 | Consider sharing girpr's exhaustive exit-variant list with its binary-target test | dropped | Raised by T13. Planning: widening public API for one test already rejected; no value |
| FU-23 | Correct stale game/ scan-file module doc in tools/capture/src/emit/gamedir.rs | batched (Batch D) | Raised by T14; renderer writes only selected names. Planning: kept, cheap factual doc fix |
| FU-24 | Deduplicate the three report planes' text layout logic | dropped | Raised by T15 (F-T15-01). Planning: §A8-blocked, would normalize vocabularies and move bytes (§A3) |
| FU-25 | Record that sled2redb's conversion report remains coupled to girsync's Record contract | dropped | Raised by T15 (F-T15-02). Planning: moot at cleanup, no action |
| FU-26 | If source-level field-name parity ever matters, add a serde rename seam so product can keep `unknown` for field 6 | dropped | Raised by T16 (F-T16-02). Planning: speculative; would reintroduce churn T16 deliberately avoided |
| FU-27 | Decide whether the three region variant names duplicated in capture's local clap Biz enum are acceptable long-term | dropped | Raised by Review F. Planning: "decide/revisit if" with no action; safe state correct — do not reintroduce shared variant docs |
| FU-28 | Consider having girpr's production_bases accept Host directly to drop its local string->enum remap | dropped | Raised by Review F. Planning: local call-site policy cleanup, no cross-program value |
| FU-29 | Cosmetic: rename product's Biz alias to Region at call sites | dropped | Raised by Review F. Planning: cosmetic only |
| FU-30 | Consider an always-on guard for logs_of_async's current-thread precondition | batched (Batch T) | Raised by Review G. Planning: kept, cheap one-line robustness fix preventing silent partial-log capture |
| FU-31 | Optionally route sync/capture suites through the shared test_support::temp_dir helper | dropped | Raised by Review G. Planning: workspace-consistency-only test churn, low value, touches multiple suites |

## Persistent context

Standing invariants from the report (pre-seeded so they survive a replacement
orchestrator; a session that finds one contradicted by the code should record
that in the relevant task's `note` instead of silently proceeding):

- **gircap independence is a hard boundary** (§A2, lines 21–27; §F30, 388–394):
  every new shared leaf must hold no program logic; adding a third leaf dep to
  `tools/capture` requires updating the table at `tools/capture/docs/00-spec.md:42`
  and the other places named in §A2 in the same work. Do not couple programs to
  prove one another's tests.
- **Byte contracts are load-bearing** (§A3, lines 29–35): key order, casing,
  spacing, flag conditions and bundle bytes must not drift.
- **Divergent policies are intentional** (§A6, lines 52–60): do not "fix" girpr
  jitter vs capture zero-jitter, last-match vs first-match INI, or separate exit
  tables during an extraction.
- **No single digest semantics** (§A7, lines 62–67): girpr strict `Md5`, girsync
  raw bytes + foreign names, gircap lowercase hex must stay distinct.
- **Report planes cannot be merged verbatim** (§A8, lines 69–76): three key
  orders, bool-vs-1/0, percent-encoding; a monolithic shared `Record` is forbidden.
- **Device-name divergence is only a defect candidate** (§A10, lines 83–88):
  confirm before changing; altering the predicate can move bundle bytes.
- **tempdir is not a pure leaf** (§A11, lines 90–94): it has non-dev deps on
  girsync and serde_json for `test_support`.
- **Not shared-crate work** (report §F12 lines 218–224, §F13 226–231, §F14
  233–237, §F29 379–386, and all of §N 396–442): local/deliberately-distinct
  duplication. Raise a follow-up rather than expanding a task into these.

Findings from T01 (e3a1bc08) that later tasks/reviewers depend on:

- girpr's strict `Md5` and `EMPTY_MD5` now live in `crates/digest/src/lib.rs`
  (`girpr-digest`); `src/util/md5.rs` only re-exports them plus scan/read_range.
  Shared crate also supplies `encode_hex`/`decode_hex`, one-shot `md5_hex`/
  `sha256_hex`, `Md5`/`Sha256` newtypes, `EMPTY_SHA256` and public RFC/FIPS
  vector helpers. Direct `hex` deps removed from girpr, gircap, sled2redb.
- gircap leaf deps are now TEN after T17 (audio-language, digest, exit,
  fs-names, log, model, proto, report-core, retry, tempdir); any future leaf addition must update the five §A2 doc
  locations (see CONVENTIONS.md) in the same commit. girsync began depending on
  girpr-digest in T08 for bytes/hex mechanics only — keep its DigestSet/AlgoSet
  and verbatim foreign names local (§A7).
- F5's "third `md5hex` spelling in `tools/sync/tests/helpers.rs`" was stale:
  those are raw-byte digest lines (no `hex` dep, no `md5hex`), so girsync code
  was left untouched. §A7's three digest representations confirmed distinct.

Finding from T02 (skipped; no code change):

- Chunk identity (§F27) is **two rules, not one**. Capture = raw lowercase-hex
  `(&str, i64)` lowercased at the manifest decode boundary, accepting any string
  (a test uses `"same"`). Product (`sophon/map.rs`, `repair/reuse.rs`) = parsed
  16-byte `Md5` + size, skipping malformed md5. Do not merge into one shared
  type (§A7). first-occurrence-wins is caller selection logic, not a value-type
  property. If a follow-up wants cleanup, capture's `assemble/assembly.rs`
  duplicates `SophonChunk::identity()` and could call it — within-capture only.

Review A confirmation (no code change):

- §A7 three digest representations confirmed distinct and must be kept: girpr
  strict `Md5`, girsync raw bytes + verbatim foreign names, gircap lowercase hex
  strings. `girpr-digest` is logic-free; girsync uses only its bytes/hex mechanics
  (from T08), never a shared semantic digest type.
- Full workspace suite green (2125 passed, 11 skipped); one intermittent,
  unrelated flake captured as FU-02.

Finding from T03 (db8c5e37):

- girpr-fs-names is a fourth logic-free leaf shared by girpr and gircap; its
  `is_reserved_device_name` takes caller-supplied reserved set + trim flag.
  girpr keeps full set incl. `CLOCK$` / trim=true; gircap keeps set minus
  `CLOCK$` / trim=false. Neither behavior changed; §A2 tables listed four
  gircap leaf deps at the time (five after T10). girsync gained girpr-fs-names
  as a dep in T04. FU-03 tracks
  the §A10 divergence question (defect candidate, unresolved).
- CONVENTIONS §A2 note "table currently names two" is stale — it named three
  (log, digest, tempdir) before T03; always check the actual doc tables.

Finding from T04 (0651c39b):

- girpr-fs-names now also owns trailing-dot/space detection, ASCII case folding
  and forward-slash rendering. Host-dependent steps are explicit caller flags;
  the crate never checks the host itself. `has_trailing_dot_or_space` is
  parameterless by design (both callers apply it on every host). girsync gained
  girpr-fs-names as a dep; girpr-fs-names was already a gircap leaf, so no §A2
  gircap table update was needed. All three §F7 host-policy divergences and both
  path_key variants preserved; no path test changed. FU-04/FU-05 are doc-only
  suggestions (FU-05: sync's comparison helper stays local by design).

Finding from T05 (ebd758b9):

- `canonical_no_verbatim(&Path)` in girpr-fs-names is the single canonical-path
  spelling (dunce-backed, no `\\?\`); returns `io::Result` unchanged so callers
  keep their own `NotFound`/fallback handling. All girsync production dunce
  sites now use it; girsync keeps a direct dunce dep only for test oracles
  (FU-07). girpr's fail-closed `resolve_existing_prefix` and capture's
  std::canonicalize containment sites were deliberately left unchanged (§N6,
  §F8 separate-variant) — §F8's capture/dunce pointer was stale (they use std).
  The `safety.rs` source-scan test pins canonicalizer paths by literal source
  text (DISK_MODULES + DIRECT_QUESTIONS) — update both if repathed. FU-06 tracks
  the gircap canonicalization question.

Review B confirmation (fix 44dcae95 — girsync README shared-deps description
corrected to list girpr-fs-names; the five §A2 gircap tables were already
correct at four leaf deps then (five after T10) and were left unchanged):

- girpr-fs-names is a logic-free leaf shared by girpr, gircap and girsync with
  five mechanisms: parameterized device-name predicate, host-independent
  trailing dot/space, explicit-flag ASCII fold, explicit-flag forward slash, and
  dunce-backed canonical_no_verbatim. Gircap boundary intact.
- girpr's fail-closed `resolve_existing_prefix` (src/util/path.rs:447) remains
  local, std::canonicalize-based; must not be folded into the shared helper.
- No §N / DO-NOT-TOUCH "keep" item pulled into a leaf; no byte/containment/bundle
  contract changed. Full workspace: 2131 passed, 11 skipped. All §F6-F8 stale
  pointers confirmed (§F8 capture/dunce wrong, §F7 lines drifted, §A2 "two" wrong).

Finding from T06 (57877cac):

- girpr-fs-identity owns platform fact reads (nlink; dev/ino or volume-serial/
  file-index; handle identity) via same-file/winapi-util seams; no unsafe in the
  leaf or either program (A4/F9 "one deliberate unsafe" claim is stale). Keep
  same-file/winapi-util only in the leaf.
- Path-read POLICY stays local per N6: girsync follows the path, girpr reads
  without following (symlink_metadata / FILE_FLAG_OPEN_REPARSE_POINT) so a
  swapped symlink compares unequal. Fail-closed Option<bool> + count->1 guard
  stay in src/repair/lock.rs; girsync refusal stays in its policy modules.
- facts_of_metadata is Unix-only; std's Windows MetadataExt volume/index
  accessors are unstable here — use winapi_util::file::information on Windows.
  No gircap dep added, so no §A2 doc change. Full suite: 2135 passed, 11 skipped.

Finding from T07 (skipped; no code change):

- Atomic publish is NOT one primitive (F10 partly stale, "high confidence" wrong
  for the sync shape). girpr has two distinct writers: write_atomic_with
  (write through the open NamedTempFile handle, anyhow + phase context, optional
  sync_all + best-effort parent sync, lexical name guards) and copy_publish_with
  (injected copy closure re-opens the temp path, io::Result, no sync, caller
  creates parent). They share only the lexical name rule in src/util/path.rs
  (stays local).
- sled2redb: persist vs persist_noclobber chosen from destination state, handle
  carried across functions, lock sidecar removed before publish, context message
  built from the random temp name — all local policies.
- girsync publish is a cross-process claim protocol (hard link, or create_new
  claim + identity check + rename), never NamedTempFile::persist; its working
  name is deliberately kept (no drop cleanup). Must stay local.
- tempfile opens NamedTempFile with share_mode(0) on Windows; handle-based and
  path-based writes are not interchangeable in a shared API. Excluded/untouched:
  capture .slot.tmp / non-atomic docs (N8), girpr _tmp resume artifacts (N6).

Review C confirmation (no fix commit):

- The "one deliberate unsafe in girsync" narrative (report A4/F9, README:505-507)
  is factually stale — no unsafe existed in the file-identity sites; the only
  real unsafe is the env-var test at tools/sync/tests/cli_dispatch.rs:17.
  Treat the report's unsafe claim as invalid (FU-08 tracks README correction).
- F9's "both derive the same platform facts" understates a real divergence:
  girsync follows links, girpr's lock deliberately does not. The leaf bridges
  this via separate from-path/handle APIs, never a shared path-read policy.
- crates/fs-identity is a shared leaf but NOT a gircap dep, so 00-spec.md:42
  (four crates then; five after T10) stays accurate. Root README module map
  (40-69) also omits several
  leaves (digest, filter, fs-names); adding fs-identity there is a low-priority
  doc nicety from the F30 under-description gap, NOT an A2 requirement.
- Leaf uses a private scratch-dir test helper (temp_dir + pid + atomic counter),
  not girpr-tempdir, avoiding a test-only inversion (A11 concern). No coupling.

Finding from T08 (f84ec7b9):

- girpr-digest now also supplies Algorithms (plain MD5/SHA membership, NOT
  girsync AlgoSet), DigestBytes (raw bytes + hex), Sink (update, non-consuming
  md5_bytes/sha256_bytes, reset, Write, finish) and hash_reader. Migrated all
  five F3 sites; each owner keeps its ordering policy: limit-before-forward
  (girpr HashingWriter), write-before-hash (gircap HashingFile), restart-on-retry
  (HashingFile::restart local truncate/rewind + shared reset), seek-to-start
  (girpr reader), girsync AlgoSet→leaf mapping local. No ceiling/accepted-byte/
  restart policy entered the leaf; digests byte-identical.
- girsync now depends on girpr-digest (mechanics only); DigestSet/AlgoSet/HashAlg
  untouched (N5). gircap leaf count unchanged at four then (five after T10);
  no A2 doc edit here. Direct
  sha2 deps removed from gircap/girsync, md-5 regular dep from girsync (dev only).
- F3's src/util/md5.rs:365,397-402 pointer stale (now test code); real sink is
  md5_reader_digest_with_buf. FU-13 tracks a possible zstd_measure follow-up
  (must respect N10 duplication + distinct error text).

Finding from T09 (skipped; no code change):

- Bounded zstd decode has NO viable shared leaf (F28 medium confidence wrong).
  Product path is async write-side (zstd write::Decoder into HashingWriter, blob
  never held, compressed + decompressed ceilings, retry/consumed accounting,
  finish() completeness); capture is sync read-side (read::Decoder, buffered Vec
  via take(limit+1) or 16 KB measure loop). Opposite APIs, distinct byte-stable
  error text, distinct result types (Md5 newtype vs hex String vs Vec) — no
  forced wrapper. Ceilings/hash-order/restart are deliberately NOT in the
  shared digest crate (T08 design); extracting them would change error messages.
- zstd_measure / zstd_measure_from stay duplicated (N10). FU-14 is a flag-only
  suggestion to consider the shared sink in capture's measure path.

Finding from T10 (217477de):

- crates/retry (girpr-retry) owns the Schedule factory (base delay, multiplier,
  jitter, cap, zero-wait mode), DelaySequence and an injectable Sleep hook
  (RealSleep/ZeroSleep/RecordingSleep). Jitter is passed IN, never chosen: girpr
  0.5 (+/-50%, 1s/x2/30s), gircap 0.0 (500ms/x2/10s, no elapsed cutoff). Kept
  local: girpr Retry/RetryState mapping + error text (manual loop so sleeps go
  through the hook), gircap RequestLog pushes, error bodies and GIRCAP_TEST_ZERO_BACKOFF.
- backoff is now leaf-only (no tokio feature in consumer programs); girpr dropped
  backoff::future::retry. gircap re-exports the leaf Sleep types via http::transport
  so its public surface is unchanged. Zero-wait test modes + byte-stable artifacts
  pass. Full workspace: 2146 passed, 11 skipped.
- gircap had FIVE leaf deps after T10 (log, digest, fs-names, retry, tempdir);
  all five A2 doc locations were updated in that commit. SIX after T13 (exit),
  SEVEN after T14 (audio-language), EIGHT after T15 (report-core), NINE after T16
  (proto).
  Earlier "four leaf deps" references in this file are point-in-time. FU-15
  likely moot at cleanup.

Review D confirmation (no fix commit):

- A7 three digest semantics confirmed distinct after T08; shared digest leaf
  carries only raw bytes/hex mechanics, never a program digest vocabulary.
- gircap has five leaf deps as of Review D (nine after T16); five A2 doc locations
  current (00-spec.md:42, README.md, tools/sync/README.md,
  tools/capture/README.md, docs/06-artifact-bridge-spec.md) — any new leaf must
  update all five.
- girpr retry sleeps must go through the injected Sleep hook; jitter is a caller
  parameter (girpr 0.5 / gircap 0.0), never baked into the leaf. girsync uses
  girpr-digest for bytes/hex only; keep DigestSet/AlgoSet/HashAlg + foreign names
  local (N5). Full suite 2146 passed, 11 skipped; clippy -D warnings clean.

Finding from T11 (7b4ff3be):

- girpr-log gains install mechanics (open_log_file, file_sink, emit_env_warnings,
  warn_subscriber_install, announce_log_file!) and format_timestamp(&DateTime).
  Layer stacks + guard ownership stay in the three programs. Shared log events
  must be emitted from a macro expanding at the caller (or carry a 'static
  target) — each program's console filter is scoped to its own target and drops
  events emitted from inside girpr-log.
- Env-warning prefixes (WARN / gircap:) and install-warning text are observable
  byte contracts (A3); kept identical via a prefix parameter. Capture's
  capture_id and emit/index.rs ManifestInput::started inject a DateTime — must
  not use wall-clock timestamp(). F18/F19 overstate remaining duplication; the
  main.rs eprintln! echo is caller-facing and stays local.
- Behavior change to assess: bare --log-file run.log now opens the file (was
  warn+no file) — matches product CLI doc, no byte impact; FU-16. Full suite:
  2151 passed, 11 skipped. No new gircap leaf dep, so no A2 doc edit.

Finding from T12 (skipped; no code change):

- F20 is NOT a valid extraction boundary for help-bearing CLI declarations. The
  three tools' --log-level/--log-file/--output help text and value_name differ
  substantively; OutputFormat variant doc comments render into --help "Possible
  values:" lines, so one shared enum changes CLI bytes (A3). The four log flags
  do share value_name=LV, default=info, girpr_log::parse_level, conflicts_with.
- Only byte-neutral shareable piece: clap-error mapper (help/version -> 0, else
  the tool's own usage code) with NO shared exit table (A9); FU-19 tracks it.
- Rust stable cannot expand macros into struct fields; a shared LogArgs must
  either be a #[derive(Args)] struct (cannot preserve per-tool help/global
  placement) or generate whole structs. capture alone has --version. F20 line
  numbers drifted; A2 spec table now names five leaf crates (five at T12; six
  after T13).

Finding from T13 (3c691e58):

- girpr-exit leaf's define_exit_table! generates only the mechanism: enum with
  caller-supplied #[repr(u8)] discriminants, ALL, code(), wildcard-free pin.
  girpr RunExit 0-5, capture ExitCode 0-4; all codes/slugs/descriptions/helpers
  and usage->exit mappings stay local, including capture's 1/2 overlap (A9/N3).
  Capture's enum gained #[repr(u8)] + explicit discriminants — internal repr only,
  table test pins all five (code, slug) pairs.
- The shared pin requires codes contiguous from zero; a future tool with
  non-contiguous exit literals (girsync/sled2redb) needs a parameterized pin,
  not a table change. No usage-error mapping moved into the leaf.
- gircap had SIX leaf deps after T13 (log, digest, exit, fs-names, retry,
  tempdir), all five A2 doc locations updated then; SEVEN after T14, EIGHT after
  T15, NINE after T16. FU-22 is non-blocking.

Finding from T14 (e67cf272):

- girpr-audio-language leaf holds normalize_audio_token (lower + _->-, with a
  CaseFold policy parameter: girpr Unicode no-trim, capture ASCII + trim at
  call site), the four canonical AUDIO_LANGUAGES code/display pairs, and pure
  lookup helpers. Alias map (girpr), none-sentinel (both), trim and
  field_component_problem validation (capture) stay local. Do NOT collapse the
  two CaseFold flavours — they are byte contracts (A3).
- gircap had SEVEN leaf deps after T14 (audio-language, digest, exit, fs-names,
  log, retry, tempdir), all five A2 doc locations + manifest header updated then;
  EIGHT after T15, NINE after T16. Audio bytes verified identical against baseline.
- Unrelated stale doc FU-23 (gamedir.rs game/ scan-file module comment) left out
  of scope. Product lib 422 + capture lib 727 + leaf 2 tests pass.

Review E confirmation (no fix commit):

- Bare --log-file behavior change reviewed and ACCEPTED — do not revert
  girpr-log::install::open_log_file. On current stable toolchain the old code
  also opened bare filenames (create_dir_all("") = Ok(())), so no observable
  delta; no byte/exit-code impact. FU-16 now just suggests cheap test coverage.
- Any shared tracing event must expand at the caller with explicit target
  (scoped prefix EnvFilter + rendered target bytes); a plain function in
  girpr-log changes log bytes. Capture's injected DateTime -> format_timestamp
  seam is a byte contract — never swap for wall-clock timestamp().
- gircap has SEVEN leaf deps as of Review E (nine after T16); five A2 doc
  locations + capture manifest header current (any new leaf must update all in
  the same work). CaseFold Unicode vs Ascii + caller-side trim are intentional
  A3 divergences. Exit codes remain separate contracts (A9/N3); shared leaf
  holds mechanism only.
- No cross-program coupling: crates/exit and crates/audio-language have empty
  [dependencies]; capture still declares no girpr dep. Full suite 2153 passed,
  11 skipped; clippy -D warnings clean.

Finding from T15 (bcdbfa02):

- girpr-report-core leaf has only write_json_object (ordered JSON via
  serialize_entry, parameterised by EventFirst/Alphabetical + caller-supplied
  event key/value) and write_line. NO shared Record and NO value-vocabulary
  normalization; NO preserve_order anywhere. Three distinct vocabularies kept:
  girpr Str/UInt/Int (no bool, 1/0 flags), girsync serde_json::Value incl. arrays
  (event in sorted slot), gircap Str/Int/Bool with percent-encoded text (real
  event field). sled2redb unchanged — still reuses girsync's writer (FU-25).
- All named byte-level report oracles pass unchanged. F21 medium confidence on
  the boundary confirmed viable: a narrow core works without normalizing values
  (A8 satisfied). FU-24 (text layout dedup) is A8-blocked, non-blocking.
- gircap had EIGHT leaf deps after T15; all five A2 doc locations + capture
  manifest comment updated then (00-spec table named eight). NINE after T16.
  Full suite 2095 passed (4-core selected crates), 11 skipped; clippy clean.

Solo review confirmation (no fix commit):

- F21 boundary confirmed: narrow ordered-JSON/write_line core works WITHOUT
  normalizing the three value vocabularies; A3/A8 satisfied, no preserve_order
  enabled anywhere. All named byte oracles pass unchanged across all planes.
- Full workspace suite 2157 passed, 11 skipped; clippy -D warnings clean. Leaf
  deps: serde + dev serde_json only. gircap eight leaf deps then (nine after T16);
  five A2 locations + capture manifest comment current. sled2redb still reuses
  girsync writer (FU-25).

Finding from T16 (e377f50c):

- girpr-proto leaf holds the three Sophon wire types + prost tags, chuncks and
  tag 6 named unknown_field6, deriving serde::Serialize (field names ARE dump
  JSON keys). Both programs re-export them; WantedManifest stays in product.
  Byte gates: the_serialized_keys_are_the_documented_ones and
  the_full_decode_carries_every_entry_verbatim_including_field_six + new tag-6
  pin (0x30). F24 accurate; A2 "table names two" stale (was eight, now nine).
- Capture's six methods + three normalise variants are public extension traits
  (SophonChunkExt/SophonChunkFileExt/SophonChunkManifestExt) re-exported from
  gircap::manifest; call sites must import the trait. No program logic in leaf.
- Source-only change: product field 6 renamed unknown -> unknown_field6 (prost
  keys by tag, product never serializes, so no wire/JSON byte changed). FU-26
  tracks a possible serde-rename seam if source parity ever matters.
- F-T16-01 (girpr-filter missing from doc tables) is a duplicate of existing
  FU-01 — folded in, not re-added. gircap now NINE leaf deps; five A2 locations
  + capture Cargo.toml comment (eight -> nine) updated. Full suite 2158 passed,
  11 skipped.

Finding from T17 (92292793):

- crates/model (girpr-model) region module is logic-free: const accessors only
  for the three region ids, launcher/game ids, channel tuples, Host families,
  four production base URLs, and the byte-stable DOWNLOAD_MODE_CHUNK +
  CHANNEL_ENDPOINT. No resolve policy, URL building or validation in the leaf.
  clap ValueEnum derive is behind an optional `clap` feature — `cargo tree
  -p girpr-model --no-default-features` has zero deps. Both programs enable the
  feature; see Review F — the product re-exports `Region as Biz`, but capture
  keeps a local byte-pinned clap `Biz` enum (variants delegate to the shared
  table) so its `--help` bytes stay identical.
- girpr's production_bases string->Host mapping, capture's Biz::resolve, and the
  full Endpoints<'a> override seam (incl. owned's Box::leak) stay at call sites.
  The channel-only-on-deprecated rule moved from capture's `const fn matches!`
  to a non-const `path() == CHANNEL_ENDPOINT` comparison, keyed off the shared
  endpoint-name value; byte-identical, pinned by
  only_the_deprecated_endpoint_carries_the_channel_tuple on both sides.
- F25 pointers were stale (as expected); duplication premise validated against
  current code. No byte contract changed; test oracles/fixture JSON deliberately
  keep literal region strings (A3) and were NOT migrated.
- gircap now has TEN leaf deps (model added); all five A2 doc locations + capture
  manifest header updated (nine -> ten) in the same commit. The "table currently
  names two" wording in CONVENTIONS/ORCHESTRATION is stale point-in-time, like
  earlier tasks noted. Full suite 2162 passed, 11 skipped; clippy -D warnings
  clean; fmt clean.

Review F confirmation (fix 042683eb — capture --help bytes):

- Byte-contract regression found and fixed: shared `Region`'s per-variant doc
  comments made clap render possible-value descriptions into the PRODUCT's
  `--biz` help (described list) while capture's was already described. Fix
  removed variant docs from the leaf and restored capture's local byte-pinned
  clap `Biz` enum (variants delegate values to the shared region table). Do NOT
  reintroduce variant docs into the leaf — workspace feature unification can
  change other programs' `--help` bytes (FU-27). Pre/post `--help` diffs for both
  binaries are now identical.
- T16 verified against pre-extraction snapshots: wire tags/types, `chuncks`
  misspelling, field 6 and Serialize key set byte-identical; product field-6
  source rename is source-only. Capture's six methods + three normalise variants
  remain local extension traits.
- gircap boundary preserved: girpr-proto deps = prost + serde; girpr-model zero
  deps without clap; no girpr dep or binary invocation in capture; both leaves
  logic-free; capture's Endpoints<'a>/Box::leak seam and resolve policies local;
  five §A2 locations current at ten leaf deps.
- No DO-NOT-TOUCH / §N item pulled into a leaf (no exit codes, report semantics,
  digest vocabularies, INI parsers, URL/status helpers, artifact writer or fake
  servers touched). Full suite --all-features 2162 passed, 11 skipped; clippy
  --all-features -D warnings clean.

Finding from T18 (skipped; no code change):

- §F26's "field sets match" claim is REFUTED by current code. The two families
  model different JSON subsets: girpr GameConfig has audio_pkg_cache_dir/
  res_category_dir/blacklist_dir/enable_resource_blacklist that capture's
  LaunchConfig lacks, and capture has installation_dir/enable_audio_pkg_mgmt/
  enable_ldiff that girpr lacks; shared field names are String (girpr) vs
  Option<String> (capture). ManifestFile/ManifestRef differ (i64 via
  num_from_string vs Option<IntOrStr>; girpr Deserialize-only vs capture
  Serialize+Deserialize). ManifestUrl/Download differ (capture adds encryption,
  password, compression). ChunkManifestMeta/BuildManifest differ (capture adds
  stats, deduplicated_stats). Wrapper<T> differs (girpr Option<String>/
  Option<T>, Deserialize vs capture String/T, Serialize+Deserialize). Closest
  look-alikes (GameIdRef/GameRef, DeprecatedFile) differ in serde strictness:
  girpr requires id/biz/name, capture defaults them. Only the decoding/URL
  divergence is real; sharing would weaken one program's validation (A3).
- num_from_string (girpr, strings -> i64) and IntOrStr (capture, wire-preserving
  re-serialization) must stay separate (§F26/§A7); URL joins stay distinct
  (Url::join trailing slash vs prefix+id+suffix, §N9). Capture DTOs carry
  program-logic inherent methods (ApiError, paths, zstd/encryption, URL join)
  and girpr relies pervasively on concrete i64/String fields — a logic-free leaf
  is not possible without a byte/validation change or program coupling (§A1/A2).
- Both candidate boundaries rejected: girpr-model module (no doc churn) still
  needs incompatible types; dedicated girpr-api leaf adds an 11th gircap dep and
  §A2 churn for nothing shareable. No §A2 table changed (no dependency added).
- Possible re-scope for a future reviewer/follow-up: (a) explicit decision
  accepting a byte/validation-contract change, or (b) share ONLY the two decoder
  mechanics as separate public adapters in a leaf while leaving structs, inherent
  logic and URL joins local — low value (each decoder has one consumer). Baseline:
  gircap hyp:: 42/42, girpr hyp:: 13/13, clippy -D warnings clean.

Review G confirmation (no fix commit):

- Both T19/T20 completed. One public girpr-log::test_support module (feature-
  gated) with logs_of/logs_of_async; all three copies are shims. T20's
  temp_dir/write_game_file/read_game_file live once in girpr-tempdir::test_support
  with product call-site-identical shims; md5hex already a shim over girpr-digest;
  the two fs2 fixtures are genuinely distinct (product run-lock cleanup vs
  capture shared-bundle cache, FNV key is a §N6 keep) and stay independent.
- Test independence preserved (§A2/§A11): helpers only in logic-free leaves, no
  program's tests depend on another's; the pre-existing girpr-tempdir->girsync
  test inversion is unchanged. No fake HTTP server or tool-specific fixture was
  shared (§N7). No new gircap leaf dep, so no §A2 doc churn.
- Byte/behavior/debug-assert/panic-message contracts preserved; only unreachable
  internal panic strings changed (unwrap -> expect), not load-bearing. girsync
  SharedBuffer and its phases.rs copy stay local. logs_of_async's CurrentThread
  debug_assert message byte-identical (FU-30 for an always-on guard).
  Full girpr suite 618 passed, 0 failed, 8 skipped; clippy clean.

Follow-up planning review (Phase 3 guard — all 20 tasks settled):

- Kept only follow-ups that are safe (no §A3 byte change, no §A2 coupling, no
  §N/DO-NOT-TOUCH 'keep' item, no expansion of a skipped task) AND bounded;
  dropped the rest. No new follow-ups proposed.
- Batch D (documentation accuracy): FU-01, FU-04, FU-08, FU-23 — correct
  factual/stale PRODUCT docs (what-ships tables, fs-names description, README
  unsafe claim, gamedir module doc); zero code/byte changes; must not touch
  REFACTOR_PLAN/ or the research report.
- Batch T (test coverage & robustness): FU-09, FU-16, FU-30 — Windows symlink-
  swap regression test, cheap bare-`--log-file` CLI test, always-on runtime
  precondition guard in logs_of_async; no production/byte changes.
- Batch C (byte-neutral CLI helper): FU-19 — extract the clap-error -> usage-code
  mapper into a logic-free leaf parameterised by caller usage code; verify exit
  codes 0/1/3 identical, keep each tool's exit mechanics; update all five A2 doc
  locations only if capture takes the dependency.
- Dropped rationale: byte/keep violations (FU-03 §A10+bundle bytes, FU-06
  containment, FU-13 §N10, FU-18/FU-20/FU-21 §A3, FU-24 §A8+§A3); skipped-task
  scope creep (FU-10, FU-11, FU-12); stale/moot at cleanup (FU-12, FU-15,
  FU-25); doc-only niceties/cosmetic/already-rejected (FU-05, FU-22, FU-28,
  FU-29, FU-31); speculative/conditional (FU-17, FU-26, FU-27); non-reproducible
  noise (FU-02). FU-14/FU-15 were already dropped earlier.
