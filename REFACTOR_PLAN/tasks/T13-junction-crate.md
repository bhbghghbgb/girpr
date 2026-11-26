# T13 — Junction creation → junction crate (Windows)

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-6 (lines 334–345) — the production
  `cmd /c mklink /J` fallback becomes `junction::create`; the `%`-refusal and shell
  quoting disappear.
- §LEAF-3 (lines 441–449) — the test-support junction fallback becomes the same call.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23). Keep items:
  `guidance/DO-NOT-TOUCH.md` — the junction/symlink error classifiers at report lines
  820–821 are policy and stay.

## Scope
- `app/util-path/src/junction_support.rs` — `try_junction_dir`/`junction_command`
  (`:78–124`), `reject_percent_path` (`:152–163`), `run_command_line` (`:166–173`).
- `crates/tempdir/src/test_support.rs:125–174` (`dir_link`, especially `:138–164`).
- Tests at `app/util-path/src/path.rs:1218` and `app/util-ini/src/ini.rs:1188`, plus
  classifier unit tests; affected manifests.

## Implementation plan
1. Validate the argument order and target/link semantics of `junction::create` against
   both call sites — §APP-6 (line 340) and §LEAF-3 (line 447) spell the order differently,
   so check the crate docs and each local signature rather than copying either hint.
2. In production, replace the command path with `junction::create(target, link)`;
   keep the skip/fail classifier and `JunctionFixture` policy. Remove
   `reject_percent_path` and `run_command_line` only when no caller remains.
3. In test support, call `junction::create(link, target)` in the Windows branch; keep
   the original error as context; remove `Command`, `native()` and stderr parsing.
4. Update classifier tests for the new error kinds; keep `skip_if_no_link!` behavior.
   Verify Windows-specific builds/tests; the crate is Windows-only, so keep both call
   sites behind their existing `#[cfg(windows)]` gates.

## Constraints / validation
- NTFS is assumed; `junction::create` supports dangling targets (§APP-6 line 345).
- `junction 2.1.0` is Windows-only and not currently locked; verify a resolved build on
  the target platform. Do not introduce it into non-Windows code paths.
- Keep the skip/fail policy — this task changes only the junction-creation mechanism.

## Done when
Both `cmd /c mklink /J` paths use the `junction` crate, the classifiers and fixture policy
survive with updated error tests, and the affected tests pass. If a premise is stale or
a keeper would change, **stop** and record `skipped`.
