//! Building the *effective* map for one side of a comparison.
//!
//! An effective map is the authoritative view of a side once the cache, the
//! glob filters, and the case mode have been applied. Disk always governs:
//! stale cache entries are dropped wholesale and rehashed, and rows for paths
//! that vanished are pruned.
//!
//! ## The submodules
//!
//! One side of a run is three steps, and each step is a module here:
//!
//! | module | phase | what it does |
//! | --- | --- | --- |
//! | [`side`] | — | what a *side* is, how its cache is opened, and which one-cache-per-run rule holds |
//! | [`phase_a`] | A | [`scan_stat_only`] — walk the tree, consult the cache, **read no bytes** |
//! | [`phase_c`] | C | [`resolve_folder`] / [`resolve_record`] — hash what the planner asked for, and finalize |
//!
//! [`entry`] holds what they all exchange: [`EffRec`], [`SideEntry`], [`ScanStats`],
//! and the cache **write rule** ([`merge_row`]), which is the one thing a reader of
//! either phase has to be able to check.
//!
//! A record side has no filesystem, so its phase C is [`resolve_record`] — a map
//! copy — and its phase A is [`load_record_side_from`] rather than a walk. Both live
//! beside the folder versions they stand in for, so the pair is visible as a pair.

mod entry;
mod phase_a;
mod phase_c;
mod record;
mod side;

pub use entry::{EffRec, ScanStats, SideCapability, SideEntry, SideScan};
pub use phase_a::scan_stat_only;
pub use phase_c::{resolve_folder, resolve_record, resolve_side};
pub use record::load_record_side_from;
pub use side::{
    OpenSide, Side, classify, ensure_distinct_sides, open_folder_cache, open_side,
    resolve_sides_concurrently,
};