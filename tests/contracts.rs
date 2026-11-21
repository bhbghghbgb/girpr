//! Consolidated contract and process integration tests.
//!
//! The replay-server, CLI-contract, process and run-lock cases used to be
//! separate Cargo test targets, each linking the whole workspace tree. This
//! root shares one crate-root `common` shim across the group and pulls the case
//! files in as modules. Each case is still its own `#[test]`; the run-lock file
//! keeps its own `#[path]` include of the lock-cleanup helpers.

#[path = "common/mod.rs"]
mod common;

#[path = "artifact_server.rs"]
mod artifact_server;
#[path = "cli_contract.rs"]
mod cli_contract;
#[path = "process.rs"]
mod process;
#[path = "run_lock.rs"]
mod run_lock;
