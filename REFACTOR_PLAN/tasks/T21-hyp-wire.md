# T21 — HYP wire DTOs and API identity

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Give envelope DTOs, `terminal_retcode` and the configurable number decoder one
home, and define the duplicated `User-Agent` identity once.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:742-746` — CAP-02, User-Agent literal.
- `docs/09-shared-code-refactor-findings.md:788-794` — CAP-09, envelope DTOs.

## Boundary / approach

Add a `girpr-hyp-wire` leaf (or extend `crates/proto` if that is the better home)
for the envelope DTOs, `terminal_retcode` and a caller-configurable number
decoder. Define the API-identity `User-Agent` constant once, in `crates/model`
or a small leaf, preserving capture's re-export through `logging`.

## Guardrails

- The two programs disagree on the meaning of a non-zero `retcode`; do not
  converge it.
- Girpr's raw-byte retention and `SophonNotFound` mapping, and capture's typed
  dumps, are policy.
- Golden and test `User-Agent` literals are pinned.
- Capture's re-export through `logging` stays.

## Commit breakdown

1. Add the User-Agent constant and adopt both producers.
2. Add envelope DTOs and configurable number decoder with tests.
3. Adopt capture `hyp` and `fetcher`.
4. Adopt girpr `hyp` without changing raw-byte behavior.

## Validation

- Typed dump tests and golden NDJSON byte equality.
- `cargo nextest` for the wire leaf, capture and fetch.

## Done when

Envelope handling and API identity have one definition each, with program
policies untouched.
