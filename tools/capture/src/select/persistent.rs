//! Cheapest-first selection over the three verified persistent manifests.
//!
//! Pure: no socket, no file, no clock. The producer verifies each raw manifest
//! against its dispatcher descriptor and calls [`parse_rows`] before anything
//! reaches here; this module only decides which full-file blobs are worth
//! carrying under the budget the content classes have **already spent**.
//!
//! The metric is `row.file_size` — the whole file as it crosses the wire, not a
//! patch or a compressed chunk — and the tie-break is `remote_name` so two rows
//! of equal size have a stable, reproducible order. One row per manifest is
//! bought first (a bundle must carry a sample of each of the three mirrored
//! manifests), then the remaining rows are bought cheapest-first globally.
//!
//! The fixed overhead (the byte-exact envelope plus the three raw manifests)
//! is charged **once**, before the blobs, because a bundle needs those bytes
//! regardless of which rows win. Persistent admissions go into their own
//! vector: [`super::Selection::admitted`] is read by `blobs::plan`,
//! `file_pairs` and `emit::fixture`, all of which assume a patch-shaped path.

use girpr_dispatch::{ManifestDescriptor, ManifestKind};
use girpr_persistent_plan::{PersistentRow, parse_rows};

use super::{Budget, Class, MatrixRow, MatrixStatus};

/// One dispatcher descriptor, its byte-exact body and the parsed rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedManifest {
    /// The matrix class this manifest answers.
    pub class: Class,
    /// The descriptor whose URL produced `raw`.
    pub descriptor: ManifestDescriptor,
    /// The raw NDJSON body, kept verbatim for the bundle.
    pub raw: Vec<u8>,
    /// Every non-blank row, in wire order.
    pub rows: Vec<PersistentRow>,
}

impl VerifiedManifest {
    /// Verify size then MD5 against the descriptor, then parse the rows.
    ///
    /// A failure is a string because the producer maps it onto its own
    /// "unavailable" matrix reason rather than failing an ordinary capture.
    pub fn new(descriptor: ManifestDescriptor, raw: Vec<u8>) -> Result<Self, String> {
        let actual_size = raw.len() as u64;
        if actual_size != descriptor.expected_size {
            return Err(format!(
                "{} manifest is {actual_size} B, descriptor says {} B",
                descriptor.kind.as_str(),
                descriptor.expected_size
            ));
        }
        let md5 = crate::hash::md5_hex(&raw);
        if !md5.eq_ignore_ascii_case(&descriptor.expected_md5) {
            return Err(format!(
                "{} manifest md5 {md5} != descriptor {}",
                descriptor.kind.as_str(),
                descriptor.expected_md5
            ));
        }
        let rows =
            parse_rows(&raw).map_err(|e| format!("{} manifest: {e}", descriptor.kind.as_str()))?;
        Ok(Self {
            class: class_for_kind(descriptor.kind),
            descriptor,
            raw,
            rows,
        })
    }
}

/// Which record class a dispatcher descriptor feeds.
pub fn class_for_kind(kind: ManifestKind) -> Class {
    match kind {
        ManifestKind::Res => Class::PersistentRes,
        ManifestKind::Data => Class::PersistentData,
        ManifestKind::Silence => Class::PersistentSilence,
    }
}

/// One full-file blob the selection admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentAdmission {
    /// `<class>` for the first row of a manifest, `<class>+<n>` for extras.
    pub capture_id: String,
    /// The record class this blob answers.
    pub class: Class,
    /// The dispatcher row's `remoteName`.
    pub remote_name: String,
    /// Bundle-relative path of the raw body.
    pub path: String,
    /// `row.file_size`, the budget charge and full-file wire length.
    pub budget_cost: u64,
    /// The row's MD5, used to gate the body on fetch.
    pub md5: String,
}

/// What the persistent selection decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentSelection {
    /// The full-file admissions, cheapest first.
    pub admitted: Vec<PersistentAdmission>,
    /// One honest row per record class, in [`Class::PERSISTENT`] order.
    pub matrix: Vec<MatrixRow>,
    /// Envelope + three manifests + every admitted blob.
    pub bytes_admitted: u64,
    /// The ledger after all charges, equal to `bytes_admitted`.
    pub budget: Budget,
}

/// A row paired with its source manifest, ordered by cost then remote name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Option_ {
    class: Class,
    row: PersistentRow,
    ordinal: usize,
}

fn blob_path(remote_name: &str) -> String {
    crate::paths::escape_path(&format!(
        "persistent/blobs/{}",
        remote_name.replace('\\', "/")
    ))
}

fn eligible(manifest: &VerifiedManifest) -> Vec<Option_> {
    let mut rows: Vec<Option_> = manifest
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            crate::paths::logical_path_problem(&row.remote_name.replace('\\', "/")).is_none()
        })
        .map(|(ordinal, row)| Option_ {
            class: manifest.class,
            row: row.clone(),
            ordinal,
        })
        .collect();
    rows.sort_by(|a, b| {
        a.row
            .file_size
            .cmp(&b.row.file_size)
            .then_with(|| a.row.remote_name.cmp(&b.row.remote_name))
            .then_with(|| a.ordinal.cmp(&b.ordinal))
    });
    rows
}

fn admission(option: &Option_, nth: usize) -> PersistentAdmission {
    let id = if nth == 0 {
        option.class.id().to_string()
    } else {
        format!("{}+{nth}", option.class.id())
    };
    PersistentAdmission {
        capture_id: id,
        class: option.class,
        remote_name: option.row.remote_name.clone(),
        path: blob_path(&option.row.remote_name),
        budget_cost: option.row.file_size,
        md5: option.row.md5.clone(),
    }
}

fn unavailable(class: Class, considered: usize, reason: impl Into<String>) -> MatrixRow {
    MatrixRow::new(class, MatrixStatus::Unavailable, considered as u64).with_reason(reason)
}

fn over_budget(class: Class, considered: usize, cost: u64) -> MatrixRow {
    MatrixRow::new(class, MatrixStatus::OverBudget, considered as u64).with_reason(format!(
        "the cheapest {class} row costs {cost} B and the budget could not fund it"
    ))
}

/// Admit the cheapest rows under a budget already spent by content captures.
///
/// The caller passes `envelope_bytes` and the three verified manifests; every
/// byte charged here is added to the same ledger the content selection used,
/// so `bytes_admitted == budget.spent()` still holds at the end.
pub fn select(
    manifests: &[VerifiedManifest; 3],
    envelope_bytes: u64,
    mut budget: Budget,
) -> PersistentSelection {
    let fixed = envelope_bytes.saturating_add(
        manifests
            .iter()
            .fold(0u64, |n, m| n.saturating_add(m.raw.len() as u64)),
    );

    // The envelope and all three manifests are indivisible at schema 4: girpr
    // always fetches all three from the one aggregate dispatch record. If the
    // fixed cost alone does not fit, no row can honestly be called captured.
    if !budget.admits(fixed) {
        let rows = Class::PERSISTENT
            .iter()
            .map(|class| {
                let considered = manifests
                    .iter()
                    .find(|m| m.class == *class)
                    .map(|m| m.rows.len())
                    .unwrap_or(0);
                over_budget(*class, considered, fixed)
            })
            .collect();
        return PersistentSelection {
            admitted: Vec::new(),
            matrix: rows,
            // The ledger includes what the content classes already spent:
            // the caller replaces `Selection.bytes_admitted` with this number.
            bytes_admitted: budget.spent(),
            budget,
        };
    }
    budget.charge(fixed);

    let mut admitted: Vec<PersistentAdmission> = Vec::new();
    let mut matrix: Vec<MatrixRow> = Vec::with_capacity(3);
    let mut extras: Vec<Option_> = Vec::new();
    let mut ordinals: std::collections::BTreeMap<Class, usize> = std::collections::BTreeMap::new();

    // Phase 1 — one blob per manifest, cheapest first within that manifest.
    for class in Class::PERSISTENT {
        let Some(manifest) = manifests.iter().find(|m| m.class == class) else {
            matrix.push(unavailable(class, 0, "no verified manifest for this class"));
            continue;
        };
        let mut choices = eligible(manifest);
        let considered = manifest.rows.len();
        let Some(first) = choices.first().cloned() else {
            matrix.push(unavailable(
                class,
                considered,
                "the verified manifest has no eligible row",
            ));
            continue;
        };
        let cost = first.row.file_size;
        if !budget.admits(cost) {
            matrix.push(over_budget(class, considered, cost));
            // Keep this manifest's rows out of the extras phase: the class
            // already said over-budget and a dearer row cannot fit either.
            continue;
        }
        budget.charge(cost);
        let nth = ordinals.get(&class).copied().unwrap_or(0);
        ordinals.insert(class, nth + 1);
        let paid = admission(&first, nth);
        let mut row = MatrixRow::new(class, MatrixStatus::Captured, considered as u64);
        row.capture_id = Some(paid.capture_id.clone());
        row.path = Some(paid.path.clone());
        admitted.push(paid);
        matrix.push(row);
        extras.extend(choices.drain(1..));
    }

    // Phase 2 — the remaining rows, cheapest first across all three manifests.
    extras.sort_by(|a, b| {
        a.row
            .file_size
            .cmp(&b.row.file_size)
            .then_with(|| a.row.remote_name.cmp(&b.row.remote_name))
            .then_with(|| a.class.cmp(&b.class))
            .then_with(|| a.ordinal.cmp(&b.ordinal))
    });
    for option in extras {
        if !budget.admits(option.row.file_size) {
            continue;
        }
        budget.charge(option.row.file_size);
        let nth = ordinals.get(&option.class).copied().unwrap_or(0);
        ordinals.insert(option.class, nth + 1);
        admitted.push(admission(&option, nth));
    }

    // The ledger is the single number: it already included whatever the
    // content classes spent, so the returned total is `spent()`, not a second
    // sum over `admitted` (which would omit both that spend and the fixed
    // overhead when the caller passes an already-charged budget).
    let bytes_admitted = budget.spent();
    PersistentSelection {
        admitted,
        matrix,
        bytes_admitted,
        budget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(kind: ManifestKind, rows: &[(&str, u64)]) -> VerifiedManifest {
        let raw = rows
            .iter()
            .map(|(name, size)| {
                format!(
                    "{{\"remoteName\":\"{name}\",\"md5\":\"b2d3f56bc197fd985d5965079b5e7148\",\"fileSize\":{size}}}\n"
                )
            })
            .collect::<String>()
            .into_bytes();
        let descriptor = ManifestDescriptor {
            kind,
            url: format!("https://test.invalid/{}.ndjson", kind.as_str()),
            expected_md5: crate::hash::md5_hex(&raw),
            expected_size: raw.len() as u64,
            mirror_rel: String::new(),
        };
        VerifiedManifest::new(descriptor, raw).expect("a verified fixture manifest")
    }

    fn trio() -> [VerifiedManifest; 3] {
        [
            manifest(ManifestKind::Res, &[("r/a", 30), ("r/b", 10)]),
            manifest(ManifestKind::Data, &[("d/a", 20), ("d/b", 10)]),
            manifest(ManifestKind::Silence, &[("s/a", 40)]),
        ]
    }

    const ENVELOPE: u64 = 50;

    #[test]
    fn the_cost_is_the_full_file_and_the_tie_break_is_remote_name() {
        let m = manifest(
            ManifestKind::Res,
            &[("z.blk", 10), ("a.blk", 10), ("dear.blk", 11)],
        );
        let s = select(
            &[m, trio()[1].clone(), trio()[2].clone()],
            0,
            Budget::unlimited(),
        );
        let res: Vec<&str> = s
            .admitted
            .iter()
            .filter(|a| a.class == Class::PersistentRes)
            .map(|a| a.remote_name.as_str())
            .collect();
        assert_eq!(res, ["a.blk", "z.blk", "dear.blk"]);
        assert_eq!(s.admitted[0].budget_cost, 10, "file_size, not a patch size");
    }

    #[test]
    fn one_row_per_manifest_comes_first_then_the_extras_cheapest_first() {
        let s = select(&trio(), ENVELOPE, Budget::unlimited());
        // Fixed 50 + three manifests; then r/b(10), d/b(10), s/a(40) before
        // d/a(20) and r/a(30) — the extra phase ignores manifest order.
        let names: Vec<&str> = s.admitted.iter().map(|a| a.remote_name.as_str()).collect();
        assert_eq!(
            names,
            ["r/b", "d/b", "s/a", "d/a", "r/a"],
            "one per manifest first, then globally cheapest"
        );
        assert_eq!(s.bytes_admitted, s.budget.spent());
    }

    #[test]
    fn every_class_gets_an_honest_capture_row() {
        let s = select(&trio(), ENVELOPE, Budget::unlimited());
        assert_eq!(s.matrix.len(), 3);
        for row in &s.matrix {
            assert_eq!(row.status, MatrixStatus::Captured, "{row:?}");
            assert!(row.capture_id.is_some(), "{row:?}");
            assert!(row.path.is_some(), "{row:?}");
            assert!(row.candidates_considered > 0, "{row:?}");
            assert!(
                row.covered_by.is_none(),
                "a record class is never covered_by"
            );
        }
    }

    #[test]
    fn a_manifest_with_no_eligible_row_is_unavailable_not_over_budget() {
        let mut manifests = trio();
        // parse_rows rejects an empty body, so build one with only an unsafe
        // remote name: it is parsed but cannot name a bundle file.
        let raw = b"{\"remoteName\":\"../escape\",\"md5\":\"b2d3f56bc197fd985d5965079b5e7148\",\"fileSize\":9}\n".to_vec();
        let descriptor = ManifestDescriptor {
            kind: ManifestKind::Silence,
            url: "https://test.invalid/silence.ndjson".into(),
            expected_md5: crate::hash::md5_hex(&raw),
            expected_size: raw.len() as u64,
            mirror_rel: String::new(),
        };
        manifests[2] = VerifiedManifest::new(descriptor, raw).expect("parsed");
        let s = select(&manifests, 0, Budget::unlimited());
        let row = s
            .matrix
            .iter()
            .find(|r| r.class == Class::PersistentSilence)
            .expect("the row exists");
        assert_eq!(row.status, MatrixStatus::Unavailable);
        assert_eq!(row.candidates_considered, 1, "the row was parsed");
        assert!(row.reason.as_deref().is_some_and(|r| !r.is_empty()));
        assert!(row.capture_id.is_none());
    }

    #[test]
    fn a_cheapest_row_that_does_not_fit_is_over_budget() {
        // Fixed = 50 + manifests; cap exactly at fixed so no blob can be paid
        // for. Every class with rows reports over_budget, never unavailable.
        let manifests = trio();
        let fixed = ENVELOPE + manifests.iter().fold(0, |n, m| n + m.raw.len() as u64);
        let s = select(&manifests, ENVELOPE, Budget::capped(fixed));
        assert!(s.admitted.is_empty());
        assert_eq!(s.bytes_admitted, fixed, "the fixed cost is charged once");
        for row in &s.matrix {
            assert_eq!(row.status, MatrixStatus::OverBudget, "{row:?}");
            assert!(
                row.reason
                    .as_deref()
                    .is_some_and(|r| r.contains("budget could not fund")),
                "{row:?}"
            );
        }
        assert_eq!(s.budget.spent(), fixed);
    }

    #[test]
    fn fixed_overhead_over_the_limit_reports_over_budget_without_charging() {
        let s = select(&trio(), ENVELOPE, Budget::capped(1));
        assert!(s.admitted.is_empty());
        assert_eq!(s.bytes_admitted, 0);
        assert_eq!(s.budget.spent(), 0);
        assert!(
            s.matrix
                .iter()
                .all(|r| r.status == MatrixStatus::OverBudget)
        );
    }

    #[test]
    fn the_budget_is_cumulative_across_content_and_persistent_admissions() {
        let manifests = trio();
        let fixed = ENVELOPE + manifests.iter().fold(0, |n, m| n + m.raw.len() as u64);
        let spent_by_content = 7;
        let cap = fixed + spent_by_content + 20;
        let mut budget = Budget::capped(cap);
        budget.charge(spent_by_content);
        let s = select(&manifests, ENVELOPE, budget);
        assert_eq!(s.budget.spent(), s.bytes_admitted);
        assert!(s.bytes_admitted <= cap);
        // r/b(10) and d/b(10) fit; the 40 B silence row does not, and no
        // extra row can be bought either.
        let names: Vec<&str> = s.admitted.iter().map(|a| a.remote_name.as_str()).collect();
        assert_eq!(names, ["r/b", "d/b"]);
        assert_eq!(
            s.matrix
                .iter()
                .find(|r| r.class == Class::PersistentSilence)
                .map(|r| r.status),
            Some(MatrixStatus::OverBudget)
        );
    }
}
