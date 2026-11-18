//! Label-valued option types and the converters of `analysis/config/CoreOptionConverters.java`.

use std::fmt;
use std::ops::Deref;

use blaze_cmdline::{Label, LabelPackageContext, LabelRepoContextImpl, RepositoryMapping};
use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type, parser::STARLARK_SKIPPED_PREFIXES,
};

pub(crate) type ConvResult = Result<Option<Value>, OptionsParsingException>;

/// A `Label` option value.
///
/// PORT: `blaze_options::FieldType`/`OptionScalar` and `blaze_cmdline::Label` are both foreign to
/// this crate, so the orphan rule forbids implementing one for the other here. Option fields of
/// Java type `Label` use this newtype instead; it dereferences to the `Label`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OptionLabel(pub Label);

impl OptionLabel {
    pub fn label(&self) -> &Label {
        &self.0
    }
}

impl Deref for OptionLabel {
    type Target = Label;
    fn deref(&self) -> &Label {
        &self.0
    }
}

impl From<Label> for OptionLabel {
    fn from(l: Label) -> Self {
        OptionLabel(l)
    }
}

impl fmt::Debug for OptionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for OptionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl OptionScalar for OptionLabel {
    // Java: com/google/devtools/build/lib/cmdline/Label.java:479 Label.toString
    fn java_to_string(&self) -> String {
        self.0.to_string()
    }
}

option_field_type!(
    OptionLabel,
    "com.google.devtools.build.lib.cmdline.Label",
    // Placeholder until the parser sets the field (Java holds null there).
    raw = OptionLabel(Label::parse_canonical_unchecked("//:__unset__"))
);

/// `Map.Entry<Label, String>` (from `Maps.immutableEntry`), the value of
/// [`LabelToStringEntryConverter`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LabelToStringEntry {
    pub key: Label,
    pub value: String,
}

impl OptionScalar for LabelToStringEntry {
    // Java: com/google/common/collect/ImmutableEntry.java toString (`key=value`)
    fn java_to_string(&self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

option_field_type!(
    LabelToStringEntry,
    "java.util.Map$Entry",
    raw = LabelToStringEntry {
        key: Label::parse_canonical_unchecked("//:__unset__"),
        value: String::new(),
    }
);

/// `Map.Entry<String, Label>` (from `Maps.immutableEntry`), the value of [`FlagAliasConverter`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StringToLabelEntry {
    pub key: String,
    pub value: Label,
}

impl OptionScalar for StringToLabelEntry {
    // Java: com/google/common/collect/ImmutableEntry.java toString (`key=value`)
    fn java_to_string(&self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

option_field_type!(
    StringToLabelEntry,
    "java.util.Map$Entry",
    raw = StringToLabelEntry {
        key: String::new(),
        value: Label::parse_canonical_unchecked("//:__unset__"),
    }
);

/// `Map<String, Label>` with nullable values in insertion order (an unmodifiable
/// `LinkedHashMap`), the value of [`LabelMapConverter`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LabelMap(pub Vec<(String, Option<Label>)>);

impl LabelMap {
    /// `Map.get`.
    pub fn get(&self, key: &str) -> Option<&Label> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_ref())
    }
    /// `Map.containsKey`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }
}

impl OptionScalar for LabelMap {
    // Java: java/util/AbstractMap.java toString (`{k=v, k2=null}`)
    fn java_to_string(&self) -> String {
        let mut s = String::from("{");
        for (i, (k, v)) in self.0.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(k);
            s.push('=');
            match v {
                Some(l) => s.push_str(&l.to_string()),
                None => s.push_str("null"),
            }
        }
        s.push('}');
        s
    }
}

option_field_type!(LabelMap, "java.util.Map");

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:67 CoreOptionConverters.BLAZE_ALIASING_FLAG
/// The name of the flag used for shorthand aliasing in blaze.
pub const BLAZE_ALIASING_FLAG: &str = "flag_alias";

// PORT: CoreOptionConverters.BUILD_SETTING_CONVERTERS (keyed by packages.Type) and its private
// StringSetConverter/StarlarkIntConverter belong with the Starlark build-setting port; they are not
// option converters of any FragmentOptions field.

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:331 CoreOptionConverters.convertOptionsLabel
pub fn convert_options_label(
    input: &str,
    conversion_context: Option<&ConversionContext>,
) -> Result<Label, OptionsParsingException> {
    let result = (|| {
        if let Some(ctx) = conversion_context
            && let Some(package_context) = ctx.downcast_ref::<LabelPackageContext>()
        {
            // This can happen if this converter is being used to convert flag values specified in
            // Starlark, for example in a transition implementation function.
            return Label::parse_with_package_context(input, package_context);
        }
        // Check if the input starts with '/'. We don't check for "//" so that
        // we get a better error message if the user accidentally tries to use
        // an absolute path (starting with '/') for a label.
        let input = if !input.starts_with('/') && !input.starts_with('@') {
            format!("//{}", input)
        } else {
            input.to_string()
        };
        let Some(ctx) = conversion_context else {
            // This can happen in the first round of option parsing, before repo mappings are
            // calculated. In this case, it actually doesn't matter how we parse label-typed flags,
            // as they shouldn't be used anywhere anyway.
            return Label::parse_canonical(&input);
        };
        let Some(repo_mapping) = ctx.downcast_ref::<RepositoryMapping>() else {
            panic!(
                "IllegalArgumentException: bad conversion context type: {}",
                // PORT: Java prints conversionContext.getClass().getName().
                "<non-RepositoryMapping conversion context>"
            );
        };
        // This can happen in the second round of option parsing.
        Label::parse_with_repo_context(
            &input,
            &LabelRepoContextImpl::of(
                blaze_cmdline::repository_name::MAIN.clone(),
                repo_mapping.clone(),
            ),
        )
    })();
    result.map_err(|e| OptionsParsingException::new(e.get_message()))
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:130 CoreOptionConverters.LabelConverter
/// A converter from strings to Labels.
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelConverter;

impl LabelConverter {
    pub fn convert_label(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<Label, OptionsParsingException> {
        convert_options_label(input, conversion_context)
    }
}

impl Converter for LabelConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:132 CoreOptionConverters.LabelConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(OptionLabel(
            self.convert_label(non_null(input), ctx)?,
        ))))
    }
    fn get_type_description(&self) -> String {
        "a build target label".into()
    }
}

/// Guava `Splitter.on(",").omitEmptyStrings().split(input)`.
fn split_omit_empty(input: &str) -> impl Iterator<Item = &str> {
    input.split(',').filter(|s| !s.is_empty())
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:143 CoreOptionConverters.LabelListConverter
/// A converter from comma-separated strings to Label lists.
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelListConverter;

impl LabelListConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:145 CoreOptionConverters.LabelListConverter.convert
    pub fn convert_list(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<Vec<Label>, OptionsParsingException> {
        let mut result = Vec::new();
        for label in split_omit_empty(input) {
            result.push(convert_options_label(label, conversion_context)?);
        }
        Ok(result)
    }
}

impl Converter for LabelListConverter {
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list_of(
            self.convert_list(non_null(input), ctx)?
                .into_iter()
                .map(OptionLabel),
        )))
    }
    fn get_type_description(&self) -> String {
        "a build target label".into()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:164 CoreOptionConverters.LabelOrderedSetConverter
/// A converter from comma-separated strings to Labels which preserves order, but in case of
/// duplicates, keeps only the first copy.
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelOrderedSetConverter;

impl Converter for LabelOrderedSetConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:166 CoreOptionConverters.LabelOrderedSetConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let mut already_seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for label in LabelListConverter.convert_list(non_null(input), ctx)? {
            if already_seen.insert(label.clone()) {
                result.push(OptionLabel(label));
            }
        }
        Ok(Some(Value::list_of(result)))
    }
    fn get_type_description(&self) -> String {
        LabelListConverter.get_type_description()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:183 CoreOptionConverters.EmptyToNullLabelConverter
/// A converter that returns null if the input string is empty, otherwise it converts the input
/// to a label.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyToNullLabelConverter;

impl Converter for EmptyToNullLabelConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:186 CoreOptionConverters.EmptyToNullLabelConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(None);
        }
        Ok(Some(Value::new(OptionLabel(convert_options_label(
            input, ctx,
        )?))))
    }
    fn get_type_description(&self) -> String {
        "a build target label".into()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:197 CoreOptionConverters.LabelMapConverter
/// Flag converter for a map of unique keys with optional labels as values.
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelMapConverter;

impl LabelMapConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:199 CoreOptionConverters.LabelMapConverter.convert
    pub fn convert_map(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<LabelMap, OptionsParsingException> {
        // Use LinkedHashMap so we can report duplicate keys more easily while preserving order
        let mut result: Vec<(String, Option<Label>)> = Vec::new();
        // Splitter.on(",").omitEmptyStrings().trimResults(): trimming happens before the
        // emptiness check.
        for entry in input.split(',').map(java_trim).filter(|s| !s.is_empty()) {
            let key;
            let label;
            match entry.find('=') {
                None => {
                    key = entry.to_string();
                    label = None;
                }
                Some(sep_index) => {
                    key = entry[..sep_index].to_string();
                    let value = &entry[sep_index + 1..];
                    label = if value.is_empty() {
                        None
                    } else {
                        Some(convert_options_label(value, conversion_context)?)
                    };
                }
            }
            if result.iter().any(|(k, _)| *k == key) {
                return Err(OptionsParsingException::new(format!(
                    "Key '{}' appears twice",
                    key
                )));
            }
            result.push((key, label));
        }
        Ok(LabelMap(result))
    }
}

/// Guava `CharMatcher.whitespace().trimFrom` as used by `Splitter.trimResults()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(is_guava_whitespace)
}

/// Guava `CharMatcher.whitespace()`.
pub(crate) fn is_guava_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{2002}'
            ..='\u{200A}'
                | '\u{3000}'
                | '\u{0009}'
                | '\u{000A}'
                | '\u{000B}'
                | '\u{000C}'
                | '\u{000D}'
                | '\u{0020}'
                | '\u{0085}'
                | '\u{00A0}'
                | '\u{1680}'
                | '\u{2000}'
                | '\u{2001}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
    )
}

impl Converter for LabelMapConverter {
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_map(non_null(input), ctx)?)))
    }
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:224 CoreOptionConverters.LabelMapConverter.starlarkConvertible
    fn starlark_convertible(&self) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:229 CoreOptionConverters.LabelMapConverter.reverseForStarlark
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        let typed_value = converted.get::<LabelMap>();
        typed_value
            .0
            .iter()
            .map(|(k, v)| match v {
                None => k.clone(),
                Some(l) => format!("{}={}", k, l),
            })
            .collect::<Vec<_>>()
            .join(",")
    }
    fn get_type_description(&self) -> String {
        "a comma-separated list of keys optionally followed by '=' and a label".into()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:248 CoreOptionConverters.LabelToStringEntryConverter
/// Flag converter for assigning a Label to a String.
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelToStringEntryConverter;

impl Converter for LabelToStringEntryConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:250 CoreOptionConverters.LabelToStringEntryConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        // TODO(twigg): This doesn't work well if the labels can themselves have an '='
        let equals_count = input.chars().filter(|c| *c == '=').count();
        // Java's input.charAt(0) throws on an empty input (StringIndexOutOfBoundsException).
        if equals_count != 1 || input.starts_with('=') || input.ends_with('=') {
            return Err(OptionsParsingException::new(
                "Variable definitions must be in the form of a 'name=value' assignment. 'name' and \
                 'value' must be non-empty and may not include '='.",
            ));
        }
        let pos = input.find('=').unwrap();
        let name = convert_options_label(&input[..pos], ctx)?;
        let value = input[pos + 1..].to_string();
        Ok(Some(Value::new(LabelToStringEntry { key: name, value })))
    }
    fn get_type_description(&self) -> String {
        "a 'label=value' assignment".into()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:275 CoreOptionConverters.CustomFlagConverter
/// Flag converter for canonicalizing a label (possibly with a "/..." suffix) and/or define by
/// converting the label to unambiguous canonical form.
#[derive(Clone, Copy, Debug, Default)]
pub struct CustomFlagConverter;

impl CustomFlagConverter {
    pub const SUBPACKAGES_SUFFIX: &'static str = "/...";

    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:279 CoreOptionConverters.CustomFlagConverter.convert
    pub fn convert_string(
        &self,
        input: &str,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<String, OptionsParsingException> {
        if !input.starts_with("//") && !input.starts_with('@') {
            // This is a --define flag.
            return Ok(input.to_string());
        }
        // A "/..." suffix is not valid label syntax, so replace it with arbitrary valid syntax and
        // transform it back after conversion.
        let invalid_subpackages_suffix = Self::SUBPACKAGES_SUFFIX;
        let valid_subpackages_suffix = ":__subpackages__";
        let escaped_unconverted_label = match input.strip_suffix(invalid_subpackages_suffix) {
            Some(stripped) => format!("{}{}", stripped, valid_subpackages_suffix),
            None => input.to_string(),
        };
        let escaped_converted_label =
            convert_options_label(&escaped_unconverted_label, conversion_context)?
                .get_unambiguous_canonical_form();
        if let Some(stripped) = escaped_converted_label.strip_suffix(valid_subpackages_suffix) {
            return Ok(format!("{}{}", stripped, invalid_subpackages_suffix));
        }
        Ok(escaped_converted_label)
    }
}

impl Converter for CustomFlagConverter {
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_string(non_null(input), ctx)?)))
    }
    fn get_type_description(&self) -> String {
        "an absolute label or define".into()
    }
}

fragment_enum! {
    #[starlark_value]
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:311 CoreOptionConverters.StrictDepsMode
    /// Values for the --strict_*_deps option
    pub enum CoreOptionConvertersStrictDepsMode {
        /// Silently allow referencing transitive dependencies.
        OFF,
        /// Warn about transitive dependencies being used directly.
        WARN,
        /// Fail the build when transitive dependencies are used directly.
        ERROR,
        /// Transition to strict by default.
        STRICT,
        /// When no flag value is specified on the command line.
        DEFAULT,
    }
}

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:325 CoreOptionConverters.StrictDepsConverter
blaze_options::enum_converter!(
    pub StrictDepsConverter,
    CoreOptionConvertersStrictDepsMode,
    "strict dependency checking level"
);

// Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:367 CoreOptionConverters.FlagAliasConverter
/// A converter for command line flag aliases. It does additional validation on the name and value
/// of the assignment to ensure they conform to the naming limitations.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlagAliasConverter;

impl Converter for FlagAliasConverter {
    // Java: com/google/devtools/build/lib/analysis/config/CoreOptionConverters.java:370 CoreOptionConverters.FlagAliasConverter.convert
    fn convert(&self, input: Option<&str>, ctx: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        let pos = match input.find('=') {
            Some(p) if p > 0 => p,
            _ => {
                return Err(OptionsParsingException::new(
                    "Flag alias definitions must be in the form of a 'name=label' assignment",
                ));
            }
        };
        let short_form = &input[..pos];
        let long_form = &input[pos + 1..];

        let cmd_line_alias = format!("--{}={}", BLAZE_ALIASING_FLAG, input);

        // Pattern.matches("\\w*", shortForm): Java's \w is [a-zA-Z_0-9].
        if !short_form
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(OptionsParsingException::with_argument(
                format!(
                    "{} should only consist of word characters to be a valid alias name.",
                    short_form
                ),
                cmd_line_alias,
            ));
        }
        if long_form.contains('=') {
            return Err(OptionsParsingException::with_argument(
                format!(
                    "--{} does not support flag value assignment.",
                    BLAZE_ALIASING_FLAG
                ),
                cmd_line_alias,
            ));
        }

        // Remove this check if native options are permitted to be aliased
        let long_form_with_dashes = format!("--{}", long_form);
        if !STARLARK_SKIPPED_PREFIXES
            .iter()
            .any(|p| long_form_with_dashes.starts_with(p))
        {
            return Err(OptionsParsingException::with_argument(
                format!(
                    "--{} only supports Starlark build settings.",
                    BLAZE_ALIASING_FLAG
                ),
                cmd_line_alias,
            ));
        }

        Ok(Some(Value::new(StringToLabelEntry {
            key: short_form.to_string(),
            value: convert_options_label(long_form, ctx)?,
        })))
    }
    fn get_type_description(&self) -> String {
        "a 'name=label' flag alias".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_converter_prefixes_double_slash() {
        let l = LabelConverter.convert_label("foo:bar", None).unwrap();
        assert_eq!(l.to_string(), "//foo:bar");
        let l = LabelConverter.convert_label("@x//foo", None).unwrap();
        assert_eq!(l.get_unambiguous_canonical_form(), "@@x//foo:foo");
    }

    #[test]
    fn label_list_converter_omits_empty() {
        let v = LabelListConverter
            .convert_list(",//a,,//b:c,", None)
            .unwrap();
        assert_eq!(
            v.iter().map(|l| l.to_string()).collect::<Vec<_>>(),
            vec!["//a:a", "//b:c"]
        );
    }

    #[test]
    fn label_ordered_set_converter_keeps_first() {
        let v = LabelOrderedSetConverter
            .convert(Some("//b,//a,//b"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "[//b:b, //a:a]");
    }

    #[test]
    fn label_map_converter() {
        let m = LabelMapConverter
            .convert_map(" a=//x , b, c=", None)
            .unwrap();
        assert_eq!(m.java_to_string(), "{a=//x:x, b=null, c=null}");
        assert_eq!(
            LabelMapConverter
                .convert_map("a,a=//x", None)
                .unwrap_err()
                .get_message(),
            "Key 'a' appears twice"
        );
        assert_eq!(
            LabelMapConverter.reverse_for_starlark(&Value::new(m)),
            "a=//x:x,b,c"
        );
    }

    #[test]
    fn label_to_string_entry_converter() {
        let v = LabelToStringEntryConverter
            .convert(Some("//a:b=k8"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "//a:b=k8");
        for bad in ["=x", "x=", "a=b=c", "ab"] {
            assert!(
                LabelToStringEntryConverter
                    .convert(Some(bad), None)
                    .is_err()
            );
        }
    }

    #[test]
    fn custom_flag_converter() {
        let c = CustomFlagConverter;
        assert_eq!(c.convert_string("my_define", None).unwrap(), "my_define");
        assert_eq!(c.convert_string("//foo:bar", None).unwrap(), "@@//foo:bar");
        assert_eq!(c.convert_string("//foo/...", None).unwrap(), "@@//foo/...");
    }

    #[test]
    fn flag_alias_converter() {
        let v = FlagAliasConverter
            .convert(Some("foo=//bar:baz"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "foo=//bar:baz");
        let e = FlagAliasConverter
            .convert(Some("foo=cpu"), None)
            .unwrap_err();
        assert_eq!(
            e.get_message(),
            "--flag_alias only supports Starlark build settings."
        );
        let e = FlagAliasConverter
            .convert(Some("f-o=//x"), None)
            .unwrap_err();
        assert_eq!(
            e.get_message(),
            "f-o should only consist of word characters to be a valid alias name."
        );
        let e = FlagAliasConverter.convert(Some("=//x"), None).unwrap_err();
        assert_eq!(
            e.get_message(),
            "Flag alias definitions must be in the form of a 'name=label' assignment"
        );
    }
}
