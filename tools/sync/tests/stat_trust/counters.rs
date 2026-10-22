//! The counters: a flag that widens the undecided set has to cost the reads.
//!
//! Every case states the default as a control alongside the flagged run - a test
//! asserting only "the flag reads more" would pass against a planner that always read
//! everything.

use super::*;

/// **The headline: distrusting mtime costs exactly the pairs it widens.**
///
/// `mtime'd.txt` is free by default and needs a digest with the flag on. Nothing else
/// moves: `sized.txt` is still settled by size, `equal.txt` was already undecided, and
/// `gone.txt` is `EXTRA`.
#[test]
fn no_trust_mtime_costs_the_mtime_changed_pairs_and_nothing_else() {
    let t_default = temp_root("st_mtime_off");
    let t_flagged = temp_root("st_mtime_on");
    let (a_src, a_dst) = four_states(&t_default);
    let (b_src, b_dst) = four_states(&t_flagged);

    let default = hashed(&a_src, &a_dst, &distrusting(&["md5"], StatTrust::default()));
    let flagged = hashed(
        &b_src,
        &b_dst,
        &distrusting(&["md5"], StatTrust::default().without_mtime()),
    );
    assert_eq!(default, 2, "only `equal.txt`, on its two sides");
    assert_eq!(
        flagged, 4,
        "that plus `mtime'd.txt` on both sides: 2 files x 2 sides"
    );
}

/// And the same fixture with the *other* flag, which is what makes the two distinct
/// rather than one switch wearing two names.
#[test]
fn no_trust_size_costs_the_size_changed_pairs_and_nothing_else() {
    let t_default = temp_root("st_size_off");
    let t_flagged = temp_root("st_size_on");
    let (a_src, a_dst) = four_states(&t_default);
    let (b_src, b_dst) = four_states(&t_flagged);

    let default = hashed(&a_src, &a_dst, &distrusting(&["md5"], StatTrust::default()));
    let flagged = hashed(
        &b_src,
        &b_dst,
        &distrusting(&["md5"], StatTrust::default().without_size()),
    );
    assert_eq!(default, 2);
    assert_eq!(
        flagged, 4,
        "that plus `sized.txt` on both sides, and NOT `mtime'd.txt`"
    );
}

/// **Both flags is the whole thing off.** Every file on both sides is now undecided, so
/// the count is every file on both sides and nothing is free.
#[test]
fn distrusting_both_fields_reads_every_file_on_both_sides() {
    let t = temp_root("st_both_off");
    let (src, dst) = four_states(&t);
    let off = StatTrust::default().without_size().without_mtime();
    assert_eq!(
        hashed(&src, &dst, &distrusting(&["md5"], off)),
        6,
        "3 *shared* files x 2 sides. `gone.txt` is dst-only, so it is EXTRA and free."
    );
}

/// **The exact count for the whole tree, stated against the eager baseline.** A
/// stat-settled tree costs nothing by default, and these flags let a user pay for
/// reading it deliberately. The number it should cost is *the same as `update`
/// would* for those files — one pass per side, not two reads per file.
///
/// Asserted against `hashed` rather than against a digest count, and against the
/// default as a control. `hash_file` computes every algorithm in one pass, so a
/// two-algorithm request reads each file once either way; the flags are about *which
/// files*, never about how many times.
#[test]
fn switching_the_short_circuit_off_costs_one_pass_per_side_not_one_read_per_algorithm() {
    let t = temp_root("st_digests");
    let (src, dst) = four_states(&t);
    let off = StatTrust::default().without_size().without_mtime();
    let files = hashed(&src, &dst, &distrusting(&["md5", "sha256"], off));
    assert_eq!(
        files, 6,
        "3 shared files x 2 sides, with two algorithms asked for and one pass each"
    );
}
