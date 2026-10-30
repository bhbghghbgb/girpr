//! Turning the src/dst diff into a concrete work list.
//!
//! [`build_plan`] is pure — it only reads the two effective maps — which is
//! what makes `--dry-run` trustworthy: the plan it prints is byte-for-byte the
//! plan a real run would execute.
//!
//! The work is split across focused submodules:
//!
//! - [`build`] — the [`Plan`] work list and [`build_plan`], pure over the maps
//! - [`link`] — link storage, alias resolution, and the on-disk/dst spellings
//! - [`safety`] — the plan-time guards (`ensure_*_safe`, `spare_link_storage`)
//! - [`records`] — the [`Summary`], [`dry_run_records`] and [`summary_record`]
//!
//! All `pub(super)` names that lived in `crate::commands::sync::plan` before
//! this split still resolve here; this module only moves code into focused
//! files.

mod build;
mod link;
mod records;
mod safety;

#[cfg(test)]
mod tests;

pub(super) use self::build::{Plan, build_plan};
// Nothing outside `plan` names these today, but they are part of the `plan::*`
// surface this split preserves, so the re-export stays.
#[allow(unused_imports)]
pub(super) use self::build::{CopyBecause, CopyItem};
pub(super) use self::records::{dry_run_records, summary_record};
pub(super) use self::safety::{
    ensure_dirs_applicable, ensure_hard_linked_targets_safe, ensure_link_safe,
    ensure_mkdir_targets_free, spare_link_storage,
};
