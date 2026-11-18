//! Port of `com/google/devtools/build/lib/util/RegexFilter.java`.

use std::fmt;

use super::prelude::*;

/// Java `String.split(regex)` with limit 0 for a regex matching one separator (given as a
/// predicate on the byte index of each candidate `sep`): trailing empty strings are removed, and
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

/// `input.split("(?<!\\\\),")`: split on `,` but not on `\,`.
pub(crate) fn split_on_unescaped_commas(input: &str) -> Vec<String> {
    java_split(input, |s, i| {
        s.as_bytes()[i] == b',' && (i == 0 || s.as_bytes()[i - 1] != b'\\')
    })
}

/// A compiled union pattern: the Java pattern string plus a regex that finds the same matches.
#[derive(Clone)]
struct UnionPattern {
    /// `Pattern.pattern()` of the Java pattern: `(?:(?>r1)|(?>r2)...)`.
    java_pattern: String,
    compiled: RegexPatternOption,
}

impl UnionPattern {
    fn find(&self, value: &str) -> bool {
        self.compiled.regex_pattern().is_match(value)
    }
}

// Java: com/google/devtools/build/lib/util/RegexFilter.java:38 RegexFilter
/// Handles options that specify list of included/excluded regex expressions. Validates whether
/// string is included in that filter.
///
/// String is considered to be included into the filter if it does not match any of the excluded
/// regex expressions and if it matches at least one included regex expression.
#[derive(Clone)]
pub struct RegexFilter {
    // Null inclusion or exclusion pattern means those patterns are not used.
    inclusion_pattern: Option<UnionPattern>,
    exclusion_pattern: Option<UnionPattern>,
    original_input: Option<String>,
}

/// The error of compiling a pattern (Java `PatternSyntaxException`), carrying its message.
#[derive(Clone, Debug)]
pub struct PatternSyntaxException(pub String);

impl RegexFilter {
    // Java: com/google/devtools/build/lib/util/RegexFilter.java:119 RegexFilter.<init>
    /// Creates new RegexFilter using provided inclusion and exclusion path lists.
    pub fn new(
        inclusions: &[String],
        exclusions: &[String],
    ) -> Result<Self, PatternSyntaxException> {
        Self::with_original_input(inclusions, exclusions, None)
    }

    fn with_original_input(
        inclusions: &[String],
        exclusions: &[String],
        original_input: Option<String>,
    ) -> Result<Self, PatternSyntaxException> {
        Ok(RegexFilter {
            inclusion_pattern: Self::take_union_of_regexes(inclusions)?,
            exclusion_pattern: Self::take_union_of_regexes(exclusions)?,
            original_input,
        })
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:128 RegexFilter.takeUnionOfRegexes
    /// Converts a list of regex expressions into a single regex representing its union or null
    /// when the list is empty.
    fn take_union_of_regexes(
        regex_list: &[String],
    ) -> Result<Option<UnionPattern>, PatternSyntaxException> {
        if regex_list.is_empty() {
            return Ok(None);
        }
        let deduped: std::collections::BTreeSet<&str> =
            regex_list.iter().map(String::as_str).collect();
        let deduped: Vec<&str> = deduped.into_iter().collect();
        // Wraps each individual regex into an independent group, then combines them using '|'
        // and wraps the result in a non-capturing group.
        let java_pattern = format!("(?:(?>{}))", deduped.join(")|(?>"));
        // PORT: the regex crate has no atomic groups. Each alternative's atomic group is the end
        // of the pattern, so it cannot change whether find() succeeds; a plain group is used.
        // `(?-s)` undoes RegexPatternOption's DOTALL (Java compiles this pattern without flags).
        let rust_pattern = format!("(?-s)(?:(?:{}))", deduped.join(")|(?:"));
        let compiled = RegexPatternOption::create(&rust_pattern)
            .map_err(|e| PatternSyntaxException(e.to_string()))?;
        Ok(Some(UnionPattern {
            java_pattern,
            compiled,
        }))
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:142 RegexFilter.isIncluded
    /// True iff the given string is included (it does not match the exclusion pattern, if any,
    /// and matches the inclusion pattern, if any).
    pub fn is_included(&self, value: &str) -> bool {
        if let Some(exclusion) = &self.exclusion_pattern
            && exclusion.find(value)
        {
            return false;
        }
        match &self.inclusion_pattern {
            None => true,
            Some(inclusion) => inclusion.find(value),
        }
    }

    // Java: com/google/devtools/build/lib/util/RegexFilter.java:180 RegexFilter.toOriginalString
    /// The original input string of the converter, if this filter came from one.
    pub fn to_original_string(&self) -> Option<&str> {
        self.original_input.as_deref()
    }
}

// Java: com/google/devtools/build/lib/util/RegexFilter.java:158 RegexFilter.toString
impl fmt::Display for RegexFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = String::new();
        if let Some(inclusion) = &self.inclusion_pattern {
            builder.push_str(&inclusion.java_pattern.replace(',', "\\,"));
            if self.exclusion_pattern.is_some() {
                builder.push(',');
            }
        }
        if let Some(exclusion) = &self.exclusion_pattern {
            builder.push('-');
            builder.push_str(&exclusion.java_pattern.replace(',', "\\,"));
        }
        f.write_str(&builder)
    }
}

impl fmt::Debug for RegexFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RegexFilter({})", self)
    }
}

// Java: com/google/devtools/build/lib/util/RegexFilter.java:185 RegexFilter.equals
impl PartialEq for RegexFilter {
    fn eq(&self, other: &Self) -> bool {
        let pattern = |p: &Option<UnionPattern>| p.as_ref().map(|p| p.java_pattern.clone());
        pattern(&self.inclusion_pattern) == pattern(&other.inclusion_pattern)
            && pattern(&self.exclusion_pattern) == pattern(&other.exclusion_pattern)
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
        for piece in split_on_unescaped_commas(input) {
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
                // Java prints PatternSyntaxException.getMessage(); the regex crate's text differs.
                OptionsParsingException::new(format!(
                    "Failed to build valid regular expression: {}",
                    e.0
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

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from src/test/java/com/google/devtools/build/lib/util/RegexFilterTest.java.

    fn create_filter(filter_string: &str) -> Result<RegexFilter, OptionsParsingException> {
        RegexFilterConverter.convert_filter(filter_string)
    }

    fn assert_included(filter: &RegexFilter, value: &str) {
        assert!(
            filter.is_included(value),
            "{} should be included by {}",
            value,
            filter
        );
    }

    fn assert_excluded(filter: &RegexFilter, value: &str) {
        assert!(
            !filter.is_included(value),
            "{} should be excluded by {}",
            value,
            filter
        );
    }

    #[test]
    fn empty_filter() {
        let f = create_filter("").unwrap();
        assert_included(&f, "a/b/c");
        assert_included(&f, "d");
    }

    #[test]
    fn inclusions() {
        let f = create_filter("a/b,+^c,_test$").unwrap();
        assert_eq!(f.to_string(), "(?:(?>^c)|(?>_test$)|(?>a/b))");
        for v in ["a/b", "a/b/c", "c", "c/d", "e/a/b", "f/1/2/3/_test"] {
            assert_included(&f, v);
        }
        for v in ["a", "a/c", "d", "e/f/g", "f/_test2"] {
            assert_excluded(&f, v);
        }
    }

    #[test]
    fn exclusions() {
        let f = create_filter("-a/b,-^c,-_test$").unwrap();
        assert_eq!(f.to_string(), "-(?:(?>^c)|(?>_test$)|(?>a/b))");
        for v in ["a/b", "a/b/c", "c", "c/d", "f/a/b/d", "f/a_test"] {
            assert_excluded(&f, v);
        }
        for v in ["a", "a/c", "d", "e/f/g", "f/a_test_case"] {
            assert_included(&f, v);
        }
    }

    #[test]
    fn inclusions_and_exclusions() {
        let f = create_filter("a,-^c,,-,+,d,+a/b/c,-a/b,a/b/d").unwrap();
        assert_eq!(
            f.to_string(),
            "(?:(?>a)|(?>a/b/c)|(?>a/b/d)|(?>d)),-(?:(?>^c)|(?>a/b))"
        );
        assert_included(&f, "a");
        assert_included(&f, "a/c");
        assert_excluded(&f, "a/b");
        // Exclusions take precedence over inclusions. Order is not important.
        assert_excluded(&f, "a/b/c");
        assert_excluded(&f, "a/b/d");
        assert_excluded(&f, "a/c/a/b/d");
        assert_excluded(&f, "c");
        assert_excluded(&f, "c/d");
        assert_included(&f, "d/e");
        assert_excluded(&f, "e");
    }

    #[test]
    fn commas() {
        let f = create_filter("a\\,b,c\\,d").unwrap();
        assert_eq!(f.to_string(), "(?:(?>a\\,b)|(?>c\\,d))");
        assert_included(&f, "a,b");
        assert_included(&f, "c,d");
        assert_excluded(&f, "a");
        assert_excluded(&f, "b,c");
        assert_excluded(&f, "d");
    }

    #[test]
    #[ignore = "the regex crate's error text differs from java.util.regex.PatternSyntaxException"]
    fn invalid_expression() {
        let e = create_filter("*a").unwrap_err();
        assert!(e.get_message().contains(
            "Failed to build valid regular expression: Dangling meta character '*' near index"
        ));
    }

    #[test]
    fn invalid_expression_prefix() {
        let e = create_filter("*a").unwrap_err();
        assert!(
            e.get_message()
                .starts_with("Failed to build valid regular expression: ")
        );
    }

    #[test]
    fn equals() {
        let groups: [&[&str]; 7] = [
            &["a,b,c", "a,b,c"],
            &["a,b,c,d"],
            &["a,b,-c", "a,b,-c"],
            &["a,b,-c,-d"],
            &["-a,-b,-c", "-a,-b,-c"],
            &["-a,-b,-c,-d"],
            &["", ""],
        ];
        for (i, gi) in groups.iter().enumerate() {
            for (j, gj) in groups.iter().enumerate() {
                for a in gi.iter() {
                    for b in gj.iter() {
                        let (fa, fb) = (create_filter(a).unwrap(), create_filter(b).unwrap());
                        assert_eq!(fa == fb, i == j, "{:?} vs {:?}", a, b);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "needs SerializationTester (skyframe serialization is not ported)"]
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
        let f = create_filter("\\-\\-compilation_mode").unwrap();
        assert_included(&f, "--compilation_mode");
    }

    // Blaze-specific: Java String.split semantics used by the converter.
    #[test]
    fn java_split_semantics() {
        assert_eq!(split_on_unescaped_commas(""), vec![""]);
        assert!(split_on_unescaped_commas(",").is_empty());
        assert_eq!(split_on_unescaped_commas(",a,,"), vec!["", "a"]);
        assert_eq!(split_on_unescaped_commas("a\\,b,c"), vec!["a\\,b", "c"]);
    }
}
