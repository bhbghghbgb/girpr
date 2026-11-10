# Panic-site audit for `girpr`

**Status: complete (2026-10-30, re-swept 2026-11-09).** Audit of the tree at
`1f4c3b2` plus the changes in this branch.

**Scope:** the manifest, parse, plan, scan and write paths — the places where a
value chosen by the server (or by a hostile local manifest) reaches arithmetic
(the `saturating_*`/`checked_*` helpers, `+=`/`-=` updates), indexing,
integer-width `as` casts, `unwrap`/`expect`, or `panic!`/`unreachable!`. Local
CLI parsing, logging and test-only code are out of scope unless they feed one
of those paths.

**Rule:** no `catch_unwind`. A server-derived value that can panic is converted
into a refusal (a per-file `anyhow::Error`, usually exit `2` metadata or exit `3`
write) or bounded with saturating arithmetic. A value that is a *local*
invariant stays an `unwrap`/`expect`/direct index, with the invariant named.

---

## 1. Method

1. Enumerated every `.unwrap()`, `.expect()`, `panic!`, `unreachable!`, direct
   `[]` index or slice, and arithmetic/integer-width `as` conversion in the
   production regions of:
   `src/sophon/{stream,proto,map,filter}.rs`, `src/hyp.rs`,
   `src/util/{path,ini,md5,atomic}.rs`,
   `src/repair/phases/{plan,check,manifest,metadata,post}.rs`,
   `src/repair/{scan,reuse,purge,audio,lock,mod}.rs`,
   `src/repair/file/{mod,temp,inplace,mode}.rs`,
   `src/repair/scope/{report,limits,ledger,filter}.rs`.
2. For each site, traced where the value comes from:
   - **server** — a protobuf field, a `getBuild`/`getGameConfigs` JSON field, a
     URL/prefix, or a path in a manifest;
   - **local state** — a CLI flag, a local file, or a process counter;
   - **invariant** — a length or index established by the same function/build.
3. Classified each as **guarded** (checked/refused), **saturated** (bounded
   arithmetic), or **invariant** (no server input can break it), and named the
   test that pins the behaviour.
4. `rg` checklist used for the sweep, re-run on 2026-11-09 against the current
   tree:

   ```
   rg -n '\.unwrap\(\)|\.expect\(|panic!|unreachable!|\[[a-zA-Z_][a-zA-Z0-9_.]*\]|\[[^\]]*\.\.[^\]]*\]| as (u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize)|saturating|checked_| \+= | -= ' src
   ```

   followed by per-file reads of the production (pre-`#[cfg(test)]`) region.

In those regions, `audio.rs` and `lock.rs` are on the plan/write paths: the
former has no production `unwrap`/`expect`/`panic!`, and the latter's only
direct index is the bounded `head[..read]` in `may_write_body` (listed below).

---

## 2. Sites and dispositions

Sites are named by file and symbol rather than by line number, so the table
survives the module splits. "Origin" says whether the value is
server-controlled. A row is either a panic-capable site or a non-panicking
clamp the same sweep surfaced (`max_chunk_bytes`, for example); "Disposition"
says which.

| Site | Value | Origin | Disposition |
|---|---|---|---|
| `src/sophon/stream.rs` `manifest_size_hint` | manifest `uncompressed_size` | server | **Guarded**: `clamp(0, 64 MiB)` before `Vec::with_capacity`, so a bogus size costs a reallocation, never an allocation panic. |
| `src/sophon/stream.rs` `try_fetch_manifest` (protobuf decode) | manifest bytes | server | **Guarded**: `decode` returns `Result`; malformed bytes are a retry/failure, not a panic. |
| `src/sophon/stream.rs` `HashingWriter::write` | network body length | server | `written.saturating_add`, `&buf[..n]` with `n` from the inner `Write` contract — **saturating + invariant**. |
| `src/sophon/stream.rs` `stream_body` (compressed-body sum) | network bytes | server | **Saturating**. |
| `src/hyp.rs` `num_from_string` | JSON number/string sizes | server | **Guarded**: `visit_u64` uses `i64::try_from`, so a value above `i64::MAX` is a deserialization error instead of a wrapped negative; the string shape parses as `i64` and rejects the same range. Negatives are refused by `validate_chunk_ranges` or clamped with `max(0)`. |
| `src/repair/phases/plan.rs` `validate_chunk_ranges` | chunk `offset`, `uncompressed_size`, file `size` | server | **Guarded**: negative/overflow/past-size are `Err` → metadata exit `2`. `checked_add` guards the addition. |
| `src/repair/phases/plan.rs` `PlannedFile::layout` | chunk offsets/lengths | server | **Invariant**: runs after `validate_chunk_ranges` in `build_plan`; both operands are non-negative `i64`, so their `u64` sum is at most `2^64-2` and cannot overflow. |
| `src/repair/phases/plan.rs` `est_file_download_bytes` | chunk `compressed_size` | server | **Saturating**. |
| `src/repair/phases/manifest.rs` `ManifestFetches::next_ordered` and `OrderedResults::push` | manifest result stream and selection index | local | **Invariant**: the stream is built from `manifests.clone().into_iter().enumerate()`, so it yields exactly one result per selected index; `has_next` stops at `slots.len()`, `manifests[index]` is that same selection, and `push` writes `slots[index]` for that same index. |
| `src/repair/reuse.rs` `survives` | local-manifest offset/size | server (local manifest) | **Guarded**: `max(0)` then `checked_add`; a source past the file is not live. `live[i]` indexes a vector built with `chunks.len()` — **invariant**. |
| `src/repair/scan.rs` `all_chunks_download_bytes` and `est_file_download_bytes` | chunk `compressed_size` | server | **Saturating**. |
| `src/repair/scan.rs` `scan_plan`; `src/repair/file/inplace.rs` `attempt_inplace_inner` and `patch_chunks` | `dest_ok[i]`, `source_ok[i]`, `sources[i]`, `captured[i]`, `bad[k]` | server manifest/plan | **Invariant**: every vector is built from `file.chunks.len()` or from `dest_ranges`/`source_ranges` over the same chunk list in the same function; `bad` is a subset of valid indices. |
| `src/util/md5.rs` `scan_reader` | range offset/length | server (plan/source ranges) | **Guarded/invariant**: ranges are interned per chunk; `offset + len ≤ 2^64-2` because both originate as non-negative `i64`; `[a..b]` is bounded by the block read (`b ≤ n ≤ buf.len()`); `order` indices come from `0..ranges.len()`. |
| `src/util/md5.rs` `read_range` | chunk size | server | Allocation is sized by the chunk. It is only called for a staged source **after** the scan proved the full range intact (`source_ok`), so a merely-declared huge size cannot reach it. **Invariant by gate**. |
| `src/repair/file/temp.rs` `chunk_end` | chunk `offset + uncompressed_size` | server | **Guarded**: `u64::try_from` + `checked_add`; failure is a per-file error. |
| `src/repair/file/temp.rs` `try_reuse_slice` (reuse copy) | offset/size casts after the gate | server | **Guarded**: casts are after the checked gate; `remaining -= n` is bounded by the `min` and the read. |
| `src/repair/file/mod.rs` `copy_within`/`zero_fill` | chunk length/offset | server | **Invariant**: offsets stay within `i64` range, so `src/dst + done` cannot overflow `u64`; each loop step is bounded by `COPY_BUF` and a successful read/write. |
| `src/repair/mod.rs` `publish_plan` (`disk.as_ref().expect`) | disk plan for the report | local | **Invariant**: `disk` is `Some` exactly when `--print-plan` or `--plan-out` is set, the same condition that builds it in this function; each `expect` is reached only under that condition. |
| `src/repair/lock.rs` `may_write_body` (`head[..read]`) | lock-file body prefix | local | **Invariant**: `read` is the count returned by `File::read` into the 8-byte `head`, so `read <= head.len()`; the slice is bounded by the read. |
| `src/repair/scope/report.rs` `DiskPlan::new` | file `size`, `compressed_size` | server | **Saturating** (`est_write_bytes`, `est_extra_bytes`, `saturating_sub`). |
| `src/repair/scope/report.rs` `PlanDocument::new` (`disk.for_id(...).expect`) | disk-plan entry for each planned file | local | **Invariant**: `DiskPlan::new` pushes exactly one entry per `plan.iter()` in plan order; `debug_assert_eq!` pins the length, and the release `expect` refuses a hand-built plan that breaks coverage instead of shifting another file's figures. |
| `src/repair/scope/report.rs` `PlanTotals::new` (`max_chunk_bytes`) | chunk `compressed_size` | server | **Guarded**: `max(0)` before the `u64` cast; `max()`/`unwrap_or(0)` is total, so a corrupt size cannot panic. |
| `src/repair/file/inplace.rs` `staged_payload` | chunk `uncompressed_size` | server | **Saturating**, so a corrupt sum falls back to the ordered tier instead of bypassing the RAM budget. |
| `src/repair/mod.rs` `estimate_download` fold | per-file estimates | server-derived | **Saturating**. |
| `src/repair/purge.rs` `finish_plan` (`total_bytes`) | on-disk sizes | local | **Saturating**; `apply_purge` counters are local file sizes, not server values. |
| `src/repair/scope/ledger.rs` `Meter` counters | counters | local | **Saturating compare-and-swap**. |

---

## 3. Findings

**No server-derived panic site was found that is not already refused or bounded.**
Every server field that reaches arithmetic is either checked at plan time
(`offset`/`size`/range overflow), clamped (`manifest_size_hint`), or saturated
(every sum that can exceed `u64`). Every direct index is into a vector whose
length is established in the same function or the same plan, not by the server.

One observation from the first sweep is now closed: the `u64 as i64`
conversion in `num_from_string` used to wrap values above `i64::MAX`. It now
uses `i64::try_from`, so an out-of-range number is a deserialization error
rather than a wrapped negative size. Values that fit but are negative are still
refused by the plan or clamped with `max(0)`, so the outcome remains a refused
file or a conservative estimate, never a write outside the install.

The plan-time range gate covers the temp-file overflow path, and the file-task
window is bounded by `join_file_tasks` without a semaphore permit to unwrap.

---

## 4. Tests that pin the refusals

Hostile-manifest and saturation evidence already in the suite, plus the
zero-length cases added with this audit:

- `repair::phases::plan::tests::malformed_chunk_ranges_are_refused_at_plan_time`
  — negative size/offset, `offset + size` overflow, range past file size.
- `repair::phases::plan::tests::a_zero_length_entry_declaring_a_chunk_is_refused_at_plan_time`
  — the zero-length/range intersection.
- `repair::phases::plan::tests::an_overflowing_download_estimate_saturates`.
- `repair::scan::tests::an_overflowing_estimate_saturates_instead_of_panicking`.
- `repair::scope::report::tests::an_overflowing_write_estimate_saturates_instead_of_panicking`.
- `repair::scope::report::tests::a_short_estimate_slice_falls_back_to_a_saturating_per_file_estimate`.
- `repair::file::inplace::tests::an_overflowing_staged_payload_saturates` and
  `the_staged_payload_boundary_saturates`.
- Integration: `a_hostile_chunk_range_refuses_every_read_only_mode_before_it_runs`
  and `a_hostile_manifest_saturates_the_plan_figures_instead_of_panicking` in
  `tests/repair_offline.rs`.
- Zero-length probes:
  `a_legitimate_empty_file_passes_every_mode_untouched` and
  `an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate`.

---

## 5. What this does not cover

- A fuzz target over manifest parsing/planning (the audit is a source sweep plus
  the hostile cases above, not exhaustive input generation).
- Non-`x86_64` `usize` widths: several casts assume a 64-bit host, the same
  assumption the release matrix already makes.
- Panics from third-party crates (`reqwest`, `zstd`, `prost`, `walkdir`) on
  hostile bodies; those are wrapped at their `Result` boundary, but their
  internals were not audited.
