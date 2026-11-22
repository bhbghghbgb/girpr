# T21 — `girpr-http-transport` assessment (high-risk, likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F26 (lines 412–422) — the HTTP
  transport wrapper is duplicated but its ownership choice is deliberate; candidate
  boundary is a program-free leaf with `HttpRequest`/`HttpResponse`/`HttpTransport` and the
  retry driver, classifiers local. **High-risk** because it reverses the documented reason
  `gircap-http` exists and would couple three programs' reqwest features. Confidence high
  on duplication, medium on extraction.
- Assumptions to validate: §A2 (29–36), §A4 (48–56), §A9 (88–97), §A14 (129–135), §N10 (734–737).

## Scope
- `tools/capture/http/src/transport.rs` (`ReqwestTransport`, 500ms/10s/zero-jitter policy),
  `crates/fetch/src/retry.rs` (1s/30s/±50% `BackoffPolicy`), root `Cargo.toml` reqwest dep.
  Locate before editing.

## Implementation plan
1. Validate §F26: confirm the deliberate reason `gircap-http` exists — keeping reqwest's
   native tree cached and no transport dependency crossing into gircap.
2. Assess whether a shared leaf would couple three programs' reqwest features (§A2) and
   violate gircap independence (§A4) / the four deviations (N10). If so, record a validated
   skip with the one-line reason.
3. Implement only on hard evidence that features stay uncoupled and gircap stays clean.
   If done, move through §A14 seams.

## Constraints / validation
- gircap must not gain a transport dependency crossing into it (§A4, N10). Retry jitter
  0.0 vs ±50% is a keeper (§A9) — classifiers and jitter stay local.
- Do not couple reqwest features across programs (§A2).

## Done when
A validated decision is recorded: either a safe uncoupled leaf with unchanged behavior, or
(expected) `skipped` with the §A2/§A4/N10 reason.
