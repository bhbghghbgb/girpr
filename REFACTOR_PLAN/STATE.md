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
| T01 | Delete the stale zstd `Decoder` workaround | pending_review | 27aa9041 | Passed HashingWriter by value to write::Decoder::new; finish() yields owned writer (§I.10); behavior identical |
| T02 | One shared timestamp helper | pending_review | 382f992f | girpr_log::timestamp is single source; girsync ts_now shim kept for cache/open.rs. Stale pointer: hard-coded stamp not at plan/tests.rs:439, now in cache/open.rs tests; all pass. capture uses caller-supplied DateTime, left out of scope; unused chrono dep removed |
| T03 | Hex encode/decode sweep | pending_review | 9f6ce80d | All six sites use hex 0.4.3; Md5 Display kept allocation-free via encode_to_slice + stack buffer; sled2redb param renamed hex→text to avoid crate shadowing; Cargo.lock updated minimally (no unrelated downgrades) |
| T04 | FNV literals → named constants | skipped | | Premise false: pinned fnv 1.0.7 exposes no named constants; goal already met by existing FNV_OFFSET/FNV_PRIME named constants in repair/lock.rs; do not add hashkit |
| T05 | `jobs` range validation in clap | skipped | | Clap-level validation changes error timing/format; --jobs 0 is a pinned post-parse test trigger; semantics cannot be preserved cleanly (stale main.rs:94-96 pointer, real code at 34-44 and 144-157) |
| T06 | `AtomicU64::fetch_update` + simplify test gate | pending_review | 8141d6fa | Used try_update (fetch_update deprecated in rustc 1.100, identical method); custom Gate harness removed, race coverage retained via Barrier; downstream ledger race test rewritten with Barrier |
| T07 | `girpr-tempdir` `TempDir` → `tempfile` | todo | | |
| T08 | `tracing-appender` as single file-log writer | todo | | |
| T09 | EnvFilter spec via `add_directive` | todo | | |
| T10 | Directory walkers → `walkdir` | todo | | |
| T11 | Percent encoding → `percent-encoding` | todo | | |
| T12 | Windows device-name / path-shape rule | todo | | |
| T13 | girpr JSON renderer → `Serialize` | todo | | |
| T14 | girsync JSON renderer → `Serialize` | todo | | |
| T15 | gircap JSON renderer → `Serialize` | todo | | |
| T16 | girpr retry/backoff → `backoff` | todo | | |
| T17 | gircap retry/backoff → `ExponentialBackoff` | todo | | |
| T18 | girsync path canonicalization → `dunce` | todo | | |
| T19 | girpr `resolve_existing_prefix` validation | todo | | |
| T20 | Glob filtering → `globset` / shared crate | todo | | |
| T21 | Atomic publish helpers → library | todo | | |
| T22 | Status error + URL join → `reqwest` | todo | | |
| T23 | gircap HTTP mock servers | todo | | |
| T24 | girpr canned HTTP reply server | todo | | |
| T25 | Hardlink identity → `same-file` | todo | | |
| T26 | girsync gate/concurrency simplification | todo | | |
| T27 | Cache wire codec → `postcard`/`bincode` | todo | | |
| T28 | `cache/open.rs` publish protocol restructure | todo | | |
| T29 | girpr INI parse/merge/render | todo | | |
| T30 | gircap `parse_game_version` | todo | | |
| T31 | `JoinSet` window → `buffer_unordered` | todo | | |
| T32 | Chunk-range tiling → interval crate | todo | | |
| T33 | Error `Display` impls → `thiserror` | todo | | |
| T34 | sled2redb temp claim → `NamedTempFile` | todo | | |
| T35 | `IntOrStr` → `serde_with` (optional) | todo | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| _none yet_ | | | |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| _none yet_ | | | |

## Persistent context

None yet. Record validation results and decisions here (as pointers to report
sections where possible). No dates, session/tool IDs or change history.
