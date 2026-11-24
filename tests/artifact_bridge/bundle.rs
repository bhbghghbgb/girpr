//! The bundle's own records: the capture matrix and the patchmaps.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../../../docs/06-artifact-bridge-spec.md)
//! §7. This is the **artifact suite's** half of a bundle: the cross-tool
//! claim is a comparison between what `girpr` did and what the patchmap says
//! the patch is, so the patchmap has to be read by the same code that runs
//! the binary.
//!
//! **What a patchmap carries, and what it does not.** The `plan[]` entries
//! are the ground truth: the per-chunk decision (`REUSE` or `FETCH`) that an
//! independent computation made, and the byte total that follows from it.
//! Nothing about *how* `girpr` got there is here — the tier it picked, the
//! direction it wrote in, whether it trimmed eagerly, what its `SUMMARY`
//! line said. Those are `girpr`'s own decisions, they are not shared with any
//! other repair tool, and they are tested where they belong: on crafted
//! fixtures, in `tests/repair_machinery.rs`.
//!
//! So this reader reads the **patch** and nothing else. A patchmap that grew
//! a `repair` block or an `expectations.summary` block would be read here
//! exactly as if those blocks were not there.

// The module is reached from two test binaries and each of them uses a
// different part of it, so most of it is dead code in any one of them.
#![allow(dead_code)]

use std::path::Path;

use serde_json::Value;

/// One entry of the capture matrix: a `patchmaps/<id>.json` plus the identity
/// its driver needs.
#[derive(Clone, Debug)]
pub struct Capture {
    /// The `capture_id`, which is also the `patchmaps/<id>.json` stem.
    pub id: String,
    /// The fixture path, as `/`-separated and relative to `--game-path`.
    pub path: String,
    /// Every class the file answers.
    pub classes: Vec<String>,
    /// Size of the file on disk in the bundle's `game/` fixture.
    pub old_size: u64,
    pub new_size: u64,
    /// `Σ compressed_size` over the chunks with no old-side match, or `None`
    /// for a capture with nothing to patch (`totals: null`).
    pub download_bytes: Option<u64>,
    /// Ground truth: the patched file's bytes live at this bundle-relative path.
    pub result_path: String,
    pub result_md5: String,
    /// `true` when the file must be **gone** afterwards.
    pub absent_after: bool,
    /// Chunks a correct patch must not request: the `REUSE` set.
    pub must_not_fetch: Vec<String>,
    /// The new side's chunks, with the decision the capture computed for each.
    pub plan: Vec<PlanChunk>,
}

impl Capture {
    /// A capture with no repair to do: it is deleted rather than patched, so
    /// it gets the `--purge-only` run and the purge counters instead of a byte
    /// total.
    pub fn is_purge_only(&self) -> bool {
        self.download_bytes.is_none()
    }

    /// The chunk ids the patch says must be **fetched**.
    pub fn fetch_set(&self) -> Vec<&str> {
        self.plan
            .iter()
            .filter(|c| c.decision == Decision::Fetch)
            .map(|c| c.chunk_id.as_str())
            .collect()
    }

    /// The chunk ids the patch says must **not** be fetched.
    pub fn reuse_set(&self) -> Vec<&str> {
        self.plan
            .iter()
            .filter(|c| c.decision == Decision::Reuse)
            .map(|c| c.chunk_id.as_str())
            .collect()
    }
}

/// One chunk of a patchmap's plan.
#[derive(Clone, Debug)]
pub struct PlanChunk {
    /// The chunk id.
    pub chunk_id: String,
    /// `REUSE` or `FETCH`: the decision the capture computed for it.
    pub decision: Decision,
    /// Where the chunk's bytes land in the result.
    pub target_start: u64,
    /// How many bytes it is.
    pub uncompressed_size: u64,
}

/// What a chunk's bytes are taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Verified byte-identical to a slice of the old file.
    Reuse,
    /// Has to be downloaded.
    Fetch,
}

impl Decision {
    /// The wire spelling, which the patchmap writes in caps.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "REUSE" => Some(Decision::Reuse),
            "FETCH" => Some(Decision::Fetch),
            _ => None,
        }
    }
}

/// One row of `MANIFEST.json`'s capture matrix.
#[derive(Clone, Debug)]
pub struct MatrixClass {
    pub class: String,
    /// The row's status: `captured`, `covered_by`, `over_budget` or
    /// `unavailable`.
    pub status: String,
    /// The capture that answers the class, when one does.
    ///
    /// For a `covered_by` row it is the **covering** capture's id: the class
    /// has no capture of its own, and the file that shows it is another
    /// capture's. This is why the class has to be resolved *through the
    /// matrix* — the patchmap's file name is the capture's id, not the class's.
    pub capture_id: Option<String>,
    /// `true` when the capture recorded a path and a `patchmaps/<id>.json`.
    pub captured: bool,
    /// Why a class no path satisfied. Never empty when `captured` is false —
    /// the bundle records *why* it has no instance rather than silently
    /// omitting it.
    pub reason: String,
}

impl MatrixClass {
    /// A class that is absent **by the bundle's own record**, as opposed to one
    /// that is simply missing. The distinction decides whether a driver case
    /// may stand down: a class the budget could not pay for says so, which is
    /// a fact, not a broken prerequisite.
    pub fn absent_by_record(&self) -> bool {
        !self.answered() && !self.reason.is_empty()
    }

    /// Whether some capture answers this class: of its own (`captured`) or by
    /// standing in for another (`covered_by`).
    ///
    /// A `covered_by` row **is** answered — that is what it says. Only
    /// `over_budget` and `unavailable` are not.
    pub fn answered(&self) -> bool {
        matches!(self.status.as_str(), "captured" | "covered_by")
    }
}

/// The bundle's records: the capture matrix and the patchmaps on disk.
///
/// The matrix and the patchmaps are two statements about the same thing, and
/// the reader keeps them side by side so a consumer can check them against
/// each other — which is `gircap`'s contract, and is checked there.
#[derive(Clone, Debug)]
pub struct Patchmaps {
    /// The matrix rows, ordered by class name.
    pub matrix: Vec<MatrixClass>,
    /// The captures on disk, ordered by capture id so runs are comparable.
    pub captures: Vec<Capture>,
}

impl Patchmaps {
    /// Read the matrix and the patchmaps of a bundle.
    pub fn read(root: &Path) -> Self {
        let manifest_path = root.join("MANIFEST.json");
        let manifest: Value = read_json(&manifest_path);
        let mut matrix: Vec<MatrixClass> = manifest["matrix"]
            .as_array()
            .unwrap_or_else(|| panic!("MANIFEST.json declares no matrix[]"))
            .iter()
            .map(|m| MatrixClass {
                class: str_field(&m["class"], &manifest_path),
                status: str_field(&m["status"], &manifest_path),
                capture_id: m["capture_id"].as_str().map(str::to_string),
                captured: m["status"].as_str() == Some("captured"),
                reason: m["reason"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        matrix.sort_by(|a, b| a.class.cmp(&b.class));

        let mut captures: Vec<Capture> = std::fs::read_dir(root.join("patchmaps"))
            .unwrap_or_else(|e| panic!("read patchmaps/ in {}: {e}", root.display()))
            .map(|e| {
                let path = e.expect("read patchmaps entry").path();
                if path.extension().is_some_and(|x| x == "json") {
                    Some(read_capture(&path))
                } else {
                    None
                }
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        captures.sort_by(|a, b| a.id.cmp(&b.id));

        Self { matrix, captures }
    }

    /// The matrix's row for a class, if the bundle records one.
    pub fn matrix_class(&self, class: &str) -> Option<&MatrixClass> {
        self.matrix.iter().find(|m| m.class == class)
    }

    /// The capture that answers a class — **through the matrix**.
    ///
    /// The matrix is the only place that knows which capture answers which
    /// class, and it is not the same as "the patchmap named after the class":
    /// a class the matrix records as `covered_by` has **no capture of its
    /// own**.
    pub fn capture_for_class(&self, class: &str) -> Option<&Capture> {
        let id = self.matrix_class(class)?.capture_id.as_deref()?;
        self.capture(id)
    }

    /// One capture, by its id.
    pub fn capture(&self, id: &str) -> Option<&Capture> {
        self.captures.iter().find(|c| c.id == id)
    }
}

/// Read one patchmap.
fn read_capture(path: &Path) -> Capture {
    let v = read_json(path);
    let ex = &v["expectations"];
    // `totals: null` is the shape of a capture with nothing to patch: there is
    // no byte total to compare against, and the capture is driven by the
    // purge counters instead.
    let download_bytes = v["totals"]["download_bytes"].as_u64();
    Capture {
        id: str_field(&v["capture_id"], path),
        path: str_field(&v["path"], path),
        classes: v["classes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        old_size: v["old"]["size"].as_u64().unwrap_or(0),
        new_size: v["new"]["size"].as_u64().unwrap_or(0),
        download_bytes,
        result_path: ex["result_path"].as_str().unwrap_or_default().to_string(),
        result_md5: ex["result_md5"].as_str().unwrap_or_default().to_string(),
        absent_after: ex["absent_after"].as_bool().unwrap_or(false),
        must_not_fetch: ex["must_not_fetch"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        plan: v["plan"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some(PlanChunk {
                            chunk_id: c["chunk_id"].as_str()?.to_string(),
                            decision: Decision::parse(c["decision"].as_str()?)?,
                            target_start: c["target_range"]["start"].as_u64().unwrap_or(0),
                            uncompressed_size: c["uncompressed_size"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn read_json(path: &Path) -> Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn str_field(v: &Value, path: &Path) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("{} is missing a string field", path.display()))
        .to_string()
}

// ---------------------------------------------------------------------------
// the optional persistent records
// ---------------------------------------------------------------------------

/// The persistent records a bundle carries. Old bundles have neither; the
/// bridge's persistent case stands down loudly if the named bundle has no
/// records rather than pretending it checked them.
#[derive(Clone, Debug)]
pub struct PersistentRecords {
    pub dispatch: Value,
    pub index: Value,
}

impl PersistentRecords {
    pub fn read(root: &Path) -> Option<Self> {
        let d = root.join("dispatch").join("record.json");
        let i = root.join("persistent").join("index.json");
        if !d.is_file() || !i.is_file() {
            return None;
        }
        Some(Self {
            dispatch: read_json(&d),
            index: read_json(&i),
        })
    }

    /// A mirror's raw bytes, by kind (`res`, `data`, `silence_data`).
    pub fn manifest(&self, root: &Path, kind: &str) -> Vec<u8> {
        let path = self.index["manifests"]
            .as_array()
            .expect("persistent/index.json manifests[]")
            .iter()
            .find(|f| f["kind"].as_str() == Some(kind))
            .and_then(|f| f["path"].as_str())
            .unwrap_or_else(|| panic!("no {kind} manifest in persistent/index.json"));
        std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    /// A full-file blob's expected bytes, by the basename of its `remoteName`.
    pub fn blob(&self, root: &Path, name: &str) -> Vec<u8> {
        let path = self.index["blobs"]
            .as_array()
            .expect("persistent/index.json blobs[]")
            .iter()
            .find(|f| {
                f["remote_name"]
                    .as_str()
                    .is_some_and(|r| r.rsplit('/').next() == Some(name))
            })
            .and_then(|f| f["path"].as_str())
            .unwrap_or_else(|| panic!("no {name} blob in persistent/index.json"));
        std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }
}

/// The `DispatchInfo` the replayed record describes.
///
/// The record is the rewritten copy from [`server::Bridge`], so the CDN
/// prefixes name this process's listeners; the descriptors' paths are exactly
/// the ones the dispatcher published. Nothing here invents a revision or a
/// digest: every value is read from the record.
pub fn dispatch_info(record: &Value) -> girpr::test_support::DispatchInfo {
    use girpr::test_support::{ManifestDescriptor, ManifestKind};

    let d = &record["decoded"];
    let s = |key: &str| -> String {
        d[key]
            .as_str()
            .unwrap_or_else(|| panic!("dispatch record has no decoded.{key}"))
            .to_string()
    };
    let n = |key: &str| -> u64 { d[key].as_u64().unwrap_or(0) };
    let repo_res = s("repo_res_version_url");
    let repo_data = s("repo_design_data_url");
    let res_rev = n("res_revision");
    let data_rev = n("data_revision");
    let sil_rev = n("silence_revision");
    let audio_rev = n("audio_revision");
    let res_hash = s("res_hash");
    let data_hash = s("data_hash");
    let sil_hash = s("silence_hash");
    let audio_hash = s("audio_hash");

    let kinds = ["res", "data", "silence_data"];
    let descriptors = record["decoded"]["descriptors"]
        .as_array()
        .expect("dispatch record descriptors[]")
        .iter()
        .zip(kinds)
        .map(|(desc, kind)| {
            let kind = match kind {
                "res" => ManifestKind::Res,
                "data" => ManifestKind::Data,
                _ => ManifestKind::Silence,
            };
            ManifestDescriptor {
                kind,
                url: desc["url"].as_str().expect("descriptor url").to_string(),
                expected_md5: desc["md5"].as_str().expect("descriptor md5").to_string(),
                expected_size: desc["file_size"].as_u64().unwrap_or(0),
                mirror_rel: format!(
                    "GenshinImpact_Data/Persistent/{}_versions_persist",
                    kind.as_str()
                ),
            }
        })
        .collect::<Vec<_>>();
    let [res, data, silence]: [ManifestDescriptor; 3] =
        descriptors.try_into().expect("three mirrored manifests");

    let client_game_res_url = format!("{repo_res}/output_{res_rev}_{res_hash}/client");
    let client_design_data_url =
        format!("{repo_data}/output_{data_rev}_{data_hash}/client/General");
    let client_silence_data_url =
        format!("{repo_data}/output_{sil_rev}_{sil_hash}/client_silence/General");
    let client_audio_assets_url = format!("{repo_res}/output_{audio_rev}_{audio_hash}/client");

    girpr::test_support::DispatchInfo {
        game_version: s("game_version"),
        channel_name: s("channel_name"),
        server_name: "TEST".to_string(),
        codename: s("codename"),
        res_revision: res_rev,
        data_revision: data_rev,
        silence_revision: sil_rev,
        audio_revision: audio_rev,
        res_hash,
        data_hash,
        silence_hash: sil_hash,
        audio_hash,
        client_game_res_url,
        client_design_data_url,
        client_silence_data_url,
        client_audio_assets_url,
        manifest_digests: [res, data, silence],
    }
}
