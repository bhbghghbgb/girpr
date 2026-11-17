# T02 — Remove duplicated fixture-helper meta-tests

Priority P0 · root product unit tests · checkpoint CP1

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-suite-reduction-audit.md:L119–L128` (§C.1).
* Report "kept intentionally" note: `docs/08-test-suite-reduction-audit.md:L143–L148` (§C.6).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T02-fixture-helper-meta-tests.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

The symlink/junction "should we skip this case" decision is tested in three
files. The canonical owner is `src/util/junction_support.rs`. Remove the six
duplicated copies:

* `src/util/path.rs:1045, 1077, 1085`
* `src/util/ini.rs:1230, 1291, 1300` — the last two are byte-identical to
  `path.rs:1077/1085`.
* `src/util/ini.rs:1370` (`link_guard_retries_a_failed_explicit_removal_on_drop`)
  tests only a test-local `LinkGuard` helper — removable.

Expected ~7 tests removed, **low risk**, no coverage lost.

## Implementation plan

Code anchors from the report (validate before editing):

* `src/util/junction_support.rs` — the canonical fail-closed / skip-decision
  tests that must remain.
* `src/util/path.rs:1045/1077/1085`, `src/util/ini.rs:1230/1291/1300/1370` —
  the copies to remove.

Possible commits:

1. Remove the three `path.rs` copies, confirming `junction_support.rs` still
   carries each assertion.
2. Remove the four `ini.rs` copies/helper test, again confirming parity.

## Acceptance criteria

* Canonical tests in `junction_support.rs` still present and passing.
* No skip-decision coverage lost (compare assertions before deleting).
* Root unit suite passes; no product code touched.

## Guardrails

* Validate parity first: if a copy asserts something the canonical test does
  not, keep that assertion (move it) rather than deleting coverage.
* Test-only change. Do not alter symlink/junction support behavior.
* Report follow-up ideas without implementing them.
