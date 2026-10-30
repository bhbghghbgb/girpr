//! Re-verification: re-deriving every claim a bundle's own bytes can support,
//! and comparing the recomputation with what the capture recorded.

use std::collections::BTreeMap;

use super::materialize::Pair;
use crate::assemble::PatchShape;
use crate::emit::selfcheck::{Assertion, coverage, wording};

use super::bundle::Bundle;

/// A18's wording, kept in one place because `verify` and the docs both quote it.
pub const A18: &str =
    "the bytes rebuilt from chunks/ are byte-for-byte the bytes game/ and expected/ hold";

/// A18's id, as the spec spells ids.
pub const A18_ID: &str = "A18";

/// The ids [`reverify`] re-derives **and** the capture also recorded.
///
/// The overlap, and the reason it is a `&'static` list rather than a computed
/// one: these are the rows `check_agreement` is entitled to compare, so putting
/// an id here silently promotes it from "re-derived" to "re-derived *and* checked
/// against the recording". A18 is re-derived but never recorded - the capture
/// cannot write it, because it is a statement about the bundle rather than about
/// the run - so it is deliberately absent.
pub const REDERIVED: [&str; 7] = ["A16", "A6", "A11", "A7", "A8", "A13", "A17"];

/// Re-derived here, and never recorded by any capture.
///
/// Kept apart from [`REDERIVED`] because that set means "re-derived *and*
/// written down", which is what makes `check_agreement` possible: there is
/// nothing to agree with for these. Putting them in `REDERIVED` would have the
/// summary claim a cross-check that cannot exist, and would drag them into a
/// comparison against a row that was never emitted.
pub const DERIVED_UNRECORDED: [&str; 2] = ["A18", "A20"];

/// Everything `verify` looks at, in the order the summary prints it.
pub fn derived_ids() -> Vec<&'static str> {
    let mut v = REDERIVED.to_vec();
    v.extend(DERIVED_UNRECORDED);
    v
}

/// What re-verification found.
///
/// The rebuilt bytes are kept rather than dropped, because [`materialize`] is the
/// same rebuild with somewhere to put them, and rebuilding twice on a 135 MB
/// bundle costs a minute of the operator's time for nothing.
///
/// Deliberately **not** [`Clone`]: `rebuilt` is the whole fixture, so a derived
/// `Clone` is a ~135 MB copy waiting for the caller that wanted a second opinion.
#[derive(Debug)]
pub struct Report {
    /// The bundle it came from.
    pub bundle: Bundle,
    /// One row per re-derived check.
    pub rows: Vec<Assertion>,
    /// Every side rebuilt from the blobs alone.
    pub rebuilt: Vec<Pair>,
    /// Findings that are not one row: the A16 list and any closure error.
    pub findings: Vec<String>,
    /// Why the closure failed, when it did.
    ///
    /// Separate from `findings` because it is the *cause* and the rest are
    /// symptoms, and [`Report::headline`] orders them differently for that
    /// reason.
    pub closure: Option<String>,
    /// Whether the recomputation agreed with what the capture recorded.
    pub agrees: bool,
}

impl Report {
    /// The one line that says what is actually wrong.
    ///
    /// Root cause first. When a blob cannot be decoded, A16 also fails - the
    /// file's digest no longer matches - and a reader who is handed the A16 row
    /// goes looking for a stale checksum list rather than a damaged blob. So:
    ///
    /// 1. the closure error, which names the chunk and why it would not decode;
    /// 2. otherwise the first red A6 row, which names the blob in words a reader
    ///    can act on;
    /// 3. otherwise the first red row of any id.
    pub fn headline(&self) -> Option<String> {
        if let Some(c) = &self.closure {
            return Some(c.clone());
        }
        let pick = |id: &str| self.rows.iter().find(|r| r.id == id && !r.ok);
        pick("A6")
            .or_else(|| self.rows.iter().find(|r| !r.ok))
            .map(|r| format!("{} {}: {}", r.id, r.subject, r.actual))
    }

    /// `pass` when every re-derived row held.
    pub fn result(&self) -> &'static str {
        if self.rows.iter().any(|r| !r.ok) {
            "fail"
        } else {
            "pass"
        }
    }

    /// The rows that did not hold.
    pub fn failures(&self) -> Vec<&Assertion> {
        self.rows.iter().filter(|r| !r.ok).collect()
    }

    /// `(total, passed, failed)`.
    pub fn counts(&self) -> (u64, u64, u64) {
        let failed = self.rows.iter().filter(|r| !r.ok).count() as u64;
        (
            self.rows.len() as u64,
            self.rows.len() as u64 - failed,
            failed,
        )
    }

    /// How many rows each id produced.
    pub fn by_id(&self) -> BTreeMap<String, u64> {
        let mut m: BTreeMap<String, u64> = BTreeMap::new();
        for r in &self.rows {
            *m.entry(r.id.clone()).or_default() += 1;
        }
        m
    }

    /// Bytes rebuilt from the blobs.
    pub fn rebuilt_bytes(&self) -> u64 {
        self.rebuilt
            .iter()
            .flat_map(|p| [p.old.as_ref(), p.new.as_ref()])
            .flatten()
            .map(|s| s.bytes.len() as u64)
            .sum()
    }
}

/// Re-derive every claim the bundle's own bytes can support.
///
/// Never fails. A bundle that cannot even be opened fails in [`Bundle::open`];
/// a bundle that opens and then does not hold is the *finding*, and returning it
/// as a report rather than an error is what lets the summary name every broken
/// row instead of only the first.
pub fn reverify(bundle: Bundle) -> Report {
    let root = bundle.root.clone();
    let mut rows: Vec<Assertion> = Vec::new();
    let mut findings: Vec<String> = Vec::new();

    // 1. A16.
    let found = bundle.checksums.findings(&root);
    let listed = bundle.checksums.entries.len();
    rows.push(if found.is_empty() {
        Assertion::pass(
            "A16",
            wording("A16"),
            "verify/checksums.json",
            &format!("{listed} files re-hashed, every digest matched"),
        )
    } else {
        findings.clone_from(&found);
        Assertion::fail(
            "A16",
            wording("A16"),
            &format!("{} of {listed} file(s)", found.len()),
            "every listed file re-hashes to its recorded digest",
            &found[0],
        )
    });

    // 2. A6, one row per stored blob.
    rows.extend(super::checks::check_blobs(
        &root,
        &bundle.index,
        bundle.manifest.max_decompressed_bytes,
    ));

    // 3. The closure check. A bundle whose blobs cannot rebuild both files is not
    // closed, and that is the single most important thing this command reports -
    // so it short-circuits: every row after this point would be about bytes that
    // do not exist.
    let rebuilt = match super::materialize::rebuild_all(
        &root,
        &bundle.index,
        &bundle.patchmaps,
        bundle.manifest.max_decompressed_bytes,
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(subject = "closure", error = %e, "the retained blobs cannot rebuild the bundle");
            findings.push(e.clone());
            rows.push(Assertion::fail(
                "A11",
                wording("A11"),
                "chunks/index.json",
                "every capture's chunks tile its file",
                &e,
            ));
            rows.push(Assertion::fail(
                A18_ID,
                A18,
                "chunks/",
                "both files rebuild from the retained blobs",
                &e,
            ));
            return Report {
                bundle,
                rows,
                rebuilt: Vec::new(),
                findings,
                closure: Some(e),
                agrees: false,
            };
        }
    };
    for p in &rebuilt {
        let Some(map) = bundle.patchmaps.get(&p.capture_id) else {
            continue;
        };
        rows.extend(super::checks::check_pair(&root, p, map));
    }

    // 4. The two claims that are about documents rather than blobs.
    rows.extend(super::checks::check_config_ini(&root, &bundle.manifest));
    let (a17_ok, a17_detail) = bundle.manifest.a17();
    rows.push(if a17_ok {
        Assertion::pass("A17", wording("A17"), "MANIFEST.json matrix", &a17_detail)
    } else {
        Assertion::fail(
            "A17",
            wording("A17"),
            "MANIFEST.json matrix",
            "every class captured | covered_by | over_budget | unavailable",
            &a17_detail,
        )
    });

    // 4b. The bundle's **coverage**: whether it holds both sides of the reuse
    // boundary, the moved boundary and the size boundary.
    //
    // A **finding**, not a failed check, and that is the whole distinction.
    // The coverage is a design property of the bundle - it is what the class
    // set was captured for - and a bundle that does not cover everything (a
    // narrow one, or a hand-built one) is still internally consistent. The
    // *capture* is where the coverage is a contract: a capture that does not
    // cover the patch space fails its own run (A21-A23 are fatal there).
    // The verifier's job is to prove the bundle still says what it said, and
    // "this bundle does not cover both sides of the reuse boundary" is a thing
    // it said.
    let shapes: Vec<PatchShape> = bundle
        .patchmaps
        .values()
        .map(crate::assemble::patch_shape)
        .collect();
    for row in coverage(&shapes) {
        if !row.ok {
            findings.push(format!("{}: {}", row.id, row.actual));
            tracing::error!(
                assertion = %row.id,
                subject = %row.subject,
                actual = %row.actual,
                "the bundle does not cover the patch space"
            );
        }
    }

    // 5. The recording, last.
    let agrees = super::checks::check_agreement(&bundle, &rows);
    Report {
        bundle,
        rows,
        rebuilt,
        findings,
        closure: None,
        agrees,
    }
}
