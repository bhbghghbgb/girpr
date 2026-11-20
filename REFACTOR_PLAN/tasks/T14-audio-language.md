# T14 — Audio-language normalisation primitive

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F23 (lines 323–327) — audio
  language normalisation shares a primitive and diverges around it.
- Assumptions to validate: §A2 (21–27), §A1 (13–19), §A3 (29–35).

## Scope
- Candidate home: an existing/new small leaf (e.g. `girpr-cli` T12 or a
  `girpr-model` leaf); choose the boundary that holds no program logic.
- Report names: `src/cli.rs` (lower + `_`→`-` + alias map + `HashSet` +
  none-sentinel), `tools/capture/src/config/mod.rs` (lower + trim + `_`→`-`,
  preserves code, validates as path component), `src/repair/audio.rs` and
  `tools/capture/src/emit/gamedir.rs` (the identical `DISPLAY` order and
  code/name pairs). Locate before editing.

## Implementation plan
1. Validate §F23: the shared primitive is lower-case + underscore→hyphen; the
   alias map, none-sentinel, path-component validation and `DISPLAY` selection
   are the parts that diverge.
2. Expose the primitive (and, if genuinely identical, the ordered code/name
   pairs) from the leaf, parameterised so each tool keeps its own alias/sentinel
   and validation policy.
3. Migrate the two normalisation sites, keeping their different validation and
   sentinel behavior local.
4. If gircap gains a new leaf dep, update §A2 doc tables.

## Constraints / validation
- Audio selection and emitted language codes must stay byte-identical (§A3).
- Do not merge the alias-map/none-sentinel rule with capture's path-component
  validation (§F23, §A6).
- No program logic in the leaf (§A1, §A2).

## Done when
The lower/`_`→`-` primitive and the shared ordered pairs come from the leaf,
each tool keeps its alias/validation policy, and audio/selection tests pass.
