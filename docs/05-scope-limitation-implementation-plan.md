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

### M1 — the path filter

- `src/repair/scope/{mod,filter}.rs`, ported from `tools/sync/src/filter.rs`.
  `PathScope::new(includes, excludes)`, `is_visible`, `is_active`, `is_open`,
  `has_includes`. Not wired into the pipeline yet — that is M2.
- `glob 0.3` added to `Cargo.toml`; already in `Cargo.lock` via `girsync`, so
  the edge resolved offline.
- Two refinements over a literal port, both forced by this tool's path shape:
  - `compile_patterns` rewrites `\` to `/` before compiling. The rel paths being
    matched are `/`-separated and a Windows user writing `--include
    "game\*.dat"` means a separator, not a glob escape. The cost is that `\`
    cannot escape a metacharacter, which nobody types at a game install.
  - `is_active` vs `is_open`: an `--exclude`-only scope hides paths, so it
    counts as *scoped* even though no `--include` was given. §4.6's `scoped=1`
    and §2.2's `config.ini` rule care about that; `--include`-matched-nothing
    does not.
- The `glob` crate's `**/` is recursive *and* matches at the root, i.e. `**/x`
  and `x` agree — `require_literal_separator: false` already makes `*` do
  that. Pinned by a test so nobody "fixes" the help text's advice later.

### M2 — thread the scope

The named blocker. `--include` / `--exclude` on `Args`, compiled once in
`Args::into_ctx` (a bad glob is exit `1` there, naming the pattern) and carried
on `RunCtx.path_scope`.

Sites touched, all one insertion at an existing branch:

| site | change |
|---|---|
| `plan::build_plan` | takes `&PathScope`; drops non-visible `rel` **before** the plan exists, so `files_total` and `expected_paths()` both see the narrowed world |
| `purge::classify_purge_path` | gains `&PathScope` as **rule 0**: a hidden path returns `Keep` before the manifest check |
| `purge::plan_purge` | passes the scope through; also records `PurgePlan.hidden`, which drives the A1 guard-3 warn |
| `post::delete_deprecated` | skips a hidden path |
| `post::move_audio_cache` | skips the whole move when either the cache or the res dir is out of scope, and skips an individual hidden source file |
| `post::write_config_ini` | **skipped entirely** when a scope is active; `warn` + `SUMMARY config_bumped=0` |
| `metadata::fetch` | A1 guard 2: explicit `--audio` under a scope whose scan file is hidden → exit `1` |

`RunCtx.readonly()` still covers `--dry-run` / `--check-only`; the scope gates
are independent of it, so a scoped dry-run is read-only *and* narrow.

`Summary` gained `flags: SummaryFlags`, carrying the **conditional** keys
(`scoped`, `config_bumped`, `plan_files`, and the not-yet-used `limited`,
`plan_download_bytes`, `write_bytes`, `stopped`). Rendering them in a fixed
order, with an `Extra` enum so the `key=value` and `--json-summary` planes stay
in agreement without a second list of quoting rules, means adding a key is one
match arm. **An unflagged run's `SUMMARY` is byte-identical to today's** —
pinned by `an_unscoped_purge_still_deletes_orphans`.

Deviation from the milestone's acceptance list: the "`--include` matching
nothing → 1" case is in `tests/repair_offline.rs`, not `tests/cli_contract.rs`.
It can only be observed after the API is fetched, and the binary contract test
has no server by design.

The A1 guard-3 warn fires once per `plan_purge` call, so a run with both
`--purge-before` and `--purge-after` says it twice — one line per purge
decision, which is what actually happened.

### M3 — `--max-files` and `--max-delete-bytes`

- `scope/limits.rs`: `Limits { files, download_bytes, write_bytes, delete_bytes }`
  (all `Option`), `Limits::check_preflight(plan, purge) -> Vec<LimitHit>`, and
  `LimitHit::describe()`. All three tripped ceilings are reported at once, not
  just the first — a user who set both file and byte limits should not have to
  re-run to discover the second one.
- The purge plan is now built in the **plan** phase (4.6), unconditionally
  whenever a purge flag is set, and reused by `--purge-before`. That is what
  makes `--max-delete-bytes` answerable at all, and it removes the duplicated
  walk. `--purge-after` still re-plans after the repair, because it must see the
  `*_tmp` files the repair creates.
- Placement: `assert` sits between the plan and the execution phase, after the
  scope filter. Nothing is written when it fires — no `game_dir`, no `_tmp`, no
  delete, no `config.ini`.

**Decisions taken here that the plan had left open:**

- **A refusal returns `Ok((summary, 5))`, it does not raise `RunFailure`.**
  `RunFailure` means "the run died before it could report"; a refusal is a
  *reported* outcome and needs the same uniform `SUMMARY` tail as `0`/`3`/`4`.
  This closes docs/04 §7.3 Q1 (see the edit there). Consequently no
  `RunFailure::limit` exists — exit `5` has exactly one path through the code.
- **`--max-delete-bytes` counts the up-front plan.** That is the union of both
  purge passes for everything that exists when the run starts. A `*_tmp` the
  repair creates mid-run is *not* counted: it did not exist at plan time, and
  `--max-write-bytes` (M6) is the flag that bounds it.
- **`check_preflight` takes `purge: Option<&PurgePlan>` and treats `None` as
  zero.** In `--check-only` no purge runs, so no purge plan is consulted and any
  delete ceiling trivially holds; in `repair::run` the caller passes
  `purge_plan.as_ref().filter(|_| !ctx.check_only)` to say so explicitly.

Deviation: the "exit `5` as its own assertion" case lives in
`tests/repair_offline.rs`, not `tests/cli_contract.rs`, because reaching a
refusal needs the API. `cli_contract.rs` instead pins that `--max-files 0`
parses into `RunCtx.limits` (`exit_five_is_a_refusal_not_an_argument_error`)
and that a non-numeric value is still exit `1`.

Incidental finding, for M6's `--fail-fast`: the 5-attempt linear-backoff retry
in `hyp::get_node` sleeps 1+2+3+4 s per API call, so **any** run against an
unreachable server spends ~10 s per call in pure sleep before it reports
`exit 2`. Measured: a `GIRPR_HYP_BASE` pointed at a dead port takes ~25 s to
return. `docs/02` does not mention this retry loop.

### M4 — `--max-download-bytes`

Two mechanisms, because only one of them enforces.

**Pre-flight.** `RepairPlan::est_download_bytes(&local_map)` —
`Σ compressed_size` over the chunks the local reuse map cannot supply — is
computed once in `repair::run` and handed to the ceiling predicate, so the
number the limit is checked against, the number `PLAN` reports (M5) and the
number `SUMMARY plan_download_bytes` reports are the same figure by
construction. `Preflight` (a small struct, not three positional args) carries it.

**Runtime.** `scope/ledger.rs` adds `Summary::reserve_download_bytes(n)`: a
compare-and-swap that adds `n` to the live `download_bytes` **only** if the
result stays under the ceiling. `write_downloaded_chunk` reserves the manifest's
`compressed_size` *before* the fetch.

Two refinements over the milestone's sketch, both to make the stop exact rather
than approximate:

- **Reserve before fetch, via CAS.** A check-then-add under `--io-threads N`
  overshoots by up to N-1 chunks; `--max-download-bytes` is a ceiling, not a
  guideline. Reserving first means the total never exceeds the limit at all. An
  8-thread × 800-reservation race against a 1000-byte ceiling is a unit test.
- **Fold the real length back in.** The reservation is `compressed_size`; the
  counter is incremented with `blob.len()`. Capture invariant A6 says they are
  equal, but `adjust_download_bytes(actual - reserved)` makes
  `SUMMARY download_bytes` exact *by construction* rather than by assumption —
  which is what `gircap`'s `download_bytes == patchmaps/<id>.json →
  totals.download_bytes` assertion is written against. Consequence:
  `download_bytes` moved from a per-file `fetch_add` (after the loop) to a
  per-chunk one (inside it), because the hard stop has to read it mid-flight.
  The final total is unchanged.

`Repaired::Stopped` is the new outcome: **not** a failure (not counted in
`files_failed`, not retried) and **not** a repair. `repair::run` turns it into
`SUMMARY stopped=bytes` + exit `5`.

`Summary`'s `Default` is hand-written for the first time, because the ceiling
sentinel is `u64::MAX` and a derived `Default` would start every run at a
ceiling of **zero** — refusing the first byte. Caught by the ledger unit tests.

The acceptance case that proves the estimate is not trusted alone is
`a_broken_reuse_path_still_trips_the_runtime_stop`: `config.ini` at the latest
tag → local build loads → the reuse map covers both of foo's chunks → the
estimate is **0** → the run is let through with `--max-download-bytes 1`; then
foo's bytes are corrupted in *both* halves, every local slice fails its MD5, and
the first download reservation trips the ceiling. `SUMMARY plan_download_bytes=0`
and `stopped=bytes` on the same line is the whole argument in one output.

### M5 — `--print-plan`

`scope/report.rs`: `PlanTotals` (pure, from the two finished plans),
`format_plan_line`, and `PlanDocument::write_to` for `--plan-out`.

Placement is 4.7 — **after** the ceilings assert, before `create_dir_all`. So a
`--print-plan` run is read-only by construction, and a ceiling still trips it
(verified: `a_ceiling_trips_print_plan_like_any_other_mode`). The purge plan is
now built when `--print-plan` asks for it too, not only on a purge flag, which is
the M0 refactor paying off: `PLAN purge_files=… purge_bytes=…` is available
without passing a purge flag that would delete things.

`--plan-out` writes **JSON** (docs/04 §7.3.2, closed here). Two shapes per
plane rather than one vocabulary everywhere, because `SERDE` JSON is what the
artifact side of this repo already speaks.

`PlanDocument::new` takes the plan's own `est_download_bytes` and reconciles the
per-file split into it, so the document and the `PLAN` line can never disagree —
a JSON file that contradicts the line above it would be worse than no file.

`max_chunk_bytes` is reported but not enforced (that is docs/04 E3 /
`--max-chunk-bytes`, still P2): it is the number that bounds peak RAM (a whole
chunk is buffered compressed + uncompressed) and, times `io_threads`,
transient disk. Printing it costs one line and is the measurement
`gircap`'s §3.6 made.

`--print-plan --dry-run` is coherent, as the plan predicted: `--print-plan`
returns first, so `--dry-run` never gets to run its hashing pass in that
combination. The milestone said "the plan prints, then the loop's hashing
happens". It does not — `--print-plan` returns before the loop regardless of
`--dry-run`, which is the *safer* reading of "read-only by construction" and
means the combination is just `--print-plan`. Pinned by
`print_plan_with_dry_run_is_coherent`, whose name says what it pins.

### M6 — `--max-write-bytes`, `--fail-fast`, `--purge-only`

**`--max-write-bytes`.** `Summary::reserve_write_bytes` on the same CAS shape as
the download reservation, called at the two `write_all` sites in `file.rs` — the
reuse-copy and the downloaded-chunk write. The first is the important one: it is
a path `download_bytes` never sees, and the case the milestone was written for.

`SUMMARY stopped=` is `write` here, **not** `bytes` as docs/04 §4.6's table
said. Two flags now emit `stopped=`, and the whole point of the key is to say
*which* ceiling bound; a value that means two things says neither. The §4.6
table in docs/04 is corrected accordingly.

**`--fail-fast`.** Two mechanisms, because one was measurably not enough:

1. *As specified*: no new file task is started and in-flight ones stop
   retrying once a file exhausts its retries. `files_failed` stays honest and the
   run exits `3`.
2. *Added*: the flag also caps a file at **one** attempt instead of
   `MAX_ATTEMPTS`. Measured, one failing file costs `5 × (15 s inside
   `fetch_chunk_bytes` + 10 s of file backoff)` = **~125 s of pure sleeping** —
   the docs' E1 estimated 85 s. Mechanism 1 alone cannot help the *first* file,
   so "reaches its SUMMARY in seconds" is false with only it, and the test suite
   paid 174 s to prove that. The flag is an explicit "tell me fast"; a user who
   wants the full retry budget does not pass it. Counter semantics are unchanged.

`MockOpts.fail_chunks` makes the mock answer every chunk with a `500` and still
log the id, so "was this chunk even asked for" is assertable — that is how
`fail_fast_reaches_its_summary_in_seconds_against_a_500_origin` proves
`bar.dat` sorts first, fails, and `foo.dat` never opens a connection. The
suite's integration time went 174 s → 15.5 s.

**`--purge-only`.** One early return beside `--check-only`, calling
`post::run` and skipping the repair loop. It does **not** bump `config.ini`, for
the same reason a scoped run does not: a cleanup puts no bytes in place, so
bumping would be the launcher desync the ordering exists to prevent — and on an
already-current install it is a no-op anyway. `config_bumped=0` is emitted, which
meant widening that key from "any path scope" to "the scope **or** `--purge-only`
deliberately did not bump" (`SummaryFlags.config_bumped_zero`).

There is deliberately **no** "without `--fail-fast`" control test: proving that
case means paying the 125 s, and it is already covered structurally by
`repairs_missing_and_corrupt_files_end_to_end`, which shows every planned file is
attempted when nothing is flagged.

### M7 — `--hyp-base` / `--sophon-base`, and closing the loop

`Args::into_ctx` stops hardcoding `None` and passes the two flags through. The
seam already existed end to end — `RunCtx::api_bases` resolves explicit > env >
production, `HypClient::new_with_bases` takes them — so this is two `Option<String>`
fields and a precedence line.

**The acceptance case that actually matters** is the one this milestone could
not previously have: `tests/repair_offline.rs` now runs the **real binary**
(`tokio::process::Command`, not the blocking one — inside `#[tokio::test]` it
would occupy the single runtime thread the mock needs to answer on) against the
mock through `--hyp-base` / `--sophon-base`, with a scope and a purge flag. Every
other test in the file injects `RunCtx::hyp_base_override` directly, which proves
nothing about whether a *flag* reaches `HypClient`.

Two further cases: the env vars pointed at a dead port while the flags point at
the mock (proves explicit beats env — env winning would be exit `2`), and the
env vars alone (proves M7 did not break the seam every offline e2e already used).

`tools/capture/docs/00-spec.md` §8.1 was marked resolved with a status line
and a short notification — **nothing in §8.1.1**. An earlier draft of this
milestone did rewrite that procedure (struck step 4 through, added two steps),
and it was reverted: §8.1.1 is `gircap`'s runbook to revise, and a branch that
rewrites a consumer's procedure to suit itself is not notifying it. The
"Defects found and fixed" section below records what changed and why, and
`gircap` has since taken the runbook over on its own side.

The shim caveat is recorded in the flag help, in the README and here:
`--sophon-base` redirects `getBuild` only; manifest and chunk URLs come from
`url_prefix` in the response bodies. That is why the **artifact-serving shim**
is still outstanding, and it is consumer-side work in `tools/capture` by design.

That is the last milestone. See §0.2 for what the plan does *not* cover.

### Defects found and fixed

Re-reading M0–M7 against this plan turned up nine items: six in code this work
introduced, three in the documentation around it. All are fixed, in two commits
(one for the code, one for the docs). What each one was, and what it taught:

- **`--purge-only --dry-run` deleted files and moved the audio cache for real.**
  The post-phase's read-only gate lived in its *caller*, and `--purge-only` added
  a second call site that was not behind it. The general lesson: a gate every
  caller has to remember is not a gate. `post::run` now consults `ctx.dry_run` at
  each destructive step itself.
- **`SUMMARY download_bytes` drifted from base with no flag set**, which is the
  one thing this plan's standing invariant forbids. Two causes: a reservation the
  `?` on a failed fetch never gave back, and — subtler — the bytes of attempts
  that failed and were retried, because M4 moved the counter from a per-file
  aggregate to a live per-chunk ledger. The second is the interesting one: a
  counter that has to be readable mid-flight is not the same counter as one that
  has to be right at the end. The fix keeps both — live while running, *handed
  back per abandoned attempt* so a finished run reports what it always reported.
  M4's "the final total is unchanged" is now true rather than aspirational.
- **`--max-delete-bytes` was checked only against the up-front plan**, leaving the
  post-repair re-plan unbounded. It is now re-checked at that pass's own plan.
  Worth recording *why* that branch has no integration test: the re-plan can only
  shrink today, because a successful repair promotes every `*_tmp` it creates and
  a run that leaves one behind has already exited `3` or `5`, both of which skip
  the post-phase. So the check is defence-in-depth against a *pipeline* property
  rather than a hole, and the predicate is pinned as a unit test. Writing that
  down is more useful than a test that cannot fire.
- **`--purge-only` on its own cleaned up nothing** — the flag's own help text said
  it ran the files-cleanup. The two tests passed only because they also set
  `--purge-after`, so the case nobody exercised was the case the flag is named
  after.
- **The deprecated-file list was fetched in every read-only mode and discarded**,
  because M0 hoisted it out of `post::run` into a call site no gate covered.
  Restored to zero calls in read-only modes.
- **Documentation inconsistencies.** The README's contract table contradicted
  itself on `stopped=` and stated a `--print-plan --dry-run` behaviour this file
  had already recorded as wrong; the `gircap` spec edit was +51/−15 against an
  agreed "one line" boundary; and two claims about the repair-spec trilogy were
  scoped to the branch when they should have been scoped to the milestones. The
  `gircap` edit is now a status line and a notification — the §8.1 body and §8.1.1
  are byte-identical to what that crate shipped.

**Three of the code defects are the same mistake**: each is a new call site or
flag that was added *around* an existing gate rather than *through* it. Three
times is a pattern, and it is the thing to watch for in the next feature that
touches this pipeline — add a path, and ask what it bypasses.

### Follow-up: the `--include`-matched-nothing guard, and reuse visibility

A second pass, after `gircap` took over its own runbook, found two more things.

**The typo guard was refusing the one request `--purge-only` exists for.** M2's
guard tested `plan.files.is_empty()` and ran before the purge plan existed, so
`--purge-only --include <path>` for a file that is on disk but absent from the
new manifest was refused as a typo — the ordinary "removed upstream" case, and
`gircap`'s `removed` capture. The guard now asks whether the run selected
**nothing at all** (no repair work *and* no cleanup work), sits after the purge
plan is built, and names which plan came back empty. An empty repair plan
alongside non-empty purge candidates is a request, not a typo.

The lesson is specific and worth keeping: **a guard has to be evaluated against
every plan the run will act on.** This one was written when there was only a
repair plan, and a later milestone gave the run a second kind of work without
revisiting the guard. Same shape as the three above, one level up — the gate was
correct when written and wrong once its subject grew.

**A degraded reuse map was invisible.** When `getBuild(local)` cannot be served
the reuse map is empty, so every chunk downloads and both `download_bytes` and
`est_download_bytes` rise — while the run still exits `0`. Nothing on the data
plane distinguished "reused 8 chunks, fetched 2" from "fetched all 10", and
`--max-download-bytes` was worst affected because its pre-flight estimate is
computed against the same empty map: the ceiling silently moves and the refusal
a caller expected never fires. `SUMMARY local_manifest=ok|unavailable` now says
so, emitted only when a local version was known and the lookup was actually
attempted, so a fresh install is unaffected. The fallback itself is unchanged
and correct — this makes the degradation assertable, not different.

While there: the note pointed out that §8.1.1's assertion 3 (`must_not_fetch`)
**cannot be made from the data plane at all** — `SUMMARY` is byte-level by
design, and per-chunk detail exists only as `trace` lines in the log file. That
is fine, but it has to be stated where a consumer will read it, so `docs/04` §1.3
and the README now say that assertion 3 needs a *request log*, and that
`local_manifest` is a summary of the situation rather than a substitute for one.

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

**Scope the claim to the milestones, not to the branch.** The branch *does*
touch the three: the pre-implementation commit `82164ab` ("docs: rename and
retitle the repair-spec trilogy") renames two of the paths and retitles all
three H1s. That is H1/H2 titles and a cross-reference paragraph — no repair
rule, no step of the repair algorithm, and no input/output expectation anywhere;
`docs/02`'s diff against the branch base is its H1 alone. The §7 checklist item
that said "unmodified — `git diff --stat` says so" was wrong in exactly this
way, and is corrected below.

`gircap`'s docs are edited by neither plan, except for the status line in M7
that marks the blocker resolved. A status line plus a short notification block:
the *procedure* in that crate's §8 / §8.1.1 is its own to revise, and M7 does
not rewrite it, strike through its caveats, or add steps to its runbook.

**Settled, not left implicit:** §8.1.1's step 4 ("do not pass `--purge-after`
/ `--purge-before`, because without path filters they delete every file the live
manifest does not contain") became false the moment M2 landed — a hidden path is
invisible to the cleanup too, so a scoped cleanup cannot reach the rest of a
fixture. It was **not** corrected on this side. `gircap` took it over in its own
M12 follow-up, which rewrote §8.1 / §8.1.1 wholesale, turned the stale reason
into a deliberate "do not pass them anyway — there is nothing to delete, and a
run that deletes something is a run whose evidence you now have to reason
about", and added the four assertions the scoped run needs. So the fate is
decided on both sides: this side notified, that side revised. A notification was
the right division of labour because the reasoning behind step 4 — what a run
that deletes something does to the *evidence* — is `gircap`'s to weigh, not
`girpr`'s.

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
state. Two files and three chunks turned out to be enough to make every ceiling
trip; the one addition to `MockOpts` is `fail_chunks`, for `--fail-fast`.

**Binary** (`tests/cli_contract.rs`): the new flag surface and the exit contract.
The existing "every argument error is `1`" rule gains one carve-out — `5` is not
an argument error — so it becomes an explicit assertion rather than a blanket.

`tests/repair_offline.rs` also runs the **real binary** three times, through
`tokio::process::Command`, for M7. `cli_contract.rs` stays pure-argument
handling; this is the level at which "the flag reaches `HypClient`" is actually
observable, and a blocking `Command` there deadlocks the single-threaded test
runtime against its own mock.

Nothing new touches the network. The `GIRPR_HYP_BASE` / `GIRPR_SOPHON_BASE` env
seam is what every offline test already uses; `--hyp-base` only makes it
reachable from a flag.

---

## 4.1 How this section's predictions held up

Kept because §3 was written before any of it was built, and the gaps are the
useful part.

| §3 said | what actually happened |
|---|---|
| M3 "the refusal happens before any of it runs" | held, and M0's `create_dir_all` move is what made it true rather than nearly true |
| M4 "the estimate is documented as a lower bound, not a promise" | held, but the stop needed a CAS *reservation*, not a check: a check overshoots by `io_threads − 1` chunks |
| M5 "`--print-plan --dry-run` — the plan prints, then the loop's hashing still happens" | **wrong**: `--print-plan` returns before the loop regardless, so the combination is just `--print-plan` |
| M6 "`--fail-fast` … the run reaches its `SUMMARY` in seconds" | **only after adding something the plan did not ask for** — the flag also has to cap a file at one attempt, or the *first* file still costs ~125 s |
| M6 "three independent flags, independent commits" | shipped as one commit; the three share `Summary`'s new fields and a `SummaryFlags` key |
| §4 test strategy: "`MockOpts` gains nothing" | gained `fail_chunks` — a 500 origin is the only way to exercise the retry path without a dead host's connection timeouts |
| §5 risks: "M0 is the largest milestone" | correct; M2 and M4 were the ones that grew beyond their sketches |
| §3 M5 "`--print-plan --dry-run` … the loop's hashing still happens" | **wrong** (M5, recorded in §4.1) — and so was the README's copy of it, now corrected |
| §1: "`gircap`'s docs are edited … the one line in M7" | **wrong** — M7 shipped +51/−15 there, rewriting a consumer's runbook. Reduced to a status line plus a notification; its §8.1/§8.1.1 are untouched |
| §1: "**No milestone** in §3 edits any of the three [repair-spec docs]" | true, but the §7 checklist restated it as "unmodified — `git diff --stat` says so", which is false against the branch base because `82164ab` renames and retitles them |
| §3 M0: "No new flag, no changed default, no changed `SUMMARY`" | held for M0, but M0's *fetch* hoist put a network call into read-only modes, which is a changed default for `--dry-run`/`--check-only`. Found and reverted |

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

M0 through M7 — the set that unblocks `gircap/docs/00-spec.md` §8.1 — plus:

- [x] every `SUMMARY` key verified **absent** when its flag is absent
      (`the_summary_growth_keys_appear_only_when_their_flag_is_active`, plus the
      `an_unscoped_purge_still_deletes_orphans` control)
- [x] README flag table, machine-contract table and exit-code table updated
- [x] [`04` §7.2](04-scope-limitation-options.md) §7.2 decisions confirmed or
      corrected, and §7.3 closed (Q1 in M3, Q2 in M5; the `stopped=` vocabulary
      corrected in M6)
- [x] the three repair-spec docs **not edited by any milestone** — the branch
      renames/retitles them in `82164ab`, before M0, which is titles and a
      cross-reference only (§1 scopes this precisely)
- [x] `cargo test` green and fully offline; `cargo clippy --all-targets` clean

### §7.2 decisions, as they landed

| # | decision | outcome |
|---|---|---|
| 1 | `--include` matching nothing → exit `1`, not `5` | **as decided** (M2) |
| 2 | `config.ini` not bumped under a scope | **as decided**, and widened in M6 to `--purge-only` for the same reason |
| 3 | ceilings apply in `--check-only` and `--dry-run` | **as decided** (M3; `check_only_trips_a_ceiling_with_exit_five_not_four`) |
| 4 | B2 keeps a runtime hard stop alongside its refusal | **as decided**, and the CAS reservation was needed to make "ceiling" mean what it says (M4) |

Two things this plan did not anticipate, both recorded above: `--fail-fast` had
to gain a per-file attempt cap to deliver its own stated value (M6), and
`--print-plan` short-circuits `--dry-run` rather than following it (M5).