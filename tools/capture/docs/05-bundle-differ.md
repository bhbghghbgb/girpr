# The bundle differ — how the output-contract history was measured

Companion to [`04-output-contract-history.md`](04-output-contract-history.md),
which reports what it found. This document is about the instrument: what it
normalises, why each exemption exists, how it was proved to work, and how to
re-run it.

**The problem it solves.** `gircap`'s own band-1 gate (`tests/bundle_diff.rs`)
compares **two captures of one binary against each other**. That catches a
change that is *non-uniform* — a row that flips depending on which worker woke
first. It is blind, by construction, to a change applied **uniformly to every
capture**: exactly the kind this history is made of. `7771323` fills in six
`stored_as` values; both captures of one binary get it, so `bundle_diff` is
green. §5.5 of the plan names this blindness twice and calls the fix
"per-field pins"; this differ is the instrument that makes it unnecessary,
because it compares **across** binaries.

The tool is `tools/capture/tools/bundle_differ.py`. It has no dependencies
beyond the Python standard library, and it reads bundles; it never runs
`gircap`.

---

## 1. What is compared

A bundle is captured twice from one commit, against the crate's own loopback
mock (`tests/common/mod.rs`, `World::five_classes()`). Each capture is walked,
every file read, and:

| step | what |
|---|---|
| gunzip | `.gz` payloads are decompressed before comparison |
| scrub | the run-local tokens (§2) are replaced, **in place on the raw bytes** |
| exempt | the cascade digests (§3) are blanked, per named document |
| compare | two fingerprints are compared file-set first, then byte-for-byte |

Scrubbing rewrites bytes, never JSON. It does not round-trip through
`serde_json`, so key order, indentation and every other byte are still compared:
a reformatted `MANIFEST.json` still fails.

## 2. The run-local tokens, and how the list was arrived at

Not assumed — **discovered**, by capturing twice from one binary and diffing
*without* normalisation first. The raw diff of two captures of one binary is
exactly five documents:

```
  api/requests.ndjson            ts, elapsed_ms
  MANIFEST.json                  argv's --out; its own digest row; bytes_on_disk
  README.md                      "N B on disk"
  verify/checksums.json          digests of the four above; total_bytes
  verify/selfcheck.json          A16's "N files, M B"
```

which is the cascade, exactly as `tests/bundle_diff.rs` documents it. After
scrubbing those five, two captures of one binary are identical — which is the
check that the normalisation is not vacuous, and it is re-run for **every**
commit probed (see §5).

| token | where it appears |
|---|---|
| the capture id | the bundle directory's own name |
| the `--out` root | `MANIFEST.json → capture.tool_argv` |
| RFC 3339 instants, both precisions | `ts` (ms) in the log, `captured_at` / README (s) |
| `elapsed_ms` | every request row |
| `127.0.0.1:<port>` | **see below** |

### The one that bit, and why it is worth writing down

The mock binds `127.0.0.1:0` and takes an ephemeral port. That port is
**recorded throughout the bundle**: `MANIFEST.json → biz.hyp_base`, every
`getBuild` body's `url_prefix`, every `manifests/*/*.meta.json`, every
`patchmaps/*.json` chunk URL, every `api/error_shapes.json` URL, and every row
of `api/requests.ndjson`.

Two captures of **one** binary share a mock, so the port matched and the
self-check passed — the trap only opens when you compare **two commits**, which
is the only thing this differ is for. It reported 13 of 58 documents as changed
between two commits that had changed nothing at all.

Fixing the token in the bodies collapsed that to 2. The residue was the
*digests* of those bodies, recorded in `MANIFEST.json` and
`verify/checksums.json` — content that compares equal after scrubbing has a
digest that does not.

The residue is handled by **detecting** which files carry a run-local value,
rather than by listing them:

```python
volatile = {rel for rel, b in raws.items() if b"127.0.0.1:" in b or rel in DERIVED}
```

Twelve of 58 files qualify under the mock, and the digest rows for exactly those
twelve are blanked. The other 46 — every chunk blob, every assembled `game/`
and `expected/` file, every `diff/` and `manifests/` document — keep their
digests compared, which is most of the signal a change would show up in.

**The general lesson, and the reason this paragraph exists:** a self-check that
only ever compares one binary to itself cannot find a volatile field that one
binary happens to share with itself. Every exemption here was proved by the
mutation control in §4, and every *other* volatile field by a diff that was
taken raw first.

## 3. The cascade exemptions

`api/requests.ndjson` is not byte-reproducible, and four documents record its
digest. So does every file that embeds the port. For those, the digest row is
blanked rather than compared:

| document | what is blanked |
|---|---|
| `MANIFEST.json` | the `files[]` digest triple of each volatile path; `bytes_on_disk` |
| `README.md` | the `N` in "… B on disk" |
| `verify/checksums.json` | the same rows; `total_bytes` |
| `verify/selfcheck.json` | the `M` in A16's "N files, M B" |

Every other byte in those four documents is still compared. That is deliberate:
`d745680` and `43f66fa` later added per-field pins for exactly these documents
inside the crate, and the two mechanisms are complementary — the crate's pins
catch drift within one binary, this catches drift across the history.

The exemption list **must match something**, or the differ fails rather than
quietly comparing nothing (`--strict`).

### A note on `encoding: gzip`

For a gzipped document, `files[].bytes` and its digests are the
**decompressed** size and digest, not the `.gz` bytes on disk. Measured on
`artifacts/…/20261017T123335Z`:

```
  manifests/7.0.0/en-us.manifest.json.gz   on disk 1 356 632 B
                                           recorded 5 700 141 B   <- decompressed
  md5(gz bytes)  3c55a60901bc25e212fa7da96e086f16
  recorded md5   6b772797cfe10a764b4ba440952bfc57           <- of the payload
```

So the compressed bytes are **not** part of the recorded contract at all, and
comparing `.gz` payloads decompressed is the right granularity. The one place
they could still have mattered is `87412d1`, which changed how the `.gz` is
produced; it was checked separately, against a capture forced down the `.gz`
path (§6).

## 4. Proving the differ works

A differ that scrubs too much reports "identical" for two different bundles,
and every conclusion drawn from it is then worthless. So the nine kinds of
change the history actually made are injected into a known fingerprint, and the
differ is required to catch each one:

```
  CAUGHT  stored_as filled in on an api row   (7771323)
  CAUGHT  a request header value changed     (cd0e876)
  CAUGHT  a chunk blob's bytes changed
  CAUGHT  an assertion's wording narrowed   (43f66fa)
  CAUGHT  a document added
  CAUGHT  a document removed
  CAUGHT  attempts=2 on a probe row         (527833c)
  CAUGHT  bytes_in set on a 404 row         (844f5c2)
  CAUGHT  paths.tsv rows reordered          (sort drift)
  OK     unmutated copy reads identical
```

The last line is the negative control, and it is the one that matters: a
harness that always fires proves nothing.

Each mutation asserts its own precondition first — that the fixture really
contains the string it is about to change — so a mutation cannot pass by
quietly doing nothing. That is not hypothetical: the first run of the wording
mutation asserted against text this bundle does not contain, and failed on its
own precondition rather than silently "passing".

## 5. The per-commit guard

Every probed commit is captured **twice** and the two fingerprints must agree,
or the probe is rejected and no fingerprint is cached. That is what stops a
badly-normalised commit from entering the search as a false boundary — the
failure mode where one commit looks like it changed the output because its
`elapsed_ms` happened to be one digit wider.

## 6. Profiles, and what each one can and cannot see

One profile, `World::five_classes()`, produces a **58-file** bundle covering
every document type the tool writes:

| | mock | reference (`20261017T123335Z`) |
|---|---|---|
| files | 58 | 97 |
| `api/` | 9 | 9 |
| `manifests/` | 16 | 16 |
| `diff/` | 5 | 5 |
| `patchmaps/` | 5 | 8 |
| chunk blobs | 10 | 38 |

It is small, deterministic, offline, and produces the whole document surface —
which is what makes 27 builds affordable. It is also a *healthy* capture, and a
healthy capture cannot see three things:

| not seen | why | how it was covered |
|---|---|---|
| the `.gz` write path | the synthetic manifests are 1.0–3.7 kB and the default threshold is 1 MiB, so nothing is ever gzipped | a second profile with `--gzip-threshold 512` |
| a mis-served chunk | every blob the mock serves is a real zstd frame | the mock patched to serve non-frame bytes |
| a non-2xx on the retryable path | the world answers 200, and the probes' 404s go through `observe`, which read the body regardless | the mock patched to answer 503/429 with a body |

The mock patch (`tools/capture/tools/mockpatch.py`) adds two builder methods and
one dispatch check in `serve`. It is anchored, so it refuses to half-apply if
the mock's shape has moved, and it is idempotent.

Two shapes had to be anchored, because the mock's `serve` changed once inside
this range: before `428fc8e` it handled one request per connection and returned,
after it speaks keep-alive and loops. The anchors are matched as *line
prefixes* — the HEAD shape writes `… tag.as_deref()) {` and the earlier one
`… else {`, so the anchors deliberately omit whatever follows.

Two more details are not obvious, and each cost a run to find:

- **The needle is matched against the whole request target, path *and* query.**
  The negative probes share their *paths* with the metadata capture
  (`/sophon/getBuild`, `/hyp/getGameConfigs`), so a path-only needle cannot tell
  "the capture asked for this" from "the probe asked for this on purpose". An
  early version armed `/chunk/`, believing it selected chunk fetches; it also
  selected the 404 probes, which live under the same prefix — so it measured
  `observe`, which already reads the body regardless, rather than the path under
  test.
- **Armed rules are re-armed between the probe's two captures.** Otherwise the
  second capture meets a mock that has already spent its one armed answer, the
  two captures legitimately differ, and the determinism guard rejects the commit
  for a reason that has nothing to do with the tool. The guard is right to
  reject; the instrument was wrong.

**The patcher was wrong four times before it was right**, and each failure had
the same shape: a textual edit that silently did not do what it looked like it
did, surfacing as a syntax error fifty lines from the cause.

1. The block was *substituted in place of* its anchor instead of inserted before
   it, so the `let … else {` the block depended on never appeared.
2. One anchor constant lost its closing `]` in an edit — `"#![allow(dead_code]"`
   became `"#![allow(dead_code)"` — so the patch matched four characters short
   and ate the `]` out of the file.
3. The guard written to catch (2) compared a *stripped* line against an
   *indented* anchor, so it never matched anything and never fired. A check that
   cannot fail is not a check.
4. The brace-balance guard that did fire then caught the next one: an "insert
   after" splice that *replaced* the anchor line, silently dropping the `{` that
   opened `impl Mock {`.

The lesson is §4's, applied to the instrument rather than the tool. The patcher
now requires each anchor to match exactly one line, requires exactly one of the
two `serve` variants to match, verifies the inserted block adds as many `{` as
`}`, and verifies every anchor survives the write — and it is exercised against
seven mock shapes across the range before being trusted.

**It is applied only for the armed profiles, never for the ordinary search**, and
that is not tidiness. The arming probe names `arm_fail`/`arm_body`
unconditionally and Rust type-checks both match arms whatever the runtime
condition is, so a single probe binary cannot serve both cases; and keeping the
search path on the tree's own unmodified mock means every fingerprint in the
history was produced by a mock that is byte-for-byte the one in the repository.

## 7. The search, and why it is not a sweep

Two captures of one binary tell you nothing about history. 56 adjacent
comparisons would. So the boundary is found by divide and conquer, using a
predicate that is monotone:

> P(lo, hi) — "some change lies in (lo, hi]" — is true **iff** F(mid) ≠ F(lo).

One probe of the midpoint splits the interval and proves one half
boundary-free. For *k* boundaries among *n* commits that is O(k·log(n/k)) builds
instead of n−1, and the results are cached, so a re-run costs nothing.

The baseline is included as index 0 deliberately. It is the **parent** of the
first src commit; leaving it out makes the first interval compare the first
commit against a midpoint, which silently attributes that commit's own change to
a later one. (It did, once, before it was noticed.)

## 8. Validated against the real bundles, not only the mock

The mock is convenient and deterministic; it is also not the thing anybody
reads. So the same differ was run over the three reference bundles in
`artifacts/hk4e_global/7.0.0__7.1.0/`, with no build at all:

| comparison | identical | changed |
|---|---|---|
| `20261016T002107Z` vs `20261017T000730Z` — both **before** the band-3 work | 93 of 97 | 4 |
| `20261017T000730Z` vs `20261017T123335Z` — **across** it | 93 of 97 | 4 |

The first row is the important one: two captures of the same world, from
different days, differ in **four** documents. That is the volatility floor of a
real capture, and it matches the mock's five closely enough to be a check on the
normaliser rather than a coincidence:

| document | why it moves on a live capture |
|---|---|
| `MANIFEST.json` | `tool_argv[0]` is the binary's own path — `target\debug\…` vs an absolute path |
| `api/error_shapes.json` | the CDN's per-request `RequestId`, echoed in the recorded 404 body |
| `api/requests.ndjson` | `ts`, `elapsed_ms`, and CDN response headers |
| `verify/checksums.json` | the digests of the three above |

`api/error_shapes.json` is the one the mock does **not** need to exempt — its
`RequestId` is a constant there — which is exactly why it is worth running the
differ over real bundles once: a live capture has one more volatile document
than a mock capture, and a gate that only ever sees mock captures will not know
that.

The second row is the finding the history is about, measured on real data rather
than on a synthetic world: **every chunk blob, every assembled `game/` and
`expected/` file, every `patchmaps/`, every `diff/`, every `manifests/` document
is byte-identical across the band-3 work.** The only document carrying a
contract change is `api/requests.ndjson`.

## 9. Cost, honestly

| | |
|---|---|
| cold build including all dependencies | 7 m 42 s |
| rebuild of `gircap` alone, shared target dir | ~3.3 min |
| two captures + fingerprint + both diffs | ~1.5 s |
| commits actually built | 19 for the search, 8 for the degraded-input pairs; a sweep would be 56 |

Sharing one `CARGO_TARGET_DIR` across the worktrees is what makes the per-commit
cost 3.3 minutes rather than 8: dependencies are compiled once and only
`gircap` itself is rebuilt. On a 4-core machine, two builds do not run
concurrently — cargo takes a target-directory lock — so this is deliberately
serial.

## 10. Reproducing it

```powershell
# one commit
pwsh -File tools/capture/tools/probe.ps1 -Commit <sha> -Profile default

# the whole search
python tools/capture/tools/search.py

# prove the differ still works
python tools/capture/tools/mutation_control.py
```

The driver checks out each commit in a `git worktree`, copies in the probe, and
leaves the build cache alone. No packet leaves the host: the mock is the only
listener and binds `127.0.0.1:0`.

## 11. What this instrument does not establish

- **It is not the crate's own suite.** `cargo test` is what proves the code is
  correct; this proves what the bytes say. They are different questions and
  neither substitutes for the other.
- **It compares mock captures, not live ones.** A change that only manifests
  against the real CDN — a header the mock does not model, a body the mock
  cannot produce — is out of reach by construction. That is why the three
  band-3 commits that *do* change the wire were additionally checked against a
  live capture; see the report.
- **"Identical" means identical to this fixture.** A 58-file bundle with 10
  blobs exercises the same document surface as the 97-file reference, but it
  does not exercise scale, and a difference that only appears at 113 610 chunks
  would not show up here.
- **It sees the bundle, not the contract.** A field that was wrong in the same
  way at both ends is invisible to any before/after comparison. Fixing that
  needs a written expectation, which is what `d745680` and `43f66fa` added
  inside the crate.