# T18 — Sync injected `TestHooks` for ambient env hooks

Priority P2 · `tools/sync` (girsync) / production code · checkpoint CP8 ·
report finding E5

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L741–L762` (§E5).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L818–L820` (Wave 3;
  E5 is the prerequisite for the failure-path cases).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T18-sync-test-hooks.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Unlock the ~15 remaining failure-path `sync.rs` spawns by replacing process-global
(partly `OnceLock`-cached) env hooks with an injectable struct. Builds on T16.

Read sites: `src/config/common.rs:366–372`, `src/cache/mod.rs:61–93`,
`src/cache/write.rs:65–93`,
`src/commands/sync/apply/copy.rs:79–140` (`FAIL_COPY`, `FAIL_FLUSH`, `NO_FSYNC`,
`OnceLock`-cached), `src/commands/sync/rename.rs:18–51`,
`src/commands/sync/apply/phases.rs:463–569` (`GATE_COPY`), `src/scan.rs:31–47`.
Test users: `tests/sync.rs:826,935,975,1024,1195,1425–1428,1526–1531,1678–1759`
and `tests/output.rs:740`.

## Implementation plan

Code anchors (validate before editing): all read sites and test users above.

Steps (large diff — multiple commits expected):

1. Group the hooks into
   `#[derive(Default)] pub struct TestHooks { cache, commit, fail_copy,
   fail_rename, gate_copy, heartbeat_entries, … }`.
2. Add `hooks` to `CommonOpts` (default = production env behavior); read from
   `common.hooks` instead of `std::env`; let `run_captured` (T16) accept a
   `TestHooks`.
3. Keep the env path as a fallback so `.config/nextest-test-env` still works.
4. Rewrite the failure-path `sync.rs` tests to inject hooks in-process.

## Acceptance criteria

* Same injected failure points and error strings as the env hooks.
* ~15 spawns unlocked; ~20 `env(...)` calls removed; suite works under plain
  `cargo test`.
* Default (production) behavior unchanged. Affected suites pass.

## Guardrails

* **Medium-large diff across ~8 production files.** The `OnceLock` in
  `copy.rs` must be replaced carefully — preserve its caching semantics only as
  an env fallback, not as an injected-hook blocker.
* Sequence after T16. Do not duplicate T15's heartbeat work — if T15 already
  exposes a heartbeat period, fold it into `TestHooks` rather than re-adding a
  separate knob.
* Report follow-up ideas without implementing them.
