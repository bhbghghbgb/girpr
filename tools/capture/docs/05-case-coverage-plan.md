# Plan — the capture cases vs. the new `girpr` repair spec

**Status: plan. Nothing here is implemented.**

**This document is temporary.** When the work is done its content is folded
into the documents that own it — `docs/00-spec.md` (the case space and the
class table), `docs/01-artifact-schema.md` (the patchmap's fields), the
`README.md`, and `girpr`'s `docs/06-artifact-bridge-spec.md` — and this file is
deleted. It is the plan of record *during* the work, not a document that
ships.

---

## 0. The goal, and the shape of the work

**The goal.** The bundle must cover every *content* decision the new repair
spec makes; the class ids and descriptions must say what the case actually
does; and the bridge must be rewritten to match.

**What changed.** `girpr`'s repair spec changed on 2026-10-20 (merge `f9e6f6a`,
branch `repair/low-disk`): the repair is now written **in place** by default,
in two tiers (a RAM-staged tier and an *ordered* tier that picks a write
direction), with a *trim* that is sometimes eager and sometimes deferred, and
with `--repair-mode {auto,inplace,temp,discard}` and `--max-ram-bytes`
choosing between the paths. The **output contract did not change** — `REPORT`,
`PLAN`, `SUMMARY` and the exit codes are the same.

**Two halves, and the order between them.** The bundle is the *input* to the
bridge, so the work runs:

```
gircap: the class set  →  gircap: the bundle's new fields  →  a capture
        →  girpr: the bridge, rewritten  →  the bundle promoted  →  the docs
```

**Backwards compatibility is not required, by decision.** The class ids change,
and a class id *is* a patchmap's file name, so the three bundles under
`artifacts/` become unreadable the moment the class set changes. They are
replaced by one re-captured bundle, not migrated. From the class-set commit
onwards, `artifacts/` is stale and the working bundle lives in a scratch
directory; the promotion step puts the new one in its place.

**What is not a commit.** The capture and the promotion touch `artifacts/`,
which is gitignored. They are steps in this plan, not commits.

---

## 1. The decisions, and which of them a class can force

`girpr` decides, per file, the following. The **flag axes** are not content
shapes and are not classes; the rest are, and each one is a decision that a
wrong answer turns into a different file or a different byte count.

| # | decision | what it depends on | a class can force it? |
|---|---|---|---|
| D1 | **skip** — the file is already correct | the whole-file MD5 | yes |
| D2 | **what happens to each chunk**: *kept* (already correct where it belongs), *copied* (its bytes live elsewhere in the file), *fetched* (no usable local source) | the old/new chunk layouts | yes |
| D3 | **the tier** — RAM-staged (the moving bytes fit the budget) or ordered (they do not) | the payload size vs `--max-ram-bytes` | the payload is content; the budget is a flag |
| D4 | **the direction** — ascending or descending, whichever keeps more reuse | the *sign* of the moves | yes |
| D5 | **the trim** — eager (the dead tail is released before the first write) or deferred (the tail waits for the end) | the old size vs the new size, and where the sources are | yes |
| D6 | **the growth** — the file on disk is shorter than the target, so the in-place path has to extend it | the two sizes | yes (as a property) |
| D7 | **the mode** — in place / temp / discard, and the fallback when the target cannot be opened for writing | a flag, and the filesystem | no — a flag axis |
| D8 | **the resume** — in place re-verifies and continues; temp resumes by length | an interrupted run | no — a fault injection |
| D9 | **the zero-fill** of a hole in the chunk coverage | a manifest that does not tile its file | no — a malformed manifest |

D1–D6 are the *content* decisions — a class is a statement about how the
content changed, so those six are what the class set has to cover. D7–D9
belong to the bridge (a flag) or to fault injection, and §6 says where each
one goes.

### 1.1 What the bundle exercises today

Measured by `tools/case_coverage.py` against the reference bundle (see §8 for
the instrument). The `game/` fixture is the **old version's** bytes, so every
capture is a version upgrade.

| capture | kept | moved | fetched | payload (RAM) | ordered direction | trim |
|---|---|---|---|---|---|---|
| `identical` | 1 | 0 | 0 | 0 | — (skipped) | — |
| `zero_reuse_single` | 0 | 0 | 1 | 0 | — | — (grows) |
| `zero_reuse_multi` | 0 | 0 | 2 | 0 | — | — (grows) |
| `partial_same_offset` | 1 | 0 | 2 | **0** | — | — (same size) |
| `partial_moved_offset` | 4 | **4** | 2 | **5 892 269** | **ascending** (keeps all 4 moves; descending keeps 0) | **deferred** |
| `partial_size_changed` | 9 | 0 | 1 | **0** | — | **eager** (24 067 B released) |
| `added` | — | — | 1 | 0 | — | — (created) |
| `removed` | — | — | — | — | — | — (purged) |

Three things follow, and they are the whole reason this work exists:

1. **`partial_same_offset` no longer names a reuse.** A same-offset source
   *is* the chunk's own destination, so the chunk is already correct where it
   belongs: it is **kept**, not copied. The capture is "one chunk kept, two
   fetched and written in place" — the corruption-repair regime's behaviour,
   payload 0.
2. **`partial_moved_offset` is the only capture that exercises the
   moving-bytes path at all**, the only one that reaches the ordered tier's
   rule, and the only one that reaches the deferred trim. Drop it and the
   bundle has no coverage of the point of the spec change.
3. **Every move in the bundle goes the same way** (down: the source is *after*
   the destination). The ascending order keeps all four; the descending order
   keeps none. The rule's other branch is never reached.

---

## 2. The target class set

Eleven classes. The predicate is over the old and the new manifest entries
only — no filesystem, no network, exactly as `select::Shape` is today.

| id | predicate | the decision it forces | replaces |
|---|---|---|---|
| `identical` | `old.md5 == new.md5` | D1 — the skip branch | — |
| `zero_reuse_single` | `reused == 0`, `nchunks == 1` | D2 — no local source at all: the one chunk is fetched and written in place | — |
| `zero_reuse_multi` | `reused == 0`, `nchunks > 1` | D2 — the same, over several writes | — |
| `kept_in_place` | `0 < reused < nchunks`, `moved == 0` | D2 — the surviving chunks are already correct where they belong: **kept**, not copied. The payload is 0, so the staged tier stages nothing | `partial_same_offset` |
| `moved_ascending` | `moved > 0`, every move goes **down** (source after destination) | D3+D4 — the bytes move earlier: the ascending order keeps every source alive | `partial_moved_offset` |
| `moved_descending` | `moved > 0`, every move goes **up** (source before destination) | D3+D4 — the bytes move later: the descending order keeps every source alive | **new** |
| `moved_mixed` | moves in both directions | D4 — the chunking re-flowed: the ordered tier keeps the larger half and fetches the rest | **new** |
| `trim_eager` | `0 < reused < nchunks`, `old.size > new.size`, no source reaches past `new.size` | D5 — the dead tail is released **before** the first write | `partial_size_changed` |
| `trim_deferred` | `0 < reused < nchunks`, `old.size > new.size`, some source reaches past `new.size` | D5 — the trim waits for the end; the file grows during the repair | **new** (the other half) |
| `added` | path only in the new manifest | the file is created | — |
| `removed` | path only in the old manifest | not planned; a purge target | — |

The one-line descriptions (`Class::exercises()`, which feeds `--help`, the
generated README and the matrix's "no candidate" reason), in the new spec's
vocabulary:

| id | description |
|---|---|
| `identical` | "the skip branch: the scan's whole-file MD5 matches, 0 bytes fetched" |
| `zero_reuse_single` | "no local source: the one chunk is fetched and written in place" |
| `zero_reuse_multi` | "no local source: every chunk is fetched, one at a time, into the target" |
| `kept_in_place` | "the surviving chunks are already correct where they belong: kept, not copied; the rest are fetched" |
| `moved_ascending` | "the bytes move earlier: the ascending write order keeps every source alive" |
| `moved_descending` | "the bytes move later: the descending write order keeps every source alive" |
| `moved_mixed` | "the chunking re-flowed: the ordered tier keeps the larger half and fetches the rest" |
| `trim_eager` | "the file on disk is longer than the target and no source reaches past it: the dead tail is released before the first write" |
| `trim_deferred` | "the file on disk is longer than the target and a source reaches past it: the trim waits for the end" |
| `added` | "the file is created: absent locally, fetched whole" |
| `removed` | "not planned; a --purge-after target" |

`full_reuse_reordered` is **removed**: a file whose every chunk moves is a
file whose moves all go one way, which the direction classes already cover,
and it has zero candidates in this pair. `partial_size_changed` is **split**
rather than dropped: it is not a distinct decision, but it is the capture that
happens to exhibit the eager trim, and the split is what stops the trim from
being an accident.

### 2.1 The candidates, and what the set costs

Measured over the pool the capture selects (`game` + `en-us`, 2 932 paths).
The ranking key is `gircap`'s own: `total_cost` = the compressed bytes of the
old and the new file; "disk" is what the capture adds to the bundle.

| class | candidates | cheapest candidate | `total_cost` | disk |
|---|---|---|---|---|
| `identical` | 1 733 | `…/Resources/browser_assets` | 54 | 28 B |
| `zero_reuse_single` | 121 | `…/AssetBundles/data_revision` | 42 | 16 B |
| `zero_reuse_multi` | 491 | `…/Resources/unity default resources` | 633 588 | 9.9 MB |
| `kept_in_place` | 29 | `…/Plugins/HoYoChannel.dll` | 1 826 067 | 4.7 MB |
| `moved_ascending` | 321 | `…/blocks/00/09801749.blk` | 22 662 068 | 23.8 MB |
| `moved_descending` | 139 | `GenshinImpact_Data/resources.assets` | 40 280 151 | **100.5 MB** |
| `moved_mixed` | 3 | `…/AudioAssets/Banks25.pck` | 243 016 539 | **290.4 MB** |
| `trim_eager` | 56 | `…/blocks/00/00981562.blk` | 21 679 587 | 21.8 MB |
| `trim_deferred` | 282 | `…/blocks/00/09801749.blk` | 22 662 068 | — (captured with `moved_ascending`) |
| `added` | 84 | `…/Plugins/libHttpClient.GDK.dll` | 114 357 | 249 KB |
| `removed` | 11 | `…/blocks/00/00664172.blk` | 1 731 133 | 1.7 MB |

**The admission order is a decision.** The greedy admission charges each class
the cheapest candidate that is not already captured, so the *direction* classes
have to be admitted **before** the trim classes: `trim_deferred`'s cheapest
candidate is the same file as `moved_ascending`'s, and admitting the trim
first would push `moved_ascending` onto the next candidate
(`…/blocks/00/02522028.blk`, +24.9 MB). The order is:

```
identical, zero_reuse_single, zero_reuse_multi, kept_in_place,
moved_ascending, moved_descending, moved_mixed, trim_eager, trim_deferred,
added, removed
```

**And the admission has to learn `covered_by`.** Today a class whose cheapest
candidate is already taken does not reuse it — it buys the *next* candidate
(`select/mod.rs`, phase 1 excludes the paths already in `wants`). So
`trim_deferred` would buy `…/blocks/00/02522028.blk` (24.9 MB) — a second
file that differs from `moved_ascending`'s only in which of the two trim
outcomes it shows — and the matrix would still attribute `trim_deferred` to
the cheaper capture. The admission should instead record the class as
**`covered_by`** the capture that already satisfies it. The status exists
(`MatrixStatus::CoveredBy`) and A17 already accepts it; what is missing is
that the check only looks at the *forced* admissions, not at the wants the
earlier classes picked. The fallback for the one edge case: a `covered_by`
row whose covering capture was refused by the budget becomes `over_budget`,
because a class that cannot have the covering capture cannot have a dearer one
either.

**`moved_mixed` is expected to be unfunded.** The rule's "keep the larger
half" branch deserves a case, but its cheapest candidate is 243 MB of
compressed bytes and 290 MB of fixture. `gircap` already has the mechanism:
`--max-download-bytes` refuses a class it cannot afford and the matrix records
it as `over_budget` with a reason, and the bridge stands such a class down out
loud. The capture's ceiling is set so that `moved_descending` is funded and
`moved_mixed` is not, and the bundle records the truth instead of pretending.

**The size of the new bundle.** The reference bundle is 128.9 MiB
(135 134 161 B). `moved_descending` adds `resources.assets`: 100.5 MB of
fixture (the old and the new file) plus its chunk blobs — up to 40.3 MB of
compressed bytes, less where a blob is shared with another capture — so the
bundle grows by roughly 140 MB, to about **270 MiB**.

---

## 3. The commits

Ten commits and two manual steps. Every commit carries its own gate result in
the message, and each one is green on its own.

| # | commit | package | the gate |
|---|---|---|---|
| C1 | gircap: one fact for "moved" — the diff and the patchmap agree | gircap | `cargo test -p gircap` |
| C2 | gircap: the class set — the new ids, the new predicates | gircap | `cargo test -p gircap` |
| C3 | gircap: the patchmap carries what the repair would do | gircap | `cargo test -p gircap` |
| C4 | gircap: the selfcheck pins the coverage | gircap | `cargo test -p gircap` |
| — | **the capture** (a manual step, not a commit) | | `gircap verify` |
| C5 | girpr: the deferred trim says so | girpr | `cargo test -p girpr` |
| C6 | girpr: the bridge, part 1 — the new class ids and the new expectations | girpr | the ignored bridge suite |
| C7 | girpr: the bridge, part 2 — the ordered tier, the modes, the faults | girpr | the ignored bridge suite |
| — | **the promotion** (a manual step, not a commit) | | `gircap verify` + the bridge suite |
| C8 | gircap: the docs — the case space and the class table | gircap | `cargo test -p gircap` |
| C9 | girpr: the docs — the bridge spec, rewritten | girpr | `cargo test -p girpr` |
| C10 | gircap: fold this plan into the documents, and delete it | gircap | `cargo test -p gircap` |

### C1 — gircap: one fact for "moved"

**Why first.** The class predicates and the patchmap's plan must be computed
from *one* rule. Today they are not:

- `manifest/diff.rs::score` counts a chunk as *moved* when the new offset is
  not among **any** of the old file's offsets for that identity;
- `assemble/mod.rs` picks the **first** old chunk with that identity as the
  source, and `PlanEntry::is_moved()` compares that source with the
  destination.

When an identity occurs more than once in the old file the two disagree, and
a class would then be a statement about a different thing than the patchmap's
plan. `girpr` itself uses the *first* match (`repair::reuse::sources_for`), so
the first match is the rule that has to win.

*Measured:* over the reference pool the two definitions agree on every path
(1 104 changed common paths, **0** disagreements), so this is a consistency fix
rather than a behaviour change — the class populations do not move, and that is
the commit's regression test.

**What changes.**

- `src/manifest/diff.rs`: `PathScore` gains `moved_up: u64`, `moved_down: u64`
  and `old_max_end: u64` (the old file's largest chunk end — the input to the
  trim test). `score()` computes the direction from the *source* offset, and
  `moved` becomes "the source offset differs from the destination".
- `src/assemble/mod.rs`: a test that the diff's `moved` and the plan's
  `is_moved()` are the same number for the same path, over the fixture the
  crate already builds.
- `src/select/classes.rs`: `Shape` grows the fields the new predicates need
  (`moved_up`, `moved_down`, the two sizes, the trim flag). **No predicate
  changes yet** — the class set is still the old one, so the bundle stays
  valid.

**The gate.** `cargo test -p gircap` — the populations must not move: the
matrix's `candidates_considered` counts (1 733 / 121 / 491 / 12 / 463 / 17 /
84 / 11 / 0) are the regression test, and the reference bundle's
`diff/matrix.json` is the fixture they are checked against.

### C2 — gircap: the class set

**What changes.**

- `src/select/classes.rs`: the `Class` enum becomes the eleven classes of §2
  — the ids, the `exercises()` descriptions, `matches()`, and `Shape::from_row`.
  `Class::ALL` and `Class::REQUIRED_ORDER` become one list again, in the order
  §2.1 gives.
- `src/select/mod.rs`: the order, the `also_satisfies` bookkeeping, and the
  **admission's `covered_by`**: a class whose cheapest candidate is already
  taken by an earlier class is recorded as `covered_by` that capture instead
  of buying the next one (see §2.1 — without this the bundle pays 24.9 MB
  for a second copy of the same shape). The one edge case: a `covered_by` row
  whose covering capture was refused by the budget becomes `over_budget`.
- `src/cli.rs`: nothing to change — the `--require-class` error message lists
  the valid set from `Class::ALL`.
- `src/manifest/diff.rs`: the `PathScore` → `Shape` conversion carries the new
  fields.
- `tests/matrix.rs` and the classes' own unit tests: the new table. The
  existing test hard-codes the number of classes (9) and the last one
  (`full_reuse_reordered`); both change.
- `docs/00-spec.md` §4.1: the class table — the predicates, the "branch it
  exercises" column, and the measured populations (which are **stale** today:
  the row says `partial_same_offset` 29 and `partial_size_changed` 447, the
  code's own predicates give 12 and 17, and the bundle's matrix agrees with
  the code). The new row is measured by `tools/case_coverage.py`.

**What breaks.** The class ids are the patchmaps' file names, so the three
bundles under `artifacts/` stop verifying from this commit on. That is the
decision, not an accident: the bundle is regenerated in §4.

**The gate.** `cargo test -p gircap`, and the class table's own unit test:
every class has a distinct id, a description, a predicate that matches
exactly the shape §2 gives it, and a place in the admission order. Plus an
admission test: a class whose cheapest candidate another class already took
is recorded as `covered_by` that capture, and **no second file is bought for
it**.

### C3 — gircap: the patchmap carries what the repair would do

**What changes.** `src/assemble/mod.rs` grows two blocks, both computable from
what the crate already has (the plan entries' `source_range` vs
`target_range`, and the two sides' sizes):

1. **A per-capture `repair` block** — what the in-place path would do with
   this file, so a consumer does not have to work it out:
   `kept`, `moved`, `fetch` (the chunk counts), `payload_bytes` (the RAM the
   staged tier needs), `direction` (`ascending` / `descending` / `n/a`) with
   `keeps_ascending` / `keeps_descending` (how many sources each order would
   salvage), `trim` (`eager` / `deferred` / `none`), `growth_bytes`
   (`max(0, new.size − old.size)`), and `mode` (`inplace` — what the default
   run takes).
2. **The expected `SUMMARY` keys**, under `expectations`. The new spec's
   counters are conditional on flags (`girpr`'s `src/report.rs`), so the
   expectations are stated per flag: `inplace` / `temp` / `kept_bytes` with
   `--repair-mode`, `ram_bytes` with `--max-ram-bytes` (`0` when the ordered
   tier is selected, because it stages nothing), `damaged_chunks` with
   `--scan`, `write_bytes` with `--max-write-bytes`. They are exact: for
   `HoYoChannel.dll`, `damaged_chunks=2` and `kept_bytes=1 626 019`; for
   `09801749.blk`, `damaged_chunks=6`, `kept_bytes=4 453 236` and
   `ram_bytes=5 892 269`.

`docs/01-artifact-schema.md` documents the new fields.

**The gate.** `cargo test -p gircap`, and the patchmap's own tests: the
`repair` block agrees with the plan it was derived from.

### C4 — gircap: the selfcheck pins the coverage

**What changes.** `src/emit/selfcheck.rs`, `src/verify/mod.rs` and
`src/emit/closeout.rs` grow three assertions, because the three properties
the new classes exist for are otherwise only held by accident:

- **A21 — the bundle covers both trim outcomes**: at least one capture with
  `trim: eager` and at least one with `trim: deferred`.
- **A22 — the bundle covers both write directions**: at least one capture
  whose salvage order is ascending and at least one whose is descending.
- **A23 — the bundle covers a file that grows**: at least one capture with
  `growth_bytes > 0`.

A17 (every class is `captured` or `unavailable`) is unchanged, and A19 (the
two-sided files differ exactly when the class says) is unchanged.

**The gate.** `cargo test -p gircap`, and a negative test: a bundle without
both trims fails A21.

### C5 — girpr: the deferred trim says so

**Why.** The trim's outcome is not in the output contract: the file ends at
the target size either way, and the difference is the *peak* disk usage, which
nothing reports. The only observable is the log — and `girpr`'s `docs/02` step
6 says "the trim is deferred to the end (the file grows during the repair
instead) and **the reason is logged**", while `repair/file.rs` logs only the
eager case (`trimming the dead tail`). A capture that forces the deferred
branch is worth little until the line is there.

**What changes.** `src/repair/file.rs`: the deferred case logs its reason,
next to the eager one, at the same level.

**The gate.** `cargo test -p girpr`.

### C6 — girpr: the bridge, part 1 — the new class ids and the new expectations

**What changes** (`tests/artifact_bridge.rs`, `tests/artifact_bridge/server.rs`):

1. **The class resolution goes through the matrix.** Today the bridge looks a
   class up by the patchmap's file name, and a class the matrix records as
   `captured` or `covered_by` under *another* capture's id (`trim_deferred` is
   covered by `moved_ascending`'s capture) would be reported as missing. The
   new resolution reads `MANIFEST.json → matrix[]` and follows `capture_id`.
   The matrix's own agreement assertion has to follow suit: a `covered_by`
   row names a capture that has a patchmap, and a class with no capture of its
   own has none.
2. **The per-class arms** learn the new ids. The *strict* write-bytes
   inequality — "the in-place run writes strictly fewer bytes than the temp
   run" — is currently keyed on `c.starts_with("partial")`; under the new
   names it is `kept_in_place` and the `moved_*` classes, and it does **not**
   hold for `zero_reuse_*` (nothing is kept) or for `added` (the file is
   created).
3. **The new expectations**: `kept_bytes` and `inplace`/`temp` (with
   `--repair-mode` passed, so the keys are emitted), `damaged_chunks` (with
   `--scan`), `ram_bytes` (with `--max-ram-bytes`) — all against the
   patchmap's new `expectations`.
4. **The exact pre-pass.** With `--scan`, `est_download_bytes` is not an
   estimate but the precise sum over the chunks that are wrong where they
   belong *and* have no intact source. For the bundle's fixture that is
   exactly the patchmap's `totals.download_bytes`, so the pre-pass asserts an
   equality rather than an approximation.

**The gate.** `cargo test -p girpr --test artifact_bridge -- --ignored --nocapture`
against the scratch bundle (`GIRPR_ARTIFACT_BUNDLE` pointing at it). The old
bundles in `artifacts/` are stale by now, so the suite must be pointed at the
scratch one.

### C7 — girpr: the bridge, part 2 — the ordered tier, the modes, the faults

**What changes** (the same two files):

1. **The ordered tier has to be run.** The direction rule is invisible in the
   staged tier: staging keeps every source, so both directions download the
   same bytes. `--max-ram-bytes 0` forces the ordered tier, and then
   `download_bytes == the patchmap total` is an **equality** for the captures
   whose shift is uniform — on `09801749.blk` the ascending order keeps all
   four movable chunks and the descending order none, and on
   `resources.assets` the descending order keeps all 27 and the ascending
   order none. A wrong direction turns the equality into a larger number, and
   that single assertion is what pins D4.
2. **The mode axis**: a third differential arm for `--repair-mode discard`
   (the same file, a full download, zero peak extra disk) and one for "the
   target cannot be opened for writing" (a read-only file in a scratch copy →
   the run falls back to the temp path and still produces the same bytes).
3. **The faults**: the resume paths (kill a run mid-flight, re-run) and the
   "redo the mismatching chunks" second pass (the shim serves one bad chunk
   blob) — the two the class set cannot force.

**The gate.** The same ignored suite, plus the girpr crate's own tests.

### C8 — gircap: the docs — the case space and the class table

`docs/00-spec.md` §4.1 grows the case space of §1 (the D1–D9 table, and the
distinction between a class and a flag axis), the class table of §2 with its
descriptions, and the measured populations — re-measured, not copied.

### C9 — girpr: the docs — the bridge spec, rewritten

`docs/06-artifact-bridge-spec.md` is rewritten to match: the new class ids
and their per-class expectations (§7.2), the ordered-tier run and what the
byte figure means under it (§7.1), the matrix-driven class resolution
(§3.2), the mode axis (§5), and the case count.

### C10 — gircap: fold this plan into the documents, and delete it

The last commit. The content that belongs to a document goes there:

- `docs/00-spec.md`: the case space (§1), the class table and its
  descriptions (§2), the candidate table and the admission order (§2.1), and
  the limits (§5 of this plan).
- `docs/01-artifact-schema.md`: the patchmap's `repair` block and the expected
  `SUMMARY` keys (C3).
- `README.md`: the bundle's size and the reference bundle's numbers.
- `tools/README.md` and `tools/case_coverage.py`: the instrument's docstring
  is re-pointed at `docs/00-spec.md` §4.1 — **the script survives**, because
  the populations it measures are worth re-measuring after any change to the
  classes.

And then `docs/05-case-coverage-plan.md` is deleted, and this sentence goes
with it.

---

## 4. The capture (a manual step, not a commit)

`artifacts/` is gitignored, so the capture is not a commit. It is run from
the working tree, into a scratch directory, and promoted at the end.

**The capture.**

```powershell
# the scratch root (the environment's approved temp)
$scratch = "E:\Symlinks\Temp\opencode\gircap-out"

cd E:\hoccode\hocrust\girpr
cargo run -p gircap -- capture --out $scratch --max-download-bytes 104857600
```

- `--max-download-bytes 104857600` (100 MiB) funds every class up to
  `moved_descending` (the admitted total is ≈ 89 MB) and refuses
  `moved_mixed` (243 MB), which the matrix then records as `over_budget` with
  its reason.
- The capture needs the network and takes minutes; it is run by hand, once
  the class set and the patchmap are in place.

**The gate.**

```powershell
$bundle = (Get-ChildItem "$scratch\hk4e_global\7.0.0__7.1.0" -Directory)[0].FullName
cargo run -p gircap -- verify --artifact $bundle     # exit 0
```

and the matrix must read: **nine** classes with a capture of their own,
`trim_deferred` **covered by** `moved_ascending`'s capture, `moved_mixed`
`over_budget` with its reason, and the selfcheck green (including A21–A23).

**The promotion** (after C7, once the bridge is green against the scratch
bundle):

```powershell
# the bridge needs exactly one bundle under the root
Remove-Item -Recurse -Force E:\hoccode\hocrust\girpr\artifacts\hk4e_global\7.0.0__7.1.0\*
Move-Item "$bundle" E:\hoccode\hocrust\girpr\artifacts\hk4e_global\7.0.0__7.1.0\
```

and then `gircap verify` on the promoted bundle, and the bridge suite once
more without `GIRPR_ARTIFACT_BUNDLE` set — the suite then discovers the bundle
itself, which is the path the spec describes.

---

## 5. What is deliberately not done

**The corruption-repair input state.** The new spec opens with two regimes,
and the first one is the common case:

> **Corruption repair** — the file is the right size and most chunks are
> intact. A chunk's reuse source is at *its own offset* (`r_i == o_i`), so the
> bad chunk is simply overwritten where it lies.

In a real corruption repair the install is at the **current** version, the
local manifest is the current manifest, and the *bytes* are damaged. The
bundle cannot produce that state: `game/` is always materialized from the
**old** manifest, so a chunk that is reused at its own offset is, by
construction, already correct — the bundle never has a damaged chunk whose
local-map entry points at the damage.

What the bundle *does* cover is the **behaviour** that regime produces: the
payload is 0, the wrong chunks are written in place, the good ones are kept
(`kept_in_place`). What it does not cover is the **input**: the case where the
local map has an entry for the damaged chunk and the source check is what
rejects it (`repair::file`'s `scan.source_ok`), rather than the entry being
absent altogether.

This work **states** that, in `docs/00-spec.md` §4.1 and in the bundle's own
README, and stops there. A second fixture flavour — a "damaged current
install" capture, with the `game/` file materialized at the new version and a
few chunks damaged — is a real feature, not a flag: the class model would need
a second dimension and the manifest diff would have to be scored against the
damage rather than against the old version. It is its own change, with its
own cost.

**The malformed-manifest cases** (a hole in the coverage, an overlap) cannot
come from a real capture: the server's manifests tile their files. They are
only reachable with a synthetic manifest, which is a bridge fixture, not a
bundle case.

---

## 6. The flag axes and the fault injections — not classes

These are decisions of `girpr` that a *class* cannot force, because they come
from a flag, a filesystem attribute, or an interruption. They belong to the
bridge (C7), and each one gets a case there:

| what | how the bridge reaches it | what it proves |
|---|---|---|
| `--repair-mode temp` | the differential (already there) | the same file, the whole file written |
| `--repair-mode discard` | a third differential arm | the same file, a full download, zero peak extra disk |
| `--max-ram-bytes 0` | a flag on the `moved_*` captures | the ordered tier and the direction rule |
| `--max-ram-bytes <small>` | a flag | a file that cannot get its reservation degrades to the ordered tier instead of failing |
| the target cannot be opened for writing | a read-only attribute in a scratch copy | the fallback to the temp path, loudly |
| the resume paths | kill a run mid-flight, re-run | in place re-verifies and continues; temp resumes by length |
| the "redo the mismatching chunks" second pass | the shim serves one bad chunk blob | only the named chunk is redone |
| the zero-fill of a hole | a synthetic manifest that does not tile | the hole is zero-filled, and the file still verifies |

---

## 7. The gates, in one place

| when | what runs |
|---|---|
| after every gircap commit | `cargo test -p gircap` |
| after every girpr commit | `cargo test -p girpr` |
| after the capture | `gircap verify --artifact <scratch bundle>` → exit 0 |
| after the bridge commits | `cargo test -p girpr --test artifact_bridge -- --ignored --nocapture` with `GIRPR_ARTIFACT_BUNDLE` set |
| after the promotion | the same, without `GIRPR_ARTIFACT_BUNDLE` (the suite finds the bundle itself) |

The bridge suite is expensive by design (~20 MB of real bundle bytes per run,
four manifests per run) and stays `#[ignore]`d: it is run by hand, when a
repair-spec change is suspected.

---

## 8. The instrument, and the measurements

`tools/case_coverage.py` — the tool that produced §1.1 and §2.1, and the one
that re-measures the populations after C2. Standard library only; it reads a
bundle and nothing else.

```powershell
python tools/capture/tools/case_coverage.py <bundle-dir>
```

- `--section population` — the population of every class, old set and new
  set, over the pool. Its "old set" column reproduces the bundle's own
  `diff/matrix.json` `candidates_considered` counts exactly (1 733 / 121 / 491
  / 12 / 463 / 17 / 84 / 11 / 0), which is what makes the "new set" column
  worth reading.
- `--section outcome` — for each capture, what the in-place repair would do:
  the kept / moved / fetched split, the payload, both directions' salvage
  counts, the trim.
- `--section ranking` — the cheapest candidate per class by `gircap`'s own
  key, and the overlap between the classes.

The two rules the instrument has to reproduce exactly, because `girpr`
implements them that way:

- the reuse **source** is the *first* old chunk with that identity
  (`repair::reuse::sources_for`), which is what `assemble/mod.rs` records in
  `source_range`;
- the direction rule is `r ≥ o` for ascending and `r ≤ o` for descending
  (`repair::reuse::survives`), with a source that reaches past the end of the
  file counting for neither.
