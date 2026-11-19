//! Port of `AttributeValuesAdapterTest`.

use blaze_bzlmod_model::gson::{JsonWriter, parse_lenient};
use blaze_bzlmod_model::repo_spec::STRING_ESCAPE_SEQUENCE;
use blaze_bzlmod_model::{AttributeValue, AttributeValues, AttributeValuesAdapter};
use blaze_cmdline::Label;

fn s(v: &str) -> AttributeValue {
    AttributeValue::string(v)
}

#[test]
fn test_attribute_values_adapter() {
    let l1 = Label::parse_canonical_unchecked("@//foo:bar");
    let l2 = Label::parse_canonical_unchecked("@//foo:tar");
    let e = STRING_ESCAPE_SEQUENCE;
    let built = AttributeValues::create([
        ("Integer", AttributeValue::Int(56)),
        ("Boolean", AttributeValue::Bool(false)),
        ("String", s("Hello String")),
        ("StringWithAngleBrackets", s("<Hello>")),
        (
            "LabelLikeString",
            AttributeValue::List(vec![s("@//foo:bar"), s(":baz"), s("@@//baz:quz")]),
        ),
        (
            "StringsWithEscapeSequence",
            AttributeValue::List(vec![
                s(&format!("@@//foo:bar{e}")),
                s(&format!("{e}@@//foo:bar")),
                s(&format!("{e}@@//foo:bar{e}")),
                s(&format!("{e}{e}@@//foo:bar{e}{e}")),
            ]),
        ),
        ("Label", AttributeValue::Label(l1.clone())),
        (
            "ListOfInts",
            AttributeValue::List(vec![AttributeValue::Int(1), AttributeValue::Int(2)]),
        ),
        (
            "ListOfLabels",
            AttributeValue::List(vec![
                AttributeValue::Label(l1.clone()),
                AttributeValue::Label(l2.clone()),
            ]),
        ),
        (
            "ListOfStrings",
            AttributeValue::List(vec![s("Hello"), s("There!")]),
        ),
        (
            "DictOfLabel-String",
            AttributeValue::Dict(vec![
                (AttributeValue::Label(l1), s("Label#1")),
                (AttributeValue::Label(l2), s("Label#2")),
            ]),
        ),
    ]);
    let mut writer = JsonWriter::new(false);
    AttributeValuesAdapter::write(&mut writer, &built);
    let json_string = writer.into_string();
    let attribute_values =
        AttributeValuesAdapter::read(&parse_lenient(&json_string).unwrap().unwrap()).unwrap();

    // The JSON string does not contain any escaped angle brackets.
    assert!(!json_string.contains("\\u003c"));
    // "Hello String" is preserved as is.
    assert!(json_string.contains(":\"Hello String\""));
    assert_eq!(attribute_values, built);
}

#[test]
#[ignore = "needs Starlark values: Tuple is normalized to a list by the Starlark bridge, \
            AttributeValue has no tuple kind"]
fn test_tuple() {}

/// Not in the Java test class. Internal strings (`é`, `🐷`, the byte FF, a label-like string)
/// are written as Unicode JSON (`internalToUnicode`) and read back as internal strings
/// (`unicodeToInternal`). The JSON forms are those of the oracle's lockfile
/// (`tests/data/bytes-B1.MODULE.bazel.lock`, attributes of `ext_repo`).
#[test]
fn test_attribute_values_adapter_non_ascii() {
    let internal = |t: &str| blaze_util::string_encoding::latin1_decode(t.as_bytes()).into_owned();
    let e = internal("é");
    let pig = internal("🐷");
    let built = AttributeValues::create([
        ("s", s(&format!("{e}{pig}"))),
        (
            "l",
            AttributeValue::List(vec![s(&e), s(&pig), s(&format!("@@not_a_label_{e}"))]),
        ),
        ("d", AttributeValue::Dict(vec![(s(&e), s(&pig))])),
        ("x", s("\u{ff}")),
    ]);
    let mut writer = JsonWriter::new(false);
    AttributeValuesAdapter::write(&mut writer, &built);
    let json_string = writer.into_string();
    assert_eq!(
        json_string,
        "{\"s\":\"é🐷\",\"l\":[\"é\",\"🐷\",\"'@@not_a_label_é'\"],\"d\":{\"é\":\"🐷\"},\"x\":\"\u{fffd}\"}"
    );
    let read =
        AttributeValuesAdapter::read(&parse_lenient(&json_string).unwrap().unwrap()).unwrap();
    assert_eq!(read.get("s"), built.get("s"));
    assert_eq!(read.get("l"), built.get("l"));
    assert_eq!(read.get("d"), built.get("d"));
    // FF is not UTF-8: it was written as U+FFFD and reads back as that char's UTF-8 bytes.
    assert_eq!(read.get("x"), Some(&s("\u{ef}\u{bf}\u{bd}")));
}
