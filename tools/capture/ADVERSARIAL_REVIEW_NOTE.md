# ADVERSARIAL REVIEW — seventh pass (`gircap`)

**Baseline:** HEAD `8d25b06` (`docs(review): compact the intermediate pass history`), branch `fix-gaps/gircap`, tree clean before this note. Reviewed 2026-10-28.

**Method:** full reads of the files named below, `rg` over `tools/capture/src` and `tools/capture/docs`, the six-pass decisions in `ADVERSARIAL_REVIEW.md`, and one executed reproduction outside the repository. The four standard checks were run on this tree (results at the end). No source, test, Cargo, or documentation file was modified; only this note was created.

The sixth pass is in `ADVERSARIAL_REVIEW.md` (J1–J3). Its binding decisions are respected here. Both findings below are sibling gaps of earlier fixes rather than re-reports: K1 is the remaining unchecked arithmetic of the class `decode_counts` and `Budget` already fixed; K2 is the post-rename refusal path that F4 did not cover.

## Verdict

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| K1 | **high** | bug | `src/assemble/ranges.rs:48` (siblings at `:58`, `:89`, `:136`; `src/verify/materialize.rs:205`; `src/verify/mod.rs:905`; `src/assemble/mod.rs:1123`; `src/blobs/mod.rs:688`; `src/manifest/diff.rs:412`, `:460`, `:467`, `:483`) | Arithmetic on manifest/index numbers is unchecked: `gircap materialize`/`verify` panics on a crafted offset in debug builds, and a wrapped (release) or negative (any build) `download_bytes` passes A20. |
| K2 | medium | bug | `src/blobs/mod.rs:488` (with `:423-426`, `:478`; `src/blobs/props.rs:243-244`, `:291`) | A fetched body that is not a zstd frame is renamed into `chunks/blobs/` but recorded as an unfilled fetch (`bytes: 0`, no digests), so the evidence the spec says to keep is lost and `BlobProps::undecodable` is dead in production. |

---

### K1 — Unchecked arithmetic on untrusted offsets and sizes

**Severity: high. Class: bug.**

The project already treats this class as real. `decode_counts` was changed to checked/saturating arithmetic with this comment:

```rust
// src/manifest/mod.rs:646-660
// `checked_add`, because this counter is a *measurement* and must not
// panic on the thing it measures. A manifest declaring
// `offset = i64::MAX` overflowed here: an abort in debug and test
// builds ...
cursor = match c.offset.checked_add(c.uncompressed_size) {
    Some(next) => next,
    None => {
        gapless = false;
        i64::MAX
    }
};
chunk_bytes = chunk_bytes.saturating_add(c.uncompressed_size);
```

The same numbers still reach plain `+` and plain `.sum()` at the sites below. The bundle's `chunks/index.json` and `patchmaps/*.json` are documents an operator does not have to have produced, and `Bundle::open` deliberately accepts them without bounding `offset`, `uncompressed_size`, or `compressed_size`.

**The reproduced panic (offline `materialize`/`verify`).** `Range::at` is the unchecked add:

```rust
// src/assemble/ranges.rs:47-49
pub const fn at(offset: i64, len: i64) -> Range {
    Range::new(offset, offset + len)
}
```

`Layout::of` maps every index reference through it:

```rust
// src/assemble/ranges.rs:134-136
pub fn of(parts: impl IntoIterator<Item = (i64, i64)>, declared_size: i64) -> Layout {
    let ranges: Vec<Range> = parts.into_iter().map(|(o, l)| Range::at(o, l)).collect();
    let total: i64 = ranges.iter().map(Range::len).sum();
```

`materialize::rebuild` builds those parts from the index, before any blob is read:

```rust
// src/verify/materialize.rs:201-208
let parts: Vec<(i64, i64)> = chunks
    .iter()
    .map(|(id, o)| (*o, by_file.size(id)))
    .collect();
let declared_size: i64 = parts.iter().map(|(_, l)| *l).sum();
let layout = Layout::of(parts.iter().copied(), declared_size);
```

A `chunks/index.json` with one blob (`uncompressed_size: 1`) and one reference at `offset: 9223372036854775807` is schema-valid and open-able; the reference path is an ordinary bundle path, so `validate_document_paths` has nothing to refuse. `gircap materialize` (and `gircap verify`, which calls the same `rebuild_all`) reaches `Layout::of([(i64::MAX, 1)], 1)`, and the add overflows. The executed reproduction used the public `verify::materialize::rebuild` with exactly that `ChunkIndex` and produced:

```text
thread 'main' (12052) panicked at E:\hoccode\hocrust\girpr\tools\capture\src\assemble\ranges.rs:48:28:
attempt to add with overflow
```

The panic happens before the missing-blob check, so no real blob is needed. This contradicts the module's stated contract "Violations are data" and "`Layout::of` never returns an error". In release the add wraps; the simple case then becomes an inverted range and `tile` reports an A11 violation, so the crash is a debug/test-build failure for this input. The next two scenarios are not debug-only. The capture path reaches the same `Layout::of` from the manifest entries at `src/assemble/mod.rs:785-788`, so a live or replayed manifest with the same arithmetic has the same debug abort in `assemble_side`.

**The released A20 bypass.** `check_patchmap_arithmetic` re-derives the cost with a plain sum:

```rust
// src/verify/mod.rs:905
let bytes: i64 = fetch.iter().map(|e| e.compressed_size).sum();
```

`PlanEntry.compressed_size` is a raw `i64` from the document. A patchmap whose plan has two `"decision": "fetch"` entries with `compressed_size: 9223372036854775807`, and whose `totals.download_bytes` and `expectations.download_bytes` are both `-2`, makes the debug sum panic and the release sum wrap to `-2`. A20 then compares `-2` with `-2` and reports the assertion as passed (`src/verify/mod.rs:915-942`), so a consumer reads a patch that costs a negative number of bytes. A single fetch entry with `compressed_size: -1` needs no overflow at all: `build_map` copies the raw value (`src/assemble/mod.rs:1033`, `:1090`) and accumulates it with `+=`:

```rust
// src/assemble/mod.rs:1121-1124
Decision::Fetch => {
    fetch += 1;
    download_bytes += e.compressed_size;
}
```

so `Totals.download_bytes` and `Expectations.download_bytes` are `-1`, A20 recomputes `-1`, and it passes in both debug and release. `decode_counts` and `diff.rs` already clamp negatives with `.max(0)` for exactly this reason; `build_map` does not.

The same arithmetic appears in the capture-side budget and scoring:

```rust
// src/blobs/mod.rs:684-689
pub fn projected_download(wants: &[Want]) -> u64 {
    wants
        .iter()
        .map(|w| w.chunk.compressed_size.max(0) as u64)
        .sum()
}
```

`i64::MAX.max(0) as u64` is `9223372036854775807`; three such wants overflow `u64` in debug (two still fit). `src/manifest/diff.rs:412`, `:460` and `:483` accumulate `compressed_size.max(0) as u64` with plain `+=`, and `:467` adds two clamped values before using them; three such chunks reach the same abort. In release these wrap and the recorded counts/totals disagree with the manifests without any row saying so.

**Other unchecked terms in the same layout.** `Range::len` and `Range::to` are plain subtractions (`src/assemble/ranges.rs:58`, `:89`), and `Layout::of` computes `total` with `.sum()` (`:136`). A layout whose first range is `(0, i64::MAX)` and whose next range starts at `i64::MIN, 0` reaches `Range::to` and panics on `i64::MIN - i64::MAX` in debug; two maximum-length ranges overflow the `total` sum. Both are reachable from the same crafted index or manifest and are part of the same defect.

**Why fixing only the sum is not enough.** `declared_size` at `src/verify/materialize.rs:205` is summed before `Layout::of`, so a checked `Range::at` alone would leave that panic; the index's `props.uncompressed_size as i64` can be negative for a `u64` above `i64::MAX`. The A20 check must also decide what a negative `compressed_size` means, because `build_map` currently writes it through.

**Suggested fix.**

1. Add one arithmetic convention for byte counts, as `decode_counts` and `diff.rs` already use: clamp with `.max(0)` and accumulate with `saturating_add` for measurements; use `checked_add` where a wrong total must be a failed check rather than a number.
2. In `Layout::of`, compute each end with `offset.checked_add(len)`; on `None`, push a violation such as `chunk {i}: offset {offset} + length {len} overflows i64` and use a saturated end so `tile` refuses via `is_clean() == false`. Compute `total` with `checked_add` and push a violation on overflow; make `Range::len`/`Range::to` use `checked_sub` (or report in `i128`) so the gap/overlap diagnostic itself cannot abort. Keep `Range::at` non-panicking (e.g. `saturating_add`) for its public callers.
3. In `materialize::rebuild`, sum `declared_size` with `checked_add`; on overflow return a named `Err` before `Layout::of` (or let `Layout::of` record it and let `tile` fail).
4. In A20, fold with `checked_add`; on overflow push `Assertion::fail(A20_ID, …)` saying the FETCH sum does not fit `i64`. Reject a negative `compressed_size` there (or clamp it with `.max(0)` and flag the manifest); use the same choice in `build_map`, `projected_download` and `diff.rs`.
5. Tests: `Layout::of([(i64::MAX, 1)], …)` and `Layout::of([(0, i64::MAX), (i64::MIN, 0)], …)` produce violations and do not panic; a `verify`/`materialize` run on a bundle with the crafted index returns a failed A11/A18 row rather than aborting; A20 with two `i64::MAX` FETCH entries fails; a capture from a manifest with `compressed_size: -1` writes a non-negative `download_bytes`.

---

### K2 — An undecodable body is kept on disk but not in the index

**Severity: medium. Class: bug.**

The spec and the store doc promise a row for a body that is not a zstd frame:

```text
docs/00-spec.md:480-485
**A blob that was fetched is kept even when it will not decode.** The store
streams every fetched body to disk, digests it on the way past and records the
row. If the bytes are not a zstd frame the row carries `decode_error` with the
real reason and the two decode-dependent `verified` keys false, instead of being
discarded. A truncated or mis-served response is the evidence this tool exists
to keep.
```

```rust
// src/blobs/mod.rs:423-426
/// **An undecodable body is kept, not dropped.** The bytes are on disk before
/// they are ever measured, so a body that is not a zstd frame becomes a row
/// carrying its digests and the reason — rather than a hole, and rather than a
/// bundle claiming the fetch failed when it succeeded.
```

The implementation does the opposite after the atomic rename:

```rust
// src/blobs/mod.rs:478-492
if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
    Self::discard_tmp(&tmp_path);
    return Err(format!(...));
}

let path = final_path;
let props =
    tokio::task::spawn_blocking(move || BlobProps::measure_file(&path, &chunk, digests))
        .await
        .map_err(|e| format!("{id}: measure task panicked: {e}"))?
        .map_err(|e| e.to_string())?;
```

`measure_file` refuses an unframed body:

```rust
// src/blobs/props.rs:243-245
if !hash::is_zstd(&head[..n]) {
    return Err(Self::not_a_frame(&what, &head[..n]));
}
```

so the `?` returns an `Err` from `stream_from` *after* the blob is on disk. `Store::one` treats that exactly like a failed fetch and moves to the next source; if none succeeds, `Store::fill` records that slot with `unfilled`:

```rust
// src/blobs/mod.rs:396-399
fn unfilled(&self, want: &Want, why: &str) -> (IndexedBlob, Vec<Set>) {
    let mut props = BlobProps::unmeasured(&want.chunk);
    props.decode_error = Some(why.to_string());
    indexed_from(props, want)
}
```

That row has `stored_path: null`, `bytes: 0`, empty `md5_compressed` and `sha256_compressed`, and no measurement, while `chunks/blobs/<id>.zst` holds the mis-served body. The constructor that was written for this case:

```rust
// src/blobs/props.rs:291
pub fn undecodable(raw: &[u8], chunk: &SophonChunk, why: String) -> BlobProps {
```

is referenced only by its own tests (`src/blobs/props.rs:470`, `:509`); no production call site exists. `rg undecodable tools/capture/src` returns the definition and those tests only.

**Concrete failure scenario.** The test fixture in `src/blobs/props.rs:440-455` already models the bytes a real mis-served response carries: a 200 response whose body is an XML `NoSuchKey` document. `stream_from` writes it, renames it to `chunks/blobs/<id>.zst`, then `measure_file` returns `not a frame`. The row recorded for that slot says the fetch failed and carries `"bytes": 0` plus empty digests; `checksums.json`'s close-out walk will still list the orphan file, so the bundle contains a blob that `chunks/index.json` says is not stored and a reader cannot check. If a second source then serves a good body, the rename replaces the first file and the mis-served bytes disappear entirely, taking the evidence with them. The spec calls this body "the evidence this tool exists to keep".

There is a second, smaller contradiction in the same row shape: the spec says the two decode-dependent `verified` keys are `false`, while `undecodable` deliberately sets `verified: None` and the row's doc explains why (`src/blobs/props.rs:287-290`). Whichever is intended, the spec and the constructor currently disagree; the H4 decision (reconcile the document and the constructor) applies.

**Suggested fix.** After the rename, treat `measure_file`'s `not a frame` refusal as a measured-but-undecodable row rather than an `Err`: return `Ok` from `stream_from` with a `BlobProps` built from the already-computed `hash::Digests` (add a constructor taking them, or refactor `BlobProps::undecodable` to accept them), so `one` calls `set_stored` and the row carries `stored_path`, `bytes`, both compressed digests, and `decode_error`. Keep the multi-source fallback, but do not lose the first body silently: either keep it at a distinct final path and record both attempts, or at least record the first source's refusal in the row/log. Align the spec and `undecodable` on `verified` (`None` is the honest "not measured" reading; if that is the decision, the spec's "false" is the stale half). Add a test through `Store::one`/`stream_from` with a mock serving a non-frame body, asserting `bytes == body.len()`, both compressed digests, `decode_error` contains `not a zstd frame`, and `stored_path == Some("chunks/blobs/<id>.zst")`.

---

## Verification

Run on this tree, in this order, after the review reads and the reproduction:

| Command | Result |
|---|---|
| `cargo build -p gircap` | `Finished dev profile ... in 0.46s`; no warnings |
| `cargo test -p gircap` | **671 passed, 0 failed** (528 lib + 143 integration: 3 `bundle_consistency`, 8 `bundle_diff`, 32 `capture_layout`, 2 `case_coverage`, 30 `cli_contract`, 10 `matrix`, 29 `measured_fixture`, 14 `offline_bundle`, 15 `schema`); doc-tests 0 |
| `cargo clippy -p gircap --all-targets -- -D warnings` | `Finished dev profile ... in 24.02s`; no diagnostics |
| `cargo fmt -p gircap --check` | exit 0, no output |
| `git status --short --branch` (before this note) | `## fix-gaps/gircap` only; tree clean |

**Executed reproduction for K1.** A throwaway crate outside the repository, at `E:\Symlinks\Temp\opencode\gircap_repro`, with `gircap = { path = "E:/hoccode/hocrust/girpr/tools/capture" }` and `serde_json = "1"`. Its `main` deserialized a `gircap::blobs::ChunkIndex` with one blob (`uncompressed_size: 1`) and one `referenced_by` entry at `offset: 9223372036854775807`, then called the production `verify::materialize::rebuild` with `ChunkSource::from_memory(Default::default())`. Command:

```text
$env:CARGO_TARGET_DIR='E:\hoccode\hocrust\girpr\target'; cargo run --quiet --manifest-path 'E:\Symlinks\Temp\opencode\gircap_repro\Cargo.toml'
```

Result:

```text
thread 'main' (12052) panicked at E:\hoccode\hocrust\girpr\tools\capture\src\assemble\ranges.rs:48:28:
attempt to add with overflow
```

The reproduction crate is outside the repository and is not part of this commit.

## Not verified

- **A20 release wrap end to end.** The `-2` example needs a bundle that closes (`rebuild_all` succeeds) and a hand-edited patchmap; only the `.sum::<i64>()` arithmetic and the source path were verified, not an executed `gircap verify` on such a bundle.
- **Capture-side sums.** The `build_map` (`src/assemble/mod.rs:1123`), `projected_download` (`src/blobs/mod.rs:688`) and `diff.rs` panics were source-read, not executed against a synthetic manifest.
- **`Range::to` / `Range::len` overflow.** The `(i64::MIN, 0)` example was reasoned from the code, not run separately; the `Range::at` panic was the executed one.
- **K2 end to end.** No `Store`/mock run was made with a non-frame body; the path is source-verified against `measure_file`, `stream_from`, `one` and `unfilled`. `BlobProps::undecodable` has unit tests, but none drives the store.
- **Real inputs.** Whether the live CDN ever serves a non-frame chunk, or a live manifest ever declares `offset`/`compressed_size` near `i64::MAX` or negative, is unmeasured; the review treats the bundle documents as untrusted regardless.
- **The orphan's checksums row.** The claim that `checksums.json` lists the orphan while the index says `stored_path: null` follows the close-out walk and the writer, but no full bundle with an orphan was produced and inspected.
