# T40 - B4: build the product by default

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1007-1025 (B4).

Dependencies: none. This is a convention change; keep it separate.

## Goal

The root workspace includes `tools/sync` and `tools/capture` as members. With
no `default-members`, `cargo build` / `cargo test` at the root operate on every
member, so the main binary's build also builds the dev tools and their
dependencies leak into workspace feature unification.

Make a root `cargo build`/`cargo test` mean "the product", while keeping the
tools buildable explicitly.

## Where to start

- `Cargo.toml`: `[workspace] members = [".", "crates/log",
  "crates/tempdir", "tools/sync", "tools/capture"]`.
- `README.md`: build/test commands and the tool descriptions.
- Any CI configuration (if present; the repository currently has no
  `.github/` workflow).
- `tests/` / docs that assume `cargo test` builds the tools.

## Steps

1. Add `default-members = ["."]` to `[workspace]`. Keep every member listed;
   the tools are still part of the workspace and still resolve one lockfile.
2. Verify:
   - `cargo build` / `cargo test -p girpr` no longer build the tools;
   - `cargo check -p girsync -p gircap` still builds them;
   - `cargo test --workspace` still runs everything.
3. Add the explicit tool check to the documented verification command. If a CI
   exists, add a separate job/step for `cargo check -p girsync -p gircap`; if
   not, add a short "Tools" note to the README and a script/command the
   maintainer can run. Do not invent a CI system.
4. Update the README's build/test section: root commands build the product;
   tools are explicit; `--workspace` is the everything command.
5. Do not remove the tools from `members`; that would change path/dependency
   resolution and the lockfile more than intended.

## Tests

- `cargo build -p girpr` and `cargo test -p girpr` pass.
- `cargo check -p girsync -p gircap` passes (the tools still compile).
- `cargo test --workspace` passes if the host has the tools' system
  dependencies; if not, record exactly which tool blocked and why.
- `cargo metadata --no-deps` shows `default-members` as `["."]` (or the
  equivalent).

## Acceptance

- Root `cargo build`/`cargo test` default to the product.
- The tools remain members and remain explicitly buildable.
- The README documents both commands.
- `cargo test -p girpr` passes.

## Commit plan

1. `build: build the product by default`
2. `docs: document the explicit tools check`

## Notes

- This is a convention change. Do not combine it with B1/B2/B3 dependency
  edits; a feature-unification surprise is easier to diagnose alone.
- CI, if added later, must run the tools explicitly, or a broken tool will go
  unnoticed.
