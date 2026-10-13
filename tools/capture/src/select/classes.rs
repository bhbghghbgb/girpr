//! The matrix classes: one predicate per distinct decision the tool under test makes.
//!
//! **M5.** A class is a *branch*, not a file. Predicates overlap — a file can
//! satisfy several — which is why admission is greedy with an exclusion set rather
//! than nine independent picks, and why every captured file is tagged with all the
//! classes it satisfies.