# T11 — Log-install helpers + timestamp reuse

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F18 (lines 273–284) — logging
  implementation near-copies below the shared policy.
- `docs/08-shared-crate-refactor-research.md` §F19 (lines 286–289) — timestamp
  format duplicated despite `girpr_log::timestamp` existing.
- Assumptions to validate: §A1 (13–19), §A2 (21–27), §A3 (29–35).

## Scope
- Candidate home: extend `crates/log` (`girpr-log`) with format-agnostic
  installation helpers (§F18), per §A1 it already owns the policy.
- Report names: `src/logging.rs`, `tools/sync/src/logging.rs`,
  `tools/capture/src/logging/mod.rs` (open_log_file, "log file:" announcement,
  env-warning print, subscriber-install warning, `non_blocking` writer +
  filter construction); `crates/log/src/plan.rs` (`timestamp`),
  `tools/capture/src/config/mod.rs` (`capture_id`). Locate before editing.

## Implementation plan
1. Validate §F18: extract only the format-agnostic installation mechanics
   (file opening, announcement/warning emission, non-blocking writer + filter
   construction) into `girpr-log`. Leave layer construction and the guard
   lifetime local — product uses a process-global slot + `flush_log()`, sync and
   capture use RAII / `finish()` (§F18).
2. Validate §F19: confirm `girpr_log::timestamp` can serve capture's
   `capture_id()` format. Capture used a caller-supplied `DateTime` seam in
   prior work — preserve that seam if it is still present; do not force capture
   to use the wall-clock helper if it deliberately injects time.
3. Migrate the named sites, keeping each tool's layer/guard policy local.
4. gircap already depends on `girpr-log`, so no new §A2 leaf dep is expected;
   still confirm no program logic enters the leaf.

## Constraints / validation
- Log file naming/format and relative timestamp strings are part of artifacts
  and tests (§A3); no format change.
- Do not move layer construction or guard lifetime into the leaf (§F18).
- The leaf must stay logic-free (§A1, §A2).

## Done when
Common log-install mechanics and the timestamp format are sourced from
`girpr-log`, guard lifetimes stay local, and all logging/timestamp tests pass.
