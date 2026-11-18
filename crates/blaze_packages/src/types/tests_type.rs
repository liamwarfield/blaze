//! Port of `src/test/java/com/google/devtools/build/lib/packages/TypeTest.java`.

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::repository_mapping::EMPTY;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::types::AttrValue;
use crate::types::LabelConverter;
use crate::types::License;
use crate::types::TriState;
use crate::types::Type;
use crate::types::test_util::eval_expr;
use crate::types::test_util::with_eval;

fn label_converter() -> LabelConverter {
    LabelConverter::new(
        &PackageIdentifier::create_in_main_repo_str("quux"),
        EMPTY.clone(),
    )
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

// Java: TypeTest.collectLabels
fn collect_labels(type_: Type, value: &AttrValue) -> Vec<Label> {
    let mut result = Vec::new();
    type_.visit_labels(
        &mut |label, _| result.push(label.clone()),
        type_.cast(value),
        None,
    );
    result
}

fn convert_err(type_: Type, code: &str, what: Option<&str>, lc: Option<&LabelConverter>) -> String {
    let what: Option<&dyn std::fmt::Display> = what.as_ref().map(|w| w as &dyn std::fmt::Display);
    with_eval(|eval| {
        let x = eval_expr(eval, code);
        type_
            .convert(x, what, lc)
            .expect_err("expected a ConversionException")
            .get_message()
            .to_owned()
    })
}

fn convert_ok(type_: Type, code: &str, lc: Option<&LabelConverter>) -> AttrValue {
    with_eval(|eval| {
        let x = eval_expr(eval, code);
        type_.convert(x, None, lc).unwrap()
    })
}

#[test]
fn test_integer() {
    let x = convert_ok(Type::Integer, "3", None);
    assert_eq!(x, AttrValue::Int(3));
    assert!(collect_labels(Type::Integer, &x).is_empty());
    // INTEGER rule attributes must be in signed 32-bit value range.
    let e = convert_err(Type::Integer, "111111111 * 111111111", Some("param"), None);
    assert!(
        e.contains("for param, got 12345678987654321, want value in signed 32-bit range"),
        "{e}"
    );
    // Ensure that the range of INTEGER.concat is int32.
    assert_eq!(
        Type::Integer.concat(vec![AttrValue::Int(0x7fffffff), AttrValue::Int(1)]),
        Some(AttrValue::Int(i32::MIN))
    );
}

#[test]
fn test_non_integer() {
    assert_eq!(
        convert_err(Type::Integer, "'foo'", None, None),
        "expected value of type 'int', but got \"foo\" (string)"
    );
}

// Ensure that types are reported correctly.
#[test]
fn test_type_error_message() {
    assert_eq!(
        convert_err(Type::StringList, "'[(1,2), 3, 4]'", Some("myexpr"), None),
        "expected value of type 'list(string)' for myexpr, but got \"[(1,2), 3, 4]\" (string)"
    );
}

#[test]
fn test_string() {
    let s = convert_ok(Type::String, "'foo'", None);
    assert_eq!(s, AttrValue::String("foo".to_owned()));
    assert!(collect_labels(Type::String, &s).is_empty());
}

#[test]
fn test_non_string() {
    assert_eq!(
        convert_err(Type::String, "3", None, None),
        "expected value of type 'string', but got 3 (int)"
    );
}

#[test]
fn test_boolean() {
    assert_eq!(convert_ok(Type::Boolean, "1", None), AttrValue::Bool(true));
    assert_eq!(convert_ok(Type::Boolean, "0", None), AttrValue::Bool(false));
    assert_eq!(
        convert_ok(Type::Boolean, "True", None),
        AttrValue::Bool(true)
    );
    assert_eq!(
        convert_ok(Type::Boolean, "False", None),
        AttrValue::Bool(false)
    );
    assert!(collect_labels(Type::Boolean, &AttrValue::Bool(true)).is_empty());
}

#[test]
fn test_non_boolean() {
    assert_eq!(
        convert_err(Type::Boolean, "'unexpected'", None, None),
        "expected one of [False, True, 0, 1], but got \"unexpected\" (string)"
    );
    // Integers other than [0, 1] should fail.
    assert_eq!(
        convert_err(Type::Boolean, "2", None, None),
        "expected one of [False, True, 0, 1], but got 2 (int)"
    );
    assert_eq!(
        convert_err(Type::Boolean, "-1", None, None),
        "expected one of [False, True, 0, 1], but got -1 (int)"
    );
}

#[test]
fn test_tri_state() {
    assert_eq!(
        convert_ok(Type::Tristate, "1", None),
        AttrValue::TriState(TriState::Yes)
    );
    assert_eq!(
        convert_ok(Type::Tristate, "0", None),
        AttrValue::TriState(TriState::No)
    );
    assert_eq!(
        convert_ok(Type::Tristate, "-1", None),
        AttrValue::TriState(TriState::Auto)
    );
    // PORT: TriState objects are not Starlark values; converting them is not portable.
    assert!(collect_labels(Type::Tristate, &AttrValue::TriState(TriState::Yes)).is_empty());
    // deprecated:
    assert_eq!(
        convert_ok(Type::Tristate, "True", None),
        AttrValue::TriState(TriState::Yes)
    );
    assert_eq!(
        convert_ok(Type::Tristate, "False", None),
        AttrValue::TriState(TriState::No)
    );
}

#[test]
fn test_tri_state_does_not_accept_arbitrary_integers() {
    for i in [2, 3, -5, -2, 20] {
        convert_err(Type::Tristate, &i.to_string(), None, None);
    }
}

#[test]
fn test_tri_state_does_not_accept_strings() {
    for entry in ["'bad'", "'true'", "'auto'", "'false'"] {
        convert_err(Type::Tristate, entry, None, None);
    }
}

#[test]
fn test_tag_conversion() {
    assert_eq!(
        Type::Boolean.to_tag_set(Some(&AttrValue::Bool(true)), "attribute"),
        IndexSet::from(["attribute".to_owned()])
    );
    assert_eq!(
        Type::Boolean.to_tag_set(Some(&AttrValue::Bool(false)), "attribute"),
        IndexSet::from(["noattribute".to_owned()])
    );
    assert_eq!(
        Type::String.to_tag_set(
            Some(&AttrValue::String("whiskey".to_owned())),
            "preferred_cocktail"
        ),
        IndexSet::from(["whiskey".to_owned()])
    );
    let cheeses = AttrValue::StringList(vec![
        "cheddar".to_owned(),
        "ementaler".to_owned(),
        "gruyere".to_owned(),
    ]);
    assert_eq!(
        Type::StringList.to_tag_set(Some(&cheeses), "cheeses"),
        IndexSet::from([
            "cheddar".to_owned(),
            "ementaler".to_owned(),
            "gruyere".to_owned()
        ])
    );
}

#[test]
fn test_illegal_tag_conversion_by_type() {
    let r = std::panic::catch_unwind(|| {
        Type::Tristate.to_tag_set(Some(&AttrValue::TriState(TriState::Auto)), "some_tristate")
    });
    assert!(r.is_err());
    let r = std::panic::catch_unwind(|| {
        Type::License.to_tag_set(
            Some(&AttrValue::License(License::no_license())),
            "output_license",
        )
    });
    assert!(r.is_err());
}

#[test]
fn test_illegal_tag_convers_ion_from_null_on_supported_type() {
    let r = std::panic::catch_unwind(|| Type::Boolean.to_tag_set(None, "a_boolean"));
    assert!(r.is_err());
}

#[test]
fn test_label() {
    let lc = label_converter();
    let l = label("//foo:bar");
    let converted = convert_ok(Type::Label, "'//foo:bar'", Some(&lc));
    assert_eq!(converted, AttrValue::Label(l.clone()));
    assert_eq!(collect_labels(Type::Label, &converted), vec![l]);
}

#[test]
fn test_nodep_label() {
    let lc = label_converter();
    let l = label("//foo:bar");
    let converted = convert_ok(Type::NodepLabel, "'//foo:bar'", Some(&lc));
    assert_eq!(converted, AttrValue::Label(l.clone()));
    assert_eq!(collect_labels(Type::NodepLabel, &converted), vec![l]);
}

#[test]
fn test_relative_label() {
    let lc = label_converter();
    assert_eq!(
        convert_ok(Type::Label, "':wiz'", Some(&lc)),
        AttrValue::Label(label("//quux:wiz"))
    );
    assert_eq!(
        convert_ok(Type::Label, "'wiz'", Some(&lc)),
        AttrValue::Label(label("//quux:wiz"))
    );
    convert_err(Type::Label, "'wiz'", None, None);
}

#[test]
fn test_invalid_label() {
    let lc = label_converter();
    let e = convert_err(Type::Label, "'not//a label'", None, Some(&lc));
    assert!(e.contains("'not//a label'"), "{e}");
}

#[test]
fn test_non_label() {
    assert_eq!(
        convert_err(Type::Label, "3", None, None),
        "expected value of type 'string', but got 3 (int)"
    );
}

#[test]
fn test_string_list() {
    let converted = convert_ok(Type::StringList, "['foo', 'bar', 'wiz']", None);
    assert_eq!(
        converted,
        AttrValue::StringList(vec!["foo".to_owned(), "bar".to_owned(), "wiz".to_owned()])
    );
    assert!(collect_labels(Type::StringList, &converted).is_empty());
}

#[test]
fn test_string_dict() {
    let converted = convert_ok(Type::StringDict, "{'foo': 'bar', 'wiz': 'bang'}", None);
    let expected: IndexMap<String, String> = [("foo", "bar"), ("wiz", "bang")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    assert_eq!(converted, AttrValue::StringDict(expected));
    assert!(collect_labels(Type::StringDict, &converted).is_empty());
}

#[test]
fn test_string_dict_bad_elements() {
    assert_eq!(
        convert_err(
            Type::StringDict,
            "{'foo': ['bar', 'baz'], 'wiz': 'bang'}",
            None,
            None
        ),
        "expected value of type 'string' for dict value element, but got [\"bar\", \"baz\"] (list)"
    );
}

#[test]
fn test_non_string_list() {
    assert_eq!(
        convert_err(Type::StringList, "3", Some("blah"), None),
        "expected value of type 'list(string)' for blah, but got 3 (int)"
    );
}

#[test]
fn test_string_list_bad_elements() {
    assert_eq!(
        convert_err(
            Type::StringList,
            "['foo', 'bar', 1]",
            Some("argument quux"),
            None
        ),
        "expected value of type 'string' for element 2 of argument quux, but got 1 (int)"
    );
}

#[test]
#[ignore = "needs a depset constructor without the Blaze universe (blaze_starlark depset global)"]
fn test_list_depset_conversion() {}

#[test]
fn test_label_list() {
    let lc = label_converter();
    let converted = convert_ok(Type::LabelList, "['//foo:bar', ':wiz']", Some(&lc));
    let expected = vec![label("//foo:bar"), label("//quux:wiz")];
    assert_eq!(converted, AttrValue::LabelList(expected.clone()));
    assert_eq!(collect_labels(Type::LabelList, &converted), expected);
}

#[test]
fn test_non_label_list() {
    let lc = label_converter();
    assert_eq!(
        convert_err(Type::LabelList, "3", Some("foo"), Some(&lc)),
        "expected value of type 'list(label)' for foo, but got 3 (int)"
    );
}

#[test]
fn test_label_list_bad_elements() {
    let lc = label_converter();
    assert_eq!(
        convert_err(Type::LabelList, "['//foo:bar', 2, 'foo']", None, Some(&lc)),
        "expected value of type 'string' for element 1 of null, but got 2 (int)"
    );
}

#[test]
fn test_label_list_syntax_error() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelList,
            "['//foo:bar/..', 'foo']",
            Some("myexpr"),
            Some(&lc)
        ),
        "invalid label '//foo:bar/..' in element 0 of myexpr: invalid target name 'bar/..': \
         target names may not contain up-level references '..'"
    );
}

fn string_list_dict(entries: &[(&str, &[&str])]) -> AttrValue {
    AttrValue::StringListDict(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect()))
            .collect(),
    )
}

#[test]
fn test_string_list_dict() {
    let lc = label_converter();
    let converted = convert_ok(
        Type::StringListDict,
        "{'foo': ['foo', 'bar'], 'wiz': ['bang']}",
        Some(&lc),
    );
    assert_eq!(
        converted,
        string_list_dict(&[("foo", &["foo", "bar"]), ("wiz", &["bang"])])
    );
    assert!(collect_labels(Type::StringListDict, &converted).is_empty());
}

#[test]
fn test_string_list_dict_concat() {
    assert_eq!(
        Type::StringListDict.concat(vec![]),
        Some(string_list_dict(&[]))
    );
    let expected = string_list_dict(&[("foo", &["foo", "bar"]), ("wiz", &["bang"])]);
    assert_eq!(
        Type::StringListDict.concat(vec![expected.clone()]),
        Some(expected)
    );
    let map1 = string_list_dict(&[("foo", &["a", "b"]), ("bar", &["c", "d"])]);
    let map2 = string_list_dict(&[("bar", &["x", "y"]), ("baz", &["z"])]);
    let expected_after_concat =
        string_list_dict(&[("foo", &["a", "b"]), ("bar", &["x", "y"]), ("baz", &["z"])]);
    let concat = Type::StringListDict.concat(vec![map1, map2]).unwrap();
    assert_eq!(concat, expected_after_concat);
    // Order: the position of the first occurrence.
    let AttrValue::StringListDict(m) = concat else {
        unreachable!()
    };
    assert_eq!(
        m.keys().cloned().collect::<Vec<_>>(),
        vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()]
    );
}

#[test]
fn test_string_list_dict_bad_first_element() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::StringListDict,
            "{2: ['foo', 'bar'], 'wiz': ['bang']}",
            None,
            Some(&lc)
        ),
        "expected value of type 'string' for dict key element, but got 2 (int)"
    );
}

#[test]
fn test_string_list_dict_bad_second_element() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::StringListDict,
            "{'foo': 'bar', 'wiz': ['bang']}",
            None,
            Some(&lc)
        ),
        "expected value of type 'list(string)' for dict value element, but got \"bar\" (string)"
    );
}

#[test]
fn test_string_list_dict_bad_elements1() {
    assert_eq!(
        convert_err(
            Type::StringListDict,
            "{('foo',): ('bang',), 'wiz': ('bang',)}",
            None,
            None
        ),
        "expected value of type 'string' for dict key element, but got (\"foo\",) (tuple)"
    );
}

#[test]
fn test_string_dict_throws_conversion_exception() {
    assert_eq!(
        convert_err(Type::StringDict, "'some string'", None, None),
        "expected value of type 'dict(string, string)', but got \"some string\" (string)"
    );
}
