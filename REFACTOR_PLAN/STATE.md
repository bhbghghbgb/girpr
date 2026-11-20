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
| T08 | Streaming hash-sink leaf | pending_review | f84ec7b9 | F3; girpr-digest gains Sink/Algorithms/DigestBytes/hash_reader; ordering traps kept local; girsync now depends on girpr-digest (mechanics only); no gircap dep change; FU-13 |
| T09 | Bounded zstd decode primitive | skipped | | F28 medium-confidence claim contradicted by code: product decode is async write-side, capture sync read-side; ceilings/error text/result types deliberately distinct; only shared piece (hashing) already in girpr-digest. zstd_measure loops stay duplicated (N10). FU-14 |
| T10 | Retry policy/schedule leaf (`girpr-retry`) | in_progress | | F15, A6; jitter divergence stays local |
| T11 | Log-install helpers + timestamp reuse | todo | | F18, F19; guard lifetime stays local |
| T12 | CLI leaf (`OutputFormat`, `LogArgs`, clap-error mapper) | todo | | F20 |
| T13 | Exit-table mechanism macro | todo | | F22, A9; tables stay independent |
| T14 | Audio-language normalisation primitive | todo | | F23 |
| T15 | Ordered-JSON / `write_line` report core | todo | | F21, A3, A8; medium confidence, byte-critical |
| T16 | Sophon protobuf wire types leaf (`girpr-proto`) | todo | | F24; load-bearing field/JSON keys |
| T17 | Region model leaf (`girpr-model::region`) | todo | | F25, A9 |
| T18 | API DTO leaf (keep both decoding strategies) | todo | | F26, A7 |
| T19 | Shared log-capture test helper | todo | | F16, A5; helper must become public/re-exported |
| T20 | Shared generic test helpers | todo | | F17, A5, A11; fake HTTP servers excluded (N7) |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02 | done | T01 completed (no fixes); T02 skipped — not reviewed. FU-01, FU-02 raised |
| B | T03, T04, T05 | done | All completed, no reverts/skips; fix 44dcae95 (girsync README); FU-03..07 unchanged |
| C | T06, T07 | done | T06 completed, no fixes; T07 skipped — not reviewed. FU-08/09 kept open |
| D | T08, T09, T10 | pending | Hashing, bounded decode, retry — review together |
| E | T11, T12, T13, T14 | pending | Logging/CLI surface — check no cross-program coupling |
| solo | T15 | pending | Report core — byte-contract critical, own review |
| F | T16, T17 | pending | Domain model leaves — review together |
| solo | T18 | pending | API DTO leaf — largest model task, own review |
| G | T19, T20 | pending | Test infrastructure — A5/A11 constraints |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| FU-01 | Add girpr-filter to cross-program "what ships" tables | open | Review A: pre-existing §F30 doc gap (README.md:44-45,62; tools/capture/docs/00-spec.md:40-42); doc-only, non-blocking, out of T01 scope |
| FU-02 | Investigate intermittent `repair_machinery the_three_tiers_produce_the_same_file` flake under full parallel nextest | open | Review A: failed once under full parallel run, passed in isolation and on clean full rerun; unrelated to digest changes |
| FU-03 | Resolve girpr/gircap Windows device-name divergence (CLOCK$ reservation + trailing-space trim): decide defect vs intentional bundle-format allowance, whether to unify | open | Raised by T03 per §A10 — defect candidate, not verdict; behavior change may move bundle bytes (§A3), needs confirmation. T03 shared mechanism only |
| FU-04 | Generalise girpr-fs-names one-line description in the five §A2 doc locations (crate now also owns trailing-dot/space, ASCII-fold, forward-slash and canonicalization mechanics) | open | Raised by T04, extended by T05 (Cargo.toml description already updated): doc-only, no behaviour change; no new gircap dep so not required; reviewer may batch with other doc cleanup |
| FU-05 | Consider documenting sync's local `ascii_case_eq`/`ascii_lower`/`FoldedKey` as intentionally unshared | open | Raised by T04: non-blocking; deliberately local (std-independent comparison, zero-allocation borrowed key, not duplicated elsewhere) |
| FU-06 | Decide whether gircap's std::canonicalize containment sites should consolidate onto the shared helper | open | Raised by T05: verify/list.rs, verify/bundle.rs, emit/index.rs use std (not dunce per §F8) and rely on verbatim spelling on both sides of starts_with; coordinated root+child change needed, preserve escape detection + byte stability |
| FU-07 | Migrate girsync test oracles off direct dunce so its direct dunce dep can be dropped | open | Raised by T05: optional cleanup — plan/tests.rs + two cfg(test) helpers still use dunce::canonicalize as oracles; all production sites now use the shared helper |
| FU-08 | Correct README sentence claiming a deliberate unsafe Windows handle call in girsync's hardlink module | open | Raised by T06: doc-only — unsafe claim is stale, both sites delegate to winapi-util/same-file; left untouched to keep T06 code-scoped |
| FU-09 | Add Windows regression test for lock path swapped to symlink after open | open | Raised by T06: non-blocking coverage — existing swap test is cfg(unix); reparse-point branch preserved structurally but not directly asserted on Windows |
| FU-10 | Re-scope atomic publish as a parameterized temp-file publish guard leaf (configurable temp creation + persist/noclobber + writer/path access; keep girsync claim local) | open | Raised by T07 (F07-a): thin-wrapper value questionable; needs explicit decision that indirection is worth it and per-phase error messages preserved |
| FU-11 | Decide whether girsync's kept-path working-name creation should use any shared temp helper | open | Raised by T07 (F07-b): girsync uses tempdir_in/tempfile_in with kept path, no cleanup, claim publish — a different primitive, excluded from T07 |
| FU-12 | Re-validate F10 against code before opening any atomic-publish follow-up | open | Raised by T07 (F07-c): report's static "high confidence" was wrong for the sync shape; F10 partly stale, not authoritative |
| FU-13 | Consider using girpr-digest Sink/hash_reader as the digest core of capture's zstd_measure / zstd_measure_from loops | open | Raised by T08: N10 requires those loops stay duplicated for distinct error text, T09 owns bounded zstd decode; any change must preserve digest bytes + error messages exactly |
| FU-14 | Consider using the shared digest sink in capture's measure path | open | Raised by T09 (FU-T09-B): zstd_measure/zstd_measure_from use raw md5::Md5 while DualDigest uses girpr_digest::Sink; low value, risks N10 error-text/byte contracts — flag only, do not implement without confirmation |

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
- gircap gained digest as a leaf in T01 (four leaf deps now: log, digest,
  fs-names, tempdir); any future leaf addition must update the five §A2 doc
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
  `CLOCK$` / trim=false. Neither behavior changed; §A2 tables now list four
  gircap leaf deps. girsync gained girpr-fs-names as a dep in T04. FU-03 tracks
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
correct at four leaf deps and were left unchanged):

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
- crates/fs-identity is a fifth shared leaf but NOT a gircap dep, so 00-spec.md:42
  (four crates) stays accurate. Root README module map (40-69) also omits several
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
  untouched (N5). gircap leaf count unchanged at four; no A2 doc edit. Direct
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
