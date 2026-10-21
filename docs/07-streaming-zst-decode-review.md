# Review note — the streaming zstd decode (`repair/stream-zst-decode`)

A hand-off note on the branch `repair/stream-zst-decode` (`main..HEAD`, 8 commits), the work
being **"implement zst decoding streaming to disk"**. It is written for the agent that did the
work: every item is a concrete, located fix, not a direction.

**This file is temporary.** It is deleted once the fixes land, exactly like
`tools/capture/docs/05-review-note.md` was. Nothing should refer to it — not the code, not the
documents, not the comments. A finding that needs a permanent home moves into the document that
owns the thing it is about.

---

## 0. What I already verified — do not redo it

Everything below was run against the branch as it stands (`bb54887`), on this machine, with the
bundle in `artifacts/hk4e_global/7.0.0__7.1.0/20261020T154810Z`:

| check | result |
|---|---|
| `cargo test -p girpr` | **143 unit + 68 integration + 7 CLI, 0 failed** (matches the fold commit's claim) |
| `cargo clippy -p girpr --all-targets` | clean |
| `cargo fmt --all -- --check` | clean |
| `cargo test -p girpr --test artifact_bridge -- --ignored --nocapture` | **26 passed; 0 failed** (17 min, real game bytes) |
| `cargo test -p girpr --test peak_heap -- --ignored --nocapture` | runs; prints `streamed 33 684 B (0.03 MB)` vs `buffered 6 435 587 B (6.14 MB)`, ratio 191× |

So: **the observable contract held.** The bridge asserts, over the real bundle and in both write
modes, that the repaired files are byte-identical, that `download_bytes` equals the patchmap
total, that `must_not_fetch` holds, and that the in-place path writes strictly fewer bytes than
the temp path — and all 26 cases pass. `SUMMARY download_bytes`, the exit codes, the `PLAN`
line and the `repaired` event are unchanged on the happy path.

**Verdict: the work is good. Three things must be fixed before it lands** (P1 below), and a
handful of sentences in the shipped documents now say something slightly different from what the
code does (P2). Nothing here says "redo the change".

The three P1s are all the same shape: **a contract that used to hold and now does not, silently.** The
trace marker's meaning moved (section 1), an unvalidated API field now drives an allocation
(section 2), and `write_bytes` lost the metric it was (section 3). None of them breaks the
repair; all three change what the tool *reports* about itself.

---

## 1. P1 — the `chunk downloaded` marker changed what it means

**Where:** `src/sophon.rs:404` (the marker), `docs/06-artifact-bridge-spec.md:338-340` (the
contract), `tests/artifact_bridge.rs:860` (the doc table).

**What changed.** The marker used to be emitted the moment the HTTP body had been read:

```rust
// main
match resp.bytes().await {
    Ok(b) => {
        tracing::trace!(chunk = %id, compressed_bytes = b.len(), attempt, "chunk downloaded");
        Attempt::Ok(b.to_vec())
    }
```

It is now emitted after the body has been **decoded and written into the file**:

```rust
// HEAD
let (compressed, digest) = match stream_body(resp.bytes_stream(), Some(expected), sink).await { ... };
tracing::trace!(chunk = %id, compressed_bytes = compressed, attempt, "chunk downloaded");
```

The text is identical, so nothing that greps for the string breaks. But the *trigger* moved, and
this line is a documented contract in two places:

- `docs/06` §4.7: "`--log-file` is the **documented last resort** … parse `chunk downloaded
  chunk=<id>` … out of the trace lines." The last resort now misses a chunk that was **asked for**
  but whose body failed to decode.
- `tests/artifact_bridge.rs:860` still says the record means "the body was read — a real
  download" and points at `sophon::try_fetch_chunk`, a function that no longer exists (it is
  `try_stream_chunk` now). The table is stale in both halves.

**Why it matters.** The bridge's `must_not_fetch` assertion is "no chunk id in this list was
requested". A chunk that is fetched and then fails to decode is now invisible to the
log-parsing route. In the bridge's own scenario the server's request log is the primary source
(`docs/06` §4.7), so the bridge itself still catches it — but the *documented* fallback does not,
and it is the fallback a user reaches for when a reuse regression shows up in the field.

**Fix (three lines).** Emit the marker where it used to mean what it says — as soon as the
response is known good, before the decode:

```rust
// src/sophon.rs, in try_stream_chunk, after the status check
tracing::trace!(chunk = %id, attempt, "chunk downloaded");
```

…or, if the `compressed_bytes` field has to stay on the line, keep the current line *and* add an
early one. Then update `tests/artifact_bridge.rs:860`'s table to the new function name and to
the new meaning, and add one clause to `docs/06` §4.7 saying what the marker now means
("the origin was asked for this chunk" vs "the origin was asked for this chunk and its body
decoded").

**The plan missed this.** Its §5 table said the marker "**must keep its exact text**". Text was
not the thing that mattered — the *position* was. Add "and its position" to the plan's wording
when the plan is next read.

---

## 2. P1 — an unvalidated API field now drives an allocation

**Where:** `src/sophon.rs:115`.

```rust
let mut decoded: Vec<u8> = Vec::with_capacity(meta.manifest.uncompressed_size.max(0) as usize);
```

`ManifestFile.uncompressed_size` (`src/hyp.rs:150-151`) is a raw `i64` out of the HoYoPlay JSON.
Nothing validates it — I grepped; there is no other reader of that field.

**What it costs.** `Vec::with_capacity` on a value above `isize::MAX` panics with "capacity
overflow"; on a merely-large value it is an allocation that fails as an OOM abort. Before this
branch the manifest's decoded bytes were whatever the body actually decoded to, so a wrong
number in the API response could not hurt anything. Now it can kill the run before the first byte
is fetched. This is the one place where the change made the tool *less* robust.

**Fix.** Clamp the hint; the buffer grows on its own if the real blob is bigger:

```rust
// The reported size sizes the buffer; it does not promise the length, and it
// is not trusted enough to allocate on. A blob bigger than the clamp grows
// the buffer; a bogus number cannot ask for an impossible allocation.
const MAX_HINT: usize = 64 << 20;
let hint = meta.manifest.uncompressed_size.clamp(0, MAX_HINT as i64) as usize;
let mut decoded: Vec<u8> = Vec::with_capacity(hint);
```

The comment at `src/sophon.rs:112-114` ("The decoded manifest is exactly the size the API
reported") should say "the size the API reported *sizes* the buffer" — the plan's §5.2 wording
("pre-sized to `ManifestFile.uncompressed_size`") is what turned into the certainty.

---

## 3. P1 - `SUMMARY write_bytes` lost the metric it was, and it has to be put back

**Where:** `src/repair/file.rs:1256` (`reserve_write` per piece), `src/repair/file.rs:1348` (the
validated write), `docs/04-scope-limitation-spec.md:570`, `README.md:262`,
`src/report.rs:264-268` (the counter's own doc comment).

**The requirement, which is the acceptance criterion.** Four things, all of them:

1. `SUMMARY write_bytes` goes **back** to `main`'s number and `main`'s meaning: the bytes written
   into the install **from sources that passed their gate** (a chunk's MD5, a reuse slice's MD5, a
   staged chunk's MD5, the scan's source verdict for an in-file copy, and the structural
   zero-fill).
2. It is a **pure function of the input state**. The same install, the same API, the same files
   give the same number whatever the transport did: no transport error, no corrupt chunk and no
   retry may move it.
3. The `--max-write-bytes` ceiling still **does not overshoot**. There is a test for that
   (`max_write_bytes_stops_mid_flight_and_leaves_the_work_for_the_next_run` asserts
   `summary.write_bytes <= limit`); it must keep passing.
4. **The output contract is untouched.** No new `SUMMARY` key, no other key's meaning changed, no
   exit-code change, no new flag. `write_bytes` is one key with one value, as it is today.

### 3.1 What changed, measured

The write ceiling used to be charged once, **after** the chunk's MD5 had passed:

```rust
// main
let dh = format!("{:x}", md5::compute(&decoded));
if !dh.eq_ignore_ascii_case(&c.uncompressed_md5) { ...bail... }
out.seek(...)?;
reserve_write(ctx, decoded.len() as u64)?;   // <-- only a verified chunk was charged
out.write_all(&decoded)?;
```

Now it is charged per piece, **before** the digest is known (`src/repair/file.rs:1256`), so a
chunk that fails its digest has already been charged, and a re-downloaded chunk is charged again.

I measured it: the same two-file fixture, the same install, three origins, `main` vs this branch
(`summary.write_bytes` read straight off the counter):

| origin | `main` | this branch |
|---|---|---|
| clean | **176** | **176** |
| one corrupt-but-well-formed chunk | **176** | **224** |
| origin failing every chunk request | 0 | 0 |

(`download_bytes` is 51 in every row of both columns. The download accounting is untouched.)

On `main`, `write_bytes` was a pure function of the input state: the corrupt chunk contributed
**0**, because a chunk that never passed its digest was never charged, and a transport error cost
0 as well. Every charged write came from a source that had passed an MD5 check, or was a
deterministic zero-fill.

On this branch the number **grows with the number of failures**: 48 extra bytes for one corrupt
chunk (it landed twice, once corrupt and once good), and it would be `N x chunk_size` for a chunk
that never recovers. Same install, same API, same files, a different number, because the network
was flaky. That is what has to be put back.

### 3.2 Why the charge cannot simply move back after the digest

The obvious fix, "charge it after the digest like `main` did", does not work, and the reason is
the whole point of this branch: **the bytes land in the file before the digest is known.** The
streaming decode writes each piece as it is produced (`src/repair/file.rs:1253-1262`), so by the
time the chunk's MD5 is checked at `:1344` the bytes are already on disk.

That leaves two numbers that are genuinely different:

- the bytes that **landed** (what the disk actually took, what the ceiling must be enforced
  against, since the ceiling exists to bound the disk and the bytes are spent whether they are
  good or not);
- the bytes that **validated** (what `main` reported, and what requirement 1 asks for).

Both have to exist. The fix is therefore **two counters, of which only one is reported**: the
live one stays internal and keeps serving the ceiling; the validated one is what the
`write_bytes=` key reads. That is an implementation detail, not an output change, which is why
it is in scope for this review and the *reported* split is not (see section 3.5).

### 3.3 The fix

The validated figure is already tracked, per file attempt, by `stats.bytes_written`: it is
incremented at exactly the validated write points, and the streamed download's increment
(`src/repair/file.rs:1348`) is **after** the digest check at `:1344`. So the validated figure is
one counter away:

| site | the validation that precedes it |
|---|---|
| `src/repair/file.rs:856` (`write_at`) | the staged bytes' MD5 (`file.rs:676-680`) |
| `src/repair/file.rs:885` (`copy_within`) | the scan's `source_ok` verdict, plus the salvage order |
| `src/repair/file.rs:1018` (`zero_fill`) | structural (a hole in the coverage) |
| `src/repair/file.rs:1205` (the reuse copy) | `md5_file_slice` against the chunk's MD5 (`file.rs:1185-1186`) |
| `src/repair/file.rs:1348` (the streamed download) | the chunk's MD5, checked at `:1344` |

So the shape of the fix is:
```rust
/// A write that passed its gate: the staged bytes' digest, the scan's source
/// verdict, the chunk's digest. The live counter (`write_bytes_spent`) counts every
/// byte that landed, because the ceiling bounds the disk; `write_bytes` counts the
/// ones the repair was *allowed* to land, and it is what the `write_bytes=` key
/// reports.
fn note_write(ctx: &FileCtx, stats: &mut ChunkStats, n: u64) {
    stats.bytes_written += n;
    ctx.summary.write_bytes.fetch_add(n, Ordering::Relaxed);
}
```

Charge it at the five sites above, and make the **`write_bytes=` key read the verified counter**.

**The naming has to be unambiguous.** The live counter is currently called `write_bytes` and the
key is called `write_bytes`; after this fix they are different numbers. Rename the *fields* so
the names match their roles (`write_bytes` = the reported/verified one, `write_bytes_spent` = the
live one the ceiling reads), or keep the fields and put a doc comment on each that says which is
which. Do not leave two numbers called `write_bytes` with no note. `docs/08` uses the same
two names for the reported keys (`write_bytes=` and `write_bytes_spent=`), so the two
documents must not drift apart on the word.

**Do not** derive the verified figure from the per-file `stats.bytes_written` at the end of the
file: those stats are per *attempt* and are dropped when the attempt fails, so the chunks an
earlier attempt wrote and validated would go missing from the total. Charging at the write sites
keeps it cumulative, exactly like `main`'s counter was.

**The alternative**, if you would rather have one counter: keep the single counter, charge it per
piece as now, and **hand the unvalidated bytes back** when a chunk fails its digest (the chunk's
landed bytes are subtracted). It is smaller, but the counter becomes non-monotonic and the
figure becomes "the bytes currently in the install that this run wrote" rather than "the bytes
this run wrote". The two-counter version is the one that reproduces `main` exactly; prefer it.

### 3.4 The acceptance test

Nothing pins the property that was lost, so the fix needs one. Add an offline test next to
`download_bytes_counts_only_promoted_files_even_after_a_retry` (`tests/repair_offline.rs`):

> The same install, two origins: a clean one, and one that serves a single corrupt-but-well-formed
> chunk. `summary.write_bytes` must be **equal** in both, and equal to the sum of the bytes the
> repair actually wrote. The corrupt chunk contributes **0** to it.

That is the regression test for requirement 2, and it fails on the branch as it stands (176 vs
224). The existing `--max-write-bytes` test covers requirement 3, and
`write_bytes_is_counted_only_when_the_flag_is_active` covers requirement 4's "one key, one value".

### 3.5 What this does NOT do: the reported split

The **reported** split - two keys, one for the live figure and one for the verified one, and the
CLI limits that go with them - is **not** in this review. It changes the output contract (new
`SUMMARY` keys, new flags, a bigger `stopped=` vocabulary), and a contract change is its own piece
of work with its own review.

It is written down, though: **`docs/08-metrics-live-vs-meaningful-plan.md`**, which is planned
against the house's own precedents and lands after this fix, in its own session. This review
deliberately keeps the output contract exactly as it is, so the plan starts from a stable base.

### 3.6 If the fix is judged too invasive

The fallback is a **documented** one number rather than a silent one: `write_bytes` keeps its
current meaning and `docs/04` section 6 gains

> `write_bytes` counts every byte written into the install, **including** the bytes of an attempt
> that later failed its digest and the bytes of a chunk that was re-downloaded. It is therefore not
> a function of the input state alone: a flaky origin makes it larger.

That is defensible as the honest disk cost, and it is the number the ceiling enforces. But it is a
**change to a reported metric's meaning**, made by a change that was supposed to be about
streaming, and it leaves the run with no deterministic "how much did the repair rewrite" figure.
If you take this fallback, say so in the commit message and in `docs/04`, and record it in
`docs/08`'s "what this plan does not settle" - because the split in `docs/08` starts from the
assumption that `write_bytes` means the verified figure.

## 4. P2 — the in-place path leaves unverified bytes behind, and the docs overstate the safety

**Where:** `src/repair/file.rs:37-42` (the module doc), `docs/02-repair-chunk-spec.md:274-280`.

The new invariant is right, and the argument is nearly right. One clause is optimistic:

> a downloaded chunk's bytes come from the network, not from the file, so they consume no source
> region and the ordered tier's salvage rule is untouched

That is true of *consuming* a source. It is not true of *destroying* one. In the in-place path a
download that fails its digest has **already overwritten** the chunk's region with bytes that
were never verified (`src/repair/file.rs:1253-1262` writes as the bytes are produced; the digest
is checked at `:1344`). Before this branch the region kept its old bytes.

Two consequences, both benign but both worth a sentence:

1. The old bytes at that offset may have been a **valid in-file reuse source** for a different
   chunk (the ordered tier's case: region *R* is chunk X's destination and chunk Y's source).
   The corrupt write destroys *R* as a source. The re-scan re-verifies every source
   (`scan.source_ok`), so the chunk is downloaded instead of salvaged — **more bytes, never wrong
   bytes** — but the plan's "the salvage rule is untouched" is not literally true.
2. The in-place path deliberately has **no rollback**, while the temp path does. That is the
   right asymmetry (the in-place path has no resume artefact; the next attempt's scan finds the
   chunk wrong and rewrites it — the C3 commit message says so, and it is correct), but a reader
   of `docs/02`'s invariant paragraph will reasonably expect the symmetry and find none.

**Fix.** In `src/repair/file.rs:37-42` and `docs/02:274-280`, say that the in-place path has no
rollback and why, and soften "untouched" to something like "the rule is not weakened: a
downloaded chunk consumes no source region, and a download that fails its digest can only cost
the reuse of the region it overwrote — which the next scan re-checks anyway".

---

## 5. P2 — the rollback's doc comment claims more exactness than the code has

**Where:** `src/repair/file.rs:1110-1140` (`rollback_tmp` and its doc).

The code is right. The comment overstates it:

> The truncation is exact because the plan sorts a file's chunks by offset: the `_tmp` is valid
> up to the start of the failed chunk and nothing after it, and the chunks before it were each
> digest-checked when they were written.

The sort is real (`src/repair/plan/mod.rs:207`, `sort_by_key` — stable), and the truncation is
`min(len, offset)`, which is the right rule. But:

- when the resume point falls **inside** the failed chunk (a `_tmp` from a previous run that ends
  mid-chunk), the truncation to `c.offset` also drops those bytes — they were never
  digest-checked, so nothing good is lost, but "nothing after it" is not what happens;
- "the chunks before it were each digest-checked when they were written" is true of the chunks
  written *in this attempt* and **not** of the prefix **resumed** from a previous run (the temp
  mode resumes by length, with no hash check — that is the whole reason the rollback exists).

**Fix.** Reword to the rule the code actually implements: the `_tmp` is truncated to the failed
chunk's offset so the chunk cannot be taken as "already complete"; the bytes before it are
either digest-checked (written in this attempt) or the prefix a previous run left, and the
whole-file MD5 gates the promotion either way.

The same two claims appear in the C3 commit message (`0d6a2c0`) — the message is history and
does not need changing, but the *code comment* does.

---

## 6. P3 — nits

| # | where | what |
|---|---|---|
| 6.1 | `src/sophon.rs:432-435` | The comment is factually wrong: "the zio writer's `finish` is what tells a complete frame from a truncated one, and **the wrapper does not expose it**". `zstd::stream::write::Decoder` exposes it — `do_finish()` (`zstd-0.13.3/src/stream/write/mod.rs:313`) and `finish(self)` (`:287`). `write::Decoder` *is* `zio::Writer` + `raw::Decoder` (`src/stream/write/mod.rs:368-371`), so the raw form buys nothing. Either use `write::Decoder` (as the plan's §2.1 sketched) or keep the raw form and fix the sentence. |
| 6.2 | `src/sophon.rs:266` | `HashingWriter::into_inner` is public, exported in the README's API table, and **never called**. Delete it, or use it. |
| 6.3 | `src/sophon.rs:397-401` vs `:164-168` | `StreamError::Content` is `Fatal` for a chunk and `Retry` for the manifest. The request pattern is the same either way (the old code also failed the *file* attempt on a bad frame), so this is a consistency nit — but the `StreamError::Content` doc says "a second identical request would answer identically", which is the manifest's reasoning and not the chunk's. Say which is which. |
| 6.4 | `src/repair/file.rs:1257-1258` | `ChunkFileSink::write` seeks before **every** piece (a syscall per 32 KB). The position already advances; one seek in `new`/`restart` would do. Low priority — the seek is what makes the sink position-independent, which is worth its price. |
| 6.5 | `src/sophon.rs:405-408` | `ChunkDigest.compressed_bytes` is set twice: `stream_body` already puts `compressed` in it, and `try_stream_chunk` rebuilds it with `..digest`. Harmless, but one of the two is redundant. |
| 6.6 | `tests/peak_heap.rs:10-12`, `docs/03-repair-rust-implementation-plan.md:176` | The instrument prints **0.03 MB** streamed (33 684 B), not the 0.2 MB the text claims; the "1.6 MB at `--io-threads 8`" figure follows from 0.2 MB. The doc figure is *conservative* (it understates the win), so it is not wrong — but the instrument and the document should agree, or the document should say "≤0.2 MB". |

---

## 7. The plan — recovered from history, and reviewed

The plan was `docs/07-streaming-zst-decode-implementation-plan.md`, added in `da3fcd1` and
deleted by the fold (`bb54887`). Recover it with:

```
git show da3fcd1:docs/07-streaming-zst-decode-implementation-plan.md
```

(The feasibility study it was based on is one commit earlier, `54fc9e3:
docs/07-streaming-zst-decode-feasibility.md`.)

### What the plan got right, and the work delivered

The plan's diagnosis was accurate and its sharp edge was the right one:

- it identified the invariant change ("hash-gated before it lands" → "lands where the bytes were
  already wrong, and the whole-file MD5 decides") and it was implemented as written;
- it identified **C3 — the temp path's resume-by-length** as the one place that needed a
  rollback, and the rollback is exactly the one the plan described (`truncate the _tmp to
  c.offset`);
- it insisted that a chunk failing its digest must still **fail the attempt** (§4.3), not be
  absorbed by the `still_bad` re-do loop — and that is what the code does, which is why
  `SUMMARY download_bytes` still means "the bytes of files that were actually repaired"
  (`download_bytes_counts_only_promoted_files_even_after_a_retry` still passes);
- it planned the write ceiling's move from one reservation per chunk to one **per piece**, and
  the code does that;
- it planned the measurement instrument (C5) and the fold (C6, with the "it was in | it is now"
  table in the commit message), and both landed;
- the six-commit split, the order, and the "each commit says what it does **not** verify"
  discipline were followed.

### What the plan missed

1. **The trace marker's position** (P1 §1). The plan said "must keep its exact text"; the text
   was not the thing that mattered.
2. **The allocation driven by `uncompressed_size`** (P1 §2). The plan treated the reported size as
   a promise (§5.2: "pre-sized to `ManifestFile.uncompressed_size`") and never asked what
   happens when the promise is wrong.
3. **`write_bytes` after a failed attempt** (P1 section 3). The plan's section 5 table enumerates the
   surfaces that change and does not mention it, and the plan never asked whether the counter was
   deterministic to begin with, which is the property that turned out to be the load-bearing one.
4. **The in-place path's unverified bytes** (P2 §4). §4.1 argues the invariant is sound for the
   in-place path; it is, but the argument is one step short of what the code does.
5. **The RAM figure** (P3 6.6): §1.1's 0.2 MB does not match the instrument the plan itself
   commissioned (0.03 MB).
6. **§C3's "does not verify" note is wrong** in the direction that matters: it says that with
   `--max-write-bytes` tripping mid-chunk "the resume then treats the *next* chunk as the
   incomplete one". With the rollback in place the resume treats *that* chunk as the incomplete
   one — which is the better behaviour. The note was written before the rollback existed.
7. **§C2/C3's split assumes two write functions.** The plan describes C2 as "the in-place path
   streams into the target" and C3 as "the temp path streams too", but both paths share
   `write_downloaded_chunk`, so C2 already streamed the temp path. The commit split looked
   cleaner than the change was. Not a defect — just don't let a reader of the plan think the
   temp path was buffered until C3.

### What the plan said and the work did **not** do

Nothing. Every item in the plan's worklist and every row of its §5 "observable contract" table
was delivered, with the two exceptions above (the trace marker's position and the write-bytes
accounting). The plan's §8 ("what this plan does not settle") was carried into `docs/03` §6's
deferred table correctly: the `streaming zstd decode` row is gone and the **incremental protobuf
parse** row is in its place, and it says the right thing — the manifest's *decoded* bytes must be
resident for `prost` to parse them, so what is left is protobuf, not zstd.

---

## 8. A sweep list for the documents

Read-through of every shipped sentence that describes the chunk or the manifest fetch. These
are the ones that need a touch-up after the fixes above:

| where | sentence | what to do |
|---|---|---|
| `docs/06-artifact-bridge-spec.md:338-340` | "`--log-file` is the documented last resort: parse `chunk downloaded chunk=<id>` …" | say what the marker now means (P1 §1) |
| `tests/artifact_bridge.rs:860` | the marker table: `sophon::try_fetch_chunk` / "the body was read" | new function name, new meaning (P1 §1) |
| `docs/04-scope-limitation-spec.md:570` | `write_bytes=<N>` | what it counts - only if the fix-back was not taken (P1 section 3.6) |
| `README.md:262` | `--max-write-bytes` | same clause, optionally (P2 §3) |
| `src/repair/file.rs:37-42` + `docs/02:274-280` | "the ordered tier's salvage rule is untouched" | P2 §4 |
| `src/repair/file.rs:1110-1140` | `rollback_tmp`'s exactness claim | P2 §5 |
| `src/sophon.rs:112-115` | "the decoded manifest is exactly the size the API reported" | "sizes", not "is exactly" (P1 §2) |
| `src/sophon.rs:432-435` | the zstd comment | P3 6.1 |
| `tests/peak_heap.rs:10-12`, `docs/03:176` | "0.2 MB" | P3 6.6 |
| `src/repair/manifest.rs:9-10` | "the blob is streamed and parsed in memory, and nothing is kept" | reads oddly now that S1 means "no manifest cache"; consider "the blob is streamed into memory, parsed, and not kept" |

Two things that are **correct** and should be left alone:

- `src/repair/file.rs:674` ("Every byte written is hash-gated before it lands, and a staged
  source is a write's source") — that is about the *staged* path, which keeps the stronger
  property. Still true.
- `README.md:221`'s S1 line ("S1 no manifest cache in `game_dir`") — the fold's redefinition of
  S1 is right, and the README's flow diagram's "download → zstd (streamed) → write at offset, MD5
  when the stream ends" is the order the code does it in.

---

## 9. Housekeeping

- The fold commit `bb54887`'s message says the bridge was run and "26 passed; 0 failed" — I
  reproduced that, so the claim stands.
- The fold's "it was in | it is now" table is complete, with one row missing: the **plan's
  section 5 table** itself. If the plan's table is the "what the observable contract does"
  list, then the trace marker's position and the write-bytes accounting are the two rows that
  turned out to be optimistic. Not a fix, just a note for whoever reads the plan next.
- **The fold: this file is deleted once the fixes land.** The house precedent is
  `cc90c41` ("gircap: a review note for the re-spec - temporary, and to be deleted"): a
  review note is a hand-off between two passes of the same work, not a project document.
  So:
  1. every finding above is either fixed or explicitly declined, and the decline is
     recorded in the commit message that declines it;
  2. each finding that needs a **permanent** home moves into the document that owns the
     thing it is about - the trace marker into `docs/06` section 4.7 and
     `tests/artifact_bridge.rs`'s marker table, the allocation clamp into `src/sophon.rs`'s
     doc comment and `docs/02` step 3, the `write_bytes` semantics into `docs/04` section 6
     and the README's `--max-write-bytes` row;
  3. **this file is deleted** in the same commit that closes the last finding, and nothing
     refers to it afterwards - not the code, not the documents, not the comments, not the
     commit messages that follow.