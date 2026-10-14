# Implementation plan — `girpr` scope-of-work limitations

Build order, module tree, acceptance checks and the risks worth knowing before
milestone 0 starts. Companions:
[`04-scope-limitation-options.md`](04-scope-limitation-options.md) (what and
why). The repair-spec series (`01-`, `02-`, `03-`) is background, not a target
— see §1.

---

## 0. Decisions

| decision | value |
|---|---|
| trigger | `gircap`'s end-to-end check is blocked on `girpr` having no path scope (`tools/capture/docs/00-spec.md` §8.1) |
| ceiling semantics | **refuse / hard-stop**, never admission — [`04` §1.5](04-scope-limitation-options.md) |
| new exit code | **`5`** = a manually requested enforcement limit was exceeded. Not `1`: the API must be fetched first and whether a limit trips depends on the answer |
| `--max-files 0` | a valid ceiling, **not** a usage error; trips or does not depending on the plan |
| admission (take the first N) | **out of scope**; it wants `--first`/`--limit` and is not an enforcement |
| `config.ini` under a scope | **not bumped** — a scoped run has deliberately not put the bytes in place |
| the repair-spec docs | **not modified**; this plan's own file is the only doc that changes (§1) |
| `tools/capture`, `tools/sync` | **not modified, not depended on.** The filter is ported from `girsync` and the budget *shape* from `gircap`; neither crate is imported |
| commit discipline | **one commit per milestone. No milestone lands uncommitted.** |

---

## 0.1 Landed

Appended per milestone, newest last. What each one actually did, and anything it
discovered about the tool that the repair-spec series (`docs/01-03`) does not
say — §1 of this file is the only place that is allowed to record that.

### M0 — the plan restructure

- `src/repair/plan.rs` → `src/repair/plan/mod.rs`, `src/repair/purge.rs` →
  `src/repair/plan/purge.rs`; `purge_extra` split into `plan_purge` (walk +
  `classify_purge_path` + sort) and `apply_purge` (the delete loop, line for
  line). `classify_purge_path` and `keep_set` took no new parameter.
- `PurgePlan.dirs` is derived **during the same walk** (`mark_kept_ancestors`
  marks the ancestors of every `Keep`-classified file), so
  `remove_emptied_dirs` no longer re-walks. This is equivalent to the old
  "try `remove_dir` on every directory" — a dir with kept content fails
  `remove_dir` harmlessly — but it is decided up front, which is the property
  the later scope work needs.
- The deprecated-file list moved from `post::delete_deprecated` to the plan
  phase as `repair::fetch_deprecated(&meta.hyp)`, and is now an argument to
  `post::run` instead of a lazy call from inside it. The call is the same
  `getGameDeprecatedFileConfigs` request with the same per-launcher channel
  (pinned by `deprecated_files_call_carries_the_per_launcher_channel`), it just
  happens one phase earlier — before the manifest fetches rather than after the
  repair loop.
- `create_dir_all` moved from the top of `run` into the execution phase.
  **The one documented behaviour delta**: a run that stops in the plan phase no
  longer leaves an empty `game_dir` behind. Pinned by
  `a_run_that_stops_before_acting_creates_no_game_dir`.
- `--purge-before --purge-after --dry-run` logged every candidate twice; the
  dry-run purge-after listing is now skipped when purge-before already listed
  it. Non-dry-run runs still get two real passes, because purge-after's plan is
  built after the repair and must see the `*_tmp` files it sweeps.

---

## 1. What this plan does not touch

`docs/01-repair-findings-and-verdict.md`, `docs/02-repair-chunk-spec.md` and
`docs/03-repair-rust-implementation-plan.md` are the **repair spec** series:
findings → spec → Rust module plan for the whole-install patcher that ships
today. They describe `girpr` as it is, and this work does not change that
description enough to be worth amending them.

**No milestone in §3 edits any of the three.** The rules, so "we'll just add a
line" does not happen by accident:

- The three keep describing the shipped tool. `--include` is a new flag, not a
  change to the repair algorithm, and the repair spec is not where a flag list
  lives — the **README** flag table is (§7).
- Anything this work learns about the *spec* (e.g. if M0 turns up that the purge
  ordering is load-bearing in a way `docs/02` does not say) is recorded **here**,
  in that milestone's **Landed** block, and left for a deliberate edit of
  `docs/02` if one is ever warranted.
- **This file is the only doc that grows.** `docs/04` is edited only to close
  §7.2/§7.3 decisions once they are settled.

`gircap`'s docs are edited by neither plan, except for the one line in M7 that
marks the blocker resolved.

---

## 2. Shape of the change

```
src/repair/
├── mod.rs              phase coordinator; gains: plan-then-assert-then-execute
├── plan/               M0 — the plan becomes complete before anything mutates
│   ├── mod.rs          RepairPlan, PlannedFile, est_download_bytes, build_plan
│   └── purge.rs        PurgePlan, PurgeCandidate, plan_purge, apply_purge
│                       (moved out of src/repair/purge.rs)
└── scope/              M1+ — new directory
    ├── mod.rs          the assertion: plans + scope + limits -> proceed | refuse(5)
    ├── filter.rs       PathScope, is_visible, compile_patterns      (pure)
    ├── limits.rs       Limits { files, download_bytes, write_bytes, delete_bytes }
    ├── ledger.rs       the runtime atomics the write/download stops share
    └── report.rs       PLAN line + --plan-out JSON
```

`girsync` made the same split (`planner/` → apply) for the same reason. `mod.rs`
is a facade re-exporting its children; leaf files own one concern; inline
`#[cfg(test)]` tests travel with the code they cover. Inherited, not invented.

New dependency: **`glob`**. Already in `Cargo.lock` via `girsync`, so the edge
resolves offline. A shared crate with `girsync` is rejected — the filter is 35
lines and the two tools' independence outranks the deduplication.

---

## 3. Milestones

Each ends with something runnable, a stated acceptance check, and **its own
commit**. Nothing starts before the previous milestone's check passes, and no
milestone is left uncommitted — if a milestone is abandoned, the work is reverted
or the reason is in the commit message.

Commit messages use `girpr: M<N> — <summary>` as the subject, so
`git log --oneline` reads as the build order.

**Standing invariant, every milestone:** an unflagged run is byte-identical to
today's. No flag changes a default, reorders a log line, or alters a counter. A
milestone that cannot pass the existing suite without editing it is wrong, not the
suite — with the one exception M0 makes explicitly.

### M0 — the plan restructure (prerequisite)

`purge_extra` walks `game_dir` and deletes in a single pass, at the point in the
pipeline where it runs. So there is no instant at which "how much would this purge
delete" is answerable without having begun deleting, and no instant at which the
tool knows what it will do. M4 (`--max-delete-bytes`) has no question to ask and
M5 (`--print-plan`) has nothing to print.

Four steps:

1. **Split `purge_extra`** into `plan_purge` (the existing `WalkDir` +
   `classify_purge_path` + sort, delete loop removed) and `apply_purge` (the
   delete loop, unchanged line for line). `plan_purge` also derives
   `PurgePlan.dirs` — directories with zero `Keep`-classified descendants, via a
   per-directory kept-count map built during the same walk, deepest-first,
   `game_dir` excluded. `remove_emptied_dirs`' re-walk disappears.
2. **Hoist `deprecated_files`** out of `post::delete_deprecated` into the plan
   phase. The call moves; the delete does not. It stays best-effort.
3. **Move `create_dir_all`** to the execution half. A run that refuses must have
   created nothing.
4. **De-duplicate the dry-run purge pass.** Today `--purge-before
   --purge-after --dry-run` calls `purge_extra` twice and logs every candidate
   twice.

`classify_purge_path` and `keep_set` do not change. Not even a parameter.

**Accept:** the existing suite passes **unmodified** — `tests/repair_offline.rs`
covers both purge timings, dry-run purge logging, check-only, repair, reuse and
resume; `tests/cli_contract.rs` covers the exit contract. `repair::run` has no
branch that decides and acts in the same call. No new flag, no changed default,
no changed `SUMMARY`. `cargo clippy --all-targets` clean.

**This milestone is the one exception to the standing invariant**, and only in
step 3: a failing run no longer leaves an empty `game_dir` behind. Say so in the
commit message and add the integration case that pins it.

**Commit:** `girpr: M0 — build the whole plan (repair + purge) before mutating anything`

### M1 — the path filter

`scope/filter.rs`, ported from `tools/sync/src/filter.rs`:

```rust
pub struct PathScope { includes: Vec<Pattern>, excludes: Vec<Pattern> }
pub fn compile_patterns(list: &[String]) -> Result<Vec<Pattern>>
pub fn is_visible(&self, rel: &str) -> bool          // exclude beats include
```

`MatchOptions { case_sensitive: false, require_literal_separator: false,
require_literal_leading_dot: false }` — `girsync`'s defaults, hardcoded, because
game paths are Windows and `girpr` has no business growing a second spelling of
that decision. Matched against the `/`-separated game-dir-rooted rel path, the
same string `build_plan` stores. `glob` added to `Cargo.toml`.

Not wired into the pipeline yet: M1 lands the filter and its unit tests, M3
threads it.

**Accept:** unit tests offline — include-only, exclude-only, exclude-beats-
include, empty include admits everything, `*` crossing `/`, case-insensitivity,
backslash normalization, a bad pattern → `Err` naming the glob. No behavioural
change: the suite is untouched and green.

**Commit:** `girpr: M1 — path filter (glob), ported from girsync's semantics`

### M2 — thread the scope

The filter is only worth having if a filtered path stops existing everywhere,
not just in the repair loop. `plan.expected_paths()` is the **only**
manifest-derived input to the purge keep-set, so narrowing the plan alone makes
purge *destructive* — the fixture `gircap` hands us would be eaten.

| site | change |
|---|---|
| `plan::build_plan` | takes `&PathScope`; drops non-visible `rel` **before** the plan exists, so `files_total` counts only what survives |
| `purge::classify_purge_path` | gains a `&PathScope`; a hidden path returns `Keep` **before** the manifest check |
| `purge::plan_purge` | same, through the classifier |
| `PurgePlan.dirs` | the kept-count pass sees hidden paths as `Keep`, so swept dirs come out right for free |
| `post::delete_deprecated` | skips a hidden path |
| `post::move_audio_cache` | skips a hidden source or target |
| `post::write_config_ini` | **skipped entirely when a scope is active**; `warn` + `SUMMARY config_bumped=0` |

Rejected alternative: widening `server_keep` with every hidden path. It grows a
`HashSet<String>` to tens of thousands of entries, turns `classify_purge_path`
from a pure classifier into one that needs an answer computed elsewhere, and gets
the emptied-dir sweep wrong for free.

Two guards, both exit `1`:

- `--include` present and matching **zero** planned paths → globs named.
- explicit `--audio` under an active scope → refused, **unless**
  `GameConfig.audio_pkg_scan_dir`'s rel path passes `is_visible`. The scan file is
  one of the few paths a scoped run may write, so it is in scope like anything
  else. The path comes from `getGameConfigs`; never hardcode it.

**Accept:** unit tests for `classify_purge_path` under a scope — a hidden manifest
path, a hidden orphan, a hidden `*_tmp`, a hidden `audio_lang_14`, and the
emptied-dir derivation for a directory whose only contents were hidden. Then,
against the existing `MockServer`: an include over one fixture file repairs
exactly that file (assert the exact chunk download set via `mock.chunk_hits()`);
a purge under a scope leaves every other fixture file on disk; `config.ini` is
not bumped; `--audio` + `--include` refuses. Binary-level, `tests/cli_contract.rs`
gains: bad glob → `1`, `--include` matching nothing → `1`.

**Commit:** `girpr: M2 — --include / --exclude path scope; excluded paths do not exist`

### M3 — `--max-files` and `--max-delete-bytes`

The two refusals that need nothing from the network.

- `--max-files <N>`: `plan.files.len()` **on the filtered plan**; `> N` →
  `RunFailure::limit`, exit `5`, before `create_dir_all`. Checking after the
  filter is what makes the flag usable at all — checking before it would refuse
  on every real install.
- `--max-delete-bytes <N>`: `PurgePlan.total_bytes`, computed **once over the
  union of both purge passes**. Per pass would let a low ceiling refuse
  `--purge-before` for work `--purge-after` would have done anyway. Over the limit
  → exit `5`, nothing deleted.

Neither flag admits, selects, or orders anything ([`04` §1.5](04-scope-limitation-options.md)).
Together they are what makes `--include` assertable: the glob selects, the
ceiling verifies the selection.

**Accept:** a refusal writes nothing — no `game_dir` created, no `_tmp`, no
deleted file, no `config.ini` write. `--max-files N` with `plan.files.len() == N`
does **not** refuse; the boundary is the point. `tests/cli_contract.rs` gains
exit `5` as its own assertion, carved out of the existing "every argument error
is `1`" rule.

**Commit:** `girpr: M3 — --max-files and --max-delete-bytes as pre-flight refusals`

### M4 — `--max-download-bytes`

Two mechanisms, and the second is the one that actually enforces.

**Pre-flight.** `est_download_bytes` per planned file, computed in `build_plan`
(pure, no I/O):

```
Σ c.compressed_size for c in chunks where (c.uncompressed_md5, uncompressed_size) ∉ local_map[rel]
```

`local_map` already exists at that point. This is character-for-character
`gircap`'s `patch_fetch` formula, so the `PLAN` line and the eventual
`SUMMARY download_bytes` agree by construction. `Σ > N` → exit `5`.

**Runtime.** The estimate is a **lower bound, not a promise** —
`try_reuse_slice` falls back to a download when a local slice fails its MD5, so
refusing on the estimate alone would bound nothing. One `AtomicU64` on `Summary`,
checked before each `fetch_chunk_bytes`; crossing it drops the remaining files,
leaves their `*_tmp` in place (S3 resume-by-length makes an aborted run a smaller
run, not a corrupt one), sets `stopped=bytes`, exits `5`.

**Accept:** `Σ est == N` does not refuse. The `SUMMARY` keys
`plan_files` / `plan_download_bytes` / `limited=1` appear **only** when the flag
is active, so an unflagged `SUMMARY` is byte-identical to today's. A run whose
reuse path is broken (slice MD5 fails) still trips the runtime stop — an
integration case that corrupts a local slice proves the estimate is not trusted
alone.

**Commit:** `girpr: M4 — --max-download-bytes: pre-flight refusal plus a runtime hard stop`

### M5 — `--print-plan`

Run phases A–H (prepare, metadata, `REPORT`, manifests, repair plan, purge plan),
emit the `PLAN` line, stop. Nothing is executed.

```
PLAN files=8 est_download_bytes=2373385 purge_files=0 purge_bytes=0 \
     max_chunk_bytes=3689476 scoped=1
```

`--plan-out <FILE>` writes `{"repair":[{rel,size,chunks_total,est_download_bytes}],
"purge":[{rel,size}]}` — format still open ([`04` §7.3](04-scope-limitation-options.md)).

**Not an offline mode and it does not pretend to be.** Fetching the manifests is
not part of what `--print-plan` does differently — every run does it, like the API
call before it. It fetches, at the usual log level. The plan *is* the manifest.
The help text says so, because "print the plan" invites the assumption that it is
a local, free operation.

Read-only by construction: it returns before `create_dir_all`. It does not imply
`--check-only` — it answers "what would you do", not "what is wrong".

**Accept:** `--print-plan` on the fixture reports the plan and writes nothing
(no `game_dir`, no `_tmp`, no deletes, no `config.ini` write). A ceiling trips it
like any other mode. `--print-plan --dry-run` is coherent — the plan prints, then
the loop's hashing happens, which is what `--dry-run` asked for.

**Commit:** `girpr: M5 — --print-plan: PLAN line and --plan-out JSON, read-only`

### M6 — `--max-write-bytes`, `--fail-fast`, `--purge-only`

Three independent flags, independent commits; ship together only if that is
easier to review.

| flag | why |
|---|---|
| `--max-write-bytes <N>` | `download_bytes` does **not** count most of the disk the run touches. Every reuse-copy and every downloaded-chunk write goes into `*_tmp` and none of it appears in `download_bytes`. The only ceiling that cannot be a refusal — reuse success is decided per chunk at runtime, against bytes that may not be there. One `AtomicU64` at the two `write_all` sites. |
| `--fail-fast` | One unreachable chunk is ~85 s of sleeping (`MAX_ATTEMPTS = 5` × linear backoff × `fetch_chunk_bytes`'s own 5× retry), before any connection timeout — `HypClient::new_with_bases` sets none. A mock-server test pointed at a dead endpoint hangs for hours instead of failing. 15 lines. |
| `--purge-only` | The `removed` matrix class is reachable **only** through the purge path, and `gircap`'s own procedure says not to pass a purge flag. With M0 done this is a one-line early return beside `--check-only`: phases 1–5 and 7, skipping 6. |

**Accept:** write-bytes stop leaves `_tmp` and exits `5`; `--fail-fast` reaches
its `SUMMARY` in seconds against a mock that returns 500 for every chunk;
`--purge-only` purges and does not repair.

**Commit:** `girpr: M6 — --max-write-bytes, --fail-fast, --purge-only`

### M7 — `--hyp-base` / `--sophon-base`, and closing the loop

`RunCtx` already has `hyp_base_override` / `sophon_base_override`; `api_bases`
already resolves explicit > env > production; `HypClient::new_with_bases` already
takes them. `Args::into_ctx` hardcodes `None`. Two `Option<String>` fields and a
precedence line.

**The caveat, which belongs in the flag help:** this redirects **`getBuild`
only**. Manifest and chunk URLs come from `url_prefix` inside the *response
bodies*, not from the configured base. A mock server must return bodies that
point at itself.

Then, and only then, the one-line edit to `tools/capture/docs/00-spec.md` §8.1
marking the blocker resolved and §8.1.1 rewritten from a manual procedure into
the invocation `girpr` now supports. **This is the only file outside `docs/`
`01-03` and this plan that any milestone touches**, and it is one line.

**Accept:** both overrides reach `HypClient::new_with_bases`; env still works;
explicit still beats env; a mock at `127.0.0.1` serves the whole pipeline.
`cargo test` still green and still offline.

**Commit:** `girpr: M7 — --hyp-base / --sophon-base flags over the existing env seam`

**Not in this plan, but unblocked by it:** the **artifact-serving shim** — serve a
`gircap` bundle on `127.0.0.1` with each body's `url_prefix` rewritten to the
shim's own origin. That is what turns `gircap/docs/00-spec.md` §8.1.1 from a
manual procedure into an automated assertion, and it is consumer-side work in
`tools/capture`.

---

## 4. Test strategy

Three layers, unchanged in shape from [`03-repair-rust-implementation-plan.md` §4](03-repair-rust-implementation-plan.md).

**Unit** (`#[cfg(test)] mod tests` next to the code): the filter, the ceiling
predicates, `est_download_bytes`, `plan_purge`'s classification and emptied-dir
derivation, `format_plan_line`, and the conditional `SUMMARY` keys. All pure, all
filesystem-free.

**Integration** (`tests/repair_offline.rs`): one `#[tokio::test]` per behaviour
against the existing `MockServer`, asserting `(summary, exit code)` plus on-disk
state. `MockOpts` gains nothing — two files and three chunks is enough to make a
ceiling trip.

**Binary** (`tests/cli_contract.rs`): the new flag surface and the exit contract.
The existing "every argument error is `1`" rule gains one carve-out — `5` is not
an argument error — so it becomes an explicit assertion rather than a blanket.

Nothing new touches the network. The `GIRPR_HYP_BASE` / `GIRPR_SOPHON_BASE` env
seam is what every offline test already uses; `--hyp-base` only makes it
reachable from a flag.

---

## 5. Risks

| risk | mitigation |
|---|---|
| M0 changes behaviour with no flags (the `create_dir_all` move) | one documented diff; an integration case pins that a failing run leaves no directory behind |
| M0 is the largest milestone | its four steps are separable commits if review wants them smaller; each is independently green |
| the `est_download_bytes` estimate is low, so a run exceeds the ceiling | M4's runtime hard stop; the estimate is documented as a lower bound, not a promise, and a test breaks the reuse path to prove it |
| scope filtering silently deletes the fixture through purge | `classify_purge_path` returns `Keep` before the manifest check, with exhaustive unit tests on hidden paths |
| `SUMMARY` grows and breaks a parser | every new key is emitted **only** when its flag is active — an unflagged run is byte-identical |
| exit `5` surprises a caller | additive; the README contract table is updated in the same commit as the flag that introduces it |
| the repair-spec docs drift from the tool | §1: this plan's file is the only doc that grows |

---

## 6. Explicitly deferred

- **Admission** — `--first <N>` / `--limit <N>`, "take the cheapest N". It is a
  selection, not an enforcement, and it wants its own name
  ([`04` §1.5](04-scope-limitation-options.md)).
- **`--max-chunk-bytes`**, **`--no-skip-intact`**, **`--manifest-field`** —
  [`04` E3/E4/E5](04-scope-limitation-options.md). Real, but P2.
- **A `--scope-profile` file.** The flags compose; a profile is a second surface.
- **The artifact-serving shim** — consumer-side, belongs to `tools/capture`.
- **Rate limiting.** Not a bandwidth cap: ceilings bound chunk downloads, writes
  and deletes. Metadata and manifest traffic (~2–3 MB per run) happens before the
  plan exists and is not bounded by anything here.

---

## 7. Definition of done

M0 through M5 — the set that unblocks `gircap/docs/00-spec.md` §8.1 — plus:

- [ ] every `SUMMARY` key verified **absent** when its flag is absent
- [ ] README flag table, machine-contract table and exit-code table updated
- [ ] [`04` §7.2](04-scope-limitation-options.md) §7.2 decisions confirmed or
      corrected, and §7.3 closed
- [ ] the three repair-spec docs **unmodified** — `git diff --stat` says so
- [ ] `cargo test` green and fully offline; `cargo clippy --all-targets` clean