//! girpr — Genshin Impact low-disk repair patcher.
//!
//! Brings a possibly-corrupt, possibly-any-older-version Genshin install to the
//! current live version with minimal extra disk. Unattended by design: args in,
//! exit code + `REPORT`/`PROGRESS`/`SUMMARY` stdout lines out, no UI.
//!
//! Core = Starward's repair-in-chunk-mode (version-agnostic, per-file
//! `{path}_tmp` streaming repair, verified local-chunk reuse, atomic promote)
//! plus Collapse's extra-file purge as a pre/post phase. Background:
//! `docs/01-repair-findings-and-verdict.md`, spec:
//! `docs/02-repair-chunk-spec.md`, module plan:
//! `docs/03-repair-rust-implementation-plan.md`.
//!
//! ## Public surface
//!
//! The binary is the product, so the library exposes one deliberate seam:
//!
//! - [`run`] — the whole pipeline, and its configuration ([`RunCtx`]) and
//!   typed failure ([`RunFailure`]);
//! - [`Summary`], the types its public fields name ([`SummaryFlags`], [`Meter`],
//!   [`Ceiling`]), [`OutputFormat`], [`Record`], [`emit`](fn@emit) and
//!   [`summary_record`] — the run counters and the stdout automation contract;
//! - [`Args`] plus the audio helpers — the command-line parser;
//! - [`Biz`] — the game/channel selector;
//! - [`init_tracing`], [`flush_log`], [`Level`] and [`girpr_log`] — the logging
//!   entry points and the policy type `init_tracing` returns.
//!
//! Everything else is an implementation detail. Integration tests reach the
//! internal types they construct through [`test_support`], a hidden seam with
//! a narrow, documented list.
//!
//! The modules behind the seam are private: `cli` / `config` (the argument
//! surface and validated per-run configuration), `logging` (console + file
//! `tracing` layers over the shared `girpr-log` policy), `repair` (the
//! pipeline, split into phases), `hyp` (HoYoPlay + Sophon `getBuild` JSON
//! APIs), `sophon` (chunk-manifest protobuf, fetch/verify/parse, manifest
//! filter, local chunk-reuse map), `biz` (biz → launcher id / game id /
//! channel tuple), `report` (run counters, the record data plane and every
//! stdout line), `retry` (the shared attempt/backoff loop), `sleep` (the
//! injectable wait), `phase` (the monotonic phase timer) and `util` (MD5, `config.ini`,
//! ignore/blacklist files, server-path containment).
//!
//! The binary (`main`, a thin shim) is: parse → install tracing → build the
//! run config → [`run`] → print `SUMMARY` → flush the log → return the exit
//! code.

// Unit tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod biz;
mod cli;
mod config;
mod hyp;
mod logging;
mod phase;
mod repair;
mod report;
mod retry;
mod sleep;
mod sophon;
mod util;

// The log policy crate, re-exported so that a caller can name the type
// [`init_tracing`] returns without adding a dependency of its own: the policy
// is part of this crate's public signature, so it is part of this crate's
// public surface.
pub use girpr_log;

pub use biz::Biz;
pub use cli::{Args, is_audio_none, normalize_audio_lang, parse_audio_selection};
pub use config::{RunCtx, RunExit, RunFailure};
pub use girpr_log::Level;
pub use logging::{flush_log, init_tracing};
pub use repair::run;
pub use repair::scope::ledger::Meter;
pub use report::{Ceiling, OutputFormat, Record, Summary, SummaryFlags, emit, summary_record};

/// The integration tests' seam.
///
/// These items are implementation details. They are public only so that the
/// integration tests can construct a run and inspect its results without
/// importing a private module. Nothing here is production API.
///
/// The list is deliberately explicit:
///
/// - `RepairMode`, `Lock`, `Limits` and `PathScope` let a test build a
///   [`RunCtx`] and drive the real pipeline;
/// - `NoSleep` and `RealSleep` choose the wait policy without a test sleeping
///   for real;
/// - `ChunkFetch`, `ChunkSink`, `SophonChunk`, `SophonChunkFile`,
///   `SophonChunkManifest` and `stream_chunk_to` are the manifest and stream
///   helpers the offline artifact server and the peak-heap probe need;
/// - `EMPTY_MD5` is the digest the fixtures use for an absent chunk.
///
/// `Record` and `summary_record` repeat the root exports on purpose: an
/// integration test can import everything it needs from this one module. That
/// repetition is a convenience of the test seam, not a second API.
#[doc(hidden)]
pub mod test_support {
    pub use crate::config::RepairMode;
    pub use crate::repair::lock::Lock;
    pub use crate::repair::scope::{Limits, PathScope};
    pub use crate::report::{Record, summary_record};
    pub use crate::sleep::{NoSleep, RealSleep};
    pub use crate::sophon::{
        ChunkFetch, ChunkSink, SophonChunk, SophonChunkFile, SophonChunkManifest, stream_chunk_to,
    };
    pub use crate::util::EMPTY_MD5;
}
