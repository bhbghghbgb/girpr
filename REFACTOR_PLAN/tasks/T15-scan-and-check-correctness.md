# T15 — Fix stale scan/check verdicts, surface scan failures, close tmp TOCTOU

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §6 **COR-02** (lines 429–441) — `--check-only --scan`
  trusts a stale scan verdict: the scan records `modified` but `ScanReport::get` does
  not re-validate it against the fresh stat `file_len` already performs, so a
  same-length content change can yield `Success`. Re-validate `modified` and fall back
  to a read when it differs, or prominently document the behavior.
- §6 **COR-05** (lines 464–472) — panicked/cancelled scans and unreadable files
  degrade silently; add a per-file `scan_failed` count / collected reasons to
  `ScanReport` and surface them in the plan line, keeping "estimate in full".
- §6 **COR-08** (lines 496–504) — `promote_tmp` stats/hashes by path then renames by
  path (TOCTOU); hold one handle, stat/hash through it, then rename (OS
  handle-relative rename where available), as the in-place path already does
  (`inplace.rs:786–790`).
- §11 **assumption 9** (lines 862–863) — confirm whether `--check-only --scan` is
  intended to be a fresh check before changing semantics.
- Keep items: conservative "estimate in full" fallback, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/phases/check.rs` (`check_one` ~339–389), `src/repair/scan.rs`
  (trust docs ~18–23, panic/unreadable handling ~316–327, ~379–392), `ScanReport`.
- `src/repair/file/temp.rs` (`promote_tmp` ~716–748), `file/inplace.rs`
  (handle-based guard ~786–790).
- Locate current structures before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 9: confirm the intended semantics of `--check-only --scan`.
   If it is deliberately a plan-answer check, document that instead of changing it.
2. Re-validate the scan's `modified` against the fresh stat and fall back to a read
   when it differs (COR-02).
3. Add a per-file `scan_failed` count / reason collection to `ScanReport` and surface
   it in the plan line while keeping the conservative "estimate in full" behavior
   (COR-05).
4. Make `promote_tmp` hold one handle for stat/hash/rename, using handle-relative
   rename where available (COR-08).
5. Deliver in several commits if useful: verdict revalidation, diagnostics, TOCTOU.

## Constraints / validation
- Keep "estimate in full" when a scan has no verdict; never turn a diagnostic into a
  silent success.
- Directory reports must stay machine-readable and consistent with the data-plane
  contract (lines 527–530).
- Do not change any §12 non-finding; the verify-path multiple reads in §12 (898–900)
  are deliberately kept.

## Done when
Stale verdicts are revalidated or documented, scan failures are surfaced, and the tmp
promotion TOCTOU is closed with tests passing. If assumption 9 or the handle-relative
rename cannot be satisfied, **stop** and record `skipped` with a one-line reason.
