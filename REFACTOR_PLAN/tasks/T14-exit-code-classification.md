# T14 — Classify lock and directory failures correctly

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §6 **COR-01** (lines 417–428) — `create_dir_all`
  failure (permission/disk-full) and `Lock::acquire` failure (commonly "another run
  holds the install") both map to `RunFailure::usage` = exit 1, whose meaning is "the
  request or configuration is wrong".
- Change suggested in the report: map `create_dir_all` to `RunFailure::write` (exit 3)
  with path and error; map lock contention to a non-usage class whose message names
  the holder (`held()` already provides it).
- §11 **assumption 10** (lines 864–866) — downstream automation may pin exit 1;
  confirm a new exit class is acceptable before changing observable behavior.
- Keep items: exit-code table meaning, data-plane output, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/mod.rs` (`create_dir_all` ~832–834, `Lock::acquire` ~859–860).
- `src/config.rs` (code meanings ~40–42), `app/lock/src/lib.rs` (`held()` ~68–73).
- Locate current failure mapping before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 10: check whether any in-repo consumer/test/doc pins exit 1
   for these failures. If the change would break a pinned contract and cannot be
   coordinated, stop.
2. Map `create_dir_all` failure to `RunFailure::write` (exit 3) with path and error
   text.
3. Map lock contention to a non-usage class whose message names the current holder
   via `held()`.
4. Update tests/docs that assert the old exit class.
5. Deliver in one or a few commits.

## Constraints / validation
- This is an intentional observable behavior change; validate assumption 10 and keep
  the exit-code table and messages consistent.
- The data-plane stdout records (`REPORT`/`PLAN`/`PROGRESS`/`SUMMARY`) stay unchanged
  (lines 527–530).
- Do not change any §12 non-finding.

## Done when
Both failure modes use truthful exit classes with actionable messages, tests/docs are
updated, and assumption 10 is validated. If the exit class cannot be changed safely,
**stop** and record `skipped` with a one-line reason.
