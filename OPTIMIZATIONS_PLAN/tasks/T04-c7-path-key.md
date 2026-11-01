# T04 - C7: use a Windows path key, not Unicode lowercase

Report: `../../OPTIMIZATIONS_REPORT.md` lines 205-225 (C7).

Dependencies: none.

## Goal

`util::path::path_key` is the single authority for "these two rel spellings may
name the same file" in the plan's dedup and the purge's keep comparison. On
Windows it currently uses `str::to_lowercase`, which is a Unicode lowercase
mapping, not the filesystem's case relationship. It can map distinct names onto
one key (the Kelvin sign and `k`, for example), causing an over-merge in the
plan or an over-keep in the purge.

Use an ASCII-only fold on Windows. Manifest paths are ASCII in practice; the
pragmatic rule is "fold ASCII case, compare every other byte exactly". Document
that choice in the function, not in a temporary plan.

## Where to start

- `src/util/path.rs`: `path_key` (around line 160) and its doc comment.
- `src/repair/phases/plan.rs` uses `path_key` for dedup.
- `src/repair/purge.rs` uses `path_key` for the expected/server/blacklist sets.
- `src/util/path.rs` tests (the file has a test module; add a focused
  `path_key` test there).

## Steps

1. On Windows, return `rel.to_ascii_lowercase()`. On Unix, return
   `rel.to_string()` unchanged, as today.
2. Rewrite the doc comment:
   - why the key exists;
   - why ASCII is enough for server manifest paths;
   - what is deliberately not folded (non-ASCII case variants are distinct
     keys);
   - that the rule is compile-time selected and both branches must stay in
     sync.
3. Do not rename the function or move it. It is used through the module; a
   rename is churn without a behavioural gain.
4. Do not change `clean_rel`, `contained_rel`, or the scope glob matching. The
   scope's case-insensitive matching is a separate, documented rule.

## Tests

- On the host platform, assert the documented cases:
  - ASCII case variants produce the same key on Windows and different keys on
    Unix (use `cfg!`/`#[cfg]` so the assertion matches the branch);
  - `"\u{212A}"` (Kelvin sign) and `"k"` produce **different** keys on
    Windows;
  - `"Game/A.DAT"` and `"game/a.dat"` produce the same Windows key;
  - a non-ASCII name is returned unchanged except for ASCII letters.
- Add one plan-level or purge-level test only if an existing test already
  pins the over-merge; otherwise the unit test is the contract.

## Acceptance

- Windows `path_key` never collapses two non-ASCII spellings through Unicode
  lowercase.
- ASCII case-insensitive dedup/keep behavior is unchanged.
- Unix behavior is unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(path): fold only ASCII case in the Windows path key`
2. `test(path): pin ASCII and non-ASCII path-key behavior`

## Notes

- Do not "improve" this into a lexical-only or filesystem-resolving equality
  check. It is a map key, not a containment check; `contained_join` owns the
  filesystem decision.
