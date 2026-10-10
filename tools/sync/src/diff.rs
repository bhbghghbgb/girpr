//! Path-set diffing between two effective maps.

use std::collections::{HashMap, HashSet};

use crate::effective::EffRec;
use crate::planner::{Required, StatTrust};

/// Re-exported rather than re-declared: `StatField` is defined beside `StatTrust`,
/// whose predicates speak it, and a second definition here would be a second
/// vocabulary. Re-exported because every caller of the reason vocabulary needs both
/// halves together — a [`Verdict`] cannot be built or matched without it.
pub use crate::planner::StatField;

/// What the diff concluded about one file-on-both-sides pair, and why.
///
/// Not `Option<Verdict>`: every pair gets exactly one, and a `None` arm would be a
/// state the diff must never be in. Directories are a verdict rather than a silent
/// fall-through precisely so that `--show-identical` cannot accidentally be partial.
///
/// **One variant per decision, not one per check.** `is_changed` was a short circuit,
/// so the reason is whatever *settled* the pair — and "never report a previous
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
        /// The algorithms whose digests differ, in the order they were requested.
        algos: Vec<String>,
    },
    /// Digests were consulted and agree. The complement of [`Self::Differs`], and
    /// only reachable when a digest was available — see [`Self::StatMatch`].
    Matches {
        /// The algorithms that were compared.
        algos: Vec<String>,
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
    /// as differs**, and it is the one that must stay visible: this is the stage-8
    /// hardening, where a size-differing pair under `--hash-all-of none
    /// --no-trust-size` used to come back *identical* — and `sync` then left `dst`
    /// stale by a run that reported success.
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
            Verdict::Differs { algos } => format!("digest-differs:{}", join(algos)),
            Verdict::Matches { algos } => format!("digest-matches:{}", join(algos)),
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
fn join(algos: &[String]) -> String {
    debug_assert!(!algos.is_empty(), "a reason detail must name something");
    algos.join("+")
}

/// Per-class diff buckets, each sorted by relative path.
///
/// Derives `PartialEq` so a test can state a whole expected verdict at once
/// rather than five fields one at a time; it has no interior state, so this
/// costs nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// src-only.
    pub missing: Vec<String>,
    /// dst-only.
    pub extra: Vec<String>,
    /// Present on both but size/mtime/hash differ.
    pub changed: Vec<String>,
    /// Same path, different kind (file vs dir).
    pub type_conflict: Vec<String>,
    /// Same path ignoring case but different casing (insensitive mode only).
    pub case_mismatch: Vec<(String, String)>,
    /// Pairs found equal. **Only populated when the caller asks** — see
    /// `want_identical` in [`diff_maps`].
    ///
    /// Gated rather than always filled because an always-populated field would make
    /// [`Self::total`] and [`Self::is_empty`] wrong *by default*, and `total()` is
    /// what feeds the exit code. A field whose emptiness changes the answer is a
    /// field whose emptiness has to be load-bearing in three places at once; an
    /// always-empty-unless-asked field is empty in exactly one.
    pub identical: Vec<String>,
    /// The classification of every pair found on **both** sides, keyed by src path.
    ///
    /// **Not a bucket.** Nothing here counts: `total()` and `is_empty()` must not read
    /// it, because a pair that came out equal is not a difference and counting one
    /// would turn a clean tree into exit 4. It is the reason behind [`Self::changed`]
    /// rather than a sixth list of paths, and [`crate::report::verdict`] reads it to
    /// put a `why=` on the `CHANGED` records.
    ///
    /// Filled from the same [`pair_verdict`] value the bucket is filled from, so a
    /// `CHANGED` record and the tag printed for it cannot disagree — one decision, read
    /// twice, rather than a classification and a bucket that could each be computed
    /// separately.
    pub verdicts: HashMap<String, Verdict>,
}

impl Diff {
    /// Number of differences that force a nonzero exit, ignoring case mismatches
    /// (which `compare` counts separately but which are still a difference).
    ///
    /// Counts the four difference buckets **and nothing else**.
    ///
    /// [`Self::identical`] is absent here deliberately rather than by oversight, and
    /// this is the most consequential line in the crate: `total()` feeds the exit
    /// code, so counting the equal set would turn "these two trees are identical"
    /// into exit `4` — the exact inverse of the truth, on the one run where the user
    /// is most entitled to confidence. A pair that came out equal is the *absence* of
    /// a difference, and a count of differences cannot include it.
    ///
    /// [`Self::verdicts`] is absent for the same reason: it holds equal pairs too.
    pub fn total(&self) -> usize {
        self.missing.len() + self.extra.len() + self.changed.len() + self.type_conflict.len()
    }

    /// True when nothing at all differs.
    ///
    /// Like [`Self::total`], blind to [`Self::identical`] and [`Self::verdicts`].
    /// This one is a separate predicate rather than a call to `total()` because it also
    /// consults `case_mismatch`, which `total()` deliberately ignores — so the two can
    /// be correct independently, and reporting "no differences" while a casing
    /// difference exists would be its own kind of wrong answer.
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.case_mismatch.is_empty()
    }

    /// The reason tag for `rel`, if the diff classified it.
    ///
    /// `None` for a path with no classification — a one-sided path, a type conflict,
    /// or a `CHANGED` entry added to the bucket by a caller rather than by the diff.
    /// The reporter treats that as "no reason to give" rather than inventing one,
    /// because a tag nobody derived is worse than no tag.
    pub fn why_tag(&self, rel: &str) -> Option<String> {
        self.verdicts.get(rel).map(Verdict::why_tag)
    }
}

/// What the diff concluded about one file-on-both-sides pair, and why.
///
/// The one predicate `diff_maps` uses in **both** of its key-matching branches, and
/// it returns a [`Verdict`] rather than a `bool` because two different questions were
/// being asked of one answer:
///
/// - is it **different**? — the `CHANGED` bucket, which is what `total()` and the
///   exit code read;
/// - is it **equal**, and on what evidence? — the `--show-identical` bucket, which
///   cannot be derived by negating the changed list: directories fall through
///   `diff_maps` into no bucket at all and are compared by presence, so the equal set
///   needs its own outcome and its own vocabulary.
///
/// Returning one value means the diff and the reason renderer read the same decision
/// rather than re-deriving it, and `Diff`'s buckets are filled from
/// [`Verdict::is_changed`] — so a classification and the tag printed for it cannot
/// disagree.
///
/// **Kinds must already agree.** A pair whose kinds differ is a `TYPE-CONFLICT`, which
/// is decided by the kind itself and needs no reason — so the caller checks that first
/// and this function is only reached for a same-kind pair. Directories are then
/// classified here rather than falling through in `diff_maps`, which is what makes
/// `--show-identical` complete: every both-sides pair gets a verdict, so there is no
/// path whose equality the flag could silently omit.
pub fn pair_verdict(
    stat: StatTrust,
    required: &Required,
    s: &EffRec,
    t: &EffRec,
    key: &str,
) -> Verdict {
    // Dirs compare by presence alone: no stat, no digest, nothing to read.
    if s.kind == "dir" {
        return Verdict::DirPresent;
    }
    // Evidence of difference, cheapest first: a stat field this run trusts, then the
    // digests the planner said would settle this pair.
    let trusted = stat.trusted_differs(s.size, s.mtime_ns, t.size, t.mtime_ns);
    if trusted.any() {
        let mut fields = trusted.fields();
        // `trusted.any()` guarantees at least one field, and `fields()` yields size
        // first — so the first is the one that settles the pair and `both` says
        // whether the other does too.
        let field = fields.next().expect("trusted.any() implies one field");
        return Verdict::Stat {
            field,
            both: fields.next().is_some(),
        };
    }
    let need = required.of(key);
    let differs = hashes_differ(&s.hashes, &t.hashes, need);
    if !differs.is_empty() {
        return Verdict::Differs { algos: differs };
    }
    // **A digest was consulted and it agreed.** Only reachable when `need` is
    // non-empty: `hashes_differ` over an empty list is silent, and silence over
    // nothing is not agreement.
    if !need.is_empty() {
        return Verdict::Matches {
            algos: need.to_vec(),
        };
    }
    // **No digest was consulted, and there was a difference we were told to ignore.**
    //
    // Reaching here with an empty `need` means the pair was left undecided with no way
    // left to decide it, so the honest verdict is `CHANGED` ("cannot confirm") rather
    // than silently clean. Reporting it equal is how a size-differing pair once came
    // back "identical" under `--hash-all-of none --no-trust-size`, and in `sync` that
    // meant `dst` was left stale by a run that reported success.
    //
    // Guarded on `need.is_empty()` deliberately. While a digest **is** available the
    // comparison above is a real verdict, and a pair whose content agrees is genuinely
    // in step — that is the whole reason `--no-trust-mtime` can *clear* a touched
    // file, and this clause must not take that away. The guard is what keeps this a fix
    // for an unanswerable request rather than a general pessimism.
    let untrusted = stat.untrusted_differs(s.size, s.mtime_ns, t.size, t.mtime_ns);
    if untrusted.any() {
        return Verdict::Unverifiable {
            fields: untrusted.fields().collect(),
        };
    }
    Verdict::StatMatch
}

/// Compare two effective maps.
///
/// In insensitive mode keys are matched by lowercase, so a casing-only
/// difference surfaces as `case_mismatch` instead of missing+extra. Dirs are
/// compared by presence alone.
///
/// `required` is the planner's per-path answer to "which algorithms settle this
/// pair", keyed by **src** path. Passing it rather than an algorithm list is what
/// lets one run settle different pairs by different algorithms: under
/// [`crate::planner::HashMode::AllOf`] every entry is the whole requested list, so a
/// uniform run would be described just as well by passing that list straight
/// through. Consulted only once stat has been found unconvincing, which is the same
/// boundary the planner applies — so a path that reaches it has an entry, **except
/// when the run requested no algorithm at all**. That exception used to be read as
/// "equal" and is now handled explicitly in [`pair_verdict`]; it is the one case where
/// `Required` has nothing to say and `hashes_differ`'s silence would otherwise be
/// mistaken for a verdict.
///
/// `stat` decides **which stat fields may settle a pair here**, and it is the same
/// value the planner was given — read from `plan.stat` rather than taken again, so
/// there is only one copy of the rule. That is the load-bearing part: this function's
/// short circuit and the planner's are the same predicate, and if they disagree then
/// one of two silent things happens — the planner hashes a pair whose digest is never
/// read, or (worse) a pair the planner called decided turns out to need a digest
/// nobody computed. One predicate is what makes that unrepresentable rather than
/// merely tested for.
///
/// Each both-sides pair is classified by [`pair_verdict`] — one value, of which
/// [`Verdict::is_changed`] and [`Verdict::is_identical`] decide the buckets here. The
/// classification itself is produced whether or not anything asks for it: a diff
/// that only computed a reason when a flag was on would be two diffs.
///
/// `want_identical` fills [`Diff::identical`], and only that. It is off by default
/// because the flag is — one record per equal file is a real cost in output volume,
/// and a caller who has not asked for it should not pay it or read it.
pub fn diff_maps(
    src: &HashMap<String, EffRec>,
    dst: &HashMap<String, EffRec>,
    required: &Required,
    stat: StatTrust,
    case_sensitive: bool,
    want_identical: bool,
) -> Diff {
    let mut d = Diff::default();
    if case_sensitive {
        let mut keys: HashSet<&String> = HashSet::new();
        for k in src.keys().chain(dst.keys()) {
            keys.insert(k);
        }
        for k in keys {
            match (src.get(k), dst.get(k)) {
                (Some(_s), None) => {
                    d.missing.push(k.clone());
                }
                (None, Some(_)) => {
                    d.extra.push(k.clone());
                }
                (Some(s), Some(t)) => {
                    if s.kind != t.kind {
                        d.type_conflict.push(k.clone());
                    } else {
                        let v = pair_verdict(stat, required, s, t, k);
                        if v.is_changed() {
                            d.changed.push(k.clone());
                        } else if want_identical {
                            d.identical.push(k.clone());
                        }
                        d.verdicts.insert(k.clone(), v);
                    }
                }
                (None, None) => {}
            }
        }
    } else {
        // lower -> original (mixed-case already validated absent within each side)
        let mut slow: HashMap<String, &String> = HashMap::new();
        let mut dlow: HashMap<String, &String> = HashMap::new();
        for k in src.keys() {
            slow.insert(k.to_lowercase(), k);
        }
        for k in dst.keys() {
            dlow.insert(k.to_lowercase(), k);
        }
        let mut keys: HashSet<String> = HashSet::new();
        for k in slow.keys().chain(dlow.keys()) {
            keys.insert(k.clone());
        }
        for lk in keys {
            match (slow.get(&lk), dlow.get(&lk)) {
                (Some(srel), None) => d.missing.push((*srel).clone()),
                (None, Some(trel)) => d.extra.push((*trel).clone()),
                (Some(srel), Some(trel)) => {
                    if *srel != *trel {
                        d.case_mismatch.push(((*srel).clone(), (*trel).clone()));
                    }
                    let s = &src[*srel];
                    let t = &dst[*trel];
                    if s.kind != t.kind {
                        d.type_conflict.push((*srel).clone());
                    } else {
                        let v = pair_verdict(stat, required, s, t, srel);
                        if v.is_changed() {
                            d.changed.push((*srel).clone());
                        } else if want_identical {
                            d.identical.push((*srel).clone());
                        }
                        d.verdicts.insert((*srel).clone(), v);
                    }
                }
                (None, None) => {}
            }
        }
    }
    d.missing.sort();
    d.extra.sort();
    d.changed.sort();
    d.type_conflict.sort();
    d.case_mismatch.sort();
    d.identical.sort();
    d
}

/// The algorithms that settle this pair and **disagree** between the two sides.
///
/// If either side lacks one of them this stays silent about that algorithm — the
/// caller has already consulted stat. That silence is safe *because* the planner
/// chose these algorithms: every one of them is obtainable on both sides, so it has
/// either been computed or was already cached. A digest that is absent here is a
/// digest nobody asked for, which is what makes this a comparison rather than a silent
/// fall-through to a verdict about content nobody read.
///
/// **The names, not a `bool`.** Under `--hash-any-of` the requested list and the list
/// that disagreed are different lists, and only this one says which digest settled
/// the pair — so a reason built from the request would name an algorithm that was
/// never compared. Empty means "no disagreement found", which covers both agreement
/// and silence; the caller tells them apart by whether it consulted anything.
pub fn hashes_differ(
    a: &HashMap<String, Vec<u8>>,
    b: &HashMap<String, Vec<u8>>,
    algos: &[String],
) -> Vec<String> {
    let mut differ: Vec<String> = Vec::new();
    for algo in algos {
        // If either side lacks the hash (e.g. --hash none history), fall back to
        // size+mtime which the caller already compared; do not force differ here.
        if let (Some(x), Some(y)) = (a.get(algo), b.get(algo))
            && x != y
        {
            differ.push(algo.clone());
        }
    }
    differ
}
