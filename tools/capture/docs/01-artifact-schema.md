# Artifact schema — `gircap` bundle

Byte-level layout and every field of every machine-readable file.

**Every document below carries `"schema_version": 3`.** A reader should reject an
unknown `schema_version` rather than guess. JSON is UTF-8, LF, 2-space indent,
keys in the order written here. `*.ndjson` is newline-delimited, one object per
line, no enclosing array, so it streams.

**Required, not additive.** `verify` and `materialize` refuse a bundle that is
missing any document below. That is a deliberate choice: a bundle is a *fixture*
other work depends on, so a field that some bundles carry and others do not is a
trap for whoever reads it next. Where a field is genuinely optional within a
document, it is marked below and is optional *everywhere*.

**Compression rule.** A document is gzipped (`*.json.gz`) only when its
uncompressed body exceeds `--gzip-threshold` (default 1 MiB). The decision is
recorded in `MANIFEST.json → files[].encoding`, so a reader never has to guess.
In a real capture the full decoded manifests trip it; every gzipped document
ships a plain-text sidecar (`*.tsv`) so an agent can `grep` it without
decompressing.

The worked values throughout are illustrative, drawn from a reference capture of
a live version pair; a bundle's own documents are authoritative, and these
examples are not constants to copy.

---

## 1. Tree

```
artifacts/<biz>/<old_tag>__<new_tag>/<capture_id>/
├── MANIFEST.json
├── README.md
├── api/
│   ├── getGameConfigs.json
│   ├── getGameBranches.json
│   ├── getGameDeprecatedFileConfigs.json
│   ├── getGamePackages.json                  # legacy; captured as staleness evidence
│   ├── getBuild.<new_tag>.json
│   ├── getBuild.<old_tag>.json
│   ├── tag_probe.json
│   ├── error_shapes.json
│   ├── error_responses.json
│   ├── error_bodies/<seq:06>.body           # the retained bytes, one file per response
│   └── requests.ndjson
├── manifests/<tag>/
│   ├── <field>.manifest.zst                  # raw blob, byte-exact
│   ├── <field>.manifest.meta.json
│   ├── <field>.manifest.json.gz              # full decode (or .json if small)
│   └── <field>.paths.tsv                     # always plain; path \t size \t md5 \t nchunks \t is_folder
├── diff/
│   ├── paths.diff.json
│   ├── matrix.json                            # one row per matrix class
│   ├── paths.tsv                             # field \t path \t state \t old_size \t new_size \t reused \t moved \t patch_bytes \t total_cost
│   ├── candidates.json
│   └── reuse_pool.json
├── game/                                     # ★ the fixture
│   ├── config.ini
│   ├── GenshinImpact_Data/Persistent/audio_lang_14
│   └── <capture paths, OLD-version bytes>
├── expected/                                 # ★ ground truth
│   └── <capture paths, NEW-version bytes>
├── chunks/
│   ├── blobs/<chunk_id>.zst                  # one per distinct chunk id
│   └── index.json
├── patchmaps/
│   └── <capture_id>.json
├── diagnostics/
│   ├── rejections.json                       # refused manifest paths and chunk ids
│   └── path_map.json                         # every logical→on-disk escaped name
└── verify/
    ├── checksums.json
    └── selfcheck.json
```

`capture_id` is `<class>` or `<class>+<n>` (e.g. `reuse_in_place`,
`reuse_moved+2`). It appears in `patchmaps/` and in `MANIFEST.json`.

An API tag directory or capture path that names a Windows device —
`manifests/CON/`, `game/…/NUL` — is written under an escaped component (`%CON`,
`%NUL`); the documents keep the game's spelling and
`diagnostics/path_map.json` (§5c) records the pair. The directory that holds the
bundle, `artifacts/<biz>/<old>__<new>/`, is escaped as a single component for the
same reason; the tags it was built from stay in `MANIFEST.json → versions`. A
manifest field name or `audio_pkg_scan_dir` is not escaped at all: it is a value
the run can refuse, so a device there is refused rather than renamed, and a
device-named tag from a flag is refused for the same reason (§5c).
`MANIFEST.json → files[]` and `checksums.json` name the on-disk form, because
that is what exists.

Nothing published under `chunks/` or `manifests/` is ever deleted by the tool.
The `.tmp` file that a chunk fetch writes is not an exception: it is removed,
best-effort, on every error between creating it and renaming it into place. It
was never a blob, and the slot in its name means a later run would not
necessarily reuse the path.

**Atomicity is not uniform, and this is deliberate.** Chunk blobs are the only
artifacts written through a temporary file — `.<file name>.<slot>.tmp`, beside
the blob's final path — that is then renamed into place, and a blob's
`stored_path` points at a name that does not exist until the rename happens.
That step is what lets `verify` trust a blob: a crash mid-write would otherwise
leave a truncated file at the final path, which nothing could tell from a good
one. The metadata documents, the assembled `game/` and `expected/` files and
`manifests/*` are written with a plain create-and-write: each is small enough
that a torn write is a small, reproducible failure, and routing them through the
same dance would only hide *which* step failed. A reader must therefore not
assume any document in a bundle was renamed into position. An interrupted run
can leave a truncated metadata document; `verify/checksums.json`, written last,
is the completion proof.

---

## 2. `MANIFEST.json` — index

```jsonc
{
  "schema_version": 3,
  "tool":   { "name": "gircap", "version": "<tool version>" },
  "capture": {
    "id": "<capture id>",
    "captured_at": "<RFC 3339 instant>",
    "host_os": "windows",
    "tool_argv": ["target\\release\\gircap.exe", "capture", "--biz", "hk4e_global", …],
    "tool_argv_raw": [[116,0,97,0,…], …],   // exact host bytes per argument, see below
    "complete": true            // false only if interrupted; selfcheck then explains why
  },
  "biz":      { "id": "hk4e_global", "launcher_id": "VYTpXlbWo8", "game_id": "gopR6Cufr3",
                "channel": "1", "sub_channel": "0", "cps": "hyp_hoyoverse",
                "hyp_base": "…", "sophon_base": "…" },
  "versions": {
    "new": { "tag": "<new tag>", "build_id": "<new build id>", "source": "main.tag" },
    "old": { "tag": "<old tag>", "build_id": "<old build id>", "source": "diff_tags[0]" },
    "diff_tags": ["<tag>", …]
  },
  "field": "game",                       // matching_field the captures were drawn from
  "audio": ["en-us"],                    // codes written into game/…/audio_lang_14
  "gzip_threshold": 1048576,             // --gzip-threshold this run used
  "max_decompressed_bytes": 268435456,   // --max-decompressed-bytes this run used
  "selection_pool": {                    // matching_fields scored and diffed
    "fields": ["game", "en-us"],
    "manifest_entries": { "game": <count>, "en-us": <count> },
    "note": "--audio widens this pool; captures may come from any listed field"
  },
  "totals": {
    "files_captured": <count>,
    "chunks_blobs": <count>,
    "bytes_downloaded": <bytes>,
    "bytes_on_disk": <bytes>
  },
  "matrix": [                            // one row per class, in admission order
    { "class": "identical", "status": "captured", "capture_id": "identical",
      "path": "GenshinImpact_Data/Resources/browser_assets", "matching_field": "game",
      "candidates_considered": <count> },
    { "class": "reuse_moved", "status": "captured", "capture_id": "reuse_moved",
      "path": "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk",
      "matching_field": "game",
      "candidates_considered": <count> }
  ],
  "files": [                             // every file under the artifact root
    { "path": "game/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": <bytes>, "md5": "<md5>", "sha256": "…", "encoding": "identity" }
  ],
  "totals_by_set": { "materialize_old": <count>, "materialize_new": <count>, "patch_fetch": <count> }
}
```

`matrix[].status` ∈ `captured` | `covered_by` | `over_budget` | `unavailable`.
A class is never simply absent (assertion A17). All recorded states are acceptable
to A17: the assertion is about the matrix being *complete* — every class has a
row and every row says what happened to it — and `over_budget` ("a candidate
existed and this run's budget would not pay for it") is a complete answer. The
refusal is `--require-class`'s job. The row's optional keys
(`capture_id`, `path`, `matching_field`, `covered_by`, `also_satisfies`,
`reason`, `candidates_considered`) are omitted rather than written `null`.

**A bundle from an older schema is refused.** `gircap verify` reads
`MANIFEST.json` first, and a `schema_version` it does not know is an error rather
than a guess. Version 1 carried a `repair` block and an
`expectations.summary` block, and its class ids named a repair tool's internal
decisions; version 2's blob rows could not say why a fetch failed, whether a body
was partial, or why a measurement did not return. Version 3 makes those fields
required and adds the documents that keep the rest of what a run sees: retained
error bodies, the manifest paths and chunk ids a shape rule refused, the
logical→on-disk mapping for device names, and the exact argv bytes. A reader
that understood the current shape would ignore all of that and report a bundle
as consistent when it is not. The refusal names the command that produces a
current bundle.

`files[]` lists every file that existed **before the close-out began** — the
fixture, the blobs, the manifests, the diff, the patchmaps and `api/`. The four
documents the close-out itself writes are absent, because the list is taken
before they are written: `MANIFEST.json` (it is being written), `README.md` and
`verify/selfcheck.json` (both render the manifest), and `verify/checksums.json`
(its own). A document that hashes itself has no fixed point, and here the
dependency runs both ways.

`verify/checksums.json` is the complete inventory: `file_count` is one less than
the number of files on disk (it cannot list the close-out documents it renders),
and it covers every state.

`totals.bytes_on_disk` sums `files[].bytes`, so for a gzipped document it counts
the **decompressed** length — the same rule as §15. It is therefore larger than
what `du` reports, which counts compressed bytes on disk;
that is the correct answer, because it is the number a reader would need to hold
the document in memory.

`gzip_threshold` is **required**, like every other field in this document. A
`default` on read would let a bundle with no recorded threshold read back as
having used the flag's default — a claim about a run nobody wrote down, on the
one field whose purpose is to say why one document is gzipped and the next is
not. A field that defaults to a constant is worse than one that is required,
because the default is indistinguishable from the real value.

`max_decompressed_bytes` is required for the same reason. Every zstd decode in
the bundle — a manifest archive, a chunk blob, or a measurement — is bounded by
this ceiling, so a reader reproducing a decode needs the number the capture
actually used. Substituting the flag's default would turn a run with a raised
ceiling into a bundle that cannot be read back, and would do so silently.

`tool_argv_raw` is the exact argv the host carried: one array of byte values per
argument, in the same order as `tool_argv`. On Windows each byte is a UTF-16LE
code unit, so an unpaired surrogate survives; on Unix the bytes are the argument
as `execve` saw them. It is required because `tool_argv` is a `to_string_lossy`
rendering and U+FFFD cannot be reversed: a path that was not valid Unicode is
recorded truthfully by the raw form and only approximately by the readable one.
No replay command reads it yet; the point is that the evidence cannot be
reconstructed later.

---

## 3. `api/*.json`

Byte-exact response bodies, unmodified — not re-serialized. A reader must be
able to `diff` a captured file against a fresh request. `api/requests.ndjson`
records how they were obtained.

The two exceptions are generated, and both are this tool's own measurements
rather than the API's words: `tag_probe.json` (§5) and `error_shapes.json` (§5a).
Two more are generated for the same reason: `error_responses.json` (§5b) and its
byte-exact `error_bodies/` files, and the two `diagnostics/` documents (§5c).

## 4. `api/requests.ndjson`

One object per HTTP request, in `seq` order. The row counts by kind depend on the
capture; the kinds and their meaning are fixed.

```jsonc
{ "seq": 1, "ts": "<RFC 3339 instant>", "kind": "api",     // api|manifest|chunk|probe
  "method": "GET", "url": "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs?…",
  "range": null,                        // literal Range header sent, if any
  "status": 200, "final_url": "https://…", "redirected": false,
  "elapsed_ms": <ms>,
  "req_headers":  { "accept": "*/*", "accept-encoding": "identity",
                    "user-agent": "<the fixed User-Agent value>" },
  "resp_headers": { "connection": "keep-alive", "content-length": "<bytes>",
                    "content-type": "application/json",
                    "date": "<HTTP date>", "vary": "Accept-Encoding",
                    "via": "1.1 …cloudfront.net (CloudFront)",
                    "x-cache": "Miss from cloudfront", "x-amz-cf-pop": "<pop>", … },
  "bytes_in": <bytes>,
  "stored_as": "api/getGameConfigs.json",
  "attempts": 1 }
```

Header keys are lowercased; values are strings exactly as received. If one name
appears more than once in a response, its values are joined into a single string
with `\n` (a header value cannot contain a raw newline, so the delimiter is
unambiguous). `kind` lets a reader ask "what did the manifest fetch cost?"
without string-matching URLs.

`kind` is a closed set — `api`, `manifest`, `chunk`, `probe` — and two of the
probes' paths share `probe` while differing in retry behaviour. A `getBuild?tag=`
tag probe goes through the retry policy and records a real `attempts`; the
negative probes behind `api/error_shapes.json` are sent **once** and always record
`attempts: 1`. On a healthy capture every probe row shows `1`, so the row alone
does not say which path produced it; the URL does. The set is deliberately not
widened to distinguish them: `kind` is a published vocabulary and a reader
matching on `probe` must not be broken by it.

`req_headers` is what the tool **sent**, not what it wishes it had sent, and the
record and the client are built from one list so they cannot disagree.
`accept-encoding` is `identity` because that is the honest value: the HTTP client
has no decompression support enabled, and naming `gzip` on the wire would make the
CDN answer with a compressed body the tool could not decode — every manifest
digest in the bundle would become a digest of compressed JSON.

`bytes_in` is the number of body bytes the run actually transferred, **on every
row including the non-2xx ones**. It is the only way to price a request without
re-downloading it, so a throttled or errored attempt is counted like any other.
A non-2xx body refused on its declared length before any read counts `0` (the
status line and headers are not body bytes); one that crosses the ceiling
mid-read counts what arrived, including the chunk that crossed it.

`stored_as` is the path, relative to the artifact root, the body was written to,
or `null` when it was not kept. Per kind:

| kind | `stored_as` |
|---|---|
| `api` | `api/<endpoint>.json` — the body was written |
| `manifest` | `manifests/<tag>/<field>.manifest.zst` |
| `chunk` | `chunks/blobs/<chunk_id>.zst` |
| `probe` | `null` — a probe response is deliberately not stored, and that is what makes it a probe |

## 5. `api/tag_probe.json`

Which tags the API actually serves, with no blob downloads.

```jsonc
{ "schema_version": 3,
  "requested": ["<new tag>", "<old tag>", "<tag>"],
  "results": [
    { "tag": "<old tag>", "in_diff_tags": true, "http": 200, "retcode": 0,
      "build_id": "<build id>", "tag_echoed": "<old tag>", "served": true,
      "already_fetched": true },
    { "tag": "<unknown tag>", "in_diff_tags": false, "http": 200, "retcode": -202,
      "build_id": null, "tag_echoed": null, "served": false,
      "already_fetched": false }
  ],
  "summary": { "requested": <count>, "served": <count>, "not_in_diff_tags_but_served": <count> } }
```

`served` is the load-bearing fact: `diff_tags` is a patch-support list, not a
retention list (spec §3.4), so a capture may pin a pair the API still serves but
`diff_tags` no longer names.

`already_fetched` is provenance, not a verdict: it is `true` when this row's
answer was reused from a `getBuild` the run had already made for the captured
pair, and `false` when a dedicated probe was sent. A `true` row therefore has no
matching `kind: "probe"` entry in `api/requests.ndjson`, which is what makes the
two documents reconcilable.

---

## 5a. `api/error_shapes.json`

What the API does with a request it will **not** serve, with no blob downloads.
The error shapes are recorded on every capture.

The route families disagree about what failure looks like, and a consumer
that replays a bundle has to reproduce both. Without this document it would have
to invent them, and its claim that "the API 404s things" would be unfounded —
a bundle recorded only successes contains no not-found evidence at all.

```jsonc
{ "schema_version": 3,
  "shapes": [
    { "id": "chunk_missing", "family": "blob",
      "request": "a chunk id under the new build's prefix that the build does not have",
      "url": "https://autopatchhk.yuanshen.com/…/chunks/<series>/<build id>/000…",
      "status": 404, "content_type": "application/xml", "retcode": null,
      "error": true,
      "body": "<?xml version=\"1.0\" …><Error><Code>NoSuchKey</Code>…</Error>",
      "volatile_fields": ["RequestId", "HostId", "Key", "body length (follows Key)"],
      "retryable": false },
    { "id": "getgameconfigs_bad_game_id", "family": "json",
      "request": "getGameConfigs with a game_ids[] the API does not know",
      "url": "https://sg-hyp-api.hoyoverse.com/…/getGameConfigs?…",
      "status": 200, "content_type": "application/json", "retcode": -1001,
      "error": true,
      "body": "{\"data\":null,\"message\":\"invalid params\",\"retcode\":-1001}",
      "volatile_fields": [], "retryable": false },
    { "id": "getgamebranches_bad_game_id", "family": "json",
      "request": "getGameBranches with a game_ids[] the API does not know …",
      "status": 200, "content_type": "application/json", "retcode": 0,
      "error": false,
      "body": "{\"data\":{\"game_branches\":[]},\"message\":\"OK\",\"retcode\":0}",
      "volatile_fields": [], "retryable": false }
  ] }
```

| field | meaning |
|---|---|
| `id` | stable; **assert on this**, never on `url`, which carries a build id and changes every capture |
| `family` | `blob` (404) or `json` (200 + `retcode`) |
| `status` / `content_type` / `retcode` | exactly as received; `retcode` is `null` for a blob |
| `error` | whether the origin treats this as a failure — see below |
| `body` | byte-exact |
| `volatile_fields` | names that change per request, so the body **cannot** be replayed byte-for-byte |
| `retryable` | **derived from the tool's own retry policy**: "would the fetcher have retried this status?" |

The ids are `blob_missing`, `chunk_missing`, `blob_wrong_build`,
`getbuild_unknown_tag`, `getbuild_bad_package`, `getgameconfigs_bad_game_id`,
`getgamebranches_bad_game_id`.

`retryable` states the **policy**, not what this run did: the negative probes go
out through `Fetcher::observe`, which never retries, so a `true` here means "the
fetcher would have retried this" and not "this was retried". On a healthy
capture every row is `false`, because every recorded shape answers `200` or
`404` and neither is in the retry set — a `429` or `503` is what separates a
stated policy from a constant.

**`error: false` is not a bug.** `getGameBranches` answers an unknown game id
with `retcode 0` and an empty `game_branches`. That is a **success with nothing
in it**, not a failure, and it is recorded here precisely so a reader cannot
infer a failure from the shape of the other rows.

**Replaying.** `girpr` calls `.error_for_status()` and never reads the body, so
**`status` and `content_type` are the contract**; the body is fidelity, for a
human reading a replay server's log. For the blob rows the `volatile_fields`
mean a replayer must synthesize `RequestId`, `HostId` and `Key` — and therefore
the body length — and nothing else. The JSON rows have no volatile fields, so
they replay exactly.

**The document is required.** `verify` and `materialize` refuse a bundle without
it, because a bundle recorded only successes contains no not-found evidence at
all: a consumer that wanted to replay one would have to invent what a `404` looks
like, and its "the API 404s things" would be unfounded. Making the document
optional would have meant "this bundle can tell you how the API fails" is true of
some bundles and not others, with nothing in the bundle to say which.

It is read back through the same type that wrote it, so a document that is
present and unusable fails at open, by name, rather than at some later consumer.

---

## 5b. `api/error_responses.json` and `api/error_bodies/`

The **incidental** non-2xx responses the run saw — a throttled chunk, a missing
blob, a retried `503` — with the body bytes that were actually received. This is
the other half of §5a: that document records the *deliberate* negative probes,
this one records what the origin said when it refused something the run wanted.

```jsonc
{ "schema_version": 3,
  "budget_bytes": 8388608,                 // the run-wide retention budget
  "total_retained_bytes": <bytes>,
  "responses": [
    { "seq": <seq>,                        // this attempt's row in requests.ndjson
      "url": "https://autopatchhk.yuanshen.com/…/chunks/…/000…",
      "final_url": "https://autopatchhk.yuanshen.com/…",
      "status": 404, "attempts": 1,
      "content_type": "application/xml",
      "bytes_received": <bytes>,
      "retained_bytes": <bytes>,
      "truncated": false,
      "stored_path": "api/error_bodies/<seq:06>.body" }
  ] }
```

`api/error_bodies/<seq:06>.body` holds the retained bytes **verbatim**, with no
JSON escaping and no re-encoding: the file is `retained_bytes` long and is the
origin's body. `stored_path` is `null` only when zero bytes were retained.

Two budgets keep an error body from becoming the artifact:

- `--max-response-bytes` bounds **reading**, exactly as it does for a success.
- `budget_bytes` (8 MiB) bounds **retention** for the whole run. A throttled run
  can fail thousands of chunk requests, and keeping every `429` page would make
  the bundle larger than the data it exists to describe.

A response that crossed either bound is still a row, with `truncated: true` and
the bytes that were kept. `seq` is reserved per HTTP attempt, not per logical
fetch, and every non-2xx attempt gets its own row: `seq` joins that row to the
same attempt's row in `api/requests.ndjson`, and `attempts` is that attempt's
retry index — `1` for the first try, `2` for the second, and so on. A `503` that
is retried therefore appears as several rows, one per failed attempt, each with
its own `seq`, `attempts` and `stored_path`.

The document is required, and an empty `responses` list is a fact: this run met
no refusal. Deleting the document would make "this bundle recorded the origin's
failures" true of some bundles and not others, with nothing to say which.

---

## 5c. `diagnostics/`

Documents that keep what a capture refused or had to rename. Each is
written on every run, including a run that writes no other diagnostics: an empty
list says "nothing was refused/escaped", which is a measurement, not a gap.

### `diagnostics/rejections.json`

Every manifest entry path and chunk id a shape rule refused, with where it came
from and which rule it failed. The store is filled by `validate_entry_paths`
while a manifest is decoded: `source` names the place as
`manifest <tag>/<field> entry N file` for an entry's `file` path and
`manifest <tag>/<field> entry N chunk M id` for one of its chunk ids.
The write rules are strict; throwing the value away would also throw away the
most interesting fact a capture can learn about the API.

Other shape refusals — a flag, a flag-supplied tag or an audio field — are not
recorded. They abort before a bundle exists and their only trace is the error
text.

```jsonc
{ "schema_version": 3,
  "cap_bytes": 512,                        // per-value cap this run used
  "total": <count>,
  "omitted": <count>,                      // refusals not retained (cap or duplicate key)
  "rejections": [
    { "source": "manifest <tag>/<field> entry <n> file",
      "value": "GenshinImpact_Data/Resources/aux.dll",
      "rule": "path.device",
      "truncated": false }
  ] }
```

`rule` is a stable identifier — `path.device`, `path.windows-fold`,
`chunk-id.shape`, `chunk-id.empty`, … — and is the thing to assert on. `value` is
JSON-escaped and capped at `cap_bytes` (512); `truncated` says whether it was
cut. `value` is text, never joined to a filesystem path. The store keeps the
4096 smallest rows by `(source, value, rule)`; `omitted` counts refusals that
were not retained — rows past the cap and repeats of a key already kept — so a
hostile manifest cannot make the document unbounded, the sample is independent
of fetch order, and the loss is still visible.

A manifest refusal that stops the run before a bundle directory exists writes
this document into the would-be bundle directory: the refusal is evidence even
when there is no capture to attach it to.

### `diagnostics/path_map.json`

Every path written under a name other than the one the documents spell. A
Windows device component (`CON`, `NUL`, `COM1`, with or without an extension)
cannot be a file name, but a live manifest may legitimately contain one. The
value is **kept** in every document — `entries[].file`, `patchmaps/*.json`'s
`path`, the index's `referenced_by[].path` — and only the on-disk name is
escaped.

```jsonc
{ "schema_version": 3,
  "entries": [
    { "logical": "game/GenshinImpact_Data/Resources/CON",
      "on_disk": "game/GenshinImpact_Data/Resources/%CON" }
  ] }
```

The escape is one component at a time: a component whose stem is a reserved
device name gets a leading `%` (`CON` → `%CON`, `CON.txt` → `%CON.txt`), and
every literal `%` is doubled (`%CON` → `%%CON`), so a device name and an
ordinary name containing `%` cannot collide. Entries are sorted by `logical`, and
the list is empty when nothing needed escaping. `verify` re-derives the escape
and refuses a mapping that disagrees with it, so the document cannot redirect a
read.

`MANIFEST.json → files[]` and `verify/checksums.json` name the **on-disk** path,
because those are the files that exist; the mapping is how a reader gets back to
the logical path.

**The boundary of the keep-and-escape policy.** Escaping applies to values the
game or API supplied, and `path_map.json` covers every such bundle path that
changed name:

- **Escaped and recorded** — a manifest tag directory (`manifests/<tag>/`) and
  each `game/`/`expected/` path. A Windows device component gets a leading `%`;
  every literal `%` is doubled, so the device form and the literal form cannot
  collide; each logical→on-disk pair is listed here when the two differ. That is
  why the document is complete for renames.
- **Refused, not escaped** — a manifest field name (`game` or an audio code) and
  `audio_pkg_scan_dir` are checked by the non-device-allowing path rule and then
  written literally. A device name there aborts the capture with a `path.device`
  refusal; a literal `%` stays a single `%` and has no entry here, because
  nothing was renamed. The same is true of `game/<scan_dir>` and the field
  component of `manifests/<tag>/<field>.manifest…`.

Escaping exists to preserve evidence the run cannot refuse: the game's own
paths, the API's tag. A field name or scan dir is a naming/config value the run
can refuse safely, so it stays strict and every read site keeps using the
literal spelling. The same holds for a device-named tag from a flag; only a
device-named tag the API supplied is escaped instead.

---

## 6. `manifests/<tag>/<field>.manifest.meta.json`

Everything known about one archive blob.

```jsonc
{ "schema_version": 3,
  "tag": "<tag>", "build_id": "<build id>", "matching_field": "game",
  "category_id": "<id>", "category_name": "…",   // the API's own name for the category
  "manifest": { "id": "manifest_<prefix>_<suffix>",
                "checksum": "<checksum>",
                "compressed_size": <bytes>, "uncompressed_size": <bytes> },
  "declared": {                                  // fields the API sends that most clients drop
    "stats": { "file_count": <count>, "chunk_count": <count>,
               "compressed_size": <bytes>, "uncompressed_size": <bytes> },
    "deduplicated_stats": { "file_count": <count>, "chunk_count": <count>,
                            "compressed_size": <bytes>,
                            "uncompressed_size": <bytes> },
    "chunk_download":   { "url_prefix": "…", "url_suffix": "", "compression": 1,
                          "encryption": 0, "password": "" },
    "manifest_download":{ "url_prefix": "…", "url_suffix": "", "compression": 1,
                          "encryption": 0, "password": "" } },
  "id_parts": { "prefix_16hex": "<prefix>", "prefix_semantics": "opaque",
                "suffix_32hex": "<checksum>",
                "suffix_equals_checksum": true },
  "stored": {
    "path": "manifests/<tag>/<field>.manifest.zst",
    "bytes": <bytes>, "container": "zstd",
    "md5_compressed": "<md5>",
    "sha256_compressed": "…",
    "md5_decompressed": "<checksum>",
    "bytes_decompressed": <bytes>, "encoding": "identity"
  },
  "verified": { "decompressed_md5_matches_checksum": true,
                "checksum_matches_id_suffix": true,
                "id_prefix_verified": false },
  "decoded": { "entries": <count>, "files": <count>, "folders": 0,
               "total_chunks": <count>, "max_chunk_compressed_bytes": <bytes>,
               "total_file_bytes": <bytes>,
               "structure": { "duplicate_paths": <count>, "empty_paths": <count>,
                              "backslash_paths": <count>, "non_ascii_paths": <count>,
                              "unsorted_chunk_offsets": <count>, "non_gapless_files": <count>,
                              "size_mismatch_files": <count> } },
  "full_decode_written": true
}
```

`verified.id_prefix_verified` is **always false** and is expected to be. The
16-hex prefix is not a hash of any observable input (spec §3.7); the field exists
so the artifact states that finding rather than implying the prefix went
unchecked.

`decoded.structure` counts the structural properties of spec §3.6. They are
recorded, never fatal: a violation still produces a capture, and A11 goes red on
the file that broke.

`deduplicated_stats` is the API's own second, de-duplicated view of the same
numbers, kept because it is the field a client would otherwise drop; `gircap`
asserts nothing against it.

## 7. `manifests/<tag>/<field>.manifest.json.gz` — full decode

The whole protobuf, verbatim, so no consumer has to re-download to inspect it.
`full_decode_written` in `meta.json` says whether this document was produced
(`false` under `--manifest-detail paths-only`).

```jsonc
{
  "schema_version": 3, "tag": "<tag>", "build_id": "<build id>",
  "matching_field": "game", "entries": [
    { "file": "GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "is_folder": false, "size": <bytes>, "md5": "<md5>",
      "chunks": [
        { "id": "<chunk id>",
          "uncompressed_md5": "<md5>", "offset": 0,
          "compressed_size": <bytes>, "uncompressed_size": <bytes>,
          "unknown_field6": <opaque>,                      // preserved verbatim
          "compressed_md5": "<md5>" } ]
    }
  ]
}
```

The document is written **streamed through the gzip encoder**, not built as one
string and compressed afterwards, because the full `game` decode is the largest
thing the tool holds. The compressed bytes are identical either way, which is
why `verify/checksums.json` records the *decompressed* digest and a `.gz`
document's identity does not depend on how it was produced.

`unknown_field6` is the wire's tag 6. Its meaning is unknown; it is preserved
because the upstream schema carries it and dropping it would make this dump a
lossy transform. It is preserved across builds for the same
logical chunk even when content differs, which is exactly why it must not be
guessed at.

## 8. `manifests/<tag>/<field>.paths.tsv`

Always plain text, tab-separated, header line, `\n` endings:

```
path	size	md5	nchunks	is_folder
GenshinImpact.exe	<size>	<md5>	<count>	false
```

This is the sidecar that makes the gzipped JSON greppable without decompression.

---

## 9. `diff/paths.diff.json`

```jsonc
{ "schema_version": 3,
  "old_tag": "<old tag>", "new_tag": "<new tag>",
  "fields": ["game", "en-us"],
  "counts": { "common": <count>, "identical": <count>, "changed": <count>,
              "added": <count>, "removed": <count>, "folders": 0 },
  "byte_counts": { "added": <bytes>, "removed": <bytes>, "patch": <bytes> },
  "added":   ["game:GenshinImpact_Data/Plugins/libHttpClient.GDK.dll", …],
  "removed": ["game:GenshinImpact_Data/…/00664172.blk", …],
  "by_field": { "game": { "common": <count>, "identical": <count>, "changed": <count>,
                          "added": <count>, "removed": <count> },
                "<audio field>": { "common": <count>, "identical": <count>, "changed": <count>,
                           "added": <count>, "removed": <count> } },
  "new_chunks_reusable_at_all": <count> }
```

Counts at the top level are aggregated over `fields`; `by_field` is the
breakdown, and every `added`/`removed` entry is prefixed with its field so a path
from two pools cannot collide. The `game` row is measured for the captured pair;
the audio rows depend on which languages `--audio` selected.

`new_chunks_reusable_at_all` is how many of the new manifest's chunks have an MD5
present **anywhere** in the old manifest — the number the same-file-only reuse
simplification in §10 of the spec gives up (see `reuse_pool.json`).

`diff/paths.tsv` is the flat per-path form and leads with the `field`, because the
same relative path can appear in more than one pool:

```
field	path	state	old_size	new_size	reused	moved	patch_bytes	total_cost
game	GenshinImpact.exe	changed	<bytes>	<bytes>	<count>	<count>	<bytes>	<bytes>
```

## 10. `diff/candidates.json`

The scoring table, with a row for **every** common path, not just the winners.
Lets a later run ask "why did it pick that?"

```jsonc
{ "schema_version": 3,
  "columns": ["path","matching_field","old_size","new_size","nchunks","reused","moved",
              "moved_later","moved_earlier","old_max_end","same_md5","old_bytes","new_bytes",
              "patch_bytes","total_cost","classes","rank","admission_rank"],
  "rows": [
    { "path": "…/HoYoChannel.dll", "matching_field": "game",
      "old_size": <bytes>, "new_size": <bytes>,
      "nchunks": <count>, "reused": <count>, "moved": <count>, "moved_later": <count>, "moved_earlier": <count>,
      "old_max_end": <bytes>, "same_md5": false,
      "old_bytes": <bytes>, "new_bytes": <bytes>, "patch_bytes": <bytes>,
      "total_cost": <bytes>, "classes": ["reuse_in_place"], "rank": <rank>,
      "admission_rank": <rank> }
  ] }
```

Column-oriented so a reader can sort without re-parsing objects.

`matching_field` names the pool the path was scored from (`game`, or an audio
code). It is a row column because the same relative path can appear in more than
one pool, and without it a reader summing `total_cost` across rows would double
count. Two 1-based ranks are published, because they answer different questions
and the picker's order is not the scorer's:

- `rank` is the **scoring rank**: the row's position in `total_cost` order, with
  the tie-break of field (`game` first), then `moved` descending, then `reused`
  descending, then `path`. Rows are listed in this order.
- `admission_rank` is the **selection rank**: where this path sits in the pool
  the picker actually walks. That pool adds the one-sided `added`/`removed` rows,
  charged their compressed download bytes and old size respectively, and breaks
  a `total_cost` tie by field and then `path` — it does not consult `moved` or
  `reused`. So for equal `total_cost` the
  two ranks can disagree, and `admission_rank` is the one that answers "why was
  this picked?". A path that `by_cost` scores but the pool excludes keeps its
  scoring `rank` and carries a `null` `admission_rank`.

One-sided paths (`added`/`removed`) are not rows in this table: they have no
patch cost to score. A folder entry is likewise not a row — it is a directory,
not a file — and so is never a capture even if it is scored. `admission_rank`
still reflects a path's position among the full pool, including one-sided paths,
so it remains meaningful even though those rows are absent here.

`moved_later` and `moved_earlier` split `moved` by **where the bytes land**:
`moved_later` counts the chunks whose source is *before* their destination, so
the chunk lands later in the file than the offset it came from, and
`moved_earlier` counts the ones whose source is *after* it. `moved` is their
sum. The split is what makes the shape of the move a *measurement* rather than
something a reader has to re-derive — and it is a measurement about the patch,
not about any repair tool's write order.

`old_max_end` is the largest end (`source offset + size`) over the file's reuse
sources: **how far into the old file the reusable bytes reach**. It is a fact
about the patch, and what a repair tool does with a source that reaches past
the end is that tool's own business.

## 11. `diff/reuse_pool.json`

Quantifies same-file-only reuse. Recorded because it bounds a deliberate
simplification in the main tool; `gircap` never acts on it.

```jsonc
{ "schema_version": 3,
  "new_chunks_total": <count>,
  "new_chunks_md5_present_in_old_manifest": <count>,
  "files_with_a_cross_file_only_chunk": <count>,
  "cross_file_only_chunks": <count>,
  "cross_file_only_bytes_forgone": <bytes>,
  "note": "same-file-only reuse is a deliberate simplification; this quantifies its cost" }
```

---

## 12. `chunks/index.json` and `chunks/blobs/`

One blob per distinct `chunk_id`, byte-exact (zstd as served), stored once even
when several files or different builds reference it.

```jsonc
{ "schema_version": 3,
  "blobs": [
    { "chunk_id": "<chunk id>",
      "stored_path": "chunks/blobs/<chunk id>.zst",
      "bytes": <bytes>, "container": "zstd",
      "md5_compressed": "<md5>",
      "md5_decompressed": "<md5>",
      "sha256_compressed": "<sha256>",
      "uncompressed_size": <bytes>,
      "unknown_field6": <opaque>,
      "verified": { "raw_size_matches": true, "md5_compressed_matches": true,
                    "md5_decompressed_matches": true, "decoded_size_matches": true },
      "decode_error": null,
      "fetch_error": null,
      "partial": false,
      "measurement_error": null,
      "referenced_by": [
        { "tag": "<tag>", "build_id": "<build id>",
          "capture_id": "<capture id>",
          "path": "<path>", "role": "materialize_old", "offset": <offset> }
      ],
      "sets": ["materialize_old"] }
  ],
  "sets": {
    "materialize_old": [ … chunk ids … ], "materialize_new": [ … ],
    "patch_fetch":     [ … ]
  } }
```

`role` ∈ `materialize_old` | `materialize_new` | `patch_fetch`. `patch_fetch` is
always a subset of `materialize_new` for the same file (assertion A12).

### `verified` — the keys

`verified` carries the A6 checks, and the names are precise:

| key | claim |
|---|---|
| `raw_size_matches` | `len(raw) == compressed_size` — the blob arrived whole |
| `md5_compressed_matches` | `md5(raw) == compressed_md5` — it arrived *intact*, not merely complete |
| `md5_decompressed_matches` | `md5(zstd(raw)) == uncompressed_md5` — it is the chunk the manifest named |
| `decoded_size_matches` | `len(decoded) == uncompressed_size` |

There is **no `size_matches` key**; the size claims are named separately
because they are separate facts about different byte counts (the frame's length
as served, and the length after decoding).

### `decode_error` — a fetched blob is evidence, not a failure

`decode_error` is `null` whenever the blob measured cleanly, and otherwise
carries the real reason: the byte count, the first magic bytes actually seen, and
the compression the build declared. **The blob is still written and still listed.**
A truncated or mis-served response is the evidence this tool exists to keep, so it
is recorded with its bytes and a truthful `decode_error` rather than discarded and
backfilled with a "the fetch failed" story — with the two decode-dependent
`verified` keys `false` and the two that need no decode still decided. Because the
fetch itself succeeded, the store does not ask a second source for that chunk: the
bytes that arrived are not replaced, and the row's `stored_path` points at them.
A successful `2xx` that carries no bytes at all follows the same rule: the empty
file is published and measured, and the row records `bytes: 0` with a
`decode_error` (`not a zstd frame (0 bytes, magic [])`) rather than a
`fetch_error`.

It is `Option<String>`, so a reader must treat it as optional: a blob that was
never fetched at all has a null `stored_path`, a null-ish row body and no
`verified` verdict.

### `fetch_error`, `partial` and `measurement_error` — why a row is unusable

Version 3 added fields so that "this blob cannot be used" always has a
machine-readable cause and is never inferred from a null path alone.

`fetch_error` is an object — `source`, `url`, `status`, `transport`, `attempts`,
`detail` — and is non-null when a fetch did not complete. `source` is the tag that
was tried, `status` the HTTP status when there was one, `transport` the error
kind when the failure was not an HTTP answer, and `detail` the same text the run's
error list carries. `decode_error` stays reserved for a body that **arrived
whole and would not decode**: conflating the two made "the bytes are here and are
wrong" indistinguishable from "the bytes are missing".

`partial` is `true` when a fetch stopped after writing some bytes but before the
body completed. Those bytes are published at `stored_path` and indexed, because a
prefix is evidence; only a **failed** fetch's zero-byte temp file is removed. A
successful `2xx` with an empty body is not a failed fetch: the empty file is
renamed into place and measured, so its row carries `stored_path`, `bytes: 0`,
`partial: false`, no `fetch_error` and a `decode_error` saying there was no
frame. `is_usable` — and therefore A6 and assembly — requires `partial == false`,
so a partial row is kept and never assembled from.

**A partial body does not end the source loop.** The store parks the prefix as a
candidate and tries the remaining sources, newest first. If any later source
completes, the **complete body wins regardless of byte count**: it replaces the
prefix, is published at `chunks/blobs/<id>.zst`, and is the only attempt whose
`stored_as` names that path. If no source completes, the **longest** candidate
(largest `bytes`) is published as the `partial` row with that attempt's
`fetch_error`; a strictly larger later prefix replaces it, while an equal or
shorter one is discarded, so a tie keeps the newest source. Losing prefixes never
occupy the final path and are deleted, and `stored_path` points only at the
winner. A fetch failure that writes **no** bytes still falls through to the next
source, and if nothing was produced the last error is what the row and the run's
error list carry. A body that arrived **whole** but would not decode is not a
partial and is still not retried.

`measurement_error` is set when the post-rename measurement did not return — an
I/O error, a panic or cancellation. The blob is kept and `stored_path` stays set,
so this is distinguishable from "no bytes arrived"; the row is unusable until a
measurement succeeds.

A `stored_path` that is `null` has these causes:

- **A fetch was attempted and failed without producing bytes** — a source that
  refused, a transport failure, a write that failed, or a fetch task that
  panicked. The row records the structured `fetch_error`, `partial` is `false`,
  and the chunk is counted once among the failed fetches. A *successful* response
  with an empty body is **not** this case: the empty file is published and
  measured, so its row has a `stored_path` and a `decode_error` instead.
- **A write or rename failed after the body was fetched** — also a
  `fetch_error`, with `detail` naming the local path and `transport` null. The
  temp file is removed: there are no trustworthy digests and no complete path to
  publish.
- **No fetch was attempted** — a `--no-chunk-blobs` row. It means "not
  measured", not "failed": `stored_path`, `verified`, `decode_error` and
  `fetch_error` are `null`, `partial` is `false`, `bytes` is `0`, and the two
  compressed digests (`md5_compressed`, `sha256_compressed`) are `""`.
  `md5_decompressed` and `uncompressed_size` are **not** empty: they carry the
  manifest's declared digest and length, so a reader can tell which bytes are
  missing. They are claims, not measurements — `verified` is `null` precisely
  because nothing was measured. The flag also skips A6–A12, A19 and A21–A23
  (see §16).

### Negative and extreme declared sizes

A manifest is the API's document, not a trusted one. A file entry can declare
`size: -1`, two chunk sizes can sum past `i64::MAX`, and a file's declaration can
disagree with its chunks. The decode keeps what can be true and records the
saturation rather than wrapping or aborting:

- a negative per-file size contributes `0` to `total_file_bytes`;
- a negative per-chunk size contributes `0` to the chunk and file byte sums;
- offsets and byte totals use saturating arithmetic, so an overflowing sum stops
  at the type's maximum rather than wrapping to a small positive number.

The individual entry still appears with its declared values in
`manifests/<tag>/<field>.manifest.json.gz`; the counters in
`meta.json → decoded` and the index are the clamped view. A reader comparing a
negative declaration with a zero contribution is seeing the rule working, not a
lossy dump.

### `build_id` — part of the chunk's address

A chunk's URL is `…/chunks/<series>/<build_id>/<16hex>_<32hex>`, so the build id
is **where the bytes live**, not a label on them. The same chunk id under the
other build's prefix answers `404` with an `application/xml` `NoSuchKey` body
(measured against `autopatchhk.yuanshen.com`, `Server: AliyunOSS`) — see
[`00-spec.md` §3.7](00-spec.md). `tag` does not determine `build_id`: different tags can
map to different build ids over time, so a consumer that has to build a chunk URL,
or decide whether an observed request was namespaced correctly, cannot derive it
from the tag.

`referenced_by` is the **only** thing `gircap materialize` consults: the chunk
list of a side is rebuilt from `referenced_by`'s `offset`s and each blob's
`uncompressed_size`, and the retained manifests are not read at all. That is
deliberate — a rebuild that consulted the manifests would only prove they are
still readable, whereas rebuilding from the closure set proves the thing the
bundle actually claims, which is that the blobs alone are enough. A bundle whose
manifests survived but whose index is incomplete fails here, and should.

---

## 13. `patchmaps/<capture_id>.json` — the core

Per target chunk: where its bytes come from.

```jsonc
{ "schema_version": 3,
  "capture_id": "reuse_in_place",
  "path": "GenshinImpact_Data/Plugins/HoYoChannel.dll",
  "matching_field": "game",
  "classes": ["reuse_in_place"],
  "old": { "tag": "<old tag>", "size": <bytes>, "md5": "<md5>", "nchunks": <count> },
  "new": { "tag": "<new tag>", "size": <bytes>, "md5": "<md5>", "nchunks": <count> },
  "plan": [
    { "i": 0,
      "decision": "FETCH",
      "target_range":  { "start": <offset>, "end": <offset> },
      "source_range":  null,
      "chunk_id":      "<chunk id>",
      "url":           "https://…/chunks/…/<build id>/<chunk id>…",
      "compressed_size": <bytes>, "uncompressed_size": <bytes>,
      "uncompressed_md5": "<md5>", "note": null },
    { "i": 1,
      "decision": "REUSE",
      "target_range":  { "start": <offset>, "end": <offset> },
      "source_range":  { "start": <offset>, "end": <offset>, "file_tag": "<old tag>" },
      "chunk_id":      "<chunk id>",
      "url":           null,
      "compressed_size": <bytes>, "uncompressed_size": <bytes>,
      "uncompressed_md5": "<md5>",
      "note": "byte-identical to the old chunk at the same offset" },
    { "i": 2,
      "decision": "FETCH",
      "target_range":  { "start": <offset>, "end": <offset> },
      "source_range":  null,
      "chunk_id":      "<chunk id>",
      "url":           "https://…/chunks/…/<build id>/<chunk id>…",
      "compressed_size": <bytes>, "uncompressed_size": <bytes>,
      "uncompressed_md5": "<md5>", "note": null }
  ],
  "totals": { "chunks": <count>, "reuse": <count>, "fetch": <count>, "moved": <count>,
              "download_bytes": <bytes>,
              "materialize_old_bytes": <bytes>, "materialize_new_bytes": <bytes> },
  "expectations": {
    "result_path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
    "result_md5": "<md5>",
    "absent_after": false,
    "download_bytes": <bytes>,
    "must_not_fetch": ["<chunk id>"]
  }
}
```

`expectations.must_not_fetch` is the negative assertion: a run that downloads
that chunk has a broken reuse path, and it is nameable from the artifact alone.
It is the **same set** as the plan's `REUSE` entries, stated as the ids a
consumer must not see requested, and `verify` checks the two against each
other so they cannot drift apart.

**What the patchmap does not carry.** It says what the **patch** is — the
per-chunk decision, the arithmetic, the reference bytes and the reference
total — and nothing about how a repair tool gets there. The tier it picks, the
write order it uses, whether it releases the dead tail early and what its
`SUMMARY` line says are that tool's own decisions; they are not shared with any
other repair tool, and they are tested on crafted fixtures against the tool
itself.

`source_range` is the local-file byte range the main tool would read and MD5-verify
before copying. `moved` in `totals` counts entries where `source_range.start !=
target_range.start`.

A REUSE carries `url: null` — the bytes are already local, so there is nothing to
fetch — and a FETCH carries `note: null` for the same reason: each field is
present and says nothing only where it genuinely has nothing to say.
`materialize_old_bytes` and `materialize_new_bytes` are the **whole-file** sizes
each side assembles to, not the compressed chunk sums; `download_bytes` is the
only figure here that is priced per chunk.

A REUSE is decided *after* the slice has been verified by MD5, so a failed REUSE
becomes a FETCH. That downgrade is recorded on the entry (`note`) and in the
outcome (`rewrites`): a silent downgrade is indistinguishable from one that was
never a candidate, and the two demand opposite debugging.

### 13.1 The one-sided classes

`old` and `new` are the two sides; `plan`, `totals` and `expectations` describe
producing the **new** one. That makes the one-sided cases asymmetric, and both are
spelled out here because getting them wrong makes the artifact lie.

**`added`** — no `old`, but `plan`, `totals` and `expectations` are all present.
Every `plan` entry is `FETCH` with `source_range: null` and a `note` saying why,
because there is nothing to slice from. `totals` reads `reuse: 0`,
`fetch: nchunks`, `moved: 0`, `materialize_old_bytes: 0`, and `download_bytes`
is the whole file's compressed size — the file has to be fetched in full. The
writer **refuses** to emit a capture that has a new side and no plan.

**`removed`** — no `new`, and therefore no `plan` and no `totals`: there is no
target to plan for and nothing to cost. `expectations` **is** present, in its own
shape:

```jsonc
{ "capture_id": "removed",
  "old": { "tag": "<old tag>", "size": <bytes>, "md5": "<md5>", "nchunks": <count> },
  "new": null, "plan": null, "totals": null,
  "expectations": {
    "result_path": null,
    "result_md5":   null,
    "absent_after": true,
    "download_bytes": 0,
    "must_not_fetch": []
  } }
```

`download_bytes: 0` here is a **true** value, not a default: a removal downloads
nothing. It is written down rather than left `null` precisely because the number
is known and a consumer should be able to compare against it.

### 13.2 `download_bytes` is always a number

`expectations.download_bytes` is the sum of `compressed_size` over the plan's
`FETCH` entries — what a correct patch costs. It is **never** null and never
defaulted. This is stated as a rule because getting it wrong is not a cosmetic
bug: `null` where a figure exists reads as *unrecorded* and `0` reads as a claim.
An `added` capture whose file has a real download cost, written as
`plan: null` / `totals: null` / `download_bytes: 0`, makes a consumer that
asserts `SUMMARY download_bytes == expectations.download_bytes` compare that real
cost against 0 and call a perfectly correct run a failure. Assertion A20 exists to stop
the rule being quietly dropped.

Absent data has to be absent. Where a figure genuinely does not exist, the field
that would hold it is `null` — which is why `result_path` and `result_md5` can be
null while `download_bytes` cannot.

`expectations` is required on every capture. A patchmap a consumer cannot assert
anything about is the defect, not a legitimate shape; that is what makes `removed`
testable rather than merely recorded.

---

## 14. `game/` and `expected/`

Plain bytes at their real relative paths — no wrappers, no metadata. `game/`
contains old-version files only; `expected/` contains new-version files only.
`game/` additionally holds the tool-read config: `config.ini` and the audio scan
file at `getGameConfigs.audio_pkg_scan_dir`.

Both trees are covered by `MANIFEST.json → files[]` and
`verify/checksums.json`. A consumer can `diff -r` `game/` against `expected/`
and every differing file is exactly one that needs patching — except `identical`,
whose two sides are deliberately byte-identical.

> **Consuming `game/` with the main tool needs `girpr --include`.** Run against
> this directory unscoped, the main tool builds its plan from the whole live
> manifest and would fetch the entire game. Point it at a **copy** of `game/` and
> name one capture per run with `--include` (`removed` takes
> `--purge-only --include` instead). See
> [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle-with-girpr) for the
> procedure and the per-capture figures.

## 15. `verify/checksums.json`

```jsonc
{ "schema_version": 3,
  "algorithm": "sha256",
  "entries": [
    { "path": "expected/GenshinImpact_Data/Plugins/HoYoChannel.dll",
      "bytes": <bytes>,
      "md5": "<md5>",
      "sha256": "…", "encoding": "identity" },
    { "path": "manifests/<tag>/<field>.manifest.json.gz",
      "bytes": <bytes>,
      "md5": "<md5>",
      "sha256": "…", "encoding": "gzip" }
  ],
  "file_count": <count>, "total_bytes": <bytes> }
```

This is the **complete** inventory of the bundle: every file except
`checksums.json` itself. `file_count` is therefore one less than the number of
files on disk, and a bundle whose count disagrees with its tree fails A16.

**`file_count` is a property of the shape and `total_bytes` is a property of the
run.** The count follows the tree's shape, which is the same for captures of the
same pair — the same documents, the same tree — while the byte total moves with
the capture's own timestamp and id, which appear in some of the documents and
change their lengths. A reader comparing bundles of the same pair should expect
the counts to agree and the byte totals to differ, and should not read the
difference as a different tree.

**`encoding: gzip` means the digest is of the _decompressed_ body**, and `bytes`
is the decompressed length too. That is the only way a reader can check the
document rather than the container, and it means a digest does not change if the
file is recompressed or produced by a different compressor. It also means
verifying a `.gz` entry means decompressing it first — which is what `gircap
verify` does, and why A16 would fail if the rule and the implementation
disagreed.

A16 is deliberately **independent passes**: one that writes the digests and one
that re-reads the tree and compares. A single pass would make the assertion a
tautology. Because `checksums.json` must cover `selfcheck.json` — and `README.md`,
which renders it — and `selfcheck.json` must carry the A16 row, the close-out
writes those documents again and the later scan is the one recorded. The passes
converge exactly: the later `checksums.json` is scanned after the final
`selfcheck.json` and `README.md`, and neither of those sizes depends on a digest,
so another pass would find nothing new.

**A16's row states a file count and not a byte total.** That is forced, not
chosen: the row lives inside `selfcheck.json`, which is one of the files
`total_bytes` sums, so the last version of that number to be written is the one in
`checksums.json` — and any total in the row describes an earlier, smaller tree.
The count is safe, because no file appears or disappears between the passes.

## 16. `verify/selfcheck.json`

```jsonc
{ "schema_version": 3,
  "result": "pass",                                  // pass | fail | incomplete
  "counts": { "total": <count>, "passed": <count>, "failed": <count> },
  "assertions": [
    { "id": "A6", "assertion": "chunk decompressed md5 matches",
      "subject": "chunks/blobs/<chunk id>.zst",
      "expected": "<md5>",
      "actual":   "<md5>",
      "ok": true }
  ],
  "matrix": [ { "class": "identical", "status": "captured", "ok": true }, … ]
}
```

One row per assertion **instance**, not per assertion kind — a bundle with many
stored chunks produces that many A6 rows, so `counts.total` is larger than the
number of assertion ids, and its value is a property of the bundle rather than a
fixed tally.
`ok:false` rows are never dropped; a failing capture leaves them in place so the
failure is inspectable rather than summarized away.

`assertion` carries the assertion's wording, so a reader looking at a failed
bundle does not have to open another file to find out what A11 means. The
wording table is pinned by a test: `id` alone would pass if an assertion were
silently narrowed, and "every reused chunk's old-file slice MD5 matches" and
"every reused chunk's MD5 matches" are both `A10`. It is also compared against
the specification's own table, so the wording a bundle publishes and the
wording the specification states cannot drift apart.

A skipped row is recorded too, and is **not** a failure: `ok` is absent and the
row is listed under `skipped`. A `--no-chunk-blobs` capture skips A6–A12, A19 and
A21–A23, reports `result: "incomplete"`, and exits `0`.

A capture's `selfcheck.json` never contains an `A18` **or** an `A20` row: both
are statements about the bundle rather than about the run, so only
`gircap verify` can assert them. A reader who sees either id in this file has
been handed a `verify` report, not a capture's own account of itself.

## 17. `README.md`

A rendering of `MANIFEST.json` and `selfcheck.json`: versions, the matrix table
with paths and sizes, the totals, any failed assertion, and how to re-verify.
It is generated from the **final** `selfcheck.json` — after the A16 row is
recorded — so its counts agree with the document it renders, and it carries no
facts of its own. It is safe to delete.
