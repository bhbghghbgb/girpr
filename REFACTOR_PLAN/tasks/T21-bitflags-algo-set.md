# T21 — AlgoSet → bitflags

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-1 (lines 540–549) — the hand-written
  `AlgoSet(u16)` masks/count/iteration/difference become `bitflags`, keeping the public
  type and two-byte layout.
- §X4 (lines 127–134) — cross-cutting small mechanics theme.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/cache/src/hash/set.rs:7–96`; `cache/src/hash/algo.rs:103–113` (`bit()`).
- `planner/plan.rs:30–32`, `:110–118`; `planner/pairs.rs:271–355`.
- Tests `cache/src/hash/tests.rs:246–333`, the `size_of == 2` pin at `:916–919`, and the
  hand-written `Debug` pin at `:926–935`. `tools/sync/cache/Cargo.toml`.

## Implementation plan
1. Validate that no code relies on the private `u16` representation beyond size and
   bit layout. Use `bitflags! { impl AlgoSet: u16 { … } }` so the public type and
   two-byte layout stay.
2. Keep a custom `iter()` that yields `HashAlg::ALL` in canonical order and a custom
   `Debug` that preserves the `["md5", …]` log spelling; do not adopt bitflags' default
   formatting.
3. Preserve set-difference and `count_ones()` behavior at every use in
   `planner/plan.rs` and `planner/pairs.rs` (one value per path).
4. Keep all named tests, especially the size and Debug pins. `bitflags 2.13.2` is already
   locked; verify a resolved build.

## Constraints / validation
- Two-byte size, canonical iteration order and Debug spelling matter (§SYNC-1 lines
  547–549). No wire-format change is implied, but verify `postcard` rows are unchanged.
- `girpr-sync-cache` is a dev-tool leaf; keep its lint table and local policy.

## Done when
`AlgoSet` is a bitflags type with the same size/order/Debug output, all named tests pass,
and the planner behavior is unchanged. If a premise is stale or a keeper would change,
**stop** and record `skipped`.
