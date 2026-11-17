# sled2redb — one-shot sled → redb cache converter

**Audience: developers and AI agents working on `girpr`. Not for end users.**

A standalone dev tool that reads a legacy `<root>/girpr-cache` **sled directory**
and writes a new **redb** cache file. It used to live inside `girsync` behind a
`convert` Cargo feature; it is now its own workspace member so that the
`girsync` package and binary have no `sled` in their dependency graph at all.

## Why it is separate

`sled` was used by exactly one thing — this one-shot migration — while every
live cache path in `girsync` uses redb. Keeping the converter in `girsync`
meant adding `sled` to girsync's graph (even as an optional dependency) and
gating a binary and a test target behind a feature. The extraction keeps the
conversion code unchanged and moves only the crate boundary:

- `sled` is a plain (non-optional) dependency of `tools/sled2redb` alone.
- `girsync` has no `[features]`, no `dep:sled`, no `required-features` targets
  and no `sled2redb` binary.
- The converter calls girsync's **public** cache/hash/util/report API through a
  normal path dependency (`girsync::cache`, `girsync::hash`, `girsync::util`,
  `girsync::report`, `girsync::init_tracing`, `girsync::logging::Level`).
  Nothing private was used, so no new public API had to be exposed.
  `crate::…` paths in the moved source became `girsync::…` paths.

## Build and run

```powershell
cargo build -p sled2redb                         # the converter binary
cargo nextest run -p sled2redb                   # its end-to-end cases
.\target\debug\sled2redb <SLED_DIR> <REDB_PATH>
```

`cargo nextest run -p girsync` no longer builds or runs any converter target.

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

**A conversion is atomic for a file destination.** The new cache is written
into a temporary beside the destination and moved onto it only once it is
whole, so the destination is either the old cache or the new one — never a
half-written mixture, and never a file that opens as a valid cache. A
conversion that fails leaves the previous cache exactly as it was under
`--force`, and leaves no destination at all without it; the temporary is
removed either way, so nothing of the tool's own is left in the folder.

The temporary is named inside the tool's own namespace —
`girpr-cache-convert-<pid>-<ts>.redb`, or `girpr-cache-convert-<pid>-<ts>_2.redb`
and its `_3`, `_4`, … spellings when a name of that shape is already taken —
because it is written into the folder the destination cache lives in. Every
spelling of it is inside that namespace, so a conversion that is killed before
it could clean up after itself leaves tool state there, not a file the next run
would mirror out of a source folder or delete as an extra in a destination one.
**That family of names is bounded** — the plain name and the `_2` … `_1000`
spellings, a thousand candidates in all — so a conversion that finds every one
of them taken fails with exit `3` rather than trying forever.

**Nothing reclaims that temporary.** A cache's retention bound ranges over its
published copies — the `girpr-cache-backup-*` copies and the `girpr-cache-old-*`
snapshots — and not over the converter's temporary, so a leftover stays in the
folder until you remove it by hand. That is deliberate: a temporary is the file
a live conversion is writing, and a sweep that could not tell a live one from a
dead one would take the copy a running conversion is making. The name carries
the process id, so a leftover says which run left it.

That process id is for **you**, not for the tool: it cannot answer "is a
conversion still writing this?". A process id is **reused**, so the number in
the name may belong to an unrelated process by now. And the folder can be
**shared between hosts** (a network folder, a container mount, a synced
directory), where the number was minted in a process table this machine cannot
ask about at all. So the id names the run for a reader, and a leftover is yours
to remove.

Replacing a **legacy sled directory** is the one step that is not atomic: the
directory has to be removed before the new file can take its name, so a failure
in that window leaves no cache at all. The error says so when it happens. The
source directory is left alone either way — the conversion never writes to it,
so deleting it is your call.

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

## Logging

The log policy is the workspace's and lives in `crates/log/`, shared with the
repair binary and the other dev tools. `sled2redb` writes a log file named
`girsync_<timestamp>.log`: the policy names log files after the *project*, and
this converter is a one-shot migration for that project, so a failure should
leave a record of how far it got. Log events are emitted on girsync's `girsync`
tracing target so they pass the same filter as every other fatal.
