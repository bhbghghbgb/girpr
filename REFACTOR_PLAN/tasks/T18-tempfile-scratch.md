# T18 — Scratch / case-tmp paths → tempfile

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-9 (lines 520–528) — scratch paths
  built from pid + counter become `tempfile::tempdir()`.
- §SYNC-4 (lines 575–587) — the hand-written `.girsync_case_tmp.<n>` claim loop becomes
  `tempfile::Builder` with a prefix in the parent directory.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/fs-identity/src/lib.rs:184–205` (`Scratch::new`);
  `crates/log/src/install.rs:120–128` (`scratch_dir`).
- `tools/sync/src/commands/sync/rename.rs` — `claim_case_tmp` (`:193–259`), caller `:448`,
  and `fail_step1`/`fail_step2` (`:261–299`).
- `tools/sync/tests/sync.rs:746–830` (literal `_0/_1/_2` suffixes) and `:931–960`.
- Affected manifests. `tempfile` is already locked.

## Implementation plan
1. Back the scratch helpers with `tempfile::tempdir()` for random names and RAII
   cleanup; keep their `join`/path helpers and any bespoke `Drop` behavior that callers
   rely on.
2. Replace `claim_case_tmp` with
   `Builder::new().prefix(".girsync_case_tmp.").tempfile_in(parent)` and
   `into_parts().1.keep()` for files, `tempdir_in(parent)` + `into_path()` for dirs.
   Keep returning `PathBuf` so `fail_step1`/`fail_step2` are unchanged.
3. Rewrite the tests that pin deterministic suffixes to assert the real contract: a
   unique sibling in the tool namespace with pre-existing files untouched (§SYNC-4
   lines 585–587).
4. Run the fs-identity/log/sync tests; confirm Windows cleanup behavior is no worse than
   today (§LEAF-9 line 527).

## Constraints / validation
- Random suffixes are acceptable; deterministic numbering is not a user contract.
- Preserve the link-rename policy around the temp name — only the name claim changes.
- Windows cleanup can still fail if a test leaves a file open; do not promise otherwise.

## Done when
Both scratch helpers and the case-tmp claim loop use `tempfile`, suffix-pinning tests are
rewritten around the real contract, and the suites pass. If a premise is stale or a
keeper would change, **stop** and record `skipped`.
