# T20 — Filter glob matching → globset

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-5 (lines 467–479) — `girpr-filter`'s
  hand-written `matches_or_ancestor` prefix walker and `compile` become `globset`
  (`GlobSet`, `case_insensitive`, `literal_separator(false)`), folding the ancestor rule
  in by adding `pattern/**` alongside `pattern`.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A12 (48–50). Keep items:
  `guidance/DO-NOT-TOUCH.md` — shared crate, both products consume it.

## Scope
- `crates/filter/src/lib.rs:18–90` and tests at `:101–119`, `:121–135`, `:137–144`.
- `crates/filter/Cargo.toml`; the two product consumers of `girpr-filter`.

## Implementation plan
1. Validate the current matching contract: `/` boundaries, leading dots not special,
   backslash normalization driven by the caller's flag, and the `anyhow` error wording.
2. Compile a `GlobSet` with `case_insensitive(!case_sensitive)` and
   `literal_separator(false)`; add `pattern/**` alongside each `pattern` to reproduce
   the ancestor rule. Keep backslash normalization in `compile` and the error wording.
3. Keep `separators_and_leading_dots_are_not_special` and
   `ancestor_matching_respects_the_separator_boundary`; adapt
   `backslash_mode_controls_normalization` by dropping only the two `as_str()` pins.
   Add a `**` case for the new ancestor representation.
4. Verify both product consumers build and their filter-driven tests pass; the
   re-export change touches both products (§LEAF-5 line 478).

## Constraints / validation
- `globset`'s default "dot is not special" matches the current rule (§LEAF-5 line 479).
- Do not change the public API shape in a way that leaves one consumer unupdated.
- `globset 0.4.22` is not currently locked; verify a resolved build.

## Done when
The hand-written ancestor walker is gone, globset implements the same matching rule,
the named tests (with the two pins removed) pass, and both products build. If a premise
is stale or a keeper would change, **stop** and record `skipped`.
