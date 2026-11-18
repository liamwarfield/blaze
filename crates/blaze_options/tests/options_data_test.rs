//! Port of src/test/java/com/google/devtools/common/options/OptionsDataTest.java.

use blaze_options::definition::OptionsClassRef;
use blaze_options::exception::DUPLICATE_OPTION_DECLARATION_EXCEPTION;
use blaze_options::{ConstructionException, OptionsClass, OptionsData, Void, options};

fn construct(classes: &[OptionsClassRef]) -> Result<OptionsData, ConstructionException> {
    OptionsData::from(
        classes, /* allow_duplicates_parsing_equivalently= */ false,
    )
}

/// `assertThat(e).hasCauseThat().isInstanceOf(DuplicateOptionDeclarationException.class)` and
/// `hasMessageThat().contains(message)`.
fn assert_duplicate(result: Result<OptionsData, ConstructionException>, message: &str) {
    let e = match result {
        Ok(_) => panic!("expected a ConstructionException containing {:?}", message),
        Err(e) => e,
    };
    assert!(e.get_cause_message().is_some());
    assert!(
        e.get_message()
            .starts_with(&format!("{}: ", DUPLICATE_OPTION_DECLARATION_EXCEPTION)),
        "{}",
        e
    );
    assert!(e.get_message().contains(message), "{}", e);
}

options! {
    /// Dummy options class.
    pub struct ExampleNameConflictOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "1",
        )]
        pub foo: i32,

        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "I should conflict with foo",
        )]
        pub another_foo: String,
    }
}

// Java: OptionsDataTest.testNameConflictInSingleClass
#[test]
fn test_name_conflict_in_single_class() {
    assert_duplicate(
        construct(&[ExampleNameConflictOptions::class()]),
        "Duplicate option name, due to option name collision: --foo",
    );
}

options! {
    /// Dummy options class.
    pub struct ExampleIntegerFooOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "5",
        )]
        pub foo: i32,
    }
}

options! {
    /// Dummy options class.
    pub struct ExampleBooleanFooOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub foo: bool,
    }
}

// Java: OptionsDataTest.testNameConflictInTwoClasses
#[test]
fn test_name_conflict_in_two_classes() {
    assert_duplicate(
        construct(&[
            ExampleIntegerFooOptions::class(),
            ExampleBooleanFooOptions::class(),
        ]),
        "Duplicate option name, due to option name collision: --foo",
    );
}

options! {
    /// Dummy options class.
    pub struct ExamplePrefixedFooOptions {
        #[option(
            name = "nofoo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub no_foo: bool,
    }
}

// Java: OptionsDataTest.testBooleanPrefixNameConflict
#[test]
fn test_boolean_prefix_name_conflict() {
    // Try the same test in both orders, the parser should fail if the overlapping flag is defined
    // before or after the boolean flag introduces the alias.
    assert_duplicate(
        construct(&[
            ExampleBooleanFooOptions::class(),
            ExamplePrefixedFooOptions::class(),
        ]),
        "Duplicate option name, due to option --nofoo, it conflicts with a negating alias for boolean flag --foo",
    );
    assert_duplicate(
        construct(&[
            ExamplePrefixedFooOptions::class(),
            ExampleBooleanFooOptions::class(),
        ]),
        "Duplicate option name, due to boolean option alias: --nofoo",
    );
}

options! {
    /// Dummy options class.
    pub struct ExampleBarWasNamedFooOption {
        #[option(
            name = "bar",
            old_name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub bar: bool,
    }
}

// Java: OptionsDataTest.testBooleanAliasWithOldNameConflict
#[test]
fn test_boolean_alias_with_old_name_conflict() {
    assert_duplicate(
        construct(&[
            ExamplePrefixedFooOptions::class(),
            ExampleBarWasNamedFooOption::class(),
        ]),
        "Duplicate option name, due to boolean option alias: --nofoo",
    );
    assert_duplicate(
        construct(&[
            ExampleBarWasNamedFooOption::class(),
            ExamplePrefixedFooOptions::class(),
        ]),
        "Duplicate option name, due to option --nofoo, it conflicts with a negating alias for boolean flag --foo",
    );
}

options! {
    /// Dummy options class.
    pub struct ExampleBarWasNamedNoFooOption {
        #[option(
            name = "bar",
            old_name = "nofoo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub bar: bool,
    }
}

// Java: OptionsDataTest.testBooleanWithOldNameAsAliasOfBooleanConflict
#[test]
fn test_boolean_with_old_name_as_alias_of_boolean_conflict() {
    assert_duplicate(
        construct(&[
            ExampleBooleanFooOptions::class(),
            ExampleBarWasNamedNoFooOption::class(),
        ]),
        "Duplicate option name, due to old option name --nofoo, it conflicts with a negating alias for boolean flag --foo",
    );
    assert_duplicate(
        construct(&[
            ExampleBarWasNamedNoFooOption::class(),
            ExampleBooleanFooOptions::class(),
        ]),
        "Duplicate option name, due to boolean option alias: --nofoo",
    );
}

options! {
    /// Dummy options class.
    pub struct ExampleFooBooleanConflictsWithOwnOldName {
        #[option(
            name = "nofoo",
            old_name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "false",
        )]
        pub foo: bool,
    }
}

// Java: OptionsDataTest.testSelfConflictBooleanAliases
#[test]
fn test_self_conflict_boolean_aliases() {
    assert_duplicate(
        construct(&[ExampleFooBooleanConflictsWithOwnOldName::class()]),
        "Duplicate option name, due to boolean option alias: --nofoo",
    );
}

options! {
    /// Dummy options class.
    pub struct OldNameToCanonicalNameConflictExample {
        #[option(
            name = "new_name",
            old_name = "old_name",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "defaultValue",
        )]
        pub flag1: String,

        #[option(
            name = "old_name",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "defaultValue",
        )]
        pub flag2: String,
    }
}

// Java: OptionsDataTest.testOldNameToCanonicalNameConflict
#[test]
fn test_old_name_to_canonical_name_conflict() {
    assert_duplicate(
        construct(&[OldNameToCanonicalNameConflictExample::class()]),
        "Duplicate option name, due to option name collision with another option's old name: --old_name",
    );
}

options! {
    /// Dummy options class.
    pub struct OldNameConflictExample {
        #[option(
            name = "new_name",
            old_name = "old_name",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "defaultValue",
        )]
        pub flag1: String,

        #[option(
            name = "another_name",
            old_name = "old_name",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "defaultValue",
        )]
        pub flag2: String,
    }
}

// Java: OptionsDataTest.testOldNameToOldNameConflict
#[test]
fn test_old_name_to_old_name_conflict() {
    assert_duplicate(
        construct(&[OldNameConflictExample::class()]),
        "Duplicate option name, due to old option name collision with another old option name: --old_name",
    );
}

options! {
    /// Dummy options class.
    pub struct FieldNamesDifferOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub a_foo: i32,

        #[option(
            name = "bar",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub b_bar: i32,

        #[option(
            name = "baz",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub c_baz: i32,

        #[option(
            name = "qux",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub d_qux: i32,
    }
}

options! {
    /// Dummy options class.
    pub struct EndOfAlphabetOptions {
        #[option(
            name = "X",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub x: i32,

        #[option(
            name = "Y",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub y: i32,
    }
}

options! {
    /// Dummy options class.
    pub struct ReverseOrderedOptions {
        #[option(
            name = "C",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub c: i32,

        #[option(
            name = "B",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub b: i32,

        #[option(
            name = "A",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "0",
        )]
        pub a: i32,
    }
}

fn three() -> [OptionsClassRef; 3] {
    [
        FieldNamesDifferOptions::class(),
        EndOfAlphabetOptions::class(),
        ReverseOrderedOptions::class(),
    ]
}

// Java: OptionsDataTest.optionsClassesIsOrdered
#[test]
fn options_classes_is_ordered() {
    let data = construct(&three()).unwrap();
    assert_eq!(data.get_options_classes(), three());
}

// Java: OptionsDataTest.getAllNamedFieldsIsOrdered
#[test]
fn get_all_named_fields_is_ordered() {
    let data = construct(&three()).unwrap();
    let names: Vec<&str> = data.get_all_option_definitions().map(|(k, _)| k).collect();
    assert_eq!(names, ["bar", "baz", "foo", "qux", "X", "Y", "A", "B", "C"]);
}

fn get_option_names(class: OptionsClassRef) -> Vec<&'static str> {
    class.definitions().map(|d| d.get_option_name()).collect()
}

// Java: OptionsDataTest.getFieldsForClassIsOrdered
#[test]
fn get_fields_for_class_is_ordered() {
    assert_eq!(
        get_option_names(FieldNamesDifferOptions::class()),
        ["bar", "baz", "foo", "qux"]
    );
    assert_eq!(get_option_names(EndOfAlphabetOptions::class()), ["X", "Y"]);
    assert_eq!(
        get_option_names(ReverseOrderedOptions::class()),
        ["A", "B", "C"]
    );
}

// Java: OptionsDataTest.optionsDefinitionsAreSharedBetweenOptionsBases
#[test]
fn options_definitions_are_shared_between_options_bases() {
    let option_definitions: Vec<_> = three().iter().flat_map(|c| c.definitions()).collect();
    let contains_same_object = |from_data: &[&'static blaze_options::OptionDefinition]| {
        for d in &option_definitions {
            assert!(from_data.iter().any(|f| std::ptr::eq(*f, *d)));
        }
    };
    let data = construct(&three()).unwrap();
    let from_data: Vec<_> = data.get_all_option_definitions().map(|(_, d)| d).collect();
    contains_same_object(&from_data);

    let mut grouped = Vec::new();
    for c in three() {
        let d = construct(&[c]).unwrap();
        grouped.extend(d.get_all_option_definitions().map(|(_, d)| d));
    }
    contains_same_object(&grouped);
}

options! {
    /// Dummy options class.
    pub struct ValidExpansionOptions {
        #[option(
            name = "foo",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "1",
        )]
        pub foo: i32,

        #[option(
            name = "bar",
            documentation_category = UNCATEGORIZED,
            effect_tags = [NO_OP],
            default_value = "null",
            expansion = ["--foo=42"],
        )]
        pub bar: Void,
    }
}

// Java: OptionsDataTest.staticExpansionOptionsCanBeVoidType
#[test]
fn static_expansion_options_can_be_void_type() {
    construct(&[ValidExpansionOptions::class()]).unwrap();
}
