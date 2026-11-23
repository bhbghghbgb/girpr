# T21 — Fix gircap indexing and canonicalization correctness

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-05** (lines 679–684) — the `build_id` closure
  rescans manifests × fields × entries per admitted path; store `(map, build_id)` in
  the index or add an O(1) `HashMap<(&str,&str),&str>`.
- §9 **TOOL-06** (686–690) — `build_manifest`'s matrix is O(rows × admitted);
  build a `HashMap<&str, &Admission>` by `capture_id` once.
- §9 **TOOL-13** (729–737) — every entry is canonicalized on each of at least four
  close-out walks; share one canonical walk result per close-out, or canonicalize only
  entries with a symlink component (cheap `file_type()` check first).
- §9 **TOOL-17** (763–768) — `rel()` allocates a `Vec<Cow>` + String per entry per
  pass; iterate `components()` and `push_str` into a pre-sized String.
- §9 **TOOL-19** (780–788) — canonical dedup can drop the real file for a symlink alias
  that sorts first; only dedup directories in `filter_entry` (`is_dir()`), files cannot
  create a traversal cycle.
- §9 **TOOL-20** (789–795) — a debug-only path round-trip assertion allocates per
  document write; move it into a test over representative paths or gate it behind a
  test-only switch.
- §11 **assumptions 16–17** (lines 881–888) — confirm the symlink dedup contract before
  changing `visited`; preserve documented write order and deterministic artifact
  bytes and leave the close-out fixpoint passes unaffected.
- Keep items: artifact bytes, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/blobs/mod.rs` (~924–933, 948–949),
  `emit/index.rs` (matrix ~86–112, `walk`/close-out ~275–337, 353–356, dedup
  ~284–302, `rel()` ~363–370), `emit/mod.rs` (~414–419, 526–531).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Validate assumptions 16–17: confirm whether symlink/real-file dedup is intentional
   and record the exact close-out fixpoint passes and write-order requirements
   before editing.
2. Add O(1) indexes for build ids and admissions (TOOL-05/06).
3. Share one canonical walk per close-out or canonicalize only symlink-containing
   entries; make `rel()` allocation-light (TOOL-13/17).
4. Restrict dedup to directories and add a regression test where a symlink sorts
   before its real file (TOOL-19).
5. Move the debug-only assertion into a test or gate it (TOOL-20).
6. Deliver in several commits if useful: indexes, canonical walk, dedup fix, assertion.

## Constraints / validation
- Documented write order and deterministic artifact bytes must survive; the two/three
  close-out fixpoint passes must be unaffected (assumption 17).
- The symlink dedup change must be confirmed against the bundle contract and backed by
  a regression test (assumption 16).
- Do not change any §12 non-finding.

## Done when
Lookups are O(1), canonicalization is shared/cheap, dedup is directory-only with a
regression test, the debug assertion is gated, and artifact bytes are byte-identical.
If an assumption fails, **stop** and record `skipped` with a one-line reason for the
affected sub-change.
