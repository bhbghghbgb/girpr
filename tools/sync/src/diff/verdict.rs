//! [`Verdict`]: what the diff concluded about one pair, and why.
//!
//! The reason vocabulary lives here rather than in the buckets that consume it,
//! because the tag is assembled off the verdict and must be assembled in exactly
//! one place — a tag built anywhere else is a tag that can disagree with the
//! decision it claims to explain.

use crate::hash::{AlgoSet, HashAlg};

use super::StatField;

/// What the diff concluded about one file-on-both-sides pair, and why.
///
/// Not `Option<Verdict>`: every pair gets exactly one, and a `None` arm would be a
/// state the diff must never be in. Directories are a verdict rather than a silent
/// fall-through precisely so that `--show-identical` cannot accidentally be partial.
///
/// **One variant per decision, not one per check.** The diff is a short circuit, so
/// the reason is whatever *settled* the pair — and "never report a previous
/// check's efforts" then falls out for free rather than needing enforcement, because
/// there is no earlier evidence to accumulate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A stat field this run trusts differs. `both` when size **and** mtime differ:
    /// still one stat decision, so one reason naming both.
    Stat {
        /// The field that differs, size first when `both`.
        field: StatField,
        /// Whether the other field differs too.
        both: bool,
    },
    /// Digests were consulted and disagree. `algos` are the ones that **disagreed**,
    /// not the ones that were requested — under `--hash-any-of` the run asked for
    /// several and the planner settled this pair with one, so naming the request
    /// would report an algorithm nobody compared.
    Differs {
        /// The algorithms whose digests differ, in [`AlgoSet`]'s canonical order.
        algos: AlgoSet,
    },
    /// Digests were consulted and agree. The complement of [`Self::Differs`], and
    /// only reachable when a digest was available — see [`Self::StatMatch`].
    Matches {
        /// The algorithms that were compared, in canonical order.
        algos: AlgoSet,
    },
    /// Every trusted stat field agrees and no digest was needed.
    ///
    /// Distinct from [`Self::Matches`] because "the digests agree" and "no digest was
    /// consulted" are different claims, and only the second is reached with
    /// `--hash-all-of none`. A run that requested a digest always gets one of the
    /// other two, so the two cannot be confused in the direction that matters.
    StatMatch,
    /// No digest was available, and a difference this run distrusts is unresolved.
    ///
    /// Reported as `CHANGED` rather than equal. **Cannot tell is not the same claim
    /// as differs**, and it is the one that must stay visible: a size-differing pair
    /// under `--hash-all-of none --no-trust-size` has no digest to fall back on, so
    /// calling it equal would let `sync` leave `dst` stale by a run that reported
    /// success.
    Unverifiable {
        /// The distrusted fields that differ.
        fields: Vec<StatField>,
    },
    /// A directory on both sides: compared by presence alone.
    DirPresent,
}

impl Verdict {
    /// The machine-readable reason, as printed by `--why` and read by a parser.
    ///
    /// **The one definition of the tag vocabulary.** A tag is `<name>` or
    /// `<name>:<detail>`, detail after a single `:`, and a combination joined with
    /// `+` — never `,`, because [`crate::report::Record`]'s text renderer joins array
    /// elements with `,` inside `[]`, so a comma inside a tag would be ambiguous in
    /// text. Kept next to the type rather than at the print site because the diff and
    /// the renderer must read the same value: a tag assembled anywhere else is a tag
    /// that can disagree about what happened.
    ///
    /// A flat string, not an object. A parser splits on the first `:` and switches on
    /// the name; an object would invite questions (`fields`? `detail`? `sources`?)
    /// that this does not need to answer.
    pub fn why_tag(&self) -> String {
        match self {
            Verdict::Stat {
                field: _,
                both: true,
            } => "stat-size+stat-mtime".to_string(),
            Verdict::Stat { field, both: false } => format!("stat-{}", field.as_str()),
            Verdict::Differs { algos } => format!("digest-differs:{}", join(*algos)),
            Verdict::Matches { algos } => format!("digest-matches:{}", join(*algos)),
            Verdict::StatMatch => "stat-match".to_string(),
            Verdict::Unverifiable { fields } => format!(
                "unverifiable:{}",
                fields
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            ),
            Verdict::DirPresent => "dir-present".to_string(),
        }
    }

    /// Whether this pair belongs in `CHANGED`.
    ///
    /// The bucket split, in one place so the two cannot drift: `sync` copies a
    /// `CHANGED` path, and `--show-identical` prints the rest, so a verdict counted
    /// as both would be copied *and* reported equal.
    pub fn is_changed(&self) -> bool {
        matches!(
            self,
            Verdict::Stat { .. } | Verdict::Differs { .. } | Verdict::Unverifiable { .. }
        )
    }

    /// Whether this pair belongs in `IDENTICAL`.
    ///
    /// The complement of [`Self::is_changed`], and stated rather than derived by
    /// negation: `--show-identical` prints this set, so "everything else" would make
    /// the flag's output a function of the *other* flag's behaviour.
    pub fn is_identical(&self) -> bool {
        matches!(
            self,
            Verdict::Matches { .. } | Verdict::StatMatch | Verdict::DirPresent
        )
    }
}

/// A tag detail list, `+`-joined.
///
/// Never `,`: see [`Verdict::why_tag`]. Non-empty because a detail that names
/// nothing is a malformed tag, and the two variants that carry one can always.
///
/// **Canonical order, not the user's flag order**, and that is the one deliberate
/// change to what a run prints. `AlgoSet` has no order of its own, so
/// `--hash-all-of sha256 md5` now reports `digest-differs:md5+sha256` where it used
/// to report `digest-differs:sha256+md5`. The tag is now a function of *the
/// disagreement* rather than of *how two flags were ordered*, which is what a
/// parser wants: `md5+sha256` means the same thing whichever way round the user
/// typed them.
fn join(algos: AlgoSet) -> String {
    debug_assert!(!algos.is_empty(), "a reason detail must name something");
    algos
        .iter()
        .map(HashAlg::as_str)
        .collect::<Vec<_>>()
        .join("+")
}
