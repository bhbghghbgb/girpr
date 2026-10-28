# Scope-of-work limitations for `girpr`

**Status: shipped.**

**Scope:** the `girpr` crate. `gircap` is named because it is the consumer that
motivated the work, not because anything here changes it. `tools/sync`
(`girsync`) is named because two of the rules below are specified to match its
behaviour.

---

## 1. The problem

`gircap` writes a self-contained artifact bundle per version pair: a real
`game/` fixture holding **one real file per test-matrix variant**, the API
responses and manifests behind them, every chunk blob, `expected/` ground truth,
and a `patchmaps/<id>.json` per capture. Three consumption paths work — inspect
from metadata alone, rebuild old/new offline (`gircap materialize`), prove the
bundle's own integrity (`gircap verify`). The fourth does not: **run `girpr
--game-path <artifact>/game` and assert it patches to the new version.**

Pointed at such a fixture, `girpr` does exactly what it is specified to do:

- `repair::phases::plan::build_plan` builds the work list from the **whole live `game` manifest**
  — 2 662 common paths measured for 7.0.0→7.1.0, several thousand more across
  the audio fields;
- `file::repair_all` spawns one task per planned file and downloads whatever is
  not intact — on the order of **130 GB**;
- `--purge-after` / `--purge-before` delete **everything the live manifest does
  not contain**, which for a fixture that small is most of it.

None of that is a bug. It is the correct behaviour of a whole-install patcher
given a whole-install input, and `gircap` must not shim around it — the fix
belongs in `girpr`.

### 1.1 Two jobs, not one

| | job | failure if wrong |
|---|---|---|
| **path scope** | *which paths exist* | repairs 130 GB instead of 2.3 MB |
| **resource scope** | *how much work* | a filter typo, or a glob that misses one path, still lets a 100 GB run or a 100 GB delete through |

The first is a **selector**. The second is a **backstop**: it exists precisely
*because* selectors are fallible. A design that ships only globs has no answer
to "what if the glob is wrong". They are separate tiers here because they answer
different questions and fail differently, and because the backstop tier is
worthless without the selector tier being right most of the time.

### 1.2 What a consumer must be able to assert

An end-to-end check against a bundle has three per-capture assertions, not one:

1. `SUMMARY download_bytes` equals `patchmaps/<id>.json → totals.download_bytes`;
2. the patched file's MD5 equals `expectations.result_md5`;
3. **no chunk id in `expectations.must_not_fetch` was requested** — the
   assertion that is invisible in a byte count, and the one that actually catches
   a broken reuse path.

**Assertion 3 cannot be made from the data plane at all.** `SUMMARY` is
byte-level by design — that is what makes assertions 1 and 2 cheap and
diffable — and per-chunk request detail exists only as `trace` lines in
`<exe-dir>/logs/girpr_<ts>.log`. It needs a **request log**: run at
`--log-level trace` and grep it for the ids, or make the requests observable
another way (a server under test can record them itself).

The key-adjacent fact that *is* on the data plane is `local_manifest` (§6):
`unavailable` means `getBuild(local)` could not be served, so the reuse map was
empty and **no chunk could have been reused at all**. That makes assertion 3
vacuously true and explains a `download_bytes` far above the capture's
`totals`. It is a summary of the situation, not a substitute for a request log:
it cannot name an id.

---

## 2. Two design rules

### 2.1 A filtered-out path does not exist

`girsync`'s filter module header states the rule this tool adopts: *"Filtered-out
paths are treated as **nonexistent on both sides**"* — skipped in walks, dropped
from records, left untouched in the destination. For `girpr` "both sides" is
**the repair plan and the purge keep-set**, and the second is the one that is
easy to forget.

That is stronger than "skip it in the repair loop", and the difference is where
the bugs are. `plan.expected_paths()` is the *only* manifest-derived input to the
keep set, so narrowing the plan alone leaves the purge with a **narrower**
keep-set than an unfiltered run — the worst possible outcome. Every site that
touches a path by name has to participate:

| site | under a path scope |
|---|---|
| `repair::phases::plan::build_plan` | drop non-matching `rel` **before** the plan exists, so `files_total` and `expected_paths()` both see the narrowed world |
| `repair::file::repair_all` | already narrowed; nothing to do |
| `repair::phases::check::verify_plan` | already narrowed |
| `repair::purge::plan_purge` / `apply_purge` | hidden paths must be invisible **here too**, or `--purge-*` deletes the fixture |
| `repair::purge::remove_emptied_dirs` | a directory whose only contents were filtered away must not be swept |
| `repair::phases::post::delete_deprecated` | skip any hidden path |
| `repair::phases::post::move_audio_cache` | gate; a scoped dev run should not relocate audio |
| `util::write_config_ini` (from `repair::phases::post::run`) | **not bumped** — see below |
| `repair::phases::metadata` → `audio::write_scan_file` | refused under a scope — see §3 A1 guard 2 |

There are two ways to make a path invisible to the purge: filter inside the
purge walk (`classify_purge_path` gains a scope argument and returns `Keep`
*before* the manifest check), or push every hidden path into `server_keep`. The
first is the better fit. `classify_purge_path` is a pure function with an
exhaustive unit test, and a scope parameter keeps it one; widening `server_keep`
grows a `HashSet<String>` to tens of thousands of entries, turns a pure
classifier into one that needs an answer computed elsewhere first, and gets the
emptied-dir sweep wrong for free — an emptied directory has no files left to
classify.

#### `config.ini` under a scoped run

`post::run` bumps `game_version` last, "so the on-disk version only advances
once the bytes are in place". Under a scoped run that premise is false by
construction: the bytes are deliberately **not** in place.

A scoped run therefore leaves an install whose `config.ini` claims `7.1.0` while
the tree is still `7.0.0` in every excluded path — precisely the launcher
desync the ordering exists to prevent, and strictly worse than a normal partial
failure, which exits `3` and skips the post-phase entirely.

**A run in which any path scope is active does not bump `config.ini`**, and says
so at `warn`. `SUMMARY config_bumped=0` lets a test assert it rather than reading
the file. Without this, every scoped run corrupts the bookkeeping of whatever it
is pointed at.

### 2.2 A ceiling is a backstop, so it refuses rather than admits

Every ceiling is an **enforcement bound**. A run whose plan exceeds a manually
requested ceiling **fails before doing the work**, exit `5`. It never truncates
the work list. Three reasons:

**1. These flags are for testing, and testing wants a failure, not a subset.**
A ceiling exists to stop a 130 GB run from happening. Admission stops the
*downloads* — but the run has already paid for the part that costs most on a
real install: the metadata calls, the manifest fetches, and a plan of thousands
of paths. What it returns is an install that reports success and is still 99%
broken. Refusal returns in seconds, writes nothing, and says by how much it was
over.

**2. A refusal is a better assertion than an admission.** Same flags, only the
ceiling's semantics differ:

| `--include "<the 8 you meant>" --max-files 8` | admission | refusal |
|---|---|---|
| glob correct | repairs 8, exit `0` | repairs 8, exit `0` |
| glob is a typo | repairs 8 **wrong** files, exit `0` | exit `5` before any write: `plan_files=0 limit=8` |
| what the ceiling checked | "did I stay under N" | "did the scope select what you meant" |

Under refusal the two flags stop being redundant. `--include` **selects**;
`--max-files` **verifies the selection**. That is the whole answer to "use it in
case the globs could miss something" — the backstop is not a weaker selector, it
is a *check on* the selector, and it can only be a check if it fails rather than
adjusts.

**3. Admission wants a different verb.** "Take the first N" / "take the cheapest
N" is a *selection*, not an *enforcement*. It should be `--first <N>` or
`--limit <N>`, and a name that says "limit" for a flag that quietly truncates is
its own hazard. Deferred (§8).

Two enforcement points exist and are not interchangeable:

| shape | when | suits |
|---|---|---|
| **pre-flight refusal** | after the plan exists, before any mutation | anything predictable from the plan: file count, download bytes, delete bytes |
| **runtime hard stop** | mid-flight, when a counter crosses the line | anything not predictable from the plan: bytes written into the install, and the **verified** figures — the bytes the repair was allowed to spend, which are only known as the run goes |

Both produce exit `5`.

**What refusal costs.** A limited run produces no partially-scoped install at
all. That is the desired outcome, not a regression: every option here
produces an install that is not a working install, and the sooner the tool says
so, the better.

**What refusal does not bound.** Metadata and manifest traffic. Those fetches
happen before the plan exists, and nothing here changes them (~2–3 MB per run
measured against a live pair). A ceiling bounds chunk downloads, writes and
deletes. It is not a bandwidth cap and must not be described as one.

---

## 3. The flags

Ordered by value. **P0** unblocks `gircap`; **P1** is what makes the resulting
test rigour rather than a demo; **P2** is worth having if cheap.

### A1 · `--include <GLOB>` / `--exclude <GLOB>` (repeatable) — **P0** — path scope

`glob` patterns matched against the `/`-separated, game-dir-rooted rel path —
the same string `phases::plan::build_plan` stores; `util::contained_rel` validates it
at the filesystem boundary. Empty `--include` admits everything; `--exclude` beats
`--include`. The semantics are `girsync`'s, including
`MatchOptions { require_literal_separator: false, require_literal_leading_dot:
false, case_sensitive: false }`.

Three spec details worth stating rather than re-deriving:

- **`*` crosses `/`.** With `require_literal_separator: false`, `--include
  "*.blk"` matches `GenshinImpact_Data/StreamingAssets/.../00/09801749.blk`. A
  user expecting `*.blk` to mean "in the current directory" will be surprised;
  the flag help must say so, because every `*.blk`-shaped include in a shell is
  at risk of it.
- **Case-insensitive**, as in `girsync` (`--case-sensitive` default *false*).
  Game paths are Windows and the live manifests are lowercase anyway, so `girpr`
  hardcodes insensitive rather than growing a second spelling of that decision.
- **A bad pattern is exit `1`**, naming the offending glob.

**Three guards.** A filter that matches nothing currently produces a cheerful
`exit 0` having done nothing, which is the single most likely way this feature is
misused:

1. **A run that selected nothing at all → exit `1`**, naming the globs. The test
   spans *both* plans: a repair plan and a purge-candidate list that are both
   empty is a typo. An empty repair plan alone is **not** — a path on disk that
   the new manifest dropped is in no repair plan by construction, and
   `--purge-only --include <that path>` is precisely the request that deletes it.
   (`--exclude` matching zero is fine and unremarkable.)
2. **Explicit `--audio` under an active path scope → exit `1`,** unless the audio
   scan file is itself in scope. The run writes `audio_pkg_scan_dir` on any
   explicit `--audio` — in the execution phase, after every plan-half refusal,
   so a refused run leaves the install untouched. Under a scope that write is a
   mutation of a path the scope may not have admitted, so the combination is
   refused outright. Two flags asking for opposite scopes is a contradiction,
   not a merge. The escape hatch
   is to include the scan file's own rel path (`GenshinImpact_Data/Persistent/
   audio_lang_14` for a global install) — it comes from `getGameConfigs`, so it
   is never hardcoded.
3. **Any active scope plus a purge that would touch a hidden path → `warn` at plan
   time**, listing the count. Silent agreement is correct, but silence should be
   visible once.

`glob` resolves offline (`Cargo.lock` carries it via `girsync`).

### B1 · `--max-files <N>` — **P0** — pre-flight refusal

Refuse when `plan.files.len() > N`: before any file task spawns, before any
directory is created, before any `_tmp` is opened. Checked on the **filtered**
plan, so `--max-files` always means "the files the scope selected". `N == len`
does not refuse; the boundary is the point. `0` is a valid ceiling.

### B2 · `--max-download-bytes <N>` — **P0** — pre-flight refusal **plus** runtime hard stop

This is the ceiling the `gircap` byte assertion is written against, so its
accuracy matters.

**The plan estimate is computable up front.** `manifest::local_reuse_map` has
already built `local_map: path -> [(uncompressed_md5, size, offset)]` by the time
the plan exists, and per planned file:

```
est_download_bytes(rel) = Σ c.compressed_size
                           for c in plan[rel].chunks
                           where (c.uncompressed_md5, c.uncompressed_size)
                                 ∉ local_map[rel]
```

That is character-for-character the capture's `patch_fetch` definition and
therefore its `totals.download_bytes`. Because capture invariant **A6** asserts
`len(raw blob) == compressed_size` while `file/mod.rs` counts
`stats.download_bytes += blob.len()`, the estimate and the runner's counter are
the same quantity by construction. This is the figure without `--scan`, and
there it is an upper bound: the map describes the local *old* build, so a chunk
already correct in a partially updated target is counted even though the
in-place path keeps it.

**`--scan` replaces that formula with the path the run will really take.** Per
file it resolves the mode at plan time — temp when requested or the target is
not a regular file, discard (delete first, so every chunk downloads), in place
otherwise — and models the ordered salvage tier with the writer's own
`salvage_order` liveness. A chunk whose source the chosen direction overwrites
is counted even if the scan found the source intact. In place uses the ordered
tier's (larger) figure, which is also the staged tier's over-approximation,
because the RAM budget that chooses between the tiers is a runtime decision.
The scan cannot predict an in-place open that fails and falls back to temp; that
file fetches its correct chunks too, and only the runtime hard stop bounds it.

**Both mechanisms are needed, and the second is the one that actually enforces.**

- **Refusal (deterministic, pre-flight).** `Σ est_download_bytes over the
  filtered plan > N` → exit `5`, nothing written, no `_tmp` created. Its value is
  that it fails *before* the expensive part.
- **Hard stop (the enforcement).** The estimate is **not a promise**:
  `try_reuse_slice` falls back to a download whenever the local slice fails its
  MD5, so a file estimated at 0 bytes can still download its whole body. Even
  under `--scan`, an in-place open that falls back to temp, or a source that
  changes after the scan, downloads more than predicted. Refusing on the
  estimate alone would therefore not bound anything. A
  live reservation on `Summary` trips at the real ceiling; in-flight and
  remaining files are dropped, their `*_tmp` left in place — S3 resume-by-length
  means an aborted run is not a corrupt run, it is a run that did less — and the
  run exits `5`.

The reservation is a compare-and-swap taken **before** each fetch, not a
check-then-add: under `--io-threads N` a check overshoots by up to N−1 chunks,
and a ceiling is not a guideline. Once the fetch ends, the reservation is
reconciled with the bytes the attempt really delivered — a failed attempt's
partial body is charged, not refunded — and a settle that carries the live
figure past the limit trips the hard stop right there, including on the run's
last chunk. The overrun is therefore bounded by the body already in flight (the
piece that crossed the line), not by a whole reservation. `SUMMARY
download_bytes` counts the bytes of the files that were actually **promoted**,
so a failed-and-retried attempt does not inflate the reported total; the bytes
that attempt did fetch are still counted against the ceiling, because they
crossed the wire.

### B3 · `--max-write-bytes <N>` — **P1** — runtime hard stop

The only ceiling that *cannot* be a refusal, and the reason it exists:
**`download_bytes` does not count most of the disk the run touches.** Every
reuse-copy (`out.write_all(&buf[..n])` in `try_reuse_slice`) and every
downloaded-chunk write lands in the install — the target itself on the
in-place path, `{path}_tmp` on the temp path — and none of it appears in
`SUMMARY.download_bytes`. A zero-reuse run writes its entire payload to disk
while reporting a network cost only.

Bytes written are also not knowable from the plan — reuse success is decided per
chunk at runtime, against bytes that may not be there. So this is a mid-flight
stop and nothing else.

The number that matters for capacity is *peak* disk (`io_threads × largest
file`). This bounds the cumulative counterpart, which is what a runaway actually
blows.

### B4 · `--max-delete-bytes <N>` — **P0** — pre-flight refusal

Deletion is the one irreversible act in the tool, and a partial purge is worse
than no purge: it leaves a tree that is neither the old install nor the new one.
Refusal is therefore the only acceptable shape. It is answerable because the purge
plan is built before anything is deleted.

Once planned: if `Σ candidate sizes > N`, delete nothing, exit `5`. The
post-repair purge pass re-checks against its own re-planned candidates before
deleting, so the bound does not rest on the pipeline's current promise that a
successful repair promotes every `*_tmp` it creates.

**Interaction with `--purge-before`.** `--purge-before` exists to free room for
the repair, so refusing it because the ceiling is low would defeat its purpose.
The pre-flight ceiling is therefore evaluated **once over the union of both purge
passes**, not per pass, or the number is wrong in both directions.

**Interaction with path scope.** A hidden path is not a purge candidate, so it
never counts toward the total. That is the reason the scope and the ceiling are
complementary rather than redundant: the glob narrows what is *counted*, the
ceiling bounds what is *done*.

`post::delete_deprecated` is a handful of files and needs no ceiling.

### B5 · `--max-verified-download-bytes <N>` / `--max-verified-write-bytes <N>` — **P1** — runtime hard stop

The two ceilings the **live/settled** split makes possible.

**The problem they solve.** `--max-download-bytes` and `--max-write-bytes` bound
the **live** figures: the traffic the run caused, and the disk it took. Neither
bounds **the repair** — how much of the install the run actually had to
rewrite. So a run against a flaky origin gets stopped early (the budget was
eaten by retries that produced nothing), while a run that legitimately rewrote
40 GB of the install is not bounded by anything except the disk.

These two bound the **settled** figures instead: the bytes of the files that
were actually repaired, and the bytes the repair actually wrote into the
install. A budget on the *outcome*, not on the *cost*.

**Both are runtime hard stops**, never pre-flight refusals: the split between
the two figures is only known at runtime, so there is nothing to refuse
against. Both count towards `limited=1`, and both answer with exit `5` and a
`stopped=` value that names the flag that bound.

**The line is checked when the bytes are settled**, so it can be crossed by the
write that settled them: one chunk for the disk, one file for the network. The
bytes are counted either way — the write happened, and a figure that stopped
counting would be a lie — and the run stops with the work it had done left in
place, which is what every other hard stop does.

**Why two flags and not one.** A user who wants "do not rewrite more than N
bytes of my install" wants the write one; a user who wants "do not fetch more
than N bytes that actually land in files" wants the download one. Collapsing
them into a single `--max-repair-bytes` would have to pick one of the two
quantities, and the other would stay unbounded.

**They compose with the existing ceilings, and the order matters.** The live
ceilings are checked first — they bound the disk and the network — and the
verified ones are checked when a resource is settled. A run with both
`--max-write-bytes` and `--max-verified-write-bytes` stops on whichever is
reached first, and `stopped=` says which.

**A flag that never fires is not a failure.** A run that never settles that
many bytes is simply not limited by it; there is nothing to report and
nothing to refuse.

### C1 · `--print-plan` + `--plan-out <FILE>` — **P0** — observability

**`--dry-run` does not cover this,** for three concrete reasons:

1. **Cost.** `--dry-run` is not cheap. `repair_one_file` runs `util::file_len`
   and then `util::md5_file` *before* it reaches `if ctx.dry_run { … }`, so a
   `--dry-run` on a real install **full-file MD5s every planned file** — on the
   130 GB case, exactly the cost the limitation is meant to avoid.
   `--print-plan` stops before the repair loop entirely, so it pays none of it.
2. **Plane.** `--dry-run` answers on **stderr / the log file**, via `tracing`.
   Asserting on it means scraping a log with `--log-level debug` turned up. The
   stdout data plane (`REPORT` / `PROGRESS` / `PLAN` / `SUMMARY`) exists
   precisely so a caller can parse it; the plan belongs there.
3. **Totals.** `--dry-run` emits no aggregate. There is no way to ask "how many
   files would this touch, and how many bytes would it download?" without reading
   per-file `tracing` lines and summing them by hand.

**What it does.** Run the phases up to and including the plans, emit the `PLAN`
line, and stop. Nothing after the plan is executed: no purge, no repair, no
post-phase. It returns before `create_dir_all`, so it is read-only by
construction — and it short-circuits `--dry-run` for the same reason.

```
PLAN files=8 est_download_bytes=2373385 est_write_bytes=… est_extra_bytes=… purge_files=0 purge_bytes=0 \
     max_chunk_bytes=3689476 scoped=1
```

`--plan-out <FILE>` writes the per-file detail as **JSON** —
`{"repair": [{rel, size, chunks_total, est_download_bytes, mode, extra_bytes}], "purge": [{rel,
size}]}` — for the `--per-class`-sized cases where diffing eight lines beats
eight `--include` flags. It works with or without `--print-plan`: on its own it
writes the plan of the run that then does the work, before the first mutation,
so a write failure is exit `3` with nothing changed. JSON because
`girsync --output json` and the whole artifact side of this repo (`gircap`'s
bundle, `patchmaps/<id>.json`) are JSON, and a parser that has to read
`SUMMARY` *and* the plan wants one reader, not two vocabularies. The
`key=value` plane stays single-purpose: one line per fact, greppable, no
nesting.

The purge half needs the purge plan built unconditionally whenever a purge
flag, `--print-plan` or `--purge-only` asks for it — which is why
`PLAN purge_files=… purge_bytes=…` is available without passing a purge flag that
would delete things.

**Not an offline mode and it does not pretend to be.** Fetching the manifests is
not part of what `--print-plan` does differently; every run does it, exactly as
it calls the API first. The plan *is* the manifest — there is no plan without it.
So `--print-plan` fetches, logs at the usual level like any other run, and the
help text says so, because "print the plan" invites the assumption that it is a
local, free operation.

**It is not `--check-only`:** it answers "what would you do", not "what is
wrong". A ceiling still trips it — narrowing what is verified is not a licence to
skip the assertion.

### D1 · `--hyp-base <URL>` / `--sophon-base <URL>` — **P1** — endpoint override

The seam already exists end to end: `RunCtx` has `hyp_base_override` /
`sophon_base_override`, `RunCtx::api_bases` resolves explicit > env > production,
and `HypClient::new_with_bases` takes them. The flag spelling matches `gircap`'s
so that one invocation works against either tool.

**The caveat is the important part.** `--sophon-base` redirects **`getBuild`
only**. Manifest and chunk URLs are not built from the configured base at all —
`sophon::fetch_manifest` and `sophon::stream_chunk_to` both call
`sophon::join_url(prefix, id)` on a `url_prefix` read out of the **response
body** (`manifest_download.url_prefix`, `chunk_download.url_prefix`). Pointing
the flag at a mock therefore redirects the metadata call and leaves every
manifest and chunk fetch wherever the mock's own body says.

Consequently a consumer needs a thin **artifact-serving shim**: serve
`api/*.json`, `manifests/*` and `chunks/blobs/*` with each body's `url_prefix`
rewritten to the shim's own origin. That shim is consumer-side work, and it is
what `docs/06` designs.

### E1 · `--fail-fast` — **P1**

The single most valuable thing for a mock-server test.

`repair_one_file` retries a file up to `MAX_ATTEMPTS = 5` with a linear backoff,
**and** each attempt's chunk fetches retry 5× internally with their own 1–5 s
sleeps. One failing file costs **~125 s of pure sleeping** before any connection
timeout is counted. Across a real plan that is hours of wall clock, and every
failure reports the same thing.

`--fail-fast` stops spawning new file tasks and aborts the rest once one file has
exhausted its retries. Mechanism 1 alone cannot help the *first* file, so the flag
also caps a file at **one** attempt: the flag is an explicit "tell me fast", and a
user who wants the full retry budget does not pass it. Counter semantics are
unchanged, `files_failed` stays honest, and the run reaches its `SUMMARY` in
seconds.

Only worth setting for `5xx`-shaped failures: `girpr` retries only
`408 | 429 | 5xx`, so a 4xx costs nothing on its own.

### E2 · `--purge-only` — **P1**

`--check-only` answers "is it intact"; this answers "make it match the manifest".
Together they cover the read and write halves of the cleanup contract — and
`removed`-class behavior is reachable *only* through the cleanup path.

It runs the phases up to and including the post-phase, skipping the repair loop.
It **implies** the cleanup, so no purge flag is needed: a flag that cleaned up
nothing would be a lie. It does **not** bump `config.ini`, for the same reason a
scoped run does not — a cleanup puts no bytes in place, so claiming the newer
version would be the launcher desync the ordering exists to prevent, and on an
already-current install it is a no-op anyway. `SUMMARY config_bumped=0` says
so.

### E3 · `--max-chunk-bytes <N>` — **P2**

One job left, and it is real:

- **Targeting.** The capture measured the largest real Genshin chunk at
  **3 689 476 B compressed**, and the same measurement makes the flag a way to
  *deliberately* exercise the largest-chunk branch, which no small synthetic
  fixture reaches.

**The memory half is gone.** It used to be the flag's reason to exist: S1
buffered whole chunks (fetch → decode → write), so the peak RAM per active
chunk was roughly `compressed + uncompressed` — at `--io-threads 8`, a real
number. The chunk path is **streamed** now: the body is decoded as it arrives
and the decoded bytes are written into the target as they are produced, so the
peak per active chunk is one read window, not the chunk. Skipping files whose
largest chunk exceeds `N` bounds nothing about RAM any more.

`PLAN max_chunk_bytes=` reports this number today without enforcing it — it is
printed because printing it costs one line.

### E4 · `--no-skip-intact` — **P2**

`repair_one_file`'s fast path is `size + full-file MD5`. On a 100 GB install that
is a 100 GB read before a single byte is downloaded — the largest fixed cost in
the tool. Under a path scope it is paid only for the files the scope selected,
which is already a large improvement. This flag skips the check and forces every
selected file down the download path, so a test can exercise reuse and downloading
*without* paying for the hash. Deliberately not a production flag: it forfeits
the skip, which is the main reason the tool is low-disk.

### E5 · `--manifest-field <NAME>` (repeatable) — **P2**

`sophon::select_manifests` returns the `game` field plus one per selected audio
language, each ~1 MB compressed and ~177 further file entries. Capping to `game`
cuts fetch and parse cost when the audio manifests cannot hold the file under
test. Low value — `--audio none` plus `--audio en-us` already spans it — and it
adds a third way to say "which manifests", so it should be cut if the surface
needs trimming.

---

## 4. Pipeline order

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

**Filter before assert.** The ceilings are checked on the *filtered* plan, so
`--max-files` always means "the files the scope selected", never "the files the
scope happened to keep from a bigger number". Checking before filtering would
make every ceiling unusable — they would always refuse on a real install.
Without a filter, a ceiling is a refusal on any real install, which is the
correct answer rather than a limitation of the design.

**Assert before everything mutates.** The purge plan is built here too, so a
`--max-delete-bytes` refusal happens before a single byte is deleted.

**A refusal has no partial state, except one.** For B1, B2's pre-flight half and
B4: nothing has been written when the run exits `5` — no directory created, no
`_tmp` opened, no file deleted, no `config.ini` touched. The single exception is
B2's, B3's and B5's runtime hard stop, which by definition fires mid-flight and
leaves `*_tmp` behind. That is safe, but it is state, and `SUMMARY stopped=`
must say so rather than leaving it to be discovered.

### 4.1 Read-only modes

`RunCtx::readonly()` already unifies `--dry-run` and `--check-only`, and every
delete/write is gated on it. Ceilings apply in read-only modes too — that is
where they are most useful, since a read-only run is the natural first probe,
and a ceiling there costs nothing and refuses for free.

| | `--dry-run` | `--check-only` | `--print-plan` |
|---|---|---|---|
| `--include` | predicts exactly the narrowed work | verifies exactly the narrowed set | reports exactly the narrowed set |
| `--max-files N` | refuses before the hash storm | refuses before verifying | reports `files=` and refuses if over |
| exit on a tripped ceiling | `5` | `5` | `5` |

`--check-only` keeps its `0`/`4` semantics — a ceiling narrows *what is
verified*, and `4` must not be masked by narrowing — but a ceiling that trips
during `--check-only` is still `5`, because that is a different fact from
"damage found".

---

## 5. Exit codes

| code | meaning |
|---|---|
| `1` | usage — bad glob, an `--include` that selected nothing, `--audio` under a scope, unparseable ceiling value |
| `2` | metadata / network |
| `3` | write / verify |
| `4` | `--check-only` found damage |
| **`5`** | **a manually requested enforcement limit was exceeded** |

**Why `5` and not `1`.** The instinct is that a flag check is a usage error. It
is not: **the API has to be fetched first**, and whether a limit is exceeded is
only knowable from the server's answer. `--max-files 0` against an empty plan is
not an error; the identical flag against a 3000-file plan is. A code that means
"the run was misconfigured" cannot mean both, so `5` means "the run failed", and
it is deliberately paired with the `SUMMARY` line and exit `0` on success.

`exit 5` whenever a limit actually bound, **even when the work that *was* meant
to happen would have succeeded**. `exit 0` means "this run did everything it was
scoped to do"; if it also meant "and the scope covered everything", a limited run
and a full run would be indistinguishable and the first thing a test script would
do is assert `0`. A script that wants the weaker guarantee tests `$? -ne 5`.

A **refusal emits `SUMMARY`** and is *returned*, not raised as a failure: it is a
reported outcome and needs the same uniform tail as `0`/`3`/`4`. The diagnosis
itself goes to stderr and the log file (`ERROR limit exceeded: --max-files
exceeded: plan_files=3000 limit=8 …`), because `SUMMARY` carries counters and
flags, not prose. The one thing a refusal has and a normal run does not is that
every value in it is `0` except the plan-derived ones, which is itself the proof
that nothing was written.

**`stopped=` names the flag.** The value is the flag that bound
(`--max-download-bytes`, `--max-write-bytes`, `--max-delete-bytes`), not a
vocabulary word. It is a **breaking change** to the `SUMMARY` line: the three
values `bytes` / `write` / `delete` are gone, replaced by the flags they stood
for. A consumer that matched on them has to be updated; a consumer that only
looked for the key's presence is unaffected. The reason is the one the plan
weighed: the flat vocabulary needed a new word per ceiling, and the words said
less than the flags do — `stopped=--max-write-bytes` is self-describing, it
scales with the number of ceilings, and it is the shape a script already has
to write.

---

## 6. `SUMMARY` additions

Emitted **only when the corresponding limit is active**, which is what keeps the
machine contract non-breaking: an unflagged run's output is byte-identical to a
run that never passes the flag.

### 6.1 Two numbers for every byte count

A byte count answers two questions, and they have different answers: *what did
the run **spend*** and *what did the run **achieve***.

- The **live** figure is the spend. It is charged *before* the resource is
  consumed, so it is what a runtime hard stop reads and what a budget is
  about: the bytes are spent whether they turn out to be good or not.
- The **settled** figure is the achievement. It is charged *after* the resource
  validated, so it is a function of the *install* rather than of the transport.

The two are equal in a clean run and differ exactly when something failed, and
the difference is the waste: the bytes that were spent and did not become part
of the install. Both are on the data plane, because a reader who has only one
of them cannot tell a clean run from a lucky one.

**What the byte counts are worth.** A byte count is worth exactly what its
question is worth: the settled figures answer *what did the repair achieve*,
so they must not move because the network was flaky — the same install against
a clean origin and against one that serves a corrupt chunk has to report the
same number — and the live figures answer *what did it cost*, which is the
question a budget asks.

### 6.2 The keys

| key | when |
|---|---|
| `limited=1` | any ceiling active |
| `scoped=1` | any path scope active |
| `plan_files=<N>` | any ceiling or scope active |
| `plan_download_bytes=<N>` | `--max-download-bytes` active |
| `download_bytes_spent=<N>` | `--max-download-bytes` active. The bytes that **crossed the wire**, the abandoned attempts' included: the traffic the run caused, which is what the ceiling bounds. `download_bytes` is the other half — the bytes of the files that were actually repaired — and the two are the same number in a clean run and differ exactly when the transport misbehaved |
| `write_bytes=<N>` | `--max-write-bytes` **or** `--max-verified-write-bytes` active. It counts the bytes written into the install **from sources that passed their gate** — a chunk's MD5, a reuse slice's MD5, a staged chunk's MD5, the scan's source verdict, the structural zero-fill — so it is a function of the *install*, not of the transport: a corrupt chunk contributes nothing, and a retry does not move it. The bytes that landed without passing a gate are real and are what the ceiling bounds, but they are not this number |
| `write_bytes_spent=<N>` | `--max-write-bytes` active. The bytes that **landed in the install**, the bytes of an attempt that later failed its digest included: the disk the run took, which is what the ceiling bounds. `write_bytes` is the other half — the bytes written from sources that passed their gate |
| `config_bumped=0` | `config.ini` was deliberately **not** bumped — a path scope is active, or `--purge-only` ran |
| `stopped=<the flag>` | a runtime hard stop fired, not a refusal. The value is the **flag** that bound — `--max-download-bytes`, `--max-write-bytes`, `--max-delete-bytes`, `--max-verified-download-bytes`, `--max-verified-write-bytes` — rather than a vocabulary word: it is self-describing, it scales with the number of ceilings, and it is the shape a script already has to write. The key says *which* ceiling bound, so two stops cannot share a value |
| `local_manifest=ok\|unavailable` | a local `config.ini` version was known and `getBuild(local)` was therefore attempted. `unavailable` means the reuse map was empty, so every chunk downloaded — the one degradation that moves `download_bytes` and the plan's estimate while still exiting `0` |

`download_bytes` is **not** in this table: it is one of the fixed keys, and it is
the **settled** download figure — the bytes of the files that were actually
repaired. It is on every `SUMMARY` line, flagged or not, because it is the
run's headline number.

`stopped=` is what distinguishes "nothing was written because the plan was too
big" from "something was written and then we ran out".

`local_manifest` is not a limit and not a verdict; it is the key a reader needs
in order to *trust* the two byte counts. It is absent on a fresh install (no
local version, so nothing was looked up) and is not a substitute for a request
log (§1.2).

---

## 7. Module layout

```
src/repair/phases/plan.rs
                RepairPlan, PlannedFile, est_download_bytes, build_plan
src/repair/purge.rs
                PurgePlan, PurgeCandidate, plan_purge, apply_purge
src/repair/scope/
  mod.rs      re-exports; the assertion that turns a plan into pass/refuse
  filter.rs   PathScope, is_visible, compile_patterns     (pure)
  limits.rs   Limits (files / download / write / verified / delete) (pure)
  ledger.rs   Meter, StoppedBy, the runtime reservations the hard stops read
  report.rs   PLAN line + --plan-out JSON
```

The plan is **complete before anything mutates**: the repair work list *and* the
purge candidate list are both built up front, so "how much would this delete?" is
answerable and "how much would this run download?" is known before the first
write. The layout mirrors `tools/sync/src/rw/` (`mod.rs` wiring, `limits.rs`
pure config, `gate.rs` counting) and `girsync`'s `planner/` → `apply` split.

`plan_purge` also derives the emptied-directory set during the same walk (a
per-directory kept-count map, deepest-first, `game_dir` excluded), so
`remove_emptied_dirs` does not re-walk. Equivalent to "try `remove_dir` on every
directory" — a dir with kept content fails `remove_dir` harmlessly — but decided
up front, which is the property the scope work needs. A directory the active
scope hides is not added to the sweep set at all (it has no file to mark its
ancestors), and is counted in `PurgePlan::hidden` so the scope warning names it.

The pure modules unit-test without a filesystem, exactly like
`purge::classify_purge_path` and `phases::plan::build_plan` do. The impure part is
the ledger and the gate calls in `file/mod.rs`.

Touch points outside the two directories, all one-line insertions at an existing
branch:

| file | change |
|---|---|
| `src/cli.rs` | flags + `into_ctx` |
| `src/config.rs` | `PathScope` + `Limits` on `RunCtx` |
| `src/repair/mod.rs` | build both plans → filter → assert → execute; `PLAN` emit; gate `config.ini` |
| `src/repair/phases/post.rs` | scope gate on deprecated deletes and audio move |
| `src/repair/file/mod.rs` | write-bytes ledger + download-bytes hard stop + fail-fast |
| `src/report.rs` | the conditional `SUMMARY` keys |
| `README.md` | flag table, machine-contract table, exit-code table |

---

## 8. Deliberately not shipped

- **Admission** — `--first <N>` / `--limit <N>`, "take the cheapest N". It is a
  selection, not an enforcement, and it wants its own name (§2.2 reason 3).
- **A `--scope-profile` file.** Readable, but the flags compose fine and a
  profile is a second surface to document, test and keep in sync.
- **Rate limiting / bandwidth caps.** `gircap` measures a 2.37 MB total for its
  patchable captures; nothing here needs a throttle, and ceilings do not bound
  metadata traffic (§2.2).
- **Sharing code with `gircap` or `girsync`.** Independence between the tools is
  a design constraint, and the filter and the budget are each a few dozen lines
  that will be written more than once.
- **Making any of this default.** Every option here produces an install that is
  not a working install. Defaults stay as they are.