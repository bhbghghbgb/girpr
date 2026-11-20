# T06 — File identity + link-count leaf

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F9 (lines 185–193) — two near-copies
  of file identity and hard-link counting.
- Assumptions to validate: §A4 (37–43), §A2 (21–27), §A1 (13–19).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N6 — symlink/reparse-point
  classification stays).

## Scope
- Candidate shared crate `crates/fs-identity` (`girpr-fs-identity`).
- Report names: `tools/sync/src/hardlink.rs` (`link_count`, `identity_from_path`,
  `identity_of`, `FileId = same_file::Handle`, one deliberate `unsafe` Windows
  handle call), `src/repair/lock.rs` (`link_count`, `same_file` on dev/ino or
  volume serial/file index, `FILE_FLAG_OPEN_REPARSE_POINT`, fail-closed
  `Option<bool>`), `tools/sync/src/cache/backup/publish.rs` (consumer).
  Locate before editing.

## Implementation plan
1. Validate that both sites derive the same platform facts despite differing
   representations and error types (§F9). Confirm the `unsafe` is still the one
   deliberate Windows handle call (§A4).
2. Decide how the shared leaf exposes identity: from-path and from-handle
   variants, delegating the platform-specific handle wrapper to a maintained
   crate (e.g. `same-file` / a small `winapi-util` seam) where possible so the
   leaf does not need `unsafe_code` exceptions, or carrying a tightly scoped
   `allow` with justification if the call must move.
3. Expose link-count and identity comparison from the leaf, with explicit
   error/option types that let each caller keep its own policy (girsync's
   refusal vs girpr's fail-closed `Option<bool>`).
4. Migrate the named call sites; keep symlink/reparse classification and the
   link-count guard outside the leaf (§N6, §F9).

## Constraints / validation
- The single deliberate `unsafe` (§A4) must remain justified and narrowly
  scoped; workspace lints deny `unsafe_code` by default (§A4), so opting the
  leaf into `[lints] workspace = true` needs an explicit, reviewed decision.
- Link-count >1 guard and fail-closed behavior must survive (§F9, §N6).
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
Identity and link-count facts come from the shared leaf, both callers keep their
own policies, the `unsafe` is accounted for, and link/hardlink tests pass.
