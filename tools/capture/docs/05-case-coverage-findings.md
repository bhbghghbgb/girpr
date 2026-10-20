# Findings — the capture cases vs. the new `girpr` repair spec

**Status: findings. Nothing here is implemented.**

**What changed.** `girpr`'s repair spec changed on 2026-10-20 (merge `f9e6f6a`,
branch `repair/low-disk`): the repair is now written **in place** by default, in
two tiers (a RAM-staged tier and an *ordered* tier that picks a write direction),
with a *trim* that is sometimes eager and sometimes deferred, and with
`--repair-mode {auto,inplace,temp,discard}` and `--max-ram-bytes` choosing
between the paths. The **output contract did not change** — `REPORT`, `PLAN`,
`SUMMARY` and the exit codes are the same — so every assertion the bridge
already makes still holds.

**What this is about.** `gircap` captures **one file per test-matrix class**
(`docs/00-spec.md` §4.1), and a class is "one predicate per distinct decision
the tool under test makes". The spec change moved the decisions. This document
answers: does the bundle still cover all of them, which cases are stale, which
are missing, and what should the ids and descriptions say now.

**How the numbers were measured.** Everything below is measured against the
bundle in `artifacts/hk4e_global/7.0.0__7.1.0/20261017T123335Z` — the two
`game`-field manifests (2 662 common paths) and the `en-us` field — by
re-implementing `gircap`'s own scoring (`tools/capture/src/manifest/diff.rs::score`)
and then applying the new spec's rules to the result. The per-capture table in
§2 is computed from the bundle's own `patchmaps/*.json` and the `game/` fixture
bytes. The scripts are reproduced in §11.

---

## 1. The case space after the spec change

`girpr` decides, per file, the following. The **flag axes** (mode, RAM budget)
are not content shapes and are not cases; the rest are, and each one is a
decision the tool makes that a wrong answer would produce a *different file* or
a *different byte count*.

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

D1–D6 are the *content* decisions. A class is a statement about how the content
changed, so those six are what the class set has to cover. D7–D9 belong to the
bridge (a flag) or to fault injection, and §8 says where each one goes.

---

## 2. What each capture in the bundle actually exercises now

The `game/` fixture is the **old version's** bytes, so every capture is a
version upgrade. Reading each capture's patchmap against the fixture bytes and
applying the new spec's rules:

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

Reading of the table:

- **`partial_same_offset` is now a *different branch* than its name says.** In
  the old spec a same-offset reuse was a *copy* into `{path}_tmp`. In the new
  spec the same-offset source **is** the chunk's own destination, so the chunk
  is already correct where it belongs: it is **kept**, not copied. The capture
  is now "one chunk kept, two fetched and written in place" — the corruption
  repair regime's *behaviour*, payload 0 (the input state is a different
  question; §9).
- **`partial_moved_offset` is the only capture that exercises the moving-bytes
  path at all** (payload 5.9 MB → the RAM-staged tier), the only one that
  exercises the *ordered* tier's rule, and the only one that exercises the
  **deferred** trim. If it were dropped, the bundle would have no coverage of
  the whole point of the spec change.
- **The two trim outcomes are already both present**, but by accident: the
  eager one is `partial_size_changed` (no reuse source reaches past the
  target's end), the deferred one is `partial_moved_offset` (a source does).
  Nothing in the bundle *names* them, so a class-set change can lose them
  silently.
- **`partial_size_changed` is not a branch any more.** With nothing moving, it
  is `partial_same_offset` with a smaller file; the only thing that
  distinguishes it is the trim.
- **Every move in the bundle goes the same way** (down: the source is *after*
  the destination). The ascending order keeps all of them; the descending order
  keeps none of the four. The other branch of the rule is never reached.

---

## 3. The gaps

| gap | why it matters | closeable by a class? |
|---|---|---|
| **G1 — the descending direction is never exercised** | D4 is the rule with two branches, and the bundle can only reach one. A tool whose ordered tier always wrote descending would pass every capture the bridge drives today, because the bridge never reaches the ordered tier (G6) | yes — 139 candidates in the pool |
| **G2 — the "re-flow" case (moves in both directions) is never exercised** | the rule's tie-break ("keep the larger half") is only visible here | yes — but only 3 candidates, all ≥ 290 MB |
| **G3 — the trim is covered but not named** | the eager/deferred decision is a real branch with its own rule and its own log line | yes — as a class pair, at no extra capture cost |
| **G4 — the "already correct" chunks are never *asserted*** | the new spec's biggest win is that a correct chunk is not rewritten; the bundle records the fact in the patchmap but no consumer asserts it | no — a patchmap/expectation change |
| **G5 — the RAM-staged tier's payload is never asserted** | the staged tier is the default; the bundle does not say how much RAM a file needs | no — a patchmap/expectation change |
| **G6 — the ordered tier is never run** | with no `--max-ram-bytes`, the ordered tier is unreachable; the direction rule is therefore untestable | no — a bridge flag (see §7) |
| **G7 — the corruption-repair *input state* is not represented** | the fixture is always the old version's bytes; a real corruption repair is a *current* install with damaged chunks | not without a second fixture flavour (§9) |
| **G8 — the mode axis is only half covered** | the bridge drives `temp` and the default; `discard` and the "target not writable → temp" fallback are not driven at all | no — a bridge change (§8) |

---

## 4. The stale cases

"Stale" here means: the id or the description no longer says what the case
does, or the case no longer names a distinct decision.

| class | what is stale | verdict |
|---|---|---|
| `identical` | nothing (the skip branch is unchanged) | **keep** |
| `zero_reuse_single` | the description says "a single download into an empty `_tmp`". The default mode no longer creates a `_tmp` | **keep, re-describe** |
| `zero_reuse_multi` | the same `_tmp` claim | **keep, re-describe** |
| `partial_same_offset` | the id is literally true but it no longer names a *reuse*: a same-offset source is the chunk's own destination, so the chunk is **kept**. "the reuse happy path" is now false | **rename** → `kept_in_place` |
| `partial_moved_offset` | true, but it does not say *which way* the bytes move, and the way is now the decision | **rename** → `moved_ascending` |
| `partial_size_changed` | the description says "reuse into an oversize `_tmp`, truncated to length" — the `_tmp` and the length gate are the *old* spec's rules. The new rule is the eager/deferred trim | **split** → `trim_eager` + `trim_deferred` |
| `added` | nothing | **keep** |
| `removed` | nothing | **keep** |
| `full_reuse_reordered` | the class is `reused == nchunks` and the layout changed. Under the new spec that is not a separate decision: a file whose every chunk moves is a file whose moves all go one way, which the direction classes already cover. It also has **zero** candidates in this pair, so it contributes nothing to the coverage | **remove** |

`partial_size_changed` is the one judgement call in this table: it is *not* a
distinct decision, but it is the capture that happens to exhibit the eager
trim. It is not dropped — it is **split**, and the two halves are named after
the decision they force, so the trim stops being an accident.

---

## 5. The proposed case set

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

The proposed one-line descriptions (`Class::exercises()`, which feeds `--help`
and the generated README), in the new spec's vocabulary:

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

### 5.1 The candidates, and what the set costs

Measured over the pool the capture selects (`game` + `en-us`, 2 932 paths). The
ranking key is `gircap`'s own: `total_cost` = the compressed bytes of the old
and the new file; "disk" is what the capture adds to the bundle.

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
| `trim_deferred` | 282 | `…/blocks/00/09801749.blk` | 22 662 068 | — (covered by `moved_ascending`) |
| `added` | 84 | `…/Plugins/libHttpClient.GDK.dll` | 114 357 | 249 KB |
| `removed` | 11 | `…/blocks/00/00664172.blk` | 1 731 133 | 1.7 MB |

**The admission order matters.** The greedy admission charges each class the
cheapest candidate that is not already captured, so the *direction* classes
have to be admitted **before** the trim classes: `trim_deferred`'s cheapest
candidate is the same file as `moved_ascending`'s, and admitting the trim
first would push `moved_ascending` onto the next candidate
(`…/blocks/00/02522028.blk`, +24.9 MB). The proposed order
(`identical`, `zero_reuse_single`, `zero_reuse_multi`, `kept_in_place`,
`moved_ascending`, `moved_descending`, `moved_mixed`, `trim_eager`,
`trim_deferred`, `added`, `removed`) is the cheap one.

**The cost of the new set.** The bundle is 128.9 MiB today (135 134 161 B).
`moved_descending` adds `resources.assets`: 100.5 MB of fixture (the old and
the new file) plus its chunk blobs — up to 40.3 MB of compressed bytes, less
where a blob is shared with another capture — so the bundle grows by roughly
140 MB, to about **270 MiB**. `moved_mixed` adds `Banks25.pck`: 290.4 MB of
fixture and up to 243 MB of compressed chunks, which takes it past **600 MiB**.

**Recommendation on `moved_mixed`.** Keep the class — the rule's "keep the
larger half" branch deserves a case — but expect it to be **unfunded** in this
pair. `gircap` already has the mechanism: `--max-download-bytes` refuses a
class it cannot afford and the matrix records it as `over_budget` with a
reason, and the bridge already stands such a class down out loud
(`girpr`'s `docs/06-artifact-bridge-spec.md` §3.2). Set the capture's ceiling
so that `moved_descending` is funded and `moved_mixed` is not, and the bundle
records the truth instead of pretending.

### 5.2 What the *bundle* has to grow, beyond the class set

The classes name the decisions; the bundle also has to carry the **numbers**
those decisions produce, or every consumer has to re-derive them from the
patchmap. Two additions, both computable from what `gircap` already has:

1. **A per-capture `repair` block in the patchmap** — what the in-place path
   would do with this file, so the reader does not have to work it out:
   `kept`, `moved`, `fetch` (the chunk counts), `payload_bytes` (the RAM the
   staged tier needs), `direction` (`ascending` / `descending` / `n/a`) and
   `keeps_ascending` / `keeps_descending` (how many sources each order would
   salvage), `trim` (`eager` / `deferred` / `none`), `growth_bytes`
   (`max(0, new.size − old.size)`), and `mode` (`inplace` — what the default
   run takes). Every one of these is derivable from the plan entries the
   patchmap already carries (`source_range` vs `target_range`, and the two
   sides' sizes); §2's table is exactly this block, computed.
2. **The expected `SUMMARY` keys, per capture.** The new spec's counters are
   conditional on flags (`src/report.rs::conditional_extras`): `inplace` /
   `temp` / `kept_bytes` appear only when `--repair-mode` was passed,
   `ram_bytes` only with `--max-ram-bytes`, `damaged_chunks` only with
   `--scan`, `write_bytes` only with `--max-write-bytes`. A patchmap that
   carries the expected values lets the bridge assert them instead of
   recomputing them. They are exact, e.g. for `HoYoChannel.dll`:
   `damaged_chunks=2` (the two chunks that are not already correct where they
   belong), `kept_bytes=1 626 019` (the kept chunk's size), `inplace=1`,
   `temp=0`; for `09801749.blk`: `damaged_chunks=6`,
   `kept_bytes=4 453 236` (the four same-offset chunks), `ram_bytes=5 892 269`
   with the staged tier — and `0` with `--max-ram-bytes 0`, which selects the
   ordered tier and stages nothing.

   The **exact** download figure is available too, and it is a free upgrade
   for the bridge: with `--scan`, `est_download_bytes` is not an estimate but
   the precise sum over the chunks that are wrong where they belong *and* have
   no intact source (`repair::scan::ScanReport::est_download_bytes`). For the
   bundle's fixture that is exactly the patchmap's `totals.download_bytes`, so
   the pre-pass can assert the equality instead of an approximation.

A selfcheck assertion should then pin the **coverage**, not just the classes:
that the bundle contains both trim outcomes, both write directions, and a file
that grows — the three properties that today are only held by accident.

---

## 6. What has to change in `gircap`

| where | what |
|---|---|
| `src/select/classes.rs` | the `Class` enum: the ids, the `exercises()` descriptions, and `matches`. `Shape` grows the axes the new predicates need: `moved_up`, `moved_down` (or a direction), `old_size`/`new_size` (already in `PathScore`), and the trim flag |
| `src/manifest/diff.rs` | `PathScore` gains the direction counts and the old file's maximum chunk extent; `score()` computes them. The source offset rule must stay "the **first** old chunk with that identity" — that is what `girpr`'s `repair::reuse::sources_for` does, and the two must not disagree |
| `src/select/mod.rs` | `Class::REQUIRED_ORDER` — the order is a decision (see §5.1) |
| `src/assemble/mod.rs` | the patchmap's new `repair` block and the expected `SUMMARY` keys; the `Totals` gain the direction split |
| `src/emit/index.rs`, `src/emit/closeout.rs` | the matrix rendering and the selfcheck's wording |
| `src/emit/selfcheck.rs` | A17 (every class captured \| covered_by \| unavailable) is unchanged; **A21 (new)**: the bundle covers both trim outcomes, both write directions, and a file that grows |
| `src/verify/mod.rs` | the same assertion list for `verify` |
| `src/cli.rs` | the `--require-class` error message, which lists the valid set from `Class::ALL` |
| `tests/matrix.rs` | the class tests; `select/classes.rs`'s own table test hard-codes the number of classes (9) and the last one (`full_reuse_reordered`) |
| `docs/00-spec.md` | §4.1's table: the predicates, the "branch it exercises" column, the measured populations, and §8.1's per-capture figures |
| `docs/01-artifact-schema.md` | the patchmap's new fields and the matrix's class list |
| `README.md` | the bundle's size and the reference bundle's numbers |

`capture_id` is `<class>` or `<class>+<n>` and it is the patchmap's file name,
so renaming a class renames a file in every bundle. That is the "backwards
compatibility is not required" part: the old bundles stop being readable by
the new `gircap`, and the new bundle is a re-capture, not a migration.

---

## 7. What has to change in the bridge (`girpr` side)

`tests/artifact_bridge.rs` and `girpr`'s `docs/06-artifact-bridge-spec.md`.

1. **The per-class arms.** `drive_capture` matches on the class id
   (`"identical"`, `"added"`, `"removed"`, `"partial_*"`, `"zero_reuse_*"`).
   The new ids need new arms. Note the *strict* write-bytes inequality — "the
   in-place run writes strictly fewer bytes than the temp run" — is currently
   keyed on `c.starts_with("partial")`; under the new names it is
   `kept_in_place` and the `moved_*` classes, and it does **not** hold for
   `zero_reuse_*` (nothing is kept) or for `added` (the file is created).
2. **The ordered tier has to be run** (G6). The direction rule is invisible in
   the staged tier: staging keeps every source, so both directions download
   the same bytes. `--max-ram-bytes 0` forces the ordered tier, and then
   `download_bytes == the patchmap total` is an **equality** for the captures
   whose shift is uniform — on `09801749.blk` the ascending order keeps all 4
   movable chunks and the descending order none, and on `resources.assets` the
   descending order keeps all 27 and the ascending order none. A wrong
   direction therefore turns the equality into a larger number, and that
   single assertion is what pins D4. (`docs/06` §7.1 already says the figure
   becomes an *upper bound* under `--max-ram-bytes 0`; for a uniform shift it
   does not, and the document should say which is which.)
3. **The trim has almost nothing to assert.** The file ends at the target size
   either way, and the difference between the two trims is the *peak* disk
   usage — which the output contract does not report. The only observable is
   the log: the eager trim logs `trimming the dead tail` (a `debug` event, so
   it needs the trace log file the bridge already writes), and the deferred
   trim logs **nothing at all** today — `docs/02`'s step 6 says "the reason is
   logged", and the implementation only logs the eager case
   (`repair/file.rs`). So the trim classes force the branch, and the
   assertion for them is currently a log line at best. If the deferred case
   is to be assertable, `girpr` has to log it; that is a one-line change on
   the `girpr` side and it is worth making before the classes are captured.
4. **The kept chunks.** Assert `kept_bytes` (with `--repair-mode` passed, so
   the key is emitted) against the patchmap's expectation, and `damaged_chunks`
   (with `--scan`) against the number of chunks that are not already correct.
   This is G4 and G5.
5. **The mode axis** (G8): a third differential arm for `--repair-mode discard`
   (the same file, a full download) and one for "the target cannot be opened
   for writing" (a read-only file in a scratch copy → the run must fall back
   to the temp path and still produce the same bytes).
6. **`docs/06` §7.2's per-class table** and §3.1's "eighteen cases" count.

---

## 8. The flag axes and the fault injections — not cases

These are decisions of `girpr` that a *class* cannot force, because they come
from a flag, a filesystem attribute, or an interruption. They belong to the
bridge, and each one should have a case there:

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

## 9. The limits — what no class can cover

**G7, the corruption-repair input state.** The new spec opens with two
regimes, and the first one is the common case:

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

Two honest options:

- **State it.** One sentence in `docs/00-spec.md` §4.1 and in the bundle's own
  README: *the fixture is always the old version's bytes, so the input state
  of a corruption repair is not represented; the behaviour it produces is
  `kept_in_place`.* Cheap, and it stops a reader from believing the bundle
  proves something it does not.
- **Add a second fixture flavour.** A "damaged current install" capture: the
  same version pair, the `game/` file materialized at the **new** version with
  a few chunks damaged, `config.ini` at the new tag, and the expected result
  the undamaged new file. This is a real feature, not a flag: the class model
  would need a second dimension (the damage) and the manifest diff would have
  to be scored against the damage rather than against the old version. It
  roughly doubles the model. It is the only way to cover the spec's *first*
  regime at the input level.

The first option is the recommendation; the second is listed so the cost is
not a surprise later.

**The malformed-manifest cases** (a hole in the coverage, an overlap) cannot
come from a real capture: the server's manifests tile their files. They are
only reachable with a synthetic manifest, which is a bridge fixture, not a
bundle case.

---

## 10. Three side findings

1. **`docs/00-spec.md` §4.1's measured populations are stale.** The row reads
   "`identical` 1 592, `partial_same_offset` 29, `partial_moved_offset` 430,
   `partial_size_changed` 447, `zero_reuse_single` 120, `zero_reuse_multi`
   491, `added` 82, `removed` 11" — the population over the `game` field's
   2 662 common paths. Re-measuring with the code's own predicates over the
   same field gives **`partial_same_offset` 12** and **`partial_size_changed`
   17**; 29 and 447 are the populations of the *pre-split* classes (`moved ==
   0` and "size differs", regardless of each other). Every other number in the
   row is right. The bundle's own matrix agrees with the code (12 and 17 —
   the audio field adds no candidate to either class), so the document is the
   odd one out. The *candidates* it quotes are right; only the counts are one
   refactor old.
2. **The bundle's `patchmaps/*.json` already carry everything the new axes
   need.** `source_range.start` vs `target_range.start` is the direction, the
   two sides' sizes are the trim and the growth, and a REUSE whose source is
   its own target is a chunk that is already correct. The new class set is a
   matter of **scoring**, not of new capture machinery — which is why §5 is
   cheap to implement even though it adds two captures.
3. **The deferred trim has no observable at all** (on the `girpr` side).
   `docs/02`'s step 6 says "the trim is deferred to the end (the file grows
   during the repair instead) and **the reason is logged**", and
   `repair/file.rs` logs only the eager case (`trimming the dead tail`). So
   the branch exists, the spec says it is observable, and the implementation
   does not observe it. A capture that forces the branch is worth little until
   the log line is there.

---

## 11. The measurements, and how to reproduce them

One script, reading only the bundle — [`../tools/case_coverage.py`](../tools/case_coverage.py):

```powershell
python tools/capture/tools/case_coverage.py artifacts\hk4e_global\7.0.0__7.1.0\20261017T123335Z
```

Three sections, one per question above:

- **`--section population`** — re-implements `gircap`'s scoring
  (`manifest/diff.rs::score`: an identity is `(uncompressed_md5,
  uncompressed_size)`; a chunk is *reused* when the old file carries that
  identity, *moved* when the old file's offset for it differs) and counts the
  population of every class, old set and proposed set, over the pool. Its
  "current" column reproduces the bundle's own `diff/matrix.json`
  `candidates_considered` counts exactly (1 733 / 121 / 491 / 12 / 463 / 17 /
  84 / 11 / 0), which is what makes the "proposed" column worth reading.
- **`--section outcome`** — reads each `patchmaps/*.json`, hashes the `game/`
  fixture's bytes at each entry's target and source range, and derives the
  kept / moved / fetched split, the payload, both directions' salvage counts
  and the trim. This is §2's table.
- **`--section ranking`** — ranks the candidates by `gircap`'s own key
  (`total_cost` = the compressed bytes of the old and the new file) and prints
  the cheapest candidate and the overlap between the proposed classes. This is
  §5.1's table.

The two rules that must be reproduced exactly, because `girpr` implements them
that way:

- the reuse **source** is the *first* old chunk with that identity
  (`repair::reuse::sources_for`), which is what `assemble/mod.rs` already
  records in `source_range`;
- the direction rule is `r ≥ o` for ascending and `r ≤ o` for descending
  (`repair::reuse::survives`), with a source that reaches past the end of the
  file counting for neither.

---

## 12. The recommended order of work

1. **Change the class set** in `gircap` (§5), re-describing the ids and the
   `exercises()` text. The bundle is stale from that moment, which is fine —
   the bundles are regenerable and the task says backwards compatibility is
   not required.
2. **Add the patchmap's `repair` block and the expected `SUMMARY` keys** (§5.2).
   This is what turns the new cases into assertions rather than descriptions.
3. **Add the selfcheck coverage assertions** for the trim outcomes, the
   directions and the growth.
4. **Re-capture** with a `--max-download-bytes` ceiling that funds
   `moved_descending` and leaves `moved_mixed` unfunded (or funds it, if the
   bundle's size is acceptable).
5. **Update the bridge**: the per-class arms, the ordered-tier run, the trim
   and kept-bytes assertions, the `discard` and read-only-target arms (§7).
6. **Update the documents**: `gircap`'s §4.1 (including the stale measured
   populations), the schema, the READMEs, and `girpr`'s `docs/06` §7.2.
7. Optionally, the second fixture flavour for G7 (§9) — as its own change,
   with its own cost.
