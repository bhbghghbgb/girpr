# T41 - B5: tune the release profile

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1027-1042 (B5).

Dependencies: none. Build only.

## Goal

The root `Cargo.toml` has no `[profile.release]`. The tool is I/O- and
MD5-bound; the default profile is not wrong, but it is untuned for a
single-binary CLI. Add a low-risk profile: thin LTO, one codegen unit, strip.

## Where to start

- `Cargo.toml` (root, after `[workspace]` or at the end).
- `README.md` build instructions, if they mention release artifacts.

## Steps

1. Add:
   ```toml
   [profile.release]
   lto = "thin"
   codegen-units = 1
   strip = true
   ```
   Do **not** add `panic = "abort"`. The report explicitly warns that abort
   bypasses `Drop`-based cleanup (locks, temp files) without a panic audit.
2. Consider `opt-level = 3` only if it is not already the release default
   (it is; omit it).
3. Do not add a custom `[profile.dev]` or change test profiles.
4. Measure before/after only if the task's reviewer asks: `cargo build
   --release -p girpr`, compare binary size and a simple `--help`/plan run.
   Do not add a timing assertion to CI.
5. Update the README if it states a release command; the output path is
   unchanged.

## Tests

- `cargo build --release -p girpr` succeeds.
- `cargo test -p girpr` (debug) still passes; profile changes do not affect it.
- Run the release binary once with `--help` and a tiny `--print-plan` fixture
  if available; it must behave the same.
- Record the binary size before/after in the return, not in a test.

## Acceptance

- A release profile with thin LTO, one codegen unit, and strip.
- No `panic = "abort"`.
- Release build works; behavior is unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `build: tune the release profile`

## Notes

- Full LTO trades build time for a marginal gain; thin LTO is the low-risk
  choice.
- The dominant cost in this tool is I/O and MD5, so do not promise a large
  speedup.
