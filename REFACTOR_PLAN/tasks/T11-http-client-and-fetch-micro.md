# T11 — Add HTTP timeouts, status checks and fetch-layer micro-optimizations

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §5 **NET-01** (lines 349–358) — no connect or overall
  timeout on the shared client; add connect and per-stream idle timeouts (not a
  blanket total that breaks large slow streams). **Assumption 6** (lines 853–855).
- §5 **NET-02** (lines 360–368) — `get_node` never checks HTTP status; call
  `error_for_status_ref()` before reading the body. **Assumption 7** (lines 856–858):
  no production endpoint returns a non-2xx body with a usable `retcode`.
- §5 **NET-03** (lines 369–375) — body-read errors are fatal; make them retryable in
  both JSON paths (the blob path already does).
- §5 **NET-04** (lines 376–385) — deterministic wrapper-parse failures and
  `data: null` are retried 5×; make wrapper-parse errors `Fatal` in `get_direct` and
  map `data: null` to a clean not-found.
- §4 **MEM-05** (lines 337–345) — consume the owned JSON value instead of deep-cloning
  the node before deserializing.
- §8 **MIC-04** (591–595), **MIC-05** (596–599), **MIC-12** (630–636) — avoid a
  `String` per escaped byte in `urlencode`, stop cloning base-URL `String`s per
  request, and normalize the deprecated list directly into its final buffer.
- Keep items: recorded JSON / wire behavior (assumption 14), §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/fetch/src/hyp.rs` (client builder ~298–302, `get_node` ~354–371, `get_direct`
  ~412–427, `data: null` ~450–451, JSON clone ~383–388, `urlencode` ~547–557,
  base URL accessors ~315–321, 333–345, 482, 536, deprecated ~486–493).
- `src/repair/mod.rs` (deprecated list ~1172–1181).
- Captured bundles for status/timeouts validation. Locate current code before editing.

## Implementation plan
1. Validate assumptions 6–7 in the current tree: confirm a per-stream idle timeout is
   viable and that no production response relies on a non-2xx body with `retcode`.
   If not, narrow or stop.
2. Add connect and per-stream idle timeouts to the client / body read.
3. Add `error_for_status_ref()` in `get_node`; make body-read errors retryable in both
   JSON paths; make wrapper-parse errors fatal and map `data: null` to not-found.
4. Consume the owned JSON (`as_object_mut().remove(node)`) instead of deep-cloning
   (MEM-05); apply the urlencode, base-URL and deprecated-list micro fixes.
5. Deliver in several commits if useful: timeouts, status/retry classification,
   JSON/micro changes.

## Constraints / validation
- Timeouts must not break large chunk streams on slow links (assumption 6).
- Status handling must not reject a legitimate production response (assumption 7).
- Recorded request/response JSON stays byte-identical where the artifact bundle pins
  it (assumption 14); do not change the JSON/text stdout contract.
- Do not change any §12 non-finding.

## Done when
Timeouts, status checks and retry classification are in place, JSON cloning and the
micro hot spots are removed, and fetch tests pass. If an assumption fails, **stop** and
record `skipped` with a one-line reason.
