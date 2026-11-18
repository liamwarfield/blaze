//! Port of `Converters`, `EnumConverter` and `BoolOrEnumConverter`.

use std::marker::PhantomData;
use std::time::Duration;

use regex::Regex;

use crate::converter::{ConversionContext, Converter, non_null};
use crate::exception::OptionsParsingException;
use crate::tags::TriState;
use crate::value::{OptionScalar, Value, java_list_to_string};

type ConvResult = Result<Option<Value>, OptionsParsingException>;

// Java: com/google/devtools/common/options/Converters.java:39 Converters.ENABLED_REPS
const ENABLED_REPS: [&str; 5] = ["true", "1", "yes", "t", "y"];
// Java: com/google/devtools/common/options/Converters.java:42 Converters.DISABLED_REPS
const DISABLED_REPS: [&str; 5] = ["false", "0", "no", "f", "n"];

/// Guava `Ascii.toLowerCase`.
pub fn ascii_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

// Java: com/google/devtools/common/options/Converters.java:46 Converters.BooleanConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct BooleanConverter;

impl BooleanConverter {
    // Java: com/google/devtools/common/options/Converters.java:48 Converters.BooleanConverter.convert
    pub fn convert_bool(&self, input: Option<&str>) -> Result<bool, OptionsParsingException> {
        let Some(input) = input else {
            return Ok(false);
        };
        let input = ascii_lower(input);
        if ENABLED_REPS.contains(&input.as_str()) {
            return Ok(true);
        }
        if DISABLED_REPS.contains(&input.as_str()) {
            return Ok(false);
        }
        Err(OptionsParsingException::new(format!(
            "'{}' is not a boolean",
            input
        )))
    }
}

impl Converter for BooleanConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_bool(input)?)))
    }
    // Java: com/google/devtools/common/options/Converters.java:63 Converters.BooleanConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a boolean".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:69 Converters.StringConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct StringConverter;

impl Converter for StringConverter {
    // Java: com/google/devtools/common/options/Converters.java:71 Converters.StringConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(input.map(|s| Value::new(s.to_string())))
    }
    fn get_type_description(&self) -> String {
        "a string".into()
    }
}

/// Java `Long.decode`, restricted to `[min, max]` (`Integer.decode` uses the int range).
pub fn java_decode(nm: &str, min: i64, max: i64) -> Option<i64> {
    if nm.is_empty() {
        return None;
    }
    let bytes = nm.as_bytes();
    let mut index = 0;
    let mut negative = false;
    match bytes[0] {
        b'-' => {
            negative = true;
            index += 1;
        }
        b'+' => index += 1,
        _ => {}
    }
    let rest = &nm[index..];
    let radix;
    if rest.starts_with("0x") || rest.starts_with("0X") {
        index += 2;
        radix = 16;
    } else if rest.starts_with('#') {
        index += 1;
        radix = 16;
    } else if rest.starts_with('0') && nm.len() > 1 + index {
        index += 1;
        radix = 8;
    } else {
        radix = 10;
    }
    let digits = &nm[index..];
    if digits.starts_with('-') || digits.starts_with('+') || digits.is_empty() {
        return None;
    }
    let magnitude = i128::from_str_radix(digits, radix).ok()?;
    let value = if negative { -magnitude } else { magnitude };
    if value < min as i128 || value > max as i128 {
        return None;
    }
    Some(value as i64)
}

/// Java `Integer.parseInt(s)` (radix 10).
pub fn java_parse_int(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    s.parse::<i32>().ok()
}

/// Java `Double.parseDouble`.
pub fn java_parse_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c <= ' ');
    if t.is_empty() {
        return None;
    }
    let unsigned = t.trim_start_matches(['+', '-']);
    if unsigned.len() + 1 < t.len() {
        return None;
    }
    let negative = t.starts_with('-');
    if unsigned == "NaN" {
        return Some(f64::NAN);
    }
    if unsigned == "Infinity" {
        return Some(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let body = unsigned
        .strip_suffix(['f', 'F', 'd', 'D'])
        .unwrap_or(unsigned);
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
        || !body.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    let v: f64 = body.parse().ok()?;
    Some(if negative { -v } else { v })
}

// Java: com/google/devtools/common/options/Converters.java:82 Converters.IntegerConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct IntegerConverter;

impl Converter for IntegerConverter {
    // Java: com/google/devtools/common/options/Converters.java:84 Converters.IntegerConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        match java_decode(input, i32::MIN as i64, i32::MAX as i64) {
            Some(v) => Ok(Some(Value::new(v as i32))),
            None => Err(OptionsParsingException::new(format!(
                "'{}' is not an int",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        "an integer".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:99 Converters.LongConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct LongConverter;

impl Converter for LongConverter {
    // Java: com/google/devtools/common/options/Converters.java:101 Converters.LongConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        match java_decode(input, i64::MIN, i64::MAX) {
            Some(v) => Ok(Some(Value::new(v))),
            None => Err(OptionsParsingException::new(format!(
                "'{}' is not a long",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        "a long integer".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:116 Converters.DoubleConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleConverter;

impl Converter for DoubleConverter {
    // Java: com/google/devtools/common/options/Converters.java:118 Converters.DoubleConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        match java_parse_double(input) {
            Some(v) => Ok(Some(Value::new(v))),
            None => Err(OptionsParsingException::new(format!(
                "'{}' is not a double",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        "a double".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:133 Converters.TriStateConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct TriStateConverter;

impl Converter for TriStateConverter {
    // Java: com/google/devtools/common/options/Converters.java:135 Converters.TriStateConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let Some(input) = input else {
            return Ok(Some(Value::new(TriState::AUTO)));
        };
        let input = ascii_lower(input);
        if input == "auto" {
            return Ok(Some(Value::new(TriState::AUTO)));
        }
        if ENABLED_REPS.contains(&input.as_str()) {
            return Ok(Some(Value::new(TriState::YES)));
        }
        if DISABLED_REPS.contains(&input.as_str()) {
            return Ok(Some(Value::new(TriState::NO)));
        }
        Err(OptionsParsingException::new(format!(
            "'{}' is not a boolean",
            input
        )))
    }
    fn get_type_description(&self) -> String {
        "a tri-state (auto, yes, no)".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:162 Converters.VoidConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct VoidConverter;

impl Converter for VoidConverter {
    // Java: com/google/devtools/common/options/Converters.java:164 Converters.VoidConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        match input {
            None | Some("null") => Ok(None),
            Some(input) => Err(OptionsParsingException::new(format!(
                "'{}' unexpected",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        String::new()
    }
}

// Java: com/google/devtools/common/options/Converters.java:178 Converters.DurationConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct DurationConverter;

impl DurationConverter {
    // Java: com/google/devtools/common/options/Converters.java:182 Converters.DurationConverter.convert
    pub fn convert_duration(&self, input: &str) -> Result<Duration, OptionsParsingException> {
        static DURATION_REGEX: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new("^([0-9]+)(d|h|m|s|ms|ns)$").unwrap());
        if input == "0" {
            return Ok(Duration::ZERO);
        }
        let Some(m) = DURATION_REGEX.captures(input) else {
            return Err(OptionsParsingException::new(format!(
                "Illegal duration '{}'.",
                input
            )));
        };
        let duration: u64 = m[1]
            .parse()
            .unwrap_or_else(|_| panic!("NumberFormatException: For input string: \"{}\"", &m[1]));
        let overflow = || -> Duration { panic!("ArithmeticException: long overflow") };
        Ok(match &m[2] {
            "d" => duration
                .checked_mul(86400)
                .map(Duration::from_secs)
                .unwrap_or_else(overflow),
            "h" => duration
                .checked_mul(3600)
                .map(Duration::from_secs)
                .unwrap_or_else(overflow),
            "m" => duration
                .checked_mul(60)
                .map(Duration::from_secs)
                .unwrap_or_else(overflow),
            "s" => Duration::from_secs(duration),
            "ms" => Duration::from_millis(duration),
            "ns" => Duration::from_nanos(duration),
            _ => panic!("This must not happen. Did you update the regex without the switch case?"),
        })
    }
}

impl Converter for DurationConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_duration(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "An immutable length of time.".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:240 Converters.joinEnglishList
/// Joins words as in English: "nothing", "one", "one or two", "one, two or three".
pub fn join_english_list<I: IntoIterator<Item = String>>(choices: I) -> String {
    let items: Vec<String> = choices.into_iter().collect();
    let mut buf = String::new();
    for (i, choice) in items.iter().enumerate() {
        if !buf.is_empty() {
            buf.push_str(if i + 1 < items.len() { ", " } else { " or " });
        }
        buf.push_str(choice);
    }
    if buf.is_empty() {
        "nothing".to_string()
    } else {
        buf
    }
}

// Java: com/google/devtools/common/options/Converters.java:253 Converters.SeparatedOptionListConverter
/// Converter for a list of options, separated by some separator character.
#[derive(Clone, Debug)]
pub struct SeparatedOptionListConverter {
    separator: char,
    separator_description: &'static str,
    allow_empty_values: bool,
    /// `SeparatedOptionSetConverter`: sort and dedupe the result.
    as_set: bool,
}

impl SeparatedOptionListConverter {
    pub const fn new(
        separator: char,
        separator_description: &'static str,
        allow_empty_values: bool,
    ) -> Self {
        SeparatedOptionListConverter {
            separator,
            separator_description,
            allow_empty_values,
            as_set: false,
        }
    }

    // Java: com/google/devtools/common/options/Converters.java:293 Converters.SeparatedOptionSetConverter
    pub const fn new_set(
        separator: char,
        separator_description: &'static str,
        allow_empty_values: bool,
    ) -> Self {
        SeparatedOptionListConverter {
            separator,
            separator_description,
            allow_empty_values,
            as_set: true,
        }
    }

    // Java: com/google/devtools/common/options/Converters.java:267 Converters.SeparatedOptionListConverter.convert
    pub fn convert_list(&self, input: &str) -> Result<Vec<String>, OptionsParsingException> {
        let result: Vec<String> = if input.is_empty() {
            vec![]
        } else {
            input.split(self.separator).map(str::to_string).collect()
        };
        let result = if !self.allow_empty_values && result.iter().any(String::is_empty) {
            if result.len() == 1 {
                vec![]
            } else {
                return Err(OptionsParsingException::new(format!(
                    "Empty values are not allowed as part of this {}",
                    self.get_type_description()
                )));
            }
        } else {
            result
        };
        if self.as_set {
            // Java: com/google/devtools/common/options/Converters.java:303 Converters.SeparatedOptionSetConverter.convert
            let mut r = result;
            r.sort();
            r.dedup();
            return Ok(r);
        }
        Ok(result)
    }
}

impl Converter for SeparatedOptionListConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list_of(self.convert_list(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        if self.as_set {
            format!("{}-separated set of options", self.separator_description)
        } else {
            format!("{}-separated list of options", self.separator_description)
        }
    }
}

macro_rules! separated_converter {
    ($(#[$m:meta])* $name:ident, $ctor:ident, $sep:expr, $desc:expr, $allow:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default)]
        pub struct $name;
        impl $name {
            pub fn convert_list(&self, input: &str) -> Result<Vec<String>, OptionsParsingException> {
                SeparatedOptionListConverter::$ctor($sep, $desc, $allow).convert_list(input)
            }
        }
        impl Converter for $name {
            fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
                SeparatedOptionListConverter::$ctor($sep, $desc, $allow).convert(input, c)
            }
            fn get_type_description(&self) -> String {
                SeparatedOptionListConverter::$ctor($sep, $desc, $allow).get_type_description()
            }
        }
    };
}

separated_converter!(
    // Java: com/google/devtools/common/options/Converters.java:319 Converters.CommaSeparatedOptionListConverter
    CommaSeparatedOptionListConverter,
    new,
    ',',
    "comma",
    true
);
separated_converter!(
    // Java: com/google/devtools/common/options/Converters.java:330 Converters.CommaSeparatedNonEmptyOptionListConverter
    CommaSeparatedNonEmptyOptionListConverter,
    new,
    ',',
    "comma",
    false
);
separated_converter!(
    // Java: com/google/devtools/common/options/Converters.java:342 Converters.ColonSeparatedOptionListConverter
    ColonSeparatedOptionListConverter,
    new,
    ':',
    "colon",
    true
);
separated_converter!(
    // Java: com/google/devtools/common/options/Converters.java:353 Converters.CommaSeparatedOptionSetConverter
    CommaSeparatedOptionSetConverter,
    new_set,
    ',',
    "comma",
    true
);

// Java: java/util/logging/Level.java Level (the seven standard levels)
/// `java.util.logging.Level`, as produced by `LogLevelConverter`.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogLevel {
    OFF,
    SEVERE,
    WARNING,
    INFO,
    FINE,
    FINER,
    FINEST,
}

impl OptionScalar for LogLevel {
    fn java_to_string(&self) -> String {
        format!("{:?}", self)
    }
}
crate::option_field_type!(LogLevel, "java.util.logging.Level", raw = LogLevel::OFF);

// Java: com/google/devtools/common/options/Converters.java:360 Converters.LogLevelConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct LogLevelConverter;

impl LogLevelConverter {
    // Java: com/google/devtools/common/options/Converters.java:362 Converters.LogLevelConverter.LEVELS
    pub const LEVELS: [LogLevel; 7] = [
        LogLevel::OFF,
        LogLevel::SEVERE,
        LogLevel::WARNING,
        LogLevel::INFO,
        LogLevel::FINE,
        LogLevel::FINER,
        LogLevel::FINEST,
    ];
}

impl Converter for LogLevelConverter {
    // Java: com/google/devtools/common/options/Converters.java:373 Converters.LogLevelConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        match java_parse_int(input) {
            Some(l) if (0..Self::LEVELS.len() as i32).contains(&l) => {
                Ok(Some(Value::new(Self::LEVELS[l as usize])))
            }
            _ => Err(OptionsParsingException::new(format!(
                "Not a log level: {}",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        format!("0 <= an integer <= {}", Self::LEVELS.len() - 1)
    }
}

// Java: com/google/devtools/common/options/Converters.java:389 Converters.StringSetConverter
/// Checks whether a string is part of a set of strings.
#[derive(Clone, Debug)]
pub struct StringSetConverter {
    values: Vec<String>,
}

impl StringSetConverter {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(values: I) -> Self {
        StringSetConverter {
            values: values.into_iter().map(Into::into).collect(),
        }
    }
}

impl Converter for StringSetConverter {
    // Java: com/google/devtools/common/options/Converters.java:400 Converters.StringSetConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if self.values.iter().any(|v| v == input) {
            return Ok(Some(Value::new(input.to_string())));
        }
        Err(OptionsParsingException::new(format!(
            "Not one of {}",
            java_list_to_string(self.values.iter().cloned())
        )))
    }
    fn get_type_description(&self) -> String {
        join_english_list(self.values.iter().cloned())
    }
}

// Java: com/google/devtools/common/options/RegexPatternOption.java:34 RegexPatternOption
/// A compiled regex option. Equality is based solely on the pattern string.
#[derive(Clone, Debug)]
pub struct RegexPatternOption {
    pattern: String,
    regex: Regex,
    full_match: Regex,
}

impl RegexPatternOption {
    // Java: com/google/devtools/common/options/RegexPatternOption.java:35 RegexPatternOption.create
    pub fn create(pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(&format!("(?s){}", pattern))?;
        let full_match = Regex::new(&format!("(?s)^(?:{})$", pattern))?;
        Ok(RegexPatternOption {
            pattern: pattern.to_string(),
            regex,
            full_match,
        })
    }

    /// `regexPattern().pattern()`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// `regexPattern()`, compiled with `DOTALL`.
    pub fn regex_pattern(&self) -> &Regex {
        &self.regex
    }

    /// `matcher().test(s)`: matches the entire input string against the pattern.
    pub fn matches(&self, s: &str) -> bool {
        self.full_match.is_match(s)
    }
}

impl PartialEq for RegexPatternOption {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
    }
}

impl OptionScalar for RegexPatternOption {
    fn java_to_string(&self) -> String {
        // AutoValue toString; the predicate's toString is not reproducible.
        format!("RegexPatternOption{{regexPattern={}}}", self.pattern)
    }
}
crate::option_field_type!(
    RegexPatternOption,
    "com.google.devtools.common.options.RegexPatternOption",
    raw = RegexPatternOption::create("").unwrap()
);

// Java: com/google/devtools/common/options/Converters.java:415 Converters.RegexPatternConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct RegexPatternConverter;

impl Converter for RegexPatternConverter {
    // Java: com/google/devtools/common/options/Converters.java:418 Converters.RegexPatternConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        match RegexPatternOption::create(input) {
            Ok(r) => Ok(Some(Value::new(r))),
            // Java prints PatternSyntaxException.getMessage(); the regex crate's text differs.
            Err(e) => Err(OptionsParsingException::new(format!(
                "Not a valid regular expression: {}",
                e
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        "a valid Java regular expression".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:434 Converters.RangeConverter
/// Checks whether an integer is in the given range.
#[derive(Clone, Copy, Debug)]
pub struct RangeConverter {
    pub min_value: i32,
    pub max_value: i32,
}

impl RangeConverter {
    pub const fn new(min_value: i32, max_value: i32) -> Self {
        RangeConverter {
            min_value,
            max_value,
        }
    }

    // Java: com/google/devtools/common/options/Converters.java:444 Converters.RangeConverter.convert
    pub fn convert_int(&self, input: &str) -> Result<i32, OptionsParsingException> {
        match java_parse_int(input) {
            Some(value) => {
                if value < self.min_value {
                    Err(OptionsParsingException::new(format!(
                        "'{}' should be >= {}",
                        input, self.min_value
                    )))
                } else if value < self.min_value || value > self.max_value {
                    Err(OptionsParsingException::new(format!(
                        "'{}' should be <= {}",
                        input, self.max_value
                    )))
                } else {
                    Ok(value)
                }
            }
            None => Err(OptionsParsingException::new(format!(
                "'{}' is not an int",
                input
            ))),
        }
    }
}

impl Converter for RangeConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_int(non_null(input))?)))
    }
    // Java: com/google/devtools/common/options/Converters.java:459 Converters.RangeConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        if self.min_value == i32::MIN {
            if self.max_value == i32::MAX {
                "an integer".into()
            } else {
                format!("an integer, <= {}", self.max_value)
            }
        } else if self.max_value == i32::MAX {
            format!("an integer, >= {}", self.min_value)
        } else {
            let min = if self.min_value < 0 {
                format!("({})", self.min_value)
            } else {
                self.min_value.to_string()
            };
            format!("an integer in {}-{} range", min, self.max_value)
        }
    }
}

// Java: com/google/devtools/common/options/Converters.java:483 Converters.AssignmentConverter
/// Converts `name=value` assignments into `(String, String)` (Java `Map.Entry`).
#[derive(Clone, Copy, Debug, Default)]
pub struct AssignmentConverter;

impl AssignmentConverter {
    // Java: com/google/devtools/common/options/Converters.java:486 Converters.AssignmentConverter.convert
    pub fn convert_entry(&self, input: &str) -> Result<(String, String), OptionsParsingException> {
        match input.find('=') {
            Some(pos) if pos > 0 => Ok((input[..pos].to_string(), input[pos + 1..].to_string())),
            _ => Err(OptionsParsingException::new(
                "Variable definitions must be in the form of a 'name=value' assignment",
            )),
        }
    }
}

impl Converter for AssignmentConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_entry(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "a 'name=value' assignment".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:504 Converters.StringToDoubleAssignmentConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct StringToDoubleAssignmentConverter;

impl Converter for StringToDoubleAssignmentConverter {
    // Java: com/google/devtools/common/options/Converters.java:509 Converters.StringToDoubleAssignmentConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let (k, v) = AssignmentConverter.convert_entry(non_null(input))?;
        // Java lets the NumberFormatException escape.
        let d = java_parse_double(&v)
            .unwrap_or_else(|| panic!("NumberFormatException: For input string: \"{}\"", v));
        Ok(Some(Value::new((k, d))))
    }
    fn get_type_description(&self) -> String {
        "a named float, 'name=value'".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:529 Converters.AssignmentToListOfValuesConverter.AllowEmptyKeys
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllowEmptyKeys {
    YES,
    NO,
}

// Java: com/google/devtools/common/options/Converters.java:525 Converters.AssignmentToListOfValuesConverter
/// Converts `[key=]value1[,..,valueN]` with the given key and value converters. The result is a
/// `(key, values)` pair built by `make`.
pub struct AssignmentToListOfValuesConverter<K: Converter, V: Converter> {
    pub key_converter: K,
    pub value_converter: V,
    pub allow_empty_keys: AllowEmptyKeys,
}

impl<K: Converter, V: Converter> AssignmentToListOfValuesConverter<K, V> {
    // Java: com/google/devtools/common/options/Converters.java:548 Converters.AssignmentToListOfValuesConverter.convert
    pub fn convert_parts(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<(Option<Value>, Vec<Option<Value>>), OptionsParsingException> {
        let pos = input.find('=');
        if self.allow_empty_keys == AllowEmptyKeys::NO && !matches!(pos, Some(p) if p > 0) {
            return Err(OptionsParsingException::new(
                "Must be in the form of a 'key=value[,value]' assignment",
            ));
        }
        let key = match pos {
            Some(p) if p > 0 => &input[..p],
            _ => "",
        };
        // Java: input.substring(pos + 1) with pos == -1 is the whole input.
        let rest = match pos {
            Some(p) => &input[p + 1..],
            None => input,
        };
        let mut values: Vec<&str> = rest.split(',').collect();
        if values.contains(&"") {
            if values.len() == 1 {
                values = vec![];
            } else {
                return Err(OptionsParsingException::new(
                    "Variable definitions must not contain empty strings or leading / trailing commas",
                ));
            }
        }
        let mut converted = Vec::new();
        for value in values {
            converted.push(
                self.value_converter
                    .convert(Some(value), conversion_context)?,
            );
        }
        Ok((
            self.key_converter.convert(Some(key), conversion_context)?,
            converted,
        ))
    }
}

// Java: com/google/devtools/common/options/Converters.java:582 Converters.StringToStringListConverter
/// `[name=]value1[,..,valueN]` into `(String, Vec<String>)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StringToStringListConverter;

impl StringToStringListConverter {
    pub fn convert_entry(
        &self,
        input: &str,
    ) -> Result<(String, Vec<String>), OptionsParsingException> {
        let inner = AssignmentToListOfValuesConverter {
            key_converter: StringConverter,
            value_converter: StringConverter,
            allow_empty_keys: AllowEmptyKeys::YES,
        };
        let (k, vs) = inner.convert_parts(input, None)?;
        Ok((
            k.map(|k| k.get::<String>()).unwrap_or_default(),
            vs.into_iter()
                .map(|v| v.map(|v| v.get::<String>()).unwrap_or_default())
                .collect(),
        ))
    }
}

impl Converter for StringToStringListConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_entry(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "a '[name=]value1[,..,valueN]' assignment".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:600 Converters.EnvVar
/// A request to set, inherit or unset an environment variable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EnvVar {
    Set { name: String, value: String },
    Inherit { name: String },
    Unset { name: String },
}

impl EnvVar {
    pub fn name(&self) -> &str {
        match self {
            EnvVar::Set { name, .. } | EnvVar::Inherit { name } | EnvVar::Unset { name } => name,
        }
    }
}

impl OptionScalar for EnvVar {
    fn java_to_string(&self) -> String {
        // Java record toString.
        match self {
            EnvVar::Set { name, value } => format!("Set[name={}, value={}]", name, value),
            EnvVar::Inherit { name } => format!("Inherit[name={}]", name),
            EnvVar::Unset { name } => format!("Unset[name={}]", name),
        }
    }
}
crate::option_field_type!(
    EnvVar,
    "com.google.devtools.common.options.Converters$EnvVar",
    raw = EnvVar::Inherit {
        name: String::new()
    }
);

// Java: com/google/devtools/common/options/Converters.java:627 Converters.EnvVarsConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct EnvVarsConverter;

impl Converter for EnvVarsConverter {
    // Java: com/google/devtools/common/options/Converters.java:630 Converters.EnvVarsConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        let pos = input.find('=');
        if input.is_empty() || input == "=" {
            return Err(OptionsParsingException::new(
                "Variable definitions must be in the form of a 'name=value', 'name', or '=name' assignment",
            ));
        }
        Ok(Some(Value::new(match pos {
            Some(0) => EnvVar::Unset {
                name: input[1..].to_string(),
            },
            None => EnvVar::Inherit {
                name: input.to_string(),
            },
            Some(p) => EnvVar::Set {
                name: input[..p].to_string(),
                value: input[p + 1..].to_string(),
            },
        })))
    }
    fn starlark_convertible(&self) -> bool {
        true
    }
    // Java: com/google/devtools/common/options/Converters.java:652 Converters.EnvVarsConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        match converted.downcast_ref::<EnvVar>() {
            Some(EnvVar::Set { name, value }) => format!("{}={}", name, value),
            Some(EnvVar::Inherit { name }) => name.clone(),
            Some(EnvVar::Unset { name }) => format!("={}", name),
            None => panic!(
                "IllegalArgumentException: EnvVarsConverter can only reverse EnvVar types, got: {}",
                converted.java_to_string()
            ),
        }
    }
    fn get_type_description(&self) -> String {
        "a 'name[=value]' assignment with an optional value part or the special syntax '=name' to unset a variable".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:681 Converters.PercentageConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct PercentageConverter;

impl Converter for PercentageConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        RangeConverter::new(0, 100).convert(input, c)
    }
    fn get_type_description(&self) -> String {
        RangeConverter::new(0, 100).get_type_description()
    }
}

// Java: com/google/devtools/common/options/Converters.java:688 Converters.OptionalPercentageConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct OptionalPercentageConverter;

impl OptionalPercentageConverter {
    pub const UNSET: &'static str = "-1";
}

impl Converter for OptionalPercentageConverter {
    // Java: com/google/devtools/common/options/Converters.java:698 Converters.OptionalPercentageConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        Ok(Some(Value::new(crate::value::OptionalInt(
            if input == Self::UNSET {
                None
            } else {
                Some(RangeConverter::new(0, 100).convert_int(input)?)
            },
        ))))
    }
    fn get_type_description(&self) -> String {
        "an integer".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:709 Converters.CaffeineSpecConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct CaffeineSpecConverter;

impl Converter for CaffeineSpecConverter {
    fn convert(&self, _input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        crate::unported!("Converters.CaffeineSpecConverter.convert (needs a CaffeineSpec port)")
    }
    fn get_type_description(&self) -> String {
        "Converts to a CaffeineSpec, or null if the input is empty".into()
    }
}

// Java: com/google/devtools/common/options/Converters.java:726 Converters.ByteSizeConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct ByteSizeConverter;

impl Converter for ByteSizeConverter {
    // Java: com/google/devtools/common/options/Converters.java:742 Converters.ByteSizeConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        static PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new("^(?P<value>[0-9]+)(?P<multiplier>[KMGT]?)$").unwrap()
        });
        let input = non_null(input);
        let invalid = || OptionsParsingException::new(format!("Invalid size: {}", input));
        let m = PATTERN.captures(input).ok_or_else(invalid)?;
        let mut value: i64 = m["value"].parse().map_err(|_| invalid())?;
        let mult = &m["multiplier"];
        if !mult.is_empty() {
            let multiplier: i64 = match mult {
                "K" => 1024,
                "M" => 1024 * 1024,
                "G" => 1024 * 1024 * 1024,
                "T" => 1024 * 1024 * 1024 * 1024,
                _ => unreachable!(),
            };
            value = value.checked_mul(multiplier).ok_or_else(invalid)?;
        }
        Ok(Some(Value::new(value)))
    }
    fn get_type_description(&self) -> String {
        "a size in bytes, optionally followed by a K, M, G or T multiplier".into()
    }
}

// ---------------------------------------------------------------------------------------------
// Enums.

/// A Java enum usable with [`EnumConverter`]. `java_to_string` (from [`OptionScalar`]) is the
/// enum's `toString()`; [`JavaEnum::name`] is `name()`.
pub trait JavaEnum: OptionScalar + Copy {
    /// `Class.getEnumConstants()`, in declaration order.
    fn values() -> &'static [Self];
    /// `Enum.name()`.
    fn name(&self) -> &'static str;
    /// Whether the Java enum implements `StarlarkValue`.
    fn is_starlark_value() -> bool {
        false
    }
}

// Java: com/google/devtools/common/options/EnumConverter.java:31 EnumConverter
/// Converts a string to an enum constant by comparing with each constant's `toString()`,
/// ignoring case.
pub struct EnumConverter<T: JavaEnum> {
    type_name: &'static str,
    _t: PhantomData<fn() -> T>,
}

impl<T: JavaEnum> EnumConverter<T> {
    pub const fn new(type_name: &'static str) -> Self {
        EnumConverter {
            type_name,
            _t: PhantomData,
        }
    }

    // Java: com/google/devtools/common/options/EnumConverter.java:51 EnumConverter.convert
    pub fn convert_enum(&self, input: &str) -> Result<T, OptionsParsingException> {
        for value in T::values() {
            if value.java_to_string().eq_ignore_ascii_case(input) {
                return Ok(*value);
            }
        }
        Err(OptionsParsingException::new(format!(
            "Not a valid {}: '{}' (should be {})",
            self.type_name,
            input,
            Converter::get_type_description(self)
        )))
    }
}

impl<T: JavaEnum> Converter for EnumConverter<T> {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_enum(non_null(input))?)))
    }
    // Java: com/google/devtools/common/options/EnumConverter.java:63 EnumConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        ascii_lower(&join_english_list(
            T::values().iter().map(|v| v.java_to_string()),
        ))
    }
    // Java: com/google/devtools/common/options/EnumConverter.java:69 EnumConverter.starlarkConvertible
    fn starlark_convertible(&self) -> bool {
        T::is_starlark_value()
    }
    // Java: com/google/devtools/common/options/EnumConverter.java:75 EnumConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        if !self.starlark_convertible() {
            panic!(
                "UnsupportedOperationException: class {} is not Starlark convertible. Implement StarlarkValue to enable this.",
                std::any::type_name::<T>()
            );
        }
        converted.get::<T>().name().to_lowercase()
    }
}

// Java: com/google/devtools/common/options/BoolOrEnumConverter.java:30 BoolOrEnumConverter
/// Converts both the standard boolean spellings and enum constants (enum values win).
pub struct BoolOrEnumConverter<T: JavaEnum> {
    inner: EnumConverter<T>,
    true_value: T,
    false_value: T,
}

impl<T: JavaEnum> BoolOrEnumConverter<T> {
    pub const fn new(type_name: &'static str, true_value: T, false_value: T) -> Self {
        BoolOrEnumConverter {
            inner: EnumConverter::new(type_name),
            true_value,
            false_value,
        }
    }

    // Java: com/google/devtools/common/options/BoolOrEnumConverter.java:53 BoolOrEnumConverter.convert
    pub fn convert_enum(&self, input: &str) -> Result<T, OptionsParsingException> {
        match self.inner.convert_enum(input) {
            Ok(v) => Ok(v),
            Err(e_enum) => match BooleanConverter.convert_bool(Some(input)) {
                Ok(true) => Ok(self.true_value),
                Ok(false) => Ok(self.false_value),
                Err(_) => Err(e_enum),
            },
        }
    }
}

impl<T: JavaEnum> Converter for BoolOrEnumConverter<T> {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_enum(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        self.inner.get_type_description()
    }
    fn starlark_convertible(&self) -> bool {
        self.inner.starlark_convertible()
    }
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        self.inner.reverse_for_starlark(converted)
    }
    fn is_bool_or_enum(&self) -> bool {
        true
    }
}

/// Declares a Java-style enum usable as an option type. Variants keep their Java names; a
/// variant may override `toString()` with `= "text"`.
///
/// ```ignore
/// option_enum! {
///     /// doc
///     pub enum CompilationMode { FASTBUILD = "fastbuild", DBG = "dbg", OPT = "opt" }
/// }
/// ```
#[macro_export]
macro_rules! option_enum {
    (
        $(#[$m:meta])*
        $vis:vis enum $name:ident { $( $variant:ident $(= $to_string:literal)? ),* $(,)? }
    ) => {
        $(#[$m])*
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name { $( $variant ),* }

        impl $crate::OptionScalar for $name {
            fn java_to_string(&self) -> ::std::string::String {
                match self {
                    $( $name::$variant => $crate::option_enum!(@str $variant $(, $to_string)?), )*
                }
                .to_string()
            }
        }

        impl $crate::converters::JavaEnum for $name {
            fn values() -> &'static [Self] {
                &[ $( $name::$variant ),* ]
            }
            fn name(&self) -> &'static str {
                match self {
                    $( $name::$variant => ::core::stringify!($variant), )*
                }
            }
        }

        $crate::option_field_type!(
            $name,
            ::core::stringify!($name),
            raw = [ $( $name::$variant ),* ][0]
        );
    };
    (@str $variant:ident) => { ::core::stringify!($variant) };
    (@str $variant:ident, $to_string:literal) => { $to_string };
}

/// Declares a named `EnumConverter` subclass: `enum_converter!(pub FooConverter, Foo, "foo");`
#[macro_export]
macro_rules! enum_converter {
    ($vis:vis $name:ident, $enum:ty, $type_name:expr) => {
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $name;
        impl $crate::Converter for $name {
            fn convert(
                &self,
                input: ::core::option::Option<&str>,
                ctx: ::core::option::Option<&$crate::ConversionContext>,
            ) -> ::core::result::Result<
                ::core::option::Option<$crate::Value>,
                $crate::OptionsParsingException,
            > {
                $crate::converters::EnumConverter::<$enum>::new($type_name).convert(input, ctx)
            }
            fn get_type_description(&self) -> ::std::string::String {
                $crate::converters::EnumConverter::<$enum>::new($type_name).get_type_description()
            }
            fn starlark_convertible(&self) -> bool {
                $crate::converters::EnumConverter::<$enum>::new($type_name).starlark_convertible()
            }
            fn reverse_for_starlark(&self, v: &$crate::Value) -> ::std::string::String {
                $crate::converters::EnumConverter::<$enum>::new($type_name).reverse_for_starlark(v)
            }
        }
    };
}

/// Declares a named `BoolOrEnumConverter` subclass:
/// `bool_or_enum_converter!(pub FooConverter, Foo, "foo", Foo::YES, Foo::NO);`
#[macro_export]
macro_rules! bool_or_enum_converter {
    ($vis:vis $name:ident, $enum:ty, $type_name:expr, $true_value:expr, $false_value:expr) => {
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $name;
        impl $name {
            fn inner() -> $crate::converters::BoolOrEnumConverter<$enum> {
                $crate::converters::BoolOrEnumConverter::new($type_name, $true_value, $false_value)
            }
        }
        impl $crate::Converter for $name {
            fn convert(
                &self,
                input: ::core::option::Option<&str>,
                ctx: ::core::option::Option<&$crate::ConversionContext>,
            ) -> ::core::result::Result<
                ::core::option::Option<$crate::Value>,
                $crate::OptionsParsingException,
            > {
                Self::inner().convert(input, ctx)
            }
            fn get_type_description(&self) -> ::std::string::String {
                Self::inner().get_type_description()
            }
            fn is_bool_or_enum(&self) -> bool {
                true
            }
        }
    };
}

// Java: com/google/devtools/common/options/OptionsParser.java:427 OptionsParser.HelpVerbosity
option_enum! {
    /// The verbosity with which option help messages are displayed.
    pub enum HelpVerbosity { LONG, MEDIUM, SHORT }
}

// Java: com/google/devtools/common/options/Converters.java:672 Converters.HelpVerbosityConverter
enum_converter!(pub HelpVerbosityConverter, HelpVerbosity, "--help_verbosity setting");
