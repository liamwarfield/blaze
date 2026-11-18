//! Port of src/test/java/com/google/devtools/common/options/EnumConverterTest.java.

use blaze_options::{Converter, OptionsClass, OptionsParser, enum_converter, option_enum, options};

option_enum! {
    enum CompilationMode { DBG, OPT }
}
enum_converter!(
    CompilationModeConverter,
    CompilationMode,
    "compilation mode"
);

// Java: EnumConverterTest.converterForEnumWithTwoValues
#[test]
fn converter_for_enum_with_two_values() {
    let c = CompilationModeConverter;
    let conv = |s: &str| {
        c.convert(Some(s), None)
            .map(|v| v.unwrap().get::<CompilationMode>())
    };
    assert_eq!(conv("dbg").unwrap(), CompilationMode::DBG);
    assert_eq!(conv("opt").unwrap(), CompilationMode::OPT);
    let e = conv("none").unwrap_err();
    assert_eq!(
        e.get_message(),
        "Not a valid compilation mode: 'none' (should be dbg or opt)"
    );
    assert_eq!(c.get_type_description(), "dbg or opt");
}

option_enum! {
    enum Fruit { Apple, Banana, Cherry }
}
enum_converter!(FruitConverter, Fruit, "fruit");

// Java: EnumConverterTest.typeDescriptionForEnumWithThreeValues
#[test]
fn type_description_for_enum_with_three_values() {
    // We always use lowercase in the user-visible messages:
    assert_eq!(
        FruitConverter.get_type_description(),
        "apple, banana or cherry"
    );
}

// Java: EnumConverterTest.converterIsCaseInsensitive
#[test]
fn converter_is_case_insensitive() {
    let v = FruitConverter
        .convert(Some("bAnANa"), None)
        .unwrap()
        .unwrap();
    assert_eq!(v.get::<Fruit>(), Fruit::Banana);
}

option_enum! {
    enum AlphabetEnum { ALPHA, BRAVO, CHARLY, DELTA, ECHO }
}
enum_converter!(AlphabetEnumConverter, AlphabetEnum, "alphabet enum");

options! {
    pub struct EnumListTestOptions {
        #[option(
            name = "goo",
            allow_multiple = true,
            converter = AlphabetEnumConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
        )]
        goo: Vec<AlphabetEnum>,
    }
}

// Java: EnumConverterTest.enumList
#[test]
fn enum_list() {
    let mut parser = OptionsParser::builder()
        .options_classes(&[EnumListTestOptions::class()])
        .build()
        .unwrap();
    parser.parse(&["--goo=ALPHA", "--goo=BRAVO"]).unwrap();
    let options = parser.get_options::<EnumListTestOptions>().unwrap();
    assert_eq!(options.goo, vec![AlphabetEnum::ALPHA, AlphabetEnum::BRAVO]);
}
