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

Start with the spec.

## Status

Built to milestone **M2** (metadata endpoints). A live `capture` writes the
seven `api/` documents — the four HoYoPlay bodies and both `getBuild`s
**byte-exact**, plus the generated tag probe — and `api/requests.ndjson` beside
them, then validates what the capture depends on. Manifests, chunks and the
`game/` fixture land in M3, M6 and M8; `verify` / `materialize` / `list` are M9.
The build order lives in `docs/02-plan.md` §3 and each module's own header says
which milestone it lands in.

```powershell
# artifacts/hk4e_global/<old>__<new>/<id>/api/*.json
.\target\release\gircap.exe capture --out artifacts

# Metadata-only, against a local server (the testing seam M10 needs)
.\target\release\gircap.exe capture --hyp-base http://127.0.0.1:8080 --sophon-base http://127.0.0.1:8080
```