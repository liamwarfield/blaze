//! Port of the build-setting part of `analysis/config/CoreOptionConverters.java`:
//! `BUILD_SETTING_CONVERTERS` (the converters that parse the string form of a Starlark build
//! setting value: on the command line, in `config_setting(flag_values)` and in platform `flags`),
//! its private `StarlarkIntConverter` and `StringSetConverter`, and `StarlarkInt.parse`.
//!
//! PORT: Java keys `BUILD_SETTING_CONVERTERS` by `packages.Type`; this crate does not depend on
//! the packages crate, so the map lookup (`BUILD_SETTING_CONVERTERS.get(type)`) is
//! `blaze_analysis::config::starlark_transition::build_setting_converter(type)`, which returns
//! one of the [`BuildSettingConverter`]s defined here. The option converters shared with native
//! flags (`LabelConverter`, `LabelListConverter`) stay in `blaze_optdefs::fragment::converters`.

use blaze_optdefs::fragment::converters::{LabelConverter, LabelListConverter};
use blaze_options::{
    BooleanConverter, CommaSeparatedOptionListConverter, ConversionContext, OptionsParsingException,
};

use crate::build_options::StarlarkOptionValue;

/// Java `NumberFormatException` (its message).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NumberFormatException(pub String);

// Java: net/starlark/java/eval/StarlarkInt.java:99 StarlarkInt.parse
/// Returns the int denoted by a literal string in the specified base, as if by the Starlark
/// expression `int(s, base)`.
///
/// PORT: Java's `StarlarkInt` is unbounded; a value beyond 64 bits fails loudly
/// (`unported!("StarlarkInt.parse")`).
pub fn starlark_int_parse(s: &str, base: i32) -> Result<i64, NumberFormatException> {
    let string_for_errors = s;
    let mut base = base;

    if s.is_empty() {
        return Err(NumberFormatException("empty string".to_string()));
    }

    // +/- prefix?
    let mut is_negative = false;
    let mut s = s;
    // Java: s.charAt(0) (a UTF-16 unit; '+' and '-' are single units).
    if let Some(rest) = s.strip_prefix('+') {
        s = rest;
    } else if let Some(rest) = s.strip_prefix('-') {
        s = rest;
        is_negative = true;
    }

    let mut digits = s;
    let mut prefix_stripped = false;

    // 0b 0o 0x prefix?
    let mut chars = s.chars();
    if let (Some('0'), Some(c)) = (chars.next(), chars.next()) {
        let prefix_base = match c {
            'b' | 'B' => 2,
            'o' | 'O' => 8,
            'x' | 'X' => 16,
            _ => 0,
        };
        if prefix_base != 0 {
            if base == 0 || base == prefix_base {
                base = prefix_base;
                digits = &s[2..]; // strip prefix
                prefix_stripped = true;
            }
        }
    }

    // No prefix, no base? Use decimal.
    if !prefix_stripped && base == 0 {
        // Don't infer base when input starts with '0' due to octal/decimal ambiguity.
        if s.chars().count() > 1 && s.starts_with('0') {
            return Err(NumberFormatException(format!(
                "cannot infer base when string begins with a 0: {}",
                blaze_starlark::printer::quote(string_for_errors)
            )));
        }
        base = 10;
    }
    if !(2..=36).contains(&base) {
        return Err(NumberFormatException(format!(
            "invalid base {} (want 2 <= base <= 36)",
            base
        )));
    }

    let invalid_literal = || {
        NumberFormatException(format!(
            "invalid base-{} literal: {}",
            base,
            blaze_starlark::printer::quote(string_for_errors)
        ))
    };

    // Do not allow Long.parseLong and new BigInteger to accept another +/- sign.
    if digits.starts_with('+') || digits.starts_with('-') {
        return Err(invalid_literal());
    }

    // Long.parseLong(digits, base), then new BigInteger(digits, base).
    if digits.is_empty() {
        return Err(invalid_literal());
    }
    let mut magnitude: i128 = 0;
    let mut beyond_64_bits = false;
    for c in digits.chars() {
        // Character.digit accepts no Latin-1 char beyond ASCII (U+00B2, U+00B3, U+00B9 and
        // U+00BC-U+00BE are numeric to Rust but not digits to Java): they are invalid literals.
        let digit = if c.is_ascii() {
            match c.to_digit(base as u32) {
                Some(d) => d,
                None => return Err(invalid_literal()),
            }
        } else if (c as u32 > 0xff && c.is_numeric()) || ('\u{ff21}'..='\u{ff5a}').contains(&c) {
            // PORT: Character.digit also accepts non-ASCII Unicode digits and fullwidth
            // letters; only ASCII digits are ported.
            blaze_util::unported!(
                "Character.digit",
                "non-ASCII digit {:?} in integer {:?}",
                c,
                string_for_errors
            )
        } else {
            return Err(invalid_literal());
        };
        if !beyond_64_bits {
            magnitude = magnitude * i128::from(base) + i128::from(digit);
            if magnitude > i128::from(u64::MAX) {
                beyond_64_bits = true;
            }
        }
    }
    let value = if is_negative { -magnitude } else { magnitude };
    if beyond_64_bits || value < i128::from(i64::MIN) || value > i128::from(i64::MAX) {
        blaze_util::unported!(
            "StarlarkInt.parse",
            "integer beyond 64 bits: {}",
            string_for_errors
        );
    }
    Ok(value as i64)
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:76 CoreOptionConverters.BUILD_SETTING_CONVERTERS
/// The converters used for `BuildSetting` value parsing (the values of
/// `BUILD_SETTING_CONVERTERS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildSettingConverter {
    /// `INTEGER`: `StarlarkIntConverter`.
    StarlarkInt,
    /// `BOOLEAN`: `Converters.BooleanConverter`.
    Boolean,
    /// `STRING`: `Converters.StringConverter`.
    String,
    /// `STRING_LIST`: `Converters.CommaSeparatedOptionListConverter`.
    CommaSeparatedOptionList,
    /// `STRING_SET`: `StringSetConverter`.
    StringSet,
    /// `LABEL` and `NODEP_LABEL`: `LabelConverter`.
    Label,
    /// `LABEL_LIST`: `LabelListConverter`.
    LabelList,
}

impl BuildSettingConverter {
    /// `converter.convert(input, conversionContext)`: the Java object the converter returns, in
    /// its `BuildOptions` form.
    pub fn convert(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<StarlarkOptionValue, OptionsParsingException> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:112 CoreOptionConverters.StarlarkIntConverter.convert
            BuildSettingConverter::StarlarkInt => {
                // Note that Starlark rule attribute values are currently restricted to the
                // signed 32-bit range, but Starlark-based flags may take on any integer value.
                match starlark_int_parse(input, 0) {
                    Ok(i) => Ok(StarlarkOptionValue::Int(i)),
                    Err(ex) => Err(OptionsParsingException::new(format!(
                        "invalid int: {}",
                        ex.0
                    ))),
                }
            }
            BuildSettingConverter::Boolean => Ok(StarlarkOptionValue::Bool(
                BooleanConverter.convert_bool(Some(input))?,
            )),
            BuildSettingConverter::String => Ok(StarlarkOptionValue::Str(input.to_string())),
            // An ImmutableList<String>.
            BuildSettingConverter::CommaSeparatedOptionList => Ok(StarlarkOptionValue::list(
                CommaSeparatedOptionListConverter
                    .convert_list(input)?
                    .into_iter()
                    .map(StarlarkOptionValue::Str)
                    .collect(),
            )),
            // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:98 CoreOptionConverters.StringSetConverter.convert
            // ImmutableSet.copyOf(result).
            BuildSettingConverter::StringSet => Ok(StarlarkOptionValue::set(
                CommaSeparatedOptionListConverter
                    .convert_list(input)?
                    .into_iter()
                    .map(StarlarkOptionValue::Str)
                    .collect(),
            )),
            BuildSettingConverter::Label => Ok(StarlarkOptionValue::Label(
                LabelConverter.convert_label(input, conversion_context)?,
            )),
            // An ImmutableList<Label>.
            BuildSettingConverter::LabelList => Ok(StarlarkOptionValue::list(
                LabelListConverter
                    .convert_list(input, conversion_context)?
                    .into_iter()
                    .map(StarlarkOptionValue::Label)
                    .collect(),
            )),
        }
    }

    /// `converter.getTypeDescription()`.
    pub fn get_type_description(&self) -> &'static str {
        match self {
            BuildSettingConverter::StarlarkInt => "an int",
            BuildSettingConverter::Boolean => "a boolean",
            BuildSettingConverter::String => "a string",
            BuildSettingConverter::CommaSeparatedOptionList => "comma-separated list of options",
            BuildSettingConverter::StringSet => "comma-separated set of strings",
            BuildSettingConverter::Label | BuildSettingConverter::LabelList => {
                "a build target label"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(s: &str) -> String {
        starlark_int_parse(s, 0).unwrap_err().0
    }

    // Expected values: net/starlark/java/eval/StarlarkInt.java:99 StarlarkInt.parse (the Java
    // algorithm and its messages) and src/test/java/net/starlark/java/eval/testdata/int.star
    // (`int("0x10", 0)`, `int("010", 0)`, `int("")`).
    #[test]
    fn starlark_int_parse_base_0() {
        assert_eq!(starlark_int_parse("7", 0), Ok(7));
        assert_eq!(starlark_int_parse("0", 0), Ok(0));
        assert_eq!(starlark_int_parse("-12", 0), Ok(-12));
        assert_eq!(starlark_int_parse("+12", 0), Ok(12));
        assert_eq!(starlark_int_parse("0x10", 0), Ok(16));
        assert_eq!(starlark_int_parse("0XfF", 0), Ok(255));
        assert_eq!(starlark_int_parse("-0x10", 0), Ok(-16));
        assert_eq!(starlark_int_parse("0o17", 0), Ok(15));
        assert_eq!(starlark_int_parse("0b101", 0), Ok(5));
        assert_eq!(starlark_int_parse("9223372036854775807", 0), Ok(i64::MAX));
        assert_eq!(starlark_int_parse("-9223372036854775808", 0), Ok(i64::MIN));
    }

    #[test]
    fn starlark_int_parse_errors() {
        assert_eq!(err(""), "empty string");
        assert_eq!(
            err("010"),
            "cannot infer base when string begins with a 0: \"010\""
        );
        assert_eq!(
            err("-010"),
            "cannot infer base when string begins with a 0: \"-010\""
        );
        assert_eq!(err("abc"), "invalid base-10 literal: \"abc\"");
        assert_eq!(err(" 5"), "invalid base-10 literal: \" 5\"");
        assert_eq!(err("5 "), "invalid base-10 literal: \"5 \"");
        assert_eq!(err("--5"), "invalid base-10 literal: \"--5\"");
        assert_eq!(err("-"), "invalid base-10 literal: \"-\"");
        assert_eq!(err("0x"), "invalid base-16 literal: \"0x\"");
        assert_eq!(err("0xg"), "invalid base-16 literal: \"0xg\"");
        assert_eq!(err("0b2"), "invalid base-2 literal: \"0b2\"");
    }

    #[test]
    fn starlark_int_parse_explicit_base() {
        assert_eq!(starlark_int_parse("ff", 16), Ok(255));
        assert_eq!(starlark_int_parse("0xff", 16), Ok(255));
        assert_eq!(starlark_int_parse("010", 8), Ok(8));
        assert_eq!(
            starlark_int_parse("1", 1).unwrap_err().0,
            "invalid base 1 (want 2 <= base <= 36)"
        );
    }

    // Expected values: the converters of CoreOptionConverters.BUILD_SETTING_CONVERTERS
    // (CommaSeparatedOptionListConverter: "" is the empty list; BooleanConverter's
    // representations) and the class-specific toString() of their results.
    #[test]
    fn build_setting_converters_convert() {
        assert_eq!(
            BuildSettingConverter::StarlarkInt.convert("0x10", None),
            Ok(StarlarkOptionValue::Int(16))
        );
        assert_eq!(
            BuildSettingConverter::StarlarkInt
                .convert("abc", None)
                .unwrap_err()
                .get_message(),
            "invalid int: invalid base-10 literal: \"abc\""
        );
        assert_eq!(
            BuildSettingConverter::Boolean.convert("0", None),
            Ok(StarlarkOptionValue::Bool(false))
        );
        assert_eq!(
            BuildSettingConverter::Boolean
                .convert("maybe", None)
                .unwrap_err()
                .get_message(),
            "'maybe' is not a boolean"
        );
        assert_eq!(
            BuildSettingConverter::String.convert("a,b", None),
            Ok(StarlarkOptionValue::Str("a,b".to_string()))
        );
        let list = BuildSettingConverter::CommaSeparatedOptionList
            .convert("a,b", None)
            .unwrap();
        assert_eq!(list.java_to_string(), "[a, b]");
        let empty = BuildSettingConverter::CommaSeparatedOptionList
            .convert("", None)
            .unwrap();
        assert_eq!(empty, StarlarkOptionValue::list(Vec::new()));
        let set = BuildSettingConverter::StringSet
            .convert("b,a,b", None)
            .unwrap();
        assert_eq!(set.java_to_string(), "[b, a]");
        assert_eq!(
            BuildSettingConverter::Label
                .convert("//pkg:t", None)
                .unwrap()
                .java_to_string(),
            "//pkg:t"
        );
    }
}
