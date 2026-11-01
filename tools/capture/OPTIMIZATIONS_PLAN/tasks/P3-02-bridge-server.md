# P3-02 — Bridge server correctness and locking

Report: `../../OPTIMIZATIONS_REPORT.md` lines 590-619 (C8), 754-784 (E4-E6).
Test-only.

Dependencies: P0-02.

## Goal

Fix three bridge-server issues:

1. `Served::manifest_index` is keyed by manifest id alone. If old and new
   builds declare the same id, the second insert overwrites the first and the
   route can serve the wrong tag's archive or reject a legitimate request.
2. `server::error_shape` takes an `extra` parameter and discards it.
3. `MANIFEST_CACHE` holds a global mutex while doing the blocking disk read,
   serializing concurrent first requests.

Keep-alive (`Connection: close`) is optional. If it is cheap to add without
complicating the tiny server, do it in a separate commit; otherwise record it
as a follow-up.

## Where to start

- `tests/artifact_bridge/server.rs`: `Served` (around line 260),
  `manifest_index`, `Served::load` (around line 440), the manifest route
  (around line 573), `manifest_resolution` (around line 411),
  `declared_manifest_ids`, `error_shape` (around line 616), `MANIFEST_CACHE`
  (around line 741), `manifest_bytes` (around line 743), the response writer
  (around line 877).
- `tests/artifact_server.rs`: shared-id and route tests.

## Steps

1. **Manifest identity.** Key the index by `(build_id, id)` or store a
   `Vec<ManifestEntry>` per id and select by the route's build id. Update the
   route lookup to include the build segment. Keep a public
   `manifest_resolution(id)` for tests only if it can return unambiguous
   information; otherwise add `manifest_resolution_for(build_id, id)` and update
   tests. `declared_manifest_ids` should still list ids (deduplicated).
   The `build_id` check must continue to reject a request for a build that did
   not declare that id.
2. **Dead parameter.** Remove `extra` from `error_shape` and every call site.
   Do not replace it with an ignored variable.
3. **Cache lock.** Change `MANIFEST_CACHE` to an `RwLock` (or a
   `Mutex<HashMap<PathBuf, Arc<...>>>` with a double-checked read):
   - take a read lock and return a hit;
   - on a miss, release the lock, read the file, then take a write lock and
     insert unless another thread already did;
   - never hold a lock across `std::fs::read`.
   Alternatively use `OnceLock` per path inside the map.
4. **Keep-alive (optional).** If implemented: accept multiple requests per
   connection, parse `Content-Length`/request end, and send
   `Connection: keep-alive` unless the client asks to close. Keep the server
   single-threaded per connection and abort cleanly on drop. If this grows the
   diff substantially, skip it and return a follow-up.
5. Update the module docs that describe the id index and cache.

## Tests

- `artifact_server`: a mini-bundle where old and new declare the same manifest
  id. Both builds' routes serve their own archive; a request with the wrong
  build id is 404 (or the recorded failure shape), not the other build's bytes.
- `artifact_server`: `declared_manifest_ids` contains the shared id once.
- Existing manifest route tests still pass.
- If keep-alive is implemented, a test sends two requests on one connection and
  gets two responses.
- Flip the P0-02 shared-id characterization test.

## Acceptance

- A shared manifest id can no longer overwrite another build's entry.
- No dead `extra` parameter remains.
- No lock is held during disk IO.
- `cargo test -p girpr --test artifact_server` passes.
- No production code changes.

## Commit plan

1. `fix(bridge): key manifest entries by build and id`
2. `chore(bridge): drop the unused error-shape parameter`
3. `perf(bridge): do not hold the manifest cache lock across a read`
4. Optional: `perf(bridge): keep connections alive`

## Notes

- This is test infrastructure; do not change the bundle format or production
  verify behavior.
- Do not reference the report or plan in code comments.
