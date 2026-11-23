# T19 — Cut allocations in gircap's HTTP recording pipeline

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-01** (lines 643–652) — `HttpResponse::chunk()`
  copies reqwest's refcounted `Bytes` into a `Vec` per body chunk; return `bytes::Bytes`
  and write `&chunk` directly, wrapping `Memory` chunks with `Bytes::from(vec)`.
- §9 **TOOL-07** (691–696) — Content-Type is built twice per request; compute
  `resp_headers` first and read `content_type` from it.
- §9 **TOOL-08** (697–702) — `header_map` allocates a `Vec` per header name; build
  the `String` directly with `push_str` and `'\n'` separators.
- §9 **TOOL-09** (703–708) — base request headers are deep-cloned per logged request;
  store `Arc<BTreeMap<...>>` (serde serializes through `Arc`), with a fresh map only
  for the `Range` case.
- §9 **TOOL-10** (710–715) — `Fetcher` clone deep-copies header maps per chunk task;
  wrap in `Arc<Fetcher>` once and pass `Arc` to the task closure.
- §11 **assumption 14** (lines 875–877) — recorded request/response JSON is pinned by
  the artifact bundle; confirm it stays byte-identical.
- Keep items: recorded JSON / artifact bytes, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/http/src/transport.rs` (`chunk()` ~236–243, `drain_capped`
  `fetcher.rs:188–199`).
- `tools/capture/http/src/fetcher.rs` (Content-Type ~651–656, 969–973, header clone
  ~1134, `Fetcher` clone ~476–491).
- `tools/capture/http/src/record.rs` (`req_headers` ~133–145, `header_map` ~176–198).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 14: confirm what the artifact bundle pins and establish a
   byte-for-byte check for recorded request/response JSON. If the bundle contract
   cannot be preserved, stop.
2. Return `bytes::Bytes` from `chunk()` and write `&chunk` into sinks; wrap `Memory`
   chunks with `Bytes::from(vec)` (TOOL-01).
3. Compute `resp_headers` once and derive Content-Type from it (TOOL-07).
4. Build header strings with `push_str` (TOOL-08) and share base headers via
   `Arc<BTreeMap<...>>` (TOOL-09).
5. Use `Arc<Fetcher>` for chunk tasks instead of deep-cloning (TOOL-10).
6. Deliver in several commits if useful: zero-copy chunk, header plumbing, Arc sharing.

## Constraints / validation
- Recorded request/response JSON must stay byte-identical (assumption 14); add a
  fixture comparison if one is not already present.
- Do not change artifact bytes or any §12 non-finding.

## Done when
The HTTP pipeline no longer copies body bytes or clones header maps per request/task,
and a byte-for-byte fixture check confirms recorded JSON is unchanged. If the bundle
contract cannot be preserved, **stop** and record `skipped` with a one-line reason.
