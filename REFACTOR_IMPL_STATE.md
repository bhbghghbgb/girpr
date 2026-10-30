# girpr refactor implementation state

Source of tasks: `REFACTOR_FINDINGS.md`. Checklist order follows the findings'
"Suggested implementation order" (low risk → high risk).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done · `[-]` skipped.

## Task checklist

### Phase 0 — pure cleanup (no behavior change)

- [x] D1 — one `case_key` helper. (`ea2acce`, shared `util::path::path_key`)
- [x] D6 — one-pass empty-chunk count. (`38bd295`)
- [x] G7 — remove dead imports / dead `min`; add `read_range` bounds. (`5fe3a99`)
- [x] C3 — extract the in-place success block. (`8d3e4c1`)
- [x] C7 — extract the reserved-write helper. (`0fd4ae1`)
- [x] G3 — `OnceLock` the version regex. (`ef47fb9`)
- [x] G4 — direct `writeln!` into the config buffer. (`655301d`)
- [x] F3 — allocation-free record rendering. (`34792e0`)
- [x] A3 — count-then-materialize salvage order. (`8d05154`)
- [x] H3 — one `stopped` setter. (`eb418bb`)

### Phase 1 — low-risk performance

- [x] B1 — `Arc<RepairPlan>` + index in `scan_plan`. (`5474064`)
- [x] C1 — per-file scratch buffers. (`0972167`)
- [x] C2 — one source handle in `try_reuse_slice`. (`200c829`)
- [x] B2 / C4 — borrowed chunks in `salvage_order`. (`58d9cf8`)
- [x] G1 — byte-digest compare in `RangeState::is_ok`. (`90746da`)
- [x] D3 — cheap tiling check in `build_plan`. (`fe69394`)
- [x] D4 — `Arc<str>` URL prefixes. (`6ee214b`)
- [x] E1 — one `is_visible` per purge file. (`c4da9d3`)
- [x] D2 — key the expected set directly in `plan_purge`. (`4cedc14`)

### Phase 2 — behavior-adjacent

- [x] A1 — one chunk-identity predicate, normalized MD5. (`28f5869`)
- [x] A2 — indexed per-file reuse entries. (`0fec9bc`)
- [x] B3 — shared scan-range construction; fix the in-place offset-0 sources. (`5e49a7b`)
- [x] D5 — deduplicate `verify_plan`'s branches. (`1783069`)
- [x] F1 / F2 — `Ceiling` newtype + `Extras` builder. (`70c13b3`)
- [x] G6 — single-pass `clean_rel`. (`80a2d5b`)
- [x] G5 — `PathRoot` cached base resolution. (`d5a8f2a`)
- [x] C5 — `OsString` `_tmp` path. (`a6bb886`)
- [ ] C6 — bounded `JoinSet` in `repair_all`.
- [ ] H2 — `apply_run_flags`.

### Phase 3 — structural / broad API

- [ ] H1 — split `repair::run` into plan/execute (mechanical move first).
- [ ] G2 — `Md5` digest newtype across the gates (separate series).
- [ ] H4 — `RunMode` enum (optional, after H1).

## Pending review tasks

- [ ] D2 — key the expected set directly in `plan_purge` (`4cedc14`; +1 unit
  test, 219). Last Phase 1 item; the rest of Phase 1 was accepted at
  `b03eb66`.
- [ ] A1 — one chunk-identity predicate, normalized MD5 (`28f5869`; +1 unit
  test, 220).
- [ ] A2 — indexed per-file reuse entries (`0fec9bc`).
- [ ] B3 — shared scan-range construction; in-place `source_ok` remapped per
  chunk (`5e49a7b`; correctness fix, no new test — existing integration
  guards cover it).
- [ ] D5 — deduplicate `verify_plan`'s branches (`1783069`; +2 unit tests,
  222). `DigestSource::Scanned(&Verdict)` and unit `Read` deviate slightly
  from the finding's sketch, to keep `damaged_chunks()` single-call and avoid
  reading an absent file.
- [ ] F1 / F2 — `Ceiling` newtype + `Extras` builder (`70c13b3`). `Ceiling` is
  `pub` (public field), otherwise as sketched; key order unchanged.
- [ ] G6 — single-pass `clean_rel` (`80a2d5b`; +1 table-driven test, 223).
  Validation order and error texts unchanged; device-name check is now
  allocation-free and byte-based.
- [ ] G5 — `PathRoot` cached base resolution (`d5a8f2a`; +1 unit test, 224).
  Wired into scan estimate/scan, `DiskPlan::new`, `repair_all`; refreshed after
  `create_dir_all`. `verify_plan` left on `contained_join` (not in the list).
  Purge never joined a rel.
- [ ] C5 — `OsString` `_tmp` path (`a6bb886`). No test change (no non-UTF-8
  harness); `_tmp` suffix and purge rule unchanged.

Phase 1 (B1, C1, C2, B2/C4, G1, D3, D4, E1) passed review:
`cargo test --all-targets` (218 unit + 149 integration passing, 7 ignored),
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean,
and each task's contract was checked against the pre-refactor code:

- B1 — scan tasks get an index and a cloned plan `Arc`; no `PlannedFile` clone.
- C1 — one scratch buffer per file; piece sizes and the write-ceiling charging
  points (`copy_within` per piece, `zero_fill` once for the whole length) are
  unchanged, and the zero block is read-only.
- C2 — one source handle; an open failure, a read failure, a digest mismatch
  and a short slice all still fall back to a download with the `_tmp`
  untouched.
- B2 / C4 — `salvage_order_refs` shares `order_from` with the owned form;
  `the_borrowed_form_agrees_with_the_owned_one` pins them together.
- G1 — the digest is parsed once into bytes; an empty or non-hex expectation
  is still "never matches" and case is still ignored.
- D3 — `is_tiled` gates `layout()`; the warning path is unchanged, and
  `the_cheap_tiling_check_agrees_with_the_layout` covers tiled/hole/overlap/
  tail/past-size/empty.
- D4 — one `Arc<str>` per manifest; the first manifest still wins on a
  duplicate rel.
- E1 — the scope is matched once; rule 0 stays first, and the `hidden` count
  and warning are unchanged.

Non-blocking follow-ups: `util::md5_file_slice_with_buf` (added by C1) lost
its only caller when C2 switched to `md5_reader_slice`, and `md5_file_slice`
now only wraps it — fold the buffer variant into `md5_file_slice` once it has
no second caller. `PlannedFile::is_tiled` relies on the plan's sorted chunk
order (documented); it can be `pub(crate)` until a second caller needs it.

Phase 0 (D1, D6, G7, C3, C7, G3, G4, F3, A3, H3) passed review:
`cargo test --all-targets` (216 unit + 149 integration passing, 7 ignored),
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean,
and the output contracts named in each task were checked against the
pre-refactor code.

## Skipped

_None yet._
