# ADVERSARIAL REVIEW NOTE — fourth pass

**Branch:** `fix-gaps/gircap` at `1cbf2f1` (`docs(review): drop the third-pass
working note`), baseline `main`; this pass added `0a21ebe`, `0b55a40`, `c9caa02`,
`b6aeb6d` and `c9bf161` on top. Reviewed and fixed 2026-10-28 on a Windows host.

**Scope:** `tools/capture` only — `src/**`, `tests/**`, `docs/**`, `README.md`,
`Cargo.toml`. Root `girpr`, `tools/sync`, `crates/log` and `crates/tempdir` were
read only where a `gircap` boundary depended on them.

**Standard applied:** strict boundary validation, fail fast, and no discarded
errors. A value that reaches a filesystem join must be proved shaped like a
path relative to the directory it is joined onto at the join, not at an earlier
stage that a different reader may bypass. `let _ =`, `unwrap_or_default`, a bare
`continue`, and `if let Ok` that discards a meaningful failure are findings.

**Method:** every F1–F12, G1–G3 and residual claim in `ADVERSARIAL_REVIEW.md` was
re-derived from the current tree; then the three read/write boundaries the third
pass added (`manifest::capture`, `GameConfigs::audio_pkg_scan_dir`,
`ChunkSource::decoded`) were traced to every other sink for the same value. The
four findings are then fixed at the shared boundary rather than at each sink,
with regression tests; this note records each verdict, evidence and commit. It is
deleted once the same material is folded into `ADVERSARIAL_REVIEW.md`.

## Verdict

All four findings were re-checked against the current tree and all four are
valid; none is dismissed as invalid or out of scope. They are fixed on this
branch — H1–H3 in code with regression tests, H4 as a doc correction.

| ID | Verdict | Commit | Resolution |
|---|---|---|---|
| H1 | valid, fixed | `0a21ebe` | `Bundle::open` now runs `verify::validate_document_paths` over the manifest's `capture.id`, every index `stored_path` and `referenced_by[].path`, and every patchmap `path`, naming the field and value on refusal. The A6/A18 joins and the materialize destination join keep their own checks as defence in depth. Three new `tests/offline_bundle.rs` tests build a hostile bundle and assert `verify` and `materialize` refuse it without writing outside the root; they failed before the fix. |
| H2 | valid, fixed | `0b55a40` | `paths::field_component_problem` (the path rule plus "no `/`") is now applied in `config::validate_audio`, in `manifest::capture` for a field chosen from the build document, and in `manifest::dump::write` before any file is created. Tests cover `a/b`, `/abs`, `a\b`, `../x`, `..` and the empty field at both the flag and the dump. |
| H3 | valid, fixed | `c9caa02`, `b6aeb6d` | `paths::relative_path_problem` now refuses a `.` component and any component ending in `.` or a space, on every host; `a/b`, `a/b.` and `a/b ` no longer name one file on Windows. The same rule refuses a reserved DOS device name (`CON`, `NUL`, `COM1`, `CONIN$`, and the same with any extension), which Windows resolves to a device instead of creating a file. New unit tests pin both refusals and keep `a/b.c`, `a/7.1.0`, `a/.hidden`, `a/CONSOLE` and `a/COM0` legal; a `validate_entry_paths` test pins that a manifest entry is refused before any tree write. |
| H4 | valid, fixed | `c9bf161` | The schema now says what an unmeasured row carries: null `stored_path`/`verified`/`decode_error`, `bytes: 0`, empty compressed digests, and the manifest's **declared** `md5_decompressed` and `uncompressed_size`. The `BlobProps::unmeasured` and field comments say the same. |

**Pre-fix state.** Before these commits the branch was not sound: the third pass
fixed each document path at one sink but not at the other sinks that consume the
same document fields. H1 was reproduced end to end — `Bundle::open` accepted a
`stored_path` of `../../../outside.zst`, and `materialize` wrote
`expected/../../../escaped.dll` and `game/../../../escaped.dll` (exit `3`). H3's
alias behaviour was reproduced on this host (see H3).

## H1 — `verify` / `materialize` still join document paths unvalidated

**Severity: high. Class: bug.**

The third pass's read-side guard is real, but it covers one sink:

```rust
// src/assemble/mod.rs:604-615
// **The read-side twin of the manifest check.** `stored_path` is
// a document field inside a bundle someone may have edited, and
// this is the join that turns it into a file handle. The shape
// rule stops a bundle from making `verify` or `materialize`
// read outside its own root, whatever the capture that wrote the
// index did.
if let Some(problem) = crate::paths::relative_path_problem(rel) {
    return Err(format!(
        "chunk {chunk_id}: the index's stored_path {problem}"
    ));
}
let path = root.join(rel);
```

That guard lives in `ChunkSource::decoded`. `src/verify/mod.rs` does not
reference `crate::paths` at all, and its own A6 check reads the same field
directly:

```rust
// src/verify/mod.rs:528-543
fn check_blobs(root: &Path, index: &ChunkIndex) -> Vec<Assertion> {
    let mut rows = Vec::new();
    for b in &index.blobs {
        let id = b.props.chunk_id.as_str();
        let Some(rel) = b.props.stored_path.as_deref() else {
            // ...
            continue;
        };
        let path = root.join(rel);
        match std::fs::read(&path) {
```

The patchmap's `path` — a second document field — is joined for the
tree-relative row and for the shipped-bytes read:

```rust
// src/verify/mod.rs:617
let rel = format!("{tree}/{}", r.path);

// src/verify/mod.rs:650
match std::fs::read(root.join(&rel)) {
```

`Bundle::open` validates only the schema version. It reads the index and every
patchmap without any path check:

```rust
// src/verify/mod.rs:172-177
let manifest: Manifest = read_json(&manifest_path)?;
let index: ChunkIndex = read_json(&root.join("chunks/index.json"))?;
let checksums: Checksums = read_json(&root.join("verify/checksums.json"))?;
let recorded: Selfcheck = read_json(&root.join("verify/selfcheck.json"))?;
let error_shapes: ErrorShapes = read_json(&root.join("api/error_shapes.json"))?;
let patchmaps = read_patchmaps(root)?;
```

**Concrete failure.** An operator runs `gircap verify` on a bundle they did not
produce. The bundle's `chunks/index.json` names
`"stored_path": "../../../Users/<name>/.ssh/id_ed25519"` (or any file the
operator can read). A16 fails because the index changed, but `verify` does not
return there: the A6 call at `:411` runs unconditionally, `check_blobs` joins
the value and `describe_bytes` publishes the file's length, md5 and sha-256 into
`verify`'s output. If the file is a zstd frame it is decompressed as well. With
a matching patchmap `path`/`referenced_by.path` the A18 read at `:650` also
compares bytes from outside the root. `gircap materialize` on the same bundle
writes the rebuilt bytes through the patchmap path:

```rust
// src/run/mod.rs:644-652
for pair in &report.rebuilt {
    for (side, tree) in [(&pair.old, "game"), (&pair.new, "expected")] {
        let Some(s) = side else { continue };
        let rel = format!("{tree}/{}", s.path);
        let dest = out.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| write_err(&plan.artifact, e))?;
        }
        std::fs::write(&dest, &s.bytes).map_err(|e| write_err(&dest, e))?;
```

`out` is `--out` or a sibling directory, and `out.join("game/../../../x")`
leaves it. The `out.starts_with(&plan.artifact)` warning at `:634` tests the
chosen output directory, not each destination, so it does not catch this.

A related sink has a narrower precondition: when `--artifact` is a filesystem
root (so `file_name()` is `None`), the default materialize output name comes
from the document's `MANIFEST.json → capture.id`
(`src/run/mod.rs:622-630`); a tampered id containing `..` or a separator then
travels into `out.join(rel)`. This is the same root cause (a document field
naming a path) but needs a root artifact, so it is recorded here rather than as
a separate finding.

**Why it fails the standard.** `verify` is the command an operator runs on a
bundle they may not have produced, which is exactly the boundary the third pass
declared closed. The comment at `assemble/mod.rs:604-609` says the shape rule
stops `verify` or `materialize` from reading outside the root; A6 never passes
through that guard. A document value reaching `root.join` without a shape check
is the G1 defect, not a new class, and validation at one of several sinks is not
validation at the boundary.

**Suggested fix.** Validate every document-supplied path once in
`Bundle::open`: `stored_path`, each patchmap `path`, and `capture.id` (which is
also a file-name candidate), refusing the bundle with the field and value named.
Keep the `ChunkSource::decoded` guard as defence in depth, and add an
end-to-end test that opens a bundle whose index `stored_path` is `../outside`
and asserts `verify` exits non-zero without reading the file (and that
`materialize` writes nothing outside `--out`).

**Resolution.** Fixed in `0a21ebe`. `verify::validate_document_paths` runs over
the manifest's `capture.id`, every index `stored_path` and `referenced_by[].path`,
and every patchmap `path`; the A6/A18 joins and the materialize destination join
keep their checks as defence in depth. The three new `offline_bundle` tests
failed before the commit and pass after it.

## H2 — `--audio` becomes `matching_field` and reaches manifest filenames raw

**Severity: medium. Class: bug.**

`--audio` is a plain repeatable string flag (`src/cli/mod.rs:173-174`). Its
validation is only the `none` sentinel and duplicate detection; the value is
otherwise just trimmed, lowercased and `_`→`-`:

```rust
// src/config/mod.rs:452-463
let mut seen: Vec<&String> = Vec::new();
for code in &normalised {
    if seen.contains(&code) {
        return Err(UsageError::new(format!(
            "--audio {code} was given twice; a repeated language adds no manifest \
             to the selection pool"
        )));
    }
    seen.push(code);
}
Ok(AudioSelection::Codes(normalised))
```

```rust
// src/config/mod.rs:465-468
/// Lowercase, and accept `_` for `-`, so `EN_US` and `en-us` are the same language.
fn normalize_audio(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}
```

The value is then the field name used to select and file manifests:

```rust
// src/run/mod.rs:503-506
fn selection_fields(plan: &CapturePlan) -> Vec<String> {
    let mut fields = vec![manifest::MANIFEST_FIELD.to_string()];
    fields.extend(plan.audio.codes().iter().cloned());
    fields
}
```

`manifest::fetch::wants` copies that string into `field_name`
(`src/manifest/fetch.rs:64-73`), `fetch_all` passes it to `manifest::capture`
(`:127`), and `capture` stores it back as `matching_field` on the captured
manifest and its meta (`src/manifest/mod.rs:465`, `:515`). The dump then
interpolates it directly into four artifact-relative names:

```rust
// src/manifest/dump.rs:32-48
pub fn archive_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest.zst", dir(tag))
}
pub fn meta_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest.meta.json", dir(tag))
}
pub fn full_prefix(tag: &str, field: &str) -> String {
    format!("{}/{field}.manifest", dir(tag))
}
pub fn paths_rel(tag: &str, field: &str) -> String {
    format!("{}/{field}.paths.tsv", dir(tag))
}
```

Those names are passed to `emit::write_rel` / `write_json_gz` /
`write_newline_doc` (`dump.rs:83`, `:119`, `:131`, `:145`), each of which ends in
`root.join(rel)` (`src/emit/mod.rs:319`, `:400`). The tag half of the same path
is validated by `tag_shape_problem` (`config/mod.rs:564-604`, the F6 fix); the
field half has no equivalent.

**Concrete failure.** With a build document that publishes the field — the live
API will not, but `--hyp-base`/`--sophon-base` (documented testing seams,
`src/cli/mod.rs:214-218`) or a compromised/proxied API will — the run is:

```
gircap capture --audio ../../../../evil ... --out C:\work\bundle
```

`selection_fields` yields `../../../../evil`; `manifest::capture` finds it via
`build.manifest_for(...)` (`src/manifest/mod.rs:376`); the dump writes
`C:\work\bundle\manifests\<tag>\../../../../evil.manifest.zst`, which resolves
outside `--out`. Because the dump happens in `emit::tree::write` before the
audio scan file is built, a later "unknown audio code" refusal in
`emit::gamedir::write_prelude` does not undo the write.

**Why it fails the standard.** The standard requires a boundary check where the
value becomes a path. This value is not even run through the shared shape rule,
while the adjacent `tag` is. The `matching_field` originates in a document
(the build response) as much as in the flag, so trusting the flag's spelling is
not enough: validate the field when it is selected and again when it names a
file.

**Suggested fix.** Apply `crate::paths::relative_path_problem` to each normalised
audio code in `validate_audio`, and reject a field that is not a single
component. Keep a second check in `manifest::dump` (or in a `field_rel` helper)
so a manifest loaded from any other source cannot name a path either. Add a
unit test for audio values containing `/`, `\` or `..`, and an end-to-end test
with a mock build publishing such a field that asserts nothing lands outside
`--out`.

**Resolution.** Fixed in `0b55a40`. `paths::field_component_problem` — the path
rule plus "no `/`" — is applied in `config::validate_audio`, in
`manifest::capture` for a field selected from the build document, and in
`manifest::dump::write` before the first write. Unit tests cover `a/b`, `/abs`,
`a\b`, `../x`, `..` and the empty field at both ends.

## H3 — The path rule accepts names Windows folds together

**Severity: low. Class: bug.**

The shared rule rejects `..`, `\`, absolute paths, `:`, empty components and
control characters, but not `.` components, trailing dots or trailing spaces:

```rust
// src/paths.rs:55-63
if value.split('/').any(|c| c == "..") {
    return Some(format!("{value:?} escapes the bundle root"));
}
if value.split('/').any(|c| c.is_empty()) {
    return Some(format!(
        "{value:?} has an empty component (a doubled or trailing separator)"
    ));
}
None
```

`a/b`, `a/b.` and `a/./b` all return `None` on every host. On Windows the first
two are one file; a PowerShell probe in the same session wrote `a\b` and `a\b `
and the directory held one entry, `b`, with the second write's content. A Rust
probe outside the repository wrote the four names `game/a/b`, `game/a/b.`,
`game/a./c`, `game/a/c` and read them back:

```
game/a/b: Ok("game/a/b.")
game/a/b.: Ok("game/a/b.")
game/a./c: Ok("game/a/c")
game/a/c: Ok("game/a/c")
walk: ["game\\a\\b", "game\\a\\c"]
```

So four written paths produced two directory entries, each holding the last
write. In `gircap`, `validate_entry_paths` (`src/manifest/mod.rs:532-544`) admits
both spellings of two different manifest entries; `emit::gamedir::write_variant`
formats each as `format!("{tree}/{}", file.rel_path)` and `write_rel` joins it
(`src/emit/gamedir.rs:192-201`); the second write silently replaces the first.
`MANIFEST.json → files[]` and `checksums.json` are built from the tree walk
(`src/emit/index.rs:245-275`, `:360`), so they list the physical name once,
while the selfcheck compares the in-memory written list against the admitted
paths (A14, `src/emit/selfcheck.rs:493-522`) and can pass. The capture can
therefore exit 0 and describe a file set it does not hold; a later `verify`
A18 compares the rebuilt bytes for the folded-away logical path against the
surviving file and fails unless the two writes were byte-identical.

**Why it fails the standard.** "No `..` component" is not the same as "names
one file". The same reasoning the third pass applied to tags — trailing dots
are rejected because Windows strips them and two tags could name one directory
(`config/mod.rs:595-600`) — applies to manifest paths and is missing. A
validator that is not injective on the host it runs on is not a boundary check
for a bundle that is meant to be read on Windows.

**Suggested fix.** Reject `.` components, any component ending in `.` or a
space, and (for portability) Windows-reserved device names, in
`relative_path_problem`. This is intentionally stricter than Unix needs, the
same choice already made for tags. Add a test that two entries differing only
by a trailing dot are refused before any tree is written.

**Resolution.** Fixed in `c9caa02` (`.` and trailing `.`/space) and `b6aeb6d`
(device names). The rule is host-independent and shared by `--files`, manifest
entries, index paths and the scan directory. Unit tests pin both refusals, keep
`a/b.c`, `a/7.1.0`, `a/.hidden`, `a/CONSOLE` and `a/COM0` legal, and a
`manifest::tests` case pins that an entry is refused before a tree write.

## H4 — The `--no-chunk-blobs` schema row contradicts `unmeasured`

**Severity: low. Class: doc error.**

```markdown
<!-- docs/01-artifact-schema.md:635-636 -->
- **No fetch was attempted** — a `--no-chunk-blobs` row. It has
  `decode_error: null`, `verified: null` and empty digest strings, and it means
  "not measured", not "failed"; the flag also skips A6–A12, A19 and A21–A23
  (see §16).
```

The constructor that produces that row fills the decompressed digest and the
declared size from the manifest:

```rust
// src/blobs/props.rs:354-368
pub fn unmeasured(chunk: &SophonChunk) -> BlobProps {
    BlobProps {
        chunk_id: chunk.id.clone(),
        stored_path: None,
        bytes: 0,
        container: "zstd".to_string(),
        md5_compressed: String::new(),
        md5_decompressed: chunk.uncompressed_md5.clone(),
        sha256_compressed: String::new(),
        uncompressed_size: chunk.uncompressed_size.max(0) as u64,
        unknown_field6: chunk.unknown_field6,
        verified: None,
        decode_error: None,
    }
}
```

Only the two compressed digests are empty. `md5_decompressed` is the manifest's
declared digest and `uncompressed_size` is its declared length; the unit test
`the_plan_path_records_the_sizes_the_manifest_declared` in
`src/blobs/fill.rs` pins the size. The "not measured" reading is right (the
`verified` verdict is null), but "empty digest strings" is not.

**Why it fails the standard.** A reader who trusts the row will expect every
digest field to be `""` and may treat a populated `md5_decompressed` as a
measurement that was taken. The distinction between declared and measured is the
point of this paragraph, so the sentence currently erases it. The G3 fold
(`ADVERSARIAL_REVIEW.md:639`) repeats the same phrase.

**Suggested fix.** Say what `unmeasured` does: `stored_path`, `bytes`,
`md5_compressed` and `sha256_compressed` are empty/zero; `md5_decompressed` and
`uncompressed_size` carry what the manifest declared; `verified` is null and
`decode_error` is null. Correct the G3 row in `ADVERSARIAL_REVIEW.md` when that
file is next edited; this note does not modify it.

**Resolution.** Fixed in `c9bf161`: the schema paragraph, the
`BlobProps::unmeasured` comment and the `md5_decompressed`/`uncompressed_size`
field docs now say declared-not-measured. The G3 row in `ADVERSARIAL_REVIEW.md`
was corrected in the same fold that added the fourth-pass subsection.

## Verification

Run in `E:\hoccode\hocrust\girpr` on the branch above after the fixes, in this
order.

| Command | Result |
|---|---|
| `cargo build -p gircap` | `Finished dev profile ... in 0.50s`, no warnings. |
| `cargo test -p gircap` | **658 passed, 0 failed, 0 ignored** — 518 lib + 140 integration (3 + 8 + 32 + 2 + 30 + 9 + 29 + 12 + 15); `main.rs` 0 tests; doc-tests 0. Baseline was 649, so the pass added nine tests. |
| `cargo clippy -p gircap --all-targets` | `Finished dev profile ... in 19.86s`, no warnings. |
| `cargo fmt -p gircap --check` | Exit `0`, no output. |

**New coverage.** `tests/offline_bundle.rs` gained three crafted-bundle tests:
an index `stored_path` of `../../../outside.zst` is refused by `Bundle::open`
and by `verify` (exit `1`), a patchmap and index reference of
`../../../escaped.dll` makes `materialize` exit `1` with nothing written outside
`--out`, and a `capture.id` of `../../escape` is refused. All three failed
before `0a21ebe`. `paths::tests` pins the `-shell`/`-audio`/entry rule and the
new `.`/trailing-dot/trailing-space/device-name refusals;
`manifest::tests` pins that an entry is refused before a tree write;
`config` and `manifest::dump` tests pin the field rule on both the flag and the
document side. The rows that already pinned the `unmeasured` values
(`props::an_unmeasured_blob_is_not_usable_and_says_so`,
`fill::the_plan_path_records_the_sizes_the_manifest_declared`) keep H4's
documented behaviour honest.

**Pre-fix reproduction.** `Bundle::open` accepted
`stored_path: "../../../outside.zst"` and returned `Ok`; `materialize` wrote
`expected/../../../escaped.dll` and `game/../../../escaped.dll` (exit `3`).
The Windows alias probe quoted under H3 is unchanged and still outside the
repository (`E:\Symlinks\Temp\opencode\pathprobe`, not committed).

No `#[cfg(unix)]` test exists in this crate, so nothing was skipped on this
Windows host; the `#[cfg(windows)]` tests ran.

## Not verified

- **H2 with a real server.** `--audio ../../../../evil` was not sent through a
  mock or proxied build document. The flag-to-`dump.rs` chain is covered by unit
  tests at both ends; whether the API publishes the field is the precondition,
  and only a hostile or proxied `--hyp-base`/`--sophon-base` can satisfy it. No
  live capture was run.
- **H3 against a real manifest.** No live manifest containing `a/b` and `a/b.`
  was sampled; the alias itself is proven by the probe, and the refusal is
  pinned by unit tests and by `validate_entry_paths`.
- **H4 on disk.** No `--no-chunk-blobs` bundle was inspected to read the actual
  `md5_decompressed` value; the doc now matches the constructor and the
  `fill.rs`/`props.rs` tests that pin it.
