# Panic-site audit for `girpr`

**Status: complete (2026-10-30).** Audit of the tree at `1f4c3b2` plus the §6
changes in this branch.

**Scope:** the manifest, parse, plan, scan and write paths — the places where a
value chosen by the server (or by a hostile local manifest) reaches arithmetic,
indexing, `unwrap` or `expect`. Local CLI parsing, logging and test-only code are
out of scope unless they feed one of those paths.

**Rule:** no `catch_unwind`. A server-derived value that can panic is converted
into a refusal (a per-file `anyhow::Error`, usually exit `2` metadata or exit `3`
write) or bounded with saturating arithmetic. A value that is a *local*
invariant stays an `unwrap`/`expect`/direct index, with the invariant named.

---

## 1. Method

1. Enumerated every `.unwrap()`, `.expect()`, direct `[]` index and
   arithmetic/`as` conversion in the production regions of:
   `src/sophon.rs`, `src/hyp.rs`, `src/util.rs`, `src/repair/phases/{plan,check,manifest,metadata,post}.rs`,
   `src/repair/{scan,reuse,purge,mod}.rs`,
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
4. `rg` checklist used for the sweep:

   ```
   rg -n '\.unwrap\(\)|\.expect\(|\[[a-zA-Z_][a-zA-Z0-9_.]*\]| as usize| as u64| as i64|saturating|checked_| \+= | -= ' src
   ```

   followed by per-file reads of the production (pre-`#[cfg(test)]`) region.

---

## 2. Sites and dispositions

Line numbers are the audit tree's. "Origin" says whether the value is
server-controlled.

| Site | Value | Origin | Disposition |
|---|---|---|---|
| `src/sophon.rs:159-161` `manifest_size_hint` | manifest `uncompressed_size` | server | **Guarded**: `clamp(0, 64 MiB)` before `Vec::with_capacity`, so a bogus size costs a reallocation, never an allocation panic. |
| `src/sophon.rs:210-219` protobuf decode | manifest bytes | server | **Guarded**: `decode` returns `Result`; malformed bytes are a retry/failure, not a panic. |
| `src/sophon.rs:296-313` `HashingWriter::write` | network body length | server | `written.saturating_add`, `&buf[..n]` with `n` from the inner `Write` contract — **saturating + invariant**. |
| `src/sophon.rs:563` compressed-body sum | network bytes | server | **Saturating**. |
| `src/hyp.rs:120-145` `num_from_string` | JSON number/string sizes | server | `u64 as i64` **wraps** for values above `i64::MAX` but does not panic; negatives are refused by `validate_chunk_ranges` or clamped with `max(0)`. Non-panic, recorded rather than changed. |
| `src/repair/phases/plan.rs:195-227` `validate_chunk_ranges` | chunk `offset`, `uncompressed_size`, file `size` | server | **Guarded**: negative/overflow/past-size are `Err` → metadata exit `2`. `checked_add` guards the addition. |
| `src/repair/phases/plan.rs:57-102` `layout` | chunk offsets/lengths | server | **Invariant**: runs after `validate_chunk_ranges` in `build_plan`; both operands are non-negative `i64`, so their `u64` sum is at most `2^64-2` and cannot overflow. |
| `src/repair/phases/plan.rs:166-181` `est_file_download_bytes` | chunk `compressed_size` | server | **Saturating**. |
| `src/repair/reuse.rs:114-138` `survives` | local-manifest offset/size | server (local manifest) | **Guarded**: `max(0)` then `checked_add`; a source past the file is not live. `live[i]` indexes a vector built with `chunks.len()` — **invariant**. |
| `src/repair/scan.rs:128-132,159-196` estimates | chunk `compressed_size` | server | **Saturating**. |
| `src/repair/scan.rs:167-193`; `src/repair/file/inplace.rs:167-170,239-300,349-366` | `dest_ok[i]`, `source_ok[i]`, `sources[i]`, `captured[i]`, `bad[k]` | server manifest/plan | **Invariant**: every vector is built from `file.chunks.len()` or from `dest_ranges`/`source_ranges` over the same chunk list in the same function; `bad` is a subset of valid indices. |
| `src/util.rs:118-188` `scan_reader` | range offset/length | server (plan/source ranges) | **Guarded/invariant**: ranges are interned per chunk; `offset + len ≤ 2^64-2` because both originate as non-negative `i64`; `[a..b]` is bounded by the block read (`b ≤ n ≤ buf.len()`); `order` indices come from `0..ranges.len()`. |
| `src/util.rs:225-230` `read_range` | chunk size | server | Allocation is sized by the chunk. It is only called for a staged source **after** the scan proved the full range intact (`source_ok`), so a merely-declared huge size cannot reach it. **Invariant by gate**. |
| `src/repair/file/temp.rs:237-246` `try_reuse_slice` | chunk `offset + uncompressed_size` | server | **Guarded**: `u64::try_from` + `checked_add`; failure is a per-file error. |
| `src/repair/file/temp.rs:261-295` reuse copy | offset/size casts after the gate | server | **Guarded**: casts are after the checked gate; `remaining -= n` is bounded by the `min` and the read. |
| `src/repair/file/mod.rs:613-652` `copy_within`/`zero_fill` | chunk length/offset | server | **Invariant**: offsets stay within `i64` range, so `src/dst + done` cannot overflow `u64`; each loop step is bounded by `COPY_BUF` and a successful read/write. |
| `src/repair/file/mod.rs:215,324,341` | semaphore permit; `disk.as_ref()` | local | **Invariant**: the semaphore is never closed (`N-12`), and `disk` is `Some` exactly when the branch that expects it ran. |
| `src/repair/scope/report.rs:95-126` | file `size`, `compressed_size` | server | **Saturating** (`est_write_bytes`, `est_extra_bytes`, `saturating_sub`). |
| `src/repair/file/inplace.rs:478-488` `staged_payload` | chunk `uncompressed_size` | server | **Saturating**, so a corrupt sum falls back to the ordered tier instead of bypassing the RAM budget. |
| `src/repair/mod.rs:276-278` estimate fold | per-file estimates | server-derived | **Saturating**. |
| `src/repair/purge.rs:199` `total_bytes` | on-disk sizes | local | **Saturating**; `apply_purge` counters are local file sizes, not server values. |
| `src/repair/scope/ledger.rs:200,254,457` | counters | local | **Saturating compare-and-swap**. |
| `src/util.rs:410` | `.chars().next().unwrap()` | local | **Invariant**: guarded by `n.len() == 1`. |
| `src/util.rs:500` | regex literal `.expect` | local | **Invariant**: the pattern is a compile-time literal. |

---

## 3. Findings

**No server-derived panic site was found that is not already refused or bounded.**
Every server field that reaches arithmetic is either checked at plan time
(`offset`/`size`/range overflow), clamped (`manifest_size_hint`), or saturated
(every sum that can exceed `u64`). Every direct index is into a vector whose
length is established in the same function or the same plan, not by the server.

One **non-panic** observation remains, deliberately not changed: the
`u64 as i64` conversion in `src/hyp.rs:135` wraps values above `i64::MAX`. The
wrapped value is negative, and the plan refuses negative sizes and clamps
non-negative fields with `max(0)`, so the outcome is a refused file or a
conservative estimate, not a panic or a write outside the install. Fixing the
cast would change the JSON acceptance boundary for a shape no capture has
produced; it is recorded here instead.

The plan-time range gate covers the temp-file overflow path, and the
file-semaphore `unwrap` is the named `expect` above.

---

## 4. Tests that pin the refusals

Hostile-manifest and saturation evidence already in the suite, plus the §6.1
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
- §6.1 zero-length probes:
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
