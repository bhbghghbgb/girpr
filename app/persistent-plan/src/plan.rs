//! The fold that produces a [`PersistentPlan`] from already-fetched values.
//!
//! No I/O: raw manifest bytes arrive from the fetch module (already verified
//! against their descriptors), rows are parsed by [`crate::manifest`], and the
//! caller supplies the unfiltered Sophon `clean_rel -> md5` map and the bytes
//! of `StreamingAssets/res_versions_streaming` if that file exists.

use std::collections::HashMap;

use girpr_dispatch::{DispatchInfo, ManifestDescriptor};

use crate::marker::render_markers;
use crate::placement::{asset_rel, asset_urls, folder_for, placement_for, rel_key};
use crate::value::{BaseTree, PersistentAsset, PersistentPlan, PersistentRow};
use crate::{PlanError, Result};

/// One fetched-and-verified manifest, with its parsed rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestInput {
    pub descriptor: ManifestDescriptor,
    /// Raw bytes, mirrored verbatim by the post phase.
    pub raw: Vec<u8>,
    pub rows: Vec<PersistentRow>,
}

/// Build the complete inspectable plan.
///
/// The force rule runs against the *pre-scope* base map: a scoped run may not
/// see the base file, but a dispatcher row that collides with it is still a
/// hotfix. Scope filtering of the resulting assets is the caller's next step.
pub fn build_plan(
    exe_file_name: &str,
    dispatch: &DispatchInfo,
    base_md5_by_rel: &HashMap<String, String>,
    manifests: &[ManifestInput],
    res_versions_streaming: Option<&[u8]>,
) -> Result<PersistentPlan> {
    // Validate the exe parameter once, before any decision uses it.
    crate::placement::data_dir_name(exe_file_name)?;

    let mut assets: Vec<PersistentAsset> = Vec::new();
    for input in manifests {
        if input.descriptor.mirror_rel.is_empty() {
            return Err(PlanError::MissingValue("descriptor mirror_rel"));
        }
        for row in &input.rows {
            let rel = asset_rel(
                exe_file_name,
                // The folder is derived from the original remote name; the
                // rel is cleaned only once, inside `asset_rel`.
                BaseTree::StreamingAssets,
                folder_for(&row.remote_name),
                &row.remote_name,
            )?;
            // The base map is keyed by the *default* rel: Collapse compares the
            // dispatcher row's path (base tree included) before forcing it to
            // Persistent.
            let placement = match placement_for(&rel, &row.md5, base_md5_by_rel) {
                crate::placement::PlacementDecision::Omit => continue,
                crate::placement::PlacementDecision::Keep(base) => base,
            };
            let rel = asset_rel(
                exe_file_name,
                placement,
                folder_for(&row.remote_name),
                &row.remote_name,
            )?;
            let (primary_url, alternate_url) = asset_urls(
                dispatch,
                input.descriptor.kind,
                folder_for(&row.remote_name),
                &row.remote_name,
                placement == BaseTree::Persistent,
            )?;
            assets.push(PersistentAsset {
                rel,
                placement,
                size: row.file_size,
                md5: row.md5.clone(),
                hash: row.hash,
                primary_url,
                alternate_url,
                source: input.descriptor.kind,
            });
        }
    }

    // First row for a rel wins; the sort makes the winner deterministic when
    // two manifests name the same cleaned path.
    assets.sort_by(|a, b| a.rel.cmp(&b.rel));
    let mut seen = std::collections::HashSet::new();
    assets.retain(|a| seen.insert(rel_key(&a.rel)));
    let total_bytes = assets
        .iter()
        .fold(0u64, |sum, a| sum.saturating_add(a.size));

    let mut mirrors: Vec<(String, Vec<u8>)> = manifests
        .iter()
        .map(|input| {
            let rel = girpr_util_path::clean_rel(&input.descriptor.mirror_rel).map_err(|e| {
                PlanError::Rel {
                    value: input.descriptor.mirror_rel.clone(),
                    reason: e.to_string(),
                }
            })?;
            Ok((rel, input.raw.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    mirrors.sort_by(|a, b| a.0.cmp(&b.0));

    let markers = render_markers(exe_file_name, dispatch, res_versions_streaming)?;
    let marker_paths: Vec<String> = markers.iter().map(|m| m.path.clone()).collect();

    let mut expected: Vec<String> = assets
        .iter()
        .map(|a| a.rel.clone())
        .chain(mirrors.iter().map(|(rel, _)| rel.clone()))
        .chain(marker_paths.iter().cloned())
        .collect();
    expected.sort();
    let mut seen_rels = std::collections::HashSet::new();
    expected.retain(|rel| seen_rels.insert(rel_key(rel)));

    Ok(PersistentPlan {
        assets,
        mirrors,
        marker_paths,
        expected_rels: expected,
        total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ManifestKind, parse_rows};

    const RES: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/res.ndjson");
    const DATA: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/data.ndjson");
    const SILENCE: &[u8] =
        include_bytes!("../../../tests/fixtures/persistent_inputs/samples/fixtures/silence.ndjson");

    fn dispatch() -> DispatchInfo {
        DispatchInfo {
            game_version: "9.9.0".into(),
            channel_name: "OSRELWin".into(),
            server_name: "os_asia".into(),
            codename: "hk4e_global".into(),
            res_revision: 99000001,
            data_revision: 99000002,
            silence_revision: 99000003,
            audio_revision: 99000004,
            res_hash: "aabbccddeeff".into(),
            data_hash: "112233445566".into(),
            silence_hash: "ffeeddccbbaa".into(),
            audio_hash: "001122334455".into(),
            client_game_res_url: "https://mock.girpr.test/output_1_aabbccddeeff/client".into(),
            client_design_data_url: "https://mock.girpr.test/output_2_112233445566/client/General"
                .into(),
            client_silence_data_url:
                "https://mock.girpr.test/output_3_ffeeddccbbaa/client_silence/General".into(),
            client_audio_assets_url: "https://mock.girpr.test/output_4_001122334455/client".into(),
            manifest_digests: std::array::from_fn(|_| ManifestDescriptor {
                kind: ManifestKind::Res,
                url: String::new(),
                expected_md5: String::new(),
                expected_size: 0,
                mirror_rel: String::new(),
            }),
        }
    }

    fn descriptor(kind: ManifestKind) -> ManifestDescriptor {
        ManifestDescriptor {
            kind,
            url: String::new(),
            expected_md5: String::new(),
            expected_size: 0,
            mirror_rel: format!(
                "GenshinImpact_Data/Persistent/{}_versions_persist",
                kind.as_str()
            ),
        }
    }

    fn inputs() -> Vec<ManifestInput> {
        vec![
            ManifestInput {
                descriptor: descriptor(ManifestKind::Res),
                raw: RES.to_vec(),
                rows: parse_rows(RES).unwrap(),
            },
            ManifestInput {
                descriptor: descriptor(ManifestKind::Data),
                raw: DATA.to_vec(),
                rows: parse_rows(DATA).unwrap(),
            },
            ManifestInput {
                descriptor: descriptor(ManifestKind::Silence),
                raw: SILENCE.to_vec(),
                rows: parse_rows(SILENCE).unwrap(),
            },
        ]
    }

    #[test]
    fn fixtures_fold_into_a_sorted_deduped_plan() {
        let base = HashMap::new();
        let plan = build_plan("GenshinImpact.exe", &dispatch(), &base, &inputs(), None).unwrap();
        // Five fixture rows, all new paths: StreamingAssets, suffix folders.
        assert_eq!(plan.assets.len(), 5);
        assert!(plan.assets.windows(2).all(|w| w[0].rel <= w[1].rel));
        assert!(
            plan.assets
                .iter()
                .all(|a| a.placement == BaseTree::StreamingAssets)
        );
        assert_eq!(plan.total_bytes, 64 + 48 + 48 + 48 + 64);
        assert_eq!(plan.mirrors.len(), 3);
        assert!(plan.mirrors.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(
            plan.mirrors
                .iter()
                .find(|(rel, _)| rel.contains("silence_data_versions_persist"))
                .unwrap()
                .1,
            SILENCE
        );
        assert_eq!(plan.marker_paths.len(), 8);
        assert_eq!(plan.expected_rels.len(), 5 + 3 + 8);
        assert!(
            plan.rels()
                .all(|rel| rel.starts_with("GenshinImpact_Data/"))
        );

        let blk = plan
            .assets
            .iter()
            .find(|a| a.rel.contains("11111111.blk"))
            .unwrap();
        assert_eq!(
            blk.rel,
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk"
        );
        assert!(
            blk.primary_url
                .contains("StandaloneWindows64/AssetBundles/")
        );
        assert!(blk.primary_url.contains("output_4_001122334455"));
        assert!(blk.alternate_url.contains("output_1_aabbccddeeff"));

        // Mirrors keep the exact verified bytes, including the blank line in
        // data.ndjson.
        let data_mirror = plan
            .mirrors
            .iter()
            .find(|(rel, _)| rel.contains("data_versions_persist"))
            .unwrap();
        assert_eq!(data_mirror.1, DATA);
    }

    #[test]
    fn base_collisions_force_persistent_or_omit_a_row() {
        let mut base = HashMap::new();
        // Different MD5 at the colliding rel: forced to Persistent.
        base.insert(
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk".into(),
            "00000000000000000000000000000000".into(),
        );
        // Equal MD5 (case-insensitive): omitted from the download set.
        base.insert(
            "genshinimpact_data/streamingassets/audioassets/english(us)/vo_test.pck".into(),
            "5117F4269CA5456F8DD0E47DDD3E0832".into(),
        );
        let plan = build_plan("GenshinImpact.exe", &dispatch(), &base, &inputs(), None).unwrap();
        // One equal-MD5 row is omitted; the different-MD5 row stays as a
        // forced hotfix; the three new paths stay in StreamingAssets.
        assert_eq!(plan.assets.len(), 4);
        assert!(plan.assets.iter().all(|a| !a.rel.contains("VO_TEST.pck")));
        let forced = plan
            .assets
            .iter()
            .find(|a| a.rel.contains("11111111.blk"))
            .unwrap();
        assert_eq!(forced.placement, BaseTree::Persistent);
        assert!(forced.rel.contains("/Persistent/AssetBundles/"));
        // A hotfix res row swaps the two res prefixes.
        assert!(forced.primary_url.contains("output_1_aabbccddeeff"));
        assert!(forced.alternate_url.contains("output_4_001122334455"));
    }

    #[test]
    fn mirror_and_exe_inputs_are_validated() {
        let mut inputs = inputs();
        inputs[0].descriptor.mirror_rel.clear();
        assert_eq!(
            build_plan(
                "GenshinImpact.exe",
                &dispatch(),
                &HashMap::new(),
                &inputs,
                None
            )
            .unwrap_err(),
            PlanError::MissingValue("descriptor mirror_rel")
        );
        assert!(build_plan("", &dispatch(), &HashMap::new(), &inputs, None).is_err());
    }
}
