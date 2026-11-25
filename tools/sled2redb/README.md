# sled2redb — one-shot sled → redb cache converter

**Audience: developers and AI agents working on `girpr`. Not for end users.**

A standalone dev tool that reads a legacy `<root>/girpr-cache` **sled directory**
and writes a new **redb** cache file. It is its own workspace member so that the
`girsync` package and binary have no `sled` in their dependency graph.

## Why it is separate

`sled` is used by exactly one thing — this one-shot migration — while every
live cache path in `girsync` uses redb. Keeping the boundary here leaves:

- `sled` as a plain (non-optional) dependency of `tools/sled2redb` alone.
- `girsync` with no `[features]`, no `dep:sled`, no `required-features` targets
  and no `sled2redb` binary.
- The converter calling girsync's **public** cache/hash/util/report API through a
  normal path dependency (`girsync::cache`, `girsync::hash`, `girsync::util`,
  `girsync::report`, `girsync::init_tracing`, `girsync::logging::Level`).
  It exposes no private girsync API.

## Build and run

```powershell
cargo build -p sled2redb                         # the converter binary
cargo nextest run -p sled2redb                   # its end-to-end cases
.\target\debug\sled2redb <SLED_DIR> <REDB_PATH>
```

`cargo nextest run -p girsync` builds no converter target.

## Command line

`sled2redb <SLED_DIR> <REDB_PATH>` takes `--force` and `--output text|json`.
The flags go on either side of the two paths — `sled2redb --force <SLED_DIR>
<REDB_PATH>` and `sled2redb <SLED_DIR> <REDB_PATH> --force` are the same run —
and `--output=json` is the same as `--output json`. A usage error exits `3`,
not clap's `2`: the converter has one failure code for every way it can fail.
This matches girsync's fatal code, while `girsync` itself keeps `2` for a parse
error.

**It has no `--version`.** The converter declares no version, so the flag is an
unknown one: the run fails with `3` like any other usage error, and no version
line is printed.

It reports through the same `girsync::report::Record` writer as `girsync`
itself, so `--output json` means the same thing in both tools.

## Conversion behaviour (unchanged by the move)

**The finished cache is moved into place once it is whole.** The new cache is
written into a temporary beside the destination and moved onto it in a single
rename, so the move cannot leave a half-written file. Replacing an existing
cache under `--force` is an atomic replace: a conversion that fails leaves the
previous cache exactly as it was, while creating a fresh one uses a no-clobber
rename that will not overwrite a destination that appeared after the initial
check. A conversion that fails leaves no destination at all without `--force`;
the temporary is removed either way, so nothing of the tool's own is left in
the folder.

The temporary is named inside the tool's own namespace —
`girpr-cache-convert-<random>.redb` — because it is written into the folder the
destination cache lives in. `tempfile` gives it a random middle and retries a
collision with a fresh name, so two conversions into the same folder cannot
share a temporary and there is no bounded family of numbered candidates to
search. The name is inside the namespace, so a conversion that is killed before it could
clean up after itself leaves tool state there, not a file the next run would
mirror out of a source folder or delete as an extra in a destination one.
Ordinarily there is no leftover at all: the temporary is owned by a handle
whose drop removes it, so a conversion that fails cleans up after itself.

**Nothing reclaims a leftover temporary.** A cache's retention bound ranges
over its published copies — the `girpr-cache-backup-*` copies and the
`girpr-cache-old-*` snapshots — and not over the converter's temporary, so one
left by a killed conversion stays in the folder until you remove it by hand.
That is deliberate: a temporary is the file a live conversion is writing, and a
sweep that could not tell a live one from a dead one would take the copy a
running conversion is making. The random name says nothing about which run left
it, so there is nothing to interpret — a leftover is simply yours to remove.

A **legacy sled directory** has to be removed before the new file can take its
name. That removal is the one step that is not atomic — a failure in that
window leaves no cache at all, and the error says so when it happens. A stale
file under `--force` is replaced by an atomic rename instead, with no such
window. A conversion that fails before that point never touches the previous
cache. The source directory is left alone either way — the conversion never
writes to it, so deleting it is your call.

Unknown hash algorithms survive conversion untouched as raw bytes, and hex
digests in the legacy JSON rows are decoded to raw bytes on the way in.

## Source layout

| File | Responsibility |
| --- | --- |
| `src/lib.rs` | the crate root; `pub mod convert` |
| `src/convert.rs` | the converter: the one module in this crate that touches `sled` |
| `src/main.rs` | the `sled2redb` binary: the two paths, `--force` and `--output`, and the shared `report::Record` writer |
| `tests/convert.rs` | end-to-end cases (build a legacy sled DB, convert, read the redb cache back) |
| `tests/common/mod.rs` | the shared test fixtures, copied from `girsync` so the cases run unchanged |

The cache API and its digest vocabulary come from `tools/sync/cache/`
(`girsync-cache`) directly, not through `girsync`'s lib facade: an unrelated
girsync edit leaves this converter's cache code and its `redb`/`postcard`/
`tempfile` tree cached.

## Logging

The log policy is the workspace's and lives in `crates/log/`, shared with the
repair binary and the other dev tools. `sled2redb` writes a log file named
`girsync_<timestamp>.log`: the policy names log files after the *project*, and
this converter is a one-shot migration for that project, so a failure should
leave a record of how far it got. Log events are emitted on girsync's `girsync`
tracing target so they pass the same filter as every other fatal.
