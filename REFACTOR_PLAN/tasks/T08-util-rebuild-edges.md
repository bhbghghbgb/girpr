# T08 — Narrow `girpr-util` rebuild edges

## Goal / pointers
- `docs/10-test-optimization-research.md` §F6 first bullet (lines 184–191) —
  `crates/util/src/lib.rs:24-28` declares five unrelated modules; `crates/audio`,
  `crates/fetch`, `crates/report` use disjoint slices and none uses `ini.rs` (~1,437
  lines), yet all rebuild on an `ini.rs` edit. Fix: split into narrow crates (path/io/
  hash/ini) with consumers depending directly; drop the re-export facade after
  migration. Keep existing test names until pinned nextest filters are updated.
- Related: §F10 (231–241) wants pure repair modules moved into leaves with a shared
  model home; that is the downstream consumer-side half of the same idea.
- Assumptions to validate: §A2 (54–59), §A3 (61–65), §A11 (112–116), §A10 (108–110),
  §6 (503–510). Guidance: `DO-NOT-TOUCH.md` (cycle/seam boundaries).

## Scope
- `crates/util/` (`ini.rs`, `path.rs`, `atomic.rs`, `md5.rs`, `counter.rs`, symlink/
  junction support), the facade, and the manifests of `girpr-audio`, `girpr-fetch`,
  `girpr-report` and every other direct consumer.
- Existing precedent from prior extraction work: a facade already aliases the util
  leaf (`use girpr_util as util`); verify the current shape before editing.

## Implementation plan
1. Validate §F6/§A3: map each consumer to the exact util modules it uses and confirm
   that an `ini.rs` edit actually rebuilds consumers that do not use it. Confirm the
   boundary is not blocked by a cycle before editing — prior restructuring attempts
   found cycle-constrained edges in this area; if the consumer set does not shrink,
   stop and record a skip.
2. Split the disjoint modules into narrow crates (e.g. path/io/hash/ini) and change
   consumers to depend directly on the pieces they use.
3. Remove or shrink the re-export facade once no consumer needs both halves through it;
   keep shipped API behavior and test names stable.
4. Update nextest filters if any crate or test moves (§A10), and doc tables/manifest
   comments listing crates.
5. Deliver in commits per narrow crate plus a final facade-removal commit (L effort).
   Verify the payoff empirically: an `ini.rs` edit must leave unrelated consumers
   Fresh, and an isolated `cargo nextest run -p <crate>` must show the win.

## Constraints / validation
- The central test is §A3: a facade that still re-exports both halves is not a win.
  If every consumer still reaches both through the facade, stop rather than produce a
  cosmetic split.
- Keep byte contracts and utils' behavior identical (§6); do not touch `md5.rs` ignored
  benchmarks or junction_support's command seam (F29).
- Do not move `#[cfg(test)]` tails without confirming a real rebuild edge shrinks
  (§A11); this is a turnaround win, not a workspace wall-clock win.
- Do not couple programs via a shared helper (gircap independence).

## Done when
Consumers depend directly on the narrow crates they use, the facade no longer forces
irrelevant rebuilds, an `ini.rs` edit leaves unrelated consumers Fresh, nextest filters
and doc tables are updated, and the full suite passes.
