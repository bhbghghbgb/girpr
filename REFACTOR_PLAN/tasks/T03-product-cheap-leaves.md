# T03 — Cheapest product leaves batch

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F19 (lines 261–264) —
  `repair/lock.rs` (796), `repair/audio.rs` (346), `repair/scope/filter.rs`
  (196), `biz.rs` (6), `phase.rs` (143), `retry.rs` (281), `logging.rs` (400)
  are already effective leaves requiring no restructuring.
- Assumptions to validate: §A2 (27–33), §A3 (35–38), §A4 (40–46), §A10 (86–94).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- The files named above under `src/`. Note that `src/retry.rs` and
  `src/logging.rs` may be thin shims over the existing `girpr-retry` /
  `girpr-log` crates (§A3 lists 13 shared crates as existing facts) — validate
  before creating anything new.
- `src/lib.rs:111-125` (`pub mod test_support`) and the re-exports that make
  these leaves visible to integration tests.
- Locate each file and its `use crate::` edges in the current tree before editing.

## Implementation plan
1. For each candidate, validate it is truly a leaf (no product-internal
   outgoing deps) and identify a high-level consumer that can depend on the new
   crate directly (§A2). Group only where one crate can hold related leaves
   without coupling unrelated policies.
2. Prefer extending an existing shared crate (e.g. `girpr-retry`, `girpr-log`,
   `girpr-filter`) over creating a duplicate one; the prior extraction already
   produced 13 leaves (§A3, §F37).
3. Move the leaves behind narrow crate boundaries and have the product depend on
   them directly so edits elsewhere leave them cached. Do not keep the product
   facade as the sole owner of every target (§A2).
4. Update nextest filters pinned to `repair::lock::tests::…` and similar paths
   (§F31, `.config/nextest.toml:106-214`) when tests move.
5. If gircap gains a new leaf dependency, update the §A5/§F39 doc tables.

## Constraints / validation
- Lock policies, walkers and the `rw` gate are deliberate keepers (§A13); move
  the file, do not change the policy.
- Byte contracts unchanged (§A6).
- A leaf that cannot be reached directly by a high-level consumer is not a win
  under §A2 — leave it local and note why.
- Keep test-only seams and the shipped `pub mod test_support` working (§F22).

## Done when
The validated leaves live in crates that high-level product code depends on
directly, no policy or byte contract changed, and the lock/filter/audio/retry/
log tests plus pinned nextest filters pass.
