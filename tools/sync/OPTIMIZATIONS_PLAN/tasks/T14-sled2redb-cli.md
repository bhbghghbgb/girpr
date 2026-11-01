---
id: T14
title: sled2redb uses clap
batch: F
checkpoint: F
report: [L478-L493]
design_notes: "The binary reports through the same Report writer as girsync; exit 3 is the fatal code"
depends_on: []
size: S
risk: low
---

# T14 — `sled2redb` uses clap

## Goal

Replace the hand-rolled positional/flag parser in `src/bin/sled2redb.rs` with a
`clap` derive CLI, preserving:

- the two positional paths;
- `--force`;
- `--output text|json`, defaulting to text;
- exit `3` for a usage or runtime failure, and exit `0` for `--help`/success.

`--help` and error text may change, but the exit-code contract may not.

## Why

`clap` is already a normal dependency and is used by `girsync`; the converter's
hand-written parser duplicates a parser the dependency graph already contains,
and its `--help` does not match the main binary.

## Files to read

- `src/bin/sled2redb.rs` — the current parser and `main`
- `src/report/mod.rs` — `OutputFormat` (already `clap::ValueEnum`)
- `src/cli.rs` — the derive style used by `girsync`
- `tests/convert.rs` — binary invocation and JSON/text assertions
- `README.md` — `sled2redb` usage line and logging mention

## Constraints

- Keep `girsync::convert::sled_to_redb(&src, &dst, force)` as the library
  entry point; do not move conversion logic into the binary.
- Keep tracing initialisation before the conversion, and keep
  `target: "girsync"` on the fatal error.
- Usage errors exit `3`, not clap's default `2`. `--help`/`--version` exit
  `0`.
- `--output` still accepts only `text`/`json`; a bad value exits `3` with a
  message that names the accepted values.
- Do not add a new dependency or a new flag beyond what exists.

## Implementation

1. Add a private derive struct:
   ```rust
   #[derive(clap::Parser, Debug)]
   #[command(name = "sled2redb", about = "Convert a legacy sled cache directory into a redb cache file")]
   struct Args {
       /// Legacy sled cache directory.
       sled_dir: PathBuf,
       /// Destination redb file.
       redb_path: PathBuf,
       /// Replace an existing destination.
       #[arg(long)]
       force: bool,
       /// How the conversion record is rendered.
       #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
       output: OutputFormat,
   }
   ```
2. Parse with `Args::try_parse()`:
   - `Ok(a)` -> use it;
   - `Err(e)` where `e.kind()` is `DisplayHelp` or `DisplayVersion` ->
     `e.exit()` (0);
   - any other error -> `e.print()` (stderr), then `std::process::exit(3)`.
   `clap::error::ErrorKind` is the thing to match. Do not use `Args::parse()`,
   which exits 2.
3. Delete `usage()` and `parse_output`; the derive's value enum replaces both.
4. Update `tests/convert.rs`:
   - the existing JSON/text success cases keep passing;
   - add: missing positional exits `3`;
   - add: `--output yaml` exits `3` and mentions text/json;
   - add: `--help` exits `0` and mentions `--force` and `--output`;
   - add: unknown flag exits `3`.
   Keep the tests spawning `env!("CARGO_BIN_EXE_sled2redb")`.
5. Update `README.md`'s converter usage line to the clap spelling and note that
   usage errors exit `3` for this binary (unlike `girsync`'s clap exit `2`).

## Commit plan

1. `refactor(sync): parse sled2redb arguments with clap`
2. `test(sync): pin sled2redb's help and exit codes`
3. `docs(sync): update the sled2redb usage line`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync --test convert
cargo test -p girsync
```

## Acceptance

- Positional arguments and `--force`/`--output` behave as before.
- A usage error exits `3`; `--help` exits `0`.
- The success record is byte-identical in text and JSON.
- The conversion library entry point is unchanged.
- No new dependency.

## Reviewer focus

- Does every clap error path exit `3` except help/version?
- Is `OutputFormat`'s value-enum spelling still `text|json`?
- Does the fatal conversion error still use `target: "girsync"`?
- Are the existing JSON assertions unchanged?
- Did the positional order stay `<sled-dir> <redb-file>`?
