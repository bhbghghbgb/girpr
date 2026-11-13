use super::*;

fn write(dir: &girpr_tempdir::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn algos(names: &[&str]) -> AlgoSet {
    AlgoSet::from_iter(names.iter().map(|s| s.parse().unwrap()))
}

fn parse(input: &[&str]) -> Vec<HashAlg> {
    parse_hash_list(&input.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

fn parse_any(input: &[&str]) -> Vec<HashAlg> {
    parse_any_of(&input.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
}

// -- HashAlg --------------------------------------------------------------

/// Every name round-trips, and `as_str` is the inverse of `FromStr`. The
/// identity has to hold for the *whole* enum: a name with no parse, or a
/// parse with no name, is an algorithm the crate cannot store.
#[test]
fn every_algorithm_name_round_trips() {
    for a in HashAlg::ALL {
        assert_eq!(a.as_str().parse::<HashAlg>().unwrap(), a);
        assert_eq!(a.as_str().to_string().parse::<HashAlg>().unwrap(), a);
    }
}

/// Case-insensitive **on its own**. `--hash MD5` works today because
/// `parse_hash_list` lowercases before validating, which is a private detail
/// of one caller; a parser that only works when someone else has normalised
/// the input is a parser every new caller gets wrong.
#[test]
fn from_str_is_case_insensitive_without_help() {
    assert_eq!("MD5".parse::<HashAlg>().unwrap(), HashAlg::Md5);
    assert_eq!("Md5".parse::<HashAlg>().unwrap(), HashAlg::Md5);
    assert_eq!("SHA256".parse::<HashAlg>().unwrap(), HashAlg::Sha256);
    assert_eq!("Sha256".parse::<HashAlg>().unwrap(), HashAlg::Sha256);
}

/// **The resolver and the parser must not drift.** The resolver answers "which
/// algorithm does this name mean?" and the parser adds the refusal on top of
/// it, so the two have to agree about every name — including the ones neither
/// knows. A resolver that folded more than the parser would let a stored name
/// into an algorithm the command line would refuse.
#[test]
fn the_name_resolver_agrees_with_the_parser() {
    for a in HashAlg::ALL {
        assert_eq!(
            HashAlg::from_name(a.as_str()),
            Some(a),
            "the canonical name"
        );
        assert_eq!(
            HashAlg::from_name(&a.as_str().to_ascii_uppercase()),
            Some(a),
            "ASCII case does not matter"
        );
        assert_eq!(
            a.as_str().parse::<HashAlg>().ok(),
            HashAlg::from_name(a.as_str()),
            "the parser and the resolver answer one question"
        );
    }
    assert_eq!(HashAlg::from_name("blake3"), None, "a name nobody knows");
    assert_eq!(HashAlg::from_name(""), None, "and an empty one");
    assert_eq!(
        HashAlg::from_name("bla\u{212A}e3"),
        None,
        "the Kelvin sign is not a `k`: the fold is ASCII case only"
    );
}

/// **The resolver cannot allocate, and the compiler says so.**
///
/// The resolver is a `const fn`, and a const fn may not allocate: the
/// allocation-free property is the compiler's guarantee rather than a matter
/// of reading the body. Calling it in a const context is what pins that — a
/// resolver that stopped being `const` would not compile here.
///
/// The answers are checked in the same place, because a const context checks
/// them without a runtime.
const _: () = {
    assert!(matches!(HashAlg::from_name("md5"), Some(HashAlg::Md5)));
    assert!(matches!(HashAlg::from_name("MD5"), Some(HashAlg::Md5)));
    assert!(matches!(
        HashAlg::from_name("sha256"),
        Some(HashAlg::Sha256)
    ));
    assert!(matches!(
        HashAlg::from_name("SHA256"),
        Some(HashAlg::Sha256)
    ));
    assert!(matches!(HashAlg::from_name("blake3"), None));
    assert!(matches!(HashAlg::from_name(""), None));
};

/// **The resolver folds exactly as the ASCII-case comparison does.**
///
/// The resolver reaches its comparison through the *slice* form of
/// `eq_ignore_ascii_case`, because that is the one a `const fn` may call.
/// This case is the check that the slice form and the string form are the
/// same rule: the names are the awkward ones — every ASCII case of a
/// supported name, names nobody supports, the empty name, a name with a
/// space in it, and the Kelvin-sign lookalike.
#[test]
fn the_resolver_folds_exactly_as_the_ascii_case_comparison_does() {
    for name in [
        "md5",
        "MD5",
        "Md5",
        "mD5",
        "sha256",
        "SHA256",
        "Sha256",
        "sHA256",
        "blake3",
        "BLAKE3",
        "Blake3",
        "blake",
        "",
        "md5 ",
        " md5",
        "md-5",
        "bla\u{212A}e3",
        "MD5\u{212A}",
        "\u{212A}md5",
    ] {
        let want = HashAlg::ALL
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(name));
        assert_eq!(HashAlg::from_name(name), want, "the name {name:?}");
    }
}

/// Folding is ASCII-only, the same rule the path key and `HashAlg`'s parser
/// follow. Accepted spellings keep working, but a name containing a non-ASCII
/// character is refused as typed: `\u{212A}` is the Kelvin sign, which
/// `to_lowercase` maps onto `k`, so a Unicode fold would rewrite
/// `bla\u{212A}e3` to `blake3` before the error was built.
///
/// No supported name contains `k` today, so the lookalike cannot reach an
/// accepted spelling. The witness is `blake3` spelled with the Kelvin sign, which
/// is the likely first supported name containing `k`: if it is added, the same
/// case becomes an accepted-name lookalike and this test still pins the refusal.
#[test]
fn hash_names_fold_ascii_case_only() {
    assert_eq!(parse(&["MD5"]), vec![HashAlg::Md5]);
    assert_eq!(parse(&["Sha256"]), vec![HashAlg::Sha256]);
    assert_eq!(parse(&["NONE"]), Vec::<HashAlg>::new());

    let err = parse_hash_list(&["bla\u{212A}e3".to_string()]).unwrap_err();
    let err = err.to_string();
    assert!(
        err.contains("bla\u{212A}e3"),
        "the refusal keeps the typed spelling: {err}"
    );
    assert!(
        !err.contains("blake3"),
        "a Unicode fold must not rewrite the name first: {err}"
    );
    assert!(
        parse_any_of(&["bla\u{212A}e3".to_string()]).is_err(),
        "and any-of inherits the same refusal"
    );
    // The parser itself keeps the typed spelling too, without `parse_hash_list`
    // having lowercased the input first.
    assert_eq!(
        "bla\u{212A}e3".parse::<HashAlg>().unwrap_err().to_string(),
        "unsupported hash 'bla\u{212A}e3'"
    );
}

/// An unknown name is refused, and the refusal names the offender rather
/// than echoing it back bare — the caller supplies what would have been right.
#[test]
fn an_unknown_name_is_refused_and_named() {
    let err = "blake3".parse::<HashAlg>().unwrap_err();
    assert_eq!(err.to_string(), "unsupported hash 'blake3'");
    // Not part of the error: the supported list belongs to the message that
    // frames the refusal, and carrying it here too would be two spellings.
    assert!(!err.to_string().contains("md5"));
}

// -- AlgoSet --------------------------------------------------------------

/// Every subset of the enum, so "for all sets" is a real claim rather than
/// "for the two sets somebody thought of".
fn subsets() -> Vec<AlgoSet> {
    let mut out = vec![AlgoSet::NONE];
    for a in HashAlg::ALL {
        let mut next = out.clone();
        for s in &mut next {
            s.insert(a);
        }
        out.extend(next);
    }
    out
}

/// **The canonical-order claim**, which is what every downstream consumer of
/// `AlgoSet::iter` relies on. A set has no order of its own, so if this ever
/// yields anything but `HashAlg::ALL` order then a `--why` tag or a coverage
/// message becomes a function of a `HashMap`'s iteration order.
#[test]
fn iteration_is_always_canonical_order() {
    for set in subsets() {
        let got: Vec<HashAlg> = set.iter().collect();
        let want: Vec<HashAlg> = HashAlg::ALL
            .into_iter()
            .filter(|a| set.contains(*a))
            .collect();
        assert_eq!(got, want, "{set:?} iterates in ALL order");
    }
    assert_eq!(subsets().len(), 4, "NONE, md5, sha256, both");
}

/// `len` is a bit count, so it cannot disagree with `iter`. Both properties
/// are stated because `digest_count()` reports the first and the reason tag
/// is built from the second.
#[test]
fn the_length_is_the_number_iterated() {
    for set in subsets() {
        assert_eq!(set.len(), set.iter().count(), "{set:?}");
    }
}

/// `HashMode::AnyOf` settles a pair with exactly one algorithm, so this is
/// the shape the planner produces for every such path.
#[test]
fn a_single_algorithm_set_holds_exactly_that_one() {
    for a in HashAlg::ALL {
        let one = AlgoSet::one(a);
        assert_eq!(one.len(), 1);
        assert!(one.contains(a));
    }
}

/// `difference` is the "everything requested, minus what the row already
/// holds" the planner needs, so its identities and its non-narrowing case
/// are the whole contract.
#[test]
fn difference_subtracts_and_narrows_never_widens() {
    let both = AlgoSet::from_iter(HashAlg::ALL);
    assert_eq!(both.difference(AlgoSet::NONE), both, "identity");
    assert_eq!(AlgoSet::NONE.difference(AlgoSet::NONE), AlgoSet::NONE);
    assert!(
        AlgoSet::NONE.difference(both).is_empty(),
        "nothing asked for, nothing left"
    );
    assert_eq!(both.difference(AlgoSet::one(HashAlg::Md5)).len(), 1);
    assert!(
        both.difference(both).is_empty(),
        "everything already held, nothing owed"
    );
    // An `other` outside `self` cannot widen the result.
    assert_eq!(
        AlgoSet::one(HashAlg::Md5).difference(both),
        AlgoSet::NONE,
        "a set cannot gain a member by subtraction"
    );
}

/// **`Display` honours the width in the format spec.** A `Display` impl that
/// writes with `write_str` prints the string and silently drops the column,
/// which is how a coverage table loses its alignment with nothing failing.
#[test]
fn display_pads_to_the_width_in_the_format_spec() {
    for a in HashAlg::ALL {
        assert_eq!(format!("{a:>12}|"), format!("{:>12}|", a.as_str()));
        assert_eq!(format!("{a:<12}|"), format!("{:<12}|", a.as_str()));
        assert_eq!(format!("{a:012}|"), format!("{:012}|", a.as_str()));
        assert_eq!(
            format!("{a:<12} 0/1"),
            format!("{:<12} 0/1", a.as_str()),
            "and it is wider than the name, which is the only case that can tell"
        );
    }
}

/// **The memory claim, as an assertion.** This pins the replacement's size so a
/// later edit that adds a `HashMap` or a `Vec` field fails here rather than
/// quietly costing 25–50 MB on a 200k-file tree.
///
/// Derived, not guessed: a `Box<[T]>` fat pointer (16), plus `Option<[u8; 16]>` at
/// 17 and `Option<[u8; 32]>` at 33 — an `Option` over an array has no niche to
/// spare, so it costs a discriminant byte. Alignment rounds the total up to 72.
#[test]
fn a_digest_set_is_seventy_two_bytes_and_allocates_nothing() {
    assert_eq!(std::mem::size_of::<DigestSet>(), 72);
    assert!(
        DigestSet::default().foreign.is_empty(),
        "the foreign slice is empty and points nowhere"
    );
}

/// Round-trip through `insert_named` / `get` / `get_named`, for every subset of
/// the known algorithms. The identity is `get` against `get_named` — a row that
/// can be found one way and not the other is a row some caller will misread.
#[test]
fn every_subset_of_the_known_algorithms_round_trips() {
    for set in subsets() {
        let mut d = DigestSet::default();
        for a in set.iter() {
            d.insert_named(a.as_str(), &vec![0xABu8; a.digest_len()]);
        }
        assert_eq!(d.len(), set.len(), "{set:?}");
        for a in HashAlg::ALL {
            let want = set.contains(a).then(|| vec![0xABu8; a.digest_len()]);
            assert_eq!(d.get(a), want.as_deref(), "{set:?} get({a:?})");
            assert_eq!(
                d.get_algo(a),
                want.as_deref(),
                "{set:?} typed lookup for {a:?}"
            );
            assert_eq!(d.get_named(a.as_str()), want.as_deref(), "{set:?} named");
        }
        assert_eq!(
            d.iter().count(),
            set.len(),
            "{set:?}: iter sees every known digest and nothing else"
        );
        assert_eq!(d.is_empty(), set.is_empty());
    }
}

/// **The foreign arm, and the guarantee it exists for.** A name this build has
/// never heard of must be findable by name, must not be mistaken for a known
/// algorithm, and must not stop the known ones being found.
#[test]
fn an_unknown_algorithm_is_carried_verbatim_and_found_by_name() {
    let mut d = DigestSet::default();
    d.insert_named("blake3", b"\x07\x07\x07\x07");
    d.insert_named("md5", &[1u8; 16]);

    assert_eq!(d.get_named("blake3"), Some(&b"\x07\x07\x07\x07"[..]));
    assert_eq!(
        d.get(HashAlg::Md5),
        Some(&[1u8; 16][..]),
        "and it does not disturb the known slots"
    );
    assert_eq!(
        d.iter().count(),
        1,
        "iter reports known algorithms only -- a caller that wants the rest asks"
    );
    let foreign: Vec<_> = d.foreign_iter().collect();
    assert_eq!(foreign, vec![("blake3", &b"\x07\x07\x07\x07"[..])]);
    assert_eq!(
        d.len(),
        2,
        "and len counts it, because it is part of the row"
    );
}

/// **A known name with an unexpected length is still a digest.** A corrupt or
/// hand-written row that says `md5` with eight bytes must decode, must compare
/// byte-wise against whatever else claims to be an md5, and must therefore report
/// a difference rather than be refused or silently accepted.
///
/// This is exactly what a `HashMap` did: `x != y` on two unequal `Vec`s. Routing it
/// to `foreign` preserves that; rejecting it would turn a reported difference into a
/// hard failure.
#[test]
fn a_known_name_with_the_wrong_length_routes_to_foreign() {
    let mut d = DigestSet::default();
    d.insert_named("md5", b"\x01\x02\x03\x04\x05\x06\x07\x08");

    assert_eq!(
        d.get(HashAlg::Md5),
        None,
        "it is not this algorithm's digest"
    );
    assert_eq!(
        d.get_named("md5"),
        Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
        "but it is still the digest this row holds under that name"
    );
    assert_eq!(d.foreign_iter().count(), 1);
    assert_eq!(d.len(), 1);
    assert_ne!(
        d,
        DigestSet::default(),
        "and it differs from a row that has nothing, which is the comparison that reports it"
    );
}

/// **A known name is found whatever its ASCII case.** A row may spell the
/// algorithm `MD5` rather than `md5` — a hand-written row, a converted cache, a
/// newer build — and it is the same algorithm: the name *parses* to
/// [`HashAlg::Md5`], so both the typed lookup and the named one find it.
///
/// Without this, the two lookups disagree about one row: the typed lookup
/// matches the canonical spelling only, and the named lookup matches the
/// spelling it was given. A row found by one and not the other is a row some
/// caller will misread — the planner sees a row it believes holds nothing.
#[test]
fn a_known_name_in_the_foreign_arm_is_found_under_any_ascii_case() {
    let mut d = DigestSet::default();
    d.insert_named("MD5", b"\x01\x02\x03\x04\x05\x06\x07\x08");

    assert_eq!(
        d.get(HashAlg::Md5),
        None,
        "the slot is untouched: this is not this algorithm's digest"
    );
    assert_eq!(
        d.get_algo(HashAlg::Md5),
        Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
        "the typed lookup finds it by name"
    );
    assert_eq!(
        d.get_named("md5"),
        Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
        "and the named lookup agrees with it"
    );
    assert_eq!(
        d.get_named("MD5"),
        Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
        "the spelling it was stored under, too"
    );
}

/// **An unknown name stays exact.** A name this build has never heard of is not
/// an algorithm, so no known-algorithm lookup may match it, and the name is
/// matched by the spelling it was given and no other.
#[test]
fn an_unknown_name_is_matched_exactly_and_by_no_algorithm() {
    let mut d = DigestSet::default();
    d.insert_named("blake3", &[7u8; 32]);

    assert_eq!(
        d.get_named("blake3"),
        Some(&[7u8; 32][..]),
        "the exact spelling finds it"
    );
    assert_eq!(
        d.get_named("BLAKE3"),
        None,
        "and no other spelling does: an unknown name is not folded"
    );
    assert_eq!(
        d.get_algo(HashAlg::Md5),
        None,
        "a known algorithm's lookup does not match it either"
    );
    assert_eq!(d.get_algo(HashAlg::Sha256), None);
}

/// **The routing rule is unchanged by the widening.** A known name with the
/// length its algorithm expects goes to its slot, whatever its case — `MD5`
/// with sixteen bytes is the md5 slot, not a second copy of it somewhere else.
#[test]
fn a_known_name_with_the_right_length_still_goes_to_its_slot() {
    let mut d = DigestSet::default();
    d.insert_named("MD5", &[1u8; 16]);

    assert_eq!(
        d.get(HashAlg::Md5),
        Some(&[1u8; 16][..]),
        "the slot holds it"
    );
    assert_eq!(d.foreign_iter().count(), 0, "and the foreign arm is empty");
    assert_eq!(d.get_algo(HashAlg::Md5), Some(&[1u8; 16][..]));
    assert_eq!(d.get_named("md5"), Some(&[1u8; 16][..]));
}

/// **The fold is ASCII case only**, the same rule the parser and the path key
/// follow. A Unicode fold would map the Kelvin sign `\u{212A}` onto `k`, so
/// `blake3` and `bla\u{212A}e3` would be one name — two spellings the
/// filesystem keeps apart.
///
/// The witness is `blake3`, spelled with the Kelvin sign: it is the likely first
/// supported name containing a `k`, and if it is added this same case becomes an
/// accepted-name lookalike, which this test still pins. No supported name
/// contains a `k` today, so this cannot reach an accepted spelling yet — which
/// is why both directions are stated.
#[test]
fn a_non_ascii_lookalike_name_is_never_matched() {
    let mut d = DigestSet::default();
    d.insert_named("blake3", &[7u8; 32]);
    assert_eq!(
        "bla\u{212A}e3".to_lowercase(),
        "blake3",
        "the witness: a Unicode fold would merge these two spellings"
    );
    assert_eq!(
        d.get_named("bla\u{212A}e3"),
        None,
        "and the lookup keeps them apart"
    );

    let mut d = DigestSet::default();
    d.insert_named("bla\u{212A}e3", &[7u8; 32]);
    assert_eq!(
        d.get_named("blake3"),
        None,
        "in either direction: the stored spelling is not the one it resembles"
    );
    assert_eq!(
        d.get_named("bla\u{212A}e3"),
        Some(&[7u8; 32][..]),
        "and it is still found under the spelling it was stored with"
    );
    assert_eq!(
        d.get_algo(HashAlg::Md5),
        None,
        "no known algorithm's lookup reaches it"
    );
    assert_eq!(d.get_algo(HashAlg::Sha256), None);
}

/// The typed lookup keeps the by-name fallback: a known algorithm whose digest
/// was carried in `foreign` because its length was unexpected is still the digest
/// the row holds under that name, and still compares byte-wise.
#[test]
fn a_typed_lookup_finds_a_known_name_carried_in_foreign() {
    let mut d = DigestSet::default();
    d.insert_named("md5", b"\x01\x02\x03\x04\x05\x06\x07\x08");
    d.insert_named("sha256", &[3u8; 32]);

    assert_eq!(
        d.get_algo(HashAlg::Md5),
        Some(&b"\x01\x02\x03\x04\x05\x06\x07\x08"[..]),
        "the malformed entry is the digest this row holds under that name"
    );
    assert_eq!(d.get_algo(HashAlg::Sha256), Some(&[3u8; 32][..]));
    assert_eq!(
        DigestSet::default().get_algo(HashAlg::Md5),
        None,
        "a row with no digest under that name still reads as absent"
    );
}

/// `merge` is the write rule: computed digests win, everything else rides along.
/// Stated over all three shapes at once — a name only one side holds, a name both
/// hold, and a foreign name — because "rides along" and "wins" are two different
/// promises and a merge that got one right could still get the other wrong.
#[test]
fn merging_keeps_what_was_not_recomputed_and_prefers_what_was() {
    let mut prior = DigestSet::default();
    prior.insert_named("sha256", &[2u8; 32]);
    prior.insert_named("blake3", &[7u8; 32]);

    let mut computed = DigestSet::default();
    computed.insert_named("md5", &[1u8; 16]);
    computed.insert_named("sha256", &[9u8; 32]);
    computed.insert_named("blake3", &[8u8; 32]);

    let mut row = prior.clone();
    row.merge(&computed);

    assert_eq!(
        row.get(HashAlg::Md5),
        Some(&[1u8; 16][..]),
        "newly computed"
    );
    assert_eq!(
        row.get(HashAlg::Sha256),
        Some(&[9u8; 32][..]),
        "recomputed: the new value wins"
    );
    assert_eq!(
        row.get_named("blake3"),
        Some(&[8u8; 32][..]),
        "and a foreign digest merges like any other"
    );
    assert_eq!(row.foreign_iter().count(), 1, "not appended beside itself");

    // The other direction is the real invariant: merging something empty changes
    // nothing, which is what a `--hash none` scan writes.
    let mut untouched = prior.clone();
    untouched.merge(&DigestSet::default());
    assert_eq!(untouched, prior);
}

/// **The memory claim, as an assertion.** One `AlgoSet` sits in
/// `HashPlan.by_rel` for every path the run will read, and one in
/// `Required.by_rel` for every undecided pair — and under `--hash-all-of`
/// every entry of the latter is the same value. As a `Vec` that is a heap
/// allocation per path holding a fresh clone of the run-wide request; as two
/// bytes it is part of a map value that already exists.
#[test]
fn a_set_is_two_bytes() {
    assert_eq!(std::mem::size_of::<AlgoSet>(), 2);
}

/// `Debug` is a log-line contract. `info!(algos = ?algos)` prints in the
/// span of all four subcommands, and it renders `["md5", "sha256"]` today; a
/// derive on either type would rewrite every one of those lines to
/// `[Md5, Sha256]` without failing a single test elsewhere.
#[test]
fn debug_renders_the_spellings_the_logs_already_carry() {
    assert_eq!(format!("{:?}", AlgoSet::NONE), "[]");
    assert_eq!(format!("{:?}", AlgoSet::one(HashAlg::Md5)), "[\"md5\"]");
    assert_eq!(
        format!("{:?}", AlgoSet::from_iter(HashAlg::ALL)),
        "[\"md5\", \"sha256\"]"
    );
    assert_eq!(format!("{:?}", HashAlg::Md5), "\"md5\"");
}

/// A repeated algorithm is **one** request, however the flags were typed.
///
/// The non-adjacent case comes first because `Vec::dedup` only collapses
/// *consecutive* repeats: `md5 sha256 md5` must still count as three algorithms
/// and reach `HashPlan::digest_count` as a promise of three digests.
#[test]
fn a_repeated_algorithm_is_one_request() {
    assert_eq!(
        parse(&["md5", "sha256", "md5"]),
        vec![HashAlg::Md5, HashAlg::Sha256]
    );
    assert_eq!(
        parse(&["md5", "md5", "sha256", "md5", "sha256"]),
        vec![HashAlg::Md5, HashAlg::Sha256]
    );
}

/// First-seen order is the order kept. **This is the property `pick_one`
/// depends on** — `--hash-any-of`'s tie-break is the user's flag order, so a
/// dedup that sorted, or that kept the *last* occurrence, would change which
/// algorithm a pair is settled by.
#[test]
fn first_seen_order_is_the_order_kept() {
    assert_eq!(
        parse(&["sha256", "md5", "sha256"]),
        vec![HashAlg::Sha256, HashAlg::Md5]
    );
    assert_eq!(
        parse(&["sha256", "md5"]),
        vec![HashAlg::Sha256, HashAlg::Md5]
    );
}

/// The adjacent case `Vec::dedup` already handled, pinned so the fix above
/// cannot be read as "dedup was always broken".
#[test]
fn adjacent_repeats_collapse() {
    assert_eq!(parse(&["md5", "md5"]), vec![HashAlg::Md5]);
    assert_eq!(
        parse(&["sha256", "md5", "md5"]),
        vec![HashAlg::Sha256, HashAlg::Md5]
    );
}

/// `parse_any_of` delegates, so it inherits the dedup. Worth pinning because
/// it is the mode where a duplicate mattered most: the flag asks for *one*
/// algorithm, and a repeated one had nothing to change.
#[test]
fn any_of_dedupes_through_the_delegate() {
    assert_eq!(parse_any(&["md5", "md5"]), vec![HashAlg::Md5]);
    assert_eq!(
        parse_any(&["md5", "sha256", "md5"]),
        vec![HashAlg::Md5, HashAlg::Sha256]
    );
}

/// The refusal names the offender and the alternatives, in the wording the
/// README documents. `UnknownHash` carries only the name and this framing
/// supplies the rest, so there is exactly one place that sentence is written.
#[test]
fn an_unsupported_name_is_refused_with_the_supported_list() {
    let err = parse_hash_list(&["bogus".to_string()]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "unsupported hash 'bogus' (supported: md5,sha256 or none)"
    );
    // The first unsupported name is the one named, whatever follows it.
    let err = parse_hash_list(&[
        "md5".to_string(),
        "nope".to_string(),
        "also-nope".to_string(),
    ])
    .unwrap_err();
    assert!(
        err.to_string().contains("'nope'"),
        "the first refusal, not the last: {err}"
    );
}

/// **The property the whole split rests on.** `copy_and_digest` folds the
/// digest into the copy loop and `hash_file` keeps its own; if those two
/// ever disagreed, every copy in a real sync would fail its verify — or,
/// worse, the copy path would stop being checked at all.
///
/// Fed in every awkward shape on purpose: one byte at a time (the split
/// lands mid-digest), the whole file at once, and a chunk larger than the
/// file. The digest must not care how the bytes were divided, and only the
/// first case proves it is reading `update` and not something incidental.
#[test]
fn a_digest_is_the_same_however_the_bytes_were_split() {
    let dir = girpr_tempdir::TempDir::new("girsync_hash", "split");
    // Longer than `CHUNK` is not practical here, but it must exceed one
    // buffer's worth of a *small* file to make "chunk bigger than the file"
    // a real case rather than a trivially-true one.
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let path = write(&dir, "f.bin", &body);

    for set in [
        algos(&["md5", "sha256"]),
        algos(&["md5"]),
        algos(&["sha256"]),
    ] {
        let reference = hash_file(&path, &set).unwrap();

        let mut by_byte = Digests::new(&set);
        for b in &body {
            by_byte.update(std::slice::from_ref(b));
        }
        assert_eq!(
            by_byte.finish(),
            reference,
            "{set:?}: one byte per update is the split hash_file never uses"
        );

        let mut whole = Digests::new(&set);
        whole.update(&body);
        assert_eq!(whole.finish(), reference, "{set:?}: one update");

        let mut oversized = Digests::new(&set);
        oversized.update(&body[..body.len() / 2]);
        oversized.update(&body[body.len() / 2..]);
        assert_eq!(oversized.finish(), reference, "{set:?}: uneven halves");
    }
}

/// An empty file is the degenerate case of the same property: a digest loop
/// that never calls `update` has still absorbed every byte there is.
#[test]
fn an_empty_file_hashes_to_the_digest_of_nothing() {
    let dir = girpr_tempdir::TempDir::new("girsync_hash", "empty");
    let path = write(&dir, "f.bin", b"");

    let set = algos(&["md5", "sha256"]);
    let never_fed = Digests::new(&set);
    assert_eq!(never_fed.finish(), hash_file(&path, &set).unwrap());
}

/// `Digests::new(&[])` must be the inert object, because `--hash none` is
/// the mode where a copy never builds one. A context built and thrown away is
/// the per-file allocation [`hash_file`] is written to avoid.
#[test]
fn no_algorithms_asked_for_means_no_context_and_no_digest() {
    let mut d = Digests::new(&AlgoSet::NONE);
    d.update(b"these bytes must not be absorbed by anything");
    assert!(d.finish().is_empty());
    // ...and `hash_file` must agree, without opening the file at all.
    let dir = girpr_tempdir::TempDir::new("girsync_hash", "absent");
    assert!(
        hash_file(&dir.join("not-there.bin"), &AlgoSet::NONE)
            .unwrap()
            .is_empty(),
        "an empty request is answered without touching the disk"
    );
}
