//! The line mechanics the workspace's text readers share: one optional leading
//! UTF-8 BOM, 1-based physical line accounting, and blank-line skipping.
//!
//! This is a mechanics-only leaf. It deliberately decides **none** of the
//! policies the callers disagree about: what a BOM-only file means, which key
//! shape counts as a match, whether a malformed line is warned or refused, and
//! every error wording stay at the call site. Passing [`BomPolicy::Keep`] or
//! [`BomPolicy::Strip`] is how a caller opts into its own BOM contract; the
//! iterator never converges the two.

/// Whether one leading UTF-8 BOM is removed before iteration.
///
/// A caller that treats a BOM-only file as blank passes [`BomPolicy::Strip`];
/// one that must still see the U+FEFF passes [`BomPolicy::Keep`]. The choice is
/// a parameter because the programs using this leaf deliberately disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BomPolicy {
    Strip,
    Keep,
}

/// An iterator over a text's non-blank lines as `(1-based line, line)`.
///
/// The line number counts *every* physical line, blank ones included, so a
/// caller's warning or error points at the line the launcher actually wrote.
/// The yielded line has its line terminator removed but is **otherwise raw**:
/// trimming is left to the caller, because a column-zero-anchored key and an
/// indented one are different lines to some readers. Blank detection uses
/// `str::trim` only to decide whether a line carries any content.
pub struct NonBlankLines<'a> {
    lines: std::str::Lines<'a>,
    next_line: usize,
}

impl<'a> NonBlankLines<'a> {
    /// Start iterating `text`, honouring `bom` for a single leading U+FEFF.
    pub fn new(text: &'a str, bom: BomPolicy) -> Self {
        let text = match bom {
            BomPolicy::Strip => text.strip_prefix('\u{feff}').unwrap_or(text),
            BomPolicy::Keep => text,
        };
        Self {
            lines: text.lines(),
            next_line: 1,
        }
    }
}

impl<'a> Iterator for NonBlankLines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        for line in self.lines.by_ref() {
            let number = self.next_line;
            self.next_line += 1;
            if !line.trim().is_empty() {
                return Some((number, line));
            }
        }
        None
    }
}

/// Convenience constructor for [`NonBlankLines`].
pub fn non_blank_lines(text: &str, bom: BomPolicy) -> NonBlankLines<'_> {
    NonBlankLines::new(text, bom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_are_1_based_and_count_blank_lines() {
        let lines: Vec<_> = non_blank_lines("\nfirst\n  \n\t\nsecond\n", BomPolicy::Keep).collect();
        assert_eq!(lines, vec![(2, "first"), (5, "second")]);
    }

    #[test]
    fn a_bom_is_stripped_before_the_first_line_is_trimmed() {
        let lines: Vec<_> = non_blank_lines("\u{feff}Chinese\n", BomPolicy::Strip).collect();
        assert_eq!(lines, vec![(1, "Chinese")]);
    }

    #[test]
    fn keeping_the_bom_leaves_it_visible_to_the_caller() {
        // U+FEFF is not whitespace, so a BOM-only file yields one non-blank
        // line with `Keep` and none with `Strip`. The leaf never picks which
        // meaning the caller gives that difference.
        let kept: Vec<_> = non_blank_lines("\u{feff}", BomPolicy::Keep).collect();
        assert_eq!(kept, vec![(1, "\u{feff}")]);
        let stripped: Vec<_> = non_blank_lines("\u{feff}", BomPolicy::Strip).collect();
        assert!(stripped.is_empty());
    }

    #[test]
    fn the_yielded_line_is_raw_apart_from_its_terminator() {
        let lines: Vec<_> =
            non_blank_lines("\u{feff} a \r\n\r\n [General]\r\n", BomPolicy::Strip).collect();
        assert_eq!(lines, vec![(1, " a "), (3, " [General]")]);
    }
}
