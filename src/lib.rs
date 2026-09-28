//! girpr — Genshin Impact low-disk repair patcher.
//!
//! Brings a possibly-corrupt, possibly-any-older-version Genshin install to the
//! current live version with minimal extra disk. Unattended by design: args in,
//! exit code + `REPORT`/`PROGRESS`/`SUMMARY` stdout lines out, no UI.
//!
//! Core = Starward's repair-in-chunk-mode (version-agnostic, per-file
//! `{path}_tmp` streaming repair, verified local-chunk reuse, atomic promote)
//! plus Collapse's extra-file purge as a pre/post phase. Background:
//! `docs/01-findings-and-verdict.md`, spec: `docs/02-repair-chunk-spec.md`,
//! module plan: `docs/03-rust-implementation-plan.md`.
//!
//! Module map:
//! - [`cli`] / [`config`] — the argument surface and the validated per-run
//!   configuration ([`config::RunCtx`]) plus the typed exit-code failure
//!   ([`config::RunFailure`])
//! - [`logging`] — console + always-`TRACE` file `tracing` layers
//! - [`repair`] — the whole pipeline, split into phases
//!   ([`repair::run`] is the coordinator)
//! - [`hyp`] — HoYoPlay + Sophon `getBuild` JSON APIs
//! - [`sophon`] — chunk-manifest protobuf, fetch/verify/parse, manifest filter,
//!   local chunk-reuse map
//! - [`biz`] — biz → launcher id / game id / channel tuple
//! - [`report`] — run counters and every stdout line of the automation contract
//! - [`util`] — MD5, `config.ini`, ignore/blacklist files, rel-path normalization
//!
//! The binary ([`main`], a thin shim) is: parse → install tracing → build the
//! run config → [`repair::run`] → print `SUMMARY` → exit with the returned code.

pub mod biz;
pub mod cli;
pub mod config;
pub mod hyp;
pub mod logging;
pub mod repair;
pub mod report;
pub mod sophon;
pub mod util;

pub use biz::Biz;
pub use cli::{Args, is_audio_none, normalize_audio_lang};
pub use config::{RunCtx, RunFailure};
pub use repair::run;
pub use report::Summary;
