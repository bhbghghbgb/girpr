# Streaming zstd decode in `girpr` — feasibility findings

Scope: the **S1** simplification ("whole-blob buffering") as it stands on `repair/stream-zst-decode`
@ `78102a9`. The question is whether `girpr` can decode zstd **as a stream** instead of buffering
the whole blob, and what that would cost in behaviour.

Everything numeric below is **measured**, not estimated: on the real bytes in
`artifacts/hk4e_global/7.0.0__7.1.0/20261020T154810Z` (the reference bundle), with a counting
global allocator. §8 says how to reproduce it.

---

## 1. Verdict

**Feasible. The chunk path is worth doing; the manifest path is optional; and the one thing that
has to change is a deliberate weakening of an invariant that both reference implementations never
had.**

| S1 site | what it buffers today | measured peak | streaming peak | verdict |
|---|---|---|---|---|
| **chunk** (`sophon::fetch_chunk_bytes` → `repair::file::write_downloaded_chunk`) | compressed body **+** decoded body, per active chunk | **6.1 MB** (8–12 MB at the real install's largest chunk) | **0.2 MB** | **do it** — 30×, and it is the only RAM term that scales with `--io-threads` |
| **manifest** (`sophon::fetch_manifest`) | compressed body **+** decoded body **+** parsed protobuf | **70.4 MB** (the `game` manifest) | 46.0 MB (zstd only) · **30.1 MB** (with an incremental protobuf parse) | **optional** — a one-shot phase; the real win needs protobuf streaming, which is a *different* concern |
| **S1 as documented** (`README.md:221`) | "whole-blob buffering (`sophon.rs`, `repair/file.rs`)" | — | — | the chunk half is a real simplification to lift; the manifest half is a deliberate trade ("lower disk use at the cost of higher peak RAM") and can stay |

Three things make this more than a memory optimisation:

1. **Both reference implementations already stream.** Starward writes the decompressed chunk
   straight into the file (`GameInstallHelper.cs:374-413`) and gates on the **whole-file** MD5
   (`GameInstallHelper.cs:443`); Collapse does the same and hashes on the fly
   (`Hi3Helper.Sophon/SophonAsset.Download.cs:576,600-606,644-668,703-719`). `girpr` is the odd
   one out: it is *stricter* than both. Lifting S1 is **parity**, not divergence.
2. **The sibling tool in this workspace already has the code.** `gircap` (`tools/capture`) ships
   `hash::zstd_decompress_from` (`tools/capture/src/hash.rs:348`) and `hash::zstd_measure_from`
   (`:448`), plus a capture path that streams a chunk to a file and digests it on the way past
   (`tools/capture/src/blobs/props.rs:216`). The port is a known quantity.
3. **It needs no new dependency.** `reqwest`'s `stream` feature is already on (`Cargo.toml:18`),
   `Response::chunk()` is available, and `zstd::stream::write::Decoder` is a `Write` adapter that
   decodes straight into whatever it is given. §5 has the sketch.

The blocker is not technical. It is that "every byte written is hash-gated before it lands"
(`src/repair/file.rs:28`, `docs/02-repair-chunk-spec.md:193`) **cannot** hold for a streamed
chunk: the digest is not known until the last byte has been written. §4 works through what the
invariant becomes, why that is sound, and the one place (the temp path's resume-by-length) where it
needs a rollback.

---

## 2. Where S1 actually is

Three call sites, all in `src/`, plus the comments that document the simplification.

### 2.1 The manifest blob — `src/sophon.rs:126-176`

```rust
let bytes = match resp.bytes().await { ... };          // sophon.rs:143  — the whole body
let decoded = match zstd::stream::decode_all(Cursor::new(&bytes)) { ... };  // sophon.rs:149
let digest = format!("{:x}", md5::compute(&decoded));  // sophon.rs:153
match SophonChunkManifest::decode(decoded.as_slice()) { ... }  // sophon.rs:165
```

The body is resident, the decoded body is resident, and the parsed protobuf is resident — all three
at once. The comment at `src/sophon.rs:95-98` states the trade: *"the whole blob is buffered in
memory … instead of Starward's file cache + streaming decompression. Simpler + lower disk use at the
cost of higher peak RAM on large manifests."* That trade is deliberate and is documented in
`src/repair/manifest.rs:9-10` as well.

### 2.2 The chunk blob, fetched — `src/sophon.rs:203-226`

```rust
match resp.bytes().await {
    Ok(b) => {
        tracing::trace!(chunk = %id, compressed_bytes = b.len(), attempt, "chunk downloaded");
        Attempt::Ok(b.to_vec())
    }
```

The whole compressed chunk is resident, and it is copied a second time by `to_vec()`.

### 2.3 The chunk blob, decoded — `src/repair/file.rs:1168-1228`

```rust
let blob = match sophon::fetch_chunk_bytes(&ctx.http, prefix, &c.id).await { ... };  // file.rs:1195
let decoded = zstd::stream::decode_all(Cursor::new(&blob)) { ... };                  // file.rs:1206
if decoded.len() as i64 != c.uncompressed_size { bail!(...) }                        // file.rs:1209
let dh = format!("{:x}", md5::compute(&decoded));                                    // file.rs:1217
if !dh.eq_ignore_ascii_case(&c.uncompressed_md5) { bail!("chunk {} md5 mismatch", ...) } // file.rs:1218
out.seek(SeekFrom::Start(c.offset))?;
reserve_write(ctx, decoded.len() as u64)?;
out.write_all(&decoded)?;                                                            // file.rs:1224
```

**Both buffers are live at the same time**, and `decode_all` (`zstd::stream::functions.rs:8-12`)
grows its `Vec` by doubling, so the decoded buffer's *capacity* is up to 2× the payload and the
final reallocation holds the old and the new buffer at once. The comment at
`src/repair/file.rs:1161-1167` names the alternative: *"instead of Starward's streaming `Pipe +
DecompressionStream` pipeline."*

### 2.4 The documentation that would have to change with it

| where | what it says |
|---|---|
| `README.md:220-224` | "**Deliberate simplifications** … S1 whole-blob buffering (`sophon.rs`, `repair/file.rs`)" |
| `docs/02-repair-chunk-spec.md:190-195` | "buffer the whole chunk, `decode_all` … A streaming `Pipe + DecompressionStream` pipeline (8–512 KiB buffer) may replace this later to lower peak RAM" |
| `docs/04-scope-limitation-spec.md:435-443` | E3 `--max-chunk-bytes`: "**Memory.** S1 buffers whole chunks … Peak RAM per active chunk is roughly `compressed + uncompressed` … At `--io-threads 8` that is a real number." |
| `src/sophon.rs:95-98`, `src/repair/file.rs:1161-1167`, `src/repair/manifest.rs:9-10` | the three `V1-SIMPLIFICATION (S1)` comments |

`docs/04` E3 is the interesting one: **the streaming decode removes that flag's memory
justification entirely.** What is left of E3 is only its second half ("Targeting" — a way to
deliberately exercise the largest-chunk branch). If the chunk path streams, E3's memory argument
is answered by the implementation rather than by a ceiling.

---

## 3. What the buffering costs — measured

Method: a counting global allocator (`LIVE`/`PEAK` bytes) around the exact three operations
`girpr` performs, on the real bundle bytes. Peak *live heap* is the honest metric here — a
Windows working-set reading is not, because the heap satisfies a later allocation from pages an
earlier `free` left behind, which is exactly what makes `decode_all`'s doubling look cheaper than
it is.

### 3.1 The chunk path — the number that matters

The largest chunk in the reference bundle (2 162 366 B compressed, 2 162 302 B decoded):

| shape | peak live heap |
|---|---|
| **today** — `bytes()` → `decode_all` → hash | **6.1 MB** (2.16 compressed + a 4 MiB decoded capacity) |
| **streamed** — the compressed body is never resident, the decoded bytes are digested on the way past | **0.2 MB** |

The real install's largest chunk is **3 689 476 B compressed** (`tools/capture/docs/00-spec.md:174`).
The measured ratios on the bundle's six largest chunks run from **1.00×** (the biggest one is
incompressible) to **1.16×**, so that chunk decodes to **3.7–4.3 MB** and today's per-chunk live
heap is **8–12 MB** (the compressed copy plus a decoded capacity of 4 or 8 MiB — `decode_all` grows
by doubling), momentarily ≈ 12–16 MB during the last reallocation.

At `--io-threads 8` that is **≈ 64–97 MB of resident chunk buffers** today, against **≈ 1.6 MB**
streamed. The chunk path is the only RAM term in the run that scales with `--io-threads`; the
manifest phase is one-shot and sequential (`src/repair/manifest.rs:28-44`).

The CPU cost is real but small: the decode of that 2.16 MB chunk takes **4 ms** buffered and
**7-8 ms** streamed, and it is flat across a 16 KiB → 1 MiB read window. Against a network
transfer, that is noise.

### 3.2 The manifest path — bigger, but one-shot

The `game` manifest of build 7.1.0: **9 007 187 B compressed → 16 827 424 B decoded** (ratio
1.87), **2 744 files**, **113 610 chunks**.

| shape | peak live heap | retained afterwards |
|---|---|---|
| **today** — `bytes()` → `decode_all` → `prost::decode` | **70.4 MB** | 29.8 MB (the parsed manifest) |
| zstd-streamed only — the compressed body never resident, the decoded body pre-sized to `ManifestFile.uncompressed_size` (`src/hyp.rs:151`) | **46.0 MB** | 29.8 MB |
| zstd-streamed **and** an incremental protobuf parse — one length-delimited `SophonChunkFile` at a time | **30.1 MB** | 30.0 MB |

The `en-us` manifest (1 231 165 B compressed → 2 283 463 B decoded, 177 files, 15 565 chunks)
goes from **9.3 MB** to **4.3 MB** on the same two shapes.

Two things follow, and they point in opposite directions:

- The **retained** 29.8 MB is the parsed manifest, and the plan needs it. No decode strategy
  removes it.
- The **zstd half alone** buys 70.4 → 46.0 MB. The rest (46.0 → 30.1) is **protobuf** streaming,
  which is *not* "zst decoding streaming" and should not be sold as part of it.

So the honest framing of the manifest site: **the decompressed bytes must be fully resident for
`prost` to parse them, so streaming the zstd decode does not remove them.** It only stops the
*compressed* copy from being resident too, and it removes the doubling spike by pre-sizing the
buffer with a number the API already hands over (`ManifestFile.uncompressed_size`).

---

## 4. The blocker: "every byte written is hash-gated before it lands"

`src/repair/file.rs:28` states it as a module-level invariant, and `docs/02:193` states it in the
spec:

> Every byte written is hash-gated before it lands, whichever path wrote it: a downloaded chunk's
> MD5 is checked before the write, and a staged source's MD5 is checked before it is staged and
> again before it is written.

**A streamed chunk cannot keep that.** The digest of a decoded stream is only known when the
stream ends, and the stream ends *after* the last byte has been written. There is no way around it
short of buffering the chunk — which is the thing being removed.

### 4.1 What the invariant becomes

> Every byte written lands in a region that was **already wrong at the moment of the write**, and
> the whole-file MD5 decides whether the file is good.

That is a strictly weaker invariant, and it is sound for `girpr`'s two write paths, for reasons
that are checkable in the code rather than hopeful:

- **The in-place path only ever writes to a region that was already wrong at the moment of the
  write.** The patch loop iterates `bad` — the chunks where `scan.dest_ok[i]` is false
  (`src/repair/file.rs:575-577`, `:700-725`). The re-do loop after a failed whole-file MD5 writes
  to `still_bad` — the chunks that are *still* wrong at that point (`src/repair/file.rs:759-778`),
  which is a superset of `bad` in the general case: the ordered tier's in-file copy can clobber a
  neighbour's destination, and that neighbour is then in `still_bad`. Both sets have the property
  that matters: the region a chunk is written into was already wrong when the write happened.
  `--repair-mode discard` deletes the target first, so everything is "already wrong" by
  construction (`src/repair/file.rs:563-570`). A streamed chunk therefore cannot destroy good
  data: the region it overwrites was wrong before and is wrong after, and the whole-file MD5 at
  `src/repair/file.rs:742-753` is the gate.
- **The ordered tier is unaffected.** The salvage rule (`src/repair/reuse.rs`) guarantees that a
  destination write never clobbers a *source* that is still needed. A downloaded chunk's bytes come
  from the network, not from the file, so it consumes no source region at all — the guarantee is
  untouched by it, exactly as it is for a reuse copy.
- **The temp path is safe by construction**: the `_tmp` is only promoted after the whole-file MD5
  passes (`src/repair/file.rs:1259-1293`), so a corrupt chunk in the `_tmp` cannot reach the
  target.

### 4.2 The one place that is *not* safe: the temp path's resume-by-length

The temp path resumes by **length** (`src/repair/file.rs:1100-1111`):

```rust
if cur_len >= end {
    tracing::trace!(... "chunk already complete -> keep");
    return Ok(ChunkOutcome::Resumed);
}
```

There is no hash check on that path. So if a streamed chunk's digest fails *after* its bytes were
written into the `_tmp`, the `_tmp` has grown past that chunk — and the **next run would skip the
chunk as "already complete"** and keep the corrupt bytes. That is a real regression, and it is the
one thing the streaming design must handle explicitly.

**The fix is small and exact**: on a chunk-digest mismatch in the temp path, truncate the `_tmp`
back to `c.offset` before failing the attempt. The plan sorts a file's chunks by offset
(`src/repair/plan/mod.rs:207`), so the `_tmp` is valid up to the start of the bad chunk and nothing
after it — the same invariant the resume already assumes. The chunks before it in this attempt were
each digest-checked when they were written, so truncating to `c.offset` throws away nothing that
was good. Collapse does the equivalent by seeking back to `chunk.ChunkOffset` and re-downloading
(`Hi3Helper.Sophon/SophonAsset.Download.cs:576,718-719`).

### 4.3 The attempt semantics must not move

Today a chunk whose MD5 does not match **fails the attempt**; the attempt's downloaded bytes are
handed back (`src/repair/file.rs:466-472`, `:1017-1028`) and the file is retried
(`src/repair/file.rs:294-337`). A streamed implementation must keep that, rather than letting the
corrupt chunk be absorbed by the `still_bad` re-do loop — otherwise the corrupt chunk's bytes would
be counted in a *successful* attempt and `SUMMARY download_bytes` would stop meaning "the bytes of
files that were actually repaired" (`README.md:161-165`).

This is pinned by an existing test: `download_bytes_counts_only_promoted_files_even_after_a_retry`
(`tests/repair_offline.rs:2426`) runs against a mock that serves one **corrupt-but-well-formed**
chunk (`tests/common/mod.rs:2443`, `corrupt_first_chunks`) and asserts that the total is exactly
the plan's chunk bytes — the abandoned attempt's byte is given back.

### 4.4 Both references are already here

- **Starward** (`reference-projects/Starward/src/Starward.RPC/GameInstall/GameInstallHelper.cs:374-413`):
  `Pipe` → `DecompressionStream` → `CopyToAsync(fs)` — the decompressed chunk goes **straight into
  the file**. The only MD5 on that path is the **whole-file** one at `:443`, and a mismatch deletes
  the `_tmp` and throws. There is no per-chunk digest check on the download path at all.
- **Collapse** (`reference-projects/Collapse/Hi3Helper.Sophon/SophonAsset.Download.cs`):
  `ZstdStream` over the HTTP body (`:600-606`), read → written to the output stream → hashed on
  the way past with `MD5.TransformBlock` (`:644-668`), finalised and compared at `:703-719`; a
  mismatch rolls the counters back and **re-downloads the chunk**.

So the "hash-gate before the write" is `girpr`'s own invention, and it is the reason S1 exists.
Dropping it is a move *towards* the two references, not away from them — and `girpr`'s version is
strictly stronger, which is worth saying out loud in the commit that does it.

---

## 5. The design that works

### 5.1 Phase 1 — the chunk path (recommended)

The shape that needs no new dependency and no async/sync bridge: **the write-side decoder**.

`zstd::stream::write::Decoder` (`zstd-0.13.3/src/stream/write/mod.rs:38`) is a `Write` adapter
that decodes what is written into it and forwards the **decompressed** bytes to the writer it was
built with. Feed it the response body and give it a writer that hashes and writes to the target
file, and the chunk never exists in memory:

```rust
// sophon.rs — a new streaming fetch
pub async fn stream_chunk_to(
    client: &reqwest::Client,
    url: &str,
    id: &str,
    sink: &mut impl Write,          // the target file, at c.offset
) -> Result<ChunkDigest> {          // { bytes_written, md5 }
    let mut resp = client.get(url).send().await?;
    // ... status check, retry policy: unchanged ...
    let mut dec = zstd::stream::write::Decoder::new(HashingWriter::new(sink))?;
    while let Some(part) = resp.chunk().await? {     // reqwest 0.12: async, never blocks a worker
        dec.write_all(&part)?;
    }
    dec.flush()?;
    Ok(dec.finish()?.digest())
}
```

`HashingWriter` is ~30 lines: `write` hashes the bytes, forwards them, and counts them.

Why this shape and not the read-side one:

- `Response::chunk()` is genuinely async, so **no async task ever blocks waiting for the network**
  — which matters, because a sync bridge over an async stream would park a runtime worker and can
  deadlock the connection pool at `--io-threads 8`.
- The decode is CPU work that the project already does inline today (`decode_all` runs on the
  async worker), so this is not a new sin.
- The file write stays blocking on the async worker, exactly as it is today
  (`out.write_all(&decoded)` at `src/repair/file.rs:1224` is a blocking `std::fs::File` write in
  an async fn).

The pieces that have to move with it:

| today | streamed |
|---|---|
| `reserve_download_bytes(c.compressed_size)` before the fetch, corrected by `adjust_download_bytes` afterwards (`src/repair/file.rs:1181-1205`) | unchanged — the compressed bytes are counted as they arrive and the same correction applies |
| `reserve_write(ctx, decoded.len())` **once**, before the write (`src/repair/file.rs:1223`) | **per piece** (like `copy_within` already does at `src/repair/file.rs:865`) — so the ceiling can now trip *mid-chunk* and leave a partially written chunk. The reuse-copy path already has this property, and the README already documents that a runtime hard stop leaves the work done in place (`README.md:152-160`) |
| the size check `decoded.len() != c.uncompressed_size` (`src/repair/file.rs:1209`) | the same check on the counted bytes, after the stream ends |
| the MD5 check before the write (`src/repair/file.rs:1217-1221`) | the same check after the stream ends — **and it fails the attempt** (§4.3) |
| — | **new**: the temp path truncates the `_tmp` to `c.offset` on a digest mismatch (§4.2) |

### 5.2 Phase 2 — the manifest path (optional)

The same write-side decoder, with a `Write` sink that appends to a `Vec` **and** hashes:

```rust
let mut sink = HashingVec::with_capacity(meta.manifest.uncompressed_size as usize);  // hyp.rs:151
let mut dec = zstd::stream::write::Decoder::new(&mut sink)?;
while let Some(part) = resp.chunk().await? { dec.write_all(&part)?; }
dec.flush()?;
let decoded = sink.into_bytes();
let digest = sink.md5();          // compared against meta.manifest.checksum, as today
```

That is 70.4 → **46.0 MB**, and it needs no new module and no new dependency. The
`ManifestFile.uncompressed_size` field (`src/hyp.rs:151`) is already in the API response and is
what makes the buffer exactly sized — the doubling spike disappears with it.

### 5.3 Phase 3 — incremental protobuf parse (optional, and *not* zstd streaming)

The remaining 46.0 → 30.1 MB is protobuf, not zstd. The manifest message is
`repeated SophonChunkFile chuncks = 1` (`src/sophon.rs:12-17`), so the wire form is a sequence of
`0x0A <varint len> <bytes>` and each `SophonChunkFile` can be parsed on its own as the bytes
arrive. That needs the **read** side of the decoder, which needs an async→sync bridge — a
`tokio::sync::mpsc` channel plus `Receiver::blocking_recv` inside a `spawn_blocking` task
(`tokio` is already a full-feature dependency, `Cargo.toml:17`), and a ~60-line wire reader.

This is a **separate piece of work with its own risk profile** (a hand-written protobuf reader must
agree with `prost` on field order, unknown fields and framing), and it should not be bundled into
"streaming zst decode". Its payoff — the manifest phase's peak becoming equal to what the plan
retains — is real but it is a one-shot phase.

---

## 6. What changes in the observable contract

Almost nothing, and the things that do change are all in the log, not the data plane:

| surface | change |
|---|---|
| `SUMMARY download_bytes` | **unchanged** — it is still the compressed bytes of the files that were repaired, and the corrupt-chunk attempt still hands its bytes back (§4.3) |
| `SUMMARY` keys | **unchanged** — no new key, no removed key |
| exit codes | **unchanged** — a chunk that fails its digest is still a failed attempt, `3` after five tries |
| `PLAN` / `--print-plan` | **unchanged** |
| the `chunk downloaded chunk=<id>` trace marker | **must keep its text** — `tests/artifact_bridge.rs:853-885` parses it to decide which chunks were really downloaded, and `must_not_fetch` is asserted from it |
| the trace line `chunk decoded compressed_bytes=… decompressed_bytes=…` (`src/repair/file.rs:1208`) | the compressed size is now known only at the end of the stream; the line still has both numbers |
| the write ceiling | a chunk can now be **partially written** when `--max-write-bytes` trips mid-chunk (the reuse-copy path already had this). The file's MD5 gate catches it and the next run re-verifies |
| `--max-chunk-bytes` (`docs/04` E3, specified but not enforced) | its **memory** justification disappears; only its "targeting" half is left |
| `--max-ram-bytes` (`src/repair/scope/ledger.rs`) | unchanged — it bounds the *staging* RAM, not the decode buffers. After streaming, the decode buffers are ~0.2 MB and the flag means what it always meant |

The **docs** that have to move with the code: `README.md:220-224` (the S1 line),
`docs/02-repair-chunk-spec.md:190-195` (the step-3 text and the "hash-gated before it lands"
sentence), `docs/04-scope-limitation-spec.md:435-443` (E3's memory argument), and the three
`V1-SIMPLIFICATION (S1)` comments.

---

## 7. Effort, risk, and the recommendation

| phase | files | size | risk |
|---|---|---|---|
| **1 · chunk path** | `src/sophon.rs` (a `stream_chunk_to` next to `fetch_chunk_bytes`), `src/repair/file.rs` (`write_downloaded_chunk` rewritten; the temp path's rollback), `src/repair/file.rs:28` (the invariant comment) | ~200-300 lines | **medium** — the hash-gate change is a *semantic* change and the temp path's rollback is the sharp edge |
| **2 · manifest path** | `src/sophon.rs::fetch_manifest` | ~40 lines | **low** — the same operations, in a different order |
| **3 · incremental protobuf** | a new module + a wire reader | ~150 lines | **medium** — a hand-written protobuf reader |

**Recommendation.** Do phase 1, then phase 2, and treat phase 3 as its own decision. Phase 1 is
where the RAM is (the only term that scales with `--io-threads`), where the parity argument is
strongest (both references stream), and where the reference implementation already exists
(`tools/capture/src/hash.rs`). Phase 2 is cheap and can ride along. Phase 3 is a protobuf project
wearing a zstd hat.

**Do phase 1 with the failure semantics pinned first.** The order that keeps the tests honest:

1. Land the *rollback* and the *attempt semantics* as they are today — a chunk whose digest fails
   fails the attempt, and the temp path truncates back to `c.offset`. Write the test for the
   rollback first (a corrupt chunk in the `_tmp` must not be resumed as "already complete").
2. Land the streamed fetch behind the same entry point, so the existing suite
   (`tests/repair_offline.rs`, 66 cases) runs unchanged against it.
3. Only then relax the wording in `README.md` / `docs/02` / `docs/04` and the `S1` line.

**The `#[ignore]`d artifact bridge is the real check** (`tests/artifact_bridge.rs`, `docs/06`): it
runs the built binary against real bundle bytes and asserts `download_bytes == patchmap total`, the
patched file's MD5, `must_not_fetch`, and that the in-place run writes strictly fewer bytes than
the temp run. It is the only test that exercises the real chunk sizes, and §3's numbers say the
peak RAM is exactly what it is designed to expose. It costs ~2 minutes per run and needs the
bundle — so it is a deliberate, manual step, not part of `cargo test`.

---

## 8. Reproducing the measurements

The numbers in §3 come from a counting global allocator wrapped around the exact three operations
`girpr` performs, run against the reference bundle. The probe is ~250 lines and depends only on
`zstd`, `prost` and `md5` — the same crates `girpr` already uses. It reads the blobs from a file
rather than from the network: the network is the same shape, and the point of the measurement is
the heap.

```
manifest-today          bytes()       -> decode_all      -> prost::decode
manifest-streamed       read::Decoder -> read_to_end     -> Vec (pre-sized)   [§5.2's shape]
manifest-fully-streamed read::Decoder -> incremental protobuf parse
chunk-today             bytes()       -> decode_all      -> md5
chunk-streamed <win>    read::Decoder -> md5 (the window size is in bytes)
```

against `artifacts/hk4e_global/7.0.0__7.1.0/20261020T154810Z`:

| input | compressed | decompressed | files | chunks |
|---|---|---|---|---|
| `manifests/7.1.0/game.manifest.zst` | 9 007 187 | 16 827 424 | 2 744 | 113 610 |
| `manifests/7.1.0/en-us.manifest.zst` | 1 231 165 | 2 283 463 | 177 | 15 565 |
| `chunks/blobs/f0b5cdc3a127f17f_*.zst` (the largest) | 2 162 366 | 2 162 302 | — | — |

The real install's largest chunk, **3 689 476 B compressed**, is recorded in
`tools/capture/docs/00-spec.md:174` and `tools/capture/docs/01-artifact-schema.md:384`
(`"max_chunk_compressed_bytes": 3689476`); the measured compression ratios on the bundle's large
chunks (1.00×–1.16×) are what turn it into the 8–12 MB per-chunk figure in §3.1.

The probe is **not** part of the repository: it was a scratch measurement, and the numbers it
produced are the ones quoted above. It lives outside the tree, at
`E:\Symlinks\Temp\opencode\zstdprobe` (`cargo build --release --offline`, then
`zstdprobe.exe <mode> <blob>`), and re-deriving the numbers needs only the bundle, which is
already in `artifacts/`.

---

## 9. The short version

- **Feasible: yes.** No new dependency, a working reference implementation in the same workspace,
  and both upstream references already do it.
- **The chunk path is worth it**: 6.1 MB → 0.2 MB of resident heap per active chunk, ≈ 64–97 MB →
  ≈ 1.6 MB at `--io-threads 8`, and the only RAM term that scales with the thread count.
- **The manifest path is optional**: the zstd half alone is 70.4 → 46.0 MB on a one-shot phase, and
  the rest of the win is protobuf streaming, which is a different project.
- **The price is one invariant**: "hash-gated before it lands" becomes "lands only where the bytes
  were already wrong, and the whole-file MD5 decides". That is what both references do, it is
  sound for `girpr`'s two write paths, and it costs one rollback (truncate the `_tmp` to
  `c.offset`) to keep the temp path's resume-by-length honest.
- **The blocker is not technical, it is editorial**: the invariant is written down in three places
  (`src/repair/file.rs:28`, `docs/02:193`, `docs/04:435`), and the change has to say out loud that
  `girpr` is giving up a guarantee the two reference implementations never had.
