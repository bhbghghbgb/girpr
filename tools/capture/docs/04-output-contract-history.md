# How `gircap`'s output contract changed over the refactor

What a reader of a `gircap` artifact bundle can expect today that they could not
expect at the start of the refactor, which commit changed it, and what it cost.

- **Scope:** `tools/capture/`, from `884eeea` ("remove client identity", the
  commit before the plan was added) to `e0c7f9a` ("stream the request log
  instead of building it whole"), the last commit to touch `tools/capture/src`.
  55 commits, plus the baseline.
- **In scope:** changes to what `gircap capture` **writes** — the bundle's
  documents, their fields, their values, and which files exist.
- **Out of scope:** wall time, memory, exit codes and stdout summaries. Those are
  recorded in §6 only where a caller reads a bundle to decide something.
- **Method:** [`05-bundle-differ.md`](05-bundle-differ.md). Instrument:
  [`../tools/README.md`](../tools/README.md).

---

## 1. The short answer

> **Two of the 55 commits changed the bundle. Both were fixes to a value that was
> wrong, not changes to the contract.**
>
> | commit | what moves |
> |---|---|
> | `7771323` — *the six api rows record where their body was written* | `stored_as`: `null` → `api/<file>.json` on 6 rows of `api/requests.ndjson` |
> | `cd0e876` — *the record claims the headers the client actually sends* | `req_headers.accept-encoding`: `gzip` → `identity` on **every** row |
>
> The other 53 — all of band 0, all of band 1, the whole 17-commit `capture()`
> split, the concurrency work, the streaming rewrite, and the whole §5.5
> hardening table — leave all 58 files byte-identical after normalisation.
> `SCHEMA_VERSION` stayed at `1` throughout, and so did every assertion id.
>
> **Across the entire refactor, 57 of 58 files are byte-identical. The single
> document that changed is `api/requests.ndjson`.**

Three further commits change what a bundle says **only when the capture
degraded**: a chunk the CDN mis-served (`54ff12d`), an origin that throttled
(`844f5c2`), a probe that got a `429` (`18de71f`). None can move a healthy
bundle, which is exactly why the crate's own gates could not see them. Each was
measured by making the mock produce the input, at the commit and at its parent;
they are in §5.

A fourth band-3 *attempt*, `87412d1` (streaming the manifest through the gzip
encoder), was re-measured down a path the default profile never takes and turns
out to change nothing at all — §4.

## 2. How the boundaries were found

The question "which commits changed the output" is not answerable by reading
commit messages — the plan *predicted* every band-3 item and *declared* the rest
output-identical, so a static read only restates the claim. It was measured.

Each commit is checked out in a worktree, built, and run twice against the
crate's own loopback mock, which serves a deterministic synthetic world. The two
captures are normalised — timestamps, `elapsed_ms`, the capture id, the `--out`
root, and the mock's ephemeral port replaced in place on the raw bytes — and then
compared file-set-first and byte-for-byte. Boundaries were located by divide and
conquer over the monotone predicate *"does F(mid) ≠ F(lo)"*, which proves one
half of an interval boundary-free per probe.

Three things make the "identical" verdicts mean something:

1. **The volatile set was discovered, not assumed.** The raw diff of two captures
   of one binary is exactly five documents, and they are exactly the documented
   cascade.
2. **Every commit is captured twice and the two must agree**, or its fingerprint
   is rejected. A normaliser with a hole cannot enter the search as a false
   boundary.
3. **The differ is proved by mutation.** Nine changes of the kinds this history
   actually made are injected into a known fingerprint; all nine are caught, with
   an unmutated copy as the negative control.

The mutation control is not ceremony. During this work the differ *did* report
**13 of 58 documents as changed between two commits that had changed nothing**,
because the mock's ephemeral port is recorded throughout a bundle — in
`MANIFEST.json → biz.hyp_base`, every `getBuild` body, every manifest meta
document, every patchmap URL, and every request row. Two captures of *one* binary
share a mock, so a self-check that only ever compares one binary to itself could
not find it. It would have invented a dozen boundaries.

The search was also wrong twice, in ways worth recording because both produced
*plausible* answers rather than an obvious failure:

- It popped one interval and pushed one back, which is a binary-search *chain*
  rather than a tree. It followed one branch, abandoned the other, and reported
  one boundary having examined half the range. Caught because two boundaries
  existed and one was missing.
- The fix seeded the frontier with `(0, n)` where `n = len(commits)`, so the
  leaf branch read `commits[n]` — out of range. It crashed *after* resolving most
  of the tree, leaving one interval unexamined, and the crash rather than the
  gap was what got noticed.

Both are the same shape as the differ's own false positive: an instrument that
reports a result rather than failing. The rule they produced is that a split
which finds a difference must search **both** halves, and only a split which
finds equality may discard one; and that an index bound belongs at `n - 1`
because that is the largest index the array has.

## 3. The two changes

### `7771323` — the six `api/` rows record where their body was written

| | |
|---|---|
| document | `api/requests.ndjson` — **the only file that changes** |
| field | `stored_as` |
| kind | **fix**: a field that was `null` where `docs/01-artifact-schema.md` said it would be populated |
| breaking | **no** — a field moves from `null` to a path that exists in the same bundle |
| effect | 6 rows gain ~24 bytes; the file grows 18 182 → 18 324 B |

Measured here, not taken from the commit message:

```
  8b12819 (parent)   api rows 6   stored_as=null: 6   marked: 0
  7771323            api rows 6   stored_as=null: 0   marked: 6
```

Chunk rows carried it throughout (10 of 10) and manifest rows did (4 of 4). Probe
rows (`kind: "probe"`, 8 of 8) stay `null` at both ends, which is correct — a
probe response is deliberately not stored, and that is what makes it a probe.

The cause was structural. `hyp::CapturedBody::seq` was hardcoded `0` with the
comment *"filled in by the caller"*, and `hyp::attach_seq` — the only thing that
could fill it in — had no production caller. The refactor plan had spotted this
as verified defect #2 and, in §2.1, listed `attach_seq` among the dead code to
delete, warning that deleting it would foreclose the fix. It survived, and it is
now called.

**Confirmed on the real bundles too.** All three reference bundles in
`artifacts/` were checked directly, with no build:

```
  20261016T002107Z   api 6  stored_as=null 6   marked 0      chunk 38 marked   manifest 4 marked
  20261017T000730Z   api 6  stored_as=null 6   marked 0      chunk 38 marked   manifest 4 marked
  20261017T123335Z   api 6  stored_as=null 0   marked 6      chunk 38 marked   manifest 4 marked
```

**Why it is not breaking.** A reader asking "do I have the body for request 3?"
previously had to go and look. `null` was never a documented value for a row
whose body *was* written, so nothing a reader could have relied on is removed or
reinterpreted.

**A near-miss worth recording**, because it is precisely the kind of bug a
before/after comparison reports as "the same change": `attach_seq` matched a body
to its row by endpoint name alone. `getBuild` is fetched twice, once per build,
and a third `getBuild` — the tag probe — also answers `200`. Both build bodies
were handed the *probe's* sequence number, the second `stored_as` overwrote the
first, and the bundle came out with **five of six** bodies marked and a probe row
marked instead of a build row. The fix matches on the tag as well.

### `cd0e876` — the record claims the headers the client actually sends

| | |
|---|---|
| document | `api/requests.ndjson` — **the only file that changes** |
| field | `req_headers.accept-encoding` |
| kind | **fix**: the log documented a request that was not the one sent |
| breaking | **no** to a reader; **yes on the wire** in the sense that matters, below |
| effect | every row's `req_headers`; the file grows 18 324 → 18 436 B |

```
  before   "accept": "*/*",  "accept-encoding": "gzip"
  after    "accept": "*/*",  "accept-encoding": "identity"
```

The client was built with neither header, and `reqwest` sends an
`accept-encoding` only when its `gzip` feature is on — which it is not. So every
row of every bundle ever captured made a false statement about its own request,
in the one field a reader would use the log for. §0 of the plan had recorded this
as verified defect #3 and `docs/01-artifact-schema.md` repeated the fiction.

**Why `identity` and not `gzip`.** The obvious fix — make the record match the
header the plan claimed was sent — would have been the wrong one. A real
`Accept-Encoding: gzip` to a CDN-fronted API can change the bytes that come back;
a client that cannot decompress would hand back compressed bytes, and every
manifest digest in the bundle would become a digest of compressed JSON. That is a
genuine layout change and the plan's contingency applies: drop the
`accept-encoding` half, keep `accept`. `identity` is what the wire does today and
is correct to send.

**Confirmed on the real bundles too**: all 56 rows of the two older reference
bundles say `gzip`; all 56 rows of `20261017T123335Z` say `identity`.

## 4. The large negative result

Fifty-three commits left all 58 files byte-identical. That is the substantive
finding, and it means something narrower than "the tool did not change":

| commits | what they are | bundle |
|---|---|---|
| 36 (`dd5159d` … `8b12819`) | band 0 (dead code, duplication, redundant passes), band 1 (algorithms, allocation, digests), the 17-commit `capture()` split, the concurrency work, `is_fatal` wiring, and `8b12819`'s summary reordering | **identical** |
| 17 (`54ff12d` … `e0c7f9a`) | `§6.6.2` streaming, the rest of the §5.5 table, `18de71f`, `49ea749`, `59a1c72`, `e0c7f9a` | **identical** |
| 2 | the two boundaries in §3 | changed |

Three groups inside that are worth naming, because "identical" is a stronger
claim than "usually identical":

- **The concurrency work** (`1e36112`, `93482c8`, `3654866`) replaced four serial
  request loops with `Fetcher::map_concurrently`. Results come back in *input*
  order rather than completion order specifically so a document cannot depend on
  which request won the race. The mock captures at `--jobs 1`; §7 is explicit
  about what that leaves open.
- **The 4 → 2 digest-pass collapse** (`63d84e7`) is the plan's single largest
  performance claim. It is identical here on 58 files; the plan's own measurement
  on the 129 MiB reference bundle is the stronger evidence for that one.
- **The streaming rewrite** (`12ef18a`, `35a8c4f`, `59a1c72`, `e0c7f9a`) moved
  blob fetches to a hashing `File`, decoding through a `BufReader` off the file
  rather than through a resident buffer. Every blob digest, every assembled
  `game/` and `expected/` file, and `chunks/index.json` come out identical —
  which is the only thing that mattered.

### The one band-3 attempt that turned out to change nothing

`87412d1` ("stream the manifest decode through the gzip encoder") was the only
item the plan filed as band 3 *conditionally*: "`manifests/*.json.gz` is recorded
in `checksums.json`, so if the compressed bytes move this is band 3".

It was invisible to the search, and not because nothing happened — because
nothing *could* happen. The default `--gzip-threshold` never gzips anything in a
mock world: the synthetic manifests are 1.0–3.7 kB against a 1 MiB threshold, so
the `.gz` write path is never taken. Re-measured with `--gzip-threshold 512`,
which produces four real `.gz` documents, at the commit and at its parent:

| granularity | result |
|---|---|
| decompressed JSON content | identical |
| the digests `MANIFEST.json` and `verify/checksums.json` record | identical |
| **the raw `.gz` bytes on disk** | **identical** |

So the commit is band 1 in fact as well as in claim, and the plan's contingency
did not have to be paid. That is a stronger statement than the commit message
makes, which rested on a unit test of the encoder; this is the end-to-end result
on a capture.

**And it holds on the real bundles, not only on the mock.** The differ was run
over the reference bundles with no build at all:

| comparison | identical | changed |
|---|---|---|
| `20261016T002107Z` vs `20261017T000730Z` — both **before** the band-3 work | 93 of 97 | 4 |
| `20261017T000730Z` vs `20261017T123335Z` — **across** it | 93 of 97 | 4 |

The second row is the finding of this document measured on real data: **every
chunk blob, every assembled `game/` and `expected/` file, every `patchmaps/`,
every `diff/`, every `manifests/` document is byte-identical across the band-3
work.** The only document carrying a contract change is
`api/requests.ndjson`.

The first row is also a result: two captures of the same world, days apart,
differ in exactly **four** documents. That is the volatility floor of a live
capture, and it matches the mock's five closely enough to be a check on the
normaliser rather than a coincidence.

| document | why it moves on a live capture |
|---|---|
| `MANIFEST.json` | `tool_argv[0]` is the binary's own path |
| `api/error_shapes.json` | the CDN's per-request `RequestId`, echoed in the recorded 404 body |
| `api/requests.ndjson` | `ts`, `elapsed_ms`, CDN response headers |
| `verify/checksums.json` | the digests of the three above |

`api/error_shapes.json` is the one a mock capture does **not** need to exempt —
its `RequestId` is a constant there — which is worth knowing: a live capture has
one more volatile document than a mock capture, and a gate that only ever sees
mock captures will not know that.

## 5. Changes that only appear when the capture misbehaves

Three commits change what a bundle says **for inputs a healthy capture never
produces**. A healthy mock world has no mis-served blob, no non-2xx on the
retryable path, and no probe that gets a `429`, so the search above cannot see
them — which is the correct result, and also why they need a separate
experiment.

Each was measured by arming the mock to produce the input, at the commit and at
its parent, so the difference is attributable.

### `54ff12d` — a mis-served chunk is the evidence, so it is kept

**What changes.** A chunk body that was fetched but is not a zstd frame used to
be *discarded*: `BlobProps::measure` refuses outright on "not a frame", the `?`
propagated, the bytes were never written, and the backfill recorded a row
reading **"fetched but not stored"** — a false statement about a fetch that had
succeeded. `blobs/mod.rs`'s own module doc calls a truncated response "the
evidence this tool exists to keep".

Measured, by serving one chunk bytes that are not a zstd frame, at the commit
and at its parent. The capture aborts at assembly in both cases — you cannot
assemble a file whose chunk will not decode — and the difference is in what
reached the disk:

| `chunks/index.json` row | before (`cd0e876`) | after (`54ff12d`) |
|---|---|---|
| `stored_path` | `null` | `chunks/blobs/059f9314….zst` |
| `bytes` | `0` | `24` |
| `md5_compressed` | `""` | `1f9cd8f1…` |
| `sha256_compressed` | `""` | `73c27bb9…` |
| `decode_error` | **"fetched but not stored"** — false | the real reason, naming the byte count, the four magic bytes it saw, and the declared compression |
| `verified` | `null` | `null` — "not measured", not four `false`s |
| the blob file | **absent** | **present** |

**Shape.** The row does not appear where there was none — it was already there,
carrying a lie. What changes is that it becomes truthful, and that the bytes a
reader would need in order to check it are now in the bundle. Row count is
unchanged at 10. **Additive in content; not breaking.** A reader iterating
`chunks/index.json` must already tolerate `decode_error` and a null
`stored_path`, because a row for a chunk that was never fetched still has them.

The neighbouring conflation went with it: a `None` slot has three causes that
cannot all be described as "fetched but not stored" — a source that refused, a
write that failed, and a task that panicked mid-measure. The last is not a
fetch failure at all.

### `844f5c2` — price the body of a failed request instead of reporting it as free

**What changes.** On a non-2xx the record was built with `bytes_in: 0` and the
response dropped without its body being read. A CDN `429` carrying a few KB of
envelope was transferred and counted as nothing — and `bytes_in` is sold in the
module's own doc as "the only way to price a manifest without re-downloading it".
Now the body is streamed and discarded, so the length is counted and the
connection returns to the pool.

**Why a healthy capture cannot tell.** The reference bundle's 56 rows are 53 ×
`200` plus **3 × `404`, all `kind: probe`** — and the probe path goes through
`observe`, which reads the body regardless of status, so those rows already
carried a length. `Fetcher::get`'s non-2xx branch is unreachable on a run that
never gets a non-2xx, which is every run in `artifacts/`.

**Measured**, by answering one chunk fetch with `503` and a 19-byte body, at the
commit and at its parent. 29 rows on both sides, same set:

| row | before (`726674e`) | after (`844f5c2`) |
|---|---|---|
| `seq 19`, `kind: chunk`, `status: 503` | `bytes_in: 0` | `bytes_in: 19` |
| the three probe 404s | 276 / 273 / 273 | 276 / 273 / 273 — unchanged |
| `Σ bytes_in` over all rows | 14 417 | 14 436 |
| `MANIFEST.json → totals.bytes_downloaded` | 14 417 | 14 436 |
| `README.md` "… B downloaded" | 14 417 | 14 436 |

55 of 58 files identical: every blob, every assembled `game/` and `expected/`
file, every `diff/`, `patchmaps/` and `manifests/` document is untouched. The
three probe rows not moving is the claim in the commit message — that the `404`s
already carried a length because they went through `observe` — confirmed rather
than repeated.

**Shape.** For a throttled run, `bytes_in` on the failing attempts goes from `0`
to the real length, and the two totals that sum it follow. **A value that was
wrong becomes right; not breaking.**

### `18de71f` — derive `ErrorShape::retryable` from the retry policy

**What changes.** `ErrorShape::retryable` in `api/error_shapes.json` was a
hardcoded `false` whose doc claimed it was "the retry policy stated in the
artifact". It stated nothing. It now calls `RetryPolicy::should_retry_status`.

**Why a healthy capture cannot tell.** Every probe answers `200` or `404`, and
neither is in the retry set, so the derived value and the constant agree on every
input the mock can produce. The pre-existing integration test asserted
`retryable == false`, which passes against the bug — so the bug had no gate at
all, on either side.

**Measured**, by answering one probe with `429`, at the commit and at its parent.
One file changes out of 58 — `api/error_shapes.json` — and one boolean in it:

| shape | status | before (`08175d9`) | after (`18de71f`) |
|---|---|---|---|
| `getbuild_bad_package` | 429 | `retryable: false` | **`retryable: true`** |
| `blob_missing` / `chunk_missing` / `blob_wrong_build` | 404 | `false` | `false` |
| `getbuild_unknown_tag` | 200 | `false` | `false` |
| `getgameconfigs_bad_game_id` / `getgamebranches_bad_game_id` | 200 | `false` | `false` |

The other six not moving is the whole point: `404` and `200` are genuinely not
retryable, so a constant and a correct derivation agree on them. Only a status
in the retry set tells them apart — which is exactly why no healthy capture
could, and why the existing test could not either.

**Shape.** For an origin that throttles a probe, one boolean starts agreeing
with the fetcher instead of being constant. **Not breaking**, and note for the
consumer contract: `girpr`'s `tests/artifact_bridge.rs` replays recorded shapes
but reads only `status`, content-type and `body` — it never reads `retryable`.

## 6. Cumulative effect, and the one change that is not in the bundle

| | `884eeea` | `e0c7f9a` |
|---|---|---|
| `api` rows with `stored_as` | 0 of 6 | **6 of 6** |
| `probe` rows with `stored_as` | 0 of 8 | 0 of 8 (correct: not stored) |
| `req_headers.accept-encoding` | `gzip` (false) | `identity` (true) |
| `SCHEMA_VERSION`, 24 documents | `1` | `1` |
| `tool.version` | `0.1.0` | `0.1.0` |
| assertion ids in `verify/selfcheck.json` | A1–A17 + A19 | A1–A17 + A19 |
| document set | 58 files | 58 files, same paths |
| every blob, `game/`, `expected/`, `diff/`, `patchmaps/`, `manifests/` | — | **byte-identical** |
| files identical | — | **57 of 58** |

One change is **not** inside a bundle but changes what a caller can conclude from
one, so it is recorded rather than filed under "internal":

`c89252c` wired `Selfcheck::is_fatal`, which was documented as deciding whether a
run ends and was called from nowhere. A bundle could previously say
`result: "fail"` — its own assertions did not hold — while the process exited `0`.
Now a failed fatal row ends the run with exit `3`. Two consequences:

- a capture that sets `--max-download-bytes` too low now exits `3` rather than
  `0` with `result: "fail"` (A17 has always said `over_budget` is not an
  acceptable matrix state; nothing consulted it);
- `8b12819` then made a failing run emit **two** summary records — the run's own
  (carrying `stage=`, `exit_code=0`) and the failure's (carrying
  `artifact_trustworthy=`, `exit_code=3`). They are told apart by shape, and the
  existing harness had to learn to select on that; a consumer taking the first
  `SUMMARY` line would read `exit_code=0` off a run that exited `3`.

Neither changes a byte inside a bundle; `c89252c` and `8b12819` are both in the
36-commit identical run.

## 7. What this does not establish

- **Mock captures, not live ones.** A change needing the real CDN — a header the
  mock does not model, a redirect, a content encoding — is out of reach. The two
  band-3 commits that touch the wire were additionally checked against live
  captures by their own authors; the rest were not, and did not need to be.
- **Scale.** Ten chunk blobs, a five-file fixture. A difference that only appears
  at 113 610 chunks would not show up here.
- **`--jobs > 1`.** Every probe pins `--jobs 1`, because `seq` is reserved at
  issue time and chunk-row *order* in the request log genuinely varies at higher
  concurrency. That is real and is not what this measures.
- **Non-default flags.** `--no-chunk-blobs`, `--audio`, `--per-class > 1`,
  `--max-download-bytes` and `--probe-tags` were not swept. A low
  `--gzip-threshold` was, for the one commit that needed it (§4).
- **Uniform drift is invisible to any before/after comparison.** If the output
  was wrong the same way at both ends, no differ can see it. That is what
  `d745680` and `43f66fa` addressed *inside* the crate by pinning the volatile
  documents; the two mechanisms are complementary and this report replaces
  neither.

## 8. Was anything breaking?

**`girpr`, the consumer.** `tests/artifact_bridge.rs` reads `MANIFEST.json`,
`patchmaps/<id>.json`, `game/`, `expected/`, and `api/error_shapes.json` — and of
that last one only `status`, content-type and `body`. It never reads `stored_as`,
`req_headers`, `bytes_in`, `attempts` or `retryable`. Every blob digest, every
assembled file, every patchmap total and every matrix row is byte-identical
across the whole refactor. The refactor's own author records running the suite by
hand against a bundle this branch produced and against one predating the
`stored_as` fix: 18 passed on both.

**A reader matching on `kind`.** `d815af6` considered splitting
`kind: "probe"` into two values so a reader could tell the retrying tag probe
from the non-retrying negative probes, and declined: `kind` is a closed
vocabulary, and widening it by subtraction silently breaks every reader matching
on `probe`. The ambiguity was documented instead. Recorded here because it is the
shape of change this refactor *declined* to make.

**So: nothing in this history is a breaking change to the bundle.** Every
artifact-visible change either fills in a field the schema already documented as
populated, corrects a recorded value that was false, or applies only to inputs a
well-formed capture does not produce. `SCHEMA_VERSION` correctly stayed at `1`.

## 9. What was spent

| | |
|---|---|
| cold build including all dependencies | 7 m 42 s, once |
| rebuild of `gircap` per commit, shared target dir | 3 m 07 s – 4 m 33 s |
| two captures + fingerprint + both diffs | ~2 s |
| **builds the final search actually needed** | **8** |
| builds spent on two abandoned search paths (§2) | 11 |
| builds for the degraded-input experiments | 8 (four before/after pairs) |
| **total** | **27 of 56** |
| a sweep would have been | 56 |

The eight are worth naming, because they are the whole search: `21517d1`,
`ca132a7`, `ef1e5cb`, `18de71f`, `8e88dd6`, `49ea749`, `54ff12d`, `87412d1` —
six midpoints and two leaves, for two boundaries among 56 commits. That is
O(k·log(n/k)) behaving as advertised.

The other eleven were spent before the search worked. The first two attempts
probed a different set of commits and built them correctly; they were thrown away
because the search itself was wrong (§2), not because the captures were. They
are counted here rather than quietly dropped, because "27 builds" is only
impressive next to the two bugs it took to get there.

The cheap first pass that decided where a build was worth spending is a static
screen — grep each commit's *added* lines for tokens that name a recorded field.
It is a filter, not an oracle: it fires on 45 of 55 commits and would have been
useless alone. The differ is what settled it, which is the argument for building
one.

Three of the four degraded-input pairs needed the mock patched to misbehave, and
that patcher was itself wrong four times before it was right (§6 of the differ
document). A good fraction of the wall time here went into the instrument rather
than the measurement, which is the honest cost of a method that has to be
trusted rather than believed.

## 10. Reproducing

See [`05-bundle-differ.md`](05-bundle-differ.md) §10 and
[`../tools/README.md`](../tools/README.md).

```powershell
pwsh -File tools/capture/tools/probe.ps1 -Commit 7771323 -Profile default
python tools/capture/tools/search.py
python tools/capture/tools/mutation_control.py
```

Commits are named by short hash plus subject throughout, because the history is
likely to be rebased and the hashes will go stale. `git log --format="%h %s"
884eeea..e0c7f9a -- tools/capture/src` reproduces the list.