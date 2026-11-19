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
| T01 | Delete the stale zstd `Decoder` workaround | todo | | |
| T02 | One shared timestamp helper | todo | | |
| T03 | Hex encode/decode sweep | todo | | |
| T04 | FNV literals → named constants | todo | | |
| T05 | `jobs` range validation in clap | todo | | |
| T06 | `AtomicU64::fetch_update` + simplify test gate | todo | | |
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
