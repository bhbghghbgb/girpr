//! The run's phases: metadata → manifests → plan → check → post.
//!
//! [`crate::repair::run`] (in `src/repair/mod.rs`) owns phase order; each
//! phase here owns its own step of docs/02. No phase imports another phase.
//!
//! | phase | module | docs/02 |
//! |---|---|---|
//! | local state + server metadata | `metadata` | steps 1–2 |
//! | manifests + local chunk map | `manifest` | step 3 |
//! | work list + purge candidates | `girpr-repair-plan` | steps 4, 5, 7 |
//! | `--check-only` verify | `check` | step 6.5 |
//! | post-phase | `post` | step 7 |
//!
//! The plan values and their pure decision live in the `girpr-repair-plan`
//! leaf; this facade keeps the fetch, walk, publish and byte-emitting policies
//! that consume them.

pub mod check;
pub mod manifest;
pub mod metadata;
pub mod post;
