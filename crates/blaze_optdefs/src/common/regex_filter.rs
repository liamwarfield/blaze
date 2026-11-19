//! Port of `lib/util/RegexFilter.java`.

use std::hash::{Hash, Hasher};

use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, RegexPatternOption, Value,
    converter::non_null, option_field_type,
};

use crate::fragment::fragment_options::java_string_compare;

type ConvResult = Result<Option<Value>, OptionsParsingException>;

/// A compiled `java.util.regex.Pattern` (no flags): the Java pattern string plus a compiled
/// matcher.
///
/// PORT: Java regex is emulated with the `regex` crate (through `blaze_options`'
/// `RegexPatternOption`, compiled without flags; its docs list the `regex` crate's limits). Atomic
/// groups `(?>...)` (unsupported by the `regex` crate) are compiled as non-capturing groups: for
/// `find()`/`matches()` of a whole alternative they accept the same inputs.
#[derive(Clone, Debug)]
pub struct JavaPattern {
    pattern: String,
    compiled: RegexPatternOption,
}

impl JavaPattern {
    /// `Pattern.compile(pattern)`; the error is the syntax error's message.
    pub fn compile(pattern: &str) -> Result<JavaPattern, String> {
        let translated = pattern.replace("(?>", "(?:");
        // Pattern.compile(pattern): no flags.
        let compiled = RegexPatternOption::compile(&translated, 0).map_err(|e| {
            // The regex crate's message; Java prints PatternSyntaxException.getMessage().
            e.to_string()
        })?;
        Ok(JavaPattern {
            pattern: pattern.to_string(),
            compiled,
        })
    }

    /// `Pattern.pattern()`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// `pattern.matcher(s).find()`.
    pub fn find(&self, s: &str) -> bool {
        self.compiled.regex_pattern().is_match(s)
    }

    /// `pattern.matcher(s).matches()`.
    pub fn matches(&self, s: &str) -> bool {
        self.compiled.pattern_matches(s)
    }
}

/// Java `String.split(regex)` (limit 0) for a single-character separator that is not preceded by
/// a backslash, i.e. `split("(?<!\\\\),")`: trailing empty strings are removed.
pub(crate) fn split_unescaped(input: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    let mut matched = false;
    for c in input.chars() {
        if c == sep && prev != Some('\\') {
            parts.push(std::mem::take(&mut cur));
            matched = true;
        } else {
            cur.push(c);
        }
        prev = Some(c);
    }
    parts.push(cur);
    if !matched {
        // "If the expression does not match any part of the input then the resulting array has
        // just one element, namely this string."
        return parts;
    }
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

/// Java `String.split(regex)` with limit 0 for a regex matching one single-byte separator (given
/// as a predicate on the byte index of each candidate): trailing empty strings are removed, and
/// an input without any separator is returned whole.
pub(crate) fn java_split(
    input: &str,
    is_separator_at: impl Fn(&str, usize) -> bool,
) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut found = false;
    for (i, _) in input.char_indices() {
        if is_separator_at(input, i) {
            found = true;
            parts.push(input[start..i].to_string());
            start = i + 1;
        }
    }
    if !found {
        return vec![input.to_string()];
    }
    parts.push(input[start..].to_string());
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

// Java: com/google/devtools/build/lib/util/RegexFilter.java:38 RegexFilter
/// Handles options that specify list of included/excluded regex expressions. Validates whether
/// string is included in that filter.
///
/// String is considered to be included into the filter if it does not match any of the excluded
/// regex expressions and if it matches at least one included regex expression.
#[derive(Clone, Debug)]
pub struct RegexFilter {
    // Null inclusion or exclusion pattern means those patterns are not used.
    inclusion_pattern: Option<JavaPattern>,
    exclusion_pattern: Option<JavaPattern>,
    original_input: Option<String>,
}

// Java: com/google/devtools/build/lib/util/RegexFilter.java:54 RegexFilter.RegexFilterConverter
/// Converts from a comma-separated list of regex expressions with optional -/+ prefix into the
/// RegexFilter. Commas prefixed with backslash are considered to be part of regex definition and
/// not a delimiter between separate regex expressions.
///
/// Order of expressions is not important. Empty entries are ignored. '-' marks an excluded
/// expression.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegexFilterConverter;

impl RegexFilterConverter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:57 RegexFilter.RegexFilterConverter.convert
    pub fn convert_filter(&self, input: &str) -> Result<RegexFilter, OptionsParsingException> {
        if input.starts_with("--") {
            return Err(OptionsParsingException::new(format!(
                "Failed to build filter: value looks like another flag ({}). Either escape the \
                 value with \"\\-\\-\", or pass an explicit value to the flag.",
                input
            )));
        }
        let mut inclusion_list = Vec::new();
        let mut exclusion_list = Vec::new();

        for piece in split_unescaped(input, ',') {
            // Split on ',' but not on '\,'
            let mut piece = piece.replace("\\,", ",");
            let is_excluded = piece.starts_with('-');
            if is_excluded || piece.starts_with('+') {
                piece = piece[1..].to_string();
            }
            if !piece.is_empty() {
                if is_excluded {
                    exclusion_list.push(piece);
                } else {
                    inclusion_list.push(piece);
                }
            }
        }

        RegexFilter::with_original_input(&inclusion_list, &exclusion_list, Some(input.to_string()))
            .map_err(|e| {
                OptionsParsingException::new(format!(
                    "Failed to build valid regular expression: {}",
                    e
                ))
            })
    }
}

impl Converter for RegexFilterConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_filter(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        "a comma-separated list of regex expressions with prefix '-' specifying excluded paths"
            .into()
    }
}

impl RegexFilter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:101 RegexFilter.<init>
    fn with_original_input(
        inclusions: &[String],
        exclusions: &[String],
        original_input: Option<String>,
    ) -> Result<RegexFilter, String> {
        Ok(RegexFilter {
            inclusion_pattern: Self::take_union_of_regexes(inclusions)?,
            exclusion_pattern: Self::take_union_of_regexes(exclusions)?,
            original_input,
        })
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:119 RegexFilter.<init>
    /// Creates new RegexFilter using provided inclusion and exclusion path lists.
    // PORT: Java throws the unchecked PatternSyntaxException; the error is its message here.
    pub fn new(inclusions: &[String], exclusions: &[String]) -> Result<RegexFilter, String> {
        Self::with_original_input(inclusions, exclusions, None)
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:128 RegexFilter.takeUnionOfRegexes
    /// Converts a list of regex expressions into a single regex representing its union or null
    /// when the list is empty.
    fn take_union_of_regexes(regex_list: &[String]) -> Result<Option<JavaPattern>, String> {
        if regex_list.is_empty() {
            return Ok(None);
        }
        let mut deduped: Vec<&str> = regex_list.iter().map(String::as_str).collect();
        deduped.sort_by(|a, b| java_string_compare(a, b));
        deduped.dedup();
        // Wraps each individual regex into an independent group, then combines them using '|'
        // and wraps the result in a non-capturing group.
        Ok(Some(JavaPattern::compile(&format!(
            "(?:(?>{}))",
            deduped.join(")|(?>")
        ))?))
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:142 RegexFilter.isIncluded
    /// Returns true iff given string is included (it does not match exclusion pattern (if any)
    /// and matches inclusionPatter (if any)).
    pub fn is_included(&self, value: &str) -> bool {
        if let Some(e) = &self.exclusion_pattern
            && e.find(value)
        {
            return false;
        }
        match &self.inclusion_pattern {
            None => true,
            Some(i) => i.find(value),
        }
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:153 RegexFilter.test
    pub fn test(&self, value: &str) -> bool {
        self.is_included(value)
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:180 RegexFilter.toOriginalString
    /// RegexFilter doesn't serialize cleanly: `!RegexFilter.convert(".*").toString().equals(".")`.
    ///
    /// This method provides the ability to reproduce the original input string.
    pub fn to_original_string(&self) -> Option<&str> {
        self.original_input.as_deref()
    }
}

impl std::fmt::Display for RegexFilter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:158 RegexFilter.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut builder = String::new();
        if let Some(i) = &self.inclusion_pattern {
            builder.push_str(&i.pattern().replace(',', "\\,"));
            if self.exclusion_pattern.is_some() {
                builder.push(',');
            }
        }
        if let Some(e) = &self.exclusion_pattern {
            builder.push('-');
            builder.push_str(&e.pattern().replace(',', "\\,"));
        }
        f.write_str(&builder)
    }
}

impl PartialEq for RegexFilter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:185 RegexFilter.equals
    fn eq(&self, other: &Self) -> bool {
        self.inclusion_pattern.as_ref().map(JavaPattern::pattern)
            == other.inclusion_pattern.as_ref().map(JavaPattern::pattern)
            && self.exclusion_pattern.as_ref().map(JavaPattern::pattern)
                == other.exclusion_pattern.as_ref().map(JavaPattern::pattern)
    }
}

impl Eq for RegexFilter {}

impl Hash for RegexFilter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:211 RegexFilter.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inclusion_pattern
            .as_ref()
            .map(JavaPattern::pattern)
            .hash(state);
        self.exclusion_pattern
            .as_ref()
            .map(JavaPattern::pattern)
            .hash(state);
    }
}

impl OptionScalar for RegexFilter {
    fn java_to_string(&self) -> String {
        self.to_string()
    }
}

option_field_type!(
    RegexFilter,
    "com.google.devtools.build.lib.util.RegexFilter",
    raw = RegexFilter {
        inclusion_pattern: None,
        exclusion_pattern: None,
        original_input: None,
    }
);

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/util/RegexFilterTest.java
    use super::*;

    fn create_filter(filter_string: &str) -> Result<RegexFilter, OptionsParsingException> {
        RegexFilterConverter.convert_filter(filter_string)
    }

    fn assert_included(filter: &RegexFilter, value: &str) {
        assert!(filter.is_included(value), "{} should be included", value);
    }

    fn assert_excluded(filter: &RegexFilter, value: &str) {
        assert!(!filter.is_included(value), "{} should be excluded", value);
    }

    #[test]
    fn empty_filter() {
        let filter = create_filter("").unwrap();
        assert_included(&filter, "a/b/c");
        assert_included(&filter, "d");
    }

    #[test]
    fn inclusions() {
        let filter = create_filter("a/b,+^c,_test$").unwrap();
        assert_eq!(filter.to_string(), "(?:(?>^c)|(?>_test$)|(?>a/b))");
        for v in ["a/b", "a/b/c", "c", "c/d", "e/a/b", "f/1/2/3/_test"] {
            assert_included(&filter, v);
        }
        for v in ["a", "a/c", "d", "e/f/g", "f/_test2"] {
            assert_excluded(&filter, v);
        }
    }

    #[test]
    fn exclusions() {
        let filter = create_filter("-a/b,-^c,-_test$").unwrap();
        assert_eq!(filter.to_string(), "-(?:(?>^c)|(?>_test$)|(?>a/b))");
        for v in ["a/b", "a/b/c", "c", "c/d", "f/a/b/d", "f/a_test"] {
            assert_excluded(&filter, v);
        }
        for v in ["a", "a/c", "d", "e/f/g", "f/a_test_case"] {
            assert_included(&filter, v);
        }
    }

    #[test]
    fn inclusions_and_exclusions() {
        let filter = create_filter("a,-^c,,-,+,d,+a/b/c,-a/b,a/b/d").unwrap();
        assert_eq!(
            filter.to_string(),
            "(?:(?>a)|(?>a/b/c)|(?>a/b/d)|(?>d)),-(?:(?>^c)|(?>a/b))"
        );
        assert_included(&filter, "a");
        assert_included(&filter, "a/c");
        assert_excluded(&filter, "a/b");
        // Exclusions take precedence over inclusions. Order is not important.
        assert_excluded(&filter, "a/b/c");
        assert_excluded(&filter, "a/b/d");
        assert_excluded(&filter, "a/c/a/b/d");
        assert_excluded(&filter, "c");
        assert_excluded(&filter, "c/d");
        assert_included(&filter, "d/e");
        assert_excluded(&filter, "e");
    }

    #[test]
    fn commas() {
        let filter = create_filter("a\\,b,c\\,d").unwrap();
        assert_eq!(filter.to_string(), "(?:(?>a\\,b)|(?>c\\,d))");
        assert_included(&filter, "a,b");
        assert_included(&filter, "c,d");
        assert_excluded(&filter, "a");
        assert_excluded(&filter, "b,c");
        assert_excluded(&filter, "d");
    }

    #[test]
    #[ignore = "the regex crate's error text differs from java.util.regex.PatternSyntaxException"]
    fn invalid_expression() {
        let e = create_filter("*a").unwrap_err();
        assert!(e.get_message().contains(
            "Failed to build valid regular expression: Dangling meta character '*' near index"
        ));
    }

    // Blaze-specific: the part of invalid_expression the regex crate can reproduce.
    #[test]
    fn invalid_expression_prefix() {
        let e = create_filter("*a").unwrap_err();
        // PORT: Java's message continues "Dangling meta character '*' near index"; the regex
        // crate words syntax errors differently.
        assert!(
            e.get_message()
                .starts_with("Failed to build valid regular expression: "),
            "{}",
            e.get_message()
        );
    }

    #[test]
    fn equals() {
        let groups: Vec<Vec<RegexFilter>> = vec![
            vec![
                create_filter("a,b,c").unwrap(),
                create_filter("a,b,c").unwrap(),
            ],
            vec![create_filter("a,b,c,d").unwrap()],
            vec![
                create_filter("a,b,-c").unwrap(),
                create_filter("a,b,-c").unwrap(),
            ],
            vec![create_filter("a,b,-c,-d").unwrap()],
            vec![
                create_filter("-a,-b,-c").unwrap(),
                create_filter("-a,-b,-c").unwrap(),
            ],
            vec![create_filter("-a,-b,-c,-d").unwrap()],
            vec![create_filter("").unwrap(), create_filter("").unwrap()],
        ];
        for (i, gi) in groups.iter().enumerate() {
            for (j, gj) in groups.iter().enumerate() {
                for a in gi {
                    for b in gj {
                        assert_eq!(a == b, i == j);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "needs the serialization framework (SerializationTester)"]
    fn codec() {}

    #[test]
    fn initial_double_dash_error() {
        let e = create_filter("--compilation_mode").unwrap_err();
        assert!(e.get_message().contains(
            "Failed to build filter: value looks like another flag (--compilation_mode). Either \
             escape the value with \"\\-\\-\", or pass an explicit value to the flag."
        ));
    }

    #[test]
    fn initial_double_dash_escaped() {
        let filter = create_filter("\\-\\-compilation_mode").unwrap();
        assert_included(&filter, "--compilation_mode");
    }

    // Blaze-specific: Java String.split semantics used by the converter.
    #[test]
    fn java_split_semantics() {
        assert_eq!(split_unescaped("", ','), vec![""]);
        assert!(split_unescaped(",", ',').is_empty());
        assert_eq!(split_unescaped(",a,,", ','), vec!["", "a"]);
        assert_eq!(split_unescaped("a\\,b,c", ','), vec!["a\\,b", "c"]);
        assert_eq!(
            java_split("a:b::", |s, i| s.as_bytes()[i] == b':'),
            vec!["a", "b"]
        );
        assert_eq!(java_split("ab", |s, i| s.as_bytes()[i] == b':'), vec!["ab"]);
    }
}
