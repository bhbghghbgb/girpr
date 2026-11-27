//! The five global flags, declared once.
//!
//! Every program takes `--log-level`, `--log-file-level`, `--log-file`,
//! `--no-log-file` and `--output`. What differs is *policy*: whether the flags
//! are clap-global, the value name on `--output`, and — because `--help` bytes
//! are published — each program's descriptions for the flags and for the two
//! possible output values. Those differences are passed in as data; the clap
//! attributes, parser wiring and field types live only here.
//!
//! The declaration is an item-level macro rather than a flattened struct for
//! two reasons. The repair binary puts `--output` first in its help while the
//! two tools put it last, and a flattened struct would fix one order for all
//! three. Flattening would also nest the fields and change public struct
//! literals. Generating the struct keeps each call site's field layout, help
//! order and struct literals exactly as they were. The doc blocks are emitted
//! verbatim as `#[doc]` attributes, so clap's own short/long-help preprocessing
//! still produces the published `--help` and `-h` bytes.

use clap::builder::{PossibleValue, PossibleValuesParser, TypedValueParser};

use girpr_report_core::OutputFormat;

/// The `--output` parser with the caller's published possible-value
/// descriptions.
///
/// The value set comes from the shared core; only the two `help` strings are
/// the caller's, so a program's `--help` possible-values block stays byte for
/// byte its own. Validation is clap's `PossibleValuesParser`, which produces
/// the same invalid-value error shape as the derived `ValueEnum` parser it
/// replaces.
pub fn output_format_parser(
    text_help: &'static str,
    json_help: &'static str,
) -> impl TypedValueParser<Value = OutputFormat> + Clone + Send + Sync + 'static {
    PossibleValuesParser::new([
        PossibleValue::new("text").help(text_help),
        PossibleValue::new("json").help(json_help),
    ])
    .map(|value: String| match value.as_str() {
        "text" => OutputFormat::Text,
        "json" => OutputFormat::Json,
        _ => unreachable!("PossibleValuesParser yields only the two named values"),
    })
}

/// Define a struct made of the five global flags, in the caller's help order.
///
/// `order = output_first` matches the repair binary's argument order;
/// `order = log_first` matches both tools. Any trailing fields (`rest = { … }`)
/// are appended after the five, so the repair binary can declare its other
/// flags in the same struct. All descriptions are string literals passed
/// through as `#[doc]` attributes and possible-value help, unchanged.
#[macro_export]
macro_rules! global_args {
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident {
            order = output_first;
            global = $global:expr;
            output_value_name = $ovn:literal;
            docs = {
                log_level = $log_level_doc:literal,
                log_file_level = $log_file_level_doc:literal,
                log_file = $log_file_doc:literal,
                no_log_file = $no_log_file_doc:literal,
                output = $output_doc:literal $(,)?
            };
            possible_values = { text = $text_help:literal, json = $json_help:literal $(,)? };
            $(rest = { $($rest:tt)* })?
        }
    ) => {
        $(#[$attr])*
        $vis struct $name {
            #[doc = $output_doc]
            #[arg(
                long,
                value_name = $ovn,
                value_parser = $crate::output_format_parser($text_help, $json_help),
                default_value_t = <$crate::OutputFormat as ::std::default::Default>::default(),
                global = $global
            )]
            pub output: $crate::OutputFormat,

            #[doc = $log_level_doc]
            #[arg(
                long,
                value_name = "LV",
                default_value = "info",
                global = $global,
                value_parser = $crate::parse_level
            )]
            pub log_level: $crate::Level,

            #[doc = $log_file_level_doc]
            #[arg(
                long,
                value_name = "LV",
                global = $global,
                value_parser = $crate::parse_level
            )]
            pub log_file_level: Option<$crate::Level>,

            #[doc = $log_file_doc]
            #[arg(long, value_name = "FILE", global = $global)]
            pub log_file: Option<::std::path::PathBuf>,

            #[doc = $no_log_file_doc]
            #[arg(long, global = $global, conflicts_with = "log_file")]
            pub no_log_file: bool,

            $($($rest)*)?
        }
    };
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident {
            order = log_first;
            global = $global:expr;
            output_value_name = $ovn:literal;
            docs = {
                log_level = $log_level_doc:literal,
                log_file_level = $log_file_level_doc:literal,
                log_file = $log_file_doc:literal,
                no_log_file = $no_log_file_doc:literal,
                output = $output_doc:literal $(,)?
            };
            possible_values = { text = $text_help:literal, json = $json_help:literal $(,)? };
            $(rest = { $($rest:tt)* })?
        }
    ) => {
        $(#[$attr])*
        $vis struct $name {
            #[doc = $log_level_doc]
            #[arg(
                long,
                value_name = "LV",
                default_value = "info",
                global = $global,
                value_parser = $crate::parse_level
            )]
            pub log_level: $crate::Level,

            #[doc = $log_file_level_doc]
            #[arg(
                long,
                value_name = "LV",
                global = $global,
                value_parser = $crate::parse_level
            )]
            pub log_file_level: Option<$crate::Level>,

            #[doc = $log_file_doc]
            #[arg(long, value_name = "FILE", global = $global)]
            pub log_file: Option<::std::path::PathBuf>,

            #[doc = $no_log_file_doc]
            #[arg(long, global = $global, conflicts_with = "log_file")]
            pub no_log_file: bool,

            #[doc = $output_doc]
            #[arg(
                long,
                value_name = $ovn,
                value_parser = $crate::output_format_parser($text_help, $json_help),
                default_value_t = <$crate::OutputFormat as ::std::default::Default>::default(),
                global = $global
            )]
            pub output: $crate::OutputFormat,

            $($($rest)*)?
        }
    };
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use clap::Command;
    use clap::builder::TypedValueParser;

    use super::output_format_parser;
    use crate::OutputFormat;

    #[test]
    fn the_shared_parser_accepts_the_two_spellings_only() {
        let parser = output_format_parser("the text help", "the json help");
        let cmd = Command::new("probe");
        assert_eq!(
            parser
                .parse_ref(&cmd, None, OsStr::new("text"))
                .expect("text"),
            OutputFormat::Text
        );
        assert_eq!(
            parser
                .parse_ref(&cmd, None, OsStr::new("json"))
                .expect("json"),
            OutputFormat::Json
        );
        assert!(parser.parse_ref(&cmd, None, OsStr::new("yaml")).is_err());
    }

    #[test]
    fn the_possible_values_keep_the_callers_descriptions() {
        let parser = output_format_parser("caller text", "caller json");
        let values: Vec<_> = parser
            .possible_values()
            .expect("enumerated")
            .map(|pv| {
                (
                    pv.get_name().to_owned(),
                    pv.get_help().map(|h| h.to_string()),
                )
            })
            .collect();
        assert_eq!(
            values,
            [
                ("text".to_owned(), Some("caller text".to_owned())),
                ("json".to_owned(), Some("caller json".to_owned())),
            ]
        );
    }
}
