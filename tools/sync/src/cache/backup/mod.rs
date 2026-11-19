//! Publishing cache copies: working names, links and claims.
//!
//! The copy/flush/publish mechanics live in [`publish`]. They are reached
//! through the [`Publisher`](publish::Publisher) interface, and the
//! cross-process record protocol they depend on is supplied by the cache layer
//! through [`PublishHooks`](super::open::PublishHooks), so the records and
//! registries stay where the rest of `open`'s protocol lives.
//!
//! The one-syscall gaps the protocol documents — a published name never
//! holding half a copy, and a publish failing loudly when its claim is no
//! longer this run's — are properties of the code in [`publish`]. Delegating
//! the working name to `tempfile` adds uniqueness and atomic creation; it does
//! not put a lock or a second conditional call into the publish path.

mod publish;

pub(super) use publish::{copy_aside, remove_cache_path};

#[cfg(test)]
pub(super) use publish::{
    LINKING_UNAVAILABLE, NameClaim, PublishError, REMOVAL_UNAVAILABLE, RENAME_UNAVAILABLE,
    WORK_PREFIX, arm_publish_gate, candidate_name, claim_sibling, drop_the_claim, identity_at,
    publish_dir, publish_gate, publish_onto_the_claimed_name,
};
