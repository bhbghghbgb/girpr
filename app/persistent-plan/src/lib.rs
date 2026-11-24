//! The persistent-directory plan: values and every inspectable decision.
//!
//! This is a **leaf**. It depends on the dispatcher's value types and on the
//! digest/path mechanics, but not on `girpr-fetch`, `girpr-repair-plan` or the
//! binary. No function here opens a socket, writes a file or decides an exit
//! code: the caller supplies the bytes it already has and gets values back.
//! That is what makes the plan half testable offline, one rule at a time.
//!
//! The rules live in focused modules:
//!
//! - [`manifest`] — NDJSON rows, blank-line/extra-key tolerance and the typed
//!   parse error;
//! - [`placement`] — suffix → folder, force-persistent, cleaned rels and CDN
//!   URL assembly;
//! - [`digest`] — which digest a row requires and the size-first gate;
//! - [`marker`] — the exact marker bytes and the C7 scope predicates;
//! - [`plan`] — the fold that produces [`PersistentPlan`] from the inputs.
//!
//! `docs/09` §4.1–4.3 is the authority for the shapes and the constants.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod digest;
pub mod manifest;
pub mod marker;
pub mod placement;
pub mod plan;

pub use digest::{
    DigestVerdict, ExpectedDigest, ObservedDigest, expected_digest, verify_bytes, verify_observed,
};
pub use manifest::{ParseError, parse_rows};
pub use marker::{
    ScopeDecision, base_res_version_hash_marker, first_out_of_scope_hotfix,
    first_out_of_scope_marker, render_markers, scope_decision,
};
pub use placement::{
    BaseTree, Folder, asset_rel, asset_urls, data_dir_name, folder_for, force_persistent_decision,
    placement_for,
};
pub use plan::{ManifestInput, build_plan};

pub use girpr_dispatch::{ManifestDescriptor, ManifestKind};

pub use value::{MarkerSet, PersistentAsset, PersistentPlan, PersistentRow};

mod value;

use std::fmt;

/// A bad input to a pure plan decision (no I/O and no exit-code policy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// `exe_file_name` from the game config is missing or names more than a stem.
    ExeName(String),
    /// A server-supplied path could not be cleaned into a contained rel.
    Rel { value: String, reason: String },
    /// A dispatcher value the URL/marker rules need is empty.
    MissingValue(&'static str),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExeName(v) => write!(f, "bad exe_file_name {v:?}"),
            Self::Rel { value, reason } => write!(f, "bad rel {value:?}: {reason}"),
            Self::MissingValue(field) => write!(f, "empty dispatcher value {field}"),
        }
    }
}

impl std::error::Error for PlanError {}

pub type Result<T> = std::result::Result<T, PlanError>;
