# T14 — Artifact-server fixtures → wiremock

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-11 (lines 391–402) — the raw
  accept loop, manual target splitting and percent decoding in `app/test-fixtures`
  should use `wiremock` mocks plus `url`/`percent-encoding`.
- §X1 (lines 56–89) — cross-cutting HTTP-server finding and test-impact guidance.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28), §A10 (39–40).
  Keep items: `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/test-fixtures/src/artifact_server.rs` — `accept_loop`/`serve_one` (`:930–1030`),
  `query_value`/`urldecode` (`:1063–1098`), header formatting and request journal.
- `app/test-fixtures/src/lib.rs:926–962`, `:1525–1543`, `:70–76`.
- Consumers of `MockServer::chunk_hits`, `request_targets`, `first_target_with`,
  `truncate_bytes` and `on_first_chunk` in `app/fetch/tests/*` and `app/audio/tests/*`.
- `app/test-fixtures/Cargo.toml`. Locate all consumers before editing.

## Implementation plan
1. Inventory the routes that must be mounted (HYP/getBuild/getPatchBuild/manifest/chunk)
   and the scripting hooks consumers rely on. Keep a thin custom server only where
   `wiremock` cannot express a deliberately truncated body or an `on_first_chunk` hook
   (§APP-11 line 399).
2. Mount `wiremock` mocks for each route, using `received_requests()` for the journal;
   keep the fixture-building and data-table policy as mock state.
3. Replace the hand-written query/percent decoding with the helpers chosen in T09's
   recorded spelling decision (or local `form_urlencoded`), and delete the local helpers
   where they have no callers.
4. Rewrite the affected `app/fetch` and `app/audio` tests; keep semantic assertions about
   which chunks were requested and how many times, regenerating target strings.

## Constraints / validation
- Test-only blast radius (§A10). No production path changes.
- The captured URL spelling must match T09's recorded `game_ids[]` decision; coordinate
  through `STATE.md` Persistent context.
- Truncated-body and hook behaviors must survive — retain a small server if `wiremock`
  cannot express them (§APP-11 lines 399–402).

## Done when
The raw accept loop and decoder are gone (except any justified thin server), journals
translate correctly, and the fetch/audio suites pass with regenerated assertions. If a
premise is stale or a keeper would change, **stop** and record `skipped`.
