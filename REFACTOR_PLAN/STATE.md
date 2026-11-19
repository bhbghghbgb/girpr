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
| T01 | Delete the stale zstd `Decoder` workaround | completed | 27aa9041 | Passed HashingWriter by value to write::Decoder::new; finish() yields owned writer (§I.10); behavior identical |
| T02 | One shared timestamp helper | completed | 382f992f | girpr_log::timestamp is single source; girsync ts_now shim kept for cache/open.rs. Stale pointer: hard-coded stamp not at plan/tests.rs:439, now in cache/open.rs tests; all pass. capture uses caller-supplied DateTime, left out of scope; unused chrono dep removed |
| T03 | Hex encode/decode sweep | completed | 9f6ce80d | All six sites use hex 0.4.3; Md5 Display kept allocation-free via encode_to_slice + stack buffer; sled2redb param renamed hex→text to avoid crate shadowing; Cargo.lock updated minimally (no unrelated downgrades) |
| T04 | FNV literals → named constants | skipped | | Premise false: pinned fnv 1.0.7 exposes no named constants; goal already met by existing FNV_OFFSET/FNV_PRIME named constants in repair/lock.rs; do not add hashkit |
| T05 | `jobs` range validation in clap | skipped | | Clap-level validation changes error timing/format; --jobs 0 is a pinned post-parse test trigger; semantics cannot be preserved cleanly (stale main.rs:94-96 pointer, real code at 34-44 and 144-157) |
| T06 | `AtomicU64::fetch_update` + simplify test gate | completed | 8141d6fa | Used try_update (fetch_update deprecated in rustc 1.100, identical method); custom Gate harness removed, race coverage retained via Barrier; downstream ledger race test rewritten with Barrier |
| T07 | `girpr-tempdir` `TempDir` → `tempfile` | pending_review | 4847a15a | Delegated to tempfile::Builder::tempdir_in; kept custom Drop for at()/absent() (TempPath only removes files); name-shape test replaced with recognizability check. Cargo.lock re-resolution downgraded unrelated transitive deps (originals uncached). Proposed follow-up §I.14: drop girsync dep from tempdir |
| T08 | `tracing-appender` as single file-log writer | pending_review | f11f7ac7 | Stale premise: only one process::exit (parse_args, pre-init_tracing); logging paths already flush. Flush-failure warning + its test removed (WorkerGuard::drop has no error channel); lossy 128k-line queue replaces backpressured BufWriter; README buffer bullet updated |
| T09 | EnvFilter spec via `add_directive` | pending_review | fb76d39e | console_filter now returns EnvFilter via try_new + add_directive (not builder().with_env_var, which ignores captured RUST_LOG); all 3 consumers updated; sync redb=warn now appended after RUST_LOG (obscure conflict behavior changes to hard-floor intent); warn/fallback preserved; no with_default_directive/parse_lossy |
| T10 | Directory walkers → `walkdir` | pending_review | 392d5cbb8f5242ff2074b69561d396cd8fffb0e7 | Premise stale: walkdir only in root/sync, added to gircap; fault-injection seam reshaped to pre-descent access check, child_dirs removed, 5 error tests rewritten; loop/dangling-link errors skipped to match old behavior; MANIFEST boundary + sorted order preserved |
| T11 | Percent encoding → `percent-encoding` | pending_review | bfdb5122 | Literal AsciiSet plan impossible in 2.3.2 (escapes all non-ASCII); used percent_encode_byte primitive; kept minimal 5-char escape set; old decoder accepted %+A sign edge now passed through; added explicit non-ASCII pinning test |
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
| A | T01, T02, T03, T06 (T04, T05 skipped) | done | All four completed; no fixes/reverts; 4 follow-ups recorded |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Collapse girsync ts_now shim; call girpr_log::timestamp directly in cache/open.rs | open | Low-payoff cleanup (from T02) |
| F02 | Optional second hex sweep of non-digest u8::from_str_radix / format sites | open | Not digest-hex; out of T03 scope |
| F03 | Restore deterministic interleaving test only if saturating logic returns in-house | open | Barrier contention suffices today (from T06) |
| F04 | Make gircap spawn tests locate the binary robustly via a runner-provided env var | open | Harness-only; task-relevant tests pass |

## Persistent context

- gircap spawn-based integration tests (bundle_consistency, capture_layout,
  cli_contract) fail in this harness with "cannot find gircap.exe": test binaries
  live in build-script OUT_DIR so the bin() helper looks in the wrong place.
  Not attributable to task code; copying the executable beside the test binaries
  makes them pass. Relevant to gircap test work (report §F, tasks T23-T24).
