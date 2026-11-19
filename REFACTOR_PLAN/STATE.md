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
| T07 | `girpr-tempdir` `TempDir` → `tempfile` | completed | 4847a15a, 4636a691 | Delegated to tempfile::Builder::tempdir_in; kept custom Drop for at()/absent() (TempPath only removes files); name-shape test replaced with recognizability check. Cargo.lock re-resolution downgraded unrelated transitive deps (originals uncached). Proposed follow-up §I.14: drop girsync dep from tempdir |
| T08 | `tracing-appender` as single file-log writer | completed | f11f7ac7 | Stale premise: only one process::exit (parse_args, pre-init_tracing); logging paths already flush. Flush-failure warning + its test removed (WorkerGuard::drop has no error channel); lossy 128k-line queue replaces backpressured BufWriter; README buffer bullet updated |
| T09 | EnvFilter spec via `add_directive` | completed | fb76d39e | console_filter now returns EnvFilter via try_new + add_directive (not builder().with_env_var, which ignores captured RUST_LOG); all 3 consumers updated; sync redb=warn now appended after RUST_LOG (obscure conflict behavior changes to hard-floor intent); warn/fallback preserved; no with_default_directive/parse_lossy |
| T10 | Directory walkers → `walkdir` | completed | 392d5cbb8f5242ff2074b69561d396cd8fffb0e7 | Premise stale: walkdir only in root/sync, added to gircap; fault-injection seam reshaped to pre-descent access check, child_dirs removed, 5 error tests rewritten; loop/dangling-link errors skipped to match old behavior; MANIFEST boundary + sorted order preserved |
| T11 | Percent encoding → `percent-encoding` | completed | bfdb5122 | Literal AsciiSet plan impossible in 2.3.2 (escapes all non-ASCII); used percent_encode_byte primitive; kept minimal 5-char escape set; old decoder accepted %+A sign edge now passed through; added explicit non-ASCII pinning test |
| T12 | Windows device-name / path-shape rule | skipped | | Share-one-rule premise already met (config calls paths::is_windows_device_name); no library predicate matches current reserved set - sanitize-filename adds COM0/LPT0 and omits CONIN$/CONOUT$, changing artifact layout |
| T13 | girpr JSON renderer → `Serialize` | completed | 69414ccb, 9177fde1 | Hand-written Serialize for local Record/Val; json() = serde_json::to_string with {} fallback. Intentional byte change in untested edge: backslash-only string values now escaped to \\ (control-char escaping unchanged); declaration order kept, no preserve_order |
| T14 | girsync JSON renderer → `Serialize` | completed | 8607fac2 | Kept alphabetical key order (event in sorted slot) - pinned by byte-level oracle; stale pointer: hand-rolled renderer was write_json, json() is public -> Value, so routed through to_value; no byte changes; no preserve_order |
| T15 | gircap JSON renderer → `Serialize` | completed | e5d6426a | Backslash caveat stale: current helpers already excluded b'\\\\' so serde_json emits \\\\ already; byte-identical, no oracle change. Kept Str|Int|Bool event-first order; test calling removed helper rewritten via public API; design note corrected |
| T16 | girpr retry/backoff → `backoff` | completed | acb8d6c4 | Adopted backoff 0.4.0 (latest stable); kept Retry/RetryState with Transient/Permanent mapping in adapter; exact attempt count enforced; default SystemClock; sleep.rs deleted, BackoffPolicy exported; determinism loss accepted per §I.3; gircap untouched |
| T17 | gircap retry/backoff → `ExponentialBackoff` | completed | bc6940c3 | Used ExponentialBackoff with randomization_factor 0.0 (jitter None, diffable artifact preserved); kept retry loop + Sleep injection + seq/bytes_in/restart; no async driver/middleware; base_delay/max_delay folded into private constants |
| T18 | girsync path canonicalization → `dunce` | completed | 97bb62dd | Scope broadened to all canonicalize sites in tools/sync (no std/dunce mixing); 3 strippers removed; UNC test body retargeted to absolute_alias; DIRECT_QUESTIONS seam updated; I.1 validated - girpr/T19 untouched; paths render without \\\\?\\ |
| T19 | girpr `resolve_existing_prefix` validation | skipped | | No library method preserves symlink-escape detection + fail-closed behavior; path-absolutize methods are purely lexical and its canonicalize() does not exist; §I.1 open assumption confirmed |
| T20 | Glob filtering → `globset` / shared crate | completed | aaa7659a | Chose shared leaf crate girpr-filter (semantics-preserving); globset rejected due to dotfile/**/backslash/case-mode drift risk; both filters delegate but keep their own policies; girsync::glob::Pattern public type unchanged, public_surface test passes |
| T21 | Atomic publish helpers → library | completed | 8e3ee9be | NamedTempFile::persist for write_atomic + pure copy_publish_with; lexical guards/create_dir_all/durable decision kept; promote_tmp + audio cache move untouched; temp_sibling kept behind cfg(test) as legacy fixture; sync-counting seam test replaced with observable behavior |
| T22 | Status error + URL join → `reqwest` | completed | b656f563 | Used Url::join + error_for_status_ref; normalized base to directory URL (append /) so production prefixes without trailing slash keep build segment; narrow retryable policy (server_error + 408/429) preserved; join_url kept as reqwest-backed wrapper |
| T23 | gircap HTTP mock servers | completed | 8f181ace | Used wiremock 0.6 for canned replies with expect(n)/received_requests(); kept HttpTransport/MemoryTransport seam (domain server not delegable, manifest bodies pin ports); raw-socket helper retained for 9 lying/omitted Content-Length tests; domain state preserved; stale serve_bodies/Served pointers belonged to T24/bridge; md5_hex duplicate switched to gircap::hash::md5_hex |
| T24 | girpr canned HTTP reply server | completed | b4055b87 | Replaced raw-socket test_http server with wiremock 0.6; Mock handle with uri() + async hits() via received_requests(); serve_status takes u16; no expect(n) (unbounded repeat-last, counts via journal); bridge/domain servers untouched; workspace 2114 passed |
| T25 | Hardlink identity → `same-file` | completed | fc2717c5 | FFI removed; same-file 1.0.6 has no FileId so used same_file::Handle; winapi-util for Windows link count; windows-sys dropped; link-count >1 guard preserved (identity never primary refusal); thin platform seam kept |
| T26 | girsync gate/concurrency simplification | completed | 15f4c5db | Used sanctioned tokio::sync::Semaphore fallback (not pure removal): Split needs cross-side min(src,dst) bounding and try_acquire/in_flight cannot come from pool sizing; src-then-dst ordering + reverse-order Drop preserve deadlock freedom; in_flight() kept; tokio sync feature only, no new crate |
| T27 | Cache wire codec → `postcard`/`bincode` | completed | 1e0e2450 | Deleted codec.rs, added wire.rs; postcard 1.1.3; custom DigestSet serde preserves iter()+foreign order and unknown-name carry; CACHE_VERSION 2→3, marker 1→2; §I.9 confirmed (old caches refused, rebuild via --ignore-cache); foreign names sorted on serialize for determinism; forward-compat tests pass |
| T28 | `cache/open.rs` publish protocol restructure | completed | 9bbdc130 | Extracted backup/ with Publisher trait (FsPublisher) + narrow PublishHooks trait (CacheHooks); open.rs keeps claim protocol/registries/retention/42 tests; delegated working names to tempfile; atomicwrites not used (offline cache + would break no-half-copy invariant); one-syscall/fail-fast docs and §H/§A11 items untouched; claim_sibling test-only |
| T29 | girpr INI parse/merge/render | skipped | | Byte-level config.ini contract matters; configparser/ini regress comment/dup-section/case-key/headerless/quote/sort/blank-line parity; parser-only saves no net LOC. Keep hand-rolled core. §I.2 validated |
| T30 | gircap `parse_game_version` | completed | ce97c9a6 | Used ini_core streaming parser (not configparser/ini, which are last-wins and would break first-match); thin adapter keeps lenient [General] header tolerance; byte-exact config_ini renderer untouched; first-match-in-[General] pinned + 33k-input differential check 0 mismatches |
| T31 | `JoinSet` window → `buffer_unordered` | pending_review | e1da9861 | JoinSet window replaced with buffer_unordered over tokio::spawn handles (keeps parallelism + JoinError panic capture); from_fn iterator preserves fail-fast no-new-file rule; accepted detach-on-drop (driver always awaited to completion, runtime drop aborts anyway); no futures-concurrency needed |
| T32 | Chunk-range tiling → interval crate | skipped | | Weakest crate case confirmed: measured rangemap rewrite +30/-15 net +15 lines; overlaps need chunk-offset identity crates discard; i64 validation un delegatable; is_tiled fast path must stay allocation-free; §I.5 |
| T33 | Error `Display` impls → `thiserror` | pending_review | 69384691 | Derived thiserror::Error for ApiError/UsageError/Failure with verbatim format strings; exit_code/url/summary kept hand-written; messages byte-identical; Failure gains std::error::Error impl (additive, required by task) |
| T34 | sled2redb temp claim → `NamedTempFile` | in_progress | | |
| T35 | `IntOrStr` → `serde_with` (optional) | todo | | |

## Reviews

| ID | Batch (task IDs) | State | Note |
|----|------------------|-------|------|
| A | T01, T02, T03, T06 (T04, T05 skipped) | done | All four completed; no fixes/reverts; 4 follow-ups recorded |
| B | T07, T08, T09, T10, T11 (T12 skipped) | done | All five completed; lockfile fix 4636a691; 5 follow-ups recorded |
| C | T13, T14, T15 | done | All three completed; cross-tool consistency PASS; fix 9177fde1 pins backslash escaping; 2 follow-ups recorded |
| D | T16, T17 | done | Both completed; two-semantics comparison PASS, no cross-leak; no fixes; 4 follow-ups recorded |
| E | T18, T20, T21, T22 (T19 skipped) | done | All four completed; no fixes; DO-NOT-TOUCH resume artifacts preserved; 6 follow-ups recorded |
| X | gircap spawn-test investigation/fix (c7e34ba4) | done | Failure predates orchestration; all 27 were env-only; fixed via CARGO_BIN_EXE_gircap; F04 resolved; review APPROVE; residual compile caveat disproven |
| F | T23, T24 | done | Both completed; no fixes; all 9 malformed-Length tests retained; bridge/domain servers untouched; 5 follow-ups recorded |
| G | T25, T26 | done | Both completed; no fixes; link-count guard and deadlock freedom verified; DO-NOT-TOUCH items untouched; 5 follow-ups recorded |
| solo | T27 | done | Completed; no fixes; forward-compat/rejection/version-gate verified; DO-NOT-TOUCH intact; 3 follow-ups recorded |
| solo | T28 | done | Completed; no fixes/reverts; moved bodies verified behavior-identical, one-syscall gaps/fail-fast/DO-NOT-TOUCH intact; 4 follow-ups recorded |
| H | T30 (T29 skipped) | done | Completed; no fixes; byte-exact renderer untouched, first-match-in-[General] preserved; 3 follow-ups recorded |

## Follow-ups

| ID | Description | State | Note |
|----|-------------|-------|------|
| F01 | Collapse girsync ts_now shim; call girpr_log::timestamp directly in cache/open.rs | open | Low-payoff cleanup (from T02) |
| F02 | Optional second hex sweep of non-digest u8::from_str_radix / format sites | open | Not digest-hex; out of T03 scope |
| F03 | Restore deterministic interleaving test only if saturating logic returns in-house | open | Barrier contention suffices today (from T06) |
| F04 | Make gircap spawn tests locate the binary robustly via a runner-provided env var | done | Fixed by c7e34ba4 using CARGO_BIN_EXE_gircap; predates orchestration; full suite green |
| F05 | Consider file-log backpressure via lossy(false) or surface dropped_lines at exit | open | T08; normal runs never approach 128k queue limit |
| F06 | Drop girsync dev-dependency from crates/tempdir behind a test-support feature | open | T07; build-weight only; proposed by implementer |
| F07 | Pin the strict percent-decoder %+A pass-through with a regression test | open | T11; encoder never emits a plus escape |
| F08 | Restore real WalkDir directory-read failure coverage in walk tests | open | T10; production path covered indirectly by passing tests |
| F09 | Confirm girsync redb=warn hard-floor precedence after RUST_LOG is wanted | open | T09; affects only rare RUST_LOG naming redb |
| F10 | Document/test one shared escaping-consistency invariant across the three JSON renderers | open | Batch C; each crate already pins its own bytes |
| F11 | Reconsider silent {} / empty-Value serialization fallbacks in the three renderers | open | Batch C; fallback unreachable for current primitive fields |
| F12 | Note that default 5-attempt girpr retry wait grew from ~10s to up to ~22.5s | open | T16; explicit goal, only slows failing runs; fail-fast unaffected |
| F13 | Restore 'after 0 attempts' error context for retry(0, ...) in girpr | open | T16; no call site passes 0 |
| F14 | Document why backoff 0.4 RNG draw at factor 0.0 cannot affect gircap artifacts | open | T17; use custom policy only if strict zero-RNG ever required |
| F15 | Consider public API impact of RetryPolicy base_delay/max_delay becoming private constants | open | T17; no in-tree external users |
| F16 | Rename stale UNC test without_verbatim_rebuilds_the_prefix_the_path_needs | open | T18; now tests absolute_alias, name only |
| F17 | Document junction read_link spelling assumption relied on by absolute_alias | open | T18; low risk, not observable in current tests |
| F18 | Add cross-crate equivalence test for girpr-filter | open | T20; extraction behavior-preserving by construction |
| F19 | Restore an observable durability assertion for write_atomic_with durable=true | open | T21; seam loss allowed, durable branch remains |
| F20 | Add successful Windows overwrite test for NamedTempFile::persist | open | T21; extra assurance only |
| F21 | Note 3xx no longer treated as failure and join_url signature String to Result<Url> | open | T22; in-tree callers updated, redirects followed by default |
| F22 | Correct M0/M10 commit hash citations for the bin() helper for traceability | open | Ad-hoc review X; active branch has d48a485d/7fd70570 |
| F23 | Add a guard comment/test that CARGO_BIN_EXE_gircap is set to prevent regression to current_exe() | open | Ad-hoc review X; non-blocking |
| F24 | Review the CARGO_BIN_EXE_* pattern for any future spawn-based integration tests in other tool crates | open | Ad-hoc review X; non-blocking |
| F25 | Replace T23 Drop-time expect(n) with explicit verify()/hits() assertion for cleaner failure diagnostics | open | Batch F; counts already pinned; failure-diagnostics only |
| F26 | Re-evaluate deleting HttpTransport/MemoryTransport seam contingent on a fast real-server 610-chunk test | open | Batch F; seam intentionally retained, ports pinned by fixtures |
| F27 | Track added test-build weight from wiremock dev-dependency in girpr and gircap | open | Batch F; dev-only, no shipped-artifact impact |
| F28 | Make retained raw-socket helper fail fast on unexpected extra request instead of hanging to timeout | open | Batch F; all 9 malformed tests consume exact scripted counts |
| F29 | Note serve_status now takes u16; custom reason phrases would need responder customization | open | Batch F; current tests assert numeric status only |
| F30 | Windows share-mode assurance test for identity_of opening a second handle per claim | open | Batch G; DuplicateHandle inherits share flags; extra assurance only |
| F31 | Note ReFS identity uniqueness caveat of same-file 1.0.6 on Windows | open | Batch G; identical to old behavior, link-count guard is primary |
| F32 | Note transient in_flight() under-report window under tokio wait queues | open | Batch G; diagnostics-only, 30x stress showed no return |
| F33 | Strengthen T26 deadlock test with asymmetric split(1,2)/split(2,1) barrier case | open | Batch G; invariant structurally covered |
| F34 | Note blocking_acquire() panics inside a tokio runtime if async is ever introduced | open | Batch G; crate fully synchronous today |
| F35 | Reword or acknowledge cobs in postcard manifest comment (non-optional dep) | open | Solo T27; tiny crate, zero behavior impact |
| F36 | Optional streaming Deserialize visitor to cut per-record decode allocations | open | Solo T27; rows small, not on hot path |
| F37 | Add multi-foreign-name determinism coverage to cache codec tests | open | Solo T27; hardens already-correct sorted-output behavior |
| F38 | Point open.rs one-syscall-gap module docs at backup::publish submodule | open | Solo T28; documentation clarity only |
| F39 | Add production-path working-name collision test for tempfile-based working_name | open | Solo T28; tempfile guarantees uniqueness upstream |
| F40 | Consolidate duplicated remove_cache_path in open.rs and publish.rs | open | Solo T28; duplication intentional and behaviorally inert |
| F41 | Re-evaluate Publisher trait if no second implementation appears | open | Solo T28; harmless abstraction with testability value today |
| F42 | Document incidental #-line tolerance in ini_core version reader | open | Batch H; # lines ignored as non-matching keys, behavior identical |
| F43 | Note widened (not narrowed) leniency for indented header-like lines with = | open | Batch H; malformed line never produced by config_ini |
| F44 | Check in compact golden corpus from the 33k-input differential check | open | Batch H; curated tests already pin the relevant shapes |

## Persistent context

- gircap spawn-based integration tests: the os error 2 "cannot find gircap.exe"
  failures PREDATE the orchestration (reproduced at baseline 36d2a030). Root cause
  was nextest placing test binaries in build-script OUT_DIR, invalidating the
  current_exe()-relative bin() lookup. Fixed in c7e34ba4 by using
  env!("CARGO_BIN_EXE_gircap"); all 2114 workspace tests pass under both cargo
  test and cargo nextest with no manual binary copying (report §F, T23-T24).
- INI migration (T29/§A4) ruled out: the byte-level config.ini contract matters
  (deliberate Starward/Collapse parity: drop comments/blanks, merge duplicate
  sections, case-insensitive key dedup + sort, headerless→[General], strip quotes,
  trailing blank line, atomic UTF-8 publish; pinned by 20 tests). configparser/ini
  regress several rules and would need a re-implementation layer; parser-only
  adoption saves no net LOC and adds a fallible path. Keep hand-rolled parse/merge/
  render in girpr; gircap config_ini renderer is byte-exact and out of scope (T30).
  See §I.2.
- JSON-renderer decision (§A1, from T13 commit 69414ccb), reuse for T14/T15:
  hand-write Serialize per crate on its own local Record/Val with
  serialize_map(Some(n)) + serialize_entry in THAT tool's own order; json() =
  serde_json::to_string(self). Do NOT enable serde_json preserve_order (emission
  order is preserved regardless of workspace feature unification). Each tool
  keeps its own field order (girpr: event-first then declaration; gircap:
  Str|Int|Bool event-first; girsync: alphabetical) - do not copy a sibling's.
