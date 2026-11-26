# T09 — URL assembly & percent-encoding → url / form_urlencoded

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §X2 (lines 90–109) — two hand-written
  URL grammars and three hand-written percent encoders/decoders should use `Url`,
  `Url::query_pairs_mut()`, `Url::join` and `form_urlencoded`; decide **once per
  program** how bracketed keys like `game_ids[]` are spelled.
- §APP-2 (lines 281–294), §APP-5 (321–332), §CAP-3 (689–700), §CAP-4 (702–710) —
  the affected call sites in fetch, dispatch/persistent-plan and capture.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md` (non-findings at lines 822–823, 852–853).

## Scope
- `app/fetch/src/hyp.rs` — `hyp_url` (`:401–416`), `sophon_build_url`/`patch_build_url`
  (`:669–696`), `urlencode` (`:719–735`); exact-string tests at `:1158`, `:1181`, `:1204`.
- `app/dispatch/src/url.rs` — `build_dispatcher_url` (`:104–136`),
  `validate_dispatcher_url` (`:138–164`); `dispatch.rs:172–179`, `:275–307`;
  `app/persistent-plan/src/placement.rs:142–184`; `dispatch/src/tests.rs:244`, `:591`.
- `tools/capture/src/biz/mod.rs` — URL builders (`:195–228`), `push_pair` (`:318–324`),
  `enc` (`:326–351`); `tools/capture/src/http/probe.rs:577–611`, `:689–735`;
  `tools/capture/src/http/tests.rs:9–30`.
- `app/test-fixtures/src/artifact_server.rs:1063–1098` (`query_value`/`urldecode`) —
  becomes unnecessary. `tools/capture/test-support/src/lib.rs:1915–1923`, `:2097–2130`.
- Locate every occurrence in the current tree before editing; the list above is a hint.

## Implementation plan
1. **Decision first.** Choose whether `game_ids[]` is sent literally or as `%5B%5D`,
   considering every consumer above. Record the decision in `STATE.md` Persistent
   context (the orchestrator passes it to later URL tasks). If byte-stable spelling is
   genuinely required, keep an exact `AsciiSet` with already-locked `percent-encoding`
   instead of `Url`'s default escaping.
2. Store base URLs as `Url` at parse time; replace `format!` query building with
   `query_pairs_mut().append_pair`, dispatcher host forcing with `Url`, CDN URLs with
   `join`/`set_path`, and query validation with `query_pairs()`.
3. Replace the two hand-written decoders with `form_urlencoded::parse` /
   `Url::query_pairs()`; delete `urlencode`, `enc`, `push_pair`, `query_value` and
   `urldecode` where they have no remaining callers.
4. Regenerate exact URL pins across fetch, dispatch and capture; keep the semantic
   assertions about which parameters are sent.

## Constraints / validation
- One spelling decision for all products — do not let fetch and capture diverge.
- `url`, `form_urlencoded` and `percent-encoding` are already locked; re-check versions.
- Do not use `Url` where it would change a security-relevant host/path check in
  dispatch; validate the forced-host semantics against current tests.
- Golden files and recorded request URLs may change spelling; regenerate (§A4/§A5).

## Done when
No hand-written URL grammar or percent encoder/decoder remains, the recorded spelling
decision is applied consistently, pins are regenerated, and the fetch/dispatch/capture
tests pass. If a premise is stale or a keeper would change, **stop** and record `skipped`.
