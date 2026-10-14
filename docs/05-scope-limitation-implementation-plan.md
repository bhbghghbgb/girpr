# Scope-of-work limitations — implementation plan

Spec: `docs/04-scope-limitation-options.md`. Pipeline spec:
`docs/02-repair-chunk-spec.md`. Module map: `docs/03-rust-implementation-plan.md`.

**Status:** plan, nothing built. Phases are ordered by dependency, not by
interest. Each phase is one commit and ships green.

**Two invariants for every phase.**

1. **An unflagged run is byte-identical to today's.** No new flag changes a
   default, reorders a log line, or alters a counter. Every phase is verifiable
   by running the existing suite unchanged — if a phase cannot pass the existing
   suite without editing it, the phase is wrong, not the suite.
2. **The two crates stay independent.** Nothing in `tools/capture` or
   `tools/sync` is imported, and no `girpr` contract leaks into either. A `Budget`
   idea is lifted as a *shape* and retyped (`gircap/docs/00-spec.md` §2).

---

## Phase 0 — the plan restructure (prerequisite, no new flags)

### 0.1 Why it is first, and why it is not optional

`--print-plan` needs a plan. `--max-delete-bytes` needs to know how much the
purge *would* delete. Neither is answerable today, because the pipeline decides
what to purge **after** it has already repaired, inside the same call that deletes
it:

```
repair::run (today)
  7.  purge_before -> purge_extra(walk, classify, delete)      <-- decides and acts
  8.  check_only  -> return
  9.  repair_all                                               <-- mutates
  11. post::run -> delete_deprecated -> audio move -> purge_extra(...) -> config.ini
                                                                       ^ decides and acts
```

`purge_extra` walks and deletes in a single pass. There is no instant at which
"how much would this purge delete" is known without having begun deleting, so
B4's refusal has no question to ask and C1 has nothing to print. `girsync` solved
this by splitting `planner/` from `apply`; `girpr` needs the same seam.

### 0.2 Target shape

```
A. prepare game_dir (canonicalize; CREATE only at execution time — see 0.5)
B. metadata::fetch                                        [unchanged]
C. emit REPORT                                            [unchanged]
D. fetch_latest manifests + local_reuse_map               [unchanged]
E. plan::build_plan                          -> RepairPlan
F. plan_purge (when purge_before | purge_after | print_plan) -> PurgePlan
G. deprecated_files (moved up from the post-phase) -> Vec<String>
H. ===== everything above this line is read-only =====
I. execute: purge_before -> check_only | repair_all -> post
```

`F` and `G` complete the plan. Everything after `H` consumes a decided list.

### 0.3 Step 1 — split `purge_extra`

`src/repair/purge.rs` becomes `src/repair/plan/purge.rs` (the module layout in
`docs/04` §5). Current one function becomes three:

```rust
pub struct PurgeCandidate { pub path: PathBuf, pub rel: String, pub size: u64 }
pub struct PurgePlan {
    pub files: Vec<PurgeCandidate>,  // sorted by path — same order as today
    pub dirs:  Vec<PathBuf>,         // emptied by this purge, deepest-first, game_dir excluded
    pub total_bytes: u64,
}
pub fn plan_purge(game_dir: &Path, plan: &RepairPlan, server_keep: &HashSet<String>) -> PurgePlan;
pub fn apply_purge(p: &PurgePlan, dry_run: bool) -> u64;   // -> deleted bytes, 0 in dry-run
```

- `plan_purge` is the current `WalkDir` + `classify_purge_path` + sort, with the
  delete loop removed. It gains a post-order pass to derive `dirs`: a directory is
  swept iff it has **zero `Keep`-classified descendants**, computed during the same
  walk with a per-directory kept-count map. Exclude `game_dir` itself.
- `apply_purge` is the current delete loop, unchanged line for line, plus
  `remove_dir` over `p.dirs` **still best-effort** (`let _ = …`). A directory can
  gain a file between plan and apply — the game may be running — and the applier
  must not be the thing that fails. Keep every `tracing` line in `apply_purge`, so
  `--dry-run` output is identical to today's.
- `remove_emptied_dirs`' re-walk disappears; it was only reachable to call
  `remove_dir`, which is now driven by `p.dirs`.

`classify_purge_path` and `keep_set` do not change in this step. Not even a
parameter.

### 0.4 Step 2 — hoist the deferred deprecated-file list

`post::delete_deprecated` calls `hyp.deprecated_files()` lazily, in the post-phase.
Move the *call* into the plan phase; keep the delete in the post-phase. It is
best-effort today (warn + continue) and stays so — only the log position moves,
earlier, into the plan.

### 0.5 Step 3 — move `create_dir_all` to execution time

`repair::run` creates `game_dir` before anything else, unless read-only. Under the
new order that is wrong: a run that refuses on a ceiling must have created
nothing. Move it to the first line of the execute half.

Two consequences, both handled:

- `canonicalize` at the top now tolerates a missing dir (it already falls back to
  the raw path). `rel_path` strips `game_dir` from `game_dir`-joined paths, so
  both sides use the same value and the fallback is consistent.
- `plan_purge` on a missing dir: `WalkDir` yields an error entry, which the
  existing `filter_map(|e| e.ok())` drops, giving an empty plan. Correct — a
  fresh install has nothing to purge.

This is the one Phase 0 step that changes behaviour with no flags (a failed run no
longer leaves an empty directory behind). It is called out here so it is a
decision rather than a surprise; the suite does not currently cover it.

### 0.6 Step 4 — de-duplicate the dry-run purge pass

Today `--purge-before --purge-after --dry-run` calls `purge_extra` **twice** and
logs every candidate twice: once from the `purge_before` branch in `run`, once
from the `else if ctx.dry_run && ctx.purge_after` branch in the post-phase. With
a planned purge the apply happens once, at whichever position the flags select
(`purge_before` position if set, otherwise the post-phase position).
`--check-only` still never purges.

Fixing this is not the point of Phase 0; it is a consequence of it, and it is
listed so the diff is expected rather than surprising.

### 0.7 Phase 0 acceptance

- The existing suite passes **unmodified**. `tests/repair_offline.rs` covers both
  purge timings, dry-run purge logging, check-only, repair, reuse and resume;
  `tests/cli_contract.rs` covers the exit contract. None of them may be edited in
  this phase.
- `repair::run` has no branch that decides and acts in the same call.
- No new flag, no changed default, no changed `SUMMARY`.
- Commit: `girpr: build the whole plan (repair + purge) before mutating anything`

---

## Phase 1 — path scope

Unblocks the named blocker: `gircap/docs/00-spec.md` §8.1.

### 1.1 Step 1 — the filter

New `src/repair/scope/filter.rs`, ported from `tools/sync/src/filter.rs`:

```rust
pub struct PathScope { includes: Vec<Pattern>, excludes: Vec<Pattern> }
pub fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>>   // exit 1 on a bad glob
pub fn is_visible(&self, rel: &str) -> bool                        // exclude beats include
```

`MatchOptions { case_sensitive: false, require_literal_separator: false,
require_literal_leading_dot: false }` — `girsync`'s defaults, hardcoded. Matched
against the `/`-separated game-dir-rooted rel path, the same string
`plan::build_plan` stores. `glob` is already in `Cargo.lock` via `girsync`, so the
dependency edge resolves offline.

`glob` becomes a `girpr` dependency. No shared crate with `girsync` — the filter is
35 lines and the independence rule outranks the deduplication.

### 1.2 Step 2 — thread the scope

| site | change |
|---|---|
| `plan::build_plan` | takes `&PathScope`; drops non-visible `rel` **before** the plan exists, so `files_total` counts only what survives |
| `purge::classify_purge_path` | gains a `&PathScope`; a hidden path returns `Keep` **before** the manifest check |
| `purge::plan_purge` | same, via the classifier |
| `purge::PurgePlan.dirs` | the kept-count pass already sees hidden paths as `Keep`, so swept dirs come out right for free |
| `post::delete_deprecated` | skips a hidden path |
| `post::move_audio_cache` | skips a hidden source or target |
| `post::write_config_ini` | **skipped entirely when a scope is active** (`docs/04` §2.2); `warn` + `SUMMARY config_bumped=0` |
| `metadata` audio-scan write | refused at the CLI layer, see 1.3 |

`classify_purge_path` stays a pure function with its exhaustive unit test; the
scope is one more argument. The alternative — widening `server_keep` with every
hidden path — is rejected in `docs/04` §2.1.

### 1.3 Step 3 — the two guards

- `--include` present and matching zero planned paths → exit `1`, globs named.
- Explicit `--audio` under an active scope → exit `1`, **unless**
  `GameConfig.audio_pkg_scan_dir`'s rel path passes `is_visible`. The scan file is
  one of the few paths a scoped run is allowed to write, so it has to be in scope
  like anything else. The path comes from `getGameConfigs`; never hardcode it.

### 1.4 Step 4 — tests

Unit, in `filter.rs`: include-only, exclude-only, exclude-beats-include, empty
include admits everything, `*` crossing `/`, case-insensitivity, backslash
normalization, bad pattern → `Err`.

Unit, in `purge.rs`: `classify_purge_path` with a scope — a hidden manifest path,
a hidden orphan, a hidden `config.ini` sibling, a hidden `*_tmp`, and the
emptied-dir derivation under a hidden-only directory.

Integration, in `tests/repair_offline.rs`, against the existing `MockServer`: an
include over one fixture file repairs exactly that file (assert the exact chunk
download set via `mock.chunk_hits()`); a purge under a scope leaves the fixture's
other files on disk; `config.ini` is not bumped; `--audio` under a scope refuses.

Binary-level, in `tests/cli_contract.rs`: bad glob → `1`; `--include` matching
nothing → `1`; `--audio` + `--include` → `1`.

Commit: `girpr: --include / --exclude path scope (excluded paths do not exist)`

---

## Phase 2 — ceilings

Refusal, not admission (`docs/04` §1.5). Every one exits `5`.

### 2.1 `--max-files <N>`

`n = plan.files.len()` on the **filtered** plan. `n > N` → `RunFailure::limit`,
exit `5`, before `create_dir_all`. Parsed as `Option<usize>`; `0` is a valid
ceiling and is *not* a usage error — whether it trips depends on the server's
answer, which is why this is `5` and not `1` (`docs/04` §4.4).

### 2.2 `--max-download-bytes <N>`

**Pre-flight.** Needs `est_download_bytes` per planned file, added to
`plan::PlannedFile`:

```
Σ c.compressed_size for c in chunks where (c.uncompressed_md5, c.uncompressed_size) ∉ local_map[rel]
```

`local_map` already exists at that point (`manifest::local_reuse_map`). Compute it
in `build_plan` — pure, no I/O, and it is exactly the capture's `patch_fetch`
formula, so the `PLAN` line and the eventual `SUMMARY download_bytes` agree by
construction. `Σ > N` → exit `5`.

**Runtime.** The estimate is a **lower bound** — `try_reuse_slice` falls back to a
download when a local slice fails its MD5 — so refusing on it alone would bound
nothing. One `AtomicU64` on `Summary`, checked before each `fetch_chunk_bytes`;
crossing it drops the remaining files, leaves their `*_tmp` in place (S3
resume-by-length makes an aborted run a smaller run, not a corrupt one), sets
`stopped=bytes`, and exits `5`.

### 2.3 `--max-delete-bytes <N>`

`PurgePlan.total_bytes`, computed **once over the union of both purge passes** —
not per pass, or a low ceiling would refuse `--purge-before` for work that
`--purge-after` would have done anyway. Over the limit → exit `5`, nothing
deleted. This is the flag that exists because Phase 0 does.

### 2.4 `--max-write-bytes <N>` — later, same shape

The only ceiling that cannot be a refusal: reuse success is decided per chunk at
runtime, against bytes that may not be there. One `AtomicU64`, incremented at the
two `write_all` sites in `file.rs` (`try_reuse_slice`'s copy and
`write_downloaded_chunk`'s decoded write). Defer to Phase 4 — it is P1.

### 2.5 Step — tests

- refusal writes nothing: no `game_dir` created, no `_tmp`, no deleted file, no
  `config.ini` write;
- `--max-files N` with `plan.files.len() == N` does **not** refuse (the boundary is
  the point);
- `--max-download-bytes N` where `Σ est == N` does not refuse;
- `--max-delete-bytes` refuses and deletes nothing;
- exit `5` appears in `tests/cli_contract.rs` as its own assertion, distinct from
  the "every arg error is `1`" rule.

Commit: `girpr: enforce byte/file ceilings as pre-flight refusals (exit 5)`

---

## Phase 3 — `--print-plan`

Run phases A–H, emit the `PLAN` line, stop. Nothing is executed.

```
PLAN files=8 est_download_bytes=2373385 purge_files=0 purge_bytes=0 \
     max_chunk_bytes=3689476 scoped=1
```

`--plan-out <FILE>` writes `{"repair":[{rel,size,chunks_total,est_download_bytes}],
"purge":[{rel,size}]}`. The format is still open — see `docs/04` §7.3.

The `purge` half depends on Phase 0's `plan_purge`; if Phase 0 slipped, C1 can ship
repair-only and be finished after.

**Not an offline mode, and the help says so.** Fetching the manifests is not part
of what `--print-plan` does differently — every run does it, like the API call
before it. It fetches, at the usual log level. The plan *is* the manifest; there is
no plan without one.

Read-only by construction: it returns before `create_dir_all`. It does not imply
`--check-only` — it answers "what would you do", not "what is wrong". It composes
with both, and a ceiling trips it like any other mode (`docs/04` §4.5).

Commit: `girpr: --print-plan (PLAN line + --plan-out JSON, read-only)`

---

## Phase 4 — second milestone

| option | why now |
|---|---|
| `--hyp-base` / `--sophon-base` | `RunCtx` already has the fields and `api_bases` already resolves explicit > env > production; `Args::into_ctx` just hardcodes `None`. Needed by the artifact-serving shim. Flag the caveat: this redirects `getBuild` **only** — manifest/chunk URLs come from `url_prefix` in the response bodies (`docs/04` §3 D1). |
| `--fail-fast` | ~85 s of sleeping per unreachable chunk, times a real plan. A mock-server test pointed at a broken endpoint hangs for hours instead of failing. 15 lines. |
| `--max-write-bytes <N>` | closes the last un-counted write path (§2.4). |
| `--purge-only` | the `removed` matrix class is only reachable through the purge path, and `gircap`'s own procedure says not to pass a purge flag. Phase 0 makes this a one-line early return beside `--check-only`. |
| `--fail-fast` + `--purge-only` + `--max-chunk-bytes` + `--no-skip-intact` | E3/E5 are P2 and can be cut if the surface needs trimming. |

Commit per option; they are independent.

---

## Test strategy

Three layers, unchanged in shape from `docs/03` §4.

**Unit** (`#[cfg(test)] mod tests` next to the code): the filter, the ceiling
predicates, `est_download_bytes`, `plan_purge`'s classification and emptied-dir
derivation, `format_plan_line`, and the conditional `SUMMARY` keys. All pure, all
filesystem-free.

**Integration** (`tests/repair_offline.rs`): one `#[tokio::test]` per behaviour,
each against the existing `MockServer`, asserting `(summary, exit code)` plus
on-disk state. `MockOpts` gains nothing — the fixture is already two files and
three chunks, which is enough to make a ceiling trip.

**Binary** (`tests/cli_contract.rs`): the new flag surface and the exit contract.
The existing "every argument error is `1`" rule needs one carve-out — `5` is not
an argument error — so it becomes an explicit assertion rather than a blanket.

**Nothing new touches the network.** The existing `GIRPR_HYP_BASE` /
`GIRPR_SOPHON_BASE` env seam is what every offline test already uses; `--hyp-base`
only makes it reachable from a flag.

---

## Risks

| risk | mitigation |
|---|---|
| Phase 0 changes behaviour with no flags (the `create_dir_all` move) | one documented diff; add an integration case that a failing run leaves no directory behind |
| A ceiling estimated low lets a run exceed it | B2's runtime hard stop; the pre-flight estimate is documented as a lower bound, not a promise |
| Phase 0 is big and lands reviewable | split into the four steps of §0.3–§0.6, each independently green; they are separable commits if the split helps |
| Scope filtering silently deletes the fixture through purge | `classify_purge_path` returns `Keep` before the manifest check, with an exhaustive unit test on hidden paths |
| `SUMMARY` grows and breaks a parser | every new key is emitted **only** when its flag is active — an unflagged run is byte-identical |
| exit `5` surprises a caller | additive; README's contract table updated in the same commit as the flag that introduces it |

---

## Definition of done

Phase 0 + 1 + 2 + 3, plus:

- [ ] `docs/04` §7.3 open items closed and the spec's decisions updated to match
- [ ] README flag table, machine-contract table and exit-code table updated
- [ ] `gircap/docs/00-spec.md` §8.1 blocker marked resolved, §8.1.1 rewritten
      from a manual procedure to the invocation `girpr` now supports
- [ ] the **artifact-serving shim** (serve a `gircap` bundle on `127.0.0.1` with
      each body's `url_prefix` rewritten to the shim's own origin) — a consumer
      side artifact, outside this plan, and the thing that finally makes the
      end-to-end assertion automated