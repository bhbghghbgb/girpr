//! Fetching one manifest per `matching_field` per build, and filing each where
//! it belongs.
//!
//! **Lived inline in `run::capture`,** where the fetch loop and the filing loop
//! were adjacent and shared one piece of index arithmetic. That arithmetic —
//! `fields[k / 2]`, `k % 2 == 0 ? old : new` — was the only thing keeping the
//! two halves together, and it is the kind of coupling that survives a rename.
//!
//! # The shape, stated once
//!
//! [`wants`] expands `(field, build)` into the flat list of manifests a run
//! needs, and each [`Want`] carries **where its result goes**: the field index it
//! belongs to and which side of the pair it is. Filing is then
//! `manifests[want.field].push(m)` with no arithmetic at all. A download that
//! finished first, or last, lands in the same slot either way, because the slot
//! travelled with the request rather than being recomputed from the position.
//!
//! [`fetch_all`] returns `Vec<Vec<CapturedManifest>>` indexed by field, and
//! within each field **`[0]` is the old build and `[1]` is the new one**. Every
//! downstream stage indexes positionally — the diff zips the pair, the negative
//! probes take `&pair[0]` and `&pair[1]` — so that ordering is a contract, and
//! `wants_are_interleaved_old_then_new_per_field` is the test that holds it.

use crate::failure::{Failure, apierr};
use crate::http::{ApiError, Fetcher};
use crate::hyp::GetBuild;

use super::{CapturedManifest, verify};

/// One manifest a run needs.
#[derive(Debug, Clone)]
pub struct Want {
    /// Which field's slot this result is filed into.
    pub field: usize,
    /// The field's name, for the error message and the verify step.
    pub field_name: String,
    /// `true` for the new build, `false` for the old.
    pub is_new: bool,
    /// The tag to fetch.
    pub tag: String,
    /// The build to fetch it from.
    pub build: GetBuild,
}

/// Expand the `(field, build)` product into the flat list of manifests to fetch.
///
/// **Old then new, per field, fields in the order given.** `game` first because
/// it is the field every later stage needs regardless of which class a capture
/// lands in; the audio codes follow because they exist only to widen the
/// *selection pool*, and a voice pack must never outrank a game file on a cost
/// tie (see [`crate::select`]).
///
/// `--no-chunk-blobs` does not change this. A manifest is metadata and the flag
/// is about chunk blobs; skipping it would leave a bundle whose `versions` block
/// names builds it has no manifest for.
pub fn wants(
    fields: &[String],
    old_tag: &str,
    new_tag: &str,
    old_build: &GetBuild,
    new_build: &GetBuild,
) -> Vec<Want> {
    let mut out = Vec::with_capacity(fields.len() * 2);
    for (i, field) in fields.iter().enumerate() {
        for (tag, build, is_new) in
            [(old_tag, old_build, false), (new_tag, new_build, true)]
        {
            out.push(Want {
                field: i,
                field_name: field.clone(),
                is_new,
                tag: (*tag).to_string(),
                build: build.clone(),
            });
        }
    }
    out
}

/// Fetch and verify every manifest in `wants`, all at once.
///
/// **All at once, and filed in input order.** The loop this replaces was
/// `for field { for tag { capture().await } }`, which put a 9 MB download after a
/// 2 MB download after a 9 MB download after a 2 MB download for no reason:
/// nothing here depends on anything else here. On the reference bundle that is
/// ~20 MB strictly in series.
///
/// The order they are *verified and filed* in is unchanged and is still input
/// order, because `map_concurrently` returns input order — so which download
/// finished first cannot move anything in the bundle.
pub async fn fetch_all(
    fetcher: &Fetcher,
    wants: &[Want],
    field_count: usize,
    jobs: usize,
) -> Result<Vec<Vec<CapturedManifest>>, Failure> {
    let fetched = fetcher
        .map_concurrently(wants, jobs, |fetcher, i, want| async move {
            match super::capture(&fetcher, &want.build, &want.tag, &want.field_name).await {
                Ok(m) => (i, Ok(m)),
                Err(e) => (i, Err(e.to_string())),
            }
        })
        .await;

    let mut manifests: Vec<Vec<CapturedManifest>> =
        (0..field_count).map(|_| Vec::with_capacity(2)).collect();
    for (k, r) in fetched.into_iter().enumerate() {
        let want = &wants[k];
        let m = match r {
            Ok(m) => m,
            Err(e) => {
                return Err(apierr(
                    &format!("fetch the {} {} manifest", want.tag, want.field_name),
                    &ApiError::shape(&want.tag, e),
                ));
            }
        };
        verify(&m, &want.tag, &want.field_name)?;
        manifests[want.field].push(m);
    }
    Ok(manifests)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(id: &str, tag: &str) -> GetBuild {
        GetBuild {
            build_id: id.to_string(),
            tag: tag.to_string(),
            manifests: Vec::new(),
        }
    }

    #[test]
    fn wants_are_interleaved_old_then_new_per_field() {
        // The contract: `fetch_all` returns `[field][0] = old`, `[1] = new`, and
        // every downstream stage indexes positionally. If this order changed,
        // the diff would compare a build against itself and the negative probes
        // would ask about prefixes they meant to contrast.
        let fields = vec!["game".to_string(), "zh-cn".to_string()];
        let w = wants(
            &fields,
            "7.0.0",
            "7.1.0",
            &build("oldid", "7.0.0"),
            &build("newid", "7.1.0"),
        );
        let shape: Vec<(usize, &str, &str, bool)> = w
            .iter()
            .map(|x| (x.field, x.field_name.as_str(), x.tag.as_str(), x.is_new))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0, "game", "7.0.0", false),
                (0, "game", "7.1.0", true),
                (1, "zh-cn", "7.0.0", false),
                (1, "zh-cn", "7.1.0", true),
            ],
            "two manifests per field, old first, in the caller's field order"
        );
    }

    #[test]
    fn every_field_gets_a_slot_even_when_the_list_is_empty() {
        // Zero fields is a run that cannot proceed, but the shape function must
        // not panic on it and must not invent a slot.
        assert!(wants(&[], "7.0.0", "7.1.0", &build("o", "7.0.0"), &build("n", "7.1.0")).is_empty());
        // One field is the common case and must still produce exactly two.
        assert_eq!(
            wants(
                &["game".to_string()],
                "7.0.0",
                "7.1.0",
                &build("o", "7.0.0"),
                &build("n", "7.1.0")
            )
            .len(),
            2
        );
    }

    #[test]
    fn each_want_carries_the_build_it_was_asked_for() {
        // The two sides come from two different `getBuild` bodies, and a mix-up
        // would fetch the new manifest for the old tag - a URL the CDN answers,
        // with the wrong content.
        let w = wants(
            &["game".to_string()],
            "7.0.0",
            "7.1.0",
            &build("oldbuildid", "7.0.0"),
            &build("newbuildid", "7.1.0"),
        );
        assert_eq!(w[0].build.build_id, "oldbuildid");
        assert_eq!(w[0].build.tag, "7.0.0");
        assert_eq!(w[1].build.build_id, "newbuildid");
        assert_eq!(w[1].build.tag, "7.1.0");
    }

    #[test]
    fn the_field_index_is_the_position_not_the_name() {
        // Filing is `manifests[want.field]`, so the index has to be the
        // position in the caller's list. Two fields with the same name would
        // collide by name; by index they cannot.
        let fields = vec!["game".to_string(), "game".to_string()];
        let w = wants(
            &fields,
            "7.0.0",
            "7.1.0",
            &build("o", "7.0.0"),
            &build("n", "7.1.0"),
        );
        assert_eq!(w.iter().map(|x| x.field).collect::<Vec<_>>(), vec![0, 0, 1, 1]);
    }
}
