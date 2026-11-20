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
| T01 | Digest newtypes + hex sweep (`girpr-digest`) | pending_review | e3a1bc08 | F2, F4, F5 (F5 stale: girsync raw bytes, no hex). Deviations + context in Persistent context |
| T02 | `ChunkIdentity` (md5, size) value type | skipped | | F27; follows T01. F27 premise contradicted by code: two distinct §A7 case-insensitive-MD5 mechanisms (capture raw normalised hex `&str` vs product parsed 16-byte `Md5` + invalid-skipped). A shared type collapses §A7 and changes behavior. Details in Persistent context |
| T03 | Windows device-name predicate leaf (`girpr-fs-names`) | todo | | F6, A10; share the rule, do not change behavior |
| T04 | Trailing-dot/space and path-key predicates | todo | | F7; host policy explicit, `path_key` not merged |
| T05 | Canonicalize + `\\?\` stripping helpers | todo | | F8 |
| T06 | File identity + link-count leaf | todo | | F9, A4; one deliberate `unsafe` to account for |
| T07 | Atomic temp-then-publish leaf | todo | | F10 |
| T08 | Streaming hash-sink leaf | todo | | F3; ordering policies stay with owners |
| T09 | Bounded zstd decode primitive | todo | | F28; medium confidence, may be skipped |
| T10 | Retry policy/schedule leaf (`girpr-retry`) | todo | | F15, A6; jitter divergence stays local |
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
| A | T01, T02 | pending | Digest/identity foundations — review together |
| B | T03, T04, T05 | pending | Path and fs-name predicates — related leaf |
| C | T06, T07 | pending | Filesystem identity + publish — medium risk |
| D | T08, T09, T10 | pending | Hashing, bounded decode, retry — review together |
| E | T11, T12, T13, T14 | pending | Logging/CLI surface — check no cross-program coupling |
| solo | T15 | pending | Report core — byte-contract critical, own review |
| F | T16, T17 | pending | Domain model leaves — review together |
| solo | T18 | pending | API DTO leaf — largest model task, own review |
| G | T19, T20 | pending | Test infrastructure — A5/A11 constraints |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| (none yet) | | | |

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
- gircap now has **three** shared leaf deps (log, digest, tempdir); any future
  leaf addition must update the five §A2 doc locations (see CONVENTIONS.md) in
  the same commit. girsync stays off `girpr-digest` — raw bytes + verbatim
  foreign names are deliberate distinct §A7 semantics.
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
