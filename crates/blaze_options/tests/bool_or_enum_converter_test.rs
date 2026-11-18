//! Port of src/test/java/com/google/devtools/common/options/BoolOrEnumConverterTest.java.

use blaze_options::{
    Converter, OptionsClass, OptionsParser, bool_or_enum_converter, option_enum, options,
};

option_enum! {
    enum CompilationMode { DBG, OPT }
}

bool_or_enum_converter!(
    CompilationModeConverter,
    CompilationMode,
    "compilation mode",
    CompilationMode::DBG,
    CompilationMode::OPT
);

options! {
    /// The test options for the CompilationMode hybrid converter.
    pub struct CompilationModeTestOptions {
        #[option(
            name = "compile_mode",
            converter = CompilationModeConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "dbg",
        )]
        compile_mode: CompilationMode,
    }
}

fn convert(input: &str) -> Result<CompilationMode, blaze_options::OptionsParsingException> {
    CompilationModeConverter
        .convert(Some(input), None)
        .map(|v| v.unwrap().get::<CompilationMode>())
}

// Java: BoolOrEnumConverterTest.converterFromEnum
#[test]
fn converter_from_enum() {
    assert_eq!(convert("dbg").unwrap(), CompilationMode::DBG);
    assert_eq!(convert("opt").unwrap(), CompilationMode::OPT);
    let e = convert("none").unwrap_err();
    assert_eq!(
        e.get_message(),
        "Not a valid compilation mode: 'none' (should be dbg or opt)"
    );
    assert_eq!(
        CompilationModeConverter.get_type_description(),
        "dbg or opt"
    );
}

// Java: BoolOrEnumConverterTest.convertFromBooleanValues
#[test]
fn convert_from_boolean_values() {
    for false_value in ["false", "0"] {
        assert_eq!(convert(false_value).unwrap(), CompilationMode::OPT);
    }
    for true_value in ["true", "1"] {
        assert_eq!(convert(true_value).unwrap(), CompilationMode::DBG);
    }
}

fn parser() -> OptionsParser {
    OptionsParser::builder()
        .options_classes(&[CompilationModeTestOptions::class()])
        .build()
        .unwrap()
}

// Java: BoolOrEnumConverterTest.prefixedWithNo
#[test]
fn prefixed_with_no() {
    let mut parser = parser();
    parser.parse(&["--nocompile_mode"]).unwrap();
    let options = parser.get_options::<CompilationModeTestOptions>().unwrap();
    assert_eq!(options.compile_mode, CompilationMode::OPT);
}

// Java: BoolOrEnumConverterTest.missingValueAsBoolConversion
#[test]
fn missing_value_as_bool_conversion() {
    let mut parser = parser();
    parser.parse(&["--compile_mode"]).unwrap();
    let options = parser.get_options::<CompilationModeTestOptions>().unwrap();
    assert_eq!(options.compile_mode, CompilationMode::DBG);
}
