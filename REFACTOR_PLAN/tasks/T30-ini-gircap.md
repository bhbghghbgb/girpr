# T30 — gircap `parse_game_version`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A4 (lines 138, 160–163,
  168–170), §F10 (lines 738–741).

## Scope
- Package: `tools/capture`, `src/emit/gamedir.rs` (`parse_game_version`, lines
  72–96; `config_ini`, lines 44–64).

## Implementation plan
1. Replace only `parse_game_version` with a library INI reader.
2. Keep the `config_ini` renderer hand-written for byte-exact artifact rendering
   (§F10).

## Constraints / validation
- Keep gircap's "first match in `[General]`" semantics distinct from girpr's
   "last match wins, no section scope" (§A4 caveat) — do not unify them unless a
   shared configurable reader is explicitly designed (out of scope).
- Tests named in §A4 (line 165).

## Done when
The small reader is delegated and the byte-exact renderer and semantics are
unchanged.
