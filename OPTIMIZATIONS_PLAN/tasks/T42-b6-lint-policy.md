# T42 - B6: add a workspace lint policy

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1044-1058 (B6).

Dependencies: none. CI/build only.

## Goal

The code is comment-rich and internally consistent, but `clippy` is only run
manually. Add a shared lint table so new code stays aligned without a review
reminder. Start small: `clippy::all` at warn and a small deny list, not a
blanket deny.

## Where to start

- `Cargo.toml` (root): add `[workspace.lints.clippy]` and
  `[workspace.lints.rust]`, then `[lints] workspace = true` in the package
  tables that opt in.
- `crates/log/Cargo.toml`, `crates/tempdir/Cargo.toml` if they should share
  it (they are workspace members and shared policy crates).
- `tools/sync`, `tools/capture`: out of scope for this plan. Do not force them
  to opt in; the workspace table applies only to packages with
  `[lints] workspace = true`.
- README / CI section.

## Steps

1. Add a workspace lint table:
   ```toml
   [workspace.lints.rust]
   unsafe_code = "deny"
   # Do not deny warnings here; CI decides.

   [workspace.lints.clippy]
   all = "warn"
   unwrap_used = "warn"
   expect_used = "warn"
   ```
   Choose the small list deliberately. Do not add
   `cast_possible_truncation`/`cast_sign_loss` as denies; the codebase has
   intentional `i64 -> u64` clamping that would explode.
   `undocumented_unsafe_blocks` only if unsafe is present; `unsafe_code =
   "deny"` is stronger and simpler if there is none.
2. Add `[lints] workspace = true` to the main package. Do the same for
   `crates/log`/`crates/tempdir` only if they already pass; a shared policy
   crate should be clean.
3. Do **not** add `#![deny(warnings)]`; that makes a new compiler warning break
   the build. Let CI run `cargo clippy -- -D warnings` if desired.
4. Add/document the verification command:
   `cargo clippy -p girpr --all-targets -- -D warnings` (or the repo's
   equivalent). If a CI exists, add it; if not, document it as the local gate.
5. Fix the existing warnings the new table surfaces, or downgrade a specific
   lint with a comment and a reason. Do not add blanket allows to silence the
   table.
6. D9's `too_many_arguments` allows are removed by T35; do not add a workspace
   allow for that lint here.

## Tests

- `cargo clippy -p girpr --all-targets` passes with the chosen policy (or the
  warnings are documented as accepted).
- `cargo test -p girpr` passes.
- `cargo check --workspace` still passes for the packages that opted in.
- The tools still build (they do not opt in).

## Acceptance

- A workspace lint table exists with a small, justified policy.
- The main package opts in.
- No `deny(warnings)` and no blanket allow.
- The verification command is documented.
- `cargo test -p girpr` passes.

## Commit plan

1. `build(lints): add a workspace lint policy`
2. `chore: fix or annotate the surfaced lints`

## Notes

- Lint churn is noise. Keep the first table small; add a lint when a real
  class of bug motivates it.
- Do not make the tools inherit the policy in this task; they are separate
  products and may have different constraints.
