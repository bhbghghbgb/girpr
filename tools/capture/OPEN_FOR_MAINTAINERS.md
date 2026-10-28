# Open for maintainers — `gircap` (`tools/capture`)

Deliberate trade-offs, unverified areas and product decisions on `fix-gaps/gircap`, not open defects.
Context, pass log and verification: [ADVERSARIAL_REVIEW.md](ADVERSARIAL_REVIEW.md).

- **F4 dedicated field.** A `fetch_error` field would describe a failed fetch more precisely but needs an artifact-schema change; a panicking chunk still gets the stable generic row text.
- **F7 ceiling intent.** If `--max-download-bytes` was meant as an input-independent policy check, the metadata-only skip should be reverted and the help changed instead.
- **F8 replay.** The recorded argv is lossy; replaying a bundle would need a raw-bytes companion.
- **F11 older orphans.** Cleanup covers handled errors only; a panic unwinds past it, orphans from older binaries are not swept, and the historical accumulation rate is unmeasured.
- **F6/G1 live inputs.** Whether a live API echoes a hostile tag, serves a hostile document or audio path, or uses an unusual but legitimate chunk-id spelling is unverified; the local sinks and rejection paths are tested.
- **L1 end to end / cross-host.** No capture was run with a device-name tag — it is now refused at argument resolution, before any request — and a Linux-produced `manifests/CON/` bundle was not replayed through `verify` on Windows; A16's missing-file consequence is reasoned, not executed.
- **L2 through the command.** The near-`u64::MAX` cost is driven through `select` with a `PathScore` fixture, not through a hand-edited manifest read from disk; no live manifest declaring extreme sizes was sampled.
- **M1 through the command.** The crafted `checksums.json` is driven through `Checksums::findings`, not written into a bundle and run through `gircap verify`; `Bundle::open` → `reverify` → `findings` is an unchanged direct call.
- **M2 cancellation.** Cancellation is exercised on the `join_failure` helper, not through `map_concurrently`, which never aborts a spawned task; the slot text for a cancelled metadata job was not driven end to end.
- **N1 panic seams.** Both direct `spawn_blocking` joins now share `tasks::join_blocking`, and the panic mapping plus the published-blob discard are driven with a panicking closure at the same seam the production call sites use. No portable input makes `BlobProps::measure_file` or `emit::write_requests_log` panic, and cancellation at those two joins was not driven. A measure task that panics removes the already-published blob, because the row is recorded with `stored_path: null`; that loss is deliberate and differs from K2, where an undecodable body is kept as evidence.
- **G2 crash tear.** Whether a metadata document can be left torn in a way `checksums.json` misses was not stress-tested.
- **H3 live manifests.** Whether a live manifest contains names differing only by a trailing dot or space was not sampled.
- **H4 on-disk bundle.** No real `--no-chunk-blobs` bundle was inspected; the doc matches the constructor and the tests.
- **I4/J1 ACLs.** No real Windows ACL-denied directory or `MANIFEST.json` was set up; the read arms are injected, the J1 share lock was reproduced end to end, and the stat/read race was reasoned, not timed.
- **I5 CLI precondition.** A hostile tag reaches the writer only through a hand-built manifest; the CLI's earlier validation is not re-driven, though the writer's refusal is unit-tested.
- **F5 production bodies.** Real non-2xx body sizes are unknown, so the practical effect of the cap is unverified.
- **F1 real corpus.** Whether `blobs::plan`'s projected check also fires on a real corpus is unverified; the selection-level test is the guarantee.
- **F8 clap typing.** Which non-`PathBuf` argument types clap accepts non-Unicode values for is unverified; only `--out` is exercised end to end.
- **K1 A20 through the command.** A20's negative and overflowing cases are unit-tested against a crafted `PatchMap`, but no full bundle with a hand-edited patchmap was run through `gircap verify`.
- **K2 removal arm.** The published blob is removed on a post-rename measurement I/O error, but no test forces that I/O failure; the undecodable path is the tested post-rename path and deliberately keeps the blob.
- **K1/K2 live inputs.** Whether a live CDN serves a non-frame chunk, or a live manifest declares extreme or negative numbers, remains unmeasured.
