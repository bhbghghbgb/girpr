//! Pure placement: suffix → folder, the force-persistent rule, cleaned rels and
//! CDN URLs. A port of Collapse `Fetch.Persistent.cs`.
//!
//! The suffix tests are case-insensitive. `Contains("blocks")` is deliberate —
//! a different mapper in the same file uses `StartsWith`; this is not it.
//! `isPatch`/`category`/`localName` never enter these decisions.

use std::collections::HashMap;

use girpr_dispatch::{DispatchInfo, ManifestKind};

pub use crate::value::{BaseTree, Folder};
use crate::{PlanError, Result};

/// The suffix table, exactly as Collapse places dispatcher rows.
///
/// | `remoteName` | folder |
/// |---|---|
/// | `*.pck` | `AudioAssets` |
/// | contains `blocks` **and** ends `.blk`, or ends `svc_catalog` | `AssetBundles` |
/// | `*.usm` / `*.cuepoint` | `VideoAssets` |
/// | anything else | root (`""`) |
pub fn folder_for(remote_name: &str) -> Folder {
    let lower = remote_name.to_ascii_lowercase();
    if lower.ends_with(".pck") {
        Folder::AudioAssets
    } else if (lower.contains("blocks") && lower.ends_with(".blk"))
        || lower.ends_with("svc_catalog")
    {
        Folder::AssetBundles
    } else if lower.ends_with(".usm") || lower.ends_with(".cuepoint") {
        Folder::VideoAssets
    } else {
        Folder::Root
    }
}

/// What the unfiltered Sophon base map says about one dispatcher rel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceDecision {
    /// The base already has these exact bytes: omit from the download set.
    Omit,
    /// The base has the rel with different MD5: a hotfix, force `Persistent`.
    Persistent,
    /// A new path: keep the suffix-derived placement.
    Absent,
}

/// The case-folded key for comparing rels. Game paths are Windows paths, so
/// ASCII case is folded on every host: the pure decision must be identical
/// whether girpr runs on Windows or elsewhere (unlike `util-path::path_key`,
/// which folds only on Windows for filesystem identity).
pub fn rel_key(rel: &str) -> String {
    rel.to_ascii_lowercase()
}

/// The three-way force rule over one base MD5 (case-insensitive compare).
pub fn force_persistent_decision(base_md5: Option<&str>, row_md5: &str) -> ForceDecision {
    match base_md5 {
        Some(base) if base.eq_ignore_ascii_case(row_md5) => ForceDecision::Omit,
        Some(_) => ForceDecision::Persistent,
        None => ForceDecision::Absent,
    }
}

/// Where a dispatcher row goes, after consulting the base map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementDecision {
    Omit,
    Keep(BaseTree),
}

/// Apply the force rule to a cleaned rel and return the placement.
pub fn placement_for(
    rel: &str,
    row_md5: &str,
    base_md5_by_rel: &HashMap<String, String>,
) -> PlacementDecision {
    let base = base_md5_by_rel
        .iter()
        .find(|(key, _)| rel_key(key) == rel_key(rel))
        .map(|(_, md5)| md5.as_str());
    match force_persistent_decision(base, row_md5) {
        ForceDecision::Omit => PlacementDecision::Omit,
        ForceDecision::Persistent => PlacementDecision::Keep(BaseTree::Persistent),
        ForceDecision::Absent => PlacementDecision::Keep(BaseTree::StreamingAssets),
    }
}

/// Turn `meta.config.exe_file_name` into the `<exe>_Data` directory name.
///
/// The game config spells it `GenshinImpact.exe`; the data directory is
/// `GenshinImpact_Data`. A bare stem is accepted too, but nothing is hardcoded
/// and a stem that could name anything but one directory component is refused.
pub fn data_dir_name(exe_file_name: &str) -> Result<String> {
    let stem = if exe_file_name.len() >= 4
        && exe_file_name
            .get(exe_file_name.len() - 4..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(".exe"))
    {
        &exe_file_name[..exe_file_name.len() - 4]
    } else {
        exe_file_name
    };
    if stem.is_empty() || stem.contains(['/', '\\', ':', '.', ' ']) || stem == ".." {
        return Err(PlanError::ExeName(exe_file_name.into()));
    }
    Ok(format!("{stem}_Data"))
}

/// `rel = <exe>_Data/<base>/<folder>/<remoteName>`, cleaned once so the plan,
/// the scope gate and the filesystem sink all compare the same spelling.
pub fn asset_rel(
    exe_file_name: &str,
    base: BaseTree,
    folder: Folder,
    remote_name: &str,
) -> Result<String> {
    let data = data_dir_name(exe_file_name)?;
    let rel = if folder == Folder::Root {
        format!("{data}/{}/{remote_name}", base.as_str())
    } else {
        format!(
            "{data}/{}/{}/{}",
            base.as_str(),
            folder.as_str(),
            remote_name
        )
    };
    girpr_util_path::clean_rel(&rel).map_err(|e| PlanError::Rel {
        value: rel,
        reason: e.to_string(),
    })
}

/// Assemble the primary and alternate CDN URLs from the **original**
/// `remoteName`, before any rel rewriting.
///
/// Res swaps the audio-assets and game-res prefixes for a hotfix; data and
/// silence use the same prefix for both attempts. The alternate is tried by
/// the downloader on a 404 only — this function only decides the two spellings.
pub fn asset_urls(
    dispatch: &DispatchInfo,
    source: ManifestKind,
    folder: Folder,
    remote_name: &str,
    hotfix: bool,
) -> Result<(String, String)> {
    let (prefix_primary, prefix_alternate, segment): (&str, &str, &str) = match source {
        ManifestKind::Res => (
            &dispatch.client_audio_assets_url,
            &dispatch.client_game_res_url,
            "StandaloneWindows64",
        ),
        ManifestKind::Data => (
            &dispatch.client_design_data_url,
            &dispatch.client_design_data_url,
            "AssetBundles",
        ),
        ManifestKind::Silence => (
            &dispatch.client_silence_data_url,
            &dispatch.client_silence_data_url,
            "AssetBundles",
        ),
    };
    if prefix_primary.is_empty() || prefix_alternate.is_empty() {
        return Err(PlanError::MissingValue("client CDN prefix"));
    }
    let name = remote_name.replace('\\', "/");
    let leaf = if folder == Folder::Root {
        name
    } else {
        format!("{}/{}", folder.as_str(), name)
    };
    let build = |prefix: &str| format!("{}/{segment}/{leaf}", prefix.trim_end_matches('/'));
    // A hotfix swaps the two res prefixes; the other two sources have identical
    // primary and alternate prefixes by design.
    let (primary, alternate) = if hotfix && source == ManifestKind::Res {
        (build(prefix_alternate), build(prefix_primary))
    } else {
        (build(prefix_primary), build(prefix_alternate))
    };
    Ok((primary, alternate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suffix_table_is_case_insensitive_and_uses_contains_blocks() {
        for name in ["a.pck", "A.PCK", "English(US)/VO_TEST.pck"] {
            assert_eq!(folder_for(name), Folder::AudioAssets, "{name}");
        }
        for name in [
            "blocks/00/11111111.blk",
            "BLOCKS/00/11111111.BLK",
            "notblocks.blk",
            "xx_blocks_yy.blk",
            "some/svc_catalog",
            "SVC_CATALOG",
        ] {
            assert_eq!(folder_for(name), Folder::AssetBundles, "{name}");
        }
        for name in ["intro.usm", "A.CUEPOINT", "clip.cuepoint"] {
            assert_eq!(folder_for(name), Folder::VideoAssets, "{name}");
        }
        // A `.blk` without `blocks`, and `blocks` without `.blk`, are root rows.
        // The other Collapse mapper uses StartsWith; that is not this rule.
        for name in ["other/00/11111111.blk", "blocks/00/x.dat", "config.json"] {
            assert_eq!(folder_for(name), Folder::Root, "{name}");
        }
    }

    fn map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn force_rule_same_different_and_missing_base() {
        let md5 = "b2d3f56bc197fd985d5965079b5e7148";
        let same = map(&[("GenshinImpact_Data/Persistent/blocks/00/a.blk", md5)]);
        let different = map(&[(
            "GenshinImpact_Data/Persistent/blocks/00/a.blk",
            "5117f4269ca5456f8dd0e47ddd3e0832",
        )]);
        let absent = map(&[("some/other/path", md5)]);
        let rel = "GenshinImpact_Data/Persistent/blocks/00/a.blk";

        assert_eq!(
            force_persistent_decision(Some(md5), &md5.to_uppercase()),
            ForceDecision::Omit,
            "MD5 comparison is case-insensitive"
        );
        assert_eq!(
            placement_for(rel, md5, &same),
            PlacementDecision::Omit,
            "identical base bytes are not downloaded again"
        );
        assert_eq!(
            placement_for(rel, md5, &different),
            PlacementDecision::Keep(BaseTree::Persistent),
            "a different MD5 forces Persistent regardless of isPatch"
        );
        assert_eq!(
            placement_for(rel, md5, &absent),
            PlacementDecision::Keep(BaseTree::StreamingAssets),
            "a new path keeps its suffix-derived placement"
        );
        assert_eq!(
            placement_for(&rel.to_uppercase(), md5, &different),
            PlacementDecision::Keep(BaseTree::Persistent),
            "rel lookup folds ASCII case like the Windows game tree"
        );
    }

    #[test]
    fn rel_uses_the_parameter_exe_and_the_suffix_folder() {
        let remote = "blocks/00/11111111.blk";
        let folder = folder_for(remote);
        assert_eq!(
            asset_rel(
                "GenshinImpact.exe",
                BaseTree::StreamingAssets,
                folder,
                remote
            )
            .unwrap(),
            "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/11111111.blk"
        );
        assert_eq!(
            asset_rel("GenshinImpact.exe", BaseTree::Persistent, folder, remote).unwrap(),
            "GenshinImpact_Data/Persistent/AssetBundles/blocks/00/11111111.blk"
        );
        assert_eq!(
            asset_rel("YuanShen.exe", BaseTree::Persistent, folder, remote).unwrap(),
            "YuanShen_Data/Persistent/AssetBundles/blocks/00/11111111.blk"
        );
        assert_eq!(
            asset_rel(
                "GenshinImpact.exe",
                BaseTree::Persistent,
                folder_for("a.pck"),
                "VO.pck"
            )
            .unwrap(),
            "GenshinImpact_Data/Persistent/AudioAssets/VO.pck"
        );
        assert_eq!(
            asset_rel(
                "GenshinImpact.exe",
                BaseTree::Persistent,
                folder_for("x.json"),
                "x.json"
            )
            .unwrap(),
            "GenshinImpact_Data/Persistent/x.json"
        );
        assert!(
            asset_rel("", BaseTree::Persistent, folder, remote).is_err(),
            "the exe stem is a parameter, never a hardcoded default"
        );
    }

    #[test]
    fn exe_stem_validation_refuses_paths() {
        for name in ["", "dir/game.exe", "dir\\game.exe", "..", "C:game.exe"] {
            assert!(data_dir_name(name).is_err(), "{name}");
        }
        assert_eq!(
            data_dir_name("GenshinImpact.exe").unwrap(),
            "GenshinImpact_Data"
        );
        assert_eq!(data_dir_name("YuanShen.EXE").unwrap(), "YuanShen_Data");
        assert_eq!(data_dir_name("YuanShen").unwrap(), "YuanShen_Data");
    }

    fn dispatch() -> DispatchInfo {
        DispatchInfo {
            game_version: "9.9.0".into(),
            channel_name: "OSRELWin".into(),
            server_name: "os_asia".into(),
            codename: "hk4e_global".into(),
            res_revision: 1,
            data_revision: 2,
            silence_revision: 3,
            audio_revision: 4,
            res_hash: "aa".into(),
            data_hash: "bb".into(),
            silence_hash: "cc".into(),
            audio_hash: "dd".into(),
            client_game_res_url: "https://cdn.test/res/output_1_aa/client".into(),
            client_design_data_url: "https://cdn.test/data/output_2_bb/client/General".into(),
            client_silence_data_url: "https://cdn.test/data/output_3_cc/client_silence/General"
                .into(),
            client_audio_assets_url: "https://cdn.test/res/output_4_dd/client".into(),
            // The descriptors themselves are not read by the URL rule.
            manifest_digests: std::array::from_fn(|_| girpr_dispatch::ManifestDescriptor {
                kind: ManifestKind::Res,
                url: String::new(),
                expected_md5: String::new(),
                expected_size: 0,
                mirror_rel: String::new(),
            }),
        }
    }

    #[test]
    fn res_primary_and_alternate_swap_only_for_hotfixes() {
        let folder = folder_for("blocks/00/a.blk");
        let remote = "blocks/00/a.blk";
        let base = asset_urls(&dispatch(), ManifestKind::Res, folder, remote, false).unwrap();
        let hotfix = asset_urls(&dispatch(), ManifestKind::Res, folder, remote, true).unwrap();
        assert_eq!(
            base,
            (
                "https://cdn.test/res/output_4_dd/client/StandaloneWindows64/AssetBundles/blocks/00/a.blk".into(),
                "https://cdn.test/res/output_1_aa/client/StandaloneWindows64/AssetBundles/blocks/00/a.blk".into(),
            )
        );
        assert_eq!(hotfix, (base.1.clone(), base.0.clone()));
    }

    #[test]
    fn data_and_silence_keep_their_single_prefix_and_use_assetbundles() {
        let d = dispatch();
        // The CDN prefix carries its own `AssetBundles` segment (the manifest
        // lives under it); the suffix folder adds another one for `.blk` rows.
        let folder = folder_for("blocks/a.blk");
        assert_eq!(folder, Folder::AssetBundles);
        for (source, prefix) in [
            (ManifestKind::Data, d.client_design_data_url.as_str()),
            (ManifestKind::Silence, d.client_silence_data_url.as_str()),
        ] {
            for hotfix in [false, true] {
                let urls = asset_urls(&d, source, folder, "blocks/a.blk", hotfix).unwrap();
                let expected = format!("{prefix}/AssetBundles/AssetBundles/blocks/a.blk");
                assert_eq!(
                    urls,
                    (expected.clone(), expected),
                    "{source:?}, hotfix={hotfix}"
                );
            }
        }
    }

    #[test]
    fn urls_use_the_original_remote_name_and_trim_a_trailing_slash() {
        let mut d = dispatch();
        d.client_game_res_url.push('/');
        let urls = asset_urls(
            &d,
            ManifestKind::Res,
            Folder::Root,
            "hardware_model_config.json",
            true,
        )
        .unwrap();
        assert_eq!(
            urls.0,
            "https://cdn.test/res/output_1_aa/client/StandaloneWindows64/hardware_model_config.json"
        );
        assert!(!urls.0.contains("Persistent"), "no rel rewriting in a URL");
        assert!(!urls.0.contains("//StandaloneWindows64"), "one slash only");
    }
}
