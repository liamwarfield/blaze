//! Port of `analysis/config/ExecutionInfoModifier.java`.

use blaze_util::string_encoding::internal_to_unicode;
use std::collections::HashMap;

use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, RegexPatternConverter,
    RegexPatternOption, Value, option_field_type,
};

use super::converters::ConvResult;
use super::regex_filter::JavaPattern;

// Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:36 ExecutionInfoModifier
/// Represents a list of regexes over mnemonics and changes to add or remove keys, parsed from a
/// `--modify_execution_info` option.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionInfoModifier {
    option: String,
    expressions: Vec<ExecutionInfoModifierExpression>,
}

// Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:48 ExecutionInfoModifier.Expression
#[derive(Clone, Debug)]
pub struct ExecutionInfoModifierExpression {
    regex: String,
    remove: bool,
    key: String,
    // Patterns do not have a useful equals(), so compare by the regex and memoize the derived
    // Pattern.
    pattern: JavaPattern,
}

impl ExecutionInfoModifierExpression {
    fn pattern(&self) -> &JavaPattern {
        &self.pattern
    }
    pub fn regex(&self) -> &str {
        &self.regex
    }
    pub fn remove(&self) -> bool {
        self.remove
    }
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl PartialEq for ExecutionInfoModifierExpression {
    fn eq(&self, other: &Self) -> bool {
        self.regex == other.regex && self.remove == other.remove && self.key == other.key
    }
}

impl Eq for ExecutionInfoModifierExpression {}

impl std::hash::Hash for ExecutionInfoModifierExpression {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.regex.hash(state);
        self.remove.hash(state);
        self.key.hash(state);
    }
}

/// `MODIFIER_PATTERN = ^(?<pattern>.+)=(?<sign>[+-])(?<key>.+)$` with `matches()`: returns
/// `(pattern, sign, key)`. The greedy `pattern` group takes the last `=` that is followed by a
/// sign and at least one more character.
// PORT: hand-written matcher for MODIFIER_PATTERN. Java's `.` excludes line terminators; inputs
// containing `\n`, `\r`, U+0085, U+2028 or U+2029 do not match here.
fn match_modifier_pattern(spec: &str) -> Option<(&str, &str, &str)> {
    if spec.contains(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let bytes = spec.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'='
            && i >= 1
            && i + 2 < bytes.len()
            && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-')
        {
            return Some((&spec[..i], &spec[i + 1..i + 2], &spec[i + 2..]));
        }
    }
    None
}

// Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:64 ExecutionInfoModifier.Converter
/// Constructs an instance of ExecutionInfoModifier by parsing an option string.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionInfoModifierConverter;

impl ExecutionInfoModifierConverter {
    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:67 ExecutionInfoModifier.Converter.convert
    pub fn convert_modifier(
        &self,
        input: &str,
    ) -> Result<ExecutionInfoModifier, OptionsParsingException> {
        if input.is_empty() {
            return Ok(ExecutionInfoModifier::create(String::new(), vec![]));
        }

        let mut expressions = Vec::new();
        for spec in input.split(',') {
            let Some((pattern, sign, key)) = match_modifier_pattern(spec) else {
                return Err(OptionsParsingException::with_argument(
                    format!("malformed expression '{}'", spec),
                    input,
                ));
            };
            // Convert to get a useful exception if it's not a valid pattern, but use the regex
            // (see comment in Expression)
            let regex = RegexPatternConverter
                .convert(Some(pattern), None)?
                .expect("RegexPatternConverter returns a value")
                .get::<RegexPatternOption>()
                .pattern()
                .to_string();
            let compiled = JavaPattern::compile(&regex)
                .unwrap_or_else(|e| panic!("PatternSyntaxException: {}", e));
            expressions.push(ExecutionInfoModifierExpression {
                regex,
                remove: sign == "-",
                key: key.to_string(),
                pattern: compiled,
            });
        }
        Ok(ExecutionInfoModifier::create(
            input.to_string(),
            expressions,
        ))
    }
}

impl Converter for ExecutionInfoModifierConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        // Strings.isNullOrEmpty(input): a null input is the empty modifier too.
        Ok(Some(Value::new(
            self.convert_modifier(input.unwrap_or(""))?,
        )))
    }
    fn get_type_description(&self) -> String {
        "regex=[+-]key,regex=[+-]key,...".into()
    }
}

impl ExecutionInfoModifier {
    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:99 ExecutionInfoModifier.create
    fn create(input: String, expressions: Vec<ExecutionInfoModifierExpression>) -> Self {
        ExecutionInfoModifier {
            option: input,
            expressions,
        }
    }

    pub fn option(&self) -> &str {
        &self.option
    }

    pub fn expressions(&self) -> &[ExecutionInfoModifierExpression] {
        &self.expressions
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:106 ExecutionInfoModifier.matches
    /// Determines whether the given `mnemonic` (e.g. "CppCompile") matches any of the patterns.
    pub fn matches(&self, mnemonic: &str) -> bool {
        self.expressions
            .iter()
            .any(|expr| expr.pattern().matches(&internal_to_unicode(mnemonic)))
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:112 ExecutionInfoModifier.matches
    /// Checks whether the `execution_info_list` matches the `mnemonic`.
    pub fn matches_list(
        execution_info_list: &[ExecutionInfoModifier],
        is_additive: bool,
        mnemonic: &str,
    ) -> bool {
        if execution_info_list.is_empty() {
            return false;
        }

        if is_additive {
            execution_info_list.iter().any(|eim| eim.matches(mnemonic))
        } else {
            execution_info_list.last().unwrap().matches(mnemonic)
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:128 ExecutionInfoModifier.wouldChange
    /// Checks whether applying this modifier would actually change the given map.
    // When --modify_execution_info is non-empty, this method helps reduce CPU/memory overhead
    // from modifying every action's execution info map.
    pub fn would_change(&self, mnemonic: &str, execution_info: &HashMap<String, String>) -> bool {
        for expr in &self.expressions {
            if !expr.pattern().matches(&internal_to_unicode(mnemonic)) {
                continue;
            }
            if expr.remove {
                if execution_info.contains_key(&expr.key) {
                    return true;
                }
            } else {
                match execution_info.get(&expr.key) {
                    None => return true,
                    Some(val) if !val.is_empty() => return true,
                    Some(_) => {}
                }
            }
        }
        false
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:148 ExecutionInfoModifier.wouldChange
    /// Checks whether any modifier in the list would actually change the given map.
    pub fn would_change_list(
        execution_info_list: &[ExecutionInfoModifier],
        is_additive: bool,
        mnemonic: &str,
        execution_info: &HashMap<String, String>,
    ) -> bool {
        if execution_info_list.is_empty() {
            return false;
        }

        if is_additive {
            execution_info_list
                .iter()
                .any(|eim| eim.would_change(mnemonic, execution_info))
        } else {
            execution_info_list
                .last()
                .unwrap()
                .would_change(mnemonic, execution_info)
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:165 ExecutionInfoModifier.apply
    /// Applies `execution_info_list` to the given `execution_info`.
    // PORT: Java's Map<String, String> parameter is a HashMap at every call site.
    pub fn apply_list(
        execution_info_list: &[ExecutionInfoModifier],
        is_additive: bool,
        mnemonic: &str,
        execution_info: &mut HashMap<String, String>,
    ) {
        if execution_info_list.is_empty() {
            return;
        }

        if is_additive {
            for eim in execution_info_list {
                eim.apply(mnemonic, execution_info);
            }
        } else {
            execution_info_list
                .last()
                .unwrap()
                .apply(mnemonic, execution_info);
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/ExecutionInfoModifier.java:182 ExecutionInfoModifier.apply
    /// Modifies the given map of `execution_info` to add or remove the keys for this option.
    pub fn apply(&self, mnemonic: &str, execution_info: &mut HashMap<String, String>) {
        for expr in &self.expressions {
            if expr.pattern().matches(&internal_to_unicode(mnemonic)) {
                if expr.remove {
                    execution_info.remove(&expr.key);
                } else {
                    execution_info.insert(expr.key.clone(), String::new());
                }
            }
        }
    }
}

impl OptionScalar for ExecutionInfoModifier {
    // AutoValue toString.
    fn java_to_string(&self) -> String {
        let exprs: Vec<String> = self
            .expressions
            .iter()
            .map(|e| {
                format!(
                    "Expression{{regex={}, remove={}, key={}}}",
                    e.regex, e.remove, e.key
                )
            })
            .collect();
        format!(
            "ExecutionInfoModifier{{option={}, expressions=[{}]}}",
            self.option,
            exprs.join(", ")
        )
    }
}

option_field_type!(
    ExecutionInfoModifier,
    "com.google.devtools.build.lib.analysis.config.ExecutionInfoModifier",
    raw = ExecutionInfoModifier::create(String::new(), vec![])
);

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/analysis/config/ExecutionInfoModifierTest.java
    use super::*;

    fn convert(s: &str) -> Result<ExecutionInfoModifier, OptionsParsingException> {
        ExecutionInfoModifierConverter.convert_modifier(s)
    }

    fn assert_modifier_matches_and_results(
        modifiers: &[ExecutionInfoModifier],
        additive: bool,
        mnemonic: &str,
        expected_keys: &[&str],
    ) {
        let mut copy = HashMap::new();
        ExecutionInfoModifier::apply_list(modifiers, additive, mnemonic, &mut copy);
        assert!(ExecutionInfoModifier::matches_list(
            modifiers, additive, mnemonic
        ));
        let expected: HashMap<String, String> = expected_keys
            .iter()
            .map(|k| (k.to_string(), String::new()))
            .collect();
        assert_eq!(copy, expected);
    }

    fn assert_single(modifier: &ExecutionInfoModifier, mnemonic: &str, expected_keys: &[&str]) {
        assert_modifier_matches_and_results(
            std::slice::from_ref(modifier),
            false,
            mnemonic,
            expected_keys,
        );
    }

    #[test]
    fn execution_info_modifier_empty() {
        let modifier = convert("").unwrap();
        assert!(!modifier.matches("Anything"));
    }

    #[test]
    fn execution_info_modifier_single_add() {
        let modifier = convert("Genrule=+x").unwrap();
        assert!(!modifier.matches("SomethingElse"));
        assert_single(&modifier, "Genrule", &["x"]);
    }

    // Blaze: the option value and the mnemonic are internal strings; the pattern is compiled
    // from internalToUnicode of the value and matched against internalToUnicode of the mnemonic,
    // so `.` matches the one char `é` (C3 A9) but not a byte that is not UTF-8 twice.
    #[test]
    fn execution_info_modifier_matches_unicode_of_internal_strings() {
        let modifier = convert("Gen.=+x").unwrap();
        assert_single(&modifier, "Gen\u{c3}\u{a9}", &["x"]);
        assert!(modifier.matches("Gen\u{ff}"));
        assert!(!modifier.matches("Gen\u{ff}\u{ff}"));
        let literal = convert("Gen\u{c3}\u{a9}=+y").unwrap();
        assert_eq!(literal.expressions()[0].regex(), "Gen\u{e9}");
        assert_single(&literal, "Gen\u{c3}\u{a9}", &["y"]);
        assert!(!literal.matches("Gen\u{e9}"));
    }

    #[test]
    fn execution_info_modifier_single_remove() {
        let modifier = convert("Genrule=-x").unwrap();
        let mut info = HashMap::new();
        info.insert("x".to_string(), String::new());
        modifier.apply("Genrule", &mut info);
        assert!(info.is_empty());
    }

    #[test]
    fn execution_info_modifier_multiple_expressions() {
        let modifier = convert("Genrule=+x,.*=+y,CppCompile=+z").unwrap();
        assert_single(&modifier, "Genrule", &["x", "y"]);
        assert_single(&modifier, "CppCompile", &["y", "z"]);
        assert_single(&modifier, "GenericAction", &["y"]);
    }

    fn three_modifiers() -> Vec<ExecutionInfoModifier> {
        vec![
            convert("Genrule=+x,CppCompile=-y1,GenericAction=+z,MergeLayers=+t,OtherAction=+o")
                .unwrap(),
            convert("Genrule=-x,CppCompile=+y1,CppCompile=+y2,GenericAction=+z,MergeLayers=+u")
                .unwrap(),
            convert(".*=-t").unwrap(),
        ]
    }

    #[test]
    fn execution_info_modifier_multiple_options_additive() {
        let modifiers = three_modifiers();
        assert_modifier_matches_and_results(&modifiers, true, "Genrule", &[]);
        assert_modifier_matches_and_results(&modifiers, true, "CppCompile", &["y1", "y2"]);
        assert_modifier_matches_and_results(&modifiers, true, "GenericAction", &["z"]);
        assert_modifier_matches_and_results(&modifiers, true, "MergeLayers", &["u"]);
        assert_modifier_matches_and_results(&modifiers, true, "OtherAction", &["o"]);
    }

    #[test]
    fn execution_info_modifier_multiple_options_non_additive() {
        let all = three_modifiers();
        let modifiers1 = &all[..2];
        assert_modifier_matches_and_results(modifiers1, false, "Genrule", &[]);
        assert_modifier_matches_and_results(modifiers1, false, "CppCompile", &["y1", "y2"]);
        assert_modifier_matches_and_results(modifiers1, false, "GenericAction", &["z"]);
        assert_modifier_matches_and_results(modifiers1, false, "MergeLayers", &["u"]);
        assert!(!ExecutionInfoModifier::matches_list(
            modifiers1,
            false,
            "OtherAction"
        ));

        let modifiers2 = &all[..];
        for m in [
            "Genrule",
            "CppCompile",
            "GenericAction",
            "MergeLayers",
            "OtherAction",
        ] {
            assert_modifier_matches_and_results(modifiers2, false, m, &[]);
        }
    }

    #[test]
    fn execution_info_modifier_invalid_format_throws() {
        for invalid in ["A", "=", "A=", "A=+", "=+", "A=-B,A", "A=B", "A", ","] {
            assert!(convert(invalid).is_err(), "{}", invalid);
        }
    }

    #[test]
    fn execution_info_modifier_invalid_format_exception_shows_offender() {
        let thrown = convert("A=+1,B=2,C=-3").unwrap_err();
        assert!(thrown.get_message().contains("malformed"));
        assert!(thrown.get_message().contains("'B=2'"));
    }

    #[test]
    fn execution_info_modifier_equals_tester() {
        let groups: Vec<Vec<ExecutionInfoModifier>> = vec![
            vec![convert("").unwrap(), convert("").unwrap()],
            vec![convert("A=+B").unwrap(), convert("A=+B").unwrap()],
            vec![convert("C=+D").unwrap()],
            vec![convert("A=+D").unwrap()],
            vec![convert("C=+B").unwrap()],
            vec![convert("A=-B").unwrap()],
            vec![convert("A=+B,C=-D").unwrap(), convert("A=+B,C=-D").unwrap()],
            vec![convert("C=-D,A=+B").unwrap()],
        ];
        for (i, gi) in groups.iter().enumerate() {
            for (j, gj) in groups.iter().enumerate() {
                for a in gi {
                    for b in gj {
                        assert_eq!(a == b, i == j, "{:?} vs {:?}", a, b);
                    }
                }
            }
        }
    }

    #[test]
    fn would_change_remove_key_not_present_returns_false() {
        let modifier = convert("Genrule=-x").unwrap();
        assert!(!modifier.would_change("Genrule", &HashMap::new()));
    }

    #[test]
    fn would_change_remove_key_present_returns_true() {
        let modifier = convert("Genrule=-x").unwrap();
        let info = HashMap::from([("x".to_string(), String::new())]);
        assert!(modifier.would_change("Genrule", &info));
    }

    #[test]
    fn would_change_add_key_not_present_returns_true() {
        let modifier = convert("Genrule=+x").unwrap();
        assert!(modifier.would_change("Genrule", &HashMap::new()));
    }

    #[test]
    fn would_change_add_key_present_with_empty_value_returns_false() {
        let modifier = convert("Genrule=+x").unwrap();
        let info = HashMap::from([("x".to_string(), String::new())]);
        assert!(!modifier.would_change("Genrule", &info));
    }

    #[test]
    fn would_change_add_key_present_with_non_empty_value_returns_true() {
        let modifier = convert("Genrule=+x").unwrap();
        let info = HashMap::from([("x".to_string(), "val".to_string())]);
        assert!(modifier.would_change("Genrule", &info));
    }
}
