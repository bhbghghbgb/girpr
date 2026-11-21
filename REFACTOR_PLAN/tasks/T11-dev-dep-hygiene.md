# T11 — Root dev-dependency hygiene and frozen manifest bodies

## Goal / pointers
- `docs/10-test-optimization-research.md` §F9 (lines 221–227) — root's `prost` dev-dep is
  **used** by unit tests in `src/repair/phases/manifest.rs`; it can only be dropped after
  those fixtures are frozen as bytes. Tokio's root `test-util` feature appears unused;
  verify with `cargo tree -e dev -p girpr` before removing.
- `docs/10-test-optimization-research.md` §F14 (lines 264–269) — `manifest_body`
  re-encodes with prost + zstd per test; only three distinct deterministic bodies. Fix:
  freeze as `const &[u8]` with pinned checksums, then remove the root `prost`/`zstd`
  dev-deps.
- Assumptions to validate: §A5 (75–80), §A11 (112–116), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- Root `Cargo.toml` dev-dependencies (`prost`, `zstd`, tokio features),
  `src/repair/phases/manifest.rs` (`manifest_body` and its call sites), and the three
  frozen bodies. Confirm with `cargo tree -e dev -p girpr` in the current tree.

## Implementation plan
1. Validate §F9/§F14: list the three distinct deterministic bodies and their checksums,
   and confirm which dev-deps are genuinely removable after freezing.
2. Freeze the three bodies as byte constants with pinned checksum tests, keeping the
   same assertions the current tests make.
3. Remove the now-unused `prost`/`zstd` dev-deps and the unused tokio `test-util` feature
   if the dependency-tree check confirms it is unused.
4. Verify with `cargo tree -e dev -p girpr` and a clean test build.

## Constraints / validation
- Do **not** drop `prost` before freezing the bodies — §F9 explicitly corrects the
  earlier claim that it is unused. Byte contracts and checksums must be identical (§6).
- Fixtures are already tiny (§A5); do not resize them. No sleeps (§A6).
- A dev-dep removed from root may still be compiled for another member under feature
  unification (§A11) — verify before claiming a workspace win.

## Done when
The three bodies are frozen with pinned checksums, only genuinely unused dev-deps and
features are removed, the dependency tree is verified, and the suite passes with
unchanged bytes.
