//! Filter manifests: drop res_category ignores + non-selected audio fields (Starward parity).
//! NOTE(starward-parity): the `len is 5 or 10 && contains('-')` audio heuristic
//! (`zh-cn` / `mini-zh-cn`) and the audio re-add loop that bypasses the ignore
//! set are both verbatim ports of `GetAvailableGameSophonChunkManifests` — the
//! heuristic is fragile upstream, but diverging here would break parity.
//! Do not "fix" without checking upstream first.
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L742-L769>
//! Audio fields look like `zh-cn` (len 5 with '-') or `mini-zh-cn` (len 10 with '-').
use std::collections::HashSet;

use crate::hyp::{ChunkBuild, ChunkManifestMeta, PatchBuild, PatchManifestMeta};

/// The shared keep predicate, before selected audio is re-added.
///
/// Both manifest families carry the same two ignore keys (`matching_field` and
/// `category_id`) and the same audio heuristic. Keeping the decision here is
/// what stops the chunk and patch filters from drifting: Starward applies the
/// same `GetAvaliable…Manifests` rule to both, including the audio re-add loop
/// that deliberately bypasses the ignore set.
fn keep_manifest(field: &str, category_id: &str, ignore: &HashSet<String>) -> bool {
    if ignore.contains(field) || ignore.contains(category_id) {
        return false;
    }
    // audio/mini -> re-add only if selected
    !((field.len() == 5 || field.len() == 10) && field.contains('-'))
}

/// Run the one selection rule over either manifest family.
///
/// The two DTOs share no trait, so `key` projects the two fields the rule reads;
/// the pass that re-adds selected audio is the same in both cases too.
fn select_by<'a, T>(
    manifests: &'a [T],
    key: impl Fn(&'a T) -> (&'a str, &'a str),
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Vec<&'a T> {
    let mut out: Vec<&T> = Vec::new();
    for m in manifests {
        let (field, category_id) = key(m);
        if keep_manifest(field, category_id, ignore) {
            out.push(m);
        }
    }
    for m in manifests {
        let (field, _) = key(m);
        if audio_langs.contains(field) {
            out.push(m);
        }
    }
    out
}

/// Filter chunk manifests: drop res_category ignores + non-selected audio fields (Starward parity).
pub fn select_manifests<'a>(
    build: &'a ChunkBuild,
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Vec<&'a ChunkManifestMeta> {
    select_by(
        &build.manifests,
        |m| (m.matching_field.as_str(), m.category_id.as_str()),
        audio_langs,
        ignore,
    )
}

/// Filter patch manifests with exactly the rule [`select_manifests`] uses.
///
/// NOTE(starward-parity): Starward's `GetAvaliableGameSophonPatchManifests` is
/// the same loop as its chunk-manifest counterpart, so both DTOs go through
/// [`select_by`]. `diff_download` is never part of the selection.
pub fn select_patch_manifests<'a>(
    build: &'a PatchBuild,
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Vec<&'a PatchManifestMeta> {
    select_by(
        &build.manifests,
        |m| (m.matching_field.as_str(), m.category_id.as_str()),
        audio_langs,
        ignore,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyp::{
        ChunkBuild, ChunkManifestMeta, ManifestFile, ManifestUrl, PatchBuild, PatchManifestMeta,
    };
    use std::collections::HashSet;

    fn meta(field: &str) -> ChunkManifestMeta {
        ChunkManifestMeta {
            category_id: field.to_string(),
            category_name: field.to_string(),
            matching_field: field.to_string(),
            manifest: ManifestFile::default(),
            chunk_download: ManifestUrl::default(),
            manifest_download: ManifestUrl::default(),
        }
    }

    fn patch_meta(field: &str) -> PatchManifestMeta {
        PatchManifestMeta {
            category_id: field.to_string(),
            category_name: field.to_string(),
            matching_field: field.to_string(),
            manifest: ManifestFile::default(),
            diff_download: ManifestUrl::default(),
            manifest_download: ManifestUrl::default(),
        }
    }

    #[test]
    fn manifest_filter_audio_and_ignore() {
        let build = ChunkBuild {
            build_id: "b".into(),
            tag: "5.0".into(),
            manifests: vec![meta("game"), meta("en-us"), meta("ja-jp"), meta("10302")],
        };
        let audio: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        let ignore: HashSet<String> = ["10302".to_string()].into_iter().collect();
        let sel = select_manifests(&build, &audio, &ignore);
        let fields: Vec<_> = sel.iter().map(|m| m.matching_field.as_str()).collect();
        assert_eq!(fields, vec!["game", "en-us"]);
    }

    /// The patch filter must mirror the chunk filter above: game + selected
    /// audio, unselected audio dropped, ignored category dropped. This pins the
    /// shared predicate used by the patch-delete union.
    #[test]
    fn patch_manifest_filter_matches_the_chunk_filter() {
        let build = PatchBuild {
            build_id: "b".into(),
            patch_id: "p".into(),
            tag: "5.0".into(),
            manifests: vec![
                patch_meta("game"),
                patch_meta("en-us"),
                patch_meta("ja-jp"),
                patch_meta("10302"),
            ],
        };
        let audio: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        let ignore: HashSet<String> = ["10302".to_string()].into_iter().collect();
        let sel = select_patch_manifests(&build, &audio, &ignore);
        let fields: Vec<_> = sel.iter().map(|m| m.matching_field.as_str()).collect();
        assert_eq!(fields, vec!["game", "en-us"]);

        // Same rule even when the selected audio field is also in the ignore
        // set: the re-add loop bypasses ignores in both families (Starward parity).
        let audio: HashSet<String> = ["en-us".to_string()].into_iter().collect();
        let ignore: HashSet<String> = ["en-us".to_string(), "10302".to_string()]
            .into_iter()
            .collect();
        let sel = select_patch_manifests(&build, &audio, &ignore);
        let fields: Vec<_> = sel.iter().map(|m| m.matching_field.as_str()).collect();
        assert_eq!(fields, vec!["game", "en-us"]);
    }
}
