# girpr refactor implementation state

Source of tasks: `REFACTOR_FINDINGS.md`. Checklist order follows the findings'
"Suggested implementation order" (low risk → high risk).

Status legend: `[ ]` pending · `[~]` in progress · `[x]` done · `[-]` skipped.

## Task checklist

### Phase 0 — pure cleanup (no behavior change)

- [x] D1 — one `case_key` helper. (`ea2acce`, shared `util::path::path_key`)
- [ ] D6 — one-pass empty-chunk count.
- [ ] G7 — remove dead imports / dead `min`; add `read_range` bounds.
- [ ] C3 — extract the in-place success block.
- [ ] C7 — extract the reserved-write helper.
- [ ] G3 — `OnceLock` the version regex.
- [ ] G4 — direct `writeln!` into the config buffer.
- [ ] F3 — allocation-free record rendering.
- [ ] A3 — count-then-materialize salvage order.
- [ ] H3 — one `stopped` setter.

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

## Skipped

_None yet._
