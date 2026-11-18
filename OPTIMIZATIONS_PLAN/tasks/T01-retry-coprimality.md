# T01 — `retry.rs` injectivity by sampling → coprimality proof

Priority P0 · root product unit tests · checkpoint CP1 · report finding C1

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L344–L367` (§C1).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L800–L804`
  (pattern 3), `L812–L814` (Wave 1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T01-retry-coprimality.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Replace a 20,480-call sampling "proof" of jitter injectivity with a scale-free
arithmetic proof that is strictly stronger, without changing production
behavior.

`src/retry.rs` `jitter_is_injective_over_an_attempts_caller_range` (:298–312)
calls `jitter(attempt, salt)` for `CALLERS = 4096` salts over 5 attempts and
inserts each spread into a `HashSet`. That samples ~0.0016% of the real period
(the first repeat is a salt difference of `RANGE = 250_000_000`).

## Implementation plan

Code anchors (validate before editing):

* `src/retry.rs:298–312` — the test.
* `src/retry.rs:138–146` — production `jitter`, with `RANGE` (:139),
  `MULTIPLIER` (:140) and the per-attempt constant
  `0x9E37_79B9_7F4A_7C15`.
* `src/retry.rs:284` — `backoff_is_linear_with_a_bounded_spread` (retained as the
  gross-regression guard).

Steps (one commit):

1. Lift `RANGE`, `MULTIPLIER` and the per-attempt constant to module-level
   `const`s — no behavior change.
2. Replace the loop with a proof: for fixed `attempt`,
   `jitter_nanos(salt) = (salt·M + c) mod RANGE` is a bijection over the full
   period exactly when `gcd(M, RANGE) == 1`. Assert coprimality with a ~10-step
   Euclid loop (`RANGE = 2^7·5^9`; `MULTIPLIER = 0x9E3779B1` is odd and not
   divisible by 5). Keep a 16-salt empirical sanity loop for a concrete example.

## Acceptance criteria

* The old 4096-salt / 20,480-insert sampling loop is gone.
* The coprimality proof is over the full period and is derived from the same
  constants production `jitter` uses (not duplicated literals that can drift).
* `backoff_is_linear_with_a_bounded_spread` remains as the linear-regression
  guard.
* Production `jitter` behavior is byte-for-byte unchanged. Affected unit suite
  passes.

## Guardrails

* If the constants differ from the report snapshot, derive the proof from the
  actual code values; if `jitter` is no longer affine, skip and report why
  rather than asserting a false property.
* Test-only change apart from lifting constants. Report follow-up ideas without
  implementing them.
