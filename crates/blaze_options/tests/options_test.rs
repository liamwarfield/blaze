//! Port of src/test/java/com/google/devtools/common/options/OptionsTest.java.

use blaze_options::{
    ConversionContext, Converter, OptionScalar, Options, OptionsBase, OptionsParsingException,
    TriState, Value, Void, option_field_type, options,
};

const NO_ARGS: &[&str] = &[];

options! {
    pub struct HttpOptions {
        #[option(
            name = "host",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "www.google.com",
            help = "The URL at which the server will be running.",
        )]
        pub host: String,

        #[option(
            name = "port",
            abbrev = 'p',
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "80",
            help = "The port at which the server will be running.",
        )]
        pub port: i32,

        #[option(
            name = "debug",
            abbrev = 'd',
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
            help = "debug",
        )]
        pub is_debugging: bool,

        #[option(
            name = "tristate",
            abbrev = 't',
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "auto",
            help = "tri-state option returning auto by default",
        )]
        pub tri_state: TriState,

        #[option(
            name = "special",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = ["--host=special.google.com", "--port=8080"],
        )]
        pub special: Void,
    }
}

fn parse(args: &[&str]) -> Result<Options<HttpOptions>, OptionsParsingException> {
    Options::<HttpOptions>::parse(args)
}

// Java: OptionsTest.paragraphFill
#[test]
fn paragraph_fill() {
    use blaze_options::options_usage::OptionsUsage;
    // TODO(bazel-team): don't include trailing space after last word in line.
    let input = "The quick brown fox jumps over the lazy dog.";

    assert_eq!(
        OptionsUsage::paragraph_fill(input, 2, 13),
        "  The quick \n  brown fox \n  jumps over \n  the lazy \n  dog."
    );
    assert_eq!(
        OptionsUsage::paragraph_fill(input, 3, 19),
        "   The quick brown \n   fox jumps over \n   the lazy dog."
    );

    let input2 = "The quick brown fox jumps\nAnother paragraph.";
    assert_eq!(
        OptionsUsage::paragraph_fill(input2, 2, 23),
        "  The quick brown fox \n  jumps\n  Another paragraph."
    );
}

// Java: OptionsTest.getsDefaults
#[test]
fn gets_defaults() {
    let options = parse(NO_ARGS).unwrap();
    let web_flags = options.get_options();
    assert_eq!(web_flags.host, "www.google.com");
    assert_eq!(web_flags.port, 80);
    assert!(!web_flags.is_debugging);
    assert_eq!(web_flags.tri_state, TriState::AUTO);
    assert!(options.get_remaining_args().is_empty());
}

// Java: OptionsTest.objectMethods
#[test]
fn object_methods() {
    let args = ["--host", "foo", "--port", "80"];
    let left = parse(&args).unwrap().into_options();
    let like_left = parse(&args).unwrap().into_options();
    let right = parse(&["--host", "other", "--port", "90"])
        .unwrap()
        .into_options();

    let to_string = (&left as &dyn OptionsBase).java_to_string();
    // PORT: the class name is the Rust type name, not
    // "com.google.devtools.common.options.OptionsTest$HttpOptions".
    assert!(to_string.starts_with("HttpOptions{"), "{}", to_string);
    assert!(to_string.contains("host=foo"));
    assert!(to_string.contains("port=80"));
    assert!(to_string.ends_with('}'));

    assert_eq!(left, left);
    assert_eq!(
        (&left as &dyn OptionsBase).java_to_string(),
        (&like_left as &dyn OptionsBase).java_to_string()
    );
    assert_eq!(left, like_left);
    assert_eq!(like_left, left);
    assert_ne!(left, right);
    assert_ne!(right, left);
    // PORT: hashCode assertions dropped; options structs do not implement Hash. The cache key
    // (OptionsBase.cacheKey) plays that role.
    assert_eq!(
        (&like_left as &dyn OptionsBase).cache_key(),
        (&left as &dyn OptionsBase).cache_key()
    );
    assert_ne!(
        (&left as &dyn OptionsBase).cache_key(),
        (&right as &dyn OptionsBase).cache_key()
    );
}

// Java: OptionsTest.equals
#[test]
fn equals() {
    let options1 = parse(&["--host", "foo", "--port", "80"])
        .unwrap()
        .into_options();
    let options2 = parse(&["-p", "80", "--host", "foo"])
        .unwrap()
        .into_options();
    // Order/abbreviations shouldn't matter.
    assert_eq!(options1, options2);

    // Explicitly setting a default shouldn't matter.
    assert_eq!(
        parse(&["--port", "80"]).unwrap().into_options(),
        parse(&[]).unwrap().into_options()
    );
    assert_ne!(
        parse(&["--port", "3"]).unwrap().into_options(),
        parse(&[]).unwrap().into_options()
    );
}

// Java: OptionsTest.getsFlagsProvidedInArguments
#[test]
fn gets_flags_provided_in_arguments() {
    let options = parse(&["--host", "google.com", "-p", "8080", "--debug"]).unwrap();
    let web_flags = options.get_options();
    assert_eq!(web_flags.host, "google.com");
    assert_eq!(web_flags.port, 8080);
    assert!(web_flags.is_debugging);
    assert!(options.get_remaining_args().is_empty());
}

// Java: OptionsTest.getsFlagsProvidedWithEquals
#[test]
fn gets_flags_provided_with_equals() {
    let options = parse(&["--host=google.com", "--port=8080", "--debug"]).unwrap();
    let web_flags = options.get_options();
    assert_eq!(web_flags.host, "google.com");
    assert_eq!(web_flags.port, 8080);
    assert!(web_flags.is_debugging);
    assert!(options.get_remaining_args().is_empty());
}

// Java: OptionsTest.booleanNo
#[test]
fn boolean_no() {
    let options = parse(&["--nodebug", "--notristate"]).unwrap();
    assert!(!options.get_options().is_debugging);
    assert_eq!(options.get_options().tri_state, TriState::NO);
}

// Java: OptionsTest.booleanAbbrevMinus
#[test]
fn boolean_abbrev_minus() {
    let options = parse(&["-d-", "-t-"]).unwrap();
    assert!(!options.get_options().is_debugging);
    assert_eq!(options.get_options().tri_state, TriState::NO);
}

// Java: OptionsTest.boolean0
#[test]
fn boolean0() {
    let options = parse(&["--debug=0", "--tristate=0"]).unwrap();
    assert!(!options.get_options().is_debugging);
    assert_eq!(options.get_options().tri_state, TriState::NO);
}

// Java: OptionsTest.boolean1
#[test]
fn boolean1() {
    let options = parse(&["--debug=1", "--tristate=1"]).unwrap();
    assert!(options.get_options().is_debugging);
    assert_eq!(options.get_options().tri_state, TriState::YES);
}

// Java: OptionsTest.retainsStuffThatsNotOptions
#[test]
fn retains_stuff_thats_not_options() {
    let args = ["these", "aint", "options"];
    let options = parse(&args).unwrap();
    assert_eq!(options.get_remaining_args(), args);
}

// Java: OptionsTest.retainsStuffThatsNotComplexOptions
#[test]
fn retains_stuff_thats_not_complex_options() {
    let args = [
        "--host",
        "google.com",
        "notta",
        "--port=8080",
        "option",
        "--debug=true",
    ];
    let options = parse(&args).unwrap();
    assert_eq!(options.get_remaining_args(), ["notta", "option"]);
}

// Java: OptionsTest.wontParseUnknownOptions
#[test]
fn wont_parse_unknown_options() {
    let e = parse(&["--unknown", "--other=23", "--options"]).unwrap_err();
    assert_eq!(e.get_message(), "Unrecognized option: --unknown");
}

// Java: OptionsTest.requiresOptionValue
#[test]
fn requires_option_value() {
    let e = parse(&["--port"]).unwrap_err();
    assert_eq!(e.get_message(), "Expected value after --port");
}

// Java: OptionsTest.handlesDuplicateOptions_full
#[test]
fn handles_duplicate_options_full() {
    let options = parse(&["--port=80", "--port", "81"]).unwrap();
    assert_eq!(options.get_options().port, 81);
}

// Java: OptionsTest.handlesDuplicateOptions_abbrev
#[test]
fn handles_duplicate_options_abbrev() {
    let options = parse(&["--port=80", "-p", "81"]).unwrap();
    assert_eq!(options.get_options().port, 81);
}

// Java: OptionsTest.duplicateOptionsOkWithSameValues
#[test]
fn duplicate_options_ok_with_same_values() {
    parse(&["--port=80", "--port", "80"]).unwrap();
    parse(&["--port=80", "-p", "80"]).unwrap();
}

// Java: OptionsTest.isPickyAboutBooleanValues
#[test]
fn is_picky_about_boolean_values() {
    let e = parse(&["--debug=not_a_boolean"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "While parsing option --debug=not_a_boolean: 'not_a_boolean' is not a boolean"
    );
}

// Java: OptionsTest.isPickyAboutBooleanNos
#[test]
fn is_picky_about_boolean_nos() {
    let e = parse(&["--nodebug=1"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unexpected value after boolean option: --nodebug=1"
    );
}

// Java: OptionsTest.usageForBuiltinTypesNoExpansion
#[test]
fn usage_for_builtin_types_no_expansion() {
    let usage = Options::<HttpOptions>::get_usage();
    // We can't rely on the option ordering.
    assert!(usage.contains("  --[no]debug [-d] (a boolean; default: \"false\")\n    debug"));
    assert!(usage.contains(
        "  --host (a string; default: \"www.google.com\")\n    The URL at which the server will be running."
    ));
    assert!(usage.contains(
        "  --port [-p] (an integer; default: \"80\")\n    The port at which the server will be running."
    ));
    assert!(usage.contains(
        "  --[no]tristate [-t] (a tri-state (auto, yes, no); default: \"auto\")\n    tri-state option returning auto by default"
    ));
}

// Java: OptionsTest.usageForStaticExpansion
#[test]
fn usage_for_static_expansion() {
    let usage = Options::<HttpOptions>::get_usage();
    assert!(usage.contains("  --special\n      Expands to: --host=special.google.com --port=8080"));
}

options! {
    pub struct NullTestOptions {
        #[option(
            name = "host",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            help = "The URL at which the server will be running.",
        )]
        pub host: Option<String>,

        #[option(
            name = "none",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = ["--host=www.google.com"],
            help = "An expanded option.",
        )]
        pub none: Void,
    }
}

// Java: OptionsTest.usageForNullDefault
#[test]
fn usage_for_null_default() {
    let usage = Options::<NullTestOptions>::get_usage();
    assert!(usage.contains(
        "  --host (a string; default: see description)\n    The URL at which the server will be running."
    ));
    assert!(
        usage
            .contains("  --none\n    An expanded option.\n      Expands to: --host=www.google.com")
    );
}

/// Test-only stand-in for `java.net.URL`: only the protocol check `new URL(String)` performs
/// first ("no protocol: ...") is modelled.
#[derive(Clone, Debug, PartialEq)]
pub struct Url(String);

impl OptionScalar for Url {
    fn java_to_string(&self) -> String {
        self.0.clone()
    }
}
option_field_type!(Url, "java.net.URL", raw = Url(String::new()));

/// Java: OptionsTest.MyURLConverter
#[derive(Default)]
pub struct MyUrlConverter;

impl Converter for MyUrlConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = input.unwrap();
        let has_protocol = input.split_once(':').is_some_and(|(scheme, _)| {
            let mut c = scheme.chars();
            c.next().is_some_and(|f| f.is_ascii_alphabetic())
                && c.all(|ch| ch.is_ascii_alphanumeric() || "+.-".contains(ch))
        });
        if !has_protocol {
            return Err(OptionsParsingException::new(format!(
                "Could not convert '{}': no protocol: {}",
                input, input
            )));
        }
        Ok(Some(Value::new(Url(input.to_string()))))
    }
    fn get_type_description(&self) -> String {
        "a url".into()
    }
}

options! {
    pub struct UsesCustomConverter {
        #[option(
            name = "url",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "http://www.google.com/",
            converter = MyUrlConverter,
        )]
        pub url: Url,
    }
}

// Java: OptionsTest.customConverter
#[test]
fn custom_converter() {
    let options = Options::<UsesCustomConverter>::parse(&[]).unwrap();
    assert_eq!(
        options.get_options().url,
        Url("http://www.google.com/".to_string())
    );
}

// Java: OptionsTest.customConverterThrowsException
#[test]
fn custom_converter_throws_exception() {
    let e = Options::<UsesCustomConverter>::parse(&["--url=a_malformed:url"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "While parsing option --url=a_malformed:url: Could not convert 'a_malformed:url': no protocol: a_malformed:url"
    );
}

// Java: OptionsTest.usageWithCustomConverter
#[test]
fn usage_with_custom_converter() {
    assert_eq!(
        Options::<UsesCustomConverter>::get_usage(),
        "  --url (a url; default: \"http://www.google.com/\")\n"
    );
}

// Java: OptionsTest.unknownBooleanOptionNegativeForm
#[test]
fn unknown_boolean_option_negative_form() {
    let e = parse(&["--no-debug"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unrecognized option: --no-debug (did you mean '--nodebug'?)"
    );
}

// Java: OptionsTest.unknownOption
#[test]
fn unknown_option() {
    let e = parse(&["--pert"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unrecognized option: --pert (did you mean '--port'?)"
    );
}

// Java: OptionsTest.unknownBooleanOptionPositiveForm
#[test]
fn unknown_boolean_option_positive_form() {
    let e = parse(&["--dbg"]).unwrap_err();
    assert_eq!(
        e.get_message(),
        "Unrecognized option: --dbg (did you mean '--debug'?)"
    );
}

options! {
    pub struct J {
        #[option(
            name = "j",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
        )]
        pub string: Option<String>,
    }
}

// Java: OptionsTest.nullDefaultForReferenceTypeOption
#[test]
fn null_default_for_reference_type_option() {
    let options = Options::<J>::parse(NO_ARGS).unwrap().into_options();
    assert_eq!(options.string, None);
}

// Java: OptionsTest.nullIsNotInterpretedSpeciallyExceptAsADefaultValue
#[test]
fn null_is_not_interpreted_specially_except_as_a_default_value() {
    let options = parse(&["--host", "null"]).unwrap().into_options();
    assert_eq!(options.host, "null");
}

// Java: OptionsTest.nonDecimalRadicesForIntegerOptions
#[test]
fn non_decimal_radices_for_integer_options() {
    let options = parse(&["--port", "0x51"]).unwrap();
    assert_eq!(options.get_options().port, 81);
}

// Java: OptionsTest.expansionOptionSimple
#[test]
fn expansion_option_simple() {
    let options = parse(&["--special"]).unwrap();
    assert_eq!(options.get_options().host, "special.google.com");
    assert_eq!(options.get_options().port, 8080);
}

// Java: OptionsTest.expansionOptionOverride
#[test]
fn expansion_option_override() {
    let options = parse(&["--port=90", "--special", "--host=foo"]).unwrap();
    assert_eq!(options.get_options().host, "foo");
    assert_eq!(options.get_options().port, 8080);
}

// Java: OptionsTest.expansionOptionEquals
#[test]
fn expansion_option_equals() {
    let options1 = parse(&["--host=special.google.com", "--port=8080"]).unwrap();
    let options2 = parse(&["--special"]).unwrap();
    assert_eq!(options1.get_options(), options2.get_options());
}
