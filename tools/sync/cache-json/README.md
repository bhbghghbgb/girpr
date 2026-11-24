# cache-json — girsync cache ⇄ NDJSON

**Audience: developers and AI agents working on `girpr`. Not for end users.**

A standalone dev tool that reads a girsync redb cache (`girpr-cache`) and writes
it as **newline-delimited JSON**, and builds a new redb cache from that same
format. It is a text view of the current cache schema — not a migration tool for
old caches. Unknown schema versions are refused on both sides.

## Build and run

```powershell
cargo build -p girsync-cache-json
cargo nextest run -p girsync-cache-json

# Export to a file, or to stdout
.\target\debug\cache-json to-json <CACHE> cache.ndjson
.\target\debug\cache-json to-json <CACHE> - > cache.ndjson

# Import from a file, or from stdin (--force replaces an existing cache)
.\target\debug\cache-json from-json cache.ndjson <NEW_CACHE>
cat cache.ndjson | .\target\debug\cache-json from-json - <NEW_CACHE> --force
```

`to-json` opens the cache read-only: no rebuild, no backup, no `meta` rewrite.
`from-json` writes a fresh cache and refuses to replace an existing file unless
`--force` is given. A usage error, a bad stream and a cache failure all exit
`3`, the same failure code `girsync` and `sled2redb` use for fatal errors. There
is no `--version` flag.

**stdout is data, stderr is narration.** By default `to-json` writes the NDJSON
to stdout, so the one-line summary goes to stderr and stdout always stays
parseable. `from-json` writes nothing to stdout.

## The NDJSON format

One JSON object per line, with no pretty-printing and no line depending on
another:

```json
{"type":"meta","version":4,"case_sensitive":true}
{"type":"entry","path":"a.txt","kind":"file","size":42,"mtime_ns":123456789,"hashes":[["md5","d41d8cd98f00b204e9800998ecf8427e"],["xxhash64","ef46db3751d8e999"]]}
{"type":"entry","path":"sub","kind":"dir","size":0,"mtime_ns":0,"hashes":[]}
```

- The **meta line must be first and appear exactly once**. `version` must equal
  the cache crate's current `CACHE_VERSION`; a foreign version is an error.
  `case_sensitive` is written back into the new cache's `meta` row.
- Each **entry line** carries the `/`-separated relative path that keys the
  row, the closed `kind` (`"file"` or `"dir"`), the raw `size` and `mtime_ns`
  (nanoseconds since the epoch), and its digests.
- Digests are `(algorithm name, lowercase hex)` pairs in a **list**, not a map.
  The cache carries algorithm names inline: a name this build does not know is
  preserved verbatim, and a known name with an unexpected digest length is kept
  too. A map could not always represent those rows. Export writes known
  algorithms first in canonical order, then the others, so the text form is
  deterministic.

Import refuses a malformed line, a line that is not an object of the expected
shape, a second meta line, a duplicate path and an invalid `kind` or hex digest.
Each error names its line number. A blank line is an error too; the trailing
newline after the last object does not produce one.

## Failure behaviour

`from-json` writes the new cache under a random temporary name beside the
destination (`girpr-cache-json-<random>.redb`) and publishes the finished file
in one rename. A bad line at the end of a large stream therefore leaves the
previous cache exactly as it was. Without `--force` the publish is no-clobber,
so a destination that appears during the import is not overwritten. The
temporary is owned by a handle whose drop removes it on any error path, and its
lock sidecar is removed with it. A process killed mid-import can leave the
temporary behind; its name is inside the `girpr-cache` namespace, so it reads as
tool state rather than as content, and nothing reclaims it automatically.

`to-json` streams rows from one cache read transaction. If the output file
already exists it is truncated. If stdout is the target and an error occurs
mid-export, the lines written so far are already on stdout — the format is
incremental, so a parser can consume them even when the run fails.

## Source layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | crate root: the clap surface, the two public conversion functions |
| `src/ndjson.rs` | the line shapes, their serde impls and the hex digest spelling |
| `src/convert.rs` | the two directions, destination safety and the temporary/publish protocol |
| `src/main.rs` | the binary: endpoint selection, dispatch, stderr summary, exit code |
| `tests/roundtrip.rs` | end-to-end cases over real caches and real text |

The cache API, its `CACHE_VERSION` and its digest vocabulary come from
`tools/sync/cache/` (`girsync-cache`) directly, not through girsync's lib facade:
an edit to girsync's planner or commands leaves this tool's cache tree cached.
The two read seams this tool needs — `CacheDb::visit_entries` and `CacheDb::meta`
— live in that crate beside the other read methods, so the wire format stays in
one place.
