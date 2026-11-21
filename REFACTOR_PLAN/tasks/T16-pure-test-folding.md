# T16 — Fold tiny pure-test clusters

## Goal / pointers
- `docs/10-test-optimization-research.md` §F26 (lines 352–360) — ~131 of 271 in-scope
  tests are tiny pure tests in their own process. Named clusters: `crates/util/src/atomic.rs`
  (four), `crates/retry/src/lib.rs` (three), `crates/fetch/src/retry.rs` (six),
  `crates/fs-names/src/lib.rs` (six), `crates/report-core/src/lib.rs` (four),
  `crates/fetch/src/sophon/stream.rs` (five). Fix: fold closest cases into one test with
  labelled subcase loops and preserved assertion messages. Benefit Low–Med wall.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (merge caveats).

## Scope
- The six named clusters; locate their current case tables and assertion messages
  before editing. Check `.config/nextest.toml` for named tests and host-dependent skips.

## Implementation plan
1. Validate §F26: measure nextest per-test overhead for one cluster before a broad
   rewrite; if merges save no wall time, skip rather than churn (F35).
2. For each validated cluster, fold the tightest cases into one test with a labelled
   subcase loop that keeps every original assertion message.
3. Do not merge a test named in a nextest filter, a host-dependent skip, or a
   log-capture user (§A10).
4. Update nextest filters if a case name changes (§A10); deliver in commits per cluster.

## Constraints / validation
- Merging coarsens failure granularity (§A1) — labelled subcases must keep failures
  diagnosable. Preserve every assertion, not just the happy path.
- No sleeps (§A6), no fixture growth (§A5), no byte changes (§6).
- Benefit is modest; prefer fewer, high-confidence merges over a broad rewrite.

## Done when
Validated clusters are folded with labelled subcases and preserved messages, no named
or host-dependent test was removed, and the affected crate suites pass with fewer
process spawns.
