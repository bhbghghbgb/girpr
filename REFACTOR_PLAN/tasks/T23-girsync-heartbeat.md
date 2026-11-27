# T23 — Girsync progress heartbeat

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share the progress-heartbeat loop across girsync's scan and effective phases
with an injected trigger predicate.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:712-716` — SYNC-12, three heartbeat loops.

## Boundary / approach

Add a `Heartbeat` struct in `girsync-leaves` with an injected trigger predicate
and adopt it in `scan`, `effective/phase_a` and `effective/phase_c`.

## Guardrails

- The shared `elapsed_s` utility stays as-is.
- The test-injected `GIRSYNC_TEST_HEARTBEAT_ENTRIES` seam must survive.
- Heartbeat cadence observable in tests must not change.

## Commit breakdown

1. Add `Heartbeat` leaf with trigger injection and tests.
2. Adopt in `scan`.
3. Adopt in `effective/phase_a` and `effective/phase_c`.

## Validation

- Heartbeat seam tests; `cargo nextest` for girsync leaves and the main tool.

## Done when

One heartbeat implementation drives all three phases with unchanged cadence.
