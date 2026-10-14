# Scope-of-work limitations for `girpr`

**Status:** proposal. Nothing here is implemented. This document states the
problem, evaluates the options, and recommends a first milestone — decisions and
deletions are marked as such inline.

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

**Two guards that are not optional.** A filter that matches nothing currently
produces a cheerful `exit 0` having done nothing, which is the single most likely
way this feature is misused:

1. `--include` present and matching **zero** planned paths → exit `1` with the
   offending globs named. (`--exclude` matching zero is fine and unremarkable.)
2. Any active scope plus `--purge-*` plus a purge that would touch a path the
   scope hides → `warn` at plan time, listing the count. Silent agreement here is
   correct, but silence should be visible once.

**Cost.** `glob` is already in `Cargo.lock` via `girsync`, so the dependency edge
resolves with no network. ~40 lines for the filter, plus the purge/`post` gating
in §2.

**Use.** The whole `gircap` matrix: `gircap/docs/00-spec.md` §8.1.1 step 2's
intended invocation is one run per capture with an `--include` set of exactly
one path.

---

### Group B — resource ceilings

The ceiling is only meaningful once the semantics of *tripping* it are pinned
down. There are three, and they are not interchangeable:

| semantic | when | suits |
|---|---|---|
| **plan-time admission** | after the plan exists, before any I/O | work you want the run to *complete* over a known subset — deterministic, and the counters stay meaningful |
| **runtime hard stop** | mid-flight, when the counter crosses the line | work you cannot predict up front, where exceeding the line must not happen even once |
| **pre-flight refusal** | before any mutation | destructive work, where a partial execution is worse than none |

Recommendation: **admission for download/files, hard stop for writes, refusal for
deletes.**

#### B1 · `--max-files <N>` — **P0** — plan-time admission

Admit the first `N` planned files and withhold the rest; the run then completes
normally over exactly that set. Admission order is the existing stable sort
(`plan::build_plan`: path ascending) — already deterministic, so a test can name
the set.

Add `--select-order path|cost` only if B2 lands too; see §4.2.

#### B2 · `--max-download-bytes <N>` — **P0** — plan-time admission, **plus** a runtime hard stop

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

**But the estimate is a lower bound, not a promise.** `try_reuse_slice` falls
back to a download whenever the local slice MD5 does not verify
(`file.rs::try_reuse_slice`), so a file admitted at 0 estimated bytes can still
download its whole body. Hence **two mechanisms**:

- **admission** — sort by `est_download_bytes` ascending, take until the ceiling,
  withhold the rest. Smallest-first across the whole set, which is the rule
  `gircap` already proved in `tools/capture/src/select/budget.rs` and the reason it
  is its own module there: admitting per-class independently lets a cheap class
  drain the budget and strand an expensive one. Sorted ascending, admission takes
  exactly a **prefix**, so the result is a pure function of `(plan, local_map, N)`
  and is reproducible across runs.
- **hard stop** — a live atomic counter in `Summary`; when a chunk fetch would
  push the run past the ceiling, stop fetching, mark remaining files withheld,
  leave their `*_tmp` files in place. `_tmp` is already resume-safe (S3,
  resume-by-length), so an aborted run is not a corrupt run — it is a run that
  did less.

**Cost.** ~120 lines. The `Budget` shape is liftable in spirit from
`gircap/src/select/budget.rs` (saturating arithmetic, `admits`-before-`charge`,
`describe()` for the log) — **lift the shape, not the code**: `gircap` shares no
code with `girpr` by design (`gircap/docs/00-spec.md` §2) and a shared crate
would violate that rule.

#### B3 · `--max-write-bytes <N>` — **P1** — runtime hard stop

Distinct from B2 and the reason the user listed write separately: **`download_bytes`
does not count most of the disk the run touches.** Every reuse-copy
(`out.write_all(&buf[..n])` in `try_reuse_slice`) and every downloaded-chunk write
goes into `*_tmp` and **none of it appears in `SUMMARY.download_bytes`**. A
zero-reuse run writes its entire payload to disk while reporting a network cost
only.

The number that matters is peak disk, which `README` states as
`io_threads × largest file`. This option bounds the *cumulative* counterpart,
which is the one a 130 GB runaway actually blows.

**Cost.** One atomic on `Summary`, incremented at the two `write_all` sites, one
check before each chunk. ~40 lines.

#### B4 · `--max-delete-bytes <N>` — **P0** — pre-flight refusal

Deletion is the one irreversible act in the tool, and a partial purge is worse
than no purge: it leaves a tree that is neither the old install nor the new one.
So this one **refuses up front**, it does not stop midway.

`purge_extra` already enumerates every candidate into a `Vec<(PathBuf, u64)>`
before touching anything (`purge.rs`), so the total is free to compute. If
`Σ candidate sizes > N`, delete nothing and fail. `post::delete_deprecated`
is a handful of files and needs no ceiling.

This is the option that answers *"use in case the exclusion/inclusion globs could
miss something"* for the one case where missing something is unrecoverable: a
glob that should have hidden `ScreenShot/` and did not, against a real 100 GB
install.

**Interaction with `--purge-before`.** `--purge-before` exists to free room for
the repair. Refusing it because the ceiling is low defeats its purpose — so
`--max-delete-bytes` should be evaluated **once over the union of both purge
passes**, not per pass, or the pre-flight number is wrong in both directions.

---

### Group C — observability

#### C1 · `--print-plan` + `--plan-out <FILE>` — **P0**

**Verdict: yes, necessary — `--dry-run` does not cover it.** Three concrete gaps,
not one:

1. **Cost.** `--dry-run` is not cheap. `repair_one_file` runs
   `util::file_len` **then `util::md5_file`** and only *then* hits
   `if ctx.dry_run { return Repaired::DryRun }`. So `--dry-run` on a real install
   **full-file MD5s every planned file** — on the 130 GB case, exactly the cost
   the limitation is meant to avoid. `--print-plan` needs only the manifests
   already fetched, plus O(plan) arithmetic.
2. **Plane.** `--dry-run` answers on **stderr / the log file**, via `tracing`.
   Asserting on it means scraping a log with `--log-level debug` turned up. The
   stdout data plane (`REPORT` / `PROGRESS` / `SUMMARY`) exists precisely so a
   caller can parse it; the plan belongs there.
3. **Totals.** `--dry-run` emits no aggregate. There is no way to ask "how many
   files would this touch, and how many bytes would it download?" without reading
   per-file `tracing` lines and summing them by hand.

Proposed line, same shape as the rest of the contract:

```
PLAN files_total=2662 admitted=8 withheld=2654 est_download_bytes=2373385 \
     est_purge_bytes=0 est_max_chunk_bytes=3689476 limited=files
```

`--plan-out <FILE>` writes the per-file list as JSON
(`{rel, size, chunks_total, est_download_bytes, decision}`) for the
`--per-class`-sized cases where eight lines of diffing beats eight `--include`
flags.

**Interaction.** `--print-plan` + `--dry-run` is coherent (`--dry-run` then costs
nothing extra, since the plan is printed before the loop and the loop still
hashes). `--print-plan` alone must still be **read-only**, and must not imply
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
the tool, and under A1 it is paid only for the admitted files, which is already a
big improvement. This flag skips the check and forces every admitted file down
the download path, so a test can exercise reuse/downloading **without** paying for
the hash. Deliberately does not exist for production: it forfeits the skip, which
is the main reason the tool is low-disk.

#### E5 · `--manifest-field <NAME>` (repeatable) — **P2**

`sophon::select_manifests` returns the `game` field plus one field per selected
audio language, each ~1 MB compressed and ~177 further file entries
(`gircap/docs/00-spec.md` §4.1). Capping to `game` cuts fetch and parse cost when
the audio manifests cannot hold the file under test. Low value — `--audio none`
plus `--audio en-us` already spans it — and it adds a third way to say "which
manifests", so it is listed last and should be cut if the surface needs trimming.

---

## 4. Interactions, and the decisions they force

### 4.1 Ordering: filter first, then admission

```
manifests ──▶ plan::build_plan ──▶ scope filter ──▶ budget admission ──▶ work
                 (blacklist)         (A1)             (B1, B2)
```

Filter before admission, so `--max-files 8 --include "*.dll"` means *the first 8
DLLs*, not *8 arbitrary files, of which some are DLLs*. The opposite order makes
`--max-files` non-deterministic under `--include`, which defeats the purpose.

### 4.2 One admission order, chosen once

| flag | admission order | use |
|---|---|---|
| `--max-files N` | path ascending (the existing sort) | "repair these N specific files" |
| `--max-download-bytes N` | `est_download_bytes` ascending | "cover as much of the code path as the budget affords" |

Two orders, one knob: `--select-order path|cost` with the above as defaults. Worth
building only if both B1 and B2 ship; if only B2 does, skip the flag.

### 4.3 What a withheld file is, and what it is not

It is **not** a failed file. `files_failed` must stay 0 for a clean scoped run —
conflating "I chose not to do this" with "this went wrong" would make every
limited run indistinguishable from a broken one in the existing `SUMMARY`.

Proposed `Summary` additions, **emitted only when the corresponding limit is
active** — the same discipline `girsync` used for `identical=N` ("an unflagged
run's output is byte-identical to a run that never passes it"), which is what
keeps the machine contract non-breaking:

| key | when |
|---|---|
| `limited=1` | any limit active |
| `limited_files=<N>` / `withheld_files=<N>` | any limit active |
| `write_bytes=<N>` | `--max-write-bytes` active |
| `config_bumped=0` | any path scope active (§2.2) |
| `est_download_bytes=<N>` | `--max-download-bytes` active |

### 4.4 Exit codes

| code | meaning | change |
|---|---|---|
| `1` | usage — bad glob, `--max-files 0`, `--include` matched nothing | extended |
| `2` | metadata / network | — |
| `3` | write / verify | — |
| `4` | `--check-only` found damage | — |
| **`5`** | **stopped by a scope limit — work was withheld** | **new** |

**Recommendation: exit `5` whenever a limit actually bound** (`withheld > 0`),
even when the work that *was* admitted succeeded. `exit 0` must mean "this run did
everything it was scoped to do"; if it also means "and the scope covered
everything", a limited run and a full run are indistinguishable, and the first
thing a test script will do is assert `0`. A script that wants the
weaker guarantee tests `[ $? -ne 5 ]`.

`--check-only` keeps its `0`/`4` semantics: a limit narrows *what is verified*,
and `4` must not be masked by narrowing.

This is additive. The README's machine-contract table is updated, `tests/cli_contract.rs`
is unaffected (new flags parse; their runtime validation is exit `1` like every
other usage error).

### 4.5 Read-only modes

`RunCtx::readonly()` already unifies `--dry-run` and `--check-only` and every
delete/write is gated on it. Path scope and budgets **compose** with both:

| | `--dry-run` | `--check-only` |
|---|---|---|
| `--include` | predicts exactly the narrowed work | verifies exactly the narrowed set |
| `--max-files N` | previews the admitted subset | verifies the admitted subset, exit `0`/`4` |
| `--print-plan` | redundant output, but harmless and cheap | the cheap way to see what will be verified |

Nothing here needs a new readonly concept. That is worth noting explicitly: the
existing `readonly` gate is the reason the whole tier is additive rather than a
new mode.

---

## 5. Suggested module layout

Four concerns — the filter, the budget, the admission walk and the plan report —
so this is a directory, not a file. The layout mirrors `tools/sync/src/rw/`
(`mod.rs` wiring, `limits.rs` pure config, `gate.rs` counting), which is the
pattern this repo already uses for exactly this split:

```
src/repair/scope/
  mod.rs      re-exports; PathScope + Budget + admit_plan + the run ledger
  filter.rs   PathScope, is_visible, compile_patterns   (pure)
  budget.rs   Budget, admit_in_order                   (pure, gircap-shaped)
  admit.rs    plan -> (admitted, withheld)             (pure)
  report.rs   PLAN line + --plan-out JSON
```

The three pure modules unit-test without a filesystem, exactly like
`purge::classify_purge_path` and `plan::build_plan` do today. The impure part is
the run ledger (atomics on `Summary`) and the gate call in `file.rs` /
`purge.rs`.

Touch points outside the module, all of them one-line insertions at an existing
branch:

| file | change |
|---|---|
| `src/cli.rs` | flags + `into_ctx` |
| `src/config.rs` | `PathScope` + `Budget` fields on `RunCtx`; `RunFailure::limit` |
| `src/repair/mod.rs` | filter + admit after `build_plan`; `PLAN` emit; gate `config.ini` |
| `src/repair/plan.rs` | — (build the filtered plan, keep `build_plan` pure) |
| `src/repair/purge.rs` | scope arg on `classify_purge_path` + pre-flight refusal |
| `src/repair/post.rs` | scope gate on deprecated deletes and audio move |
| `src/repair/file.rs` | fail-fast + write-bytes gate |
| `src/report.rs` | conditional `Summary` keys + `format_plan_line` |
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

## 7. Open questions

1. **`--max-files` with no `--include` on a real install** — is admitting the
   first 8 paths alphabetically a useful thing to be able to do, or is it only
   meaningful in combination with `--include`? Affects whether B1 ships alone.
2. **`--include` and the audio scan file.** `metadata` writes it on explicit
   `--audio`. A scoped run that also passes `--audio` — does it write, or refuse
   the combination as contradictory? (Recommendation: refuse, exit `1`; the two
   flags are asking for opposite scopes.)
3. **Should `--print-plan` imply fetching manifests at all?** It must — the plan
   *is* the manifest. But that means it is not an offline operation, and the help
   text should say so.
4. **Is `5` acceptable as an exit code?** It is the one change here that touches
   the published contract. Additive, but a caller written against `0`–`4` will
   need updating.

---

## 8. Recommended first milestone

Scoped to what unblocks `gircap` §8.1.1, nothing more.

| # | option | why in scope |
|---|---|---|
| 1 | **A1** `--include` / `--exclude` + purge/post gating + "matched nothing" guard | the named blocker |
| 2 | **B1** `--max-files N` | cheapest hard backstop; makes the run deterministic |
| 3 | **B4** `--max-delete-bytes N` (pre-flight refusal) | the irreversible case |
| 4 | **C1** `--print-plan` | makes A1/B1 auditable before committing |
| 5 | **B2** `--max-download-bytes N` (admission + hard stop) | the assertion §8.1.1 step 3 is actually written against |

`D1`, `B3`, `E1`, `E2` are the natural second milestone: D1 because the
artifact-serving shim needs it, E1 because every mock-server test needs it, E2
because the `removed` class needs it, B3 because it closes the last
un-counted write path.

Follow-on work that this unblocks but does not include: the **artifact-serving
shim** (serve a `gircap` bundle on `127.0.0.1` with `url_prefix` bodies rewritten
to itself), which is what finally turns `gircap/docs/00-spec.md` §8.1.1 from a
manual procedure into an automated assertion.