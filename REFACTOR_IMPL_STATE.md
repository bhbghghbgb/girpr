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

- [ ] B1 — `Arc<RepairPlan>` + index in `scan_plan` (`5474064`).
- [ ] C1 — per-file scratch buffers (`0972167`).

Phase 0 (D1, D6, G7, C3, C7, G3, G4, F3, A3, H3) passed review:
`cargo test --all-targets` (216 unit + 149 integration passing, 7 ignored),
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean,
and the output contracts named in each task were checked against the
pre-refactor code.

## Skipped

_None yet._
