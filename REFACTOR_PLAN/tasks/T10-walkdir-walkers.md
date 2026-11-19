# T10 — Directory walkers → `walkdir`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A6 (lines 208–232), §F3
  (lines 654–656).

## Scope
- Package: `tools/capture`, `src/emit/index.rs` (`walk`,
  `canonical_under_root`, `rel`) and `src/verify/list.rs` (`list`, `list_with`,
  `child_dirs`, `canonical_under_root`). `walkdir` is already a workspace
  dependency.

## Implementation plan
1. Replace both hand-rolled recursive `read_dir` walkers with
   `WalkDir::new(root).follow_links(true).sort_by_file_name()` plus the existing
   canonical-root containment filter.
2. Keep the "MANIFEST.json is a bundle boundary — do not descend" rule as
   `filter_entry`.
3. Keep `list_with` as the fault-injection seam, or rewrite that test at
   `Bundle::open` if the seam can be removed.

## Constraints / validation
- `walkdir` detects symlink loops via ancestor checks, not a global visited set;
   dedupe by canonical path if a sibling symlink can be walked twice.
- Sorting is already used, so document bytes should be unchanged.
- Tests named in §A6 (including injected read-error tests).

## Done when
Both walkers use `walkdir` and the bundle boundary / error-injection behavior is
preserved.
