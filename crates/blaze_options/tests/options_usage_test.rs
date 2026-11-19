//! Port of src/test/java/com/google/devtools/common/options/OptionsUsageTest.java.

mod common;

use blaze_options::html_utils::{HtmlUtils, html_escaper};
use blaze_options::{HelpVerbosity, OptionsClass, OptionsData, OptionsUsage};
use common::test_options::TestOptions;

fn data() -> std::sync::Arc<OptionsData> {
    OptionsData::get(&[TestOptions::class()], false).unwrap_or_else(|e| e.throw())
}

fn get_html_usage_without_tags(field_name: &str) -> String {
    let data = data();
    let mut builder = String::new();
    HtmlUtils::get_usage_html(
        data.get_option_definition_from_name(field_name).unwrap(),
        &mut builder,
        html_escaper,
        &data,
        false,
        None,
    );
    builder
}

fn get_html_usage_with_tags(field_name: &str) -> String {
    let data = data();
    let mut builder = String::new();
    HtmlUtils::get_usage_html(
        data.get_option_definition_from_name(field_name).unwrap(),
        &mut builder,
        html_escaper,
        &data,
        true,
        None,
    );
    builder
}

fn get_html_usage_with_command_name(field_name: &str, command_name: &str) -> String {
    let data = data();
    let mut builder = String::new();
    HtmlUtils::get_usage_html(
        data.get_option_definition_from_name(field_name).unwrap(),
        &mut builder,
        html_escaper,
        &data,
        false,
        Some(command_name),
    );
    builder
}

fn get_terminal_usage_without_tags(field_name: &str, verbosity: HelpVerbosity) -> String {
    let data = data();
    let mut builder = String::new();
    OptionsUsage::get_usage(
        data.get_option_definition_from_name(field_name).unwrap(),
        &mut builder,
        verbosity,
        &data,
        false,
    );
    builder
}

/// Tests the future behavior of the options usage output. For short & medium verbosity, this
/// should be the same as the current default
fn get_terminal_usage_with_tags(field_name: &str, verbosity: HelpVerbosity) -> String {
    let data = data();
    let mut builder = String::new();
    OptionsUsage::get_usage(
        data.get_option_definition_from_name(field_name).unwrap(),
        &mut builder,
        verbosity,
        &data,
        true,
    );
    builder
}

// Java: OptionsUsageTest.commandNameAnchorId_htmlOutput

// Java: OptionsUsageTest.commandNameAnchorId_htmlOutput
#[test]
fn command_name_anchor_id_html_output() {
    assert_eq!(
        get_html_usage_with_command_name("test_string", "command_name"),
        "<dt id=\"command_name-flag--test_string\"><code id=\"test_string\"><a href=\"#command_name-flag--test_string\">--test_string</a>=&lt;a string&gt;</code> default: \"test string default\"</dt>\n<dd>\n<p>a string-valued option to test simple option operations</p>\n</dd>\n"
    );
}

// Java: OptionsUsageTest.stringValue_shortTerminalOutput
#[test]
fn string_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_string", HelpVerbosity::SHORT),
        "  --test_string\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_string", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_string", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.stringValue_mediumTerminalOutput
#[test]
fn string_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_string", HelpVerbosity::MEDIUM),
        "  --test_string (a string; default: \"test string default\")\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_string", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_string", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.stringValue_longTerminalOutput
#[test]
fn string_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_string", HelpVerbosity::LONG),
        concat!(
            "  --test_string (a string; default: \"test string default\")\n",
            "    a string-valued option to test simple option operations\n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_string", HelpVerbosity::LONG),
        concat!(
            "  --test_string (a string; default: \"test string default\")\n",
            "    a string-valued option to test simple option operations\n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.stringValue_htmlOutput
#[test]
fn string_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_string"),
        "<dt id=\"flag--test_string\"><code><a href=\"#flag--test_string\">--test_string</a>=&lt;a string&gt;</code> default: \"test string default\"</dt>\n<dd>\n<p>a string-valued option to test simple option operations</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_string"),
        "<dt id=\"flag--test_string\"><code><a href=\"#flag--test_string\">--test_string</a>=&lt;a string&gt;</code> default: \"test string default\"</dt>\n<dd>\n<p>a string-valued option to test simple option operations</p>\n<p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.intValue_shortTerminalOutput
#[test]
fn int_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_c", HelpVerbosity::SHORT),
        "  --expanded_c\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("expanded_c", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("expanded_c", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.intValue_mediumTerminalOutput
#[test]
fn int_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_c", HelpVerbosity::MEDIUM),
        "  --expanded_c (an integer; default: \"12\")\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("expanded_c", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("expanded_c", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.intValue_longTerminalOutput
#[test]
fn int_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_c", HelpVerbosity::LONG),
        concat!(
            "  --expanded_c (an integer; default: \"12\")\n",
            "    an int-value'd flag used to test expansion logic\n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("expanded_c", HelpVerbosity::LONG),
        concat!(
            "  --expanded_c (an integer; default: \"12\")\n",
            "    an int-value'd flag used to test expansion logic\n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.intValue_htmlOutput
#[test]
fn int_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("expanded_c"),
        "<dt id=\"flag--expanded_c\"><code><a href=\"#flag--expanded_c\">--expanded_c</a>=&lt;an integer&gt;</code> default: \"12\"</dt>\n<dd>\n<p>an int-value'd flag used to test expansion logic</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("expanded_c"),
        "<dt id=\"flag--expanded_c\"><code><a href=\"#flag--expanded_c\">--expanded_c</a>=&lt;an integer&gt;</code> default: \"12\"</dt>\n<dd>\n<p>an int-value'd flag used to test expansion logic</p>\n<p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.booleanValue_shortTerminalOutput
#[test]
fn boolean_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::SHORT),
        "  --[no]expanded_a\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("expanded_a", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.booleanValue_mediumTerminalOutput
#[test]
fn boolean_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::MEDIUM),
        "  --[no]expanded_a (a boolean; default: \"true\")\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("expanded_a", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.booleanValue_longTerminalOutput
#[test]
fn boolean_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::LONG),
        concat!(
            "  --[no]expanded_a (a boolean; default: \"true\")\n",
            "    A boolean flag with unknown effect to test tagless usage text.\n"
        )
    );
    // This flag has no useful tags, to verify that the tag line is omitted, so the usage line
    // should be the same in both tag and tag-free world.
    assert_eq!(
        get_terminal_usage_without_tags("expanded_a", HelpVerbosity::LONG),
        get_terminal_usage_with_tags("expanded_a", HelpVerbosity::LONG)
    );
}

// Java: OptionsUsageTest.booleanValue_htmlOutput
#[test]
fn boolean_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("expanded_a"),
        "<dt id=\"flag--expanded_a\"><code><a href=\"#flag--expanded_a\">--[no]expanded_a</a></code> default: \"true\"</dt>\n<dd>\n<p>A boolean flag with unknown effect to test tagless usage text.</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_without_tags("expanded_a"),
        get_html_usage_with_tags("expanded_a")
    );
}

// Java: OptionsUsageTest.multipleValue_shortTerminalOutput
#[test]
fn multiple_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_multiple_string", HelpVerbosity::SHORT),
        "  --test_multiple_string\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_multiple_string", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_multiple_string", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.multipleValue_mediumTerminalOutput
#[test]
fn multiple_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_multiple_string", HelpVerbosity::MEDIUM),
        "  --test_multiple_string (a string; may be used multiple times)\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_multiple_string", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_multiple_string", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.multipleValue_longTerminalOutput
#[test]
fn multiple_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_multiple_string", HelpVerbosity::LONG),
        concat!(
            "  --test_multiple_string (a string; may be used multiple times)\n",
            "    a repeatable string-valued flag with its own unhelpful help text\n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_multiple_string", HelpVerbosity::LONG),
        concat!(
            "  --test_multiple_string (a string; may be used multiple times)\n",
            "    a repeatable string-valued flag with its own unhelpful help text\n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.multipleValue_htmlOutput
#[test]
fn multiple_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_multiple_string"),
        "<dt id=\"flag--test_multiple_string\"><code><a href=\"#flag--test_multiple_string\">--test_multiple_string</a>=&lt;a string&gt;</code> multiple uses are accumulated</dt>\n<dd>\n<p>a repeatable string-valued flag with its own unhelpful help text</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_multiple_string"),
        "<dt id=\"flag--test_multiple_string\"><code><a href=\"#flag--test_multiple_string\">--test_multiple_string</a>=&lt;a string&gt;</code> multiple uses are accumulated</dt>\n<dd>\n<p>a repeatable string-valued flag with its own unhelpful help text</p>\n<p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.customConverterValue_shortTerminalOutput
#[test]
fn custom_converter_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_list_converters", HelpVerbosity::SHORT),
        "  --test_list_converters\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_list_converters", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_list_converters", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.customConverterValue_mediumTerminalOutput
#[test]
fn custom_converter_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_list_converters", HelpVerbosity::MEDIUM),
        "  --test_list_converters (a list of strings; may be used multiple times)\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_list_converters", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_list_converters", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.customConverterValue_longTerminalOutput
#[test]
fn custom_converter_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_list_converters", HelpVerbosity::LONG),
        concat!(
            "  --test_list_converters (a list of strings; may be used multiple times)\n",
            "    a repeatable flag that accepts lists, but doesn't want to have lists of \n",
            "    lists as a final type\n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_list_converters", HelpVerbosity::LONG),
        concat!(
            "  --test_list_converters (a list of strings; may be used multiple times)\n",
            "    a repeatable flag that accepts lists, but doesn't want to have lists of \n",
            "    lists as a final type\n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.customConverterValue_htmlOutput
#[test]
fn custom_converter_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_list_converters"),
        "<dt id=\"flag--test_list_converters\"><code><a href=\"#flag--test_list_converters\">--test_list_converters</a>=&lt;a list of strings&gt;</code> multiple uses are accumulated</dt>\n<dd>\n<p>a repeatable flag that accepts lists, but doesn't want to have lists of lists as a final type</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_list_converters"),
        "<dt id=\"flag--test_list_converters\"><code><a href=\"#flag--test_list_converters\">--test_list_converters</a>=&lt;a list of strings&gt;</code> multiple uses are accumulated</dt>\n<dd>\n<p>a repeatable flag that accepts lists, but doesn't want to have lists of lists as a final type</p>\n<p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.staticExpansionOption_shortTerminalOutput
#[test]
fn static_expansion_option_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion", HelpVerbosity::SHORT),
        "  --test_expansion\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_expansion", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.staticExpansionOption_mediumTerminalOutput
#[test]
fn static_expansion_option_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion", HelpVerbosity::MEDIUM),
        "  --test_expansion\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_expansion", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.staticExpansionOption_longTerminalOutput
#[test]
fn static_expansion_option_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion", HelpVerbosity::LONG),
        concat!(
            "  --test_expansion\n",
            "    this expands to an alphabet soup.\n",
            "      Expands to: --noexpanded_a --expanded_b=false --expanded_c 42 --\n",
            "      expanded_d bar \n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_expansion", HelpVerbosity::LONG),
        concat!(
            "  --test_expansion\n",
            "    this expands to an alphabet soup.\n",
            "      Expands to: --noexpanded_a --expanded_b=false --expanded_c 42 --\n",
            "      expanded_d bar \n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.staticExpansionOption_htmlOutput
#[test]
fn static_expansion_option_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_expansion"),
        "<dt id=\"flag--test_expansion\"><code><a href=\"#flag--test_expansion\">--test_expansion</a></code></dt>\n<dd>\n<p>this expands to an alphabet soup.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--noexpanded_a\">--noexpanded_a</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_b\">--expanded_b=false</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_c\">--expanded_c</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag42\">42</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_d\">--expanded_d</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flagbar\">bar</a></code>\n</p></dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_expansion"),
        "<dt id=\"flag--test_expansion\"><code><a href=\"#flag--test_expansion\">--test_expansion</a></code></dt>\n<dd>\n<p>this expands to an alphabet soup.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--noexpanded_a\">--noexpanded_a</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_b\">--expanded_b=false</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_c\">--expanded_c</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag42\">42</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--expanded_d\">--expanded_d</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flagbar\">bar</a></code>\n</p><p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.recursiveExpansionOption_shortTerminalOutput
#[test]
fn recursive_expansion_option_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_recursive_expansion_top_level", HelpVerbosity::SHORT),
        "  --test_recursive_expansion_top_level\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_recursive_expansion_top_level", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_recursive_expansion_top_level", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.recursiveExpansionOption_mediumTerminalOutput
#[test]
fn recursive_expansion_option_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags(
            "test_recursive_expansion_top_level",
            HelpVerbosity::MEDIUM
        ),
        "  --test_recursive_expansion_top_level\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags(
            "test_recursive_expansion_top_level",
            HelpVerbosity::MEDIUM
        ),
        get_terminal_usage_with_tags("test_recursive_expansion_top_level", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.recursiveExpansionOption_longTerminalOutput
#[test]
fn recursive_expansion_option_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_recursive_expansion_top_level", HelpVerbosity::LONG),
        concat!(
            "  --test_recursive_expansion_top_level\n",
            "    Lets the children do all the work.\n",
            "      Expands to: --test_recursive_expansion_middle1 --\n",
            "      test_recursive_expansion_middle2 \n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_recursive_expansion_top_level", HelpVerbosity::LONG),
        concat!(
            "  --test_recursive_expansion_top_level\n",
            "    Lets the children do all the work.\n",
            "      Expands to: --test_recursive_expansion_middle1 --\n",
            "      test_recursive_expansion_middle2 \n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.recursiveExpansionOption_htmlOutput
#[test]
fn recursive_expansion_option_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_recursive_expansion_top_level"),
        "<dt id=\"flag--test_recursive_expansion_top_level\"><code><a href=\"#flag--test_recursive_expansion_top_level\">--test_recursive_expansion_top_level</a></code></dt>\n<dd>\n<p>Lets the children do all the work.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_recursive_expansion_middle1\">--test_recursive_expansion_middle1</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_recursive_expansion_middle2\">--test_recursive_expansion_middle2</a></code>\n</p></dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_recursive_expansion_top_level"),
        "<dt id=\"flag--test_recursive_expansion_top_level\"><code><a href=\"#flag--test_recursive_expansion_top_level\">--test_recursive_expansion_top_level</a></code></dt>\n<dd>\n<p>Lets the children do all the work.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_recursive_expansion_middle1\">--test_recursive_expansion_middle1</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_recursive_expansion_middle2\">--test_recursive_expansion_middle2</a></code>\n</p><p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.expansionToMultipleValue_shortTerminalOutput
#[test]
fn expansion_to_multiple_value_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion_to_repeatable", HelpVerbosity::SHORT),
        "  --test_expansion_to_repeatable\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion_to_repeatable", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_expansion_to_repeatable", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.expansionToMultipleValue_mediumTerminalOutput
#[test]
fn expansion_to_multiple_value_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion_to_repeatable", HelpVerbosity::MEDIUM),
        "  --test_expansion_to_repeatable\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion_to_repeatable", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_expansion_to_repeatable", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.expansionToMultipleValue_longTerminalOutput
#[test]
fn expansion_to_multiple_value_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_expansion_to_repeatable", HelpVerbosity::LONG),
        concat!(
            "  --test_expansion_to_repeatable\n",
            "    Go forth and multiply, they said.\n",
            "      Expands to: --test_multiple_string=expandedFirstValue --\n",
            "      test_multiple_string=expandedSecondValue \n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_expansion_to_repeatable", HelpVerbosity::LONG),
        concat!(
            "  --test_expansion_to_repeatable\n",
            "    Go forth and multiply, they said.\n",
            "      Expands to: --test_multiple_string=expandedFirstValue --\n",
            "      test_multiple_string=expandedSecondValue \n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.expansionToMultipleValue_htmlOutput
#[test]
fn expansion_to_multiple_value_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_expansion_to_repeatable"),
        "<dt id=\"flag--test_expansion_to_repeatable\"><code><a href=\"#flag--test_expansion_to_repeatable\">--test_expansion_to_repeatable</a></code></dt>\n<dd>\n<p>Go forth and multiply, they said.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_multiple_string\">--test_multiple_string=expandedFirstValue</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_multiple_string\">--test_multiple_string=expandedSecondValue</a></code>\n</p></dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_expansion_to_repeatable"),
        "<dt id=\"flag--test_expansion_to_repeatable\"><code><a href=\"#flag--test_expansion_to_repeatable\">--test_expansion_to_repeatable</a></code></dt>\n<dd>\n<p>Go forth and multiply, they said.</p>\n<p>Expands to:\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_multiple_string\">--test_multiple_string=expandedFirstValue</a></code>\n<br/>&nbsp;&nbsp;<code><a href=\"#flag--test_multiple_string\">--test_multiple_string=expandedSecondValue</a></code>\n</p><p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.implicitRequirementOption_shortTerminalOutput
#[test]
fn implicit_requirement_option_short_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_implicit_requirement", HelpVerbosity::SHORT),
        "  --test_implicit_requirement\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_implicit_requirement", HelpVerbosity::SHORT),
        get_terminal_usage_with_tags("test_implicit_requirement", HelpVerbosity::SHORT)
    );
}

// Java: OptionsUsageTest.implicitRequirementOption_mediumTerminalOutput
#[test]
fn implicit_requirement_option_medium_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_implicit_requirement", HelpVerbosity::MEDIUM),
        "  --test_implicit_requirement (a string; default: \"direct implicit\")\n"
    );
    assert_eq!(
        get_terminal_usage_without_tags("test_implicit_requirement", HelpVerbosity::MEDIUM),
        get_terminal_usage_with_tags("test_implicit_requirement", HelpVerbosity::MEDIUM)
    );
}

// Java: OptionsUsageTest.implicitRequirementOption_longTerminalOutput
#[test]
fn implicit_requirement_option_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("test_implicit_requirement", HelpVerbosity::LONG),
        concat!(
            "  --test_implicit_requirement (a string; default: \"direct implicit\")\n",
            "    this option really needs that other one, isolation of purpose has failed.\n",
            "      Using this option will also add: --implicit_requirement_a=implicit \n",
            "      requirement, required \n"
        )
    );
    assert_eq!(
        get_terminal_usage_with_tags("test_implicit_requirement", HelpVerbosity::LONG),
        concat!(
            "  --test_implicit_requirement (a string; default: \"direct implicit\")\n",
            "    this option really needs that other one, isolation of purpose has failed.\n",
            "      Using this option will also add: --implicit_requirement_a=implicit \n",
            "      requirement, required \n",
            "      Tags: no_op\n"
        )
    );
}

// Java: OptionsUsageTest.implicitRequirementOption_htmlOutput
#[test]
fn implicit_requirement_option_html_output() {
    assert_eq!(
        get_html_usage_without_tags("test_implicit_requirement"),
        "<dt id=\"flag--test_implicit_requirement\"><code><a href=\"#flag--test_implicit_requirement\">--test_implicit_requirement</a>=&lt;a string&gt;</code> default: \"direct implicit\"</dt>\n<dd>\n<p>this option really needs that other one, isolation of purpose has failed.</p>\n</dd>\n"
    );
    assert_eq!(
        get_html_usage_with_tags("test_implicit_requirement"),
        "<dt id=\"flag--test_implicit_requirement\"><code><a href=\"#flag--test_implicit_requirement\">--test_implicit_requirement</a>=&lt;a string&gt;</code> default: \"direct implicit\"</dt>\n<dd>\n<p>this option really needs that other one, isolation of purpose has failed.</p>\n<p>Tags:\n<a href=\"#effect_tag_NO_OP\"><code>no_op</code></a>\n</p></dd>\n"
    );
}

// Java: OptionsUsageTest.markdownInHelp_longTerminalOutput
#[test]
fn markdown_in_help_long_terminal_output() {
    assert_eq!(
        get_terminal_usage_without_tags("markdown_in_help", HelpVerbosity::LONG),
        "  --markdown_in_help (a string; default: \"default\")\n    normal\n    `code span`\n    *emphasis*\n    **strong emphasis**\n    [inline link](/url (title))\n    [reference link][ref]\n    [shorthand reference link]\n    [`complex` shorthand reference link]\n    hard line\\\n    break\n    ```\n    code block\n    ```\n    - unordered\n    - list\n    1. ordered\n    2. list\n    \n    paragraph 1\n    \n    paragraph 2\n    \n    `<HTML> \"syntax\" 'within' &codeblocks&`\n    \n    [ref]: /url (title)\n    [shorthand reference link]: /url (title)\n    [`complex` shorthand reference link]: /url (title)\n    \n"
    );
}

// Java: OptionsUsageTest.markdownInHelp_htmlOutput
#[test]
fn markdown_in_help_html_output() {
    assert_eq!(
        get_html_usage_without_tags("markdown_in_help"),
        "<dt id=\"flag--markdown_in_help\"><code><a href=\"#flag--markdown_in_help\">--markdown_in_help</a>=&lt;a string&gt;</code> default: \"default\"</dt>\n<dd>\n<p>normal\n<code>code span</code>\n<em>emphasis</em>\n<strong>strong emphasis</strong>\n<a href=\"/url\" title=\"title\">inline link</a>\n<a href=\"/url\" title=\"title\">reference link</a>\n<a href=\"/url\" title=\"title\">shorthand reference link</a>\n<a href=\"/url\" title=\"title\"><code>complex</code> shorthand reference link</a>\nhard line<br />\nbreak</p>\n<pre><code>code block\n</code></pre>\n<ul>\n<li>unordered</li>\n<li>list</li>\n</ul>\n<ol>\n<li>ordered</li>\n<li>list</li>\n</ol>\n<p>paragraph 1</p>\n<p>paragraph 2</p>\n<p><code>&lt;HTML&gt; &quot;syntax&quot; 'within' &amp;codeblocks&amp;</code></p>\n</dd>\n"
    );
}
