//! The runtime ledger now lives in `girpr-report`, together with the `Summary`
//! it charges and the `StoppedBy` marker it reads. Keeping the whole
//! `Summary`/`Meter`/`StoppedBy` group in one crate is what makes the former
//! `report::summary ↔ scope::ledger` cycle legal: the boundary sits around the
//! group instead of cutting through it.
//!
//! This module remains as a thin alias so the repair internals can keep their
//! `crate::repair::scope::ledger::*` paths; no policy or type is defined here.

pub use girpr_report::{Meter, StoppedBy};
