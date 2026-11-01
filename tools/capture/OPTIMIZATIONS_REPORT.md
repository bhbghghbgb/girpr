# gircap and artifact-bridge review

Date: 2026-11-01

Scope: `tools/capture` (the `gircap` crate) and `tests/artifact_bridge*` in
`girpr`. This is a review of clean-code, verification, correctness, and
runtime/resource opportunities. It is not an implementation plan and it does
not prescribe line-level edits.

The crate's own design notes mark several choices as deliberate; those are not
re-reported here. In particular: the hand-rolled JSON writer, the two-pass A16
fixpoint, single-pass range logic, byte movement through assembly, the biz/exit
tables, `Box::leak`, prost decoding, and `unknown_field6` are out of scope as
"bugs".

## How to read this

- **Confirmed**: the behavior is visible in the named module or symbol.
- **Latent**: the code path is real, but live data or current flags may not
  exercise it.
- **Risk** is blast radius, not severity: how much artifact-visible behavior or
  consumer expectation changes if the finding is acted on.
- **Gain** is the value of fixing it, not the size of the diff.

Severity shorthand: **S1** can produce a wrong or misleading bundle/report;
**S2** is a verification hole or a real edge-case failure; **S3** is
performance/cleanliness with little or no behavioral risk.

---

## Executive summary

1. Two user-facing correctness bugs are worth fixing before anything else: a
   resolved old/new tag can be identical while the run still exits `0`, and any
   captured file whose name ends in `.gz` is treated as a gzip document during
   close-out, which can abort the bundle or record the wrong digest.
2. The verification layer is strong on re-hashing blobs and weak on
   cross-document consistency. A11 in `verify` is tautological, A14 checks the
   writer's intended file list rather than `game/` on disk, the published
   `checksums.json` is not independently checked, and a patchmap's `plan[]` is
   not cross-checked against `chunks/index.json`.
3. Selection and reporting have two different orderings. The artifact's
   `candidates.json → rank` is not the order `select` actually uses, and
   `SizedCandidate::budget_cost` does not match its own documentation for
   `added` files.
4. The largest avoidable runtime cost is repeated full-tree hashing, especially
   in the bridge: each of six tests re-runs `gircap verify` (which re-hashes the
   whole bundle) and rebinds four listeners.
5. Many remaining items are low-risk allocation, locking, and dead-code fixes;
   they should follow the correctness work, not lead it.

---

## A. Correctness and data integrity

### A1. Resolved old and new tags can be the same (S1, confirmed)

`config::capture_plan` rejects identical tags only when both flags are present
and only before resolution. `VersionPair::select` resolves `latest` to
`main.tag` and the default old tag to `diff_tags[0]`, but never compares the
resolved strings. If the API returns a branch head that also appears first in
`diff_tags`, the run proceeds, diffs a manifest against itself, captures
"identical" files, and can still exit `0`.

`manifest::diff::PathDiff::is_a_real_upgrade` already expresses the intended
invariant, but it has no production caller. The fix is conceptually small: make
the resolved pair's inequality a precondition, and decide whether a deliberate
same-tag recapture deserves an explicit opt-in flag.

- Gain: prevents a useless bundle and a misleading success.
- Risk: low blast radius; a new refusal is observable, but no artifact bytes
  change for valid runs.
- Tradeoff: if same-tag recapture is a legitimate workflow, an opt-in keeps it
  possible; otherwise the refusal is simpler.

### A2. `encoding_of` classifies by filename, not by content (S1/S2, confirmed)

`emit::index::encoding_of` treats every path ending in `.gz` as
`hash::Encoding::Gzip`. `digest_file` then pipes that file through
`GzDecoder`. A captured game asset that happens to be named `*.gz` but is not
gzip will fail close-out with a decode error, or, if it is coincidentally a
gzip stream, will be hashed as its decompressed bytes while the file on disk is
the compressed form. Either outcome is wrong for a byte-exact fixture.

This is a filename heuristic standing in for a fact the writer already knows:
the writer decided to gzip a document. The durable fix is to carry the encoding
with the write result or derive it from the document kind, not from the final
path suffix. A narrower stopgap is to restrict the suffix rule to the documents
this tool itself compresses.

- Gain: captures arbitrary game assets named `*.gz`.
- Risk: medium, because it changes `checksums.json` encoding for existing
  `.gz` documents; test fixtures and published bundles must be checked.
- Tradeoff: carrying encoding metadata through every write is more plumbing;
  a whitelist of gzip-eligible document paths is less general but smaller.

### A3. Folder entries can enter selection and materialization (S2, latent)

`manifest::diff::diff_field` iterates every `SophonChunkFile`, including
`is_folder` entries. `select::candidates` turns those rows into candidates.
`assemble`/`materialize` then treat a folder path as a zero-byte file. Live
manifests currently contain no folders, so this is latent, but a future build
could publish one and the matrix could admit it.

The decision should be explicit: folders are either excluded from the diff
candidate pool, or represented as directories end-to-end. The current
middle ground silently produces a file where the manifest said folder.

- Gain: avoids a meaningless capture and a wrong fixture.
- Risk: medium if folder counts or `diff/paths.*` are expected to include them;
  low if folders are filtered before scoring.
- Tradeoff: excluding folders changes aggregate counts for data that has them;
  representing them properly changes the on-disk layout and consumers.

### A4. Duplicate manifest paths are recorded but the TSV picks a winner (S2, confirmed)

`manifest::dump::paths_tsv` sorts path strings and then indexes a
`HashMap<&str, &SophonChunkFile>`. With duplicate paths, the map keeps the last
entry; every duplicate row for that path prints the last entry's size, md5,
chunk count and folder flag. `decode_counts` already counts
`duplicate_paths`, so the artifact knows the input is malformed, but the sidecar
still misreports it.

The fix is to sort references (or indices) rather than path strings, or to emit
one row per entry in a defined order. This is independent of the documented
"no tab escaping" limitation.

- Gain: sidecar remains truthful on malformed input.
- Risk: low; affects only duplicate-path inputs.
- Tradeoff: sorting references needs a stable tie-break (entry index) to remain
  deterministic.

### A5. Chunk URL suffixes are dropped by `blobs::Sources` (S1/S2, confirmed)

`CapturedManifest::chunk_url` correctly appends
`chunk_download.url_suffix`, and `hyp::types` documents the measured
`{prefix}/{id}{suffix}` join. `blobs::Sources::from_manifests` stores only
`chunk_url_prefix`, and `Sources::url` returns `{base}/{id}` with no suffix.

Every response seen so far has an empty suffix, so live runs work. A response
with a non-empty suffix would make every chunk URL wrong, and the error would
surface as CDN 404s rather than as a suffix bug. This is a genuine latent
breakage in the chunk store, not just a cosmetic inconsistency.

- Gain: correct URLs for any server that uses a suffix.
- Risk: low; empty-suffix behavior is unchanged.
- Tradeoff: `Sources` must either store a base plus suffix per tag, or store a
  closure/URL template; the former is simpler.

### A6. A14 checks the writer's list, not `game/` on disk (S2, confirmed)

`emit::selfcheck::closing` builds `game_files` from the `written` vector passed
by the capture pipeline. A stray file that already exists under `game/` but was
not written by this run is invisible to A14. The check's stated claim is about
the tree ("contains no path outside..."), so it should enumerate disk, or at
least reconcile the written list against a directory walk.

This matters most when a bundle directory is reused or pre-created, or when a
previous failed run left files behind. The capture id is timestamped, so the
normal path is a fresh directory, but "normally fresh" is not the assertion's
claim.

- Gain: A14 means what it says.
- Risk: medium; a dirty tree that previously passed will now fail. That is the
  intended effect, but it can surface in test setups.
- Tradeoff: an extra directory walk at close-out; cheap relative to hashing.

### A7. A11 in `verify::materialize::rebuild` is tautological (S2, confirmed)

`rebuild` accumulates `declared_size` as the sum of chunk lengths, then calls
`Layout::of(parts, declared_size)`. `Layout::of` checks `total != declared_size`,
but `total` is itself the sum of the same lengths. The size half of A11 can
therefore never fail. Gap/overlap/ascending checks still run and are useful; the
"sum equals the side's declared size" claim does not.

There is a related nuance: `verify::checks::check_pair` compares the rebuilt
byte length against the patchmap side's `size` under A7/A8. So a wrong side
size is not completely invisible. But A11's own row is still a false claim, and
a consumer reading A11 alone would be misled. The fix is to pass the declared
side size from the patchmap into the layout check, or to stop claiming the size
half in A11 and rely on A7/A8 explicitly.

- Gain: verification rows become independently meaningful.
- Risk: medium; could turn previously green bundles red if their side sizes
  disagree.
- Tradeoff: either A11 grows a parameter or the assertion wording/ownership
  must change; the former is less disruptive to consumers.

### A8. The published `checksums.json` is not independently checked (S2, confirmed)

`emit::closeout::close` writes `checksums.json` twice. A16 is computed from the
first document (`checksums.a16(i.dir)`). The second document is produced by
`write_checksums_reusing` with the first document's entries and two rewritten
paths, then published without another `a16`/`findings` pass.

The second scan is probably correct because the rewritten list includes the two
changed documents, but "probably correct" is exactly what A16 exists to
replace. If the reuse contract is ever violated, the published checksums can be
wrong while the selfcheck says A16 passed.

- Gain: the document a reader trusts is the one that was checked.
- Risk: medium; a late discrepancy becomes exit `3` rather than a silent pass.
- Tradeoff: one more full re-hash, unless the check is narrowed to the rewritten
  paths plus a membership check. A targeted re-hash of the rewritten files plus
  the existing first-pass A16 would preserve most of the cost saving.

### A9. Only `MANIFEST.json`'s schema version is validated (S2, confirmed)

`verify::Bundle::open` reads `MANIFEST.json`'s version first and refuses a
mismatch, but `chunks/index.json`, `verify/selfcheck.json`,
`api/error_shapes.json`, `api/error_responses.json`,
`diagnostics/rejections.json`, `diagnostics/path_map.json`, and every
patchmap are deserialized without a version check. A current `MANIFEST.json`
paired with an old or hand-edited patchmap can still open, and the reader will
apply current semantics to a document that may mean something else.

`patchmap.expectations.result_path` is also not covered by
`validate_document_paths` (which validates index `stored_path`, index reference
paths, patchmap `path`, error-response paths and the path map). `result_path` is
a document-supplied path that consumers use to locate the expected bytes.

- Gain: schema drift is diagnosed at the document that caused it; path-shaped
  fields are all checked at the door.
- Risk: low-to-medium; old bundles that currently open will be refused or
  reported, which is the intended contract.
- Tradeoff: each document type needs a single version predicate; doing it
  generically avoids seven bespoke checks.

### A10. `verify` does not cross-check a patchmap plan against the index (S2, confirmed)

`verify::checks::check_patchmap_arithmetic` re-derives `totals` and several
expectations from `plan[]`, but it treats `plan[]` as the source of truth. The
rebuilt old/new sides come from `chunks/index.json`; the plan's chunk ids,
target ranges, source ranges, decisions and URLs are not compared against those
rebuilt sides or the index references. A tampered plan with self-consistent
counts and byte totals can pass even if it describes a different patch from the
one the blobs can actually produce.

The bridge is especially exposed here: it trusts `plan[]` for the FETCH/REUSE
sets, so a wrong-but-consistent plan makes the cross-tool test assert the wrong
thing.

- Gain: `verify` proves the plan and the blob closure describe the same patch.
- Risk: medium; a previously accepted bundle with a stale plan will fail.
- Tradeoff: requires a canonical mapping from plan entries to index
  `(role, path, offset, chunk_id)` and a decision on duplicate placements.

### A11. `verify::list` stops descending at a non-bundle `MANIFEST.json` (S2, confirmed)

`verify::list::list_with` treats any directory with a `MANIFEST.json` as a
bundle boundary. When `probe_manifest` returns `NotABundle` (unparseable JSON
or no `schema_version`), `Bundle::open` fails, the code does not add an
`Unreadable` row (that branch is only for `Version`), and it `continue`s
without descending. A nested bundle under that directory is hidden.

The doc comment distinguishes "not a bundle" from "a bundle this build
refuses", so the intent is already present; the control flow does not match it.

- Gain: complete inventory under mixed/half-written roots.
- Risk: low; `list` may report more bundles.
- Tradeoff: descending into arbitrary directories costs more `read_dir` calls;
  a visited set is advisable if symlinks are followed (see C9).

### A12. Multi-valued response headers collapse to the last value (S2, confirmed)

`http::record::header_map` collects a `HeaderMap` into a
`BTreeMap<String, String>`. Multiple values for one header overwrite each
other, so a response with repeated `Set-Cookie`, `Link`, `Warning`, or
`Cache-Control` fields is recorded incompletely. The artifact is evidence;
losing all but one value is a fidelity bug even when the value is not used by
the tool.

- Gain: response evidence is complete.
- Risk: low; consumers of the map need to accept a list for repeated keys.
- Tradeoff: changes the document shape from string to array/joined string;
  a schema-compatible rendering (join with a documented delimiter) avoids that.

### A13. Error-body retention can transiently hold `jobs × budget` (S3/S2, confirmed)

`http::fetcher` reads an error body with a per-response retain limit equal to
`error_bodies.remaining()`. `remaining()` is advisory: concurrent requests can
all observe the same remainder before any `push` reserves it. Each may buffer up
to that amount, so peak memory is roughly `jobs × min(max_response_bytes,
remaining)`, even though the final retained total is bounded.

The `push` path truncates correctly, and the total in the document is right. The
problem is transient memory during a burst of failures, which is exactly when a
stressed process can least afford it.

- Gain: bounded peak memory under many simultaneous error responses.
- Risk: low; no artifact change if the final rows are unchanged.
- Tradeoff: a reservation API on `ErrorBodies` (reserve before reading) reduces
  concurrency for error bodies; a smaller per-response cap is simpler but can
  truncate more evidence.

### A14. Rejection rows are capped in arrival order (S2/S3, confirmed)

`rejections::Rejections::record` keeps the first `MAX_ROWS` arrivals and counts
the rest in `omitted`. Rows are sorted at write time by `(source, value, rule)`,
so the document is deterministic only if the same set of rows arrived first.
Under concurrent manifest fetching, which refusals land in the first 4096
depends on scheduling. Two captures of the same hostile input can publish
different `rejections.json` contents (same total/omitted maybe, different
sample).

- Gain: deterministic evidence document under concurrency.
- Risk: low-to-medium; the `omitted` count may shift, and a deterministic cap
  policy may keep different rows than today.
- Tradeoff: deterministic selection requires either sorting all rows before
  capping (unbounded memory for a hostile manifest) or a bounded top-k
  selection (more CPU, bounded memory). A documented "first by source order"
  policy is the middle ground.

### A15. `why_unusable` can say "all four A6 checks hold" for a pathless row (S3, confirmed)

`blobs::props::BlobProps::why_unusable` starts from `is_usable`, which requires
`stored_path.is_some()`, but its match arms only look at measurement/fetch/
decode/verified. A row with all four A6 checks green and `stored_path: None`
falls into the `Some(v)` arm and renders the verified text, which reads as if
the row is fine. `check_blobs` already has a separate no-path branch, so the
message is mostly diagnostic, but it is the wrong diagnostic.

- Gain: the reason names the actual defect.
- Risk: very low; message-only.
- Tradeoff: add a pathless arm before the verified arm.

### A16. Magic sniff uses one `read`, not `read_exact`/`read_upto` (S3, confirmed)

`blobs::props::measure_file` does `reader.read(&mut head)` for four bytes and
then decides whether the body is a zstd frame. `Read::read` may return fewer
than four bytes without EOF. A short read on a valid frame would be classified
as "not a zstd frame", turning a valid blob into an `undecodable` row. In
practice a `BufReader` over a local file usually fills the buffer, but the code
should use `read_exact` (or the crate's `hash::read_upto` if available) and
treat EOF-before-four as the actual malformed case.

- Gain: removes a rare false negative.
- Risk: very low; only changes classification on short reads.
- Tradeoff: handle `UnexpectedEof` explicitly so the diagnostic still says how
  many bytes were seen.

### A17. `IntOrStr` conversion is lossy; tag length is measured in bytes (S3, confirmed)

`hyp::types::From<u64> for IntOrStr` uses `as i64`, so values above
`i64::MAX` become negative. If those values feed byte/size arithmetic, the
saturation conventions elsewhere do not help. Separately,
`config::tag_shape_rule_inner` rejects tags longer than 32 *bytes* but says
"32 characters"; a multi-byte tag is rejected earlier than the message
promises.

- Gain: consistent numeric semantics and honest diagnostics.
- Risk: low; a conversion that previously wrapped now fails or saturates.
- Tradeoff: changing `IntOrStr` representation may ripple through JSON
  serialization; a checked conversion at the numeric use site may be enough.

### A18. Terminal `SUMMARY` is absent from the JSON-lines log (S3, confirmed)

`main.rs` calls `log_guard.finish()` before `summary.emit(...)`. `report::emit`
writes the `SUMMARY` line to stdout and then emits a `tracing::debug!` record;
by that time the JSON-lines log has been flushed and closed, so the terminal
record never reaches the file. A reader of the log sees the run's narration but
not its final verdict.

- Gain: the log file is self-contained.
- Risk: very low; output file gains one line.
- Tradeoff: `finish()` must move after `emit`, or `SUMMARY` must be logged
  through a still-open writer. Moving `finish` is likely simpler, but check
  that `exit_with` does not need the guard closed first.

### A19. `report::push_json_key` double-quotes escaped keys (S3, latent)

`push_json_key` writes an opening quote, and for a non-plain key calls
`json_string(k)`, which already returns a quoted literal; then it writes a
closing quote. A dynamic key containing `"` or a control character becomes
`""key"""` and corrupts the line. Current keys are static ASCII, so the bug is
latent, but the function explicitly claims to support the escaped case.

- Gain: correct JSON for any future dynamic key.
- Risk: very low; no change for current keys.
- Tradeoff: either make `json_string` return an unquoted body or have
  `push_json_key` append the already-quoted form without extra quotes.

---

## B. Selection, accounting, and reporting consistency

### B1. The published candidate rank is not the selection order (S2, confirmed)

`select::candidates` sorts with `cost_then_field_then_path`:
`budget_cost`, then field rank, then path. `manifest::diff::by_cost` sorts with
`total_cost`, field rank, `moved` descending, `reused` descending, then path.
`select::candidates_document` uses `by_cost` and writes `rank` from that
enumeration. For common paths with equal `total_cost`, the two orders can
differ whenever `moved` or `reused` differs. The document's `rank` therefore
does not answer "why was this picked?" in the order the picker used.

There is a second gap: `by_cost` only covers scored common paths, while the
actual pool also includes one-sided `added`/`removed` rows sized by their own
size. The document is titled as a scoring table for common paths, so excluding
one-sided rows may be intentional, but then `rank` should not be presented as
the selection rank. Either way, the schema and prose need one definition.

- Gain: the artifact's explanation matches the artifact's decision.
- Risk: high for artifact-visible ordering; changing selection order can change
  which files are captured under a budget.
- Tradeoff: unifying on the selection comparator makes `candidates.json`
  disagree with spec §5's stated tie-break; unifying on `by_cost` changes
  admission. The safer first step is to separate "scoring rank" from
  "admission rank" and document both, then decide whether the spec or the
  picker is authoritative.

### B2. `budget_cost` documentation and behavior disagree for `added` (S2, confirmed)

`SizedCandidate::budget_cost` documents that an `added` path is charged "its
compressed size" because it must be fetched whole. The implementation charges
`self.size`, and `PathRow.size` for `added` is the new entry's declared
`size`, which is uncompressed. `removed` is charged the old size, which is the
bytes it occupies in `game/`, as documented.

So an `added` candidate can be overcharged relative to `total_cost`/download
semantics, or the comments and spec are wrong. Either way, the budget ledger and
the artifact's explanation currently rest on an undocumented mix of compressed
and uncompressed sizes.

- Gain: budget behavior is predictable and matches the flag's name.
- Risk: high if changed, because admission under `--max-download-bytes` changes.
- Tradeoff: carrying compressed size into `PathRow`/`SizedCandidate` adds a
  field; charging uncompressed size consistently is simpler but changes the
  meaning of the budget and may need a doc/schema revision.

### B3. `decode_counts` skips empty-chunk non-folders for size mismatch (S2, confirmed)

`manifest::decode_counts` skips the byte-level checks for `e.is_folder ||
e.chunks.is_empty()`. The comment justifies folders, but the `chunks.is_empty()`
half also suppresses `size_mismatch_files` for a non-folder file whose declared
`size` is nonzero. That is exactly a file claiming bytes with no chunks, and it
is a structural violation the counter should report.

- Gain: `size_mismatch_files` means what its name says.
- Risk: low; only malformed manifests newly report a violation. A zero-byte
  empty file still has `size == 0` and remains clean.
- Tradeoff: an empty non-folder with nonzero size may also be `non_gapless`;
  decide whether one or both counters should fire. Reporting both is consistent
  with the existing "independent violations are separate" rule.

### B4. `is_a_real_upgrade` is test-only (S3, confirmed)

`PathDiff::is_a_real_upgrade` encodes the "something changed" predicate but has
no production caller. Either use it to guard the same-tag/empty-diff case (A1)
or remove it and keep the predicate in the tests that need it. Leaving a
production-shaped invariant with no production user invites drift.

- Gain: one fewer dead concept, or one more real guard.
- Risk: very low.
- Tradeoff: if reused for A1, the refusal message should distinguish "same
  resolved tag" from "different tags but no changed paths"; they are different
  facts.

### B5. Rejection/capture-id uniqueness is not asserted (S3, latent)

`select` assigns forced ordinals before the budget check, while phase 1 starts
its per-class ordinals at zero. The current flow usually avoids a collision
because an admitted forced capture covers its class, and a rejected forced path
cannot become admissible later under the same ledger. But the code only
de-duplicates rejections by path; it does not assert that `capture_id` is unique
across admitted and rejected records, nor that two paths never claim the same
`patchmaps/<id>.json` stem. A future budget/order change could publish two
records with one id.

- Gain: an invariant that protects the artifact's file naming and matrix.
- Risk: low if added as a `debug_assert`/test; higher if enforced as a hard
  refusal.
- Tradeoff: a hard refusal could reject a shape that currently produces a
  usable bundle under a degenerate manifest; a debug assertion catches the bug
  without changing release behavior.

---

## C. Performance, IO, and resource use

### C1. Close-out hashes the bundle more than once; the bridge multiplies it (S3, confirmed)

`emit::closeout::close` performs:

- `write_manifest_files` → full-tree `Checksums::scan`;
- first `write_checksums_reusing` → walks the tree and reuses known digests,
  hashing the newly written `selfcheck`, `MANIFEST` and `README`;
- `checksums.a16` → re-hashes every file to prove A16;
- second `write_checksums_reusing` → walks and re-hashes the two rewritten
  documents.

That is roughly two full reads of every blob plus a few small documents. On a
~98 MB bundle this is visible but acceptable for capture. The bridge then runs
`gircap verify` once per test, and `verify` re-hashes the entire bundle again.
Six tests means six full verify passes over the same immutable bundle.

- Gain: substantially lower suite wall-clock and disk read volume.
- Risk: low for the bridge (test-only); medium for capture if the close-out
  pass structure changes.
- Tradeoff: verifying once per suite requires a shared fixture or a single
  test that loops over captures; per-test isolation and parallelism are lost.
  For capture, a targeted second-pass check (rewritten paths + membership)
  preserves most of the saving without a full re-hash.

### C2. `retention(pair)` is computed twice per pair (S3, confirmed)

`blobs::plan::plan` calls `retention(p)` for the A12 check and logging, then
`blobs::chunks::wants(&pairs)` calls `retention(pair)` again for every pair to
build the reusable set. Each call rebuilds a `HashSet` of old chunk identities
and clones every old/new chunk id into the retention vectors. For large files
this is duplicate CPU and allocation on the hot planning path.

- Gain: one pass per pair instead of two.
- Risk: low; the two calls are pure over the same `FilePair`.
- Tradeoff: `plan` would need to pass the computed retentions into `wants`, or
  `wants` would need to return them. This changes an internal API but not the
  artifact.

### C3. `a16` claims first-finding short-circuit but scans everything (S3, confirmed)

`Checksums::a16` calls `self.findings(root).into_iter().next()`. `findings`
walks the whole tree, builds full difference sets, sums all entries, and
re-hashes every listed file before returning. So the capture's "first finding
only" path pays the same cost as the exhaustive verifier path.

- Gain: a capture with an early mismatch can stop early.
- Risk: very low; only changes how quickly a failure is reported.
- Tradeoff: an early-exit iterator is more code than a full `Vec`; if close-out
  is dominated by the second full pass anyway, the gain is modest.

### C4. `digest_file` uses `io::copy`'s default 8 KiB buffer (S3, confirmed)

The test comment claims a 64 KiB buffer, but `std::io::copy` chooses its own
buffer (currently 8 KiB for generic readers/writers). On a bundle with many
large files, a larger `BufReader`/`BufWriter` or a platform-specific copy can
reduce syscall overhead. The gain depends on OS and filesystem; it is a
measurement candidate, not a guaranteed win.

- Gain: fewer read syscalls on large blobs.
- Risk: very low; bytes hashed are unchanged.
- Tradeoff: a larger buffer uses more memory per concurrent digest, but
  close-out is mostly serial.

### C5. `ChunksByFile` uses a `BTreeMap` for sizes and rebuilds per call (S3, confirmed)

`verify::materialize::ChunksByFile::build` stores chunk sizes in a `BTreeMap`
keyed by `&str`. Chunk ids are effectively random hex strings, so the ordering
is not useful and `HashMap` would be cheaper. More importantly, the public
`chunks_for` convenience calls `ChunksByFile::build(index)` on every call,
which is O(index) each time. `rebuild_all` correctly builds once, but other
callers can accidentally pay the full grouping cost per file.

- Gain: lower CPU and allocation in verify/materialize.
- Risk: very low; lookup results are identical.
- Tradeoff: `HashMap` iteration order is nondeterministic, but only lookup is
  used; if iteration is ever added, sort explicitly.

### C6. Assembly copies reused and fetched bytes more than necessary (S3, confirmed)

`assemble::assembly::patch_flow` builds a per-chunk `body` (`slice.to_vec()` or
`decoded(...).into_owned()`), then `out.extend_from_slice(&body)`. For a REUSE,
that is a copy from the old file into a temporary `Vec`, then another copy into
`out`. For a FETCH, the decoded bytes may already be owned; the extra `body`
still exists only to be appended. The old-identity map is a `BTreeMap` keyed by
`(String, i64)`, cloning the md5 for every old chunk and every new lookup.

- Gain: fewer allocations and one less copy per chunk on the largest files.
- Risk: low; assembly output is unchanged if the copies are replaced with
  direct writes at the target offset.
- Tradeoff: writing reused slices directly into `out` requires `out` to be
  sized to the final file and offsets to be known before the loop (they are, via
  `target_range`), but it makes the flow less linear. A `HashMap` with borrowed
  keys removes the clones but requires the old entry to outlive the map, which
  it does.

### C7. `Assembled::a12_holds` is quadratic (S3, confirmed)

`assemble::types::Assembled::a12_holds` collects fetched and reused ids into
`Vec`s and then asks `!reused.contains(f)` for each fetched id. That is
O(fetch × reuse) per capture, and the same predicate is already computed in
`blobs::chunks::Retention::a12_failure` with `HashSet`s. The plan size is usually
small, so this is a cleanup rather than a hot path, but the duplicate
implementation is also a correctness risk.

- Gain: linear check and one definition of A12.
- Risk: very low; same boolean.
- Tradeoff: sharing the retention check would couple `assemble` to
  `blobs::chunks`, or require a small helper in a neutral module.

### C8. Bridge server: per-test listeners, per-request connections, cache lock (S3, confirmed)

`tests/artifact_bridge.rs` calls `bridge()` per test, which opens the bundle,
runs `gircap verify`, and binds four listeners. Six tests therefore pay six
verifies and 24 listener binds. The server also sends `Connection: close` on
every response, so each chunk request opens a new TCP connection; with a small
fixture this is fine, but a larger capture pays connection setup per chunk.

`server::manifest_bytes` holds the global `MANIFEST_CACHE` mutex while doing the
blocking disk read. The cache is shared across tests, which is good, but the
lock turns concurrent first requests into serialized disk reads. A
double-checked insert or an `OnceLock` per path would avoid holding the lock
during IO.

`Served::manifest_index` is keyed by manifest id alone. If the old and new
builds declare the same manifest id (same content referenced by two tags), the
second insert overwrites the first, and the `build_id`/`tag`/archive associated
with the first build is lost. The route's build-id check can then reject a
legitimate request or serve the wrong tag's archive. Keying by `(build_id, id)`
or storing a list per id fixes it.

`server::error_shape` takes an `extra` parameter and immediately discards it
(`let _ = extra;`); it is dead weight at every call site.

- Gain: faster, more faithful bridge; removes a latent manifest-identity bug.
- Risk: low (test-only), except the shared-fixture restructuring changes how
  tests isolate state.
- Tradeoff: keep-alive complicates the tiny HTTP server; a single suite-level
  fixture is simpler but loses per-test parallelism and independent failure
  isolation.

### C9. `list`/`walk` follow symlinks without a visited set (S3/S2, confirmed)

`verify::list::child_dirs` uses `std::fs::metadata`, which follows symlinks, and
`emit::index::walk` uses `Path::is_dir`/`is_file`, which also follow links. A
symlink/junction cycle under `--root` or the artifact directory can make the
walk loop, or can make a bundle include files outside its root. The doc comments
explicitly say the metadata follows symlinks, so this is a known property rather
than an accident, but there is no visited set or root-containment check.

- Gain: termination and containment for hostile/odd trees.
- Risk: low for normal bundles; medium if a legitimate symlinked artifact root
  currently works.
- Tradeoff: refusing symlinks changes behavior for valid setups; a visited set
  plus a canonical-root check preserves most of it at the cost of a canonical
  path lookup per directory.

### C10. `Paths::escape_path` always allocates (S3, confirmed)

`paths::escape_path` splits, maps each component through
`escape_component(...).into_owned()`, collects into a `Vec`, and joins into a
new `String`. For the overwhelmingly common case where no component needs
escaping, this allocates a `Vec` and a `String` for nothing. It is called in
A14, path validation, bundle opening, and several readers.

- Gain: less allocation in path-heavy loops.
- Risk: very low; output unchanged.
- Tradeoff: the fast path must scan for `%` and device components before
  deciding to allocate; a `Cow` return would push the decision to callers.

### C11. `check_agreement` is O(recorded × recomputed) (S3, confirmed)

`verify::checks::check_agreement` iterates recorded assertions and, for each
holding row, calls `recomputed(id)`, which scans the entire recomputed `rows`
vector. With a few hundred rows this is small, but it is quadratic in a document
whose size is not bounded by the schema. A `HashMap<&str, bool>` or a single
pass over `rows` would be linear.

- Gain: predictable verify cost on large bundles.
- Risk: very low.
- Tradeoff: must preserve the current "all rows with this id are green" and
  "missing id counts as agreement" semantics; a map must be initialized to the
  same default.

### C12. `ChunkIndex::build` clones each want's roles twice (S3, confirmed)

`indexed_from` returns `(IndexedBlob { sets: want.sets.clone(), ... },
want.sets.clone())`. `ChunkIndex::build` consumes the second copy to build the
role lists, and keeps the first on the blob. The role lists are derived from the
same data, so one of the clones can be avoided by passing the roles separately
or by building the index from `want` once. This is small (a few roles per
blob), but it is repeated for every blob and is easy to remove.

- Gain: fewer small allocations.
- Risk: very low.
- Tradeoff: the constructor's current shape is convenient; changing it touches
  three call sites.

---

## D. Clean code, duplication, and dead weight

### D1. A17's acceptable-state logic is duplicated (S3, confirmed)

`select::MatrixStatus::is_acceptable` and
`emit::selfcheck::MatrixRow::ok_status` both encode the four acceptable status
strings. A test already asserts they agree, which is good, but the duplication
means a fifth state or a wording change must be made twice. The same pattern
appears in the wording tables and in the A6/A7/A8 row construction.

- Gain: one source of truth for "which matrix states are acceptable".
- Risk: very low.
- Tradeoff: `emit::selfcheck` already depends on `select`, so it can call the
  model directly; the reverse would be a layering violation.

### D2. Dead fields and parameters (S3, confirmed)

- `http::probe::ErrorProbes::other_manifest_prefix` is populated by the caller
  but never read by the probe logic.
- `server::error_shape`'s `extra` parameter is explicitly discarded.
- `PathDiff::is_a_real_upgrade` is test-only (see B4).
- `manifest::diff::by_cost` and `select::cost_then_field_then_path` are two
  comparators for one concept (see B1).

- Gain: less surface area and fewer misleading knobs.
- Risk: very low, except removing `by_cost` would change `candidates.json`'s
  order.
- Tradeoff: if `other_manifest_prefix` is intended for a future probe, document
  that; otherwise remove it rather than leaving a field that looks meaningful.

### D3. `hyp` parses `getBuild` more than once and keeps parallel arrays (S3, confirmed)

`hyp` represents a few related fields as parallel arrays and re-parses the
`GetBuild` body at multiple points in the capture flow. This is not a hot path
compared with manifests/chunks, but it is repeated work and a place where two
arrays can drift. A single parsed struct carried through the flow would be
simpler and cheaper.

- Gain: fewer parses and one representation.
- Risk: low; internal refactor.
- Tradeoff: the current shape may be deliberate for serialization or for
  preserving raw bytes; any change must keep the raw body for `api/` output.

### D4. `config`'s tag diagnostics count bytes but say characters (S3, confirmed)

`tag_shape_rule_inner` checks `tag.len() > 32` (bytes) and emits "longer than 32
characters". For ASCII version tags this is the same; for a non-ASCII tag the
message is wrong and the limit is stricter than advertised. Either count
`chars()` or change the wording to bytes.

- Gain: honest diagnostic.
- Risk: very low; may admit some non-ASCII tags that were previously refused if
  the check becomes character-based.
- Tradeoff: the path/Windows rules already constrain tags heavily, so the
  practical effect is small.

### D5. The two `paths_tsv` writers are intentionally separate (S3, note)

`manifest::diff::paths_tsv` (diff table) and `manifest::dump::paths_tsv`
(per-manifest sidecar) have different columns and purposes. Their doc comments
already say not to merge them. The only shared weakness is the unescaped tab,
which is recorded as deliberate. This is not a finding to act on, but it is a
place where a future reader may try to "DRY" two things that are not the same.

### D6. `push_json_key` and `Paths::escape_path` are small, high-leverage cleanups (S3)

Both are covered above as correctness/performance items; they are also examples
of a broader pattern: hand-written fast paths whose comments claim a property
the code does not fully enforce. The fix is to make the fast path and the slow
path share one encoded representation, or to narrow the comment to the actual
guarantee.

---

## E. Artifact-bridge-specific observations

The bridge is deliberately expensive and `#[ignore]`d, so "slow" is not by
itself a bug. The useful distinction is between cost that buys coverage and cost
that repeats the same proof.

1. **Verification is repeated per test.** `bridge()` runs `gircap verify` for
   each of the six capture tests. The bundle is read-only and immutable during
   the suite, so one verify per suite (or one verify per bundle identity) is
   enough. If tests must remain independent, a shared `OnceCell`/fixture can
   still avoid the repeated subprocess.
2. **Listeners are repeated per test.** Four `TcpListener::bind` calls per test
   are cheap but unnecessary if the suite shares a bridge. The per-capture
   scratch tree is still needed; the server state is not.
3. **The patchmap reader re-reads `MANIFEST.json` and every patchmap per test.**
   `Bundle::open` already parsed `MANIFEST.json`, and `Patchmaps::read` parses
   it again. This is small relative to verify, but it is another repeated read.
4. **`manifest_index` keyed by id alone** can mis-serve a manifest id shared by
   two builds (see C8). This is the one bridge issue that can make a green test
   assert the wrong thing rather than merely run slowly.
5. **`Connection: close` per response** makes each chunk a new TCP connection.
   With a small fixture this is negligible; with a large chunk count it is a
   measurable, avoidable cost. Keep-alive is a tradeoff in server complexity.
6. **The manifest cache lock is held during disk IO** (see C8). A
   double-checked cache avoids serializing concurrent first reads.
7. **The bridge's `result_path` handling trusts the patchmap field.** If A9's
   path validation is added, the bridge benefits automatically; until then the
   test harness is another consumer of an unvalidated path.
8. **The pre-pass and real run each start the binary**, which is necessary for
   the two failure modes the suite separates. Do not collapse those without
   losing the diagnostic distinction.

---

## F. Risk and blast radius

| Area | Risk if changed | Observable effect | Why |
|---|---|---|---|
| Same resolved tag refusal (A1) | Low | New error/exit before capture | No valid-run artifact change |
| `.gz` encoding (A2) | Medium | Different checksums/encoding for `.gz` docs; possible new close-out failures | Changes a published field and file classification |
| Folder handling (A3) | Medium-high | Selection and fixture layout change for manifests with folders | Touches diff, selection, assembly |
| Duplicate-path TSV (A4) | Low | Sidecar rows change only on duplicate paths | Local document fix |
| Chunk URL suffix (A5) | Low | URLs change only when suffix is non-empty | Latent network behavior |
| A14 disk scan (A6) | Medium | Dirty `game/` can turn green→red | Strengthens a selfcheck |
| A11 side size (A7) | Medium | Previously green bundles can fail | Verification semantics |
| A16 second pass (A8) | Medium | Late checksum discrepancy can turn exit `0`→`3` | Strengthens the strongest check |
| Schema/path validation (A9) | Low-medium | Old/hand-edited bundles are refused | Intended contract, but observable |
| Plan/index cross-check (A10) | Medium | Tampered or stale plans fail | Strengthens verify |
| `list` descent (A11) | Low | More bundles listed | Inventory only |
| Header map (A12) | Low | Document shape may change for repeated headers | Consumer compatibility |
| Error-body reservation (A13) | Low | Peak memory, possibly more truncation | No artifact change if rows equal |
| Rejection cap (A14) | Low-medium | Which sampled rows are kept changes | Determinism tradeoff |
| `why_unusable` (A15) | Very low | Message only | Diagnostic |
| Magic sniff (A16) | Very low | Rare classification change | Local IO fix |
| `IntOrStr`/tag bytes (A17) | Low | Numeric/diagnostic edge cases | Local |
| `SUMMARY` logging (A18) | Very low | One more log line | Output only |
| `push_json_key` (A19) | Very low | Only dynamic keys | Latent |
| Selection comparator (B1) | High | Which files are captured; `rank` meaning | Artifact-visible decision |
| `budget_cost` semantics (B2) | High | Admission under a byte ceiling | Artifact-visible decision |
| `decode_counts` (B3) | Low | New structural violations reported | Malformed input only |
| Hash pass restructure (C1) | Low-medium | Capture wall-clock; verify rows unchanged if done carefully | Performance |
| Retention reuse (C2) | Low | None | Pure duplicate work |
| Bridge restructure (C8) | Low (test-only) | Suite runtime, isolation | Test behavior |
| Symlink policy (C9) | Medium | Legitimate symlinked roots may be refused | Filesystem semantics |

The highest-risk changes are those that alter admission (B1/B2) and those that
alter what a valid-looking bundle is allowed to contain (A2/A3/A7/A10). They
should be made only with characterization tests that pin the intended artifact
bytes or the intended refusal.

---

## G. Recommended ordering

The ordering below is by value and risk, not by module. The first phase is
deliberately test-heavy: several later fixes change artifact-visible behavior,
and without characterization tests it is impossible to tell an improvement from
a regression.

### Phase 0 — Pin the current contract

Add or strengthen tests for the cases that currently have no witness:

- a captured file named `*.gz` that is not gzip;
- a resolved pair where `main.tag == diff_tags[0]`;
- duplicate manifest paths in the sidecar;
- a folder entry in a diff/selection;
- a non-empty `chunk_download.url_suffix`;
- a stray pre-existing `game/` file;
- a patchmap side size that disagrees with the chunk sum;
- a second-pass checksum change after A16;
- a patchmap plan that is self-consistent but disagrees with the index;
- a non-bundle directory containing a nested bundle;
- repeated response headers.

These are tests, not behavior changes. Their value is that every later phase
can distinguish "intended new refusal" from "accidental artifact drift".

### Phase 1 — Correctness and verification (highest value)

Order within the phase:

1. **A1 same resolved tag** — small, high signal, no artifact bytes for valid
   runs.
2. **A2 `.gz` encoding** — the only finding that can abort a legitimate capture
   on ordinary data; fix before more captures are produced.
3. **A5 chunk URL suffix** — small and latent, but cheap to fix while the URL
   code is in hand.
4. **A6 A14 disk scan**, **A7 A11 side size**, **A8 A16 second pass** — these
   turn selfcheck claims into real checks. Do them together so the selfcheck
   rows and verify rows stay consistent.
5. **A9 schema/path validation**, **A10 plan/index cross-check**, **A11 list
   descent** — the verify-surface hardening pass.
6. **A3 folder policy**, **A4 duplicate-path TSV**, **B3 decode_counts** — data
   edge cases. A3 needs a product decision; A4/B3 are local.
7. **A12 headers**, **A13 error-body reservation**, **A14 rejection cap**,
   **A15-A19** — diagnostic/fidelity fixes that do not change the main path.

Why this order: A1/A2 are the only items that can make a normal capture wrong
or fail; A5 is a one-line-class latent break; A6-A11 are the verification
backbone; the rest are lower-frequency or diagnostic.

### Phase 2 — Selection and accounting semantics (highest blast radius)

Do this only after Phase 0 tests exist and after deciding which document is
authoritative.

1. Decide whether `candidates.json → rank` is a scoring rank or an admission
   rank. If both are useful, publish both with distinct names; do not leave one
   `rank` that means neither.
2. Reconcile `budget_cost` with the documented compressed/uncompressed rule.
   If the flag is `--max-download-bytes`, compressed size is the defensible
   meaning; if the intent is "cost to materialize", document that and rename
   the prose, not the flag.
3. Then, and only then, consider unifying the comparators. Unifying changes
   admission; separating the concepts does not.
4. Add the capture-id uniqueness invariant (B5) as a debug assertion/test while
   the selection code is open.

Why last among correctness: these are not bugs in the sense of "the code
contradicts itself" only; they are choices about what the artifact should mean.
They need a decision, not just a patch.

### Phase 3 — Runtime and resource work

1. **Bridge suite**: verify once per immutable bundle, share the bridge
   listeners, cache the parsed records. This is the single largest wall-clock
   win and is test-only.
2. **Close-out hashing**: keep the two-pass fixpoint, but avoid a second full
   independent hash if the first A16 pass can be scoped to the rewritten files
   plus a membership check. Measure before changing; the current design is
   correct, just expensive.
3. **Retention reuse (C2)**, **assembly copies (C6)**, **ChunksByFile (C5)**,
   **a16 early exit (C3)**, **digest buffer (C4)**, **error-body reservation
   (A13)**, **cache locking (C8)** — independent, low-risk, measurable.
4. **Symlink policy (C9)** — do after the bridge/list tests, because it changes
   which trees are considered bundles.

Why after Phase 2: the artifact's selection can change the amount of data
hashed and fetched, so optimizing before the semantics settle can optimize the
wrong workload.

### Phase 4 — Clean code and polish

- A15-A19 message/format fixes.
- D1-D4 duplication and dead fields.
- C10-C12 allocation cleanups.
- D5's note is documentation only; do not merge the two TSVs.

Why last: these have almost no runtime or correctness payoff, and doing them
earlier increases the diff surface of the phases that matter.

---

## H. Tradeoffs to make explicit before starting

1. **Determinism vs. bounded memory.** A14's rejection cap and A13's error-body
   budget both trade completeness for bounded documents. Pick a policy that is
   deterministic under concurrency; "first arrival" is not.
2. **Exactness vs. close-out cost.** A8's independent second check is the
   honest version of A16. If the cost is unacceptable, scope the check to the
   files that changed and state that scope in the row; do not silently publish
   an unchecked document.
3. **Strictness vs. compatibility.** A9's schema checks and A11's side-size
   check will refuse or fail bundles that currently pass. That is the point, but
   it needs a migration story for old artifacts: a clear "this bundle predates
   the check" message is better than a generic parse error.
4. **Selection semantics vs. spec §5.** B1/B2 cannot both preserve spec §5's
   stated tie-break and the picker's current order unless the two are
   reconciled. Decide whether the spec or the implementation is authoritative
   and update the other; publishing two different ranks is the worst outcome.
5. **Bridge isolation vs. suite runtime.** Sharing a bridge is the biggest win,
   but it couples tests through shared server state and the request log. If the
   log is global, a per-test log offset or a reset is required; otherwise tests
   will see each other's hits.

---

## I. What not to change

For completeness, the following are deliberate and should not be "fixed" as
part of this review:

- the hand-rolled JSON report writer (performance and key order are intentional);
- the two-pass A16 fixpoint (the second pass is the convergence proof);
- the single-pass `decode_counts` and range logic (already optimized);
- the assembly byte movement and `ChunkSource` memoization (measured design);
- the biz/exit-code tables and `Box::leak` (stable addresses are intentional);
- prost decoding and `unknown_field6` preservation (wire compatibility);
- the two `paths_tsv` formats (different documents, same name);
- the unescaped tab in TSV (recorded compatibility choice).

If any of these become blockers, treat them as a separate design change with
their own rationale, not as cleanup under this review.
