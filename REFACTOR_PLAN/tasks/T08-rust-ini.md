# T08 — INI parser/merger → rust-ini

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-1 (lines 262–279) — the ~150-line
  hand-written INI parser/merger/renderer should use `rust-ini` with
  `features = ["case-insensitive"]`; a thin policy layer keeps the forced keys, sort
  order and blank-line shape, and still hands the rendered string to `write_atomic`.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23), §A5 (26–28).
  Keep items: `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/util-ini/src/ini.rs` — `write_config_ini` (`:334–388`), `Section` (`:390–419`),
  `find_section`/`ensure_general` (`:421–439`), `parse_ini` (`:441–495`), `merge_ini`
  (`:497–520`), `render_ini` (`:522–540`) and `read_game_version` (`:10–57`).
- Byte-exact test pins at `:575`, `:589`, `:604`, `:915`, `:973`, `:1044`, `:1352`, `:1401`.
- `app/util-ini/Cargo.toml`; root manifest if the dependency is declared there.

## Implementation plan
1. Validate the exact policy the hand-rolled code applies: lowercasing, duplicate-section
   merge, comment dropping, forced `[General]` keys, key sorting and blank-line shape.
   Decide what must be preserved versus what may change (§A4).
2. Introduce `ini::Ini` with case-insensitive keys; move headerless keys into
   `Some("General")`. Keep a thin policy layer for the forced keys, sort order and
   rendering shape; continue to publish atomically via `write_atomic`.
3. Decide deliberately whether `read_game_version` stays a deliberate Starward-parity
   last-match line scan or becomes `Ini::get` (which changes to first-match semantics).
   Record the choice in the task note if it affects other work.
4. Adjust or remove the byte-exact pins; keep tests of the intended config/version
   behavior. Add `rust-ini` (not currently in `Cargo.lock`) and verify a resolved build.

## Constraints / validation
- `rust-ini` canonicalizes escaping and may render non-ASCII as `\x…`; game values are
  effectively ASCII (§APP-1 lines 278–279). Verify against real fixtures before accepting.
- Preserve the forced `[General]` keys and atomic publish; do not change consumers.
- Regenerate pins rather than deleting semantic assertions (§A5).

## Done when
The hand-rolled parser/renderer is gone, `rust-ini` owns parsing/merging, the policy
layer preserves the intended config output, and the util-ini tests pass with adjusted
pins. If a premise is stale or a keeper would change, **stop** and record `skipped`.
