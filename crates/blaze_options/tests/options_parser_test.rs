//! Port of src/test/java/com/google/devtools/common/options/OptionsParserTest.java.
//!
//! Java options classes that extend other options classes are transcribed flattened (the Rust
//! `options!` macro has no inheritance); the parent's fields are copied into the subclass.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use blaze_options::parsed::ParsedOptionRef;
use blaze_options::{
    ArgAndFallbackData, CommaSeparatedOptionListConverter, ConversionContext, Converter,
    EnumConverter, OptionDefinition, OptionInstanceOrigin, OptionPriority, OptionScalar,
    OptionValueDescription, Options, OptionsBase, OptionsClass, OptionsParser,
    OptionsParsingException, ParsedOptionDescription, PriorityCategory, StringConverter, Value,
    Void, option_enum, option_field_type, options,
};

const DUPLICATE_EXCEPTION: &str =
    "com.google.devtools.common.options.DuplicateOptionDeclarationException";

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn parser_for(classes: &[&'static blaze_options::OptionsClassInfo]) -> OptionsParser {
    OptionsParser::builder()
        .options_classes(classes)
        .build()
        .unwrap()
}

fn sv(v: &str) -> Option<Value> {
    Some(Value::new(v.to_string()))
}

options! {
    /// Dummy comment (linter suppression)
    pub struct BadOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub foo1: bool,
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub foo2: bool,
    }
}

options! {
    pub struct ExampleAliasOptions {
        #[option(name = "flag_alias", allow_multiple = true, default_value = "null",
                 documentation_category = UNDOCUMENTED, effect_tags = [NO_OP])]
        pub flag_aliases: Vec<String>,
    }
}

// Java: OptionsParserTest.errorsDuringConstructionAreWrapped
#[test]
fn errors_during_construction_are_wrapped() {
    let e = OptionsParser::builder()
        .options_classes(&[BadOptions::class()])
        .build()
        .err()
        .expect("ConstructionException");
    assert!(e.get_message().starts_with(DUPLICATE_EXCEPTION));
    assert!(e.get_cause_message().is_some());
}

option_enum! {
    pub enum TestEnum { DEFAULT, EXPLICIT }
}

#[allow(dead_code)]
fn test_enum_converter() -> EnumConverter<TestEnum> {
    EnumConverter::new("test enum")
}

#[derive(Default)]
pub struct ChoosyConverter;

impl Converter for ChoosyConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        match input.unwrap() {
            "default" => Ok(sv("default")),
            " explicit" => Ok(sv("explicit")),
            _ => Err(OptionsParsingException::new("illegal")),
        }
    }
    fn get_type_description(&self) -> String {
        "choosy".into()
    }
}

options! {
    pub struct ChoosyOptions {
        #[option(name = "choosy", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "default", converter = ChoosyConverter)]
        pub choosy: String,
    }
}

options! {
    pub struct ExampleFoo {
        #[option(name = "foo", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultFoo")]
        pub foo: String,
        #[option(name = "bar", category = "two", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "42")]
        pub bar: i32,
        #[option(name = "bing", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", allow_multiple = true)]
        pub bing: Vec<String>,
        #[option(name = "bang", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", converter = StringConverter,
                 allow_multiple = true)]
        pub bang: Vec<String>,
        #[option(name = "nodoc", documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "")]
        pub nodoc: String,
    }
}

options! {
    pub struct ExampleBaz {
        #[option(name = "baz", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultBaz")]
        pub baz: String,
    }
}

options! {
    /// Subclass of an options class (flattened: includes ExampleBaz's fields).
    pub struct ExampleBazSubclass {
        #[option(name = "baz", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultBaz")]
        pub baz: String,
        #[option(name = "baz_subclass", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultBazSubclass")]
        pub baz_subclass: String,
    }
}

/// A converter that defaults to null if the input is the empty string.
#[derive(Default)]
pub struct EmptyToNullStringConverter;

impl Converter for EmptyToNullStringConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = input.unwrap();
        Ok(if input.is_empty() { None } else { sv(input) })
    }
    fn get_type_description(&self) -> String {
        "a string".into()
    }
}

options! {
    /// Example with empty to null string converter
    pub struct ExampleBoom {
        #[option(name = "boom", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "defaultBoom", converter = EmptyToNullStringConverter)]
        pub boom: Option<String>,
    }
}

options! {
    /// Example with internal options
    pub struct ExampleInternalOptions {
        #[option(name = "internal_boolean", metadata_tags = [INTERNAL],
                 documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "true")]
        pub private_boolean: bool,
        #[option(name = "internal_string", metadata_tags = [INTERNAL],
                 documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "super secret")]
        pub private_string: String,
    }
}

options! {
    pub struct ExampleEquivalentWithFoo {
        #[option(name = "foo", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "differentDefault")]
        pub foo: String,
        #[option(name = "bar", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "differentDefault")]
        pub bar: String,
        #[option(name = "ignored_with_value", category = "one",
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "differentDefault")]
        pub ignored_with_value: String,
        #[option(name = "ignored_without_value", category = "one",
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub ignored_without_value: bool,
    }
}

options! {
    pub struct BooleanAliasOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "true")]
        pub foo: bool,
        #[option(name = "bar", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "true")]
        pub bar: bool,
        #[option(name = "flag_alias", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", allow_multiple = true)]
        pub flag_alias: Vec<String>,
    }
}

options! {
    pub struct DeprecatedAliasOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "true", deprecation_warning = "Don't use foo.")]
        pub foo: bool,
        #[option(name = "bar", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "true")]
        pub bar: bool,
        #[option(name = "flag_alias", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", allow_multiple = true)]
        pub flag_alias: Vec<String>,
    }
}

options! {
    pub struct ExampleIncompatibleWithFoo {
        #[option(name = "foo", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "true")]
        pub foo: bool,
    }
}

// Java: OptionsParserTest.defaultValueOfBadOptionRemains
#[test]
fn default_value_of_bad_option_remains() {
    let mut parser = parser_for(&[ChoosyOptions::class()]);
    assert!(parser.parse(&["--choosy=wat"]).is_err());
    assert_eq!(
        parser.get_options::<ChoosyOptions>().unwrap().choosy,
        "default"
    );
}

// Java: OptionsParserTest.parseWithMultipleOptionsInterfaces
#[test]
fn parse_with_multiple_options_interfaces() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    parser.parse(&["--baz=oops", "--bar", "17"]).unwrap();
    let foo = parser.get_options::<ExampleFoo>().unwrap();
    assert_eq!(foo.foo, "defaultFoo");
    assert_eq!(foo.bar, 17);
    let baz = parser.get_options::<ExampleBaz>().unwrap();
    assert_eq!(baz.baz, "oops");
}

// Java: OptionsParserTest.parseWithSourceFunctionThrowsExceptionIfResidueIsNotAllowed
#[test]
fn parse_with_source_function_throws_exception_if_residue_is_not_allowed() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[ExampleFoo::class(), ExampleBaz::class()])
        .allow_residue(false)
        .build()
        .unwrap();
    let e = parser
        .parse_with_source_function(
            PriorityCategory::COMMAND_LINE,
            &|_| Some("command line".to_string()),
            s(&["residue", "not", "allowed", "in", "parseWithSource"]),
            None,
        )
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unrecognized arguments: residue not allowed in parseWithSource"
    );
    assert_eq!(
        parser.get_residue(),
        s(&["residue", "not", "allowed", "in", "parseWithSource"])
    );
}

// Java: OptionsParserTest.parseWithSourceFunctionDoesntThrowExceptionIfResidueIsAllowed
#[test]
fn parse_with_source_function_doesnt_throw_exception_if_residue_is_allowed() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[ExampleFoo::class(), ExampleBaz::class()])
        .allow_residue(true)
        .build()
        .unwrap();
    parser
        .parse_with_source_function(
            PriorityCategory::COMMAND_LINE,
            &|_| Some("command line".to_string()),
            s(&["residue", "is", "allowed", "in", "parseWithSource"]),
            None,
        )
        .unwrap();
    assert_eq!(
        parser.get_residue(),
        s(&["residue", "is", "allowed", "in", "parseWithSource"])
    );
}

// Java: OptionsParserTest.parseArgsAsExpansionOfOptionThrowsExceptionIfResidueIsNotAllowed
#[test]
fn parse_args_as_expansion_of_option_throws_exception_if_residue_is_not_allowed() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[ExpansionOptions::class()])
        .allow_residue(false)
        .build()
        .unwrap();
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--expands"]))
        .unwrap();
    assert!(parser.get_option_value_description("expands").is_some());
    let option_value = parser.get_option_value_description("underlying").unwrap();
    let option_to_expand = option_value.get_canonical_instances().unwrap()[0].clone();
    let e = parser
        .parse_args_as_expansion_of_option(
            &option_to_expand,
            "source",
            ArgAndFallbackData::wrap_with_fallback_data(
                ["--underlying=direct_value", "residue", "in", "expansion"],
                None,
            ),
        )
        .unwrap_err();
    assert!(!parser.get_residue().is_empty());
    assert_eq!(
        e.get_message(),
        "Unrecognized arguments: residue in expansion"
    );
}

// Java: OptionsParserTest.parseWithOptionsInheritance
#[test]
fn parse_with_options_inheritance() {
    let mut parser = parser_for(&[ExampleBazSubclass::class()]);
    parser.parse(&["--baz_subclass=cat", "--baz=dog"]).unwrap();
    let subclass_options = parser.get_options::<ExampleBazSubclass>().unwrap();
    assert_eq!(subclass_options.baz_subclass, "cat");
    assert_eq!(subclass_options.baz, "dog");
    assert!(parser.get_options::<ExampleBaz>().is_none());
}

// Java: OptionsParserTest.parserWithUnknownOption
#[test]
fn parser_with_unknown_option() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    let e = parser.parse(&["--unknown", "option"]).unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--unknown"));
    assert_eq!(e.get_message(), "Unrecognized option: --unknown");
    assert!(parser.get_residue().is_empty());
}

// Java: OptionsParserTest.parserWithSingleDashOption_notAllowed
#[test]
fn parser_with_single_dash_option_not_allowed() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    assert!(parser.parse(&["-baz=oops", "-bar", "17"]).is_err());
}

// Java: OptionsParserTest.parsingFailsWithUnknownOptions
#[test]
fn parsing_fails_with_unknown_options() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    let e = parser
        .parse_list(s(&["--unknown", "option", "--more_unknowns"]))
        .unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--unknown"));
    assert_eq!(e.get_message(), "Unrecognized option: --unknown");
    assert!(parser.get_options::<ExampleFoo>().is_some());
    assert!(parser.get_options::<ExampleBaz>().is_some());
}

// Java: OptionsParserTest.parsingFailsWithInternalBooleanOptionAsIfUnknown
#[test]
fn parsing_fails_with_internal_boolean_option_as_if_unknown() {
    let mut parser = parser_for(&[ExampleInternalOptions::class()]);
    let e = parser.parse_list(s(&["--internal_boolean"])).unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--internal_boolean"));
    assert_eq!(e.get_message(), "Unrecognized option: --internal_boolean");
    assert!(parser.get_options::<ExampleInternalOptions>().is_some());
}

// Java: OptionsParserTest.parsingFailsWithNegatedInternalBooleanOptionAsIfUnknown
#[test]
fn parsing_fails_with_negated_internal_boolean_option_as_if_unknown() {
    let mut parser = parser_for(&[ExampleInternalOptions::class()]);
    let e = parser.parse_list(s(&["--nointernal_boolean"])).unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--nointernal_boolean"));
    assert_eq!(e.get_message(), "Unrecognized option: --nointernal_boolean");
    assert!(parser.get_options::<ExampleInternalOptions>().is_some());
}

// Java: OptionsParserTest.parsingFailsForInternalOptionWithValueInSameArgAsIfUnknown
#[test]
fn parsing_fails_for_internal_option_with_value_in_same_arg_as_if_unknown() {
    let mut parser = parser_for(&[ExampleInternalOptions::class()]);
    let e = parser
        .parse_list(s(&["--internal_string=any_value"]))
        .unwrap_err();
    assert_eq!(
        e.get_invalid_argument(),
        Some("--internal_string=any_value")
    );
    assert_eq!(
        e.get_message(),
        "Unrecognized option: --internal_string=any_value"
    );
    assert!(parser.get_options::<ExampleInternalOptions>().is_some());
}

// Java: OptionsParserTest.parsingFailsForInternalOptionWithValueInSeparateArgAsIfUnknown
#[test]
fn parsing_fails_for_internal_option_with_value_in_separate_arg_as_if_unknown() {
    let mut parser = parser_for(&[ExampleInternalOptions::class()]);
    let e = parser
        .parse_list(s(&["--internal_string", "any_value"]))
        .unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--internal_string"));
    assert_eq!(e.get_message(), "Unrecognized option: --internal_string");
    assert!(parser.get_options::<ExampleInternalOptions>().is_some());
}

// Java: OptionsParserTest.parseKnownAndUnknownOptions
#[test]
fn parse_known_and_unknown_options() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    let e = parser
        .parse_list(s(&["--bar", "17", "--unknown", "option"]))
        .unwrap_err();
    assert_eq!(e.get_invalid_argument(), Some("--unknown"));
    assert_eq!(e.get_message(), "Unrecognized option: --unknown");
    assert!(parser.get_options::<ExampleFoo>().is_some());
    assert!(parser.get_options::<ExampleBaz>().is_some());
}

// Java: OptionsParserTest.parseAndOverrideWithEmptyStringToObtainNullValueInOption
#[test]
fn parse_and_override_with_empty_string_to_obtain_null_value_in_option() {
    let mut parser = parser_for(&[ExampleBoom::class()]);
    parser.parse(&["--boom="]).unwrap();
    let boom = parser.get_options::<ExampleBoom>().unwrap();
    assert_eq!(boom.boom, None);
}

options! {
    pub struct CategoryTest {
        #[option(name = "swiss_bank_account_number", documentation_category = UNDOCUMENTED,
                 effect_tags = [NO_OP], default_value = "123456789")]
        pub swiss_bank_account_number: i32,
        #[option(name = "student_bank_account_number", category = "one",
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "987654321")]
        pub student_bank_account_number: i32,
    }
}

// Java: OptionsParserTest.getOptionsAndGetResidueWithNoCallToParse
#[test]
fn get_options_and_get_residue_with_no_call_to_parse() {
    assert_eq!(
        parser_for(&[ExampleFoo::class()])
            .get_options::<ExampleFoo>()
            .unwrap()
            .foo,
        "defaultFoo"
    );
    assert!(parser_for(&[ExampleFoo::class()]).get_residue().is_empty());
}

// Java: OptionsParserTest.parserCanBeCalledRepeatedly
#[test]
fn parser_can_be_called_repeatedly() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser.parse(&["--foo", "foo1"]).unwrap();
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().foo, "foo1");
    parser.parse(&[]).unwrap();
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().foo, "foo1");
    parser.parse(&["--foo", "foo2"]).unwrap();
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().foo, "foo2");
}

// Java: OptionsParserTest.multipleOccurringOption
#[test]
fn multiple_occurring_option() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser
        .parse(&["--bing", "abcdef", "--foo", "foo1", "--bing", "123456"])
        .unwrap();
    assert_eq!(
        sorted(parser.get_options::<ExampleFoo>().unwrap().bing),
        sorted(s(&["abcdef", "123456"]))
    );
}

// Java: OptionsParserTest.multipleOccurringOptionWithConverter
#[test]
fn multiple_occurring_option_with_converter() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser
        .parse(&["--bang", "abc,def ghi", "--foo", "foo1", "--bang", "123456"])
        .unwrap();
    assert_eq!(
        sorted(parser.get_options::<ExampleFoo>().unwrap().bang),
        sorted(s(&["abc,def ghi", "123456"]))
    );
}

// Java: OptionsParserTest.parserIgnoresOptionsAfterMinusMinus
#[test]
fn parser_ignores_options_after_minus_minus() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    parser
        .parse(&["--foo", "well", "--baz", "here", "--", "--bar", "ignore"])
        .unwrap();
    let foo = parser.get_options::<ExampleFoo>().unwrap();
    let baz = parser.get_options::<ExampleBaz>().unwrap();
    assert_eq!(foo.foo, "well");
    assert_eq!(baz.baz, "here");
    assert_eq!(foo.bar, 42);
    assert_eq!(parser.get_residue(), s(&["--bar", "ignore"]));
}

// Java: OptionsParserTest.parserThrowsExceptionIfResidueIsNotAllowed
#[test]
fn parser_throws_exception_if_residue_is_not_allowed() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[ExampleFoo::class()])
        .allow_residue(false)
        .build()
        .unwrap();
    let e = parser.parse(&["residue", "is", "not", "OK"]).unwrap_err();
    assert_eq!(e.get_message(), "Unrecognized arguments: residue is not OK");
}

// Java: OptionsParserTest.multipleCallsToParse
#[test]
fn multiple_calls_to_parse() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[ExampleFoo::class()])
        .allow_residue(true)
        .build()
        .unwrap();
    parser
        .parse(&["--foo", "one", "--bar", "43", "unknown1"])
        .unwrap();
    parser.parse(&["--foo", "two", "unknown2"]).unwrap();
    let foo = parser.get_options::<ExampleFoo>().unwrap();
    assert_eq!(foo.foo, "two");
    assert_eq!(foo.bar, 43);
    assert_eq!(parser.get_residue(), s(&["unknown1", "unknown2"]));
}

// Java: OptionsParserTest.toStringDoesntIncludeFlagsForOtherOptionsInParserInstance
#[test]
fn to_string_doesnt_include_flags_for_other_options_in_parser_instance() {
    let mut parser = parser_for(&[ExampleFoo::class(), ExampleBaz::class()]);
    parser
        .parse(&["--foo", "foo", "--bar", "43", "--baz", "baz"])
        .unwrap();
    let foo_string =
        <dyn OptionsBase>::java_to_string(&parser.get_options::<ExampleFoo>().unwrap());
    assert!(
        foo_string.contains("foo=foo")
            && foo_string.contains("bar=43")
            && foo_string.contains("ExampleFoo")
            && !foo_string.contains("baz=baz"),
        "ExampleFoo.toString() is incorrect: {}",
        foo_string
    );
    let baz_string =
        <dyn OptionsBase>::java_to_string(&parser.get_options::<ExampleBaz>().unwrap());
    assert!(
        baz_string.contains("baz=baz")
            && baz_string.contains("ExampleBaz")
            && !baz_string.contains("foo=foo")
            && !baz_string.contains("bar=43"),
        "ExampleBaz.toString() is incorrect: {}",
        baz_string
    );
}

// Java: OptionsParserTest.toStringIsIndependentOfExplicitCommandLineOptions
#[test]
fn to_string_is_independent_of_explicit_command_line_options() {
    let foo1 = Options::<ExampleFoo>::parse(&[]).unwrap().into_options();
    let foo2 = Options::<ExampleFoo>::parse(&["--bar", "42"])
        .unwrap()
        .into_options();
    assert_eq!(foo2, foo1);
    assert_eq!(
        <dyn OptionsBase>::java_to_string(&foo2),
        <dyn OptionsBase>::java_to_string(&foo1)
    );

    let expected: HashMap<String, Option<Value>> = [
        ("bing", Some(Value::list(vec![]))),
        ("bar", Some(Value::new(42i32))),
        ("nodoc", sv("")),
        ("bang", Some(Value::list(vec![]))),
        ("foo", sv("defaultFoo")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let m1: HashMap<String, Option<Value>> = <dyn OptionsBase>::as_map(&foo1).into_iter().collect();
    let m2: HashMap<String, Option<Value>> = <dyn OptionsBase>::as_map(&foo2).into_iter().collect();
    assert_eq!(m1, expected);
    assert_eq!(m2, expected);
}

options! {
    /// DerivedBaz extends ExampleBaz (flattened).
    pub struct DerivedBaz {
        #[option(name = "baz", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultBaz")]
        pub baz: String,
        #[option(name = "derived", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "defaultDerived")]
        pub derived: String,
    }
}

// Java: OptionsParserTest.toStringPrintsInheritedOptionsToo_Duh
#[test]
fn to_string_prints_inherited_options_too_duh() {
    let derived_baz = Options::<DerivedBaz>::parse(&[]).unwrap().into_options();
    let derived_baz_string = <dyn OptionsBase>::java_to_string(&derived_baz);
    assert!(
        derived_baz_string.contains("derived=defaultDerived")
            && derived_baz_string.contains("baz=defaultBaz"),
        "DerivedBaz.toString() is incorrect: {}",
        derived_baz_string
    );
}

options! {
    pub struct CustomOptions {
        #[option(name = "simple", category = "custom", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "simple default")]
        pub simple: String,
        #[option(name = "multipart_name", category = "custom",
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "multipart default")]
        pub multipart_name: String,
    }
}

// Java: OptionsParserTest.assertDefaultStringsForCustomOptions
#[test]
fn assert_default_strings_for_custom_options() {
    let options = Options::<CustomOptions>::parse(&[]).unwrap().into_options();
    assert_eq!(options.simple, "simple default");
    assert_eq!(options.multipart_name, "multipart default");
}

options! {
    pub struct NullTestOptions {
        #[option(name = "simple", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub simple: Option<String>,
    }
}

// Java: OptionsParserTest.defaultNullStringGivesNull
#[test]
fn default_null_string_gives_null() {
    let options = Options::<NullTestOptions>::parse(&[])
        .unwrap()
        .into_options();
    assert_eq!(options.simple, None);
}

#[derive(Default)]
pub struct ConverterWithContext;

impl Converter for ConverterWithContext {
    fn convert(
        &self,
        input: Option<&str>,
        conversion_context: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = input.unwrap();
        if let Some(ctx) = conversion_context {
            let prefix = ctx.downcast_ref::<String>().unwrap();
            return Ok(sv(&format!("{}{}", prefix, input)));
        }
        Ok(sv(input))
    }
    fn get_type_description(&self) -> String {
        "a funky string".into()
    }
}

options! {
    pub struct ConverterWithContextTestOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED,
                 converter = ConverterWithContext, effect_tags = [NO_OP], default_value = "bar")]
        pub foo: String,
    }
}

// Java: OptionsParserTest.convertWithContext
#[test]
fn convert_with_context() {
    let ctx: Arc<ConversionContext> = Arc::new("bleh ".to_string());
    let mut parser = OptionsParser::builder()
        .options_classes(&[ConverterWithContextTestOptions::class()])
        .with_conversion_context(ctx)
        .build()
        .unwrap();
    parser.parse(&["--foo", "quux"]).unwrap();
    let options = parser
        .get_options::<ConverterWithContextTestOptions>()
        .unwrap();
    assert_eq!(options.foo, "bleh quux");
}

options! {
    pub struct ImplicitDependencyOptions {
        #[option(name = "first", implicit_requirements = ["--second=second"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub first: Option<String>,
        #[option(name = "second", implicit_requirements = ["--third=third"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub second: Option<String>,
        #[option(name = "third", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub third: Option<String>,
    }
}

// Java: OptionsParserTest.implicitDependencyHasImplicitDependency
#[test]
fn implicit_dependency_has_implicit_dependency() {
    let mut parser = parser_for(&[ImplicitDependencyOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first=first"]))
        .unwrap();
    let o = parser.get_options::<ImplicitDependencyOptions>().unwrap();
    assert_eq!(o.first.as_deref(), Some("first"));
    assert_eq!(o.second.as_deref(), Some("second"));
    assert_eq!(o.third.as_deref(), Some("third"));
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct BadImplicitDependencyOptions {
        #[option(name = "first", implicit_requirements = ["xxx"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub first: Option<String>,
    }
}

// Java: OptionsParserTest.badImplicitDependency
#[test]
#[should_panic(expected = "AssertionError")]
fn bad_implicit_dependency() {
    let mut parser = parser_for(&[BadImplicitDependencyOptions::class()]);
    let _ = parser.parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first=first"]));
}

options! {
    pub struct BadExpansionOptions {
        #[option(name = "first", expansion = ["xxx"], documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null")]
        pub first: Void,
    }
}

// Java: OptionsParserTest.badExpansionOptions
#[test]
#[should_panic(expected = "AssertionError")]
fn bad_expansion_options() {
    let mut parser = parser_for(&[BadExpansionOptions::class()]);
    let _ = parser.parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first"]));
}

options! {
    /// ExpansionOptions
    pub struct ExpansionOptions {
        #[option(name = "underlying", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null")]
        pub underlying: Option<String>,
        #[option(name = "expands", expansion = ["--underlying=from_expansion"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub expands: Void,
    }
}

// Java: OptionsParserTest.describeOptionsWithExpansion
#[test]
#[ignore = "needs OptionsUsage (describeOptionsWithDeprecatedCategories), not ported"]
fn describe_options_with_expansion() {
    let parser = parser_for(&[ExpansionOptions::class()]);
    let _ = parser.describe_options();
}

// Java: OptionsParserTest.overrideExpansionWithExplicit
#[test]
fn override_expansion_with_explicit() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            s(&["--expands", "--underlying=direct_value"]),
        )
        .unwrap();
    let options = parser.get_options::<ExpansionOptions>().unwrap();
    assert_eq!(options.underlying.as_deref(), Some("direct_value"));
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.testExpansionOriginIsPropagatedToOption
#[test]
fn test_expansion_origin_is_propagated_to_option() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--expands"]))
        .unwrap();
    let expansion_description = parser.get_option_value_description("expands").unwrap();
    let option_value = parser.get_option_value_description("underlying").unwrap();
    assert_eq!(
        option_value.get_source_string().as_deref(),
        Some("expanded from option '--expands'")
    );
    let instances = option_value.get_canonical_instances().unwrap();
    assert_eq!(instances.len(), 1);
    let effective_instance = &instances[0];
    assert!(std::ptr::eq(
        effective_instance
            .get_expanded_from()
            .unwrap()
            .get_option_definition(),
        expansion_description.get_option_definition()
    ));
    assert!(effective_instance.get_implicit_dependent().is_none());
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.overrideExplicitWithExpansion
#[test]
fn override_explicit_with_expansion() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            s(&["--underlying=direct_value", "--expands"]),
        )
        .unwrap();
    let options = parser.get_options::<ExpansionOptions>().unwrap();
    assert_eq!(options.underlying.as_deref(), Some("from_expansion"));
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "option '--expands' was expanded and now overrides the explicit option \
             --underlying=direct_value with --underlying=from_expansion"
        ])
    );
}

// Java: OptionsParserTest.noWarningsWhenOverrideExplicitWithExpansion
#[test]
fn no_warnings_when_override_explicit_with_expansion() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            None,
            s(&["--underlying=direct_value", "--expands"]),
        )
        .unwrap();
    let options = parser.get_options::<ExpansionOptions>().unwrap();
    assert_eq!(options.underlying.as_deref(), Some("from_expansion"));
    assert!(
        !parser.get_warnings().contains(
            &"option '--expands' was expanded and now overrides the explicit option \
          --underlying=direct_value with --underlying=from_expansion"
                .to_string()
        )
    );
}

// Java: OptionsParserTest.noWarningsWhenValueNotChanged
#[test]
fn no_warnings_when_value_not_changed() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            s(&["--underlying=from_expansion", "--expands"]),
        )
        .unwrap();
    let options = parser.get_options::<ExpansionOptions>().unwrap();
    assert_eq!(options.underlying.as_deref(), Some("from_expansion"));
    assert!(parser.get_warnings().is_empty());
}

options! {
    /// ExpansionOptions to allow-multiple values.
    pub struct ExpansionOptionsToMultiple {
        #[option(name = "underlying", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", allow_multiple = true)]
        pub underlying: Vec<String>,
        #[option(name = "expands", expansion = ["--underlying=from_expansion"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub expands: Void,
    }
}

// Java: OptionsParserTest.multipleExpansionOptionsWithValue
#[test]
fn multiple_expansion_options_with_value() {
    let mut parser = parser_for(&[ExpansionOptionsToMultiple::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            s(&["--expands", "--underlying=direct_value", "--expands"]),
        )
        .unwrap();
    let options = parser.get_options::<ExpansionOptionsToMultiple>().unwrap();
    assert_eq!(
        options.underlying,
        s(&["from_expansion", "direct_value", "from_expansion"])
    );
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.checkExpansionValueWarning
#[test]
fn check_expansion_value_warning() {
    let mut parser = parser_for(&[ExpansionOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--expands=no"]))
        .unwrap();
    let options = parser.get_options::<ExpansionOptions>().unwrap();
    assert_eq!(options.underlying.as_deref(), Some("from_expansion"));
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "option '--expands' is an expansion option. It does not accept values, and does not \
             change its expansion based on the value provided. Value 'no' will be ignored."
        ])
    );
}

// Java: OptionsParserTest.overrideWithHigherPriority
#[test]
fn override_with_higher_priority() {
    let mut parser = parser_for(&[NullTestOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::RC_FILE, None, s(&["--simple=a"]))
        .unwrap();
    assert_eq!(
        parser
            .get_options::<NullTestOptions>()
            .unwrap()
            .simple
            .as_deref(),
        Some("a")
    );
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--simple=b"]))
        .unwrap();
    assert_eq!(
        parser
            .get_options::<NullTestOptions>()
            .unwrap()
            .simple
            .as_deref(),
        Some("b")
    );
}

// Java: OptionsParserTest.overrideWithLowerPriority
#[test]
fn override_with_lower_priority() {
    let mut parser = parser_for(&[NullTestOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--simple=a"]))
        .unwrap();
    assert_eq!(
        parser
            .get_options::<NullTestOptions>()
            .unwrap()
            .simple
            .as_deref(),
        Some("a")
    );
    parser
        .parse_with_priority(PriorityCategory::RC_FILE, None, s(&["--simple=b"]))
        .unwrap();
    assert_eq!(
        parser
            .get_options::<NullTestOptions>()
            .unwrap()
            .simple
            .as_deref(),
        Some("a")
    );
}

// Java: OptionsParserTest.getOptionValueDescriptionWithNonExistingOption
#[test]
#[should_panic(expected = "IllegalArgumentException")]
fn get_option_value_description_with_non_existing_option() {
    let parser = parser_for(&[NullTestOptions::class()]);
    let _ = parser.get_option_value_description("notexisting");
}

// Java: OptionsParserTest.getOptionValueDescriptionWithoutValue
#[test]
fn get_option_value_description_without_value() {
    let parser = parser_for(&[NullTestOptions::class()]);
    assert!(parser.get_option_value_description("simple").is_none());
}

// Java: OptionsParserTest.getOptionValueDescriptionWithValue
#[test]
fn get_option_value_description_with_value() {
    let mut parser = parser_for(&[NullTestOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("my description"),
            s(&["--simple=abc"]),
        )
        .unwrap();
    let result = parser.get_option_value_description("simple").unwrap();
    assert_eq!(result.get_option_definition().get_option_name(), "simple");
    assert_eq!(result.get_value(), sv("abc"));
    assert_eq!(
        result.get_source_string().as_deref(),
        Some("my description")
    );
    let instances = result.get_canonical_instances().unwrap();
    assert_eq!(instances.len(), 1);
    let single_option_instance = &instances[0];
    assert_eq!(
        single_option_instance
            .get_priority()
            .get_priority_category(),
        PriorityCategory::COMMAND_LINE
    );
    assert!(
        !single_option_instance
            .get_option_definition()
            .is_expansion_option()
    );
    assert!(single_option_instance.get_implicit_dependent().is_none());
    assert!(single_option_instance.get_expanded_from().is_none());
}

options! {
    pub struct ImplicitDependencyWarningOptions {
        #[option(name = "first", implicit_requirements = ["--second=requiredByFirst"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub first: bool,
        #[option(name = "second", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub second: Option<String>,
        #[option(name = "third", implicit_requirements = ["--second=requiredByThird"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub third: Option<String>,
    }
}

// Java: OptionsParserTest.warningForImplicitOverridingExplicitOption
#[test]
fn warning_for_implicit_overriding_explicit_option() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser.parse(&["--second=second", "--first"]).unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "option '--second' is implicitly defined by option '--first'; the implicitly set \
             value overrides the previous one"
        ])
    );
}

// Java: OptionsParserTest.warningForExplicitOverridingImplicitOption
#[test]
fn warning_for_explicit_overriding_implicit_option() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser.parse(&["--first"]).unwrap();
    assert!(parser.get_warnings().is_empty());
    parser.parse(&["--second=second"]).unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "A new value for option '--second' overrides a previous implicit setting of that \
             option by option '--first'"
        ])
    );
}

// Java: OptionsParserTest.warningForExplicitOverridingImplicitOptionInSameCall
#[test]
fn warning_for_explicit_overriding_implicit_option_in_same_call() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser.parse(&["--first", "--second=second"]).unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "A new value for option '--second' overrides a previous implicit setting of that \
             option by option '--first'"
        ])
    );
}

// Java: OptionsParserTest.warningForImplicitOverridingImplicitOption
#[test]
fn warning_for_implicit_overriding_implicit_option() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser.parse(&["--first"]).unwrap();
    assert!(parser.get_warnings().is_empty());
    parser.parse(&["--third=third"]).unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "option '--second' is implicitly defined by both option '--first' and option \
             '--third=third'"
        ])
    );
}

// Java: OptionsParserTest.noWarningsForNonConflictingOverrides
#[test]
fn no_warnings_for_non_conflicting_overrides() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser
        .parse(&["--first", "--second=requiredByFirst"])
        .unwrap();
    let options = parser
        .get_options::<ImplicitDependencyWarningOptions>()
        .unwrap();
    assert!(options.first);
    assert_eq!(options.second.as_deref(), Some("requiredByFirst"));
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.warningForImplicitRequirementsExpandedForDefaultValue
#[test]
fn warning_for_implicit_requirements_expanded_for_default_value() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser.parse(&["--nofirst"]).unwrap();
    let options = parser
        .get_options::<ImplicitDependencyWarningOptions>()
        .unwrap();
    assert!(!options.first);
    assert_eq!(options.second.as_deref(), Some("requiredByFirst"));
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "--nofirst sets option '--first' to its default value. Since this option has \
             implicit requirements that are set whenever the option is explicitly provided, \
             regardless of the value, this will behave differently than letting a default be a \
             default. Specifically, this options expands to {--second=requiredByFirst}."
        ])
    );
}

// Java: OptionsParserTest.testDependentOriginIsPropagatedToOption
#[test]
fn test_dependent_origin_is_propagated_to_option() {
    let mut parser = parser_for(&[ImplicitDependencyWarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first"]))
        .unwrap();
    let first = parser.get_option_value_description("first").unwrap();
    assert_eq!(first.get_canonical_instances().unwrap().len(), 1);
    let second = parser.get_option_value_description("second").unwrap();
    assert_eq!(
        second.get_source_string().as_deref(),
        Some("implicit requirement of option '--first'")
    );
    assert_eq!(second.get_canonical_instances().unwrap().len(), 0);
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct ImplicitDependencyOnAllowMultipleOptions {
        #[option(name = "first", implicit_requirements = ["--second=requiredByFirst"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub first: bool,
        #[option(name = "second", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", allow_multiple = true)]
        pub second: Vec<String>,
        #[option(name = "third", implicit_requirements = ["--second=requiredByThird"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub third: Option<String>,
    }
}

// Java: OptionsParserTest.testCanonicalizeExcludesImplicitDependencyOnRepeatableOption
#[test]
fn test_canonicalize_excludes_implicit_dependency_on_repeatable_option() {
    let mut parser = parser_for(&[ImplicitDependencyOnAllowMultipleOptions::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            None,
            s(&["--first", "--second=explicitValue"]),
        )
        .unwrap();
    let first = parser.get_option_value_description("first").unwrap();
    assert_eq!(first.get_canonical_instances().unwrap().len(), 1);
    let second = parser.get_option_value_description("second").unwrap();
    assert_eq!(
        second.get_source_string().as_deref(),
        Some("implicit requirement of option '--first', null")
    );
    assert_eq!(second.get_canonical_instances().unwrap().len(), 1);
    assert_eq!(
        sorted(parser.canonicalize()),
        sorted(s(&["--first=1", "--second=explicitValue"]))
    );
    let options = parser
        .get_options::<ImplicitDependencyOnAllowMultipleOptions>()
        .unwrap();
    assert!(options.first);
    assert_eq!(
        sorted(options.second),
        sorted(s(&["explicitValue", "requiredByFirst"]))
    );
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.testCanonicalizeExcludesImplicitDependencyForOtherwiseUnmentionedRepeatableOption
#[test]
fn test_canonicalize_excludes_implicit_dependency_for_otherwise_unmentioned_repeatable_option() {
    let mut parser = parser_for(&[ImplicitDependencyOnAllowMultipleOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first"]))
        .unwrap();
    let first = parser.get_option_value_description("first").unwrap();
    assert_eq!(first.get_canonical_instances().unwrap().len(), 1);
    let second = parser.get_option_value_description("second").unwrap();
    assert_eq!(
        second.get_source_string().as_deref(),
        Some("implicit requirement of option '--first'")
    );
    assert!(second.get_canonical_instances().unwrap().is_empty());
    assert_eq!(parser.canonicalize(), s(&["--first=1"]));
    let options = parser
        .get_options::<ImplicitDependencyOnAllowMultipleOptions>()
        .unwrap();
    assert!(options.first);
    assert_eq!(options.second, s(&["requiredByFirst"]));
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct WarningOptions {
        #[option(name = "first", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", deprecated = true)]
        pub first: Void,
        #[option(name = "second", allow_multiple = true, documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null", deprecated = true)]
        pub second: Vec<String>,
        #[option(name = "third", expansion = ["--fourth=true"], abbrev = 't',
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", deprecated = true)]
        pub third: Void,
        #[option(name = "fourth", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub fourth: bool,
    }
}

// Java: OptionsParserTest.deprecationWarning
#[test]
fn deprecation_warning() {
    let mut parser = parser_for(&[WarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first"]))
        .unwrap();
    assert_eq!(parser.get_warnings(), s(&["Option 'first' is deprecated"]));
}

// Java: OptionsParserTest.deprecationWarningForListOption
#[test]
fn deprecation_warning_for_list_option() {
    let mut parser = parser_for(&[WarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--second=a"]))
        .unwrap();
    assert_eq!(parser.get_warnings(), s(&["Option 'second' is deprecated"]));
}

// Java: OptionsParserTest.deprecationWarningForExpansionOption
#[test]
fn deprecation_warning_for_expansion_option() {
    let mut parser = parser_for(&[WarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--third"]))
        .unwrap();
    assert_eq!(parser.get_warnings(), s(&["Option 'third' is deprecated"]));
    assert!(parser.get_options::<WarningOptions>().unwrap().fourth);
}

// Java: OptionsParserTest.deprecationWarningForAbbreviatedExpansionOption
#[test]
fn deprecation_warning_for_abbreviated_expansion_option() {
    let mut parser = parser_for(&[WarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["-t"]))
        .unwrap();
    assert_eq!(parser.get_warnings(), s(&["Option 'third' is deprecated"]));
    assert!(parser.get_options::<WarningOptions>().unwrap().fourth);
}

options! {
    pub struct NewWarningOptions {
        #[option(name = "first", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", deprecation_warning = "it's gone")]
        pub first: Void,
        #[option(name = "second", allow_multiple = true, documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null",
                 deprecation_warning = "sorry, no replacement")]
        pub second: Vec<String>,
        #[option(name = "third", expansion = ["--fourth=true"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", deprecation_warning = "use --forth instead")]
        pub third: Void,
        #[option(name = "fourth", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub fourth: bool,
    }
}

// Java: OptionsParserTest.newDeprecationWarning
#[test]
fn new_deprecation_warning() {
    let mut parser = parser_for(&[NewWarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--first"]))
        .unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&["Option 'first' is deprecated: it's gone"])
    );
}

// Java: OptionsParserTest.newDeprecationWarningForListOption
#[test]
fn new_deprecation_warning_for_list_option() {
    let mut parser = parser_for(&[NewWarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--second=a"]))
        .unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&["Option 'second' is deprecated: sorry, no replacement"])
    );
}

// Java: OptionsParserTest.newDeprecationWarningForExpansionOption
#[test]
fn new_deprecation_warning_for_expansion_option() {
    let mut parser = parser_for(&[NewWarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::COMMAND_LINE, None, s(&["--third"]))
        .unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&["Option 'third' is deprecated: use --forth instead"])
    );
    assert!(parser.get_options::<NewWarningOptions>().unwrap().fourth);
}

options! {
    pub struct ExpansionWarningOptions {
        #[option(name = "first", expansion = ["--underlying=expandedFromFirst"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub first: Void,
        #[option(name = "second", expansion = ["--underlying=expandedFromSecond"],
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub second: Void,
        #[option(name = "underlying", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null")]
        pub underlying: Option<String>,
    }
}

// Java: OptionsParserTest.warningForExpansionOverridingExplicitOption
#[test]
fn warning_for_expansion_overriding_explicit_option() {
    let mut parser = parser_for(&[ExpansionWarningOptions::class()]);
    parser
        .parse(&["--underlying=underlying", "--first"])
        .unwrap();
    assert_eq!(
        parser.get_warnings(),
        s(&[
            "option '--first' was expanded and now overrides the explicit option \
             --underlying=underlying with --underlying=expandedFromFirst"
        ])
    );
}

// Java: OptionsParserTest.warningForTwoConflictingExpansionOptions
#[test]
fn warning_for_two_conflicting_expansion_options() {
    let mut parser = parser_for(&[ExpansionWarningOptions::class()]);
    parser.parse(&["--first", "--second"]).unwrap();
    assert!(
        parser.get_warnings().contains(
            &"option '--underlying' was expanded from both option '--first' and option '--second'"
                .to_string()
        )
    );
}

// Java: OptionsParserTest.noWarningForTwoConflictingExpansionOptionsFromRcFile
#[test]
fn no_warning_for_two_conflicting_expansion_options_from_rc_file() {
    let mut parser = parser_for(&[ExpansionWarningOptions::class()]);
    parser
        .parse_with_priority(PriorityCategory::RC_FILE, None, s(&["--first", "--second"]))
        .unwrap();
    assert!(
        !parser.get_warnings().contains(
            &"option '--underlying' was expanded from both option '--first' and option '--second'"
                .to_string()
        )
    );
}

// Java: OptionsParserTest.optionPrioritiesAreCorrectlyOrdered
#[test]
fn option_priorities_are_correctly_ordered() {
    assert_eq!(PriorityCategory::VALUES.len(), 6);
    assert!(PriorityCategory::DEFAULT < PriorityCategory::COMPUTED_DEFAULT);
    assert!(PriorityCategory::COMPUTED_DEFAULT < PriorityCategory::RC_FILE);
    assert!(PriorityCategory::RC_FILE < PriorityCategory::COMMAND_LINE);
    assert!(PriorityCategory::COMMAND_LINE < PriorityCategory::INVOCATION_POLICY);
    assert!(PriorityCategory::INVOCATION_POLICY < PriorityCategory::SOFTWARE_REQUIREMENT);
}

options! {
    pub struct IntrospectionExample {
        #[option(name = "alpha", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "alphaDefaultValue")]
        pub alpha: String,
        #[option(name = "beta", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "betaDefaultValue")]
        pub beta: String,
        #[option(name = "gamma", documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "gammaDefaultValue")]
        pub gamma: String,
        #[option(name = "delta", documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "deltaDefaultValue")]
        pub delta: String,
        #[option(name = "echo", metadata_tags = [HIDDEN], documentation_category = UNDOCUMENTED,
                 effect_tags = [NO_OP], default_value = "echoDefaultValue")]
        pub echo: String,
    }
}

// Java: OptionsParserTest.asListOfUnparsedOptions
#[test]
fn as_list_of_unparsed_options() {
    let mut parser = parser_for(&[IntrospectionExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("source"),
            s(&["--alpha=one", "--gamma=two", "--echo=three"]),
        )
        .unwrap();
    let result = parser.as_complete_list_of_parsed_options();
    assert_eq!(result.len(), 3);
    let expect = [
        ("alpha", true, false, "one"),
        ("gamma", false, false, "two"),
        ("echo", false, true, "three"),
    ];
    for (r, (name, documented, hidden, value)) in result.iter().zip(expect) {
        assert_eq!(r.get_option_definition().get_option_name(), name);
        assert_eq!(r.is_documented(), documented);
        assert_eq!(r.is_hidden(), hidden);
        assert_eq!(r.get_unconverted_value(), Some(value));
        assert_eq!(r.get_source(), Some("source"));
        assert_eq!(
            r.get_priority().get_priority_category(),
            PriorityCategory::COMMAND_LINE
        );
    }
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.asListOfExplicitOptions
#[test]
fn as_list_of_explicit_options() {
    let mut parser = parser_for(&[IntrospectionExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("source"),
            s(&["--alpha=one", "--gamma=two"]),
        )
        .unwrap();
    let result = parser.as_list_of_explicit_options();
    assert_eq!(result.len(), 2);
    let expect = [("alpha", true, "one"), ("gamma", false, "two")];
    for (r, (name, documented, value)) in result.iter().zip(expect) {
        assert_eq!(r.get_option_definition().get_option_name(), name);
        assert_eq!(r.is_documented(), documented);
        assert_eq!(r.get_unconverted_value(), Some(value));
        assert_eq!(r.get_source(), Some("source"));
        assert_eq!(
            r.get_priority().get_priority_category(),
            PriorityCategory::COMMAND_LINE
        );
    }
    assert!(parser.get_warnings().is_empty());
}

fn assert_option_value(
    expected_name: &str,
    expected_value: &str,
    actual: Option<&OptionValueDescription>,
) {
    let actual = actual.expect("value description");
    assert_eq!(
        actual.get_option_definition().get_option_name(),
        expected_name
    );
    assert_eq!(actual.get_value(), sv(expected_value));
}

fn assert_option_value_with_source(
    expected_name: &str,
    expected_value: &str,
    expected_priority: PriorityCategory,
    expected_source: &str,
    actual: Option<&OptionValueDescription>,
) {
    assert_option_value(expected_name, expected_value, actual);
    let actual = actual.unwrap();
    assert_eq!(actual.get_source_string().as_deref(), Some(expected_source));
    let instances = actual.get_canonical_instances().unwrap();
    assert!(!instances.is_empty());
    assert_eq!(
        instances[0].get_priority().get_priority_category(),
        expected_priority
    );
}

// Java: OptionsParserTest.asListOfEffectiveOptions
#[test]
fn as_list_of_effective_options() {
    let mut parser = parser_for(&[IntrospectionExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source"),
            s(&[
                "--alpha=alphaValueSetOnCommandLine",
                "--gamma=gammaValueSetOnCommandLine",
            ]),
        )
        .unwrap();
    let result = parser.as_list_of_option_values();
    assert_eq!(result.len(), 5);
    let map: HashMap<&str, OptionValueDescription> = result
        .into_iter()
        .map(|d| (d.get_option_definition().get_option_name(), d))
        .collect();
    assert_option_value_with_source(
        "alpha",
        "alphaValueSetOnCommandLine",
        PriorityCategory::COMMAND_LINE,
        "command line source",
        map.get("alpha"),
    );
    assert_option_value_with_source(
        "gamma",
        "gammaValueSetOnCommandLine",
        PriorityCategory::COMMAND_LINE,
        "command line source",
        map.get("gamma"),
    );
    assert_option_value("beta", "betaDefaultValue", map.get("beta"));
    assert_option_value("delta", "deltaDefaultValue", map.get("delta"));
    assert_option_value("echo", "echoDefaultValue", map.get("echo"));
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct ListExample {
        #[option(name = "alpha", converter = StringConverter, allow_multiple = true,
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null")]
        pub alpha: Vec<String>,
    }
}

// Java: OptionsParserTest.overrideListOptions
#[test]
fn override_list_options() {
    let mut parser = parser_for(&[ListExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source"),
            s(&["--alpha=cli"]),
        )
        .unwrap();
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("rc file origin"),
            s(&["--alpha=rc1", "--alpha=rc2"]),
        )
        .unwrap();
    assert_eq!(
        parser.get_options::<ListExample>().unwrap().alpha,
        s(&["rc1", "rc2", "cli"])
    );
}

// Java: OptionsParserTest.testDashDash
#[test]
fn test_dash_dash() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source"),
            s(&["--foo=woohoo", "residue", "--", "--bar=42"]),
        )
        .unwrap();
    assert_eq!(parser.get_residue(), s(&["residue", "--bar=42"]));
    assert_eq!(parser.get_pre_double_dash_residue(), s(&["residue"]));
}

// Java: OptionsParserTest.listOptionsHaveCorrectPriorities
#[test]
fn list_options_have_correct_priorities() {
    let mut parser = parser_for(&[ListExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source, part 1"),
            s(&["--alpha=cli1", "--alpha=cli2"]),
        )
        .unwrap();
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source, part 2"),
            s(&["--alpha=cli3", "--alpha=cli4"]),
        )
        .unwrap();
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("rc file origin"),
            s(&["--alpha=rc1", "--alpha=rc2"]),
        )
        .unwrap();
    let alpha_value = parser.get_option_value_description("alpha").unwrap();
    let parsed_options = alpha_value.get_canonical_instances().unwrap();
    assert_eq!(parsed_options.len(), 6);
    let expect = [
        ("rc file origin", "rc1"),
        ("rc file origin", "rc2"),
        ("command line source, part 1", "cli1"),
        ("command line source, part 1", "cli2"),
        ("command line source, part 2", "cli3"),
        ("command line source, part 2", "cli4"),
    ];
    for (p, (source, value)) in parsed_options.iter().zip(expect) {
        assert_eq!(p.get_source(), Some(source));
        assert_eq!(p.get_unconverted_value(), Some(value));
    }
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct CommaSeparatedOptionsExample {
        #[option(name = "alpha", converter = CommaSeparatedOptionListConverter,
                 allow_multiple = true, documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "null")]
        pub alpha: Vec<String>,
    }
}

// Java: OptionsParserTest.commaSeparatedOptionsWithAllowMultiple
#[test]
fn comma_separated_options_with_allow_multiple() {
    let mut parser = parser_for(&[CommaSeparatedOptionsExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source"),
            s(&["--alpha=one", "--alpha=two,three"]),
        )
        .unwrap();
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("rc file origin"),
            s(&["--alpha=rc1,rc2"]),
        )
        .unwrap();
    assert_eq!(
        parser
            .get_options::<CommaSeparatedOptionsExample>()
            .unwrap()
            .alpha,
        s(&["rc1", "rc2", "one", "two", "three"])
    );
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.commaSeparatedListOptionsHaveCorrectPriorities
#[test]
fn comma_separated_list_options_have_correct_priorities() {
    let mut parser = parser_for(&[CommaSeparatedOptionsExample::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::COMMAND_LINE,
            Some("command line source"),
            s(&["--alpha=one", "--alpha=two,three"]),
        )
        .unwrap();
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("rc file origin"),
            s(&["--alpha=rc1,rc2,rc3"]),
        )
        .unwrap();
    let alpha_value = parser.get_option_value_description("alpha").unwrap();
    let parsed_options = alpha_value.get_canonical_instances().unwrap();
    assert_eq!(parsed_options.len(), 3);
    let expect = [
        ("rc file origin", "rc1,rc2,rc3"),
        ("command line source", "one"),
        ("command line source", "two,three"),
    ];
    for (p, (source, value)) in parsed_options.iter().zip(expect) {
        assert_eq!(p.get_source(), Some(source));
        assert_eq!(p.get_unconverted_value(), Some(value));
    }
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct Yesterday {
        #[option(name = "a", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "a")]
        pub a: String,
        #[option(name = "b", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "b")]
        pub b: String,
        #[option(name = "c", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", expansion = ["--a=cExpansion"])]
        pub c: Void,
        #[option(name = "d", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", allow_multiple = true)]
        pub d: Vec<String>,
        #[option(name = "e", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", implicit_requirements = ["--a=eRequirement"])]
        pub e: Option<String>,
        #[option(name = "f", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", implicit_requirements = ["--b=fRequirement"])]
        pub f: Option<String>,
        #[option(name = "g", abbrev = 'h', documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "false")]
        pub g: bool,
    }
}

// Java: OptionsParserTest.canonicalize
fn canonicalize<O: OptionsClass>(args: &[&str]) -> Vec<String> {
    let mut parser = OptionsParser::builder()
        .options_classes(&[O::class()])
        .allow_residue(false)
        .build()
        .unwrap();
    parser.parse(args).unwrap();
    parser.canonicalize()
}

// Java: OptionsParserTest.canonicalizeEasy
#[test]
fn canonicalize_easy() {
    assert_eq!(canonicalize::<Yesterday>(&["--a=x"]), s(&["--a=x"]));
}

// Java: OptionsParserTest.canonicalizeSkipDuplicate
#[test]
fn canonicalize_skip_duplicate() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--a=y", "--a=x"]),
        s(&["--a=x"])
    );
}

// Java: OptionsParserTest.canonicalizeExpands
#[test]
fn canonicalize_expands() {
    assert_eq!(canonicalize::<Yesterday>(&["--c"]), s(&["--a=cExpansion"]));
}

// Java: OptionsParserTest.canonicalizeExpansionOverridesExplicit
#[test]
fn canonicalize_expansion_overrides_explicit() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--a=x", "--c"]),
        s(&["--a=cExpansion"])
    );
}

// Java: OptionsParserTest.canonicalizeExplicitOverridesExpansion
#[test]
fn canonicalize_explicit_overrides_expansion() {
    assert_eq!(canonicalize::<Yesterday>(&["--c", "--a=x"]), s(&["--a=x"]));
}

// Java: OptionsParserTest.canonicalizeDoesNotReorder
#[test]
fn canonicalize_does_not_reorder() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--b=y", "--d=x", "--a=z"]),
        s(&["--b=y", "--d=x", "--a=z"])
    );
}

// Java: OptionsParserTest.canonicalizeImplicitDepsNotListed
#[test]
fn canonicalize_implicit_deps_not_listed() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--a=x", "--e=y"]),
        s(&["--e=y"])
    );
}

// Java: OptionsParserTest.canonicalizeSkipsDuplicateAndStillOmitsImplicitDeps
#[test]
fn canonicalize_skips_duplicate_and_still_omits_implicit_deps() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--e=x", "--e=y"]),
        s(&["--e=y"])
    );
}

// Java: OptionsParserTest.implicitDepsAreNotInTheCanonicalOrderWhenTheyAreOverridden
#[test]
fn implicit_deps_are_not_in_the_canonical_order_when_they_are_overridden() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--e=y", "--a=x"]),
        s(&["--e=y", "--a=x"])
    );
}

// Java: OptionsParserTest.implicitDepsAreNotInTheCanonicalOrder
#[test]
fn implicit_deps_are_not_in_the_canonical_order() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--f=z", "--a=x"]),
        s(&["--f=z", "--a=x"])
    );
}

// Java: OptionsParserTest.canonicalizeDoesNotSkipAllowMultiple
#[test]
fn canonicalize_does_not_skip_allow_multiple() {
    assert_eq!(
        canonicalize::<Yesterday>(&["--d=a", "--d=b"]),
        s(&["--d=a", "--d=b"])
    );
}

// Java: OptionsParserTest.canonicalizeReplacesAbbrevWithName
#[test]
fn canonicalize_replaces_abbrev_with_name() {
    assert_eq!(canonicalize::<Yesterday>(&["-h"]), s(&["--g=1"]));
}

// Java: OptionsParserTest.canonicalizeNormalizesBooleanFlags
#[test]
fn canonicalize_normalizes_boolean_flags() {
    for f in ["--g", "--g=1", "--g=true", "--g=t", "--g=yes", "--g=y"] {
        assert_eq!(canonicalize::<Yesterday>(&[f]), s(&["--g=1"]), "{}", f);
    }
    for f in ["--nog", "--g=0", "--g=false", "--g=f", "--g=no", "--g=n"] {
        assert_eq!(canonicalize::<Yesterday>(&[f]), s(&["--g=0"]), "{}", f);
    }
}

options! {
    pub struct LongValueExample {
        #[option(name = "longval", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "2147483648")]
        pub longval: i64,
        #[option(name = "intval", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "2147483647")]
        pub intval: i32,
    }
}

// Java: OptionsParserTest.parseLong
#[test]
fn parse_long() {
    let mut parser = parser_for(&[LongValueExample::class()]);
    parser.parse(&[""]).unwrap();
    let result = parser.get_options::<LongValueExample>().unwrap();
    assert_eq!(result.longval, 2147483648i64);
    assert_eq!(result.intval, 2147483647);

    parser.parse(&["--longval", &i64::MIN.to_string()]).unwrap();
    assert_eq!(
        parser.get_options::<LongValueExample>().unwrap().longval,
        i64::MIN
    );

    parser.parse(&["--longval", "100"]).unwrap();
    assert_eq!(
        parser.get_options::<LongValueExample>().unwrap().longval,
        100
    );
}

// Java: OptionsParserTest.intOutOfBounds
#[test]
fn int_out_of_bounds() {
    let mut parser = parser_for(&[LongValueExample::class()]);
    let e = parser.parse(&["--intval=2147483648"]).unwrap_err();
    assert!(e.get_message().contains("'2147483648' is not an int"));
}

options! {
    pub struct OldNameExample {
        #[option(name = "new_name", old_name = "old_name", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "defaultValue")]
        pub flag: String,
        #[option(name = "new_boolean_name", old_name = "old_boolean_name",
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub boolean_flag: bool,
    }
}

fn assert_no_duplicates(v: &[String]) {
    let mut set = std::collections::HashSet::new();
    for x in v {
        assert!(set.insert(x), "duplicate {}", x);
    }
}

fn old_name_parser() -> OptionsParser {
    parser_for(&[OldNameExample::class()])
}

const OLD_BOOLEAN_WARNING: &str =
    "Option 'old_boolean_name' is deprecated: Use --new_boolean_name instead";

// Java: OptionsParserTest.testOldName
#[test]
fn test_old_name() {
    let mut parser = old_name_parser();
    parser.parse(&["--old_name=foo"]).unwrap();
    assert_eq!(parser.get_options::<OldNameExample>().unwrap().flag, "foo");
    assert!(
        parser
            .get_warnings()
            .contains(&"Option 'old_name' is deprecated: Use --new_name instead".to_string())
    );
    assert_no_duplicates(&parser.get_warnings());

    let mut parser = old_name_parser();
    parser.parse(&["--new_name=foo"]).unwrap();
    assert_eq!(parser.get_options::<OldNameExample>().unwrap().flag, "foo");
    assert!(parser.get_warnings().is_empty());
}

// Java: OptionsParserTest.testOldName_repeatedFlag
#[test]
fn test_old_name_repeated_flag() {
    let mut parser = old_name_parser();
    parser.parse(&["--old_name=foo", "--old_name=bar"]).unwrap();
    assert_eq!(parser.get_options::<OldNameExample>().unwrap().flag, "bar");
    assert!(
        parser
            .get_warnings()
            .contains(&"Option 'old_name' is deprecated: Use --new_name instead".to_string())
    );
    assert_no_duplicates(&parser.get_warnings());
}

// Java: OptionsParserTest.testOldNameCanonicalization
#[test]
fn test_old_name_canonicalization() {
    assert_eq!(
        canonicalize::<OldNameExample>(&["--old_name=foo"]),
        s(&["--new_name=foo"])
    );
}

fn check_old_boolean(old_args: &[&str], new_args: &[&str], expected: bool) {
    let mut parser = old_name_parser();
    parser.parse(old_args).unwrap();
    assert_eq!(
        parser.get_options::<OldNameExample>().unwrap().boolean_flag,
        expected
    );
    assert!(
        parser
            .get_warnings()
            .contains(&OLD_BOOLEAN_WARNING.to_string())
    );
    assert_no_duplicates(&parser.get_warnings());

    if !new_args.is_empty() {
        let mut parser = old_name_parser();
        parser.parse(new_args).unwrap();
        assert_eq!(
            parser.get_options::<OldNameExample>().unwrap().boolean_flag,
            expected
        );
        assert!(parser.get_warnings().is_empty());
    }
}

// Java: OptionsParserTest.testOldName_booleanTrue
#[test]
fn test_old_name_boolean_true() {
    check_old_boolean(
        &["--old_boolean_name=true"],
        &["--new_boolean_name=true"],
        true,
    );
}

// Java: OptionsParserTest.testOldName_booleanFalse
#[test]
fn test_old_name_boolean_false() {
    check_old_boolean(
        &["--old_boolean_name=false"],
        &["--new_boolean_name=false"],
        false,
    );
}

// Java: OptionsParserTest.testOldName_specialBooleanSyntax
#[test]
fn test_old_name_special_boolean_syntax() {
    check_old_boolean(&["--old_boolean_name"], &["--new_boolean_name"], true);
}

// Java: OptionsParserTest.testOldName_negatedSpecialBooleanSyntax
#[test]
fn test_old_name_negated_special_boolean_syntax() {
    check_old_boolean(&["--noold_boolean_name"], &["--nonew_boolean_name"], false);
}

// Java: OptionsParserTest.testOldName_repeatedBooleanFlag
#[test]
fn test_old_name_repeated_boolean_flag() {
    check_old_boolean(
        &["--old_boolean_name=false", "--old_boolean_name"],
        &[],
        true,
    );
}

// Java: OptionsParserTest.testOldName_overriddenByNewName
#[test]
fn test_old_name_overridden_by_new_name() {
    check_old_boolean(
        &["--old_boolean_name=false", "--new_boolean_name"],
        &[],
        true,
    );
}

options! {
    pub struct OldNameNoWarningExample {
        #[option(name = "new_name", old_name = "old_name", old_name_warning = false,
                 documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "defaultValue")]
        pub flag: String,
    }
}

// Java: OptionsParserTest.testOldName_noWarning
#[test]
fn test_old_name_no_warning() {
    let mut parser = parser_for(&[OldNameNoWarningExample::class()]);
    parser.parse(&["--old_name=foo"]).unwrap();
    assert_eq!(
        parser
            .get_options::<OldNameNoWarningExample>()
            .unwrap()
            .flag,
        "foo"
    );
    assert!(parser.get_warnings().is_empty());
}

options! {
    pub struct ExampleBooleanFooOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub foo: bool,
    }
}

// Java: OptionsParserTest.testBooleanUnderscorePrefixError
#[test]
fn test_boolean_underscore_prefix_error() {
    let mut parser = parser_for(&[ExampleBooleanFooOptions::class()]);
    let e = parser.parse(&["--no_foo"]).unwrap_err();
    assert!(e.get_message().contains("Unrecognized option: --no_foo"));
}

options! {
    #[uses_only_core_types]
    /// Dummy options that declares it uses only core types.
    pub struct CoreTypesOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub foo: bool,
        #[option(name = "bar", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "abc")]
        pub bar: String,
    }
}

options! {
    /// Dummy options that does not declare using only core types.
    pub struct NonCoreTypesOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub foo: bool,
    }
}

/// Dummy unsafe type (Java: BadCoreTypesOptions.Foo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foo {
    pub i: i32,
}

impl OptionScalar for Foo {
    fn java_to_string(&self) -> String {
        format!("Foo@{}", self.i)
    }
}
option_field_type!(Foo, "OptionsParserTest$BadCoreTypesOptions$Foo");

/// Converter for Foo.
#[derive(Default)]
pub struct FooConverter;

impl Converter for FooConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        Ok(Some(Value::new(Foo {
            i: input.unwrap().parse().unwrap(),
        })))
    }
    fn get_type_description(&self) -> String {
        "a foo".into()
    }
}

options! {
    #[uses_only_core_types]
    /// Dummy options that incorrectly claims to use only core types.
    pub struct BadCoreTypesOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", converter = FooConverter)]
        pub foo: Option<Foo>,
    }
}

options! {
    #[uses_only_core_types]
    /// InheritedBadCoreTypesOptions extends SuperBadCoreTypesOptions (flattened).
    pub struct InheritedBadCoreTypesOptions {
        #[option(name = "foo", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "null", converter = FooConverter)]
        pub foo: Option<Foo>,
        #[option(name = "bar", documentation_category = UNCATEGORIZED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub bar: bool,
    }
}

// Java: OptionsParserTest.testUsesOnlyCoreTypes
#[test]
fn test_uses_only_core_types() {
    assert!(OptionsParser::get_uses_only_core_types(CoreTypesOptions::class()).unwrap());
    assert!(!OptionsParser::get_uses_only_core_types(NonCoreTypesOptions::class()).unwrap());
}

fn check_core_types_message(message: &str, class: &str) {
    // Java regex: "Options class '.*<class>' is marked as @UsesOnlyCoreTypes, but field 'foo' has
    // type '.*Foo'"
    let prefix = "Options class '";
    assert!(message.starts_with(prefix), "{}", message);
    let marker = format!(
        "{}' is marked as @UsesOnlyCoreTypes, but field 'foo' has type '",
        class
    );
    let pos = message
        .find(&marker)
        .unwrap_or_else(|| panic!("{}", message));
    assert!(message.ends_with("Foo'"), "{}", message);
    assert!(pos >= prefix.len());
}

// Java: OptionsParserTest.testValidationOfUsesOnlyCoreTypes
#[test]
fn test_validation_of_uses_only_core_types() {
    let expected = OptionsParser::get_uses_only_core_types(BadCoreTypesOptions::class())
        .expect_err("Should have detected illegal use of @UsesOnlyCoreTypes");
    check_core_types_message(expected.get_message(), "BadCoreTypesOptions");
}

// Java: OptionsParserTest.testValidationOfUsesOnlyCoreTypes_Inherited
#[test]
fn test_validation_of_uses_only_core_types_inherited() {
    let expected = OptionsParser::get_uses_only_core_types(InheritedBadCoreTypesOptions::class())
        .expect_err("Should have detected illegal use of @UsesOnlyCoreTypes");
    check_core_types_message(expected.get_message(), "InheritedBadCoreTypesOptions");
}

options! {
    /// Dummy options for testing getHelpCompletion() and visitOptions().
    pub struct CompletionOptions {
        #[option(name = "secret", documentation_category = UNDOCUMENTED, effect_tags = [NO_OP],
                 default_value = "false")]
        pub secret: bool,
        #[option(name = "b", documentation_category = LOGGING, effect_tags = [NO_OP],
                 default_value = "false")]
        pub b: bool,
        #[option(name = "a", documentation_category = QUERY, effect_tags = [NO_OP],
                 default_value = "false")]
        pub a: bool,
    }
}

// Java: OptionsParserTest.getOptionsCompletionShouldFilterUndocumentedOptions
#[test]
#[ignore = "needs OptionsUsage.getCompletion (OptionsParser.getOptionsCompletion), not ported"]
fn get_options_completion_should_filter_undocumented_options() {
    let _ = parser_for(&[CompletionOptions::class()]);
}

// Java: OptionsParserTest.visitOptionsShouldFailWithoutPredicate
#[test]
#[ignore = "null predicate: not representable in Rust (visit_options takes a non-null closure)"]
fn visit_options_should_fail_without_predicate() {}

// Java: OptionsParserTest.visitOptionsShouldFailWithoutVisitor
#[test]
#[ignore = "null visitor: not representable in Rust (visit_options takes a non-null closure)"]
fn visit_options_should_fail_without_visitor() {}

fn visit_options_to_collect_their_names(
    predicate: impl Fn(&'static OptionDefinition) -> bool,
) -> Vec<String> {
    let mut names = Vec::new();
    let parser = parser_for(&[CompletionOptions::class()]);
    parser.visit_options(predicate, |option| {
        names.push(option.get_option_name().to_string())
    });
    names
}

// Java: OptionsParserTest.visitOptionsShouldReturnAllOptionsInOrder
#[test]
fn visit_options_should_return_all_options_in_order() {
    assert_eq!(
        visit_options_to_collect_their_names(|_| true),
        s(&["a", "b", "secret"])
    );
}

// Java: OptionsParserTest.visitOptionsShouldObeyPredicate
#[test]
fn visit_options_should_obey_predicate() {
    assert!(visit_options_to_collect_their_names(|_| false).is_empty());
    assert_eq!(
        visit_options_to_collect_their_names(|option| option.get_option_name().len() > 1),
        s(&["secret"])
    );
}

fn create_invocation_policy_origin(
    implicit_dependent: Option<ParsedOptionRef>,
    expanded_from: Option<ParsedOptionRef>,
) -> OptionInstanceOrigin {
    OptionInstanceOrigin::new(
        OptionPriority::lowest_option_priority_at_category(PriorityCategory::INVOCATION_POLICY),
        Some("invocation policy".to_string()),
        implicit_dependent,
        expanded_from,
    )
}

fn command_line_forms(parser: &OptionsParser) -> Vec<String> {
    parser
        .as_complete_list_of_parsed_options()
        .iter()
        .map(|p| p.get_command_line_form().unwrap().to_string())
        .collect()
}

// Java: OptionsParserTest.setOptionValueAtSpecificPriorityWithoutExpansion_setsOptionAndAddsParsedValue
#[test]
fn set_option_value_at_specific_priority_without_expansion_sets_option_and_adds_parsed_value() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    let origin = create_invocation_policy_origin(None, None);
    let option_definition =
        OptionsParser::get_option_definition_by_name(ExampleFoo::class(), "foo");
    parser
        .set_option_value_at_specific_priority_without_expansion(origin, option_definition, "hello")
        .unwrap();
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().foo, "hello");
    assert_eq!(command_line_forms(&parser), s(&["--foo=hello"]));
}

// Java: OptionsParserTest.setOptionValueAtSpecificPriorityWithoutExpansion_addsFlagAlias
#[test]
fn set_option_value_at_specific_priority_without_expansion_adds_flag_alias() {
    let mut parser = OptionsParser::builder()
        .with_alias_flag(Some("foo"))
        .options_classes(&[ExampleFoo::class()])
        .build()
        .unwrap();
    let origin = create_invocation_policy_origin(None, None);
    let option_definition =
        OptionsParser::get_option_definition_by_name(ExampleFoo::class(), "foo");
    parser
        .set_option_value_at_specific_priority_without_expansion(
            origin,
            option_definition,
            "hi=bar",
        )
        .unwrap();
    parser.parse(&["--hi=123"]).unwrap();
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().foo, "hi=bar");
    assert_eq!(parser.get_options::<ExampleFoo>().unwrap().bar, 123);
    assert_eq!(
        command_line_forms(&parser),
        s(&["--bar=123", "--foo=hi=bar"])
    );
}

// Java: OptionsParserTest.setOptionValueAtSpecificPriorityWithoutExpansion_implicitReqs_setsTopFlagOnly
#[test]
fn set_option_value_at_specific_priority_without_expansion_implicit_reqs_sets_top_flag_only() {
    let mut parser = parser_for(&[ImplicitDependencyOptions::class()]);
    let origin = create_invocation_policy_origin(None, None);
    let option_definition =
        OptionsParser::get_option_definition_by_name(ImplicitDependencyOptions::class(), "first");
    parser
        .set_option_value_at_specific_priority_without_expansion(origin, option_definition, "hello")
        .unwrap();
    let options = parser.get_options::<ImplicitDependencyOptions>().unwrap();
    assert_eq!(options.first.as_deref(), Some("hello"));
    assert_eq!(options.second, None);
    assert_eq!(options.third, None);
    assert_eq!(command_line_forms(&parser), s(&["--first=hello"]));
}

// Java: OptionsParserTest.setOptionValueAtSpecificPriorityWithoutExpansion_impliedFlag_setsValueSkipsParsed
#[test]
fn set_option_value_at_specific_priority_without_expansion_implied_flag_sets_value_skips_parsed() {
    let mut parser = parser_for(&[ImplicitDependencyOptions::class()]);
    let first = ParsedOptionDescription::new_dummy_instance(
        OptionsParser::get_option_definition_by_name(ImplicitDependencyOptions::class(), "first"),
        create_invocation_policy_origin(None, None),
        None,
    );
    let origin = create_invocation_policy_origin(Some(first), None);
    let option_definition =
        OptionsParser::get_option_definition_by_name(ImplicitDependencyOptions::class(), "second");
    parser
        .set_option_value_at_specific_priority_without_expansion(origin, option_definition, "hello")
        .unwrap();
    let options = parser.get_options::<ImplicitDependencyOptions>().unwrap();
    assert_eq!(options.second.as_deref(), Some("hello"));
    assert_eq!(options.third, None);
    assert!(command_line_forms(&parser).is_empty());
}

// Java: OptionsParserTest.setOptionValueAtSpecificPriorityWithoutExpansion_expandedFlag_setsValueAndParsed
#[test]
fn set_option_value_at_specific_priority_without_expansion_expanded_flag_sets_value_and_parsed() {
    let mut parser = parser_for(&[ImplicitDependencyOptions::class()]);
    let first = ParsedOptionDescription::new_dummy_instance(
        OptionsParser::get_option_definition_by_name(ImplicitDependencyOptions::class(), "first"),
        create_invocation_policy_origin(None, None),
        None,
    );
    let origin = create_invocation_policy_origin(None, Some(first));
    let option_definition =
        OptionsParser::get_option_definition_by_name(ImplicitDependencyOptions::class(), "second");
    parser
        .set_option_value_at_specific_priority_without_expansion(origin, option_definition, "hello")
        .unwrap();
    let options = parser.get_options::<ImplicitDependencyOptions>().unwrap();
    assert_eq!(options.second.as_deref(), Some("hello"));
    assert_eq!(options.third, None);
    assert_eq!(command_line_forms(&parser), s(&["--second=hello"]));
}

// Java: OptionsParserTest.negativeTargetPatternsInOptions_failsDistinctively
#[test]
fn negative_target_patterns_in_options_fails_distinctively() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    let e = parser.parse(&["//foo", "-//bar", "//baz"]).unwrap_err();
    assert!(e.get_message().contains("-//bar"));
    assert!(
        e.get_message()
            .contains("Negative target patterns can only appear after the end of options marker")
    );
    assert!(
        e.get_message().contains(
            "Flags corresponding to Starlark-defined build settings always start with '--'"
        )
    );
}

// Java: OptionsParserTest.negativeExternalTargetPatternsInOptions_failsDistinctively
#[test]
fn negative_external_target_patterns_in_options_fails_distinctively() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    let e = parser
        .parse(&["//foo", "-@repo//bar", "//baz"])
        .unwrap_err();
    assert!(e.get_message().contains("-@repo//bar"));
    assert!(
        e.get_message()
            .contains("Negative target patterns can only appear after the end of options marker")
    );
    assert!(
        e.get_message().contains(
            "Flags corresponding to Starlark-defined build settings always start with '--'"
        )
    );
}

// Java: OptionsParserTest.fallbackOptions_optionsParsingEquivalently
#[test]
fn fallback_options_options_parsing_equivalently() {
    let fallback_data = OptionsParser::get_fallback_options_data(&[
        ExampleFoo::class(),
        ExampleEquivalentWithFoo::class(),
    ])
    .unwrap();
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some(".bazelrc".to_string()),
            s(&[
                "--ignored_with_value",
                "--foo",
                "--foo=bar",
                "--ignored_without_value",
                "--bar",
                "1",
            ]),
            Some(fallback_data),
        )
        .unwrap();
    let foo = parser.get_options::<ExampleFoo>().unwrap();
    assert_eq!(foo.foo, "bar");
    assert_eq!(foo.bar, 1);
    assert!(parser.get_options::<ExampleEquivalentWithFoo>().is_none());
}

// Java: OptionsParserTest.fallbackOptions_optionsParsingDifferently
#[test]
fn fallback_options_options_parsing_differently() {
    let e = OptionsParser::get_fallback_options_data(&[
        ExampleFoo::class(),
        ExampleIncompatibleWithFoo::class(),
    ])
    .err()
    .expect("ConstructionException");
    assert!(e.get_message().starts_with(DUPLICATE_EXCEPTION));
}

options! {
    pub struct ExpandingOptions {
        #[option(name = "foo", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], expansion = ["--nobar"], default_value = "null")]
        pub foo: Void,
    }
}

options! {
    pub struct ExpandingOptionsFallback {
        #[option(name = "bar", category = "one", documentation_category = UNCATEGORIZED,
                 effect_tags = [NO_OP], default_value = "true")]
        pub bar: bool,
    }
}

// Java: OptionsParserTest.fallbackOptions_expansionToNegativeBooleanFlag
#[test]
fn fallback_options_expansion_to_negative_boolean_flag() {
    let fallback_data = OptionsParser::get_fallback_options_data(&[
        ExpandingOptions::class(),
        ExpandingOptionsFallback::class(),
    ])
    .unwrap();
    let mut parser = parser_for(&[ExpandingOptions::class()]);
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some(".bazelrc".to_string()),
            s(&["--foo"]),
            Some(fallback_data),
        )
        .unwrap();
    assert!(parser.get_options::<ExpandingOptions>().is_some());
    assert!(parser.get_options::<ExpandingOptionsFallback>().is_none());
}

fn user_option_keys(parser: &OptionsParser) -> Vec<String> {
    sorted(parser.get_user_options().keys().cloned().collect())
}

// Java: OptionsParserTest.testOptionsParser_getUserOptions_excludesClientOptions
#[test]
fn test_options_parser_get_user_options_excludes_client_options() {
    let mut parser = parser_for(&[ExpandingOptions::class(), ExpandingOptionsFallback::class()]);
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some("client".to_string()),
            s(&["--foo"]),
            None,
        )
        .unwrap();
    assert!(parser.get_user_options().is_empty());
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some(".bazelrc".to_string()),
            s(&["--foo"]),
            None,
        )
        .unwrap();
    assert_eq!(user_option_keys(&parser), sorted(s(&["--foo", "--nobar"])));
}

// Java: OptionsParserTest.testOptionsParser_explicitOptions_excludesFlagsetOptions
#[test]
fn test_options_parser_explicit_options_excludes_flagset_options() {
    let mut parser = parser_for(&[ExampleFoo::class()]);
    parser
        .parse_with_priority(
            PriorityCategory::RC_FILE,
            Some("//test:PROJECT.scl"),
            s(&["--foo=set_by_flagset"]),
        )
        .unwrap();
    assert!(parser.as_list_of_explicit_options().is_empty());
    assert!(
        parser
            .canonicalize()
            .contains(&"--foo=set_by_flagset".to_string())
    );
}

// Java: OptionsParserTest.testOptionsParser_getUserOptions_excludesInvocationPolicy
#[test]
fn test_options_parser_get_user_options_excludes_invocation_policy() {
    let mut parser = parser_for(&[ExpandingOptions::class(), ExpandingOptionsFallback::class()]);
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some("Invocation policy".to_string()),
            s(&["--foo"]),
            None,
        )
        .unwrap();
    assert!(parser.get_user_options().is_empty());
    parser
        .parse_with_source_function(
            PriorityCategory::RC_FILE,
            &|_| Some(".bazelrc".to_string()),
            s(&["--foo"]),
            None,
        )
        .unwrap();
    assert_eq!(user_option_keys(&parser), sorted(s(&["--foo", "--nobar"])));
}

fn alias_parser(class: &'static blaze_options::OptionsClassInfo) -> OptionsParser {
    OptionsParser::builder()
        .with_alias_flag(Some("flag_alias"))
        .options_classes(&[class])
        .build()
        .unwrap()
}

// Java: OptionsParserTest.aliasWithNoPrefix_emitsWarningIfNative
#[test]
fn alias_with_no_prefix_emits_warning_if_native() {
    let mut parser = alias_parser(BooleanAliasOptions::class());
    parser.parse(&["--flag_alias=foo=bar"]).unwrap();
    parser.parse(&["--nofoo"]).unwrap();
    let options = parser.get_options::<BooleanAliasOptions>().unwrap();
    assert!(options.bar);
    assert!(!options.foo);
    assert!(
        parser
            .get_warnings()
            .contains(&"Flag --nofoo is deprecated. Use --foo=false instead.".to_string())
    );
}

// Java: OptionsParserTest.aliasWithNoPrefix_failsIfNotNative
#[test]
fn alias_with_no_prefix_fails_if_not_native() {
    let mut parser = alias_parser(BooleanAliasOptions::class());
    parser.parse(&["--flag_alias=baz=bar"]).unwrap();
    assert!(parser.parse(&["--nobaz"]).is_err());
}

// Java: OptionsParserTest.aliasWithNoPrefixAndCustomWarning_emitsCustomWarning
#[test]
fn alias_with_no_prefix_and_custom_warning_emits_custom_warning() {
    let mut parser = alias_parser(DeprecatedAliasOptions::class());
    parser.parse(&["--flag_alias=foo=bar"]).unwrap();
    parser.parse(&["--nofoo"]).unwrap();
    assert!(
        parser
            .get_warnings()
            .contains(&"Option 'foo' is deprecated: Don't use foo.".to_string())
    );
}

// Java: OptionsParserTest.aliasWithNoPrefix_emitsCustomWarningIfAvailable
#[test]
fn alias_with_no_prefix_emits_custom_warning_if_available() {
    let mut parser = alias_parser(DeprecatedAliasOptions::class());
    parser.parse(&["--flag_alias=foo=bar"]).unwrap();
    parser.parse(&["--nofoo"]).unwrap();
    let options = parser.get_options::<DeprecatedAliasOptions>().unwrap();
    assert!(options.bar);
    assert!(!options.foo);
    let warnings = parser.get_warnings();
    assert!(warnings.contains(&"Option 'foo' is deprecated: Don't use foo.".to_string()));
    assert!(
        !warnings.contains(&"Flag --nofoo is deprecated. Use --foo=false instead.".to_string())
    );
}

// Java: OptionsParserTest.deletedNativeFlagAliasedToStarlarkFlag_readStarlarkValue
#[test]
fn deleted_native_flag_aliased_to_starlark_flag_read_starlark_value() {
    let mut first_pass_parser = OptionsParser::builder()
        .options_classes(&[ExampleFoo::class(), ExampleAliasOptions::class()])
        .is_first_round_of_parsing(true)
        .skip_starlark_option_prefixes()
        .with_alias_flag(Some("flag_alias"))
        .build()
        .unwrap();
    let args = s(&[
        "--deleted_native_flag=123",
        "--flag_alias=deleted_native_flag=//my:starlark_flag",
    ]);
    first_pass_parser.parse_list(args.clone()).unwrap();
    let mut second_pass_parser = first_pass_parser.to_builder().build().unwrap();
    second_pass_parser.parse_list(args).unwrap();
    assert!(
        second_pass_parser
            .get_skipped_args()
            .contains(&"--//my:starlark_flag=123".to_string())
    );
}
