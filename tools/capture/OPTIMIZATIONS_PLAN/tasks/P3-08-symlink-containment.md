# P3-08 — Symlink containment and termination

Report: `../../OPTIMIZATIONS_REPORT.md` lines 621-635 (C9).

Dependencies: P1-09, P3-01.

## Goal

Prevent `verify::list` and `emit::index::walk` from looping on a
symlink/junction cycle and from including files outside their root. The current
`metadata`/`is_dir` checks follow links with no visited set or containment
check.

## Where to start

- `src/verify/list.rs`: `list_with` (around line 93), `child_dirs` (around line
  151), and the `read` injection.
- `src/emit/index.rs`: `walk` (around line 245) and `rel`.
- Any other recursive walker `rg -n "read_dir"` finds; do not change unrelated
  one-shot reads.
- Tests in `src/verify/list.rs`, `src/emit/index.rs`.

## Design

Use a visited set of canonical directory paths plus a canonical root:

- Canonicalize the root once. If that fails, keep the current error behavior.
- Before descending into a directory, canonicalize it. If it does not start
  with the canonical root, skip it (record a warning or a finding where the
  caller has a place for one; do not silently include outside bytes).
- If the canonical path is already in the visited set, skip it (cycle).
- For `walk`, apply the same rule to files: a file whose canonical path is
  outside the root is not part of the artifact. Do not follow a symlinked file
  to its target and hash it as if it were under the root.
- Keep the output sorted and artifact-relative. Canonicalization must not leak
  absolute paths into the document.

## Steps

1. Add a small private helper for "canonical path under root" to the module
   that needs it; share it if a neutral module exists. Do not over-abstract.
2. Update `walk` to canonicalize each directory before recursion; use a
   `HashSet<PathBuf>` visited set. For entries, use `symlink_metadata` to
   distinguish a link if needed, or canonicalize and containment-check.
3. Update `list_with`/`child_dirs` similarly. The injected reader returns
   paths; canonicalization happens after the read, so the seam still works.
   Keep the existing behavior where an unreadable entry is an error.
4. Do not refuse all symlinks; a symlinked artifact root may be legitimate. A
   visited set plus canonical-root containment preserves that case.
5. Update the doc comments that currently say "metadata follows symlinks" as a
   known property; state the new containment rule.

## Tests

- A directory symlink/junction cycle terminates and reports each real directory
  once.
- A symlink under the root pointing outside is not walked/listed.
- A symlinked root still works (canonical root is the target).
- Platform note: on Windows, creating a symlink may require privilege. Gate the
  test to skip with a clear message if creation fails; do not fail the suite
  for an unavailable privilege. On Unix, use `std::os::unix::fs::symlink`.
- If a filesystem-level test is not possible, unit-test the containment/visited
  helper and note the gap.

## Acceptance

- Cyclic trees terminate.
- A bundle cannot include bytes outside its root through a link.
- Normal bundles and a legitimately symlinked root are unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(emit): keep the artifact walk inside its canonical root`
2. `fix(verify): terminate list traversal on visited directories`
3. `test(emit,verify): cover link cycles and outside-root links`

## Notes

- This is filesystem semantics; do not change how files are hashed once they
  are accepted.
- Do not reference the report or plan in code comments.
