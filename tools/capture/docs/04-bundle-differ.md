# The bundle differ — comparing bundles across commits

`gircap`'s own gate (`tests/bundle_diff.rs`) compares **two captures of one
binary against each other**. That catches a change that is *non-uniform* — a row
that flips depending on which worker woke first — and it is blind by
construction to a change applied **uniformly to every capture**. This is the
instrument for the cross-binary version of that question: it captures the same
fixture at two different commits and compares the results.

The tool is `tools/capture/tools/bundle_differ.py`. It has no dependencies beyond
the Python standard library, and it reads bundles; it never runs `gircap` itself
(the probe driver builds a commit and runs the binary). Read
[`../tools/README.md`](../tools/README.md) for how to run it.

Which gate to reach for is in
[`03-design-notes.md` §3](03-design-notes.md#3-the-gates-and-what-each-one-cannot-see).

---

## 1. What is compared

A bundle is captured twice from one commit, against the crate's own loopback
mock (`tests/common/mod.rs`, `World::all_shapes()`). Each capture is walked,
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
exactly four documents:

```
  api/requests.ndjson            ts, elapsed_ms
  MANIFEST.json                  argv's --out; its own digest row; bytes_on_disk
  README.md                      "N B on disk"
  verify/checksums.json          digests of the three above; total_bytes
```

which is the cascade, exactly as `tests/bundle_diff.rs` documents it. After
scrubbing those four, two captures of one binary are identical — which is the
check that the normalisation is not vacuous, and it is re-run for **every** commit
probed (see §5).

`verify/selfcheck.json` is **not** in the list. It used to be, blanking A16's
"N files, M B" totals — and that exemption was hiding a number rather than
removing one, because A16's byte total described the *previous* pass rather than
the tree it was written into. A16 no longer states a byte total, so nothing in
that document moves between two captures of one binary and it is compared in full.

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

Every other byte in those three documents is still compared, and every byte of
every other document. The crate's per-document pins catch drift *within* one
binary and this catches drift *across* commits; the two mechanisms are
complementary — neither replaces the other.

The exemption list **must match something**, or the differ fails rather than
quietly comparing nothing (`--strict`).

### A note on `encoding: gzip`

For a gzipped document, `files[].bytes` and its digests are the
**decompressed** size and digest, not the `.gz` bytes on disk. Measured on
`artifacts/hk4e_global/7.0.0__7.1.0/20261021T141336Z`:

```
  manifests/7.0.0/en-us.manifest.json.gz   on disk 1 356 632 B
                                           recorded 5 700 141 B   <- decompressed
  md5(gz bytes)  3c55a60901bc25e212fa7da96e086f16
  recorded md5   6b772797cfe10a764b4ba440952bfc57           <- of the payload
```

So the compressed bytes are **not** part of the recorded contract at all, and
comparing `.gz` payloads decompressed is the right granularity. If a change ever
does alter how the `.gz` is produced, it must be checked with a capture forced
down the `.gz` path — the default profile never takes it (§6).

## 4. Proving the differ works

A differ that scrubs too much reports "identical" for two different bundles,
and every conclusion drawn from it is then worthless. So the nine kinds of
change listed in `tools/mutation_control.py` are injected into a known
fingerprint, and the differ is required to catch each one:

```
  CAUGHT  stored_as filled in on an api row
  CAUGHT  a request header value changed
  CAUGHT  a chunk blob's bytes changed
  CAUGHT  an assertion's wording narrowed
  CAUGHT  a document added
  CAUGHT  a document removed
  CAUGHT  attempts=2 on a probe row
  CAUGHT  bytes_in set on a 404 row
  CAUGHT  paths.tsv rows reordered          (sort drift)
  OK     unmutated copy reads identical
```

The last line is the negative control, and it is the one that matters: a harness
that always fires proves nothing.

Each mutation asserts its own precondition first — that the fixture really
contains the string it is about to change — so a mutation cannot pass by quietly
doing nothing. That is not hypothetical: a wording mutation once asserted
against text this bundle does not contain, and failed on its own precondition
rather than silently "passing".

## 5. The per-commit guard

Every probed commit is captured **twice** and the two fingerprints must agree,
or the probe is rejected and no fingerprint is cached. That is what stops a
badly-normalised commit from entering the search as a false boundary — the
failure mode where one commit looks like it changed the output because its
`elapsed_ms` happened to be one digit wider.

## 6. Profiles, and what each one can and cannot see

One profile, `World::all_shapes()`, produces a **79-file** bundle covering
every document type the tool writes:

| | mock | reference (`20261021T141336Z`) |
|---|---|---|
| files | 79 | 78 |
| `api/` | 9 | 9 |
| `manifests/` | 16 | 16 |
| `diff/` | 5 | 5 |
| `patchmaps/` | 6 | 6 |
| chunk blobs | 22 | 25 |

It is small, deterministic, offline, and produces the whole document surface —
which is what makes a commit-by-commit search affordable. It is also a *healthy*
capture, and a healthy capture cannot see three things:

| not seen | why | how it is covered |
|---|---|---|
| the `.gz` write path | the synthetic manifests are 1.0–3.7 kB and the default threshold is 1 MiB, so nothing is ever gzipped | a second profile with `--gzip-threshold 512` |
| a mis-served chunk | every blob the mock serves is a real zstd frame | the mock patched to serve non-frame bytes |
| a non-2xx on the retryable path | the world answers 200, and the probes' 404s go through `observe`, which read the body regardless | the mock patched to answer 503/429 with a body |

The mock patch (`tools/capture/tools/mockpatch.py`) adds two builder methods and
one dispatch check in `serve`. It is anchored, so it refuses to half-apply if
the mock's shape has moved, and it is idempotent.

Two shapes have to be anchored, because the mock's `serve` has changed inside the
ranges this has been run over: one handled one request per connection and
returned, the other speaks keep-alive and loops. The anchors are matched as *line
prefixes* — deliberately omitting whatever follows — so a change further along
the line does not break them.

Two more details are not obvious, and each cost a run to find:

- **The needle is matched against the whole request target, path *and* query.**
  The negative probes share their *paths* with the metadata capture
  (`/sophon/getBuild`, `/hyp/getGameConfigs`), so a path-only needle cannot tell
  "the capture asked for this" from "the probe asked for this on purpose". A
  needle of `/chunk/` is not sufficient either: it also selects the 404 probes,
  which live under the same prefix — so it would measure `observe`, which already
  reads the body regardless, rather than the path under test.
- **Armed rules are re-armed between the probe's two captures.** Otherwise the
  second capture meets a mock that has already spent its one armed answer, the
  two captures legitimately differ, and the determinism guard rejects the commit
  for a reason that has nothing to do with the tool. The guard is right to
  reject; the instrument was wrong.

**The patcher was wrong four times before it was right**, and each failure had the
same shape: a textual edit that silently did not do what it looked like it did,
surfacing as a syntax error fifty lines from the cause — a block substituted
*in place of* its anchor instead of inserted before it; an anchor constant that
lost its closing bracket; a guard that compared a *stripped* line against an
*indented* anchor, so it could never fire; and an "insert after" splice that
*replaced* the anchor line, dropping the `{` that opened `impl Mock {`.

The patcher now requires each anchor to match exactly one line, requires exactly
one of the two `serve` variants to match, verifies the inserted block adds as many
`{` as `}`, and verifies every anchor survives the write. A check that cannot
fail is not a check, and that is the lesson §4's negative control teaches,
applied to the instrument rather than to the tool.

**It is applied only for the armed profiles, never for the ordinary search**, and
that is not tidiness. The arming probe names `arm_fail`/`arm_body`
unconditionally and Rust type-checks both match arms whatever the runtime
condition is, so a single probe binary cannot serve both cases; and keeping the
search path on the tree's own unmodified mock means every fingerprint was
produced by a mock that is byte-for-byte the one in the repository.

## 7. The search, and why it is not a sweep

Two captures of one binary tell you nothing about history. 56 adjacent
comparisons would. So a boundary is found by divide and conquer, using a
predicate that is monotone:

> P(lo, hi) — "some change lies in (lo, hi]" — is true **iff** F(mid) ≠ F(lo).

One probe of the midpoint splits the interval and proves one half boundary-free.
For *k* boundaries among *n* commits that is O(k·log(n/k)) builds instead of n−1,
and the results are cached, so a re-run costs nothing.

Two rules the search must follow, both learned the hard way:

- **A split that finds a difference must search both halves; only a split that
  finds equality may discard one.** A search that pops one interval and pushes
  one back is a binary-search *chain* rather than a tree: it follows one branch,
  abandons the other, and reports one boundary having examined half the range.
- **The index bound is `n - 1`.** Seeding the frontier at `(0, n)` makes the leaf
  branch read `commits[n]`, one past the end. That crashes *after* resolving most
  of the tree, leaving one interval unexamined — and it is the crash, not the
  gap, that gets noticed.

Both are the same shape as the differ's own false positive: an instrument that
reports a result rather than failing.

## 8. Validated against the real bundles, not only the mock

The mock is convenient and deterministic; it is also not the thing anybody
reads. So the same differ is run over the reference bundles in
`artifacts/hk4e_global/7.0.0__7.1.0/`, with no build at all:

| comparison | identical | changed |
|---|---|---|
| `20261016T002107Z` vs `20261017T000730Z` — two captures of the same world, days apart | 93 of 97 | 4 |
| `20261017T000730Z` vs `20261017T123335Z` — the older against the then-current reference | 93 of 97 | 4 |

Those two rows are a recorded result, not a recipe: the bundles they compared
have since been replaced, and the reference bundle under `artifacts/` is now
the one `docs/00-spec.md` §4.1 describes. The number to beat is the same one
either way — **four** documents move on a capture of the same world.

The first row is a result in its own right: two captures of the same world, from
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

The second row is the reusable finding: **every chunk blob, every assembled
`game/` and `expected/` file, every `patchmaps/`, every `diff/`, every
`manifests/` document is byte-identical across two captures of the same world on
different days.** That is the claim worth re-checking after any change to the
writer, and it is a cheap check — no build required.

## 9. Cost

| | |
|---|---|
| commits built for the search | 8 — six midpoints and two leaves, for two boundaries among 56 |
| commits built for the degraded-input pairs | 8, in four before/after pairs |
| what a sweep would have built | 56 |

That ratio is the whole argument for §7: O(k·log(n/k)) behaving as advertised,
rather than one build per adjacent pair.

Sharing one `CARGO_TARGET_DIR` across the worktrees is what makes a per-commit
build affordable at all: dependencies are compiled once and only `gircap` itself
is rebuilt. On a small machine two builds do not run concurrently anyway — cargo
takes a target-directory lock — so this is deliberately serial.

## 10. Reproducing it

```powershell
# one commit
pwsh -File tools/capture/tools/probe.ps1 -Commit <sha> -Profile default

# the whole history (caches; safe to re-run, safe to interrupt)
python tools/capture/tools/search.py

# prove the differ still works - run this after any edit to bundle_differ.py
python tools/capture/tools/mutation_control.py
```

The driver checks out each commit in a `git worktree`, copies in the probe, and
leaves the build cache alone. No packet leaves the host: the mock is the only
listener and binds `127.0.0.1:0`.

## 11. What this instrument does not establish

- **It is not the crate's own suite.** `cargo nextest run` is what proves the
  code is correct; this proves what the bytes say. They are different questions
  and neither substitutes for the other.
- **It compares mock captures, not live ones.** A change that only manifests
  against the real CDN — a header the mock does not model, a body the mock
  cannot produce — is out of reach by construction. A change to the wire needs a
  live check as well.
- **"Identical" means identical to this fixture.** A 58-file bundle with 10 blobs
  exercises the same document surface as the 97-file reference, but it does not
  exercise scale, and a difference that only appears at 113 610 chunks would not
  show up here.
- **`--jobs > 1` is not swept.** Every probe pins `--jobs 1`, because `seq` is
  reserved at issue time and chunk-row *order* in the request log genuinely varies
  at higher concurrency. That is real, and it is not what this measures.
- **Non-default flags are not swept.** `--no-chunk-blobs`, `--audio`,
  `--per-class > 1`, `--max-download-bytes` and `--probe-tags` are out of scope.
  A low `--gzip-threshold` is the one exception, for the case in §3.
- **It sees the bundle, not the contract.** A field that was wrong in the same
  way at both ends is invisible to any before/after comparison. Fixing that needs
  a written expectation, which is what the crate's own per-document pins are.
