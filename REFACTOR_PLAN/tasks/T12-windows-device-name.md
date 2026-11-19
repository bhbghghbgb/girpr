# T12 — Windows device-name / path-shape rule

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §F7 (lines 687–706).

## Scope
- Package: `tools/capture`, `src/paths.rs` (`path_rule`,
  `is_windows_device_name`, `escape_component`/`unescape_component`) and
  `src/config/mod.rs` (tag-shape rule).

## Implementation plan
1. Replace **only** `is_windows_device_name` with a library reserved-name
   predicate (e.g. `sanitize-filename`).
2. Share one rule between `paths.rs` and `config/mod.rs`.
3. Leave reject-don't-rewrite semantics, the `%` escape scheme, and the `..`/`.`
   normalization hand-rolled — no library expresses the whole rule.

## Constraints / validation
- A crate's reserved-name set may include/exclude `CONIN$`, `CONOUT$`, `COM0`
   differently — validate against the current set.
- The escape scheme is a bundle-format contract in `diagnostics/path_map.json`;
   changing it changes artifact layout.
- Tests named in §F7.

## Done when
The reserved-name predicate is delegated and shared, with artifact layout
unchanged.
