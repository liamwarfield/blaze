//! Port of src/test/java/com/google/devtools/common/options/OptionsMapConversionTest.java.

use blaze_options::{Options, OptionsBase, Value, options};

options! {
    /// Dummy options base class.
    pub struct FooOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub foo: bool,
    }
}

// Java: OptionsMapConversionTest.asMap_Basic
#[test]
fn as_map_basic() {
    let foo = Options::<FooOptions>::get_defaults();
    let map = (&foo as &dyn OptionsBase).as_map();
    assert_eq!(map.len(), 1);
    assert_eq!(map["foo"], Some(Value::new(false)));
}

// Java: OptionsMapConversionTest.asMap_Inheritance
#[test]
#[ignore = "needs options-class inheritance (options! has no `extends`; subclasses are \
            transcribed flat)"]
fn as_map_inheritance() {}

options! {
    /// Dummy options class for checking alphabetizing. Field name order differs from option
    /// name order.
    pub struct AlphaOptions {
        #[option(
            name = "c",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub v: i32,
        #[option(
            name = "d",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub w: i32,
        #[option(
            name = "a",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub x: i32,
        #[option(
            name = "e",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub y: i32,
        #[option(
            name = "b",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub z: i32,
    }
}

// Java: OptionsMapConversionTest.asMap_AlphabeticalOrder
#[test]
fn as_map_alphabetical_order() {
    let alpha = Options::<AlphaOptions>::get_defaults();
    let map = (&alpha as &dyn OptionsBase).as_map();
    let entries: Vec<(String, Option<Value>)> = map.into_iter().collect();
    let expected: Vec<(String, Option<Value>)> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|k| (k.to_string(), Some(Value::new(0i32))))
        .collect();
    assert_eq!(entries, expected);
}
