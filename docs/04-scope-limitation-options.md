# Scope-of-work limitations for `girpr`

**Status:** proposal. Nothing here is implemented. This document states the
problem, evaluates the options, and recommends a first milestone — decisions and
deletions are marked as such inline.

**Revision 2 (ceilings are enforcement bounds, not selectors).** The ceilings
were first proposed as *admission* — admit what fits, withhold the rest. They are
now **refuse / hard-stop**: a run whose plan exceeds a manually requested ceiling
**fails before doing the work**, exit `5`. The "ceilings are assertions, not
selectors" reframing and the three reasons are in §1.5; the design follow-through
is Group B; the open questions from revision 1 are closed in §7.

**Prerequisite.** Everything here needs `girpr` to build its whole plan — repair
work list *and* purge candidate list — before it mutates anything. It does not
today. That refactor is milestone **M0** of
`docs/05-scope-limitation-implementation-plan.md` and it is a prerequisite for
Groups B and C, not a nice-to-have.

**Scope:** `girpr` (the main crate) only. `gircap` is named here because it is
the consumer that motivated the work, not because anything in this document
changes it. `tools/sync` is named because two of the options are specified to
match its behaviour.

---

## 1. The context

### 1.1 What is blocked, and by what

`gircap` has reached its final milestone. It writes a self-contained artifact
bundle per version pair — a real `game/` fixture holding **one real file per
test-matrix variant**, every API response and manifest behind them, every chunk
blob, and `expected/` ground truth (`tools/capture/docs/00-spec.md`).

The bundle is built to be handed to `girpr`. Three of the four consumption
paths work; one does not:

| what | status |
|---|---|
| inspect / assert from metadata alone | ✅ |
| rebuild old/new files offline (`gircap materialize`) | ✅ |
| prove the bundle's own integrity (`gircap verify`) | ✅ |
| **run `girpr --game-path <artifact>/game` and assert it patches to the new version** | ❌ **blocked** |

The blocker is recorded verbatim in `gircap/docs/00-spec.md` §8.1 and re-confirmed
at M11 against the shipped binary: `girpr --help` offers `--game-path`, `--biz`,
`--audio`, `--io-threads`, `--purge-after`, `--purge-before`, `--check-only`,
`--dry-run`, `--json-summary`, `--log-level`. **Nothing narrows a path.**

### 1.2 Why the missing flag is expensive, precisely

`game/` holds one real file per test-matrix variant — 8 or 9 per bundle.
`girpr` run against it as-is does exactly what it is specified to do:

- `plan::build_plan` builds the work list from the **whole live `game` manifest**
  — 2 662 common paths measured for 7.0.0→7.1.0, several thousand more across
  the audio fields;
- `file::repair_all` spawns one task per planned file and downloads whatever is
  not intact — on the order of **130 GB**;
- `--purge-after` / `--purge-before` walk `game/` and delete **everything the
  live manifest does not contain**, which for a fixture that small is most of it,
  including `config.ini`'s neighbours and the retained chunk blobs if the bundle
  is pointed at in place.

None of that is a bug. It is the correct behaviour of a whole-install patcher
given a whole-install input, and `gircap` must not shim around it — the fix
belongs in `girpr` (`gircap/docs/00-spec.md` §8.1).

### 1.3 What the end-to-end check has to produce

The procedure is already written down (`gircap/docs/00-spec.md` §8.1.1) and it
has three assertions per capture, not one:

1. `SUMMARY download_bytes` equals `patchmaps/<id>.json → totals.download_bytes`;
2. the patched file's MD5 equals `expectations.result_md5`;
3. **no chunk id in `expectations.must_not_fetch` was requested** — the assertion
   that is invisible in a byte count and is the one that actually catches a
   broken S2 reuse path.

Plus the structural constraint from §8.1.1 step 4: **do not pass a purge flag**,
because without path filters the purge eats the fixture. A `--include` that also
narrows the purge keep-set removes that caveat, which is worth having on its own
merits.

### 1.4 The shape of the answer

Two distinct jobs are being conflated under "limitation":

| | job | failure if wrong |
|---|---|---|
| **path scope** | *which paths exist* | repairs 130 GB instead of 2.3 MB |
| **resource scope** | *how much work* | a filter typo, or a glob that misses one path, still lets a 100 GB run or a 100 GB delete through |

The first is a selector. The second is a **backstop**: it exists precisely
*because* selectors are fallible. A design that only ships globs has no answer
to "what if the glob is wrong", which is the user's stated reason for wanting
byte and file ceilings.

This document treats them as two tiers because they answer different questions
and have different failure modes, and because the backstop tier is worthless
without the selector tier being right most of the time.

### 1.5 Why refusal, not admission — and what it reframes

The ceilings were first proposed as *admission*: sort the plan, take what fits,
withhold the rest, exit `0`. Revision 2 makes every ceiling a **refusal**.
Three reasons, in order of how much they matter:

**1. These flags are for testing, and testing wants a failure, not a subset.**
A ceiling exists to stop a 130 GB run from happening. Admission stops the
*downloads* — but the run has already paid for the part that costs the most on a
real install: the metadata calls, the manifest fetches, and a plan of thousands of
paths. What it returns is an install that reports success and is still 99% broken.
Refusal returns in seconds, writes nothing, and says by how much it was over.

**2. A refusal is a better assertion than an admission.** This is the real
insight and it is worth stating plainly — same flags, only the ceiling's
semantics differ:

| `--include "<the 8 you meant>" --max-files 8` | admission | refusal |
|---|---|---|
| glob correct | repairs 8, exit `0` | repairs 8, exit `0` |
| glob is a typo | repairs 8 **wrong** files, exit `0` | exit `5` before any write: `plan_files=0 limit=8` |
| what the ceiling checked | "did I stay under N" | "did the scope select what you meant" |

Under refusal, the two flags stop being redundant. `--include` **selects**;
`--max-files` **verifies the selection**. That is the whole answer to "use it in
case the globs could miss something" — the backstop is not a weaker selector, it
is a *check on* the selector, and it can only be a check if it fails rather than
adjusts.

**3. Admission wants a different verb.** "Take the first N" / "take the cheapest
N" is a *selection*, not an *enforcement*. It should be `--first <N>` or
`--limit <N>`, and a name that says "limit" for a flag that quietly truncates is
its own hazard. Out of scope for this work; noted so it is not forgotten.

**What refusal costs.** A limited run no longer produces a partially-scoped
install — which is the desired outcome, not a regression: every option in this
document produces an install that is not a working install, and the sooner the
tool says so, the better.

**What refusal does not bound.** Metadata and manifest traffic. Those fetches
happen before the plan exists, and nothing here changes them (~2–3 MB per run
measured against a live pair). A ceiling bounds chunk downloads, writes and
deletes. It is not a bandwidth cap and should not be described as one.

---

## 2. The invariant: a filtered-out path does not exist

The user's spec is the right one and it matches `girsync`
(`tools/sync/src/filter.rs`, module header): *"Filtered-out paths are treated as
nonexistent on **both** sides"* — skipped in walks, dropped from records, left
untouched in the destination. For `girpr` "both sides" is **the plan and the
purge keep-set**, and the second is the one that is easy to forget.

That is stronger than "skip it in the repair loop", and the difference is where
the bugs will be. Every site in `girpr` that touches a path by name has to be
enumerated, because **narrowing the plan alone makes purge destructive**:

| site | today | under path scope |
|---|---|---|
| `repair::plan::build_plan` | blacklist filter | additionally drop non-matching `rel` — **before** the plan exists, so `files_total` already counts only what survives |
| `repair::file::repair_all` | iterates `plan.files` | already narrowed; nothing to do |
| `repair::check::verify_plan` | iterates `plan.files` | already narrowed |
| `repair::purge::purge_extra` | keep = `expected_paths()` ∪ `keep_set()` ∪ audio-version patterns; **everything else deletes** | filtered paths must be invisible **here too**, or `--purge-*` deletes the fixture |
| `repair::purge::remove_emptied_dirs` | `remove_dir` on every empty dir | a directory whose only contents were filtered away must not be swept |
| `repair::post::delete_deprecated` | deletes every present deprecated path | skip any hidden path |
| `repair::post::move_audio_cache` | moves `cache/**` → `res/**` unconditionally | gate; a scoped dev run should not relocate audio |
| `repair::post::write_config_ini` | always bumps `game_version` | **see §2.2** |
| `repair::metadata` → `audio::write_scan_file` | writes the scan file on explicit `--audio` | gate under a scope |

`purge` is the one that must not be missed. Note also that `plan.expected_paths()`
is the *only* manifest-derived input to the keep set, so a filter applied
upstream of `build_plan` and nothing else produces the worst possible outcome: a
narrower keep set than an unfiltered run.

### 2.1 Two ways to hide a path from purge — pick one

- **(a) filter inside the purge walk**: `classify_purge_path` gains a scope
  argument and a hidden path returns `Keep` *before* the manifest check.
- **(b) widen the keep set**: push every hidden path into `server_keep`.

(a) is the better fit. `classify_purge_path` is already a pure function with an
exhaustive unit test (`purge.rs`'s `mod tests`); adding a scope parameter keeps it
one, whereas (b) grows a `HashSet<String>` to tens of thousands of entries and
turns a "pure classification function" into a "pure function that needs the answer
computed somewhere else first". (b) also gets the emptied-dir sweep wrong for
free, because an emptied directory has no files left to classify.

### 2.2 `config.ini` under a scoped run — a real decision, not an implementation detail

`post::run` bumps `game_version` to the latest tag, last, "so the on-disk version
only advances once the bytes are in place" (`post.rs` header). Under a scoped
run that premise is false by construction: the bytes are deliberately **not** in
place.

A scoped run therefore leaves an install whose `config.ini` claims `7.1.0` while
the tree is still `7.0.0` in every excluded path. That is precisely the launcher
desync the ordering exists to prevent, and it is strictly worse than a normal
partial failure, which exits `3` and skips the post-phase entirely.

**Recommendation:** a run in which any path scope is active **does not bump
`config.ini`**, and says so at `warn`. `SUMMARY` gains `config_bumped=0` so a
test can assert it rather than reading the file. Without this, every scoped run
corrupts the bookkeeping of whatever it is pointed at — including, in the
`gircap` workflow, the artifact bundle's own `game/config.ini`, which must stay
byte-identical for `gircap verify` to keep passing (§8.1.1 step 1).

---

## 3. Option inventory

Grouped by what they answer. **P0** unblocks `gircap`. **P1** is what makes the
resulting test rigour rather than a demo. **P2** is worth having if cheap.

### Group A — path scope

#### A1 · `--include <GLOB>` / `--exclude <GLOB>` (repeatable) — **P0**

**How.** `glob` crate patterns matched against the `/`-separated, game-dir-rooted
rel path — the same string `plan.rs` stores and `util::normalize_rel` converts at
the filesystem boundary. Empty `--include` admits everything. `--exclude` beats
`--include`. Ported from `tools/sync/src/filter.rs` essentially verbatim,
including `MatchOptions { require_literal_separator: false,
require_literal_leading_dot: false, case_sensitive }`.

Three spec details worth copying rather than re-deriving:

- **`*` crosses `/`.** With `require_literal_separator: false`, `--include
  "*.blk"` matches `GenshinImpact_Data/StreamingAssets/.../00/09801749.blk`. A
  user expecting `*.blk` to mean "in the current directory" will be surprised;
  the flag help must say so, because every `*.blk`-shaped include in a shell is
  at risk of it.
- **Case-insensitive by default**, as in `girsync` (`--case-sensitive` default
  *false*). Game paths are Windows; the live manifests are lowercase anyway.
  `girpr` has no business growing a second spelling of that decision, so: no
  case flag in `girpr`, hardcoded insensitive, one line in the module doc.
- **A bad pattern is exit `1`**, via a `compile_patterns` shaped exactly like
  `girsync`'s (`anyhow::Context` naming the offending glob).

**Three guards that are not optional.** A filter that matches nothing currently
produces a cheerful `exit 0` having done nothing, which is the single most likely
way this feature is misused:

1. **`--include` matching zero planned paths → exit `1`**, with the offending
   globs named. (`--exclude` matching zero is fine and unremarkable.) A scope that
   selected nothing is a typo, and a typo that costs 130 GB to discover is the
   failure mode this whole document exists to remove.
2. **Explicit `--audio` under an active path scope → exit `1`, unless the audio
   scan file is itself in scope.** `metadata` writes
   `audio_pkg_scan_dir` on any explicit `--audio` (`metadata.rs`, gated only on
   `!readonly`). Under a scope that write is a mutation of a path the scope may
   not have admitted — so the combination is refused outright, and the escape
   hatch is to include the scan file's own rel path (`GenshinImpact_Data/Persistent/audio_lang_14`
   for a global install; never hardcode it, it comes from `getGameConfigs`).
   Two flags asking for opposite scopes is a contradiction, not a merge.
3. **Any active scope plus `--purge-*` plus a purge that would touch a path the
   scope hides → `warn` at plan time**, listing the count. Silent agreement here
   is correct, but silence should be visible once.

**Cost.** `glob` is already in `Cargo.lock` via `girsync`, so the dependency edge
resolves with no network. ~40 lines for the filter, plus the purge/`post` gating
in §2.

**Use.** The whole `gircap` matrix: `gircap/docs/00-spec.md` §8.1.1 step 2's
intended invocation is one run per capture with an `--include` set of exactly
one path.

---

### Group B — resource ceilings

**Revised (rev 2). Every ceiling here is an enforcement bound, not a selector.**
A run whose plan exceeds one **refuses before it does the work**, exit `5`. There
are exactly two enforcement points and they are not interchangeable:

| semantic | when | suits |
|---|---|---|
| **pre-flight refusal** | after the plan exists, before any mutation | anything predictable from the plan: file count, download bytes, delete bytes |
| **runtime hard stop** | mid-flight, when a counter crosses the line | anything not predictable from the plan: bytes written into `*_tmp` |

Both produce exit `5`. There is deliberately no third form; see §1.5 for why
admission is gone.

A refusal is checked on the **filtered** plan, so `--include` selects and a
ceiling verifies. Without a filter, a ceiling is a refusal on any real install —
which is the correct answer, not a limitation of the design.

#### B1 · `--max-files <N>` — **P0** — pre-flight refusal

Refuse when `plan.files.len() > N`: before any file task spawns, before any
directory is created, before any `_tmp` is opened. One comparison, one exit.

#### B2 · `--max-download-bytes <N>` — **P0** — pre-flight refusal, **plus** a runtime hard stop

This is the one the `gircap` assertion is written against
(`SUMMARY download_bytes == patchmaps/<id>.json → totals.download_bytes`), so its
accuracy matters.

**The estimate is exactly computable up front**, because `manifest::local_reuse_map`
has already built `local_map: path -> [(uncompressed_md5, size, offset)]` by the
time the plan exists. Per planned file:

```
est_download_bytes(rel) = Σ c.compressed_size
                           for c in plan[rel].chunks
                           where (c.uncompressed_md5, c.uncompressed_size)
                                 ∉ local_map[rel]
```

That is character-for-character the capture's `patch_fetch` definition
(`gircap/docs/00-spec.md` §5) and therefore its `totals.download_bytes`. And
because capture invariant **A6** asserts `len(raw blob) == compressed_size` while
`file.rs` counts `stats.download_bytes += blob.len()`, the estimate and the
runner's counter are the same quantity by construction — a testable prediction,
not a hope.

**Both mechanisms are needed, and the second is the one that actually enforces.**

- **Refusal (primary, deterministic, pre-flight).**
  `Σ est_download_bytes over the filtered plan > N` → exit `5`, nothing written,
  no `_tmp` created. The same predicate as B1 measured in bytes. Its value is
  that it fails *before* the expensive part.
- **Hard stop (the enforcement).** The estimate is a **lower bound, not a
  promise**: `try_reuse_slice` falls back to a download whenever the local slice
  fails its MD5, so a file estimated at 0 bytes can still download its whole
  body. A refusal on the estimate alone would therefore not bound anything. A
  live atomic on `Summary` trips at the real ceiling; the in-flight and remaining
  files are dropped, their `*_tmp` files left in place — S3 resume-by-length
  means an aborted run is not a corrupt run, it is a run that did less — and the
  run exits `5`.

**Cost.** ~120 lines. The `Budget` shape is liftable in spirit from
`gircap/src/select/budget.rs` (saturating arithmetic, `describe()` for the log) —
**lift the shape, not the code**: `gircap` shares no code with `girpr` by design
(`gircap/docs/00-spec.md` §2) and a shared crate would violate that rule.

#### B3 · `--max-write-bytes <N>` — **P1** — runtime hard stop

The only one that *cannot* be a refusal, and the reason the user listed write
separately: **`download_bytes` does not count most of the disk the run touches.**
Every reuse-copy (`out.write_all(&buf[..n])` in `try_reuse_slice`) and every
downloaded-chunk write goes into `*_tmp` and **none of it appears in
`SUMMARY.download_bytes`**. A zero-reuse run writes its entire payload to disk
while reporting a network cost only.

Bytes written are also not knowable from the plan — reuse success is decided per
chunk at runtime, against bytes that may not be there. So this one is a
mid-flight stop and nothing else.

The number the README cares about is *peak* disk (`io_threads × largest file`).
This bounds the cumulative counterpart, which is what a runaway actually blows.

**Cost.** One atomic on `Summary`, incremented at the two `write_all` sites, one
check before each chunk. ~40 lines.

#### B4 · `--max-delete-bytes <N>` — **P0** — pre-flight refusal

Deletion is the one irreversible act in the tool, and a partial purge is worse
than no purge: it leaves a tree that is neither the old install nor the new one.
Refusal is therefore the only acceptable shape, and it is cheap — but only once
the plan is up front. Today `purge_extra` walks and deletes in one pass, so there
is no point at which "how much would this delete" is answerable without already
having started. This is the clearest single justification for the M0 refactor
(`docs/05`, M0).

Once planned: if `Σ candidate sizes > N`, delete nothing, exit `5`.
`post::delete_deprecated` is a handful of files and needs no ceiling.

**Interaction with `--purge-before`.** `--purge-before` exists to free room for
the repair. Refusing it because the ceiling is low defeats its purpose — so the
ceiling is evaluated **once over the union of both purge passes**, not per pass,
or the pre-flight number is wrong in both directions.

**Interaction with path scope.** A filtered-out path is not a purge candidate, so
it never counts toward the total. That is the correct interaction and the reason
the scope and the ceiling are complementary rather than redundant: the glob
narrows what is *counted*, the ceiling bounds what is *done*.

---

### Group C — observability

#### C1 · `--print-plan` + `--plan-out <FILE>` — **P0**

**Verdict: yes, necessary — `--dry-run` does not cover it.** Three concrete gaps,
not one:

1. **Cost.** `--dry-run` is not cheap. `repair_one_file` runs
   `util::file_len` **then `util::md5_file`** and only *then* hits
   `if ctx.dry_run { return Repaired::DryRun }`. So `--dry-run` on a real install
   **full-file MD5s every planned file** — on the 130 GB case, exactly the cost
   the limitation is meant to avoid. `--print-plan` stops before the repair loop
   entirely, so it pays none of it.
2. **Plane.** `--dry-run` answers on **stderr / the log file**, via `tracing`.
   Asserting on it means scraping a log with `--log-level debug` turned up. The
   stdout data plane (`REPORT` / `PROGRESS` / `SUMMARY`) exists precisely so a
   caller can parse it; the plan belongs there.
3. **Totals.** `--dry-run` emits no aggregate. There is no way to ask "how many
   files would this touch, and how many bytes would it download?" without reading
   per-file `tracing` lines and summing them by hand.

**What it does, precisely.** Run phases 1–4 — prepare, metadata, `REPORT`,
manifests, plan — print the `PLAN` line, and stop. Nothing after the plan is
executed: no purge, no repair, no post-phase.

**It is not an offline mode and does not pretend to be.** Fetching the manifests
is not part of what `--print-plan` does differently; every run does it, exactly
as it calls the API first. The plan *is* the manifest — there is no plan without
it. So `--print-plan` fetches, and the fetches are logged at the usual level like
any other run. The help text says so, because "print the plan" invites the
assumption that it is a local, free operation.

Proposed line, same shape as the rest of the contract:

```
PLAN files=8 est_download_bytes=2373385 purge_files=0 purge_bytes=0 \
     max_chunk_bytes=3689476 scoped=1
```

`--plan-out <FILE>` writes the per-file detail as JSON —
`{"repair": [{rel, size, chunks_total, est_download_bytes}], "purge": [{rel, size}]}`
— for the `--per-class`-sized cases where diffing eight lines beats eight
`--include` flags.

**The purge half needs the M0 refactor.** To-be-purged files are enumerated by a
`WalkDir` over `game_dir` that today happens *after* the repair loop and *only*
when a purge flag is set. `--print-plan` needs that list up front and
unconditionally. M0 splits `purge_extra` into plan and apply
(`docs/05`, M0); until then `--print-plan` can report the repair plan only.

**Interaction.** `--print-plan` + `--dry-run` is coherent (the plan is printed
before the loop, so the loop's hashing still happens but is what `--dry-run`
asked for). `--print-plan` alone must still be **read-only**, and must not imply
`--check-only`: it answers "what would you do", not "what is wrong".

#### C2 · `PLAN` guard: an `--include` that matches nothing — folded into A1

Not a separate option; see A1. It is listed here because it is the *reason* C1
matters: C1 makes the mismatch visible, A1 makes it fatal.

---

### Group D — endpoint override

#### D1 · `--hyp-base <URL>` / `--sophon-base <URL>` — **P1**

**How.** The seam already exists end to end. `RunCtx` has
`hyp_base_override` / `sophon_base_override`; `RunCtx::api_bases` resolves
explicit > env > production; `HypClient::new_with_bases` takes them.
`Args::into_ctx` hardcodes both to `None` and only the `GIRPR_HYP_BASE` /
`GIRPR_SOPHON_BASE` env path is reachable. Two `Option<String>` fields and a
precedence line.

Matching `gircap`'s flag spelling (`--hyp-base` / `--sophon-base`) is worth the
small inconsistency with `girpr`'s flag style, so that one invocation works
against either tool.

**The caveat, which is the important part of this option.** `--sophon-base`
redirects **`getBuild` only**. Manifest and chunk URLs are not built from the
configured base at all — `sophon::fetch_manifest` and `sophon::fetch_chunk_bytes`
both call `sophon::join_url(prefix, id)` on a `url_prefix` read out of the
**response body** (`manifest_download.url_prefix`, `chunk_download.url_prefix`).
Pointing `--sophon-base` at a mock therefore redirects the metadata call and
leaves every manifest and chunk fetch wherever the mock's own body says.

This is not a reason to skip D1. It is the reason the `gircap` consumer needs a
thin **artifact-serving shim** — serve `api/*.json`, `manifests/*` and
`chunks/blobs/*` from the bundle with each body's `url_prefix` rewritten to the
shim's own origin. That shim is a *consumer-side* artifact of this feature, it
belongs to the follow-on work (`gircap/docs/00-spec.md` §8.1.1 step 3 is
otherwise unrunnable), and its existence should be recorded here so it is not
rediscovered later as a mystery.

---

### Group E — additional recommendations

Beyond the four the user listed. Ordered by value-per-line.

#### E1 · `--fail-fast` — **P1** — *recommended*

The single most valuable thing for a mock-server test, and it is 15 lines.

`repair_one_file` retries a file up to `MAX_ATTEMPTS = 5` with a linear
`sleep(attempt seconds)` backoff, **and** each attempt's chunk fetches retry 5×
internally in `sophon::fetch_chunk_bytes` with their own 1–5 s sleeps. That is
`5 × 15 s + 10 s ≈ 85 s` of pure sleeping per file, before any connection timeout
is counted (`HypClient::new_with_bases` sets none, so a dead host also pays the
OS default). Across a real plan that is hours of wall clock, and every failure
reports the same thing.

`--fail-fast` stops spawning new file tasks and aborts the rest once one file has
exhausted its retries. The counter semantics are unchanged, `files_failed` is
honest, and the run reaches its `SUMMARY` in seconds.

#### E2 · `--purge-only` — **P1** — *recommended*

The `removed` matrix class is *only* reachable through the purge path, and
`gircap`'s own procedure (§8.1.1) says not to pass a purge flag at all. So the
one class that exercises `purge::purge_extra` + `remove_emptied_dirs` end to end
is the one class the harness tells you to skip.

`--purge-only` runs phases 1–5 and 7 and skips 6: metadata → manifests → plan →
purge → post. It is `check_only` inverted — the existing flag answers "is it
intact", this one answers "make it match the manifest". Together they cover the
read and write halves of the cleanup contract.

Cheap (an early return beside the `--check-only` one), and it retires a stated
gap in the capture procedure rather than a hypothetical one.

#### E3 · `--max-chunk-bytes <N>` — **P2** — *recommended*

Two different jobs in one flag, both real:

- **Memory.** S1 buffers whole chunks (`sophon::fetch_chunk_bytes` → `decode_all`,
  `file.rs::write_downloaded_chunk`). Peak RAM per active chunk is roughly
  `compressed + uncompressed`, and the capture measured the largest real Genshin
  chunk at **3 689 476 B compressed** (`gircap/docs/00-spec.md` §3.6). At
  `--io-threads 8` that is a real number. Skipping files whose largest chunk
  exceeds `N` bounds it.
- **Targeting.** The same measurement makes the flag a way to *deliberately
  exercise* the largest-chunk branch, which no small synthetic fixture in
  `tests/common/mod.rs` will ever reach.

#### E4 · `--no-skip-intact` — **P2** — *recommended*

`repair_one_file`'s fast path is `size + full-file MD5`. On a 100 GB install that
is a 100 GB read before a single byte is downloaded — the largest fixed cost in
the tool, and under A1 it is paid only for the files the scope selected, which is
already a big improvement. This flag skips the check and forces every selected
file down the download path, so a test can exercise reuse/downloading **without**
paying for the hash. Deliberately does not exist for production: it forfeits the
skip, which is the main reason the tool is low-disk.

#### E5 · `--manifest-field <NAME>` (repeatable) — **P2**

`sophon::select_manifests` returns the `game` field plus one field per selected
audio language, each ~1 MB compressed and ~177 further file entries
(`gircap/docs/00-spec.md` §4.1). Capping to `game` cuts fetch and parse cost when
the audio manifests cannot hold the file under test. Low value — `--audio none`
plus `--audio en-us` already spans it — and it adds a third way to say "which
manifests", so it is listed last and should be cut if the surface needs trimming.

---

## 4. Interactions, and the decisions they force

### 4.1 Ordering: filter, then assert, then work

```
metadata ──▶ manifests ──▶ build_plan ──▶ purge plan ──┐
                              │                          │
                          scope filter (A1)          purge plan
                              ▼                          │
                        ┌─────────────┐                  │
                        │ assert      │  ◀── exit 5 on ──┘
                        │ ceilings    │      violation
                        │ (B1 B2 B4)  │
                        └──────┬──────┘
                               ▼  exit 0-or-execute
                    purge-before ──▶ repair loop ──▶ post
```

Two consequences worth stating:

- **Filter before assert.** The ceilings are checked on the *filtered* plan, so
  `--max-files` always means "the files the scope selected", never "the files the
  scope happened to keep from a bigger number". Checking before filtering would
  make every ceiling unusable — they would always refuse on a real install.
- **Assert before everything mutates.** The purge plan is built here too, so a
  `--max-delete-bytes` refusal happens before a single byte is deleted. This is
  the whole reason for the M0 refactor.

### 4.2 Admission order is no longer a question

Revision 1 had to choose an admission order (`path` ascending for
`--max-files`, `est_download_bytes` ascending for `--max-download-bytes`) and
considered a `--select-order` knob. Refusal never chooses, so the question is
gone and the knob is not proposed. `est_download_bytes` is still *computed* per
file, for B2's assertion and for the `PLAN` line.

### 4.3 A refusal has no partial state — except one

For B1, B2's pre-flight half and B4: **nothing has been written when the run
exits `5`.** No directory created, no `_tmp` opened, no file deleted, no
`config.ini` touched. A caller can retry with a bigger ceiling and get the same
run.

The single exception is **B2's runtime hard stop**, which by definition fires
mid-flight. It leaves `*_tmp` files behind. That is safe — S3 resume-by-length
means the next run picks up from the existing prefix — but it is state, and the
`SUMMARY` must say so rather than leaving it to be discovered.

There is no `files_withheld` counter and no `admitted`/`withheld` framing any
more. What a limited run reports is what it *would* have done and why it stopped.

### 4.4 Exit codes

| code | meaning | change |
|---|---|---|
| `1` | usage — bad glob, `--include` matched nothing, `--audio` under a scope | extended |
| `2` | metadata / network | — |
| `3` | write / verify | — |
| `4` | `--check-only` found damage | — |
| **`5`** | **a manually requested enforcement limit was exceeded** | **new** |

**Why `5` and not `1`.** It is worth being explicit, because the usual instinct is
that a flag check is a usage error. It is not: **the API has to be fetched
first**, and whether a limit is exceeded is only knowable from the server's
answer. `--max-files 0` against an empty plan is not an error; the identical flag
against a 3000-file plan is. A code that means "the run was misconfigured" cannot
mean both, so `5` means "the run failed", and it is deliberately paired with the
`SUMMARY` line and exit `0` on success.

This is also why `--max-files 0` is **not** rejected at parse time: `0` is a
perfectly meaningful ceiling. Only an unparseable value (negative, non-numeric) is
exit `1`, via clap.

**`exit 5` whenever a limit actually bound**, even when the work that *was* meant
to happen would have succeeded. `exit 0` means "this run did everything it was
scoped to do"; if it also meant "and the scope covered everything", a limited run
and a full run would be indistinguishable and the first thing a test script would
do is assert `0`. A script that wants the weaker guarantee tests `$? -ne 5`.

`--check-only` keeps its `0`/`4` semantics: a ceiling narrows *what is verified*,
and `4` must not be masked by narrowing — but a ceiling that trips during
`--check-only` is still `5`, because that is a different fact from "damage found".

This is additive. The README's machine-contract table is updated;
`tests/cli_contract.rs` is unaffected (new flags parse; their runtime validation
is exit `1` like every other usage error, and exit `5` is a new `assert` case).

### 4.5 Read-only modes

`RunCtx::readonly()` already unifies `--dry-run` and `--check-only`, and every
delete/write is gated on it. Ceilings apply in read-only modes too — that is
where they are most useful, since a read-only run is the natural first probe:

| | `--dry-run` | `--check-only` | `--print-plan` |
|---|---|---|---|
| `--include` | predicts exactly the narrowed work | verifies exactly the narrowed set | reports exactly the narrowed set |
| `--max-files N` | refuses before the hash storm | refuses before verifying | reports `files=` and refuses if over |
| exit on a tripped ceiling | `5` | `5` | `5` |

A read-only run that trips a ceiling has written nothing at all, so there is no
partial-state question. Nothing here needs a new readonly concept: the existing
`readonly` gate is why this whole tier is additive rather than a new mode.

### 4.6 `SUMMARY` additions

Emitted **only when the corresponding limit is active** — the same discipline
`girsync` used for `identical=N` ("an unflagged run's output is byte-identical to
a run that never passes it"), which is what keeps the machine contract
non-breaking:

| key | when |
|---|---|
| `limited=1` | any ceiling active |
| `scoped=1` | any path scope active |
| `plan_files=<N>` | any ceiling or scope active |
| `plan_download_bytes=<N>` | `--max-download-bytes` active |
| `write_bytes=<N>` | `--max-write-bytes` active |
| `config_bumped=0` | `config.ini` was deliberately **not** bumped — a path scope is active, or `--purge-only` ran (§2.2) |
| `stopped=<bytes>\|write` | a runtime hard stop fired, not a refusal. `bytes` = `--max-download-bytes`, `write` = `--max-write-bytes`; the key says *which* ceiling bound, so the two cannot share a value |

`stopped=` is what distinguishes "nothing was written because the plan was too
big" from "something was written and then we ran out" (§4.3).
---

## 5. Suggested module layout

Two directories, because this work adds a concern (`scope`) *and* introduces one
(`the up-front plan`).

**The plan restructure** is the prerequisite and it belongs next to the work it
exists for, mirroring `girsync`'s `planner/` → `apply` split:

```
src/repair/plan/
  mod.rs        RepairPlan, PlannedFile, est_download_bytes, build_plan
  purge.rs      PurgePlan, PurgeCandidate, plan_purge, apply_purge   (was purge.rs)
```

`purge_extra` currently walks and deletes in one pass; splitting it into
`plan_purge` (walk + classify → `Vec<PurgeCandidate>` + the emptied-dir set) and
`apply_purge` (delete a decided list) is what makes B4's refusal and C1's purge
half possible at all. **No flag changes, no changed defaults** — the existing
tests must pass through this refactor untouched. The one behavioural delta is
spelled out in `docs/05` M0 step 3: `create_dir_all` moves to execution time, so
a run that later fails leaves no directory behind.

**The scope layer** is new. Four concerns — the filter, the ceilings, the
assertion, the plan report — so it is a directory too. The layout mirrors
`tools/sync/src/rw/` (`mod.rs` wiring, `limits.rs` pure config, `gate.rs`
counting), the pattern this repo already uses for exactly this split:

```
src/repair/scope/
  mod.rs      re-exports; the assertion that turns a plan into pass/refuse
  filter.rs   PathScope, is_visible, compile_patterns     (pure)
  limits.rs   Limits (files / download / write / delete) (pure)
  ledger.rs   the runtime atomics B3 and B2's hard stop share
  report.rs   PLAN line + --plan-out JSON
```

The pure modules unit-test without a filesystem, exactly like
`purge::classify_purge_path` and `plan::build_plan` do today. The impure part is
the ledger and the gate calls in `file.rs`.

Touch points outside the two directories, all of them one-line insertions at an
existing branch:

| file | change |
|---|---|
| `src/cli.rs` | flags + `into_ctx` |
| `src/config.rs` | `PathScope` + `Limits` on `RunCtx`; `RunFailure::limit` |
| `src/repair/mod.rs` | build both plans → filter → assert → execute; `PLAN` emit; gate `config.ini` |
| `src/repair/post.rs` | scope gate on deprecated deletes and audio move |
| `src/repair/file.rs` | write-bytes ledger + download-bytes hard stop + fail-fast |
| `src/report.rs` | conditional `SUMMARY` keys + `format_plan_line` |
| `README.md` | flag table, machine-contract table, exit-code table |

---

## 6. What this document does not recommend

- **A `--scope-profile` file.** Readable, but the flags compose fine and a
  profile is a second surface to document, test and keep in sync. `girsync` has
  no profile and its flag set is already large.
- **Rate limiting / bandwidth caps.** `gircap` measures a 2.37 MB total for its
  six patchable captures; nothing here needs a throttle. The README already
  lists "no speed limiter" as a v1 gap, and it is still correctly a gap.
- **Sharing code with `gircap`.** `gircap/docs/00-spec.md` §2 makes independence
  non-negotiable and CI-enforced. `Budget` is a ~40-line idea that will be
  written twice.
- **Making any of this default.** Every option here produces an install that is
  not a working install. Defaults stay as they are.

---

## 7. Decisions taken, and what is still open

### 7.1 Closed in revision 2

| # | question | decision |
|---|---|---|
| 1 | `--max-files` without `--include` — useful, or only meaningful in combination? | **Void.** The premise was admission ("admit the first 8 alphabetically"). Under refusal the question does not exist: `--max-files 8` alone is a refusal, and the flag's purpose is to *verify* an `--include` set. |
| 2 | `--include` + explicit `--audio` — write, or refuse? | **Refuse, exit `1`**, unless the audio scan file's own rel path (from `getGameConfigs.audio_pkg_scan_dir`) is inside the scope. Two flags asking for opposite scopes is a contradiction. See A1 guard 2. |
| 3 | Should `--print-plan` avoid fetching manifests? | **No, and it does not try.** Manifest fetch is not part of what `--print-plan` does differently — every run does it, like the API call before it. It fetches, logs at the usual level, and the help text says so. |
| 4 | Is exit `5` acceptable? | **Yes.** `5` = a manually requested enforcement limit was exceeded. Not `1`, because the API must be fetched first and whether a limit trips depends on the server's answer — `--max-files 0` is not an error on an empty plan and is on a 3000-file plan. A "misconfigured" code cannot mean both. |

### 7.2 Decided here, flag them if wrong

These were not asked; they are the calls that had to be made to write the plan.
Each is cheap to reverse before anything is built.

1. **`--include` matching nothing → exit `1`, not `5`.** Split from Q4 on purpose:
   `1` is "your request cannot be satisfied for a reason that is a property of
   your request" (a glob that matches nothing is a typo); `5` is "your request was
   well-formed and the answer was too big". Both are detected after the fetch, but
   the diagnosis a caller acts on is different.
2. **`config.ini` is not bumped under an active path scope** (§2.2). Unanswered so
   far because it was not asked; it changes what a scoped run leaves on disk and
   a test will notice.
3. **Ceilings apply in `--check-only` and `--dry-run`** (§4.5). They are read-only
   runs, so a ceiling there costs nothing and refuses for free.
4. **B2 keeps a runtime hard stop alongside its refusal** (§3 B2). The pre-flight
   estimate is a lower bound because reuse can fall back to a download, so
   refusing on it alone would not actually bound anything.

### 7.3 Closed

**1. A refusal does emit `SUMMARY`** (decided in M3). `repair::run` *returns*
`(summary, 5)` rather than raising `RunFailure`, so a refusal lands on exactly
the same tail as `0`, `3` and `4` — one uniform ending for every exit path, and
the `limited=1` / `plan_files=…` / `purge_bytes` keys a parser needs to see why.
The diagnosis itself goes to stderr and the log file (`ERROR limit exceeded:
--max-files exceeded: plan_files=3000 limit=8 …`), not to stdout: `SUMMARY`
carries counters and flags, not prose. The one thing a refusal has and a normal
run does not is that every value in it is `0` except the plan-derived ones,
which is itself the proof that nothing was written.

**2. `--plan-out` is JSON.** `girsync --output json` and the whole artifact
side of this repo (`gircap`'s bundle, `patchmaps/<id>.json`) are JSON, and a
parser that has to read `SUMMARY` *and* the plan wants one `serde_json` reader,
not two vocabularies. The `key=value` plane stays single-purpose: one line per
fact, greppable, no nesting.

---

## 8. Milestones

Build order, acceptance checks and the commit per milestone live in
`docs/05-scope-limitation-implementation-plan.md`. Summary only:

| milestone | option | why in scope |
|---|---|---|
| **M0** | the plan restructure — repair *and* purge plans built before mutating | no new flags; the prerequisite B4 and C1 are both blocked on |
| **M1** | **A1** the filter, unwired | lands the semantics and their tests before anything depends on them |
| **M2** | **A1** threaded through the pipeline + the two guards | the named blocker |
| **M3** | **B1** `--max-files`, **B4** `--max-delete-bytes` | the assertion that the scope selected what you meant; B4 is answerable thanks to M0 |
| **M4** | **B2** `--max-download-bytes` | the assertion §8.1.1 step 3 is written against |
| **M5** | **C1** `--print-plan` | makes the scope and the ceilings auditable before committing |
| **M6** | **B3** `--max-write-bytes`, **E1** `--fail-fast`, **E2** `--purge-only` | E1 because every mock-server test needs it, E2 because the `removed` class needs it, B3 because it closes the last un-counted write path |
| **M7** | **D1** `--hyp-base` / `--sophon-base` | the artifact-serving shim needs it; this is also the milestone that closes the `gircap` blocker |

Follow-on work that this unblocks but does not include: the **artifact-serving
shim** (serve a `gircap` bundle on `127.0.0.1` with `url_prefix` bodies rewritten
to itself), which is what finally turns `gircap/docs/00-spec.md` §8.1.1 from a
manual procedure into an automated assertion.