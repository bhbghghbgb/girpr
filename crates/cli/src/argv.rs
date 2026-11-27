//! The full-argv incantation the in-process test runners and the recorded
//! manifests share.

use std::ffi::OsString;

/// Prepend a program name to an argument list, producing the full argv sequence
/// a `clap::Parser::try_parse_from` call or a recorded manifest expects.
///
/// The sentinel is supplied by the caller and is never chosen here: capture
/// records the whole vector in `MANIFEST.json → tool_argv`, so its spelling is
/// a byte contract. This helper only concatenates, in order and without
/// conversion beyond `Into<OsString>`, exactly what the caller hands it.
pub fn argv_with_program<P, I, S>(program: P, args: I) -> impl Iterator<Item = OsString>
where
    P: Into<OsString>,
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    std::iter::once(program.into()).chain(args.into_iter().map(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepends_the_caller_supplied_sentinel_in_order() {
        let argv: Vec<OsString> = argv_with_program("gircap", ["capture", "--out", "x"]).collect();
        let expected: Vec<OsString> = ["gircap", "capture", "--out", "x"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(argv, expected);
    }
}
