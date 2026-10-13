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
//! | module | milestone | owns |
//! |---|---|---|
//! | [`cli`] | M0 | the clap surface, verbatim |
//! | [`exit`] | M0 | [`ExitCode`] and the exit-code table |
//! | [`report`] | M0 | the stdout data plane; the only `println!` in the crate |
//! | [`logging`] | M0 | stderr narration via `tracing`, plus `--log-file` |
//! | [`config`] | M0 | the resolved run plan every command works from |
//! | [`run`] | M0 | command dispatch: the four entry points |
//! | [`biz`] | M1 | launcher/game ids, API bases, the channel tuple |
//! | [`http`] | M1 | [`http::Fetcher`]: retries, timeout, UA, request log |
//! | [`hyp`] | M2 | the four HoYoPlay endpoints + Sophon `getBuild` |
//! | [`manifest`] | M3-M4 | fetch → zstd → verify → decode → dump → diff |
//! | [`select`] | M5 | scoring, variant classes, budget admission |
//! | [`blobs`] | M6 | the content-addressed chunk store and the three sets |
//! | [`assemble`] | M7 | building a file from chunks: old, new, and patch flow |
//! | [`emit`] | M8 | the artifact writer |
//! | [`verify`] | M9 | the offline `verify` / `materialize` / `list` commands |
//!
//! ## Two channels
//!
//! **stdout** is the answer — exactly one `SUMMARY` per run, or one JSON object
//! under `--output json` — and [`report`] owns it. **stderr** is the narration
//! and [`logging`] owns that. Neither writes to the other's channel, which is
//! what lets `--output json` hand a caller a clean stream with no log chatter
//! mixed in.
//!
//! ## Independence
//!
//! There is no dependency on the repair crate, no `Command` against its binary,
//! no shared flags and no shared exit-code contract. The only shared vocabulary
//! is the README's version strings (`docs/00-spec.md` §2).

pub mod assemble;
pub mod blobs;
pub mod biz;
pub mod cli;
pub mod config;
pub mod emit;
pub mod exit;
pub mod http;
pub mod hyp;
pub mod logging;
pub mod manifest;
pub mod report;
pub mod run;
pub mod select;
pub mod verify;

pub use exit::ExitCode;

/// The version this tool reports in `MANIFEST.json` and in its `User-Agent`.
///
/// Kept in step with `Cargo.toml` by the M0 acceptance grep rather than by a
/// build script, so a reader of the artifact can always find the same string the
/// tool emits.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The artifact schema version stamped on every machine-readable document.
///
/// A reader must reject an unknown value rather than guess
/// (`docs/01-artifact-schema.md`). It is bumped only for a breaking change to
/// the bundle's byte layout.
pub const SCHEMA_VERSION: u32 = 1;