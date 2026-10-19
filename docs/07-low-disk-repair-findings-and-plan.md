# Low-disk repair — findings, options, verdict, and implementation plan

Series note: this is a **third** docs series, independent of the `repair-`
prefix series (`docs/01`–`docs/03`, which describe the shipped whole-install
patcher) and of `docs/04` (scope limitations) and `docs/06` (the artifact
bridge). It describes the algorithm in `docs/02` step 6, so where the two
disagree this file is the spec. Every claim about behaviour is anchored to a
file and line so it can be re-checked when the code moves.

**Status: implemented.** The in-place repair, `--repair-mode`, `--max-ram-bytes`
and `--scan` are in the tree; `docs/02` step 6 and `docs/03` §6 have been
rewritten to match, and this file is now the spec for the repair algorithm.
The commit-separated plan in §9 is kept as the record of how it was built —
one commit per behaviour, each with its own tests — and every commit in it
landed as described, with two deliberate deviations: the **mode resolution**
arrived with the writer rather than after it (the writer needs to know which
path it is on), and the **ordered tier** fell back to the temp path rather than
being deferred (it shipped in the next commit, as planned).

## 0. The question

> The current approach is "write `{path}_tmp`, then `rename` over the target".
> Can `girpr` recover more space **before** and **during** a repair, and what
> is the right way to write a file?
> Primary goal: **disk**. Network is secondary. Other wins are welcome.

Everything below is about *transient* disk (the extra bytes the install needs
while the tool runs). Permanent shrinkage is `--purge-*`'s job and is not
re-litigated here except where it interacts.

## 1. What the disk was spent on

### 1.1 The mechanism

`repair_one_file` (`src/repair/file.rs:180`) opens `{path}_tmp`
(`src/repair/file.rs:225`), and `repair_attempt_inner`
(`src/repair/file.rs:291`) walks the manifest's chunks in order. For each chunk
`try_reuse_slice` (`src/repair/file.rs:349`) first asks:

```rust
let cur_len = out.metadata().map(|m| m.len()).unwrap_or(0);
if cur_len >= end { return Ok(ChunkOutcome::Resumed); }   // file.rs:363
```

`cur_len` is the length of the **temp** file. On a first run the temp does not
exist, so `cur_len == 0`, so **no chunk is ever "resumed"** — every chunk is
either copied out of the local file or downloaded, and every chunk is written
into the temp. `promote_tmp` (`src/repair/file.rs:511`) then verifies the whole
temp and renames it over the target.

### 1.2 The finding

> **A repair rewrites every byte of every file it touches, even when only a few
> bytes of it are wrong.**

Worked example, from `tests/repair_offline.rs:293`
(`verified_local_slice_is_reused_without_redownload`): a 128-byte file whose
chunk 0 is intact and whose chunk 1 is corrupt downloads **one** chunk
(`download_bytes = 64`) and writes **128** bytes to disk. Scale that to a
2 GiB `.blk` with three bad chunks and the run fetches ~2 MiB and writes
~2 GiB.

Three consequences:

| consequence | the temp-then-promote path | evidence |
|---|---|---|
| transient free space needed | `size` of each file being repaired, `× io_threads` in flight | `README.md:200` ("free bytes ≈ `io_threads × largest file`") |
| disk **writes** per run | `Σ size(file)` over the repaired set | `src/repair/file.rs:363` |
| disk **reads** per run | one full read for the skip check, one full read of the temp for the final MD5 | `file.rs:200`, `file.rs:521` |

The write volume is the one that hurts most on a large version upgrade: an
install where 60 GiB of files change from 2 % to 60 % writes **60 GiB** to
produce ~1.2 GiB of actual change, and needs headroom for the largest in-flight
file even though it is only keeping one byte of it. The first row of the table
is what `README.md:200` states as the tool's requirement; it follows from the
code rather than from a choice.

### 1.3 Why the temp-then-promote contract exists

The temp-then-promote contract buys three things:

1. **The original is never damaged.** A crash, a full disk or a bad chunk can
   never leave a half-written target.
2. **Promotion is atomic.** `rename` is a metadata operation.
3. **Resume-by-length (S3).** A leftover `*_tmp` is a completed prefix, so the
   next run continues rather than restarts.

Those three are what `--repair-mode temp` offers, and they are why the mode
stays: a target that must not be touched has a way to be repaired that does not
touch it. What the contract costs is the whole file, twice — and §1.2 is what
that cost is.

Starward does exactly the same thing (`GameInstallHelper.cs:330-331` opens
`FullPath + "_tmp"` with `FileMode.OpenOrCreate`; `:445` `File.Move(tmp, final,
true)`; `:450` `File.Delete(tmp)` on mismatch) and Collapse does it for its
Sophon path (`InstallManagerBase.Sophon.cs:1002-1011`, `_tempSophon` → `MoveTo`
at `:458`) while writing *in place* for its plain HTTP repair
(`DownloadClient.cs:234-241` opens the final path directly, and
`ChunkSession.cs:71-100` deletes before writing when `overwrite` is on — the
default, `ProgressBase.cs:1309`). **So "write the broken file where it lies"
is a shipped, shipping-launcher behaviour for exactly this scenario.** It is
simply not what `girpr` copied from Starward.

### 1.4 What each mode buys, and what it costs

The temp mode's guarantee is real and worth having: the repair is assembled
beside the target and swapped in with an atomic rename, so **the target file is
never modified**, and a repair that fails leaves it exactly as it was. For
some installs that is the property you want, and it is one flag away.

What it costs is the whole file, twice, for as long as the repair takes. And
the file it is protecting is one that has **already failed its own whole-file
MD5** — that is the definition of a file being repaired. Its previous bytes are:

- not part of the install the manifest describes,
- not usable by the game,
- worth at most a *network* saving to the next run, and only through the reuse
  map, which `try_reuse_slice` hash-checks anyway.

So the choice is not "safety against risk" — it is **a property against disk**.
Most repairs want the disk, and the property stays available for the ones that
want it instead.

## 2. The disk model, per mode

Let `S` = target size, `O` = current on-disk size (0 if absent), `G` = growth.

| mode | bytes written per repaired file | peak extra free space | reuse sources readable | if the run is interrupted |
|---|---|---|---|---|
| `temp` | `S` — the whole file | `S`, so `jobs × S_max` across a run | the old file, entirely | the old file untouched + a partial `*_tmp` |
| `inplace` (default) | only the chunks that are wrong *or moved* | `max(0, S − min(O, S))` | the old file, subject to a write order (§4.4) | a mixed file, re-verified next run |
| `discard` | `S` | `0` | none | a partial file |

The two rows are not a safety ladder: they are two ways to spend disk, and
each has a property of its own. `inplace` spends the least and writes the
least; `temp` spends the whole file and, in exchange, **the target file is
never modified**. `discard` (delete the file, then download it whole) has `0`
peak but a full network cost, so it is only ever right when nothing on the
file is reusable — which the plan can decide without reading it.

For a corrupt-but-right-sized file — the common repair case — `inplace` needs
**zero** extra space and writes only the corrupt bytes. For a file that grows by
`Δ`, `inplace` needs `Δ`, not `S`. For a fresh install nothing is saved because
nothing is on disk to save.

The `inplace` row is the *bound* that holds for both of its tiers (§4). Tier 1
— the default — is strictly better than the row says: it writes only the wrong
chunks, and it stages the moving bytes in RAM rather than letting the write
order constrain them.

## 3. Options considered

Each is stated as *idea → how → what it buys → what it costs / risks → verdict*.

### A. Keep temp-then-promote, tune around it

*Idea.* No algorithm change: run `--purge-before`, drop `--io-threads` to 1,
raise the write ceiling.

*Buys.* Nothing structural. Headroom `S_max` instead of `jobs × S_max`; slower.
`--purge-before` reclaims permanently-dead files, which is orthogonal (it helps
the *install*, not the *repair*).

*Verdict.* **Rejected** as a solution — it bounds the problem rather than
removing it, and `jobs=1` costs real wall-clock on the network-bound work this
tool is mostly waiting on.

### B. Trim the original in place (eager `set_len`)

*Idea.* Before writing, if `O > S`, `set_len(S)` to drop the tail.

*Buys.* Frees `O − S` immediately; costs nothing but the reuse sources that live
past `S` (rare, and cheaply detected — §4.5).

*Costs.* On its own it saves only the *excess*. A file that is the right size
and 2 bytes wrong still needs `S` of headroom under `temp`.

*Verdict.* **Adopted**, but as a component of `inplace`, not as a standalone.
It is necessary (a 3 GiB leftover with a 50 MiB target must not hold 3 GiB while
we write 50 MiB) and not sufficient.

### C. Delete before write (`discard`)

*Idea.* `remove_file` the target, then write it fresh.

*Buys.* Exactly `0` extra space, always. The strongest guarantee on the space
axis there is.

*Costs.* Full network for that file — it throws away every reusable chunk, and
it throws them away *silently*, since a reuse opportunity discovered too late
is indistinguishable from one that never existed.

*Verdict.* **Adopted as a user-selectable mode only** (`--repair-mode discard`),
plus as the automatic choice when the target does not exist (where it is free).
Making it automatic for files with an empty reuse map is wrong under the new
design, because per-chunk verification (§4.2) can find most of such a file
already correct — deleting it would throw away bytes that cost nothing.

### D. Repair in place, staging the moving bytes — `inplace`

*Idea.* Write the target file where it lies. Scan it once to learn which
chunks are already correct where they belong; write only the wrong ones, each
to its final offset. Where a wrong chunk's bytes live *elsewhere* in the same
file, stage them in RAM first (§4.3) so the write order does not have to
constrain the reuse; where that does not fit, fall back to ordering the
relocations so no source is overwritten before it is read (§4.4).

*Buys.*

- peak extra space `G` per file instead of `S` — **zero for a same-size repair**;
- disk writes drop from `S` to the sum of the chunks that actually change;
- the run also **reads** each file twice instead of three times, because the
  per-chunk verdicts come out of the skip check's read;
- **a network win that needs no local manifest at all**: a corrupt-but-mostly-
  intact file now downloads only its corrupt chunks, where today it downloads
  the whole file whenever `getBuild(local)` failed;
- with the moving bytes staged in RAM (§4.3), the write order stops
  constraining the reuse, so both shift directions survive and the writes run
  forward instead of backwards.

*Costs / risks.* A file that grows needs `Δ` of free space, with no query
to check it against; the staging costs RAM, bounded by a budget; the pre-flight
download estimate becomes an upper bound instead of a tight figure (§6).

*What it does not do.* The temp mode's property — the target file is never
modified — is not carried over, and it is not carried over *by accident*: it
is the property `--repair-mode temp` exists to offer (§2), and the choice
between the two is a choice of what to spend, not of what to risk. A file that
is interrupted mid-repair fails the same check it failed before, and the next
run re-verifies and continues.

*Verdict.* **SELECTED.** §4 is the design.

### E. Cross-file salvage (Starward's `OriginalFileFullPath`)

*Idea.* Reuse a chunk from *any* local file, not just the same path
(`GameInstallFile.cs:122-129`).

*Buys.* More reuse, so fewer downloads. Does nothing for disk.

*Costs.* It is **mutually exclusive with `inplace`** unless file order is made
to satisfy a donor/beneficiary topological order — repairing a donor destroys a
donee's source. `girpr` is same-file-only today (S2, `sophon.rs:291`), so the
conflict is latent, but it must be written down before anyone adds cross-file
reuse.

*Verdict.* **Deferred**, with the constraint recorded (§6, D6).

### F. Hard-link identical files

*Idea.* Starward hard-links launcher-provided content
(`GameInstallHelper.cs:288-297`). If two manifest paths are byte-identical,
repair one and link the other.

*Buys.* Permanent space, not transient.

*Costs.* NTFS-only (no hard links on exFAT, where Genshin installs often live),
and it interacts with the same in-place hazard: rewriting one link rewrites
both.

*Verdict.* **Deferred**, out of scope for a space-during-repair change.

### G. Keep a compressed chunk cache on disk (`chunk/`, Starward/Collapse style)

*Buys.* Resume without re-downloading.

*Costs.* It is strictly *more* disk, retained on purpose. Directly contrary to
the goal.

*Verdict.* **Rejected.**

### H. Streaming zstd decode (replace S1 whole-blob buffering)

*Buys.* Lower peak **RAM**, and partial chunk bytes survive a crash.

*Costs.* Nothing on disk. It does not advance the goal.

*Verdict.* **Deferred** as an independent improvement, not part of this change.

### I. Exact planning via a verification scan — `--scan`

*Idea.* Do the skip-check pass during the plan phase instead of inside the
repair loop, and record a per-chunk verdict while the bytes are already being
read. Then `est_download_bytes` is exact and `--max-download-bytes` means what
it says.

*Buys.* An exact plan; `--check-only` gains damaged-chunk counts; the repair
loop becomes write-only.

*Costs.* The ceiling refusals get slower (a full read pass before them), and
`--fail-fast`'s "SUMMARY in seconds" property moves.

*Verdict.* **Adopted as a separate, opt-in flag** (§7, commit 10). It is the
answer to the estimate regression `inplace` introduces, so it belongs to the
change, but it must not be on by default.

### J. Reclaim-on-demand (delete while repairing) + a free-space query

*Idea.* Before a write that would not fit, delete the next purge candidate
until it does. Collapse has the `EnsureDiskSpaceSufficiencyAsync` shape
(`InstallManagerBase.Sophon.cs:1033-1071`) but only as a pre-flight refusal,
not as a mid-run reclaim.

*Buys.* The run finishes instead of failing with `ENOSPC` on a nearly-full
volume. Needs a free-space source: `windows-sys` 0.61.2 is already in
`Cargo.lock` transitively (via tokio/mio), so `GetDiskFreeSpaceExW` costs one
direct dependency line, not a new tree.

*Costs.* Deleting a user's file without them asking is a product decision, and
it collides with `--max-delete-bytes` being checked "before anything is
deleted".

*Verdict.* **Deferred** (§8). `inplace` removes most of the pressure that makes
it necessary; if it is needed later it should be its own flag and its own doc.

## 4. The selected design

`--repair-mode <auto|inplace|temp|discard>`, default `auto`.

- `auto` → `inplace` for a target that exists as a regular file; falls back to
  `temp` (with a `warn`) if the target cannot be opened read-write.
- `inplace` / `discard` — forced; no fallback except the same I/O-error one,
  which is reported rather than hidden.
- `temp` — the mode whose property is that **the target file is never
  modified**: the repair is assembled in `{path}_tmp` and swapped in with an
  atomic rename. It is the mode to pick when the target must not be touched,
  and the mode the artifact bridge exercises alongside the default.

### 4.0 Two regimes: patching and relocating

The in-place repair is **not** a defragmentation, and the difference decides the
design. There is no staging area anywhere in it — a reuse source and its
destination are both inside the same file, and the copy between them uses the
existing 512 KiB `COPY_BUF` and nothing else.

- **Corruption repair** — the file is the right size and most chunks are
  intact. A chunk's reuse source is at *its own offset* (`r_i == o_i`), so the
  bad chunk is simply overwritten where it lies. **No bytes move.** There is no
  ordering constraint and nothing to stage: this is a patch, not a relocation.
- **Version upgrade** — the file's content shifted, so a chunk's source is
  elsewhere and its bytes have to physically move from `r_i` to `o_i`. *This*
  is the defrag-shaped case.

The second regime is the classic "relocate blocks inside a file without a
staging area" problem, and it has exactly two answers: **order the relocations**
so no source is overwritten before it is read, or **stage the moving bytes in
RAM**. The first is free and forces the writes into one direction (a
backwards-seek pattern, which is why the RAM answer is preferred); the second
costs RAM and buys everything else. §4.2 is the RAM answer and is the default;
§4.4 is the ordering answer and is the fallback for when the RAM does not fit.

Which of the two answers a file gets is decided per file, so `inplace` is not
one algorithm but three tiers:

| tier | when | extra disk | RAM | reuse | writes | direction |
|---|---|---|---|---|---|---|
| **1. RAM-staged** (§4.2–4.3) | the repair payload fits the budget | `0` | the payload | 100 % | only the wrong chunks | ascending |
| **2. ordered** (§4.4) | the payload does not fit | `0` | `0` | one direction | the wrong chunks | ascending *or* descending |
| **3. `temp`** (`--repair-mode temp`) | asked for | `S` | `0` | 100 % | the whole file | ascending |

Tier 1 and tier 2 produce **the same file**; they differ in how much reuse
they keep and in which direction they write. Tier 3 is the other way to write a
file: the target is never modified, and the whole file is paid for in disk. It
is there for the users who want that property, and the artifact bridge drives
it alongside the default.

### 4.1 Preconditions (all checkable, none assumed)

| # | precondition | how it is met |
|---|---|---|
| P1 | chunk offsets are ascending | sorted in `plan::build_plan` (commit 2); today nothing sorts them |
| P2 | chunks tile `[0, size)` exactly | validated at plan time; a gap is zero-filled and logged (commit 2) |
| P3 | the target is openable `read+write` | `open_inplace`; on failure the file falls back to `temp` (commit 5) |
| P4 | no read-only attribute | cleared before an in-place write, Starward-parity (`GameInstallService.cs:232-236`) — `girpr` does not do this today |
| P5 | one writer per `game_dir` | lock file (commit 9) — under `temp` a collision only wastes work; under `inplace` two runs with different plans can write conflicting bytes (D1) |
| P6 | the game is not running | already required; under `inplace` it is fatal rather than wasteful |
| P7 | free space ≥ growth | not checked in v1; a short write fails the file, and `--repair-mode temp` is the mode for a volume with no room for the file's growth |
| P8 | scope still governs everything | unchanged: out-of-scope paths are neither read for reuse nor written |
| P9 | RAM ≥ the repair payload | a run-level budget (`--max-ram-bytes`); a file that cannot get its payload falls to the ordered tier (§4.4), not to a failure |

### 4.2 The algorithm: scan once, patch in place

The scan **replaces** today's skip check, so it is not an extra read — it is
the same read with more answers computed from it.

```
repair_one_file(file):
    whole-file MD5 == expected            -> Skipped            (unchanged)
    dry-run                               -> DryRun             (unchanged)
    mode = resolve(target)                                  (commit 7)

attempt_inplace(file):
    h = open(target, read+write+create)            # P3; failure -> mode = temp
    clear read-only if the open needed it                    # P4

    # ---- pass 1: one read, every answer we need ----
    scan = scan_file(h, dest_ranges = chunks, source_ranges = reuse_candidates)
        scan.file_md5    -> whole-file MD5
        scan.dest_ok[i]  -> chunk i is already correct where it belongs
        scan.source_ok[j]-> candidate source j is intact
    if scan.file_md5 == expected: Skipped
    bad = { i : !scan.dest_ok[i] }

    # ---- stage the moving bytes, if any are needed ----
    payload = sum(size(i) for i in bad if source is elsewhere and source_ok)
    if payload > 0 and reserve_ram(payload):   # the run-level budget
        capture = read_sources(h, bad)          # pass 2, only the needed ranges

    # ---- patch: only the wrong chunks, ascending ----
    for i in bad, ascending offset:
        if i has a captured source: write it at dest
        else: download -> zstd -> verify md5 -> write at dest
    h.set_len(size)                              # §4.5
    if md5(h) == file.md5: Ok
    else: reverify_chunks(h, file) -> redo only the mismatching ones -> re-check

attempt_temp(file):                    # the temp mode's repair_attempt_inner, verbatim
    ... existing _tmp resume / reuse / download / promote ...
```

Six properties worth naming explicitly:

1. **The scan is free.** It replaces the skip check's read (`file.rs:200`) and
   adds per-chunk verdicts to it. The run reads each file **twice** in total
   (scan, then the final verify) where the temp path reads it **three** times
   (skip check, reuse-slice MD5s, final MD5 of the temp).
2. **Only the wrong chunks are written.** For a corruption repair that is a
   handful of chunks, not the whole file.
3. **The writes run ascending** — forward, never backwards. That is the
   opposite of the ordered tier's pattern (§4.4) and it is what makes the
   in-place path acceptable on an HDD.
4. **Every byte written is hash-gated before it lands.** Unchanged from the
   temp path: a downloaded chunk's MD5 is checked before `write_all`
   (`file.rs:471`), a reused slice's before the copy (`file.rs:380`).
5. **Resume is free and needs no artefact.** An interrupted in-place file is
   simply a file with some correct and some incorrect chunks; re-running finds
   the correct ones in the scan and rewrites the rest. There is no
   `*_tmp`, no marker file, no length heuristic — so the temp mode's
   "resume-by-length" becomes "resume-by-verification", which is strictly
   stronger (it cannot inherit a corrupt prefix).
6. **Final-MD5 failure costs one read pass, not a re-download.** The temp path
   deletes the temp and re-fetches the whole file up to five times
   (`file.rs:523-530`). Re-verifying per chunk names the culprit.

### 4.3 The RAM tier — what is staged, and what it is not

**Not the file. The repair payload.** A chunk needs staging only if its reuse
source is *elsewhere* in the file (`r_i != o_i`) **and** the chunk is actually
wrong. The second condition is what makes the payload small in the case that
matters:

| regime | wrong chunks | sources elsewhere? | payload |
|---|---|---|---|
| corruption repair | a few | no — normally `r_i == o_i`, and that source *is* the bad chunk, so it is not a source at all | **0** |
| version upgrade | most | yes | ~`S` |
| mixed re-flow | some | some | the sum over those chunks |

So the common repair case stages **nothing**, and the case that stages the most
is the one where the file is being rewritten wholesale anyway.

The budget is **run-level**, reserved per file the way `--max-write-bytes`
already reserves (`Summary::reserve_write_bytes`), because `io_threads` files
are in flight at once. A file that cannot get its reservation does not fail: it
falls to the ordered tier (§4.4), or to `temp` if the user asked for that.

A RAM tier is a *quality* choice, not a correctness one: it buys 100 % reuse
(both shift directions) and forward writes, and costs `payload` bytes of RAM.
It never changes the result — the ordered tier produces the same file, with
less reuse and a different write order.

### 4.4 The ordered tier — the salvage order

Used when the payload does not fit the RAM budget. This is the defrag-shaped
regime: chunk bytes physically move from `r_i` to `o_i` inside the file, and
the ordering is what keeps a source from being overwritten before it is read.

Write chunk `i`'s destination `W_i = [o_i, o_i+s_i)` from its reuse source
`R_i = [r_i, r_i+s_i)` in the same file. The `W`s are disjoint and, after P2's
zero-fill, tile `[0, size)`. So:

- **ascending order** (smallest `o` first): before chunk `i` the written region
  is `[0, o_i)`. `R_i` survives ⟺ `r_i ≥ o_i` (and `r_i+s_i ≤ O`).
- **descending order**: before chunk `i` the written region is
  `(o_i+s_i, size)`. `R_i` survives ⟺ `r_i ≤ o_i` (and `r_i+s_i ≤ O`).

Neither condition mentions any other chunk, so both are **exactly computable
at plan time** — no simulation, no heuristic. A pure function picks whichever
direction preserves more reuse and marks the surviving chunks; the rest
download. `r_i == o_i` counts for both. If the layout has holes, P2 zero-fills
them *after* the ordered loop, so during the loop the written region is a
subset of the one the rule assumes and both conditions stay conservative —
never wrong, occasionally pessimistic.

A uniform upward shift of a file's content (everything moved later) is the
descending case; a downward shift is the ascending case. A file whose chunking
re-flowed has sources in both directions and keeps the larger half. A full
interval-graph optimum (topological order over "chunk `j`'s write clobbers
chunk `i`'s source", breaking cycles by downloading) is possible and is
**deferred** (§8): the order is produced by one pure function
(`reuse::salvage_order`) precisely so it can be swapped without touching the
writer.

Illustration — content shifted one chunk to the right:

```
on disk (old):      [0,4)=AAAA  [4,8)=BBBB  [8,12)=CCCC  [12,16)=DDDD
target  (manifest): [0,4)=0000  [4,8)=AAAA  [8,12)=BBBB  [16-)=CCCC
reuse sources:        none        r=0        r=4          r=8

descending: CCCC@12 <- @8   ✓  r=8  <= o=12
            BBBB@8  <- @4   ✓  r=4  <= o=8
            AAAA@4  <- @0   ✓  r=0  <= o=4
            0000@0  <- --      no source -> download          3/4 salvaged
ascending:  AAAA@4  <- @0   x  r=0  <  o=4  (already overwritten by 0000)
            BBBB@8  <- @4   x
            CCCC@12 <- @8   x                                        0/4
```

### 4.5 Eager trim is nearly always free

`trim_is_free(file)` = "no reuse candidate has `r + s > size`". That is the
whole test: an entry in the local reuse map past the target's end can only ever
be a source, and only if the file shrinks a lot. When it is true the tail is
dead weight and `set_len` releases it before a single byte is written. When it
is false and `O − S` is large, the trim is deferred to the end (the file grows
during the repair instead) and the reason is logged.

## 5. The invariants, and each mode's own

**The tool's, unchanged.**

- The plan derives from the latest manifest alone; the local manifest is an
  optimisation, never an input to correctness.
- Every byte written is hash-gated before it lands, and the whole-file MD5 is
  the gate that decides a file is good.
- A rerun is idempotent and repairs only what is still wrong.
- Ceilings refuse before any mutation; a runtime hard stop is visible in
  `SUMMARY stopped=`.
- `config.ini` advances only when every planned file verified.
- `--dry-run`, `--check-only`, `--purge-only` and the path scope write exactly
  what they wrote before.

**Each mode's own.** These are the properties the two ways of writing a file
have, side by side. They are not a ladder: the left column is what the mode
gives you, the right column is what it costs.

| | `inplace` (default) | `temp` (`--repair-mode temp`) |
|---|---|---|
| the target file | is written where it lies: the wrong bytes are replaced in place | is **never modified**: the repair is assembled in `{path}_tmp` and swapped in with an atomic rename |
| an interrupted run | leaves a file that fails the same check; the next run re-verifies and continues | leaves the target exactly as it was, plus a partial `*_tmp` the next run resumes by length |
| a final-MD5 mismatch | redoes only the chunks the per-chunk verdicts name | deletes the temp and restarts the file |
| the resume artefact | none: the scan finds the chunks that are already right, so a corrupt prefix cannot be inherited | the `*_tmp` prefix, kept and continued |
| peak extra disk | `io_threads × largest *growth*` — 0 for a same-size repair | `io_threads × largest file` |
| `stopped=bytes`/`stopped=write` | the next run re-verifies and continues | the next run resumes the `*_tmp` prefix |

**New.**

- N1 a single-writer lock (commit 9).
- N2 read-only attributes cleared before an in-place write.
- N3 gaps in a chunk layout are zero-filled rather than left as stale bytes.
- N4 a file whose target cannot be opened `read+write` is written the temp
  way, loudly.
- N5 a run-level RAM budget (`--max-ram-bytes`) that a file reserves against and
  that falls back to the ordered tier rather than failing.

## 6. Risks

| # | risk | severity | handling |
|---|---|---|---|
| D1 | two concurrent `girpr` runs | **medium** | lock file, exit 1. Two in-place runs pursuing the *same* manifest write the same bytes and converge, but runs with different `--audio`/`--include` plans write conflicting bytes into one file, and `discard` deletes a file the other run has open (silently losing its work on Unix). Under `temp` a collision only wastes work: both runs share one `*_tmp`, its MD5 fails, nothing is promoted |
| D2 | read-only attribute on a game file | medium | clear it (P4); today the tool would fail the same way at rename |
| D3 | power loss mid-file | medium | whole-file MD5 catches it on the next run |
| D4 | the file is grown and the volume is full | medium | file fails, counted, retried, reported; `--repair-mode temp` is the mode for a volume with no room for the file's growth |
| D5 | `est_download_bytes` becomes an **upper** bound, so `--max-download-bytes` over-refuses | medium | `--scan` (commit 10) makes it exact; documented until then |
| D6 | cross-file reuse, if ever added, breaks in-place unless file order is topologically sorted | latent | S2 stays same-file-only; recorded here |
| D7 | the artifact bridge asserts `download_bytes == patchmap total` | **low** | it **still holds** under the RAM tier, and that is worth pinning rather than relaxing. The patchmap's `FETCH` set is "chunks this old file cannot supply" (`tools/capture/src/assemble/mod.rs:745-749`), and the RAM tier downloads exactly that set — a chunk whose content is absent from the old file cannot be "already correct" at its destination, so it is always fetched. The *ordered* tier can **exceed** the total (a shifted chunk it could not salvage is downloaded instead of copied), never fall below it. The bridge therefore gains a mode-vs-mode differential rather than a relaxed bound (commit 11) |
| D8 | hard-linked game files are unlinked by an in-place write | low | no launcher creates them here; noted |
| D9 | `--max-write-bytes` documentation says "`*_tmp`" | low | reworded to "into the install"; the arithmetic is unchanged |
| D10 | a manifest whose chunks do not tile the file | low | validated at plan time, zero-filled, logged (P2) |
| D11 | the staged payload is larger than expected and the process is memory-pressured | low | the budget is a hard reservation, so the run never exceeds it; a file that cannot get a reservation degrades to the ordered tier. The payload is bounded by the sum of the *wrong* chunks' source sizes, and is 0 for the common corruption repair |

## 7. What changes in the machine contract

Only additive, and only under a flag or a mode, so the discipline in
`README.md:104-107` holds:

- `SUMMARY`: `repair_mode=auto|inplace|temp|discard`, `inplace=N`, `temp=N`,
  `kept_bytes=N` — emitted **only** when `--repair-mode` was passed. Always
  logged either way.
- `SUMMARY`: `write_bytes` (existing) keeps its meaning as "bytes written into
  the install"; only its prose changes.
- `SUMMARY`: `ram_bytes` — the peak staged payload, emitted only when
  `--max-ram-bytes` is passed, like every other requested ceiling.
- `PLAN` (`--print-plan` only): `est_write_bytes=`, `est_extra_bytes=`, and
  under `--scan` `scan_bytes=`.
- `--plan-out`: per-file `mode` and `extra_bytes`.
- No new exit code. The lock-file refusal is `1` (usage/config), like every
  other "the request was not well-formed" failure.

## 8. Deferred, with the reason

| item | why not now |
|---|---|
| interval-graph optimum for the chunk order (§4.4) | the RAM tier (§4.3) removes the ordering constraint entirely wherever the payload fits, and the two-direction rule already captures a uniform shift — the dominant real shape — for the rest. It is one pure function to swap |
| a speculative single-pass capture (capture every candidate source range during the scan, instead of a second pass over the bad ones only) | saves one read pass when the plan-time estimate fits the budget; costs RAM for sources that turn out not to be needed. The exact two-pass scheme is simpler to reason about and the read is mostly served by the page cache |
| cross-file salvage (§E) | mutually exclusive with `inplace` absent a file-level order |
| reclaim-on-demand + `GetDiskFreeSpaceExW` (§J) | `inplace` removes most of the pressure; it collides with `--max-delete-bytes`' "before anything is deleted" promise and deserves its own flag and doc |
| streaming zstd decode (§H) | RAM, not disk |
| hard-link dedup (§F) | permanent space, exFAT-hostile, interacts with in-place |
| a compressed chunk cache (§G) | strictly more disk |

## 9. Implementation plan

Eleven commits, each one behaviour, each carrying its own tests, each leaving
the tree green. The series is ordered so that the risky part (the writer) lands
after the parts it depends on and before the parts that describe it, and so
that every commit is useful on its own — the ordered tier arrives after the
RAM tier, and the RAM tier is useful before it.

### 9.0 The working agreement: one commit, then turn around

This plan is executed **one commit at a time, with a turn-around after each
one**. The implementing agent must not begin commit *N+1* before the reviewer
has seen commit *N*. A plan of eleven commits delivered as one branch is not
this plan.

Each turn-around is a **review packet**, and it carries all five of these:

1. the commit message, in full;
2. `git show --stat` and the complete diff;
3. the exact commands that were run and their results — `cargo test`,
   `cargo clippy --all-targets`, and the bundle-backed suite of §9.1;
4. **what the commit does not verify**, stated plainly rather than left to be
   discovered;
5. what the reviewer should look at hardest, and why that is where the risk is.

The rules that make the turn-around worth something:

- **The tree is green before the commit is offered.** `cargo test` and
  `cargo clippy --all-targets` pass, and §9.1's suite passes. A turn-around
  that reports a red test is a turn-around asking the reviewer to debug, which
  is not what a review is for.
- **One commit per behaviour.** If a commit needs two behaviours, it is two
  commits and two turn-arounds.
- **History is not rewritten to satisfy a review.** A defect the review finds
  in an earlier commit is fixed by a new follow-up commit, unless the reviewer
  explicitly asks for the history to be rebased. A reviewer who cannot see the
  defect in the diff is reviewing nothing.
- **A commit that cannot be fully verified is offered anyway, with that said.**
  "The bundle suite could not run because the capture needs a tag the server
  no longer serves" is a legitimate thing to report; quietly not running it is
  not.
- **The reviewer may send the plan back.** Reordering, splitting a commit, or
  dropping one is a normal outcome, not a failure of the plan.

### 9.1 The bundle-backed tests are mandatory at every turn-around

`tests/artifact_bridge.rs` is `#[ignore]`d, and the file says why: **cost**,
not correctness ("~18 cases and ~2 minutes", `tests/artifact_bridge.rs:13-20`).
That reasoning was sound when the file only had to prove the *existing*
pipeline still worked. It stops being sound now: this change rewrites the
repair writer **and** the data plane, so the ignored suite is the only thing
that runs the real binary against real game bytes, real API bodies and real
chunk blobs, and the only thing that pins the `SUMMARY`/`PLAN` contract
against a body nobody hand-wrote.

So it is **required at every turn-around**, not only at the end:

```text
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

**A missing bundle is a failure, not a skip.** That is the bridge's own rule
(`tests/artifact_bridge.rs:33-50`) and it is kept: a skip on a missing
prerequisite would turn "I ran the check and it found nothing to check" into a
green line.

Three bundles are already present, and they are the ones the plan is written
against:

```text
artifacts/hk4e_global/7.0.0__7.1.0/20261016T002107Z
artifacts/hk4e_global/7.0.0__7.1.0/20261017T000730Z
artifacts/hk4e_global/7.0.0__7.1.0/20261017T123335Z
```

**Capturing a new bundle is allowed and expected.** A bundle captured before
the change is evidence about the *old* code; if a case is missing, stale, or
the change needs a shape the existing bundles do not have, capture a new one:

```text
cargo run -p gircap -- capture --out artifacts
# narrower, when a specific shape is wanted:
cargo run -p gircap -- capture --out artifacts --biz hk4e_global --files <REL_PATH>
```

That reaches the live HoYoPlay/Sophon endpoints, so it needs network. The
bundle lands in `artifacts/<biz>/<old>__<new>/<timestamp>/` and is
self-describing (`MANIFEST.json`), so a new capture is **additive** — the
existing three stay, and a reviewer can compare a run against the bundle it
was captured from.

The bundle is **read-only evidence**. The bridge works on a scratch copy
(`tests/artifact_bridge.rs:52-57`), and the bundle must still pass its own
integrity check afterwards:

```text
cargo run -p gircap -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261017T123335Z
```

**What the bridge is protecting.** Two different things, and they need
different guards:

- *The bytes.* That the patched file still hashes to `expectations.result_md5`
  and the run still exits `0` — the bridge's existing assertions, and the
  reason a rewrite of the writer cannot be waved through.
- *The contract.* That `SUMMARY` and `PLAN` still say what they say. The
  discipline that an **unflagged** run's line stays byte-identical is pinned by
  the offline unit tests in `src/report.rs` (every conditional key is emitted
  only when its flag is active, `report.rs:398-431`); the bridge is where an
  accidentally *unconditional* new key would show up as a changed line, and
  where a changed `download_bytes`/`est_download_bytes` meaning would show up
  as a wrong number against a body nobody hand-wrote.

### 9.2 The commits

#### 1. `util`: the scan primitive — one read, every answer

`src/util.rs`: add

```rust
pub struct Scan {
    pub file_md5: String,
    /// per destination range: is the chunk already correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// per candidate source range: is the reuse source intact?
    pub source_ok: Vec<bool>,
}
fn scan_file(
    path: &Path,
    dest: &[(u64, u64, &str)],
    source: &[(u64, u64, &str)],
) -> Result<Scan>
```

One read pass in 512 KiB blocks, feeding a running whole-file `md5::Context`
plus one context per distinct range (ranges sorted and deduped once, indices
mapped back). This is the primitive every other piece needs: the skip check
becomes `scan.file_md5`, the "is this chunk already right" test becomes
`scan.dest_ok[i]`, and the "is this source intact" test becomes
`scan.source_ok[j]`. A missing or short file returns
`("d41d8cd98f00b204e9800998ecf8427e", vec![false; n])` rather than an error —
"nothing is correct" is the answer, not a failure. Unit tests: intact file, one
corrupt chunk, truncated file, absent file, unsorted ranges, a range that is
also a destination.

#### 2. `plan`: a trustworthy chunk layout

`src/repair/plan/mod.rs`: sort `f.chunks` by `offset` in `build_plan`; drop
zero-length chunks defensively (logged); add `PlannedFile::layout()` returning
`(tiled, gaps)`. Nothing else in the crate can currently assume ascending
order, and both the "already correct" test and the ordered tier's proof do.
Unit tests: unsorted manifest input is sorted; a hole is reported; an
overlapping pair is reported.

#### 3. `reuse`: the salvage order (pure, no I/O)

New `src/repair/reuse.rs`: `salvage_order(chunks, candidates, usable_len) ->
Order { ascending: bool, live: Vec<bool> }`, implementing §4.4 exactly. The
module doc carries the two-line proof. Unit tests pin every case the proof
makes: `r > o` counts ascending, `r < o` counts descending, `r == o` counts
both, a source past `usable_len` counts for neither, and an empty candidate
list yields an arbitrary-but-valid direction.

#### 4. the flags: `--repair-mode` and `--max-ram-bytes`

`src/cli.rs` (`#[arg(long, value_enum, default_value_t)]`), `src/config.rs`
(`RepairMode` + `RunCtx::repair_mode` + `RunCtx::ram_budget`),
`src/repair/scope/limits.rs` (`Limits::ram_bytes`), and
`src/repair/scope/ledger.rs` (`Summary::reserve_ram_bytes`, mirroring
`reserve_write_bytes`: a compare-and-swap against a `u64::MAX` sentinel, no
`Option` check on the hot path). Unit tests: default is `auto`; an unknown
value is exit 1 through the existing clap remap; the reservation is
all-or-nothing and the boundary is inclusive.

#### 5. `inplace`: the writer — the RAM tier

`src/repair/file.rs`. Split `repair_attempt_inner` into `attempt_temp` (the
current body, unchanged, still promoting `_tmp`) and `attempt_inplace` (the
§4.2 algorithm): `open_inplace` with the read-only clear and the `temp`
fallback, the scan, the payload computation and its RAM reservation, the
capture pass, the ascending patch loop (captured source → download), the
eager trim, the final `set_len`, and `reverify_and_redo` replacing "delete the
temp and restart" on a final-MD5 mismatch. Both attempts funnel through the
existing `repair_attempt`, so the give-back-the-bytes rule (`file.rs:283`)
still holds for both.

Until commit 6 lands, a file that cannot get its RAM reservation falls back to
`temp` — which is the safe behaviour and is what the artifact bridge expects.

Integration tests (`tests/repair_offline.rs`): a right-sized file with one
corrupt chunk is repaired **without** writing a `_tmp` at all, and the file's
bytes outside that chunk are not rewritten; a file that grows writes only the
growth; an interrupted in-place run resumes by verification on the next run,
with no `_tmp` anywhere.

#### 6. `inplace`: the ordered tier

The same writer, with the RAM tier bypassed: `salvage_order` picks the
direction, the sources are read from the file at write time (which is safe
because the order guarantees they are still intact), and the writes run in
that direction. The tier is chosen when the RAM reservation fails, or when
`--repair-mode inplace` is combined with `--max-ram-bytes 0`. Integration
tests: a file whose content shifted one chunk to the right salvages three of
four chunks in the descending direction and downloads the fourth; the same
file with the RAM tier enabled salvages all four and writes them ascending.

#### 7. mode resolution + the counters

`src/repair/file.rs`: `resolve_mode()` (missing target → create-in-place;
existing regular file → in-place; open failure → `temp` + `warn`). `ChunkStats`
gains `chunks_kept`, `bytes_written`, `mode`; `Summary` gains `files_inplace`,
`files_temp` and `ram_bytes`. Unit tests for the resolution table.

#### 8. the data plane

`src/report.rs` (`SummaryFlags.repair_mode`, the two counters, `kept_bytes` and
`ram_bytes` in `conditional_extras` at fixed positions),
`src/repair/scope/report.rs` (`PLAN est_write_bytes`/`est_extra_bytes`,
`PlanDocument` per-file `mode` and `extra_bytes`). Follows the existing rule:
conditional keys only when their flag is passed. Unit tests for the rendering
of each new key on both planes.

#### 9. the single-writer lock

New `src/repair/lock.rs`: `{game_dir}/girpr.lock` via `create_new(true)`,
holding pid + timestamp; a live lock refuses with exit 1 naming the file and
the pid; a lock older than 24 h is taken over. `purge::keep_set()` gains
`girpr.lock` so the files-cleanup never deletes it, and the lock is only
created when the path scope admits it — the same rule `config.ini` lives
under. Unit tests: fresh lock, stale lock, scope-hidden path.

#### 10. `--scan`: an exact plan

New `src/repair/scan.rs`: the scan pass hoisted into the plan phase, parallel
over `jobs`, using `scan_file`. It produces per-file chunk verdicts that make
`est_download_bytes` exact, feed `--check-only` a damaged chunk count, and
let `--print-plan` report `scan_bytes`. `RepairPlan` carries an optional
verdict map; the ceilings are checked after it. Integration tests:
`--scan --print-plan` reports an estimate equal to the bytes the following
repair actually downloads, which is the assertion that does not exist today.

#### 11. docs and the artifact bridge

`README.md` (the "How it keeps disk usage low" section, the requirements line
`README.md:200`, the `--max-write-bytes` row, the two new flags, and the
S1/S2/S3 "deliberate simplifications" note, which becomes S1/S2 + the two
modes' properties of §5), `docs/02` step 6, `docs/03` §6, and `docs/06`'s
per-capture assertions.

The bridge amendment is the substantive part, and it is **additive** — nothing
is relaxed:

- **`download_bytes == patchmap total` stays.** It holds under the RAM tier for
  the reason in D7, and holding it is the point: it is what proves the new
  writer fetches the same set the patchmap says a consumer must fetch, and not
  one chunk more.
- **A differential assertion is added.** Each repair capture is driven
  **twice** — once with `--repair-mode temp` and once with the default — and
  the two runs must agree on the file they produce (MD5 *and* size) while
  differing on the cost: the in-place run must leave no `*_tmp` behind and
  must report strictly fewer bytes written than the temp run, which writes the
  whole file. **That is the space claim, asserted against real game bytes** —
  the offline fixture is too small to distinguish "wrote 128 bytes" from
  "wrote 64".
- The `must_not_fetch` check is unaffected (it is about the build namespace,
  not about reuse) and stays as it is.
- If the bridge is ever run with `--max-ram-bytes 0`, the ordered tier is
  selected and the `download_bytes` equality becomes an **upper** bound
  instead; the bridge says so rather than silently asserting the wrong
  direction.

Running it twice per capture roughly doubles the suite's cost (~4 minutes
instead of ~2). That is the price of proving the claim on real bytes, and it
is the reason §9.1 makes the suite mandatory rather than optional.

## 10. How the result is checked

- `cargo test` — 82 unit + 59 integration, all offline; the new cases ride with
  their commits.
- `cargo clippy --all-targets`.
- **`cargo test -p girpr --test artifact_bridge -- --ignored --nocapture` at
  every turn-around** (§9.1). Not at the end, not "when something looks wrong":
  every time. It is the only check that runs the real binary against real
  game bytes, and this change rewrites both the writer and the data plane.
  A missing bundle is a failure with instructions, and capturing a new one
  (`cargo run -p gircap -- capture --out artifacts`) is allowed.
- The fixture is 2 files / 3 chunks / 128 + 48 bytes, which is enough to prove
  every branch but not the *scale*. The scale claim (`S` → `G`) is arithmetic,
  and the one assertion that would catch a regression in the real world is
  `SUMMARY write_bytes` against `Σ size(repaired files)`, which the new
  `kept_bytes` key makes visible per run — and which the bridge's differential
  assertion now pins on real bytes.
- **The RAM tier is asserted, not assumed.** The integration tests pin that a
  repair writes only the wrong chunks (the file's other bytes are byte-identical
  before and after), that the ordered tier and the RAM tier produce the *same*
  file, and that `--max-ram-bytes 0` takes the ordered path. Without those
  three, "the payload is 0 for a corruption repair" would be a claim rather
  than a fact.
