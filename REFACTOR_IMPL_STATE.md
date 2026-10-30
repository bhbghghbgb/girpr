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
- [x] C6 — bounded `JoinSet` in `repair_all`. (`a7e9703`)
- [ ] H2 — `apply_run_flags`.

### Phase 3 — structural / broad API

- [ ] H1 — split `repair::run` into plan/execute (mechanical move first).
- [ ] G2 — `Md5` digest newtype across the gates (separate series).
- [ ] H4 — `RunMode` enum (optional, after H1).

## Pending review tasks

_None._ D2, every Phase 2 task (A1, A2, B3, D5, F1/F2, G6, G5, C5, C6) and the
review follow-ups (`2e02df7`, `7ab4ff5`, `edfb9b2`, `87807bb`, `e20ca5b`,
`edbb471`) passed review; H2 and the Phase 3 tasks are not implemented yet.

D2 and Phase 2 (A1, A2, B3, D5, F1/F2, G6, G5, C5, C6) passed review:
`cargo test --all-targets` (225 unit + 149 integration passing, 7 ignored),
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean,
and each task's contract was checked against its pre-refactor code:

- D2 — `expected_rels()` is the allocation-free view and `expected_paths()`
  still hands back the raw spelling (the tests' entry point). `plan_purge` keys
  the plan's rels straight into its one expected set, so the keep set is still
  exactly the plan's cleaned rels and `classify_purge_path`'s "sets are already
  keyed" contract is untouched.
- A1 — `ReuseEntry::matches` is the only `(md5, size)` predicate, and the map
  normalizes the digest once at its boundary. First-entry-wins is preserved
  (the two existing tests) and the new test pins case-variant collapse.
- A2 — `FileReuse` keeps `entries` in first-match order and answers from a
  bucket index keyed by chunk identity that never re-orders insertions, so
  `get` cannot change the winner; a missing entry still degrades to a download.
- B3 — `scan_ranges` is the one range builder. In the in-place path
  `source_ok` is remapped to one verdict per chunk (discard path included), so
  no consumer sees a per-range vector, and a source-less chunk is no longer
  hashed at offset 0. `Scan::absent(dest.len(), source.len())` is unchanged.
- D5 — the failure shapes, field names and counter updates are the
  pre-refactor ones; `damaged_chunks()` is computed once, and presence/length
  are still checked before the digest for both digest sources. The fallback
  "unreadable" text is byte-identical (`{e:#}` formatted into the message).
- F1 / F2 — `Summary::default()`, `new()` and the old hand-written `new()`
  agree field for field (`ram_limit` still `u64::MAX`), `snapshot` still copies
  the ceiling and the stop marker, and `Extras` pushes the same keys in the
  same order.
- G6 — the component walk is shared; accepted/rejected spellings and the
  `/`-separated output are unchanged, and the device-name check is
  allocation-free and byte-based (no character-boundary slicing).
- G5 — `PathRoot` caches only the base resolution; each joined path is still
  resolved on the filesystem and compared with it, `contained_join` keeps its
  message and its lexical-first ordering, and the execute phase rebuilds the
  root after `create_dir_all`.
- C5 — the `_tmp` suffix is appended to the `OsString`; the suffix and the
  purge rule are unchanged, and UTF-8 paths are byte-identical.
- C6 — the in-flight window is the bound, `JoinError` accounting and the
  completion-order drain are kept, and `--fail-fast` is checked before every
  spawn. The three concerns recorded with the commit are answered below.

C6 review concerns:

1. `task = ?tokio::task::try_id()` still names the **file** task: the span is
   built in `file_task`'s body, and an `async fn` body runs on the spawned
   task, so the field is unchanged. The note assumed a driver-built span.
2. `--fail-fast` is now deterministic: draining the window in completion order
   means no file starts after an observed failure. That is the documented
   contract ("no further file is started"); the `remaining` warning keeps its
   text and still fires at most once.
3. A cancelled `repair_all` now aborts the in-flight tasks (the `JoinSet`
   drop) instead of detaching them. That is the safer outcome — no writer keeps
   running after the run returned — and both write paths recover: the temp path
   resumes by length, the in-place path re-scans.

Non-blocking follow-ups from this review (addressed and accepted):

- B3 — in-place tests for the source-less chunk and the discard remap.
  (`2e02df7`; `source_ok_by_chunk` extracted, +2 unit tests)
- C6 — dropped-driver abort documented in the `file` module docs. (`7ab4ff5`)
- A2 — allocation-free reuse lookup. (`edfb9b2`; bucket by streaming hash of
  the lowercased digest + size, candidates confirmed with `matches`, +2 unit
  tests)
- G5 — `verify_plan` takes the execute-phase `PathRoot`. (`87807bb`)
- F1 — `Ceiling::load`/`store` are `pub`. (`e20ca5b`)
- Phase 1 leftovers — `md5_file_slice_with_buf` folded away (`edbb471`), and
  `PlannedFile::is_tiled` narrowed to `pub(crate)` (same commit).

The follow-ups passed review at `20a47ad`
(`cargo test --all-targets`: 229 unit + 149 integration passing, 7 ignored;
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean):

- `2e02df7` — the extracted remap is the same code that ran inline. The
  source-less test builds the ranges through `scan_ranges` (empty source list,
  empty owner) and shows the head-of-file copy would otherwise verify; the
  discard test pins `Scan::absent`'s per-range tuple and that a `true` verdict
  lands on its owner, so the remap cannot pass by keeping everything false.
- `7ab4ff5` — the doc states the `JoinSet` drop semantics that C6 introduced;
  no behavior change.
- `edfb9b2` — the bucket key is a streaming hash of the ASCII-lowercased
  digest plus size, so any identity-equal entry hashes alike (`eq_ignore_ascii_case`
  and `to_ascii_lowercase` fold exactly the same bytes) and no lookup is
  missed. Buckets keep insertion order and every candidate is confirmed with
  `matches`, so first-match-wins and collision safety both survive; the dedup
  path confirms the same way. The hash is process-local (the map is built and
  used in one run and nothing serializes it), so `DefaultHasher`'s
  unspecified-across-releases algorithm is not load-bearing.
- `87807bb` — `verify_plan` takes the execute-phase root and keeps the
  `contained_join` fallback, so a `--check-only` run no longer resolves the
  base once per file; containment is still resolved per joined path.
- `e20ca5b` — `Ceiling::load`/`store` are `pub`, matching the public
  `ram_limit` field; the `u64::MAX` sentinel and the `Ordering` pass-through
  are unchanged.
- `edbb471` — `md5_file_slice_with_buf` folds into `md5_file_slice` (the reader
  still grows one `MD5_BUF`), so the one-off form is still one open, one
  buffer; `is_tiled` is `pub(crate)`. `md5_file_slice` now has no in-crate
  caller but remains public API.

Non-blocking follow-up: `md5_file_slice` is uncalled inside the crate; keep it
as public API or drop it with the G2 digest series.

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
Both addressed in `edbb471`.

Phase 0 (D1, D6, G7, C3, C7, G3, G4, F3, A3, H3) passed review:
`cargo test --all-targets` (216 unit + 149 integration passing, 7 ignored),
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean,
and the output contracts named in each task were checked against the
pre-refactor code.

## Skipped

_None yet._
