//! Port of src/test/java/com/google/devtools/common/options/TestOptions.java.

use blaze_options::{ConversionContext, Converter, OptionsParsingException, Value, Void, options};

pub const TEST_STRING_DEFAULT: &str = "test string default";
pub const EXPANDED_A_TEST_EXPANSION: bool = false;
pub const EXPANDED_B_TEST_EXPANSION: bool = false;
pub const EXPANDED_C_TEST_EXPANSION: i32 = 42;
pub const EXPANDED_D_TEST_EXPANSION: &str = "bar";
pub const EXPANDED_A_TEST_RECURSIVE_EXPANSION: bool = false;
pub const EXPANDED_B_TEST_RECURSIVE_EXPANSION: bool = false;
pub const EXPANDED_C_TEST_RECURSIVE_EXPANSION: i32 = 56;
pub const EXPANDED_D_TEST_RECURSIVE_EXPANSION: &str = "baz";
pub const EXPANDED_A_DEFAULT: bool = true;
pub const EXPANDED_B_DEFAULT: bool = true;
pub const EXPANDED_C_DEFAULT: i32 = 12;
pub const EXPANDED_D_DEFAULT: &str = "foo";
pub const EXPANDED_MULTIPLE_1: &str = "expandedFirstValue";
pub const EXPANDED_MULTIPLE_2: &str = "expandedSecondValue";
pub const TEST_IMPLICIT_REQUIREMENT_DEFAULT: &str = "direct implicit";
pub const IMPLICIT_REQUIREMENT_A_REQUIRED: &str = "implicit requirement, required";
pub const IMPLICIT_REQUIREMENT_A_DEFAULT: &str = "implicit requirement, unrequired";
pub const TEST_RECURSIVE_IMPLICIT_REQUIREMENT_DEFAULT: &str = "recursive implicit";
pub const TEST_IMPLICIT_REQUIREMENT_REQUIRED: &str = "intermediate, required";

/// Java: TestOptions.ToListConverter (used by test_list_converters).
#[derive(Default)]
pub struct ToListConverter;

impl Converter for ToListConverter {
    fn convert(
        &self,
        input: Option<&str>,
        _: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        let input = input.unwrap();
        Ok(Some(Value::list_of(
            input.split(',').map(str::to_string).collect::<Vec<_>>(),
        )))
    }
    fn get_type_description(&self) -> String {
        "a list of strings".into()
    }
}

options! {
    /// Options for testing.
    pub struct TestOptions {
        #[option(
            name = "test_string",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = TEST_STRING_DEFAULT,
            help = "a string-valued option to test simple option operations",
        )]
        pub test_string: String,

        #[option(
            name = "test_string_null_by_default",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            help = "a string-valued option that has the special string 'null' as its default.",
        )]
        pub test_string_null_by_default: Option<String>,

        #[option(
            name = "test_multiple_string",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            allow_multiple = true,
            help = "a repeatable string-valued flag with its own unhelpful help text",
        )]
        pub test_multiple_string: Vec<String>,

        #[option(
            name = "test_list_converters",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            allow_multiple = true,
            converter = ToListConverter,
            help = "a repeatable flag that accepts lists, but doesn't want to have lists of lists as a final type",
        )]
        pub test_list_converters: Vec<String>,

        #[option(
            name = "test_expansion",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = [
                "--noexpanded_a",
                "--expanded_b=false",
                "--expanded_c",
                "42",
                "--expanded_d",
                "bar",
            ],
            help = "this expands to an alphabet soup.",
        )]
        pub test_expansion: Void,

        #[option(
            name = "test_recursive_expansion_top_level",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = [
                "--test_recursive_expansion_middle1",
                "--test_recursive_expansion_middle2",
            ],
            help = "Lets the children do all the work.",
        )]
        pub test_recursive_expansion_top_level: Void,

        #[option(
            name = "test_recursive_expansion_middle1",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = ["--expanded_a=false", "--expanded_c=56"],
        )]
        pub test_recursive_expansion_middle1: Void,

        #[option(
            name = "test_recursive_expansion_middle2",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = ["--expanded_b=false", "--expanded_d=baz"],
        )]
        pub test_recursive_expansion_middle2: Void,

        #[option(
            name = "expanded_a",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            default_value = "true",
            help = "A boolean flag with unknown effect to test tagless usage text.",
        )]
        pub expanded_a: bool,

        #[option(
            name = "expanded_b",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "true",
        )]
        pub expanded_b: bool,

        #[option(
            name = "expanded_c",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "12",
            help = "an int-value'd flag used to test expansion logic",
        )]
        pub expanded_c: i32,

        #[option(
            name = "expanded_d",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "foo",
        )]
        pub expanded_d: String,

        #[option(
            name = "test_expansion_to_repeatable",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = [
                "--test_multiple_string=expandedFirstValue",
                "--test_multiple_string=expandedSecondValue",
            ],
            help = "Go forth and multiply, they said.",
        )]
        pub test_expansion_to_repeatable: Void,

        #[option(
            name = "test_implicit_requirement",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = TEST_IMPLICIT_REQUIREMENT_DEFAULT,
            implicit_requirements = ["--implicit_requirement_a=implicit requirement, required"],
            help = "this option really needs that other one, isolation of purpose has failed.",
        )]
        pub test_implicit_requirement: String,

        #[option(
            name = "implicit_requirement_a",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = IMPLICIT_REQUIREMENT_A_DEFAULT,
        )]
        pub implicit_requirement_a: String,

        #[option(
            name = "test_recursive_implicit_requirement",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = TEST_RECURSIVE_IMPLICIT_REQUIREMENT_DEFAULT,
            implicit_requirements = ["--test_implicit_requirement=intermediate, required"],
        )]
        pub test_recursive_implicit_requirement: String,

        #[option(
            name = "test_deprecated",
            default_value = "default",
            deprecation_warning = "Flag for testing deprecation behavior.",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub test_deprecated: String,

        #[option(
            name = "test_new_and_old_name",
            old_name = "test_old_name",
            default_value = "default",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            help = "A test option with both a new name and an old name.",
        )]
        pub test_new_and_old_name: String,

        #[option(
            name = "markdown_in_help",
            default_value = "default",
            help = "normal\n`code span`\n*emphasis*\n**strong emphasis**\n[inline link](/url (title))\n[reference link][ref]\n[shorthand reference link]\n[`complex` shorthand reference link]\nhard line\\\nbreak\n```\ncode block\n```\n- unordered\n- list\n1. ordered\n2. list\n\nparagraph 1\n\nparagraph 2\n\n`<HTML> \"syntax\" 'within' &codeblocks&`\n\n[ref]: /url (title)\n[shorthand reference link]: /url (title)\n[`complex` shorthand reference link]: /url (title)\n",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
        )]
        pub markdown_in_help: String,
    }
}
