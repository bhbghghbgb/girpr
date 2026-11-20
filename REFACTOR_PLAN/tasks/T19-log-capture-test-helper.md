# T19 — Shared log-capture test helper

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F16 (lines 253–258) — `logs_of`
  exists in three copies.
- Assumptions to validate: §A5 (45–50), §A2 (21–27).
- Related: T11 (`girpr-log`), §F17 precedent.

## Scope
- Candidate home: a dev-only `log_capture` module in `girpr-log`'s test support
  (§F16), with thin shims in each consumer's `tests/common`.
- Report names: `src/repair/test_support.rs` (`logs_of`, sync),
  `tests/common/mod.rs` (`logs_of`, async with `RuntimeFlavor::CurrentThread`
  debug assert — comment at :839–842 explains the duplication),
  `src/repair/phases/metadata.rs` (same async copy),
  `tools/sync/tests/common/mod.rs` (`SharedBuffer` `MakeWriter`).
  Locate before editing.

## Implementation plan
1. Validate §F16 and §A5: nextest runs one process per test and skips doctests,
   so shared test helpers must be public or re-exported; that is why one copy
   exists today. Follow the §F17 precedent: one implementation plus thin
   re-export shims (as done for `crates/tempdir/src/test_support.rs`).
2. Make the log-capture helper public from a shared dev/test module and provide
   both the sync and async (current-thread debug-assert) variants needed by the
   existing call sites.
3. Replace the three copies with shims, keeping the debug assertion and buffer
   semantics identical.
4. Ensure no program's test depends on another program's test (§A2); the helper
   must be a leaf test utility, not a tool fixture.

## Constraints / validation
- Do not share the tool-specific fake HTTP servers (§N7) or domain fixtures —
  only the log-capture mechanics.
- nextest/one-process-per-test constraints must keep working (§A5).
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
All `logs_of` copies are shims over one public helper, the debug assertion and
buffer behavior are preserved, and all logging tests pass under nextest.
