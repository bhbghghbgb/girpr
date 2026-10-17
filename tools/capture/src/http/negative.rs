//! Choosing the inputs for the negative probes.
//!
//! **Lived inline in `run::capture`,** as two blocks of arithmetic between the
//! manifest fetch and the probe call. Both are pure over data the run already
//! holds, and both were untestable there: reaching either meant running a
//! capture against a live API.
//!
//! # Why the chunk id must be *new-only*
//!
//! The probe asks for a real chunk id under the **old** build's prefix and
//! records the shape of the refusal. That is a claim about namespacing: "an id
//! the old build does not have is a `404` here, even though it is a valid id
//! three characters away."
//!
//! A *shared* chunk breaks the claim silently. It is one id at two addresses,
//! the old prefix does serve it, the request returns `200`, and the probe
//! records "a valid id under the other build's prefix is not a 404" — the exact
//! opposite of the thing it exists to show, from a probe that looks like it
//! worked. So the id is selected to be new-build-exclusive, and
//! [`new_only_chunk_id`] is where that rule lives rather than being a `find`
//! with a comment beside it.

use std::collections::HashSet;

use crate::failure::{Failure, apierr};
use crate::http::ApiError;
use crate::manifest::{CapturedManifest, MANIFEST_FIELD};

/// The two `game` manifests' slots, as a slice.
///
/// Placed after the manifest fetch rather than beside the tag probe because it
/// needs the pair's blob prefixes, and a `404` measured against an invented
/// prefix would prove nothing: the point is that the *real* prefix answers
/// `404` for an id the real build does not have, and `404` for a real id under
/// the other build's.
pub fn game_pair<'a>(
    fields: &[String],
    manifests: &'a [Vec<CapturedManifest>],
) -> Result<&'a [CapturedManifest], Failure> {
    fields
        .iter()
        .zip(manifests)
        .find(|(field, _)| field.as_str() == MANIFEST_FIELD)
        .map(|(_, pair)| pair.as_slice())
        .ok_or_else(|| {
            apierr(
                "the negative probes",
                &ApiError::shape(
                    "the game manifest pair",
                    "no `game` manifest to take blob prefixes from",
                ),
            )
        })
}

/// Every chunk id a manifest declares, in manifest order.
pub fn chunk_ids(m: &CapturedManifest) -> impl Iterator<Item = &str> {
    m.entries
        .iter()
        .flat_map(|f| f.chunks.iter())
        .map(|c| c.id.as_str())
}

/// A chunk id the second build declares and the first does not.
///
/// **Exclusive to `new`, not merely real.** See the module doc: a shared chunk
/// would make the probe record the opposite of its claim, from a run that looks
/// like it worked.
///
/// The first such id in manifest order is taken. Which one is immaterial — the
/// probe asks for one, and any new-only id demonstrates the same thing — so
/// determinism is for reproducibility of `api/error_shapes.json`, not for the
/// probe to mean something different.
pub fn new_only_chunk_id<'a>(
    old_ids: impl IntoIterator<Item = &'a str>,
    new_ids: impl IntoIterator<Item = &'a str>,
) -> Result<String, Failure> {
    let old: HashSet<&str> = old_ids.into_iter().collect();
    new_ids
        .into_iter()
        .find(|id| !old.contains(*id))
        .map(str::to_string)
        .ok_or_else(|| {
            apierr(
                "the negative probes",
                &ApiError::shape(
                    "the two `game` manifests",
                    "every chunk of the new build is also in the old one, so there is \
                     no id to ask for under the wrong prefix",
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_the_new_build_alone_has_is_chosen() {
        assert_eq!(
            new_only_chunk_id(["shared", "oldonly"], ["shared", "newonly"]).expect("one exists"),
            "newonly",
            "the shared chunk must be skipped: the old prefix serves it too"
        );
    }

    #[test]
    fn every_chunk_shared_is_an_error_rather_than_a_wrong_probe() {
        // The failure mode this function exists to prevent. With no new-only
        // chunk the honest answer is "there is nothing to ask for", not "ask for
        // a shared one and record the `200` as evidence of the opposite".
        let e = new_only_chunk_id(["a", "b"], ["a", "b"]).expect_err("no new-only id");
        assert!(
            e.detail.contains("also in the old one"),
            "the error must say why there is nothing to probe: {}",
            e.detail
        );
    }

    #[test]
    fn a_new_build_that_adds_no_chunks_is_an_error_not_a_silent_shared_probe() {
        // Same as above from the other direction: the new build may legitimately
        // add nothing, and then the probe is impossible.
        assert!(new_only_chunk_id(["only"], ["only"]).is_err());
        // And an empty new manifest is too - there is nothing to name.
        assert!(new_only_chunk_id(["only"], []).is_err());
    }

    #[test]
    fn the_choice_does_not_depend_on_which_build_is_passed_first() {
        // A swapped pair would still find *an* id whenever the sets differ, so
        // "did it return something" would not catch it. The id's value does.
        assert_eq!(new_only_chunk_id(["shared"], ["shared", "only_in_new"]).unwrap(), "only_in_new");
        assert!(
            new_only_chunk_id(["shared", "only_in_new"], ["shared"]).is_err(),
            "with the pair swapped there is no id exclusive to the second build"
        );
    }

    #[test]
    fn game_pair_reports_a_missing_game_field_rather_than_indexing_panicking() {
        let manifests: Vec<Vec<CapturedManifest>> = vec![Vec::new()];
        let e = game_pair(&["zh-cn".to_string()], &manifests).expect_err("no game field");
        assert!(
            e.detail.contains("no `game` manifest"),
            "the error must name what was missing: {}",
            e.detail
        );
        // And an empty field list is the same failure, not a panic.
        assert!(game_pair(&[], &[]).is_err());
    }
}
