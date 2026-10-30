//! Filter manifests: drop res_category ignores + non-selected audio fields (Starward parity).
//! NOTE(starward-parity): the `len is 5 or 10 && contains('-')` audio heuristic
//! (`zh-cn` / `mini-zh-cn`) and the audio re-add loop that bypasses the ignore
//! set are both verbatim ports of `GetAvailableGameSophonChunkManifests` — the
//! heuristic is fragile upstream, but diverging here would break parity.
//! Do not "fix" without checking upstream first.
//! See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L742-L769>
//! Audio fields look like `zh-cn` (len 5 with '-') or `mini-zh-cn` (len 10 with '-').
use std::collections::HashSet;

use crate::hyp::{ChunkBuild, ChunkManifestMeta};

/// Filter manifests: drop res_category ignores + non-selected audio fields (Starward parity).
pub fn select_manifests<'a>(
    build: &'a ChunkBuild,
    audio_langs: &HashSet<String>,
    ignore: &HashSet<String>,
) -> Vec<&'a ChunkManifestMeta> {
    let mut out: Vec<&ChunkManifestMeta> = Vec::new();
    for m in &build.manifests {
        if ignore.contains(&m.matching_field) || ignore.contains(&m.category_id) {
            continue;
        }
        let f = m.matching_field.as_str();
        if (f.len() == 5 || f.len() == 10) && f.contains('-') {
            continue; // audio/mini -> re-add only if selected
        }
        out.push(m);
    }
    for m in &build.manifests {
        if audio_langs.contains(&m.matching_field) {
            out.push(m);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyp::{ChunkBuild, ChunkManifestMeta, ManifestFile, ManifestUrl};
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
}
