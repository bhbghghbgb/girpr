# T06 — `AtomicU64::fetch_update` + simplify the test gate

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B3 (lines 418–423).

## Scope
- Package: root `girpr`, `src/util/counter.rs`.

## Implementation plan
1. Replace the ~10-line saturating CAS with
   `fetch_update(Relaxed, Relaxed, |c| Some(c.saturating_add(n)))`.
2. Replace the custom `Gate` and thread-local CAS-pause race harness with
   `std::sync::Barrier` or `tokio::sync::Notify`.
3. The five race tests that pin the harness (lines 274–579) may be rewritten or
   removed along with it.

## Constraints / validation
- This is a test-code reduction, not a library adoption; production saturating
  behavior must be unchanged.
- Do not silently lose race coverage — rewrite rather than delete if the race
  is still meaningful.

## Done when
The hand-rolled CAS and gate harness are gone and production behavior is
unchanged.
