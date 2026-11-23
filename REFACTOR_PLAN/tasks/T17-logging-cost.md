# T17 — Reduce logging cost without changing the data plane

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §7 **LOG-01** (lines 517–531) — a release build installs
  a log file at `debug` by default and every chunk emits formatted `debug!`/`trace!`
  events; default the file layer to `info` in release, or move per-chunk lines to
  `trace` while keeping per-file lines at `debug`. The `REPORT`/`PLAN`/`PROGRESS`/
  `SUMMARY` data plane stays unchanged.
- §7 **LOG-02** (lines 533–547) — `tracing` `%` fields eagerly evaluate expressions
  (e.g. `String::from_utf8_lossy(&body)`, `format!("{e:#}"))` even when the event is
  disabled; gate expensive fields behind `tracing::enabled!(Level::TRACE)` or a lazy
  wrapper, or drop the body field.
- §7 **LOG-03** (lines 549–559) — `tracing_appender::non_blocking` is lossy with a
  bounded channel that silently drops lines; use `lossy(false)` or report
  `dropped_lines()` on flush.
- §11 **assumptions 11–12** (lines 867–871) — operators can opt back in with
  `--log-file-level debug`; no bundled artifact depends on per-chunk lines;
  `lossy(false)` can backpressure a verbose run.
- Keep items: data plane, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/logging.rs` (default file level ~12–21, `flush_log` ~229–235).
- `crates/log/src/plan.rs` (default plan), `crates/log/src/install.rs` (~57).
- Expensive field call sites: `app/fetch/src/hyp.rs` (~361, 421), `src/repair/file/
  inplace.rs` (~202), `src/repair/scan.rs` (~390), plus per-chunk sites in
  `temp.rs`, `inplace.rs`, `scan.rs`.
- Locate all call sites before editing; the list above is a hint.

## Implementation plan
1. Confirm the default file level and the expensive field expressions in the current
   tree.
2. Lower the release default to `info` or move per-chunk events to `trace`, keeping
   per-file lines at `debug` and the data plane untouched (LOG-01).
3. Gate expensive field expressions behind `enabled!(TRACE)` or a lazy wrapper; drop
   the full-body field where it adds no value (LOG-02).
4. Make the file writer honest: `lossy(false)` or report `dropped_lines()` on flush,
   with a larger channel if backpressure is a concern (LOG-03).
5. Deliver in several commits if useful: default level, field gating, writer policy.

## Constraints / validation
- `REPORT`/`PLAN`/`PROGRESS`/`SUMMARY` output is unchanged (lines 527–530).
- Operators can still opt into per-chunk logs; no bundled artifact depends on them
  (assumption 11).
- The lossy/backpressure trade-off must be deliberate and validated (assumption 12).
- Do not change any §12 non-finding.

## Done when
Per-chunk logging is off by default and cheap when disabled, the writer does not
silently drop lines (or reports them), and the data plane is byte-identical. If an
assumption fails, **stop** and record `skipped` with a one-line reason.
