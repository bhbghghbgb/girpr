//! Marker bytes and the C7 scope predicates (docs/03 §7.4.3, §7.5.1 item 4).
//!
//! Every value is plain text with **no trailing newline**. The whole block is
//! written last, after a successful execute pass, by the post phase; this
//! module only decides the paths and the bytes. `base_res_version_hash` has an
//! explicit omit branch: when its source is absent, no marker is produced and
//! therefore no path is expected or published.

use girpr_digest::md5_hex;
use girpr_dispatch::DispatchInfo;

use crate::Result;
use crate::placement::{BaseTree, asset_rel};
use crate::value::{MarkerSet, PersistentAsset, PersistentPlan};

/// The marker that describes the streaming resource file, when that file
/// exists. Its value is the lowercase-hex MD5 of the source bytes.
pub fn base_res_version_hash_marker(
    exe_file_name: &str,
    res_versions_streaming: &[u8],
) -> Result<MarkerSet> {
    Ok(MarkerSet {
        path: asset_rel(
            exe_file_name,
            BaseTree::Persistent,
            crate::Folder::Root,
            "base_res_version_hash",
        )?,
        value: md5_hex(res_versions_streaming),
    })
}

/// Render the full marker block in its pinned order.
///
/// `res_versions_streaming: None` is the C16 omit branch: `base_res_version_hash`
/// is left out entirely rather than written with a placeholder digest.
pub fn render_markers(
    exe_file_name: &str,
    dispatch: &DispatchInfo,
    res_versions_streaming: Option<&[u8]>,
) -> Result<Vec<MarkerSet>> {
    let marker = |name: &str, value: String| -> Result<MarkerSet> {
        Ok(MarkerSet {
            path: asset_rel(
                exe_file_name,
                BaseTree::Persistent,
                crate::Folder::Root,
                name,
            )?,
            value,
        })
    };
    let mut out = Vec::with_capacity(9);
    if let Some(bytes) = res_versions_streaming {
        out.push(base_res_version_hash_marker(exe_file_name, bytes)?);
    }
    out.push(marker("res_revision", dispatch.res_revision.to_string())?);
    // HoYo's own typo for "external": both files carry the same dispatcher
    // value. An external check reads them together, so writing one alone is
    // not enough.
    out.push(marker(
        "res_revision_eternal",
        dispatch.res_revision.to_string(),
    )?);
    out.push(marker("data_revision", dispatch.data_revision.to_string())?);
    out.push(marker(
        "silence_revision",
        dispatch.silence_revision.to_string(),
    )?);
    out.push(marker(
        "audio_revision",
        dispatch.audio_revision.to_string(),
    )?);
    // Collapse parity, with no documented reader: pinned by the byte tests.
    out.push(marker("PatchDone", dispatch.audio_revision.to_string())?);
    out.push(marker("ChannelName", dispatch.channel_name.clone())?);
    out.push(marker("ScriptVersion", dispatch.game_version.clone())?);
    Ok(out)
}

/// First marker the active path scope hides, if any (C7(d)).
pub fn first_out_of_scope_marker<'a>(
    markers: &'a [MarkerSet],
    visible: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    markers
        .iter()
        .find(|m| !visible(&m.path))
        .map(|m| m.path.as_str())
}

/// First **mirror** the active path scope hides, if any (C7(d)).
///
/// The post phase writes mirrors as well as markers, so the usage guard must
/// cover every rel it publishes, not just the rendered marker paths.
pub fn first_out_of_scope_mirror<'a>(
    mirrors: &'a [(String, Vec<u8>)],
    visible: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    mirrors
        .iter()
        .find(|(rel, _)| !visible(rel))
        .map(|(rel, _)| rel.as_str())
}

/// First **hotfix** asset the active path scope hides, if any (C7(f)).
///
/// A new-path row in `StreamingAssets` does not carry a revision claim, so it
/// does not suppress the marker block; only forced-persistent paths do.
pub fn first_out_of_scope_hotfix<'a>(
    assets: &'a [PersistentAsset],
    visible: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    assets
        .iter()
        .find(|a| a.placement == BaseTree::Persistent && !visible(&a.rel))
        .map(|a| a.rel.as_str())
}

/// The combined C7 decision: a marker or mirror outside the scope is a usage
/// error, because the post phase writes both; a hotfix path outside the scope
/// suppresses markers and mirrors while the in-scope files are still repaired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeDecision<'a> {
    Proceed,
    MarkersOutOfScope(&'a str),
    MirrorOutOfScope(&'a str),
    HotfixOutOfScope(&'a str),
}

/// Apply the scope guards in their pinned order.
///
/// Every file the post phase writes is covered: rendered markers first, then
/// mirrors, then the hotfix assets whose partial install would make a revision
/// claim false.
pub fn scope_decision<'a>(
    plan: &'a PersistentPlan,
    markers: &'a [MarkerSet],
    visible: impl Fn(&str) -> bool,
) -> ScopeDecision<'a> {
    if let Some(path) = first_out_of_scope_marker(markers, &visible) {
        ScopeDecision::MarkersOutOfScope(path)
    } else if let Some(path) = first_out_of_scope_mirror(&plan.mirrors, &visible) {
        ScopeDecision::MirrorOutOfScope(path)
    } else if let Some(path) = first_out_of_scope_hotfix(&plan.assets, &visible) {
        ScopeDecision::HotfixOutOfScope(path)
    } else {
        ScopeDecision::Proceed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::asset_urls;
    use crate::{ManifestKind, parse_rows};
    use girpr_dispatch::ManifestDescriptor;

    fn dispatch() -> DispatchInfo {
        DispatchInfo {
            game_version: "9.9.0".into(),
            channel_name: "OSRELWin".into(),
            server_name: "os_asia".into(),
            codename: "hk4e_global".into(),
            res_revision: 49530578,
            data_revision: 1234,
            silence_revision: 5678,
            audio_revision: 9012,
            res_hash: "aa".into(),
            data_hash: "bb".into(),
            silence_hash: "cc".into(),
            audio_hash: "dd".into(),
            client_game_res_url: "https://cdn.test/res/client".into(),
            client_design_data_url: "https://cdn.test/data/client/General".into(),
            client_silence_data_url: "https://cdn.test/data/client_silence/General".into(),
            client_audio_assets_url: "https://cdn.test/res2/client".into(),
            manifest_digests: std::array::from_fn(|_| ManifestDescriptor {
                kind: ManifestKind::Res,
                url: String::new(),
                expected_md5: String::new(),
                expected_size: 0,
                mirror_rel: String::new(),
            }),
        }
    }

    #[test]
    fn marker_bytes_are_plain_text_with_no_trailing_newline() {
        let source = b"res_versions_streaming bytes";
        let markers = render_markers("GenshinImpact.exe", &dispatch(), Some(source)).unwrap();
        let values: Vec<(&str, &str)> = markers
            .iter()
            .map(|m| {
                (
                    m.path.rsplit('/').next().unwrap_or_default(),
                    m.value.as_str(),
                )
            })
            .collect();
        assert_eq!(
            values,
            [
                (
                    "base_res_version_hash",
                    girpr_digest::md5_hex(source).as_str()
                ),
                ("res_revision", "49530578"),
                ("res_revision_eternal", "49530578"),
                ("data_revision", "1234"),
                ("silence_revision", "5678"),
                ("audio_revision", "9012"),
                ("PatchDone", "9012"),
                ("ChannelName", "OSRELWin"),
                ("ScriptVersion", "9.9.0"),
            ]
        );
        for m in &markers {
            assert!(
                !m.value.ends_with(['\n', '\r']),
                "{} has no newline",
                m.path
            );
            assert!(
                m.path.starts_with("GenshinImpact_Data/Persistent/"),
                "{}",
                m.path
            );
        }
        // The eternal twin is written from the same dispatcher value, never
        // from the base revision or the MD5.
        assert_eq!(markers[2].value, markers[1].value);
        assert_eq!(markers[6].value, markers[5].value);
    }

    #[test]
    fn missing_streaming_source_omits_only_the_hash_marker() {
        let markers = render_markers("YuanShen.exe", &dispatch(), None).unwrap();
        assert_eq!(markers.len(), 8);
        assert!(
            markers
                .iter()
                .all(|m| !m.path.contains("base_res_version_hash"))
        );
        assert_eq!(markers[0].value, "49530578");
        assert_eq!(
            base_res_version_hash_marker("YuanShen.exe", b"x")
                .unwrap()
                .value,
            girpr_digest::md5_hex(b"x")
        );
    }

    fn asset(rel: &str, placement: BaseTree) -> PersistentAsset {
        let (primary, alternate) = asset_urls(
            &dispatch(),
            ManifestKind::Res,
            crate::Folder::Root,
            "hotfix.dat",
            placement == BaseTree::Persistent,
        )
        .unwrap();
        PersistentAsset {
            rel: rel.into(),
            placement,
            size: 1,
            md5: String::new(),
            hash: None,
            primary_url: primary,
            alternate_url: alternate,
            source: ManifestKind::Res,
        }
    }

    #[test]
    fn scope_predicates_distinguish_markers_from_hotfix_paths() {
        let markers = render_markers("GenshinImpact.exe", &dispatch(), None).unwrap();
        let plan = PersistentPlan {
            assets: vec![
                asset(
                    "GenshinImpact_Data/Persistent/hotfix.dat",
                    BaseTree::Persistent,
                ),
                asset(
                    "GenshinImpact_Data/StreamingAssets/new.dat",
                    BaseTree::StreamingAssets,
                ),
            ],
            mirrors: vec![(
                "GenshinImpact_Data/Persistent/res_versions_persist".into(),
                b"raw manifest".to_vec(),
            )],
            marker_paths: markers.iter().map(|m| m.path.clone()).collect(),
            expected_rels: vec![],
            total_bytes: 2,
        };
        let all = |_: &str| true;
        assert_eq!(scope_decision(&plan, &markers, all), ScopeDecision::Proceed);

        // A hidden marker is a usage error even when a hotfix is also hidden.
        let hide_marker = |rel: &str| !rel.contains("PatchDone");
        assert_eq!(
            scope_decision(&plan, &markers, hide_marker),
            ScopeDecision::MarkersOutOfScope("GenshinImpact_Data/Persistent/PatchDone")
        );

        // A hidden hotfix path suppresses; a hidden StreamingAssets new path
        // alone does not.
        let hide_hotfix = |rel: &str| !rel.contains("Persistent/hotfix.dat");
        assert_eq!(
            scope_decision(&plan, &markers, hide_hotfix),
            ScopeDecision::HotfixOutOfScope("GenshinImpact_Data/Persistent/hotfix.dat")
        );
        // A hidden mirror is a usage error too: post writes it just like a
        // marker, and an out-of-scope write is never allowed.
        let hide_mirror = |rel: &str| !rel.contains("_versions_persist");
        assert_eq!(
            scope_decision(&plan, &markers, hide_mirror),
            ScopeDecision::MirrorOutOfScope("GenshinImpact_Data/Persistent/res_versions_persist")
        );
        let hide_new = |rel: &str| !rel.contains("StreamingAssets/new.dat");
        assert_eq!(
            scope_decision(&plan, &markers, hide_new),
            ScopeDecision::Proceed
        );
        assert_eq!(
            first_out_of_scope_marker(&markers, |_: &str| false),
            Some(markers[0].path.as_str())
        );
        assert_eq!(
            first_out_of_scope_mirror(&plan.mirrors, |_: &str| false),
            Some("GenshinImpact_Data/Persistent/res_versions_persist")
        );
    }

    #[test]
    fn fixture_rows_keep_the_server_keys_out_of_the_value_types() {
        let bytes =
            include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/res.ndjson");
        let rows = parse_rows(bytes).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].hash.is_some());
    }
}
