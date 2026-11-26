//! Consolidated repair integration tests.
//!
//! The repair-focused cases used to be eight separate Cargo test targets,
//! each linking the whole workspace tree. This root keeps one
//! crate-root `common` shim and pulls the case files in as modules, so the
//! linker work happens once for the group instead of once per file. Each case
//! is still its own `#[test]`; only its fully-qualified path gains the module
//! prefix.

#[path = "common/mod.rs"]
mod common;

#[path = "repair_limits.rs"]
mod repair_limits;
#[path = "repair_machinery.rs"]
mod repair_machinery;
#[path = "repair_offline.rs"]
mod repair_offline;
#[path = "repair_persistent.rs"]
mod repair_persistent;
#[path = "repair_plan.rs"]
mod repair_plan;
#[path = "repair_purge.rs"]
mod repair_purge;
#[path = "repair_scan.rs"]
mod repair_scan;
#[path = "repair_scope.rs"]
mod repair_scope;
#[path = "repair_writer.rs"]
mod repair_writer;
