# Low-disk repair — findings, options, verdict, and implementation plan

Series note: this is a **third** docs series, independent of the `repair-`
prefix series (`docs/01`–`docs/03`, which describe the shipped whole-install
patcher) and of `docs/04` (scope limitations) and `docs/06` (the artifact
bridge). It describes a change to the algorithm in `docs/02` step 6, so where
the two disagree this file wins once implemented; until then `docs/02` is still
the spec. Every claim about current behaviour is anchored to a file and line so
it can be re-checked when the code moves.

## 0. The question

> The current approach is "write `{path}_tmp`, then `rename` over the target".
> Can `girpr` recover more space **before** and **during** a repair, given that
> invariants may be weakened and backwards compatibility is not required?
> Primary goal: **disk**. Network is secondary. Other wins are welcome.

Everything below is about *transient* disk (the extra bytes the install needs
while the tool runs). Permanent shrinkage is `--purge-*`'s job and is not
re-litigated here except where it interacts.

## 1. What the disk is actually spent on today

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

| consequence | today | evidence |
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

### 1.3 Why the current shape exists

The temp-then-promote contract buys three things:

1. **The original is never damaged.** A crash, a full disk or a bad chunk can
   never leave a half-written target.
2. **Promotion is atomic.** `rename` is a metadata operation.
3. **Resume-by-length (S3).** A leftover `*_tmp` is a completed prefix, so the
   next run continues rather than restarts.

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

### 1.4 What is actually being protected

The thing the temp protects is a file that has **already failed its own
whole-file MD5**. That is the definition of a file being repaired. Its previous
bytes are:

- not part of the install the manifest describes,
- not usable by the game,
- worth at most a *network* saving to the next run, and only through the reuse
  map, which `try_reuse_slice` hash-checks anyway.

So the invariant "the original is never modified" is protecting bytes that are
already known-bad, at the price of rewriting the good ones. That is the whole
opportunity.

## 2. The disk model, per mode

Let `S` = target size, `O` = current on-disk size (0 if absent), `G` = growth.

| mode | bytes written per repaired file | peak extra free space | reuse sources readable | a crash leaves |
|---|---|---|---|---|
| `temp` (today) | `S` — the whole file | `S`, so `jobs × S_max` across a run | the old file, entirely | the old file + a partial `*_tmp` |
| `inplace` | only the chunks that are wrong *or moved* | `max(0, S − min(O, S))` | the old file, subject to a write order (§4.3) | a mixed file, re-verified next run |
| `discard` | `S` | `0` | none | a partial file |

For a corrupt-but-right-sized file — the common repair case — `inplace` needs
**zero** extra space and writes only the corrupt bytes. For a file that grows by
`Δ`, `inplace` needs `Δ`, not `S`. For a fresh install nothing is saved because
nothing is on disk to save.

`discard` (delete the file, then download it whole) has `0` peak but a full
network cost, so it is only ever right when nothing on the file is reusable —
which the plan can decide without reading it.

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
past `S` (rare, and cheaply detected — §4.4).

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

### D. Repair in place with ordered salvage — `inplace`

*Idea.* Write the target file where it lies. For each chunk: if the bytes
already at its destination hash correctly, write nothing; else copy from a
still-intact local slice; else download. Choose the chunk order so that the
reuse sources survive (§4.3).

*Buys.*

- peak extra space `G` per file instead of `S` — **zero for a same-size repair**;
- disk writes drop from `S` to the sum of the chunks that actually change;
- **a network win that needs no local manifest at all**: a corrupt-but-mostly-
  intact file now downloads only its corrupt chunks, where today it downloads
  the whole file whenever `getBuild(local)` failed.

*Costs / risks.* The "original is never modified" and "a crash leaves the
install as it was" invariants go away; reuse needs an order; a file that grows
needs `Δ` of free space with no query to check it against; the pre-flight
download estimate becomes an upper bound instead of a tight figure (§6).

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

*Verdict.* **Adopted as a separate, opt-in flag** (§7, commit 9). It is the
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
- `temp` — today's behaviour, kept as the escape hatch and as the mode the
  artifact bridge exercises.

### 4.1 Preconditions (all checkable, none assumed)

| # | precondition | how it is met |
|---|---|---|
| P1 | chunk offsets are ascending | sorted in `plan::build_plan` (commit 2); today nothing sorts them |
| P2 | chunks tile `[0, size)` exactly | validated at plan time; a gap is zero-filled and logged (commit 2) |
| P3 | the target is openable `read+write` | `open_inplace`; on failure the file falls back to `temp` (commit 5) |
| P4 | no read-only attribute | cleared before an in-place write, Starward-parity (`GameInstallService.cs:232-236`) — `girpr` does not do this today |
| P5 | one writer per `game_dir` | lock file (commit 8) — under `temp` a collision only wastes work; under `inplace` two runs with different plans can write conflicting bytes (D1) |
| P6 | the game is not running | already required; under `inplace` it is fatal rather than wasteful |
| P7 | free space ≥ growth | not checked in v1; a short write fails the file, and `--repair-mode temp` is the recovery |
| P8 | scope still governs everything | unchanged: out-of-scope paths are neither read for reuse nor written |

### 4.2 The algorithm

```
repair_one_file(file):
    whole-file MD5 == expected            -> Skipped            (unchanged)
    dry-run                               -> DryRun             (unchanged)
    mode = resolve(target)                                  (commit 6)

attempt_inplace(file):
    h = open(target, read+write+create)            # P3; failure -> mode = temp
    clear read-only if the open needed it                    # P4
    if len(h) > size and trim_is_free(file):                # §4.4, option B
        h.set_len(size)
    order = salvage_order(file.chunks, reuse, usable_len)   # §4.3, commit 3
    for c in chunks, in order:
        if c fits in len(h) and md5(h[c.offset ..+c.size]) == c.md5:
            chunks_kept += 1; continue                        # nothing written
        if order.live[i] and md5(h[c.src ..+c.size]) == c.md5:
            copy h[c.src ..+c.size] -> h[c.offset ..+c.size] # salvage
        else:
            download -> zstd -> verify md5 -> write at c.offset
    h.set_len(size)
    if md5(h) == file.md5: Ok
    else: reverify_chunks(h, file) -> redo only the mismatching ones -> re-check

attempt_temp(file):                    # today's repair_attempt_inner, verbatim
    ... existing _tmp resume / reuse / download / promote ...
```

Four properties worth naming explicitly:

1. **Keep costs one read and zero writes.** The per-chunk MD5 is the same
   computation the final whole-file MD5 would do, done per range.
2. **Every byte written is hash-gated before it lands.** Unchanged from today:
   a downloaded chunk's MD5 is checked before `write_all` (`file.rs:471`), a
   reused slice's before the copy (`file.rs:380`).
3. **Resume is free and needs no artefact.** An interrupted in-place file is
   simply a file with some correct and some incorrect chunks; re-running finds
   the correct ones with step 1 and rewrites the rest. There is no
   `*_tmp`, no marker file, no length heuristic — so `S3`'s "resume-by-length"
   becomes "resume-by-verification", which is strictly stronger (it cannot
   inherit a corrupt prefix).
4. **Final-MD5 failure costs one read pass, not a re-download.** Today a
   mismatch deletes the temp and re-fetches the whole file up to five times
   (`file.rs:523-530`). Re-verifying per chunk names the culprit.

### 4.3 The salvage order — the one non-obvious piece

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

### 4.4 Eager trim is nearly always free

`trim_is_free(file)` = "no reuse candidate has `r + s > size`". That is the
whole test: an entry in the local reuse map past the target's end can only ever
be a source, and only if the file shrinks a lot. When it is true the tail is
dead weight and `set_len` releases it before a single byte is written. When it
is false and `O − S` is large, the trim is deferred to the end (the file grows
during the repair instead) and the reason is logged.

## 5. Invariants: kept, weakened, new

**Kept, unchanged.**

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

**Weakened — deliberately, with the reason.**

| # | invariant | why it is safe to drop it |
|---|---|---|
| W1 | "the original is never modified in place" | the file already failed its own whole-file MD5; its bytes are not part of the install and are worth at most a network saving |
| W2 | "a crash leaves the install at its previous state" | it leaves a file that fails the same check and converges on the next run; the version in `config.ini` is not bumped by a run that did not finish |
| W3 | "a final-MD5 mismatch deletes the temp and restarts the file" | replaced by per-chunk re-verification, which is both cheaper and narrower |
| W4 | "`*_tmp` is the resume artefact" | resume-by-verification needs no artefact and cannot inherit a corrupt prefix; the `_tmp` path still exists under `--repair-mode temp` |
| W5 | "peak disk ≈ `io_threads × largest file`" (`README.md:200`) | it is now `io_threads × largest *growth*`, which is 0 for a same-size repair |
| W6 | "`stopped=bytes`/`stopped=write`: the next run resumes" | the next run re-verifies and continues; the wording must say so |

**New.**

- N1 a single-writer lock (commit 8).
- N2 read-only attributes cleared before an in-place write.
- N3 gaps in a chunk layout are zero-filled rather than left as stale bytes.
- N4 a file whose target cannot be opened `read+write` degrades to `temp`, loudly.

## 6. Risks

| # | risk | severity | handling |
|---|---|---|---|
| D1 | two concurrent `girpr` runs | **medium** | lock file, exit 1. Two in-place runs pursuing the *same* manifest write the same bytes and converge, but runs with different `--audio`/`--include` plans write conflicting bytes into one file, and `discard` deletes a file the other run has open (silently losing its work on Unix). Under `temp` a collision only wastes work: both runs share one `*_tmp`, its MD5 fails, nothing is promoted |
| D2 | read-only attribute on a game file | medium | clear it (P4); today the tool would fail the same way at rename |
| D3 | power loss mid-file | medium | whole-file MD5 catches it on the next run |
| D4 | the file is grown and the volume is full | medium | file fails, counted, retried, reported; recovery is `--repair-mode temp` |
| D5 | `est_download_bytes` becomes an **upper** bound, so `--max-download-bytes` over-refuses | medium | `--scan` (commit 9) makes it exact; documented until then |
| D6 | cross-file reuse, if ever added, breaks in-place unless file order is topologically sorted | latent | S2 stays same-file-only; recorded here |
| D7 | the artifact bridge asserts `download_bytes == patchmap total` | medium | that assertion becomes `≤`; `must_not_fetch` still holds. Amended in commit 10 |
| D8 | hard-linked game files are unlinked by an in-place write | low | no launcher creates them here; noted |
| D9 | `--max-write-bytes` documentation says "`*_tmp`" | low | reworded to "into the install"; the arithmetic is unchanged |
| D10 | a manifest whose chunks do not tile the file | low | validated at plan time, zero-filled, logged (P2) |

## 7. What changes in the machine contract

Only additive, and only under a flag or a mode, so the discipline in
`README.md:104-107` holds:

- `SUMMARY`: `repair_mode=auto|inplace|temp|discard`, `inplace=N`, `temp=N`,
  `kept_bytes=N` — emitted **only** when `--repair-mode` was passed. Always
  logged either way.
- `SUMMARY`: `write_bytes` (existing) keeps its meaning as "bytes written into
  the install"; only its prose changes.
- `PLAN` (`--print-plan` only): `est_write_bytes=`, `est_extra_bytes=`, and
  under `--scan` `scan_bytes=`.
- `--plan-out`: per-file `mode` and `extra_bytes`.
- No new exit code. The lock-file refusal is `1` (usage/config), like every
  other "the request was not well-formed" failure.

## 8. Deferred, with the reason

| item | why not now |
|---|---|
| interval-graph optimum for the chunk order (§4.3) | correct, but the two-direction rule already captures a uniform shift, which is the dominant real shape; it is one pure function to swap |
| cross-file salvage (§E) | mutually exclusive with `inplace` absent a file-level order |
| reclaim-on-demand + `GetDiskFreeSpaceExW` (§J) | `inplace` removes most of the pressure; it collides with `--max-delete-bytes`' "before anything is deleted" promise and deserves its own flag and doc |
| streaming zstd decode (§H) | RAM, not disk |
| hard-link dedup (§F) | permanent space, exFAT-hostile, interacts with in-place |
| a compressed chunk cache (§G) | strictly more disk |

## 9. Implementation plan

Ten commits, each one behaviour, each carrying its own tests, each leaving the
tree green. The series is ordered so that the risky part (the writer) lands
after the parts it depends on and before the parts that describe it.

### 1. `util`: one read pass, two answers

`src/util.rs`: add

```rust
fn verify_chunks(path: &Path, chunks: &[(u64, u64, &str)]) -> Result<(String, Vec<bool>)>
```

It walks the file once in 512 KiB blocks, feeding a running whole-file
`md5::Context` and one context per chunk (indices mapped from a single sort),
so the skip check, the in-place keep decision and the final re-verification
share one pass instead of three. A missing or short file returns
`("d41d8cd98f00b204e9800998ecf8427e", vec![false; n])` rather than an error —
"nothing is correct" is the answer, not a failure. Unit tests: intact file, one
corrupt chunk, truncated file, absent file, unsorted chunk input.

### 2. `plan`: a trustworthy chunk layout

`src/repair/plan/mod.rs`: sort `f.chunks` by `offset` in `build_plan`; drop
zero-length chunks defensively (logged); add `PlannedFile::layout()` returning
`(tiled, gaps)`. Nothing else in the crate can currently assume ascending
order, and §4.2's "already correct" test and §4.3's order proof both do.
Unit tests: unsorted manifest input is sorted; a hole is reported; an
overlapping pair is reported.

### 3. `reuse`: the salvage order (pure, no I/O)

New `src/repair/reuse.rs`: `salvage_order(chunks, candidates, usable_len) ->
Order { ascending: bool, live: Vec<bool> }`, implementing §4.3 exactly. The
module doc carries the two-line proof. Unit tests pin every case the proof
makes: `r > o` counts ascending, `r < o` counts descending, `r == o` counts
both, a source past `usable_len` counts for neither, and an empty candidate
list yields an arbitrary-but-valid direction.

### 4. `--repair-mode`

`src/cli.rs` (`#[arg(long, value_enum, default_value_t)]`), `src/config.rs`
(`RepairMode` + `RunCtx::repair_mode`). Unit tests: default is `auto`; an
unknown value is exit 1 through the existing clap remap.

### 5. `inplace`: the writer

`src/repair/file.rs`. Split `repair_attempt_inner` into `attempt_temp` (the
current body, unchanged, still promoting `_tmp`) and `attempt_inplace` (the
§4.2 algorithm): `open_inplace` with the read-only clear and the `temp`
fallback, `trim_is_free` + eager `set_len`, the ordered chunk loop with
keep/reuse/download, the final `set_len`, and `reverify_and_redo` replacing
"delete the temp and restart" on a final-MD5 mismatch. Both attempts funnel
through the existing `repair_attempt`, so the give-back-the-bytes rule
(`file.rs:283`) still holds for both.

Integration tests (`tests/repair_offline.rs`): a right-sized file with one
corrupt chunk is repaired **without** writing a `_tmp` at all; a file that
grows writes only the growth; a shifted-content file reuses its slices in the
direction the order picks and downloads the rest; an interrupted in-place run
resumes by verification on the next run.

### 6. mode resolution + the counters

`src/repair/file.rs`: `resolve_mode()` (missing target → create-in-place;
existing regular file → in-place; open failure → `temp` + `warn`). `ChunkStats`
gains `chunks_kept`, `bytes_written`, `mode`; `Summary` gains `files_inplace`
and `files_temp`. Unit tests for the resolution table.

### 7. the data plane

`src/report.rs` (`SummaryFlags.repair_mode`, the two counters, `kept_bytes` in
`conditional_extras` at a fixed position), `src/repair/scope/report.rs`
(`PLAN est_write_bytes`/`est_extra_bytes`, `PlanDocument` per-file `mode` and
`extra_bytes`). Follows the existing rule: conditional keys only when their
flag is passed. Unit tests for the rendering of each new key on both planes.

### 8. the single-writer lock

New `src/repair/lock.rs`: `{game_dir}/girpr.lock` via `create_new(true)`,
holding pid + timestamp; a live lock refuses with exit 1 naming the file and
the pid; a lock older than 24 h is taken over. `purge::keep_set()` gains
`girpr.lock` so the files-cleanup never deletes it, and the lock is only
created when the path scope admits it — the same rule `config.ini` lives
under. Unit tests: fresh lock, stale lock, scope-hidden path.

### 9. `--scan`: an exact plan

New `src/repair/scan.rs`: the skip-check pass hoisted into the plan phase,
parallel over `jobs`, using `verify_chunks`. It produces per-file chunk
verdicts that make `est_download_bytes` exact, feed `--check-only` a damaged
chunk count, and let `--print-plan` report `scan_bytes`. `RepairPlan` carries
an optional verdict map; the ceilings are checked after it. Integration tests:
`--scan --print-plan` reports an estimate equal to the bytes the following
repair actually downloads, which is the assertion that does not exist today.

### 10. docs and the artifact bridge

`README.md` (the "How it keeps disk usage low" section, the requirements line
`README.md:200`, the `--max-write-bytes` row, the new flag, and the S1/S2/S3
"deliberate simplifications" note, which becomes S1/S2 + the weakened
invariants of §5), `docs/02` step 6, `docs/03` §6, and `docs/06`'s
`download_bytes == patchmap total` assertion, which becomes `≤` with the
reason (already-correct chunks are not fetched).

## 10. How the result is checked

- `cargo test` — 82 unit + 59 integration, all offline; the new cases ride with
  their commits.
- `cargo clippy --all-targets`.
- The fixture is 2 files / 3 chunks / 128 + 48 bytes, which is enough to prove
  every branch but not the *scale*. The scale claim (`S` → `G`) is arithmetic,
  and the one assertion that would catch a regression in the real world is
  `SUMMARY write_bytes` against `Σ size(repaired files)`, which the new
  `kept_bytes` key makes visible per run.
- `tests/artifact_bridge.rs` stays `#[ignore]`d and is re-run by hand for
  commits 5, 9 and 10 — the only three that change what crosses the wire.
