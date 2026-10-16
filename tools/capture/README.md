# gircap — live-artifact capture for `girpr` testing

**Dev-only. Not a patcher, not part of the `girpr` binary, shares no code with
it.** It talks to the real HoYoPlay + Sophon APIs and writes a self-contained
artifact bundle: a real `game_dir` fixture holding one real file per test-matrix
variant, every API response and manifest behind it, every chunk blob needed to
materialize the old file, the current file, and the patch between them, and
ground truth for the result.

The point: an agent or test can later run the main tool against `game/` and
assert, offline and by hash, that it produced `expected/` and fetched exactly
the chunks the artifact says it must fetch.

```powershell
# Capture hk4e_global 7.0.0 -> 7.1.0 (defaults: new = main.tag, old = diff_tags[0])
.\target\release\gircap.exe capture --biz hk4e_global --out artifacts

# Re-check the bundle from stored bytes, no network
.\target\release\gircap.exe verify --artifact artifacts\hk4e_global\7.0.0__7.1.0\<id>

# Rebuild old + new files from the retained chunks, no network
.\target\release\gircap.exe materialize --artifact <same dir>
```

## Docs

| | |
|---|---|
| [`docs/00-spec.md`](docs/00-spec.md) | what it captures, from where, and the invariants it asserts |
| [`docs/01-artifact-schema.md`](docs/01-artifact-schema.md) | the bundle's byte layout and every field of every document |
| [`docs/02-plan.md`](docs/02-plan.md) | module tree, build order, tests, risks |
| [`docs/03-refactor-plan.md`](docs/03-refactor-plan.md) | making it cheaper and smaller, ranked by risk |

Start with the spec.

## Artifact bundles

`capture` is the only command here that touches the network. Everything else —
`verify`, `materialize`, `list` — runs entirely from stored bytes, which is what
makes a bundle a **cache of the live world** rather than a build output.

**Where they live.** `--out` defaults to `./artifacts`, laid out as
`artifacts/<biz>/<old_tag>__<new_tag>/<capture_id>/`. The directory is gitignored:
bundles are large (≈129 MiB for `hk4e_global` 7.0.0 → 7.1.0, of which
`manifests/` is ≈43 MB and `chunks/` ≈28 MB) and regenerable, so they belong on
disk, not in history. **This is the project location**: anything a test or
another tool is expected to rely on lives here.

**When to re-capture.** A bundle is out of date when the world moved or when
this tool moved:

- you need a version pair the existing bundles do not cover;
- `gircap`'s own output changed — a new document, a changed field, a bumped
  `schema_version` — and an existing bundle no longer represents what the tool
  writes today;
- a test wants a pair that only exists upstream now.

Do **not** re-capture merely to refresh a bundle you are not otherwise changing.
A capture downloads tens of megabytes of blobs; doing that on a whim is the
difference between a five-second loop and a two-minute one.

**Being efficient about it.**

- **Scratch first.** `--out <some temp dir>` writes exactly the same tree
  somewhere disposable. Iterate there: the repo stays clean and there is nothing
  to clean up afterwards. Use `--hyp-base` / `--sophon-base` against a local
  server when you only need to exercise the writer.
- **Promote when it settles.** Capture into `artifacts/` only once the bundle is
  good and you expect to re-use it — in practice, while `gircap` itself is not
  changing shape. A bundle in `artifacts/` is meant to be a stable fixture that
  other work depends on, not a scratch directory with a permanent name.
- **Check before you promote.** `gircap verify --artifact <dir>` must exit `0`.
  It is the cheapest possible answer to "is this usable", and it is what stops a
  later commit from discovering the problem.
- **Keep the pair in mind.** `--new-tag` defaults to `main.tag` and `--old-tag`
  to `diff_tags[0]`, so an unpinned capture follows whatever upstream has moved
  to. Pin both when you want a specific pair, especially when the result is going
  to be kept.

**Compatibility.** Every document carries `"schema_version": 1`, and a reader
should reject an unknown one rather than guess. Additive changes leave bundles
readable — `verify` reports what it cannot re-derive instead of refusing the
bundle — so a bundle you already paid for does not become unreadable because the
tool gained a document. A bundle whose `schema_version` has moved on, or whose
selfcheck does not pass, is not a fixture: re-capture it.

## Status

Built through milestone **M13**. A live `capture` writes the `api/` documents
byte-exact — including `api/tag_probe.json` and `api/error_shapes.json`, the two
generated measurements — fetches and verifies the `game` manifest for both
builds, diffs them, and assembles a **complete artifact**:

| | shipped |
|---|---|
| `api/` | six response bodies, byte-exact, + `tag_probe.json` + `error_shapes.json` + `requests.ndjson` |
| `diff/` | `paths.diff.json`, `paths.tsv`, `candidates.json`, `reuse_pool.json` |
| `chunks/` | the three retention sets, every blob, `index.json` with `build_id` on each reference |
| `game/` / `expected/` | the fixture and the ground truth, per capture |
| `patchmaps/` | one per capture: where every chunk's bytes come from |
| `verify/` | `checksums.json` (A16) and `selfcheck.json` |
| commands | `capture`, `verify`, `materialize`, `list` |
| extras | gzip above the threshold, and assertions A18/A19/A20 |

Both reference bundles — `hk4e_global` 7.0.0 → 7.1.0 — **verify 90/90, exit
`0`**. The end-to-end check is **automated**: the bundle is replayed
over loopback and the real `girpr` binary is run against it, once per capture,
by `girpr`'s `tests/artifact_bridge.rs`. All eight captures exit `0` and every
`download_bytes` matches its patchmap to the byte (§8.1.1).

```powershell
# in the girpr package. Expensive (~2 min) and run by hand only when a
# repair-spec change is suspected; needs a bundle and FAILS without one.
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

The harness lives on the `girpr` side and **must**: `gircap` does not depend on
`girpr` and does not invoke it, so the check that `girpr` works does not couple
the two tools. The spec is `girpr`'s `docs/06-artifact-bridge-spec.md`, and the
consumer never
regenerates or repairs a bundle — it reads `patchmaps/`, `expected/` and `game/`,
and runs `gircap verify` as its precondition.

The build order lives in `docs/02-plan.md` §3 and each module's own header
says which milestone it lands in.

```powershell
# artifacts/hk4e_global/<old>__<new>/<id>/{api,manifests,diff,chunks,game,expected,patchmaps,verify}/
.\target\release\gircap.exe capture --out artifacts

# Metadata-only, against a local server (the testing seam)
.\target\release\gircap.exe capture --hyp-base http://127.0.0.1:8080 --sophon-base http://127.0.0.1:8080

# What is on disk. One `bundle` record per bundle, then one `summary`.
.\target\release\gircap.exe list --root artifacts
.\target\release\gircap.exe list --root artifacts --output json
```