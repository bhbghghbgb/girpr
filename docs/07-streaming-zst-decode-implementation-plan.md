# Streaming zstd decode — implementation plan

The **S1** simplification ("whole-blob buffering") is lifted: the chunk and manifest blobs are
decoded **as a stream** instead of being buffered whole. This plan is the worklist; it is deleted
when the work is done, and its content lives in the documents that own it (§7, the fold).

**Scope.** `src/sophon.rs` (the manifest blob, the chunk blob) and `src/repair/file.rs` (the chunk
write). No flag, no `SUMMARY` key, no exit code changes.

**Why it is worth doing.** The chunk path is the only RAM term in a run that scales with
`--io-threads`, and it is the one place where `girpr` is *stricter* than both reference
implementations: Starward writes the decompressed chunk straight into the file and gates on the
whole-file MD5 (`GameInstallHelper.cs:374-413,443`), and Collapse streams the body through a
`ZstdStream` and hashes it on the way past
(`Hi3Helper.Sophon/SophonAsset.Download.cs:600-606,644-668,703-719`). `girpr` buffers the chunk,
decodes it whole, and checks the digest *before* the write — so it pays the RAM and is stricter
than both. Lifting S1 is **parity**, not divergence.

**What it costs.** One invariant changes, and §4 says exactly how and what it buys. The
observable contract does not change (§5).

---

## 1. The measurements that justify it

Measured on the real bytes in `artifacts/hk4e_global/7.0.0__7.1.0/20261020T154810Z` with a
counting global allocator around the exact three operations `girpr` performs today. The metric is
**peak live heap**, not a working set: the Windows heap satisfies a later allocation from pages an
earlier `free` left behind, which makes `decode_all`'s doubling look cheaper than it is.

### 1.1 The chunk path — the number that matters

The largest chunk in the reference bundle (2 162 366 B compressed, 2 162 302 B decoded):

| shape | peak live heap |
|---|---|
| **today** — `resp.bytes()` → `zstd::stream::decode_all` → `md5::compute` | **6.1 MB** (2.16 compressed + a 4 MiB decoded capacity) |
| **streamed** — the compressed body is never resident; the decoded bytes are digested on the way past into the file | **0.2 MB** |

The real install's largest chunk is **3 689 476 B compressed**
(`tools/capture/docs/00-spec.md:174`). The measured ratios on the bundle's six largest chunks run
from **1.00×** to **1.16×**, so it decodes to 3.7–4.3 MB and today's per-chunk live heap is
**8–12 MB** — momentarily 12–16 MB during `decode_all`'s last reallocation, because
`zstd::stream::decode_all` grows its `Vec` by doubling (`zstd-0.13.3/src/stream/functions.rs:8-12`).

At `--io-threads 8` that is **≈ 64–97 MB of resident chunk buffers** today, against **≈ 1.6 MB**
streamed. The CPU cost is negligible: 4 ms buffered vs 7–8 ms streamed for that chunk, flat across
a 16 KiB → 1 MiB read window, against a network transfer.

### 1.2 The manifest path — bigger, but one-shot

The `game` manifest of build 7.1.0: **9 007 187 B compressed → 16 827 424 B decoded**, **2 744
files**, **113 610 chunks**.

| shape | peak live heap | retained afterwards |
|---|---|---|
| **today** — `bytes()` → `decode_all` → `prost::decode` | **70.4 MB** | 29.8 MB (the parsed manifest) |
| **streamed** — the compressed body never resident, the decoded body exactly sized | **46.0 MB** | 29.8 MB |
| **streamed + an incremental protobuf parse** | **30.1 MB** | 30.0 MB |

The `en-us` manifest (1 231 165 → 2 283 463 B, 177 files, 15 565 chunks) goes from **9.3 MB** to
**4.3 MB**.

Two facts, and they point in opposite directions:

- The **retained** 29.8 MB is the parsed manifest, and the plan needs it. No decode strategy
  removes it.
- The **zstd half alone** buys 70.4 → 46.0 MB. The rest (46.0 → 30.1) is **protobuf** streaming,
  which is a different concern and is **not** in this plan's scope (§8).

So the manifest site's honest framing: the decompressed bytes must be fully resident for `prost`
to parse them, so streaming the zstd decode does not remove them. It stops the *compressed* copy
from being resident too, and it removes the doubling spike — because
`ManifestFile.uncompressed_size` (`src/hyp.rs:151`) is already in the API response and sizes the
buffer exactly.

---

## 2. The design

### 2.1 The chunk path — the write-side decoder

`zstd::stream::write::Decoder` (`zstd-0.13.3/src/stream/write/mod.rs:38`) is a `Write` adapter
that decodes what is written into it and forwards the **decompressed** bytes to the writer it was
built with. Feed it the response body and give it a writer that hashes and writes to the target
file, and the chunk never exists in memory:

```rust
// src/sophon.rs
/// What a streamed chunk produced: how many decoded bytes landed, and their digest.
pub struct ChunkDigest {
    pub bytes: u64,
    pub md5: String,
}

/// Stream one chunk's compressed body, decode it, and write the decoded bytes
/// into `sink` — digested on the way past.
pub async fn stream_chunk_to(
    client: &reqwest::Client,
    url: &str,
    id: &str,
    sink: &mut impl Write,
) -> Result<ChunkDigest>;
```

`HashingWriter` is the sink: `write` hashes the bytes, forwards them, counts them, and refuses to
forward more than the chunk's `uncompressed_size` (the size check moves from "after the decode" to
"as the bytes go", which is stricter, not looser).

**Why the write side and not the read side.** The read side (`zstd::stream::read::Decoder`) needs
an async→sync bridge, and a sync bridge over an async stream parks a runtime worker waiting for
the network — with `--io-threads 8` that can deadlock the connection pool. The write side is fed by
`reqwest::Response::chunk()`, which is genuinely async, so **no task ever blocks waiting for the
network**. The decode is CPU work the project already does inline today (`decode_all` runs on the
async worker), so this is not a new sin; and the file write stays blocking on the async worker,
exactly as `out.write_all(&decoded)` is today (`src/repair/file.rs:1224`).

**No new dependency.** `reqwest`'s `stream` feature is already on (`Cargo.toml:18`),
`Response::chunk()` is `async fn chunk(&mut self) -> Result<Option<Bytes>>` in reqwest 0.12, and
`zstd` is already a dependency.

### 2.2 The manifest path — the same decoder, an exactly-sized sink

The write-side decoder again, with a sink that appends to a `Vec` **and** hashes it:

```rust
let mut sink = HashingVec::with_capacity(meta.manifest.uncompressed_size as usize);  // hyp.rs:151
let mut dec = zstd::stream::write::Decoder::new(&mut sink)?;
while let Some(part) = resp.chunk().await? { dec.write_all(&part)?; }
dec.flush()?;
let decoded = sink.into_bytes();
let digest = sink.md5();          // compared against meta.manifest.checksum, as today
```

The checksum is still the MD5 of the **decompressed** bytes (`src/sophon.rs:148`, Starward parity
at `GamePackageService.cs:822-864`), and it is still compared before the protobuf parse.

### 2.3 What the sibling tool already has

`gircap` (`tools/capture`) ships the same primitives: `hash::zstd_decompress_from`
(`tools/capture/src/hash.rs:348`), `hash::zstd_measure_from` (`:448`), and a capture path that
streams a chunk to a file and digests it on the way past (`tools/capture/src/blobs/props.rs:216`).
The port is a known quantity, and the error messages there ("not a zstd frame", the head's magic)
are worth copying rather than reinventing.

---

## 3. The worklist

Six commits. Each carries the message below; each is verified the way the house verifies
(`cargo test -p girpr`, `cargo clippy -p girpr --all-targets`, and the **mandatory artifact
bridge** against the bundle), and each says what it does **not** verify.

### C1 · `girpr: sophon — the streaming fetch, and a writer that hashes on the way past`

**Changes.** `src/sophon.rs`: `ChunkDigest`, `HashingWriter`, and `stream_chunk_to` — the
write-side decoder over `Response::chunk()`, with the same three-way `Attempt` retry split
(`sophon.rs:236-243`) and the same `retryable` status rule (`:282-284`) the buffered path uses.
The `chunk downloaded chunk=<id>` trace marker keeps its exact text: the artifact bridge parses
it to decide which chunks were really downloaded (`tests/artifact_bridge.rs:853-885`).

**Verifies.** The streamed digest equals the buffered decode's digest for the same bytes (the
differential, over a locally encoded frame); a corrupt payload is detected; a truncated frame is
an error and not a short success; the bytes that reach the sink are the decoded bytes.

**Does not verify.** Nothing production-side: the repair loop still buffers. The retry behaviour
is inherited, not re-tested — the buffered path's retry tests do not move.

### C2 · `girpr: inplace — the chunk is streamed into the file, and the digest decides after`

**Changes.** `src/repair/file.rs::write_downloaded_chunk` streams into the target instead of
decoding into a `Vec` first. The digest is checked **when the stream ends**; a mismatch fails the
attempt, exactly as today (`file.rs:1217-1221`) — the attempt's bytes are handed back
(`file.rs:466-472`) and the file is retried (`file.rs:294-337`). The write reservation moves from
one call for the whole chunk (`file.rs:1223`) to **per piece**, like `copy_within` already does
(`file.rs:865`). The `chunk decoded` trace line keeps both numbers.

The module doc's invariant sentence (`file.rs:28`) is rewritten **in this commit**, because a
comment that lies about the code is worse than a comment that is out of date.

**Verifies.** The whole offline suite, and above all
`download_bytes_counts_only_promoted_files_even_after_a_retry`
(`tests/repair_offline.rs:2426`), which runs against a mock serving one **corrupt-but-well-formed**
chunk (`tests/common/mod.rs:2443`) and asserts the total is exactly the plan's chunk bytes — the
abandoned attempt's byte is given back. The in-place path's file-level gate is unchanged.

**Does not verify.** That the streamed bytes are the *same* bytes the buffered path wrote, on
real game bytes — the bridge does that, and the bridge is C3's and later commits' verification.
Nor the temp path, which is still buffered at this point.

### C3 · `girpr: temp — the streamed chunk, and the rollback that keeps the resume honest`

**Changes.** The temp path streams too, and it gets the one thing the in-place path does not need:
**the rollback**. The temp path resumes by **length** (`file.rs:1100-1111`) — there is no hash
check on that path — so a streamed chunk whose digest fails *after* its bytes were written would
leave a `_tmp` that has grown past that chunk, and the next run would skip it as "already
complete" and keep the corrupt bytes. On a digest mismatch the `_tmp` is therefore truncated back
to `c.offset` before the attempt fails. The plan sorts a file's chunks by offset
(`src/repair/plan/mod.rs:207`), so the `_tmp` is valid up to the start of the bad chunk and
nothing after it — the same invariant the resume already assumes, and the chunks before it were
each digest-checked when they were written.

**Verifies.** A new integration test on the mock: a corrupt chunk in a `_tmp` must **not** be
resumed as "already complete" — the `_tmp` is truncated, the chunk is downloaded again, and the
file is repaired. Plus the whole offline suite and the bridge.

**Does not verify.** The rollback's interaction with `--max-write-bytes` tripping mid-chunk (a
partial chunk is left in the `_tmp`; the resume then treats the *next* chunk as the incomplete
one, which is correct, but the case is not simulated).

### C4 · `girpr: manifest — the blob is decoded into a buffer that is already the right size`

**Changes.** `src/sophon.rs::fetch_manifest` streams the body into an exactly-sized hashing
buffer (`meta.manifest.uncompressed_size`, `src/hyp.rs:151`) instead of `resp.bytes()` +
`decode_all`. The checksum is still the MD5 of the decompressed bytes and is still verified
before the protobuf parse (`sophon.rs:153-165`). The manifest's retry split is unchanged.

**Verifies.** The streamed manifest parses to exactly the same `SophonChunkManifest` as the
buffered one (the differential, over a locally encoded manifest), the checksum comparison still
rejects a wrong checksum, and the suite is green.

**Does not verify.** The peak-RAM claim — that is C5.

### C5 · `girpr: the measurement — the chunk path's peak heap, printed by a test that is `#[ignore]`d`

**Changes.** A measurement instrument, in the shape `gircap` already uses for its own: an
integration test binary with a counting `GlobalAlloc` that prints the **peak live heap** of the
chunk path — one chunk, streamed — and, for comparison, the same chunk through the buffered
decode. It is `#[ignore]`d, like the bridge, because it is a measurement and not an assertion:
a number a human reads, not a threshold a test enforces.

**Why.** The claim "6.1 MB → 0.2 MB" is the plan's whole justification, and without an instrument
it is a sentence in a document that a later change can silently invalidate. With one, the number is
re-derivable in a minute, and a commit that reintroduces buffering shows up as a number that moved.

**Verifies.** That the instrument runs and prints both numbers, and that they are the numbers §1.1
claims.

**Does not verify.** A bound. The instrument does not fail when the number grows; it reports.

### C6 · `girpr: fold the streaming plan into the docs that describe the tool` — **the last commit**

The plan is done, so its content lives in the documents that own it and the plan itself goes
away. This is the house pattern (`be5fc4b`, `516d710`, `818f8ee`): a plan document lives while the
work is in flight and is deleted at the end, with the message carrying the table below.

**Changes.**

| it was in | it is now |
|---|---|
| the chunk fetch's algorithm (stream → decode → digest → write; the digest decides when the stream ends) | `docs/02` step 6.3, replacing the "buffer the whole chunk, `decode_all`" sentence |
| the new invariant ("every byte written lands where the bytes were already wrong; the whole-file MD5 decides") | `docs/02` step 6, and the module doc of `src/repair/file.rs` |
| the temp path's rollback (a chunk that fails its digest truncates the `_tmp` to its offset) | `docs/02` step 6, in the paragraph that describes how the temp mode resumes |
| the manifest fetch (stream → decode into an exactly-sized buffer → digest → parse) | `docs/02` step 3 |
| the chunk path's RAM ("the write buffer, not the chunk") | `docs/03` §6, in the line that says what the RAM now is |
| the **remaining** deferral (an incremental protobuf parse of the manifest) | `docs/03` §6's deferred table, replacing the `streaming zstd decode` row |
| E3's consequence (`--max-chunk-bytes` no longer bounds anything) | `docs/04` E3, whose memory half is gone |
| the S1 line | `README.md`'s "Deliberate simplifications", which loses the chunk half |
| the plan itself | deleted |

**Verifies.** The whole suite, the bridge, and a read-through: every sentence in the shipped docs
that describes the chunk or the manifest fetch is true of the code as it stands.

**Does not verify.** Anything about the code — it changes no code.

---

## 4. The invariant that changes

`src/repair/file.rs:28` and `docs/02:193` both say:

> Every byte written is hash-gated before it lands, whichever path wrote it.

**A streamed chunk cannot keep that.** The digest of a decoded stream is only known when the
stream ends, and the stream ends *after* the last byte has been written. There is no way around
it short of buffering the chunk — which is the thing being removed.

**What it becomes:**

> Every byte written lands in a region that was **already wrong at the moment of the write**, and
> the whole-file MD5 decides whether the file is good.

This is strictly weaker, and it is sound for both write paths, for reasons that are checkable in
the code rather than hopeful:

- **The in-place path writes only to regions that were already wrong.** The patch loop iterates
  `bad` — the chunks where the scan's `dest_ok[i]` is false (`file.rs:575-577`, `:700-725`). The
  re-do loop after a failed whole-file MD5 writes to `still_bad` — the chunks that are *still*
  wrong at that point (`file.rs:759-778`) — which is a superset of `bad` in the general case: the
  ordered tier's in-file copy can clobber a neighbour's destination, and that neighbour is then
  in `still_bad`. Both sets have the property that matters: the region was wrong when the write
  happened. `--repair-mode discard` deletes the target first, so everything is "already wrong" by
  construction (`file.rs:563-570`).
- **The ordered tier is untouched.** The salvage rule (`src/repair/reuse.rs`) guarantees that a
  destination write never clobbers a *source* that is still needed. A downloaded chunk's bytes
  come from the network, not from the file, so it consumes no source region at all.
- **The temp path is safe by construction**: the `_tmp` is promoted only after the whole-file MD5
  passes (`file.rs:1259-1293`), so a corrupt chunk in the `_tmp` cannot reach the target. The
  rollback (C3) is what keeps its *resume* honest.

**What does not weaken.** The file-level gate is unchanged: the whole-file MD5 still decides
whether a file is good, `config.ini` still advances only over a fully verified install, and a
failed attempt still hands its downloaded bytes back.

| invariant | before | after |
|---|---|---|
| a downloaded chunk's digest is checked before its bytes are written | **yes** | **no** — it is checked when the stream ends |
| every byte written lands where the bytes were already wrong | yes (by construction) | yes (by construction) |
| the whole-file MD5 gates the file | yes | yes |
| a failed attempt hands its downloaded bytes back | yes | yes |
| the temp mode resumes by length | yes | yes — and the `_tmp` is truncated back to the failed chunk's offset, so the resume point is honest |
| the target file is never modified in the temp mode | yes | yes |
| `SUMMARY download_bytes` = the bytes of the files that were repaired | yes | yes |

---

## 5. What the observable contract does

Almost nothing, and what does change is in the log, not the data plane:

| surface | change |
|---|---|
| `SUMMARY download_bytes` | **unchanged** — still the compressed bytes of the files that were repaired, and a failed attempt still hands its bytes back |
| every other `SUMMARY` key | **unchanged** — none added, none removed |
| exit codes | **unchanged** — a chunk that fails its digest is still a failed attempt, `3` after five tries |
| `PLAN` / `--print-plan` / `--plan-out` | **unchanged** |
| the `chunk downloaded chunk=<id>` trace marker | **must keep its exact text** — the bridge parses it (`tests/artifact_bridge.rs:853-885`) and `must_not_fetch` is asserted from it |
| the `chunk decoded compressed_bytes=… decompressed_bytes=…` trace line | keeps both numbers; the compressed size is now known at the end of the stream |
| `--max-write-bytes` | a chunk can now be **partially written** when the ceiling trips mid-chunk. The reuse-copy path already had this property (`file.rs:865`), and the README already documents that a runtime hard stop leaves the work done in place (`README.md:150-160`) |
| `--max-ram-bytes` | **unchanged** — it bounds the *staging* RAM, not the decode buffers. After this the decode buffers are ~0.2 MB and the flag means what it always meant |
| `--max-chunk-bytes` (`docs/04` E3, specified but not enforced) | its **memory** justification disappears; only its "targeting" half is left |

---

## 6. The risks

| risk | how it is handled | where |
|---|---|---|
| a corrupt streamed chunk lands in the file and the file is left worse | the destination was already wrong; the whole-file MD5 gates the file; the attempt fails and is retried | C2 |
| the temp path resumes a corrupt chunk as "already complete" | the `_tmp` is truncated back to the chunk's offset on a digest mismatch | C3 |
| the corrupt chunk's bytes end up counted in a *successful* attempt | a digest mismatch **fails the attempt**; it is not absorbed by the `still_bad` re-do loop | C2, and the existing test at `tests/repair_offline.rs:2426` |
| the bridge's "which chunks were downloaded" reading breaks | the trace marker keeps its exact text | C1 |
| the write ceiling overshoots by a chunk | the reservation is per piece, like the reuse copy's | C2 |
| a runtime worker blocks on the network | the body is consumed with `Response::chunk()`, which is async | C1 |
| the manifest's checksum is verified against the wrong bytes | the checksum is still the MD5 of the **decompressed** bytes, computed by the same sink that received them | C4 |
| the RAM claim is a sentence that later changes invalidate | an `#[ignore]`d instrument prints the number | C5 |
| the docs keep describing a buffering tool | the fold moves the content and deletes the plan | C6 |

---

## 7. The fold

The last commit is C6, and it is not optional: a plan document that outlives its work is
archaeology. The house has done this three times — `be5fc4b` ("girpr: fold the low-disk plan
into the docs that describe the tool"), `516d710` ("gircap: fold this plan into the documents, and
delete it") and `818f8ee` ("docs: merge the option/plan pairs into two specs, and drop the
archaeology") — and the shape is always the same: the plan is deleted, its content is distributed
to the documents that own it, and the commit message carries an **"it was in | it is now"** table.

Two rules from those three commits, which this plan's fold follows:

1. **The account of how the thing got here is `git log`'s job.** The measurements in §1 are the
   argument for the change, not a description of the tool; once the change is in, the *behaviour*
   is what the docs carry, and the numbers that remain are the ones a reader needs (what the RAM
   is now), not the ones that justified the change.
2. **The fold changes no code.** `be5fc4b` says so explicitly: the source comments that pointed
   into the deleted file were rewritten in the previous commit, and nothing else referenced it.
   The same holds here — C2 rewrites `src/repair/file.rs`'s module doc, and C6 touches documents
   only.

---

## 8. What this plan does not settle

- **The incremental protobuf parse of the manifest.** The remaining 46.0 → 30.1 MB is protobuf, not
  zstd: the message is `repeated SophonChunkFile chuncks = 1` (`src/sophon.rs:12-17`), so the wire
  form is a sequence of `0x0A <varint len> <bytes>` and each entry could be parsed on its own as
  the bytes arrive. It needs the **read** side of the decoder, which needs an async→sync bridge
  (a `tokio::sync::mpsc` channel plus `Receiver::blocking_recv` inside a `spawn_blocking` task),
  and a ~60-line wire reader that must agree with `prost` on field order, unknown fields and
  framing. That is a separate piece of work with its own risk profile, and it is **not** zstd
  streaming — it should not be bundled into it. Its row stays in `docs/03` §6 after the fold,
  reworded from `streaming zstd decode` to what is actually left.
- **A compressed chunk cache on disk.** Still refused, for the same reason: strictly more disk,
  retained on purpose.
- **Whether the manifest's decoded bytes should live in a file at all.** Starward caches the
  *compressed* manifest under the app cache (`GamePackageService.cs:822-864`); `girpr` does not,
  and S1's "lower disk use" half is a real property that streaming does not change. This plan
  leaves it alone.
- **The speed limiter and the predownload paths.** Neither exists in `girpr` v1, and a streaming
  fetch is where they would plug in; the plan does not add them.
