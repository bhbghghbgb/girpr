# Adversarial review note — third pass

**Tree:** `fix-gaps/gircap` at `577c399` (`docs(review): fold the second-pass verdicts into the review`), clean working tree.

**Scope:** `tools/capture` only — `src/**`, `tests/**`, `docs/**`, `README.md`, `Cargo.toml`. The root `girpr`, `tools/sync`, `crates/log` and `crates/tempdir` were read only where `gircap` depends on them.

**Standard applied:** strict validation at every trust boundary, fail fast, and no error silently dropped. The review's own second pass already treats a hostile API, a compromised CDN and a local proxy as in scope (F6), and `docs/00-spec.md:181` says wire-supplied sizes are untrusted. That standard is what the findings below are judged against.

## F1–F12

Re-derived from source and tests at `577c399`. All twelve fixes are present, and no regression was found:

| ID | Verdict |
|---|---|
| F1 cumulative forced budget | fixed — `select/mod.rs:496-561`, matrix test |
| F2 record and continue | fixed — `select/mod.rs:534-549` |
| F3 `forced_unscored` | fixed — `select/mod.rs:524-531`, `run/mod.rs:399` |
| F4 slot and reason | fixed — `blobs/mod.rs:270-400` |
| F5 error-body drain | fixed — `http/fetcher.rs:166-232` |
| F6 tag validation, flags and API | fixed — `config/mod.rs:110-124`, `:575-628` |
| F7 `--no-chunk-blobs` ceiling | fixed — `blobs/plan.rs:132-140` |
| F8 lossy `recorded_argv` | fixed — `cli/mod.rs:398-416`, `main.rs:58-63` |
| F9 duplicate `--files` | fixed — `config/mod.rs:481-521` |
| F10 help text | fixed — `cli/mod.rs:161-162`, `:188` |
| F11 failed-fetch temp cleanup | fixed — `blobs/mod.rs:456-514` (doc caveat in G2) |
| F12 closeout skip-set literal | fixed — `emit/closeout.rs:635-670` |

The suite is green (see **Verification**), but it does not exercise the sinks in G1: a passing run is not evidence that untrusted document values are safe.

## New findings

| ID | Severity | Class | Location | Finding |
|---|---|---|---|---|
| G1 | high under the F6 threat model | security / path traversal | `blobs/props.rs:168-170`, `blobs/mod.rs:448-450`, `:539-547`, `emit/gamedir.rs:192-201`, `emit/mod.rs:316-321` | Three values from untrusted documents are joined into filesystem paths without the component checks `--files` already has. A hostile or compromised API/CDN can write outside `--out`. |
| G2 | low | documentation | `src/emit/mod.rs:9-10`, `:283-286`, `docs/01-artifact-schema.md:81-82`, `docs/03-design-notes.md:185` | The crate still claims a `tmp/` staging directory and universal atomic document writes. Both claims are contradicted by the writer in the same file. |
| G3 | low | documentation | `docs/01-artifact-schema.md:625-629` | The "null `stored_path` has three causes" rule omits `--no-chunk-blobs`, whose rows have a null `stored_path` **and** a null `decode_error`. |

### G1 — Document-supplied paths are joined raw, so a capture can write outside its bundle

**Severity:** high when the upstream is hostile or compromised, which is the threat model F6 already adopts; low for a fully trusted upstream. **Class:** security / path traversal. This is the same class F6 fixed for version tags, on the same trust boundary, for three fields F6 did not cover.

#### The three sinks

| Value | Enters from | Reaches | What is written |
|---|---|---|---|
| `SophonChunk.id` | manifest protobuf (`manifest/proto.rs:61-62`, decoded at `manifest/mod.rs:438-448`) | `BlobProps::stored_path_for` (`blobs/props.rs:168-170`) → `Store::paths_for` (`blobs/mod.rs:539-547`) → `std::fs::rename` (`:478`); also `Sources::url` (`:183-191`) | The CDN body. It is written and renamed **before** `measure_file` runs, so even a body that is not a zstd frame stays on disk. |
| `SophonChunkFile.file` | manifest protobuf (`proto.rs:26`) | diff `path` (`manifest/diff.rs:322`, `:336`, `:350`) → `FilePair.path` (`blobs/mod.rs:644-662`) → `Expectations.result_path` (`assemble/mod.rs:1150-1154`) and `write_variant` (`emit/gamedir.rs:192-201`) → `write_parts` (`emit/mod.rs:316-321`) | The assembled `game/` or `expected/` bytes. The attacker also declares the md5 they are checked against. |
| `audio_pkg_scan_dir` | `getGameConfigs` JSON (`hyp/types.rs:199-204`) | `write_prelude` (`emit/gamedir.rs:186-187`) → `write_rel` (`emit/mod.rs:311-321`) | The generated audio scan text. Content is constrained, but the target path is not. |

`write_parts` creates every parent directory and then calls `File::create`, which truncates an existing file (`emit/mod.rs:316-321`). `Store::ensure_blob_dir` creates the parent of the blob path on the first write (`blobs/mod.rs:553-560`); later writes reuse that cached success, so a hostile chunk whose parent already exists is enough. There is no `strip_prefix`, canonicalisation, component check, or extension guard between the document value and those calls.

#### Evidence that no check exists

- `stored_path_for` is a bare `format!` (`blobs/props.rs:168-170`). Nothing between protobuf decode (`manifest/mod.rs:438-448`) and `paths_for` inspects the id. The only test, `the_stored_path_is_predictable_from_the_chunk_id_alone` (`props.rs:709-728`), asserts a hardcoded plausible id and then comments that "an id is hex plus one underscore". It does not enforce that invariant, and its `!name.contains("..")` assertion is about the literal id it supplied, not about the space of ids the decoder accepts.
- `config::validate_paths` already has exactly the right policy — non-empty, no `\`, no leading `/`, no `:`, no `..` component, no empty component (`config/mod.rs:481-519`) — but it is called only for `--files` (`config/mod.rs:322`). Manifest paths and `audio_pkg_scan_dir` bypass it.
- F6's fix is the precedent: `VersionPair::select` validates the API-resolved `main.tag`/`diff_tags[0]` before `pair_dir` sees them (`config/mod.rs:110-124`), with the stated reason that "a hostile or compromised API (or a local proxy) could reach the same path sink" (`:114-116`). Chunk ids, manifest file paths and the scan directory are the same value class and are still unvalidated.
- `stored_path` is copied into `chunks/index.json` and re-joined on read by `ChunkSource::decoded` (`assemble/mod.rs:604`), so a captured bundle can also make `verify`/`materialize` read outside its root.

#### Concrete failure scenarios

1. **Manifest file path.** A hostile or compromised `getBuild`/CDN serves an archive containing an `added` entry with `file = "..\..\..\..\Users\victim\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\gircap"` and chunks whose assembled bytes are the payload, with a declared whole-file md5 that matches. The diff admits the path like any other added file, and `write_variant` writes `expected/..\..\..\..\Users\...` — the `expected/` prefix plus four `..` components leaves `<out>`, `create_dir_all` creates the missing directories, and `File::create` writes the payload. On Unix the same works with `/`-separated `..`.
2. **Chunk id.** A manifest declares a chunk id with five parent components plus a target name, e.g. `..\..\..\..\..\Users\victim\AppData\Local\Temp\x`. `Sources::url` requests `…/chunks/<id>` (URL injection into the CDN path), and `stream_from` computes `chunks/blobs/..\..\..\..\..\...\x.zst`, creates the parents, streams the body there, and renames it into place. If the body is not a zstd frame, `measure_file` fails after the rename and the file is left behind; if it decodes, the index carries that `stored_path` and `verify` joins it again.
3. **Scan directory.** A hostile `getGameConfigs` returns `audio_pkg_scan_dir` with enough `..` components to leave `<out>`, e.g. `"..\..\..\..\Users\victim\..."`. `write_prelude` overwrites or creates that path with the generated scan text.

The bundle's own selfcheck does not cover this: the escaped writes are outside the tree that `checksums.json` walks, and the writer records the escaped relative string as if it were a normal bundle file (`emit/gamedir.rs:201`).

#### Why this is actionable rather than theoretical

The tool already refuses an enormous declared size because "a manifest can declare anything at all" (`assemble/mod.rs:665-676`), and F6 was accepted as a blocking residual for tags alone. A test/proxy server (`--hyp-base`, `--sophon-base`) is an explicitly supported setup; a compromised CDN or API is the same precondition F6 used. A capture must not write outside `--out` under either.

#### Concrete fix

1. **One shared relative-path validator.** Factor the component rules from `config::validate_paths` (`config/mod.rs:481-519`) into a helper and apply it to every document-supplied artifact path: `SophonChunkFile.file` and `audio_pkg_scan_dir`. Reject empty, `\`, a leading `/`, `:`, any `..` component, any empty component, and control characters. Manifest paths are documented as `/`-separated, so `/` inside the path stays legal.
2. **A chunk-id policy.** The documented and observed shape is `<16 hex>_<32 hex>` (`docs/01-artifact-schema.md:633`); enforce that exact shape at manifest decode. If the live corpus may contain other spellings, at minimum require a single component: non-empty, no `/`, `\`, `:`, `..`, NUL or control characters. Keep a defensive check in `stored_path_for` / `ChunkSource::from_index` so a bundle from elsewhere cannot make `verify` read outside its root.
3. **Fail at the boundary.** Return `ApiError::shape` (or `UsageError` for a flag) naming the tag, file or id and the offending value, before any fetch, directory creation or write. Do not clamp or silently sanitise: a manifest that names an escaping path is wrong, not a path to rewrite.
4. **Tests.** Add cases for a manifest file `..\..\escape`, `a/../../b`, `C:`, `a\b`, a doubled separator and an empty path; a chunk id containing `/`, `\`, `..` and `:`; and a hostile `audio_pkg_scan_dir`. Assert the run fails and a temporary output root contains no file outside it. Add a unit test that `write_rel`/`write_parts` refuses an escaping `rel` as a final line of defence, so a future caller cannot reopen the sink by forgetting the boundary check.

### G2 — The `tmp/` directory and universal-atomicity claims are false

**Severity:** low. **Class:** documentation. This is a doc contradiction, not a runtime defect.

`src/emit/mod.rs:9-10` says:

> Everything temporary goes to `tmp/` and is moved into place, so an interrupted run never leaves a half-written document that reads as a complete one.

The same file says the opposite at `:283-286`: only chunk blobs are written temp-and-move, and "the small metadata documents written here are **not** written atomically". `write_bytes`/`write_parts` are a plain `create_dir_all` + `File::create` + `write_all`. There is no `tmp/` directory in the tree (`tools/capture/src/tmp` and `tools/capture/tmp` do not exist), and the blob temp file is a dot-prefixed `.<name>.<slot>.tmp` beside its final path (`blobs/mod.rs:543`), not under `tmp/`.

The same stale phrase survives in `docs/01-artifact-schema.md:81-82` ("written through a `tmp/` file") and in `docs/03-design-notes.md:185` ("`tmp/`-and-rename step"). The second pass's F11 verdict in `ADVERSARIAL_REVIEW.md:622` says "the stale `tmp/` staging claim is gone"; it is not. The schema document's atomicity section also states the opposite of the module comment, so a reader gets two incompatible accounts of what a crash can leave behind.

**Fix:** say what the code does. Only chunk blobs use a temp file, and it is `.<file name>.<slot>.tmp` in the blob's own directory, renamed on success. Metadata documents, `game/` and `expected/` files, and `manifests/*` are plain create-and-write, as `docs/01-artifact-schema.md:81-90` already explains. Correct the `emit` module comment, the schema's "through a `tmp/` file" phrase, and the design-note shorthand. Optionally add a sentence to the schema's crash paragraph: an interrupted run can leave a truncated metadata document, which is why `checksums.json` is the completion proof.

### G3 — The "three causes" of a null `stored_path` omits `--no-chunk-blobs`

**Severity:** low. **Class:** documentation.

`docs/01-artifact-schema.md:625-629` says a null `stored_path` has "three causes that cannot all be described the same way — a source that refused, a write that failed, and a fetch task that panicked mid-measure — and the row records which, in `decode_error`."

A metadata-only run is a fourth cause. `BlobProps::unmeasured` (`blobs/props.rs:346-368`) sets `stored_path: None` **and** `decode_error: None`, and `Store::plan_only` (`blobs/mod.rs:322-327`) emits one such row per want. So for `--no-chunk-blobs` rows the stated rule ("the row records which, in `decode_error`") is false: the row records nothing, by design. Lines `:621-623` already mention a never-fetched blob separately, which makes the later "three causes" sentence contradictory rather than merely incomplete.

**Fix:** name four causes and split them by whether a fetch was attempted. The three failure causes carry `decode_error`; `--no-chunk-blobs` rows carry `stored_path: null`, `decode_error: null`, `verified: null` and empty digest strings, and mean "not measured", not "failed". A one-sentence cross-reference to the `--no-chunk-blobs` A6–A12 skip note at `:897` would keep the two sections consistent.

## Verification

Run at `577c399` (the tree above; no source, test or document was modified for this review):

| Command | Result |
|---|---|
| `cargo build -p gircap` | finished, no warnings |
| `cargo test -p gircap` | 640 passed, 0 failed (506 lib + 134 integration) |
| `cargo clippy -p gircap --all-targets` | finished, no warnings |
| `cargo fmt -p gircap --check` | clean |

The integration total is 3 + 8 + 29 + 2 + 30 + 9 + 29 + 9 + 15. No test is `#[cfg(unix)]`, so none was skipped on this Windows host; the `#[cfg(windows)]` tests ran.

## Not verified

- Whether a live `getBuild`/`getGameConfigs` can be induced to serve the hostile values in G1. The precondition is the same one F6 already accepts (hostile or compromised upstream, or a local proxy); the local sinks are reachable from any document the tool accepts.
- The traversal was reasoned from `std::path` and the cited joins, not exercised with a hostile bundle: the review constraints forbade adding a test. The code path is platform-independent for `/`-separated `..` on Unix and `\` on Windows; a concrete end-to-end test should be written with the fix.
- The full set of live chunk-id spellings. The documented shape is `<16 hex>_<32 hex>`, and the sampled ids match it; if a build ever uses another spelling, the weaker single-component rule in G1 is the safe fallback.
- Whether any metadata document can be left torn in a way that `checksums.json` fails to detect at closeout. `write_parts` is not atomic; the closing order and the final checksum pass were not stress-tested with a crash between the two.

## Conclusion

F1–F12 remain fixed, and the second pass's smaller items are in place. G1 is a real boundary gap of the same class F6 fixed, with arbitrary file write outside `--out` as the impact under the threat model the review already uses; it should block the "strict validation" claim until the three sinks share the `--files` component rules. G2 and G3 are documentation inconsistencies that would mislead the next reader about crash behaviour and about `--no-chunk-blobs` rows.
