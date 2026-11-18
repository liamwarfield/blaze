//! Port of `com/google/devtools/build/lib/packages/EnumFilterConverter.java` (its `Set<E>` value is
//! `blaze_options::EnumSet`), plus `StringUtil.joinEnglishList`
//! (`com/google/devtools/build/lib/util/StringUtil.java`).

use std::collections::BTreeSet;
use std::marker::PhantomData;

use blaze_options::converters::JavaEnum;
use blaze_options::{
    ConversionContext, Converter, EnumSet, OptionsParsingException, Value, converter::non_null,
};

type ConvResult = Result<Option<Value>, OptionsParsingException>;

// Java: com/google/devtools/build/lib/util/StringUtil.java:35 StringUtil.joinEnglishList
/// Creates a comma-separated list of words as in English, with an Oxford comma:
/// `["a", "b", "c"]` -> `"a, b, or c"`; an empty list is `"nothing"`.
pub fn join_english_list(choices: &[String]) -> String {
    // Java: joinEnglishList(choices, "or", "", /* oxfordComma= */ true)
    let mut buf = String::new();
    for (num_choices_seen, choice) in choices.iter().enumerate() {
        let has_next = num_choices_seen + 1 < choices.len();
        if !buf.is_empty() {
            if has_next || num_choices_seen >= 2 {
                buf.push(',');
            }
            if !has_next {
                buf.push_str(" or");
            }
            buf.push(' ');
        }
        buf.push_str(choice);
    }
    if buf.is_empty() {
        "nothing".to_string()
    } else {
        buf
    }
}

// Java: com/google/devtools/build/lib/packages/EnumFilterConverter.java:34 EnumFilterConverter
/// Converter that translates a comma-separated list of enum values (with optional `-` prefixes
/// for exclusion) into the set of included values.
pub struct EnumFilterConverter<E: JavaEnum> {
    pretty_enum_name: &'static str,
    _e: PhantomData<fn() -> E>,
}

impl<E: JavaEnum + Ord + std::hash::Hash + Eq> EnumFilterConverter<E> {
    pub const fn new(user_friendly_name: &'static str) -> Self {
        EnumFilterConverter {
            pretty_enum_name: user_friendly_name,
            _e: PhantomData,
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnumFilterConverter.java:60 EnumFilterConverter.convert
    pub fn convert_set(&self, input: &str) -> Result<EnumSet<E>, OptionsParsingException> {
        if input.is_empty() {
            return Ok(EnumSet(BTreeSet::new()));
        }
        let mut included_set = BTreeSet::new();
        let mut excluded_set = BTreeSet::new();
        // Java: input.split(",", -1) keeps trailing empty strings.
        for value in input.split(',') {
            let exclude_flag = value.starts_with('-');
            let s = (if exclude_flag { &value[1..] } else { value }).to_uppercase();
            let Some(e) = E::values().iter().find(|e| e.name() == s) else {
                return Err(OptionsParsingException::new(format!(
                    "Invalid {} filter '{}' in the input '{}'",
                    self.pretty_enum_name, value, input
                )));
            };
            if exclude_flag {
                excluded_set.insert(*e);
            } else {
                included_set.insert(*e);
            }
        }
        if included_set.is_empty() {
            included_set = E::values()
                .iter()
                .copied()
                .filter(|e| !excluded_set.contains(e))
                .collect();
        } else {
            included_set.retain(|e| !excluded_set.contains(e));
        }
        if included_set.is_empty() {
            let mut chars = self.pretty_enum_name.chars();
            let first = chars
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            return Err(OptionsParsingException::new(format!(
                "{}{} filter '{}' definition cannot match any tests",
                first,
                chars.as_str(),
                input
            )));
        }
        Ok(EnumSet(included_set))
    }

    // Java: com/google/devtools/build/lib/packages/EnumFilterConverter.java:92 EnumFilterConverter.getTypeDescription
    pub fn type_description(&self) -> String {
        let names: Vec<String> = E::values().iter().map(|e| e.name().to_string()).collect();
        format!(
            "comma-separated list of values: {}",
            join_english_list(&names).to_lowercase()
        )
    }
}

impl<E: JavaEnum + Ord + std::hash::Hash + Eq> Converter for EnumFilterConverter<E> {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_set(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        self.type_description()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_english_list_oxford_comma() {
        let s =
            |v: &[&str]| join_english_list(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&[]), "nothing");
        assert_eq!(s(&["a"]), "a");
        assert_eq!(s(&["a", "b"]), "a or b");
        assert_eq!(s(&["a", "b", "c"]), "a, b, or c");
    }
}
