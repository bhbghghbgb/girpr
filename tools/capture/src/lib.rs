//! # `gircap` — live-artifact capture for `girpr` testing
//!
//! Dev-only. Not a patcher, not part of the repair binary, sharing no code with
//! it. `gircap` talks to the live HoYoPlay + Sophon APIs and writes a
//! self-contained artifact bundle: a real `game_dir` fixture, every API response
//! and manifest behind it, every chunk blob needed to materialize the old file,
//! the current file and the patch between them, and ground truth for the result.
//!
//! ## Module map
//!
//! | module | owns |
//! |---|---|
//! | [`cli`] | the clap surface, verbatim |
//! | [`exit`] | [`ExitCode`] and the exit-code table |
//! | [`hash`] | digests, one-pass and one-shot; the gzip rule; zstd helpers |
//! | [`report`] | the stdout data plane; the only `println!` in the crate |
//! | [`logging`] | stderr narration via `tracing`, plus the log file |
//! | [`config`] | the resolved run plan every command works from |
//! | [`failure`] | [`failure::Failure`] and the four ways a command can say it failed |
//! | [`run`] | command dispatch: the four entry points |
//! | [`biz`] | launcher/game ids, API bases, the channel tuple |
//! | [`http`] | [`http::Fetcher`]: retries, timeout, headers, request log |
//! | [`hyp`] | the four HoYoPlay endpoints + Sophon `getBuild` |
//! | [`manifest`] | fetch → zstd → verify → decode → dump → diff |
//! | [`paths`] | the shape rule for values that name something inside a bundle |
//! | [`select`] | scoring, variant classes, budget admission |
//! | [`blobs`] | the content-addressed chunk store and the three sets |
//! | [`assemble`] | building a file from chunks: old, new, and patch flow |
//! | [`emit`] | the artifact writer |
//! | [`verify`] | the offline `verify` / `materialize` / `list` commands |
//!
//! ## Two channels
//!
//! **stdout** is the answer — a stream of [`report::Record`]s, one per line,
//! ending in exactly one `SUMMARY`, or the same records as NDJSON objects under
//! `--output json` — and [`report`] owns it. **stderr** is the narration
//! and [`logging`] owns that. Neither writes to the other's channel, which is
//! what lets `--output json` hand a caller a clean stream with no log chatter
//! mixed in.
//!
//! ## Independence
//!
//! There is no dependency on the repair crate, no `Command` against its binary,
//! no shared exit-code contract and no shared flags apart from the log ones.
//! The only other shared vocabulary is the README's version strings
//! (`docs/00-spec.md` §2), and the one shared crate is `girpr-log`
//! (`crates/log/`): the log-file and log-level *policy*, which holds no logic
//! of any program.

pub mod assemble;
pub mod biz;
pub mod blobs;
pub mod cli;
pub mod config;
pub mod emit;
mod encoding;
pub mod exit;
pub mod failure;
pub mod hash;
pub mod http;
pub mod hyp;
pub mod logging;
pub mod manifest;
pub mod paths;
pub mod report;
pub mod run;
pub mod select;
mod tasks;
pub mod verify;

pub use exit::ExitCode;

// The log policy crate, re-exported so that a caller can name the type
// [`logging::LogGuard::plan`] returns without adding a dependency of its own:
// the policy is part of this crate's public signature, so it is part of this
// crate's public surface.
pub use girpr_log;

/// The version this tool reports in `MANIFEST.json` and in its `User-Agent`.
///
/// Kept in step with `Cargo.toml` by reading it at build time rather than by a
/// build script, so a reader of the artifact can always find the same string the
/// tool emits.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The artifact schema version stamped on every machine-readable document.
///
/// A reader must reject an unknown value rather than guess
/// (`docs/01-artifact-schema.md`). It is bumped only for a breaking change to
/// the bundle's byte layout.
///
/// **Version 2** dropped two blocks from the patchmap and replaced the class
/// set:
///
/// - the `repair` block is gone — it stated what a repair tool *would* do
///   with the file (the tier, the write order, the trim, the byte
///   accounting), which is that tool's own business and is not shared with any
///   other repair tool;
/// - the `expectations.summary` block is gone — it transcribed a repair
///   tool's `SUMMARY` line, per flag, into a foreign schema;
/// - the class ids are the six **patch shapes** rather than the repair tool's
///   internal decisions.
///
/// A version-1 bundle is **refused**: `gircap verify` says so and names the
/// command that produces a current one.
pub const SCHEMA_VERSION: u32 = 2;
