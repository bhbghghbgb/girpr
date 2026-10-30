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

- [ ] B1 — `Arc<RepairPlan>` + index in `scan_plan`.
- [ ] C1 — per-file scratch buffers.
- [ ] C2 — one source handle in `try_reuse_slice`.
- [ ] B2 / C4 — borrowed chunks in `salvage_order`.
- [ ] G1 — byte-digest compare in `RangeState::is_ok`.
- [ ] D3 — cheap tiling check in `build_plan`.
- [ ] D4 — `Arc<str>` URL prefixes.
- [ ] E1 — one `is_visible` per purge file.
- [ ] D2 — key the expected set directly in `plan_purge`.

### Phase 2 — behavior-adjacent

- [ ] A1 — one chunk-identity predicate, normalized MD5.
- [ ] A2 — indexed per-file reuse entries.
- [ ] B3 — shared scan-range construction; fix the in-place offset-0 sources.
- [ ] D5 — deduplicate `verify_plan`'s branches.
- [ ] F1 / F2 — `Ceiling` newtype + `Extras` builder.
- [ ] G6 — single-pass `clean_rel`.
- [ ] G5 — `PathRoot` cached base resolution.
- [ ] C5 — `OsString` `_tmp` path.
- [ ] C6 — bounded `JoinSet` in `repair_all`.
- [ ] H2 — `apply_run_flags`.

### Phase 3 — structural / broad API

- [ ] H1 — split `repair::run` into plan/execute (mechanical move first).
- [ ] G2 — `Md5` digest newtype across the gates (separate series).
- [ ] H4 — `RunMode` enum (optional, after H1).

## Pending review tasks

- D1 — `ea2acce` · `path_key` in `src/util/path.rs` replaces `dedup_key` / `purge_key`; `pub(crate)`, not re-exported from `util`. Behavior claimed identical; verify both pinned case-folding tests and that no doc references the old names.
- D6 — `38bd295` · one-pass `retain` + count in `build_plan`; confirm validation still precedes the drop and the `empty` warning fields/text are unchanged.
- G7 — `5fe3a99` · `read_range` now `usize::try_from` + `try_reserve_exact` with a new refusal test; `md5_file_slice` buffer is plain `MD5_BUF` and its inner imports are gone. Reviewer note: `md5_file_slice` still does `remaining as usize` (truncation on 32-bit), left out of scope.
- C3 — `8d3e4c1` · `repaired_in_place` helper used by both success paths in `src/repair/file/inplace.rs`; check the seven log fields and their order match the old inline blocks exactly.
- C7 — `0fd4ae1` · `write_reserved` shared by `write_at` / `copy_within` / `zero_fill`; verify reservation granularity and `note_write` order are unchanged (note: `zero_fill` now seeks per piece).
- G3 — `ef47fb9` · `OnceLock<Regex>` in `src/util/ini.rs` via `game_version_re()`, plus a new anchored-shape test; verify last-match-wins and the absent-vs-unreadable split are untouched.
- G4 — `655301d` · `writeln!` serializer + single `read_to_string` with `NotFound => empty` in `src/util/ini.rs`. Reviewer note: an unreadable-but-existing config path now errors instead of being treated as empty (findings-sanctioned); confirm that is acceptable.
- F3 — `34792e0` · `Cow` bare values and push-based JSON in `src/report/record.rs`, incl. a new `push_json_event`; verify text/JSON output is byte-identical (UInt vs Int, escaping).
- A3 — `8d05154` · `survives_one`/`survives_count` + single winning-vector materialization in `src/repair/reuse.rs`; verify tie-goes-to-ascending and the `live` vectors are unchanged by the existing tests.
- H3 — `eb418bb` · `note_stopped` in `src/repair/mod.rs` with a first-wins guard, used by the runtime-stop branch and `finish`, plus a new unit test. Reviewer note: the overwrite guard is currently latent (a runtime stop returns exit 5 before `finish`), so it only documents intent.

## Skipped

_None yet._
