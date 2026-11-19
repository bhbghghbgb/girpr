# T29 — girpr INI parse/merge/render

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A4 (lines 132–170), especially
  the two conflicting positions (147–155) and restructure (157–161).
- Assumption to validate: §I.2 (lines 817–821).

## Scope
- Package: root `girpr`, `src/util/ini.rs` (`Section`, `find_key`, `insert`,
  `force`, `find_section`, `parse_ini`, `merge_ini`, `render_ini`,
  `write_config_ini`, `read_game_version`).

## Implementation plan
1. Validate the two conflicting research positions (§A4) against the current
   code and the byte-level `config.ini` contract.
2. If delegating the parse/merge core is worthwhile, replace `Vec<Section>` with
   `configparser`/`ini`, keeping the forced-key table, the UTF-8/atomic boundary
   and a renderer if the crate's writer orders differently. Do not pull the
   JSON-lines readers (lines 59–216) into a config framework.
3. Keep `read_game_version` distinct from gircap's reader (§A4 caveat).

## Constraints / validation
- Current behavior is deliberate Starward/Collapse parity (drop comments, merge
   duplicate sections, sort keys, force `[General]` keys, blank line after each
   section). A general parser changes several rules; tolerance for observable
   changes makes both positions viable, so the decision must be validated, not
   assumed (§I.2).
- If the byte-exact contract matters and migration would regress it, **stop** and
   record as `skipped` (or adopt only the parser and keep the renderer).
- Tests named in §A4 (lines 163–165) pin the merge/render rules.

## Done when
Either the parse/merge core is delegated with parity validated, or the task is
skipped with the decision recorded in `STATE.md` persistent context.
