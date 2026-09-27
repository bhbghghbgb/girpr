use std::collections::{HashMap, HashSet};

use crate::sophon::{SophonChunk, SophonChunkFile};

/// The work list handed to the repair and files-cleanup phases: every latest
/// file, once, with the chunk URL prefix of the manifest it came from.
pub struct RepairPlan {
    pub latest: String,
    pub files: Vec<PlannedFile>,
    pub url_prefix_by_file: HashMap<String, String>,
}

pub struct PlannedFile {
    pub rel: String, // forward-slash rel path
    pub size: i64,
    pub md5: String,
    pub chunks: Vec<SophonChunk>,
}

/// Build sorted, deduped, blacklist-filtered plan. `per_manifest`: (chunk_url_prefix, files).
pub fn build_plan(
    per_manifest: Vec<(String, Vec<SophonChunkFile>)>,
    blacklist: &HashSet<String>,
    latest_tag: &str,
) -> RepairPlan {
    let mut files: Vec<PlannedFile> = Vec::new();
    let mut prefix: HashMap<String, String> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (url_prefix, list) in per_manifest {
        for f in list {
            if f.is_folder || f.file.is_empty() {
                continue;
            }
            let rel = f.file.replace('\\', "/");
            if blacklist.contains(&rel) || !seen.insert(rel.clone()) {
                continue;
            }
            prefix.insert(rel.clone(), url_prefix.clone());
            files.push(PlannedFile {
                rel,
                size: f.size,
                md5: f.md5.clone(),
                chunks: f.chunks,
            });
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    RepairPlan {
        latest: latest_tag.to_string(),
        files,
        url_prefix_by_file: prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sorted_dedup_blacklist() {
        let files = vec![
            SophonChunkFile {
                file: "b.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 2,
                md5: "m2".into(),
            },
            SophonChunkFile {
                file: "a.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m1".into(),
            },
            SophonChunkFile {
                file: "drop.dat".into(),
                chunks: vec![],
                is_folder: false,
                size: 1,
                md5: "m".into(),
            },
        ];
        let per = vec![("http://x".to_string(), files)];
        let bl: HashSet<String> = ["drop.dat".to_string()].into_iter().collect();
        let plan = build_plan(per, &bl, "5.0");
        assert_eq!(
            plan.files
                .iter()
                .map(|f| f.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["a.dat", "b.dat"]
        );
        assert_eq!(plan.latest, "5.0");
    }
}
