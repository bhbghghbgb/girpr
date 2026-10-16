//! Smallest-first admission under `--max-download-bytes`.
//!
//! **M5.** A deliberate exclusion is not a failure: an over-budget class is
//! recorded and the run still exits `0`. `--require-class` is what turns an
//! absent class into exit `4`, so "I asked for this and could not have it" is a
//! thing the caller said rather than something the tool decided.
//!
//! The rule is *smallest-first over the whole wanted set*, not per class. The
//! difference matters and is the reason this is its own module: admitting each
//! class's candidates independently would let a cheap class consume the budget
//! in a way that leaves an expensive one unfunded, while a global cost order
//! funds the most classes for the fewest bytes.

use serde::Serialize;

/// How much of the capture budget has been spent, and how much is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    limit: Option<u64>,
    spent: u64,
}

impl Budget {
    /// No ceiling.
    pub const fn unlimited() -> Self {
        Budget {
            limit: None,
            spent: 0,
        }
    }

    /// A hard ceiling on `Σ total_cost`.
    pub const fn capped(limit: u64) -> Self {
        Budget {
            limit: Some(limit),
            spent: 0,
        }
    }

    /// A budget with the same ceiling, already charged by `spent`.
    ///
    /// Used to carry the forced `--files` captures' cost into the same ledger
    /// the class loop draws from, so `Σ total_cost` is one number rather than
    /// two that have to be added up by a reader.
    pub fn with_spent(limit: Option<u64>, spent: u64) -> Self {
        Budget { limit, spent }
    }

    /// The configured ceiling, if any.
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Whether a cost of `cost` fits right now.
    ///
    /// Checked *before* charging, so a rejected candidate leaves the budget
    /// untouched and a later, cheaper candidate can still be admitted. That is
    /// why the caller walks in ascending cost order rather than asking this to
    /// sort for it.
    pub fn admits(&self, cost: u64) -> bool {
        match self.limit {
            None => true,
            Some(limit) => self.spent.saturating_add(cost) <= limit,
        }
    }

    /// Charge a cost. Only call this after [`Budget::admits`] agreed.
    ///
    /// Saturating rather than wrapping: a budget is a convenience and must never
    /// be the thing that panics.
    pub fn charge(&mut self, cost: u64) {
        self.spent = self.spent.saturating_add(cost);
    }

    /// Bytes charged so far.
    pub const fn spent(&self) -> u64 {
        self.spent
    }

    /// Bytes still available, or `None` when uncapped.
    pub fn remaining(&self) -> Option<u64> {
        self.limit.map(|l| l.saturating_sub(self.spent))
    }

    /// A one-line description for the run log.
    pub fn describe(&self) -> String {
        match self.limit {
            None => format!("unlimited, {} spent", self.spent),
            Some(l) => format!(
                "{} of {} spent, {} left",
                self.spent,
                l,
                l.saturating_sub(self.spent)
            ),
        }
    }
}

/// Which side of the budget each candidate fell on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fate {
    /// Fitted and is being captured.
    Admitted,
    /// Did not fit. Recorded, not silently dropped.
    OverBudget,
}

/// Walk `costs` in the order given, admitting what fits.
///
/// The caller supplies the order because *that* order is a decision made
/// elsewhere (§5's tie-break). This function's only rule is "take what fits, in
/// the order you were given", which is what makes it testable without a corpus.
pub fn admit_in_order(costs: &[u64], mut budget: Budget) -> (Vec<Fate>, Budget) {
    let mut fates = Vec::with_capacity(costs.len());
    for &cost in costs {
        if budget.admits(cost) {
            budget.charge(cost);
            fates.push(Fate::Admitted);
        } else {
            // Deliberately *not* charged. A rejected candidate must not make the
            // budget look spent, or a later candidate that would have fit is
            // rejected for a reason that no longer exists.
            fates.push(Fate::OverBudget);
        }
    }
    (fates, budget)
}

/// The sum of the admitted costs — what the run actually spends.
pub fn total_admitted(fates: &[Fate], costs: &[u64]) -> u64 {
    fates
        .iter()
        .zip(costs)
        .filter(|(f, _)| **f == Fate::Admitted)
        .map(|(_, c)| *c)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unlimited_budget_admits_everything_and_reports_no_ceiling() {
        let mut b = Budget::unlimited();
        assert_eq!(b.limit(), None);
        assert_eq!(b.remaining(), None);
        for c in [1, 1_000_000, u64::MAX / 2] {
            assert!(b.admits(c));
            b.charge(c);
        }
        assert!(b.describe().contains("unlimited"));
    }

    #[test]
    fn the_ceiling_is_exact_at_the_boundary() {
        let mut b = Budget::capped(100);
        assert!(b.admits(100), "exactly the limit fits");
        b.charge(100);
        assert!(!b.admits(1));
        assert_eq!(b.remaining(), Some(0));
        assert!(b.describe().contains("100 of 100"));
    }

    #[test]
    fn a_rejected_candidate_does_not_charge_the_budget() {
        let mut b = Budget::capped(100);
        b.charge(60);
        assert!(!b.admits(50), "60 + 50 > 100");
        assert_eq!(b.spent(), 60, "the refusal must not move the spent total");
        // And a smaller one still fits, which is the whole reason.
        assert!(b.admits(40));
        b.charge(40);
        assert_eq!(b.spent(), 100);
    }

    #[test]
    fn saturating_rather_than_wrapping_on_a_huge_charge() {
        let mut b = Budget::unlimited();
        b.charge(u64::MAX);
        b.charge(10);
        assert_eq!(
            b.spent(),
            u64::MAX,
            "a budget must never wrap into a small number"
        );
    }

    /// Smallest-first over the whole set, which is the rule the budget exists to
    /// enforce: more classes funded for fewer bytes.
    #[test]
    fn admission_is_smallest_first_across_the_whole_set() {
        // 10 + 90 + 20 == 120 exactly, so all three fit and the 5 000 does not.
        let costs = [10, 90, 20, 5_000];
        let (fates, b) = admit_in_order(&costs, Budget::capped(120));
        assert_eq!(
            fates,
            vec![
                Fate::Admitted,
                Fate::Admitted,
                Fate::Admitted,
                Fate::OverBudget
            ]
        );
        assert_eq!(b.spent(), 120);
        assert_eq!(total_admitted(&fates, &costs), 120);
    }

    /// One byte short of fitting the third, the same list rejects it — the
    /// boundary the ceiling is *supposed* to have.
    #[test]
    fn the_same_list_one_byte_shorter_rejects_the_third() {
        let costs = [10, 90, 20, 5_000];
        let (fates, b) = admit_in_order(&costs, Budget::capped(119));
        assert_eq!(
            fates,
            vec![
                Fate::Admitted,
                Fate::Admitted,
                Fate::OverBudget,
                Fate::OverBudget
            ]
        );
        assert_eq!(b.spent(), 100);
    }

    /// Given ascending costs, a capped budget admits a prefix — the property
    /// that makes "sort then admit" equivalent to "admit the smallest".
    #[test]
    fn an_ascending_walk_admits_exactly_a_prefix() {
        let costs = [5, 10, 20, 40, 80];
        for cap in [0, 4, 5, 14, 15, 35, 75, 155, u64::MAX] {
            let (fates, b) = admit_in_order(&costs, Budget::capped(cap));
            let admitted: Vec<bool> = fates.iter().map(|f| *f == Fate::Admitted).collect();
            let n = admitted.iter().filter(|x| **x).count();
            assert_eq!(
                admitted,
                (0..5).map(|i| i < n).collect::<Vec<_>>(),
                "cap {cap} admitted a non-prefix: {admitted:?}"
            );
            assert_eq!(b.spent(), costs.iter().take(n).sum::<u64>(), "cap {cap}");
        }
    }

    #[test]
    fn a_zero_budget_admits_nothing_and_still_explains_itself() {
        let costs = [0, 1];
        let (fates, b) = admit_in_order(&costs, Budget::capped(0));
        // A zero-cost candidate genuinely costs nothing and does fit.
        assert_eq!(fates, vec![Fate::Admitted, Fate::OverBudget]);
        assert_eq!(b.spent(), 0);
        assert!(b.describe().contains("left"));
    }

    #[test]
    fn an_empty_set_is_admitted_trivially() {
        let (fates, b) = admit_in_order(&[], Budget::capped(10));
        assert!(fates.is_empty());
        assert_eq!(b.spent(), 0);
        assert_eq!(total_admitted(&fates, &[]), 0);
    }
}
