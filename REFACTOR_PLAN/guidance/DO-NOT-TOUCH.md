# Do not touch (or touch only as a task says)

These findings were examined and deliberately kept, or are items that are **not** tasks.
Before editing anything, a session should confirm its target is not on this list. Full
reasoning is in the report; only pointers are given here. If a session finds one of these
should change after all, it **stops** and raises a follow-up — it does not change the
keeper in place.

## Shared crates — never move or rename in this round

- **§F1 shared rows (lines 47–67), §F2 (85–89):** the 19 shared `crates/` members stay
  where they are and keep their `girpr-` names. The ownership split does not affect them.
- **§F5 (lines 116–123):** moving or renaming the 16 single-project crates does not remove
  the second consumer of any shared crate; do not use that as a reason to touch a shared
  crate. `girpr-fs-identity`, `girpr-filter`, `girpr-loopback-mock` and `girpr-report-core`
  remain shared.
- **§F9 (lines 166–174):** `girpr-proto`, `girpr-model`, `girpr-chunk-identity`,
  `girpr-chunk-cost`, `girpr-exit`, `girpr-cli`, `girpr-value-ranges`, `girpr-retry`,
  `girpr-time`, `girpr-fs-names`, `girpr-path-shape` and `girpr-report-core` are consumed by
  gircap and cannot be classified as single-project.
- **§F9 (lines 169–170):** `girpr-audio-language` is shared because gircap uses it; it is
  not a girpr-only crate.
- **§A6 (lines 33–35):** renames are consistency consequences of the ownership rule. No
  rename of a genuinely shared crate is required or assumed.

## gircap crates — already correct

- **§F9 (lines 164–168):** `gircap-http`, `gircap-hash` and `gircap-test-support` under
  `tools/capture/` are already gircap-owned, correctly placed and correctly named; they
  are not candidates. Do not "align" them with the girpr or girsync moves.

## Dependency-direction facts that must survive (§F6, lines 125–139)

- `girpr-test-support` depends on the root `girpr` package as a normal (test-only
  inversion) dependency — do not place it where the product depends on it in a shipped
  build, and do not remove the inversion.
- `girpr-repair-plan` → `girpr-fetch` stays inside the girpr project under either layout.
- `girpr-sync-cache` is consumed **directly** by sled2redb, not through girsync's lib
  facade, and pins the digest vocabulary in its `postcard` wire format. The direct
  consumer edge and the wire vocabulary must survive the move byte-for-byte.
- `girpr-sync-test-support` depends on girsync and is taken as a dev-dependency by both
  girsync and sled2redb; that inversion must survive.
- `girpr-retry`, `girpr-log`, `girpr-time`, `girpr-digest` and `girpr-fs-identity` are
  dependencies of crates that move; all five are shared and stay in `crates/`.

## Lint policy (§F7, lines 141–148)

- Girpr-owned crates keep `[lints] workspace = true`. Do not change their lint table to
  the tools-only subset just because they move.
- Girsync-owned crates moved under `tools/sync/` switch to the tools-only
  `[lints.rust]` / `[lints.clippy]` table (denied `unsafe_code`, `clippy::all` warning,
  no `unwrap_used`/`expect_used`). Do not keep the workspace table on them after the move.

## Scope boundaries

- The report does not fix the girpr-owned subtree's name (§F4, lines 104–114). Do not
  treat any name in this package as mandated; T04 chooses it and records it in
  `STATE.md` Persistent context.
- No rename of the 13 girpr-only crates is implied by §F4 — their `girpr-` prefix already
  names their owner. Only the three `girpr-sync-*` crates are renamed (§F3).
- Misleading manifest descriptions (e.g. `girpr-lock`'s "shared by the product and its
  integration tests", §F9 lines 175–177) are finding material, not drive-by edits; they
  belong to T07 or an endorsed follow-up.
- This package and the report are deleted at cleanup. No shipped source, test, comment,
  manifest, doc table or commit message may reference them.
