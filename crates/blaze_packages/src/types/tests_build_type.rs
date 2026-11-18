//! Port of `src/test/java/com/google/devtools/build/lib/packages/BuildTypeTest.java`.

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::repository_mapping::EMPTY;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::eval::Evaluator;
use starlark::values::Value;

use crate::attribute::attr;
use crate::types::AttrValue;
use crate::types::BuildTypeSelector;
use crate::types::BuildTypeSelectorList;
use crate::types::DEFAULT_CONDITION_KEY;
use crate::types::FileTypeSet;
use crate::types::LabelConverter;
use crate::types::SelectorList;
use crate::types::SelectorValue;
use crate::types::Type;
use crate::types::build_type;
use crate::types::test_util::eval_expr;
use crate::types::test_util::with_eval;

fn label_converter() -> LabelConverter {
    LabelConverter::new(
        &PackageIdentifier::create_in_main_repo_str("quux"),
        EMPTY.clone(),
    )
}

fn converter_for(pkg: &str) -> LabelConverter {
    LabelConverter::new(&PackageIdentifier::parse(pkg).unwrap(), EMPTY.clone())
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

fn labels(s: &[&str]) -> Vec<Label> {
    s.iter().map(|s| label(s)).collect()
}

/// A `SelectorValue` (Java `new SelectorValue(map, noMatchError)`) with Starlark-expression values.
fn selector_value<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    entries: &[(&str, &str)],
    no_match_error: &str,
) -> Value<'v> {
    let dictionary: Vec<(Value<'v>, Value<'v>)> = entries
        .iter()
        .map(|(k, v)| (eval.heap().alloc(*k), eval_expr(eval, v)))
        .collect();
    eval.heap()
        .alloc(SelectorValue::new(dictionary, no_match_error.to_owned()))
}

fn selector_list<'v>(eval: &mut Evaluator<'v, '_, '_>, values: &[Value<'v>]) -> Value<'v> {
    let l = SelectorList::of(values).unwrap();
    eval.heap().alloc(l)
}

fn selector_dictionary<'v>(selector_value: Value<'v>) -> Vec<(Value<'v>, Value<'v>)> {
    SelectorValue::from_value(selector_value)
        .unwrap()
        .get_dictionary()
        .collect()
}

// Java: BuildTypeTest.collectLabels
fn collect_labels(type_: Type, value: &AttrValue) -> Vec<Label> {
    let mut result = Vec::new();
    type_.visit_labels(&mut |label, _| result.push(label.clone()), value, None);
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

#[test]
fn test_keeps_dict_ordering() {
    let lc = label_converter();
    let converted = with_eval(|eval| {
        let x = eval_expr(
            eval,
            "{'c': '//c', 'b': '//b', 'a': '//a', 'f': '//f', 'e': '//e', 'd': '//d'}",
        );
        Type::LabelDictUnary.convert(x, None, Some(&lc)).unwrap()
    });
    let AttrValue::LabelDictUnary(m) = converted else {
        panic!()
    };
    assert_eq!(
        m.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["c", "b", "a", "f", "e", "d"]
    );
}

#[test]
fn test_label_keyed_string_dict_converts_to_map_from_label_to_string() {
    let converter = converter_for("//current/package");
    let converted = with_eval(|eval| {
        let x = eval_expr(
            eval,
            "{'//absolute:label': 'absolute value', ':relative': 'theory of relativity', \
             'nocolon': 'colonial times', '//current/package:explicit': 'explicit content'}",
        );
        // Add a key that already is a Label.
        let dict = starlark::values::dict::DictRef::from_value(x).unwrap();
        let mut entries: Vec<(Value, Value)> = dict.iter().collect();
        entries.push((
            eval.heap()
                .alloc(LabelValue(label("//i/was/already/a/label"))),
            eval.heap().alloc("and that's okay"),
        ));
        let x = eval
            .heap()
            .alloc(starlark::values::dict::AllocDict(entries));
        Type::LabelKeyedStringDict
            .convert(x, None, Some(&converter))
            .unwrap()
    });
    let expected: IndexMap<Label, String> = [
        ("//absolute:label", "absolute value"),
        ("//current/package:relative", "theory of relativity"),
        ("//current/package:nocolon", "colonial times"),
        ("//current/package:explicit", "explicit content"),
        ("//i/was/already/a/label", "and that's okay"),
    ]
    .into_iter()
    .map(|(k, v)| (label(k), v.to_owned()))
    .collect();
    assert_eq!(converted, AttrValue::LabelKeyedStringDict(expected));
}

#[test]
fn test_label_keyed_string_dict_converting_string_should_fail() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "'//actually/a:label'",
            None,
            Some(&lc)
        ),
        "expected value of type 'dict(label, string)', but got \"//actually/a:label\" (string)"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_list_should_fail() {
    let lc = label_converter();
    // PORT: Java passes a Java ImmutableList, whose Starlark.type is "List"; a Starlark list is
    // "list".
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "['//actually/a:label']",
            None,
            Some(&lc)
        ),
        "expected value of type 'dict(label, string)', but got [\"//actually/a:label\"] (list)"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_map_with_non_string_key_should_fail() {
    let lc = label_converter();
    assert_eq!(
        convert_err(Type::LabelKeyedStringDict, "{1: 'OK'}", None, Some(&lc)),
        "expected value of type 'string' for dict key element, but got 1 (int)"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_map_with_non_string_value_should_fail() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "{'//actually/a:label': 3}",
            None,
            Some(&lc)
        ),
        "expected value of type 'string' for dict value element, but got 3 (int)"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_map_with_invalid_label_key_should_fail() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "{'//uplevel/references/are:../../forbidden': 'OK'}",
            None,
            Some(&lc)
        ),
        "invalid label '//uplevel/references/are:../../forbidden' in dict key element: invalid \
         target name '../../forbidden': target names may not contain up-level references '..'"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_map_with_multiple_equivalent_keys_should_fail() {
    let converter = converter_for("//current/package");
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "{':reference': 'value1', '//current/package:reference': 'value2'}",
            None,
            Some(&converter)
        ),
        "duplicate labels: //current/package:reference (as [\":reference\", \
         \"//current/package:reference\"])"
    );
}

#[test]
fn test_label_keyed_string_dict_converting_map_with_multiple_sets_of_equivalent_keys_should_fail() {
    let converter = converter_for("//current/rule");
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "{':rule': 'first set', '//current/rule:rule': 'also first set', \
             '//other/package:package': 'interrupting rule', \
             '//other/package': \"interrupting rule's friend\", \
             '//current/rule': 'part of first set but non-contiguous in iteration order', \
             '//not/involved/in/any:collisions': 'same value', \
             '//also/not/involved/in/any:collisions': 'same value'}",
            None,
            Some(&converter)
        ),
        "duplicate labels: //current/rule:rule (as [\":rule\", \"//current/rule:rule\", \
         \"//current/rule\"]), //other/package:package (as [\"//other/package:package\", \
         \"//other/package\"])"
    );
}

#[test]
fn test_label_keyed_string_dict_error_converting_map_with_multiple_equivalent_keys_includes_context()
 {
    let converter = converter_for("//current/package");
    assert_eq!(
        convert_err(
            Type::LabelKeyedStringDict,
            "{':reference': 'value1', '//current/package:reference': 'value2'}",
            Some("flag map"),
            Some(&converter)
        ),
        "duplicate labels in flag map: //current/package:reference (as [\":reference\", \
         \"//current/package:reference\"])"
    );
}

#[test]
fn test_label_keyed_string_dict_collect_labels() {
    let expected = labels(&[
        "//absolute:label",
        "//current/package:relative",
        "//current/package:nocolon",
        "//current/package:explicit",
        "//i/was/already/a/label",
    ]);
    let input: IndexMap<Label, String> = expected
        .iter()
        .map(|l| (l.clone(), "value".to_owned()))
        .collect();
    assert_eq!(
        collect_labels(
            Type::LabelKeyedStringDict,
            &AttrValue::LabelKeyedStringDict(input)
        ),
        expected
    );
}

fn selector(entries: &[(&str, &str)], type_: Type) -> Result<BuildTypeSelector, String> {
    let lc = label_converter();
    with_eval(|eval| {
        let x: Vec<(Value, Value)> = entries
            .iter()
            .map(|(k, v)| (eval.heap().alloc(*k), eval_expr(eval, v)))
            .collect();
        BuildTypeSelector::from_starlark(&x, None, Some(&lc), type_, "")
            .map_err(|e| e.get_message().to_owned())
    })
}

/// Tests basic Selector functionality.
#[test]
fn test_selector() {
    let selector = selector(
        &[
            ("//conditions:a", "'//a:a'"),
            ("//conditions:b", "'//b:b'"),
            (DEFAULT_CONDITION_KEY, "'//d:d'"),
        ],
        Type::Label,
    )
    .unwrap();
    assert_eq!(selector.get_original_type(), Type::Label);
    let expected: IndexMap<Label, Option<AttrValue>> = [
        ("//conditions:a", "//a:a"),
        ("//conditions:b", "//b:b"),
        (DEFAULT_CONDITION_KEY, "//d:d"),
    ]
    .into_iter()
    .map(|(k, v)| (label(k), Some(AttrValue::Label(label(v)))))
    .collect();
    assert_eq!(selector.map_copy(), expected);
}

/// Tests that creating a Selector over a mismatching native type triggers an exception.
#[test]
fn test_selector_wrong_type() {
    let e = selector(
        &[
            ("//conditions:a", "'not a/../label'"),
            ("//conditions:b", "'also not a/../label'"),
            (DEFAULT_CONDITION_KEY, "'whatever'"),
        ],
        Type::Label,
    )
    .unwrap_err();
    assert!(e.contains("invalid label 'not a/../label'"), "{e}");
}

/// Tests that non-label selector keys trigger an exception.
#[test]
fn test_selector_key_is_not_a_label() {
    let e = selector(
        &[
            ("not a/../label", "'//a:a'"),
            (DEFAULT_CONDITION_KEY, "'whatever'"),
        ],
        Type::Label,
    )
    .unwrap_err();
    assert!(e.contains("invalid label 'not a/../label'"), "{e}");
}

/// Tests that Selector correctly references its default value.
#[test]
fn test_selector_default() {
    let selector = selector(
        &[
            ("//conditions:a", "'//a:a'"),
            ("//conditions:b", "'//b:b'"),
            (DEFAULT_CONDITION_KEY, "'//d:d'"),
        ],
        Type::Label,
    )
    .unwrap();
    assert!(selector.has_default());
    assert_eq!(
        selector.get_default(),
        Some(&AttrValue::Label(label("//d:d")))
    );
}

#[test]
fn test_selector_default_null() {
    let selector = selector(
        &[
            ("//conditions:a", "'//a:a'"),
            (DEFAULT_CONDITION_KEY, "None"),
        ],
        Type::Label,
    )
    .unwrap();
    assert!(selector.has_default());
    assert!(!selector.is_unconditional());
    assert_eq!(selector.get_default(), None);
}

#[test]
fn test_selector_default_null_singleton() {
    let selector = selector(&[(DEFAULT_CONDITION_KEY, "None")], Type::Label).unwrap();
    assert!(selector.has_default());
    assert!(selector.is_unconditional());
    assert_eq!(selector.get_default(), None);
}

fn build_type_selector_list(
    selectors: &[(&[(&str, &str)], &str)],
    type_: Type,
) -> Result<BuildTypeSelectorList, String> {
    let lc = label_converter();
    with_eval(|eval| {
        let elements: Vec<Value> = selectors
            .iter()
            .map(|(entries, nme)| selector_value(eval, entries, nme))
            .collect();
        BuildTypeSelectorList::from_starlark(&elements, None, Some(&lc), type_)
            .map_err(|e| e.get_message().to_owned())
    })
}

#[test]
fn test_selector_list() {
    let selector_list = build_type_selector_list(
        &[
            (
                &[
                    ("//conditions:a", "['//a:a']"),
                    ("//conditions:b", "['//b:b']"),
                ],
                "",
            ),
            (
                &[
                    ("//conditions:c", "['//c:c']"),
                    ("//conditions:d", "['//d:d']"),
                ],
                "",
            ),
        ],
        Type::LabelList,
    )
    .unwrap();
    assert_eq!(selector_list.get_original_type(), Type::LabelList);
    assert_eq!(
        selector_list
            .get_key_labels()
            .into_iter()
            .collect::<Vec<_>>(),
        labels(&[
            "//conditions:a",
            "//conditions:b",
            "//conditions:c",
            "//conditions:d"
        ])
    );
    let selectors = selector_list.get_selectors();
    let map0: Vec<(Label, Option<AttrValue>)> = selectors[0].map_copy().into_iter().collect();
    assert_eq!(
        map0,
        vec![
            (
                label("//conditions:a"),
                Some(AttrValue::LabelList(labels(&["//a:a"])))
            ),
            (
                label("//conditions:b"),
                Some(AttrValue::LabelList(labels(&["//b:b"])))
            ),
        ]
    );
    let map1: Vec<(Label, Option<AttrValue>)> = selectors[1].map_copy().into_iter().collect();
    assert_eq!(
        map1,
        vec![
            (
                label("//conditions:c"),
                Some(AttrValue::LabelList(labels(&["//c:c"])))
            ),
            (
                label("//conditions:d"),
                Some(AttrValue::LabelList(labels(&["//d:d"])))
            ),
        ]
    );
}

#[test]
fn test_selector_dict() {
    let selector_list = build_type_selector_list(
        &[
            (
                &[
                    ("//conditions:a", "{'//a:a': 'a'}"),
                    ("//conditions:b", "{'//b:b': 'b'}"),
                ],
                "",
            ),
            (
                &[
                    ("//conditions:c", "{'//c:c': 'c'}"),
                    ("//conditions:d", "{'//d:d': 'd'}"),
                ],
                "",
            ),
        ],
        Type::LabelKeyedStringDict,
    )
    .unwrap();
    assert_eq!(
        selector_list.get_original_type(),
        Type::LabelKeyedStringDict
    );
    assert_eq!(selector_list.get_key_labels().len(), 4);
    let map0: Vec<(Label, Option<AttrValue>)> = selector_list.get_selectors()[0]
        .map_copy()
        .into_iter()
        .collect();
    let dict = |l: &str, v: &str| {
        Some(AttrValue::LabelKeyedStringDict(
            [(label(l), v.to_owned())].into_iter().collect(),
        ))
    };
    assert_eq!(
        map0,
        vec![
            (label("//conditions:a"), dict("//a:a", "a")),
            (label("//conditions:b"), dict("//b:b", "b")),
        ]
    );
}

#[test]
fn test_selector_list_mixed_types() {
    let e = build_type_selector_list(
        &[
            (&[("//conditions:a", "['//a:a']")], ""),
            (&[("//conditions:b", "'//b:b'")], ""),
        ],
        Type::LabelList,
    )
    .unwrap_err();
    assert!(e.contains("expected value of type 'list(label)'"), "{e}");
}

#[test]
fn test_selector_list_concatenate_selector_list() {
    with_eval(|eval| {
        let sv = selector_value(eval, &[("//conditions:a", "['//a:a']")], "");
        let sl = selector_list(eval, &[sv]);
        let list = eval_expr(eval, "['//a:a', '//b:b']");
        // Creating a SelectorList from a SelectorList and a list should work properly.
        let result = SelectorList::concat(sl, list).unwrap();
        assert_eq!(result.get_type(), "list");
    });
}

#[test]
fn test_selector_list_concatenate_selector_value() {
    with_eval(|eval| {
        let sv = selector_value(eval, &[("//conditions:a", "['//a:a']")], "");
        let list = eval_expr(eval, "['//a:a', '//b:b']");
        // Creating a SelectorList from a SelectorValue and a list should work properly.
        let result = SelectorList::concat(sv, list).unwrap();
        assert_eq!(result.get_type(), "list");
    });
}

#[test]
fn test_selector_list_concatenate_different_list_types() {
    with_eval(|eval| {
        // PORT: Java concatenates two Java List implementations; here a list and a tuple, both
        // Java Lists.
        let list = eval_expr(eval, "['//a:a', '//b:b']");
        let tuple = eval_expr(eval, "('//a:a',)");
        let result = SelectorList::concat(list, tuple).unwrap();
        assert_eq!(result.get_type(), "list");
    });
}

#[test]
fn test_selector_list_concatenate_invalid_type() {
    with_eval(|eval| {
        let list = eval_expr(eval, "['//a:a', '//b:b']");
        let s = eval.heap().alloc("A string");
        // Creating a SelectorList from a list and a non-list should fail.
        assert!(SelectorList::concat(list, s).is_err());
    });
}

/// Tests that selectable_convert returns either the native type or a selector on that type.
#[test]
fn selectable_convert_basic_usage() {
    let lc = label_converter();
    let expected_labels = labels(&["//a:a1", "//a:a2"]);
    with_eval(|eval| {
        let native_input = eval_expr(eval, "['//a:a1', '//a:a2']");
        let sv = selector_value(
            eval,
            &[
                ("//conditions:a", "['//a:a1', '//a:a2']"),
                (DEFAULT_CONDITION_KEY, "['//a:a1', '//a:a2']"),
            ],
            "",
        );
        let selectable_input = selector_list(eval, &[sv]);
        // Conversion to direct type:
        let converted =
            build_type::selectable_convert(Type::LabelList, native_input, None, Some(&lc), false)
                .unwrap();
        assert_eq!(
            converted,
            Some(AttrValue::LabelList(expected_labels.clone()))
        );
        // Conversion to selectable type:
        let converted = build_type::selectable_convert(
            Type::LabelList,
            selectable_input,
            None,
            Some(&lc),
            false,
        )
        .unwrap()
        .unwrap();
        let selector_list = converted.as_selector_list().unwrap();
        let map: Vec<(Label, Option<AttrValue>)> = selector_list.get_selectors()[0]
            .map_copy()
            .into_iter()
            .collect();
        assert_eq!(
            map,
            vec![
                (
                    label("//conditions:a"),
                    Some(AttrValue::LabelList(expected_labels.clone()))
                ),
                (
                    label(DEFAULT_CONDITION_KEY),
                    Some(AttrValue::LabelList(expected_labels.clone()))
                ),
            ]
        );
    });
}

/// selectable_convert with simplify_unconditional_selects.
#[test]
fn selectable_convert_simplifying_unconditionals() {
    let lc = label_converter();
    let label_a = label("//a");
    let label_x = label("//x");
    with_eval(|eval| {
        let value_a = eval_expr(eval, "['//a']");
        let unconditional_selector_x =
            selector_value(eval, &[(DEFAULT_CONDITION_KEY, "['//x']")], "");
        let conditional_selector_yz = selector_value(
            eval,
            &[
                ("//conditions:a", "['//y']"),
                (DEFAULT_CONDITION_KEY, "['//z']"),
            ],
            "",
        );
        // select({"//conditions:default": ["//x"]}) simplified to ["//x"]
        let input = selector_list(eval, &[unconditional_selector_x]);
        assert_eq!(
            build_type::selectable_convert(Type::LabelList, input, None, Some(&lc), true).unwrap(),
            Some(AttrValue::LabelList(vec![label_x.clone()]))
        );
        // ["//a"] + select({"//conditions:default": ["//x"]}) simplified to ["//a", "//x"]
        let input = selector_list(eval, &[value_a, unconditional_selector_x]);
        assert_eq!(
            build_type::selectable_convert(Type::LabelList, input, None, Some(&lc), true).unwrap(),
            Some(AttrValue::LabelList(vec![label_a.clone(), label_x.clone()]))
        );
        // ["//a"] + select({"//conditions:a": ["//y"], "//conditions:default": ["//z"]}) cannot
        // be simplified
        let input = selector_list(eval, &[value_a, conditional_selector_yz]);
        let unsimplified =
            build_type::selectable_convert(Type::LabelList, input, None, Some(&lc), true)
                .unwrap()
                .unwrap();
        let plain = build_type::selectable_convert(Type::LabelList, input, None, Some(&lc), false)
            .unwrap()
            .unwrap();
        let maps = |v: &AttrValue| {
            v.as_selector_list()
                .unwrap()
                .get_selectors()
                .iter()
                .map(|s| s.map_copy().into_iter().collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert_eq!(maps(&unsimplified), maps(&plain));
    });
}

const ALL_BUILD_TYPES: &[Type] = &[
    Type::Integer,
    Type::String,
    Type::StringNoIntern,
    Type::Boolean,
    Type::StringList,
    Type::StringSet,
    Type::IntegerList,
    Type::StringDict,
    Type::StringListDict,
    Type::Label,
    Type::LabelDictUnary,
    Type::LabelKeyedStringDict,
    Type::LabelList,
    Type::LabelListDict,
    Type::NodepLabel,
    Type::NodepLabelList,
    Type::DormantLabel,
    Type::DormantLabelList,
    Type::GenqueryScopeType,
    Type::GenqueryScopeTypeList,
    Type::License,
    Type::Output,
    Type::OutputList,
    Type::Tristate,
];

#[test]
fn selectable_convert_simplifying_unconditionals_handles_unconditional_none() {
    let lc = label_converter();
    with_eval(|eval| {
        let unconditional_selector_none =
            selector_value(eval, &[(DEFAULT_CONDITION_KEY, "None")], "");
        for type_ in ALL_BUILD_TYPES {
            let type_ = *type_;
            // select({"//conditions:default": None}) simplifies to the type's default value.
            let input = selector_list(eval, &[unconditional_selector_none]);
            assert_eq!(
                build_type::selectable_convert(type_, input, None, Some(&lc), true).unwrap(),
                type_.get_default_value(),
                "{type_}"
            );
            // select({"//conditions:default": None}) + select({"//conditions:default": None})
            // either simplifies to the type's non-null default value, or cleanly fails to concat.
            let input = selector_list(
                eval,
                &[unconditional_selector_none, unconditional_selector_none],
            );
            if type_.concat(vec![]).is_some() {
                let concatenation =
                    build_type::selectable_convert(type_, input, None, Some(&lc), true).unwrap();
                assert_eq!(concatenation, type_.get_default_value(), "{type_}");
                assert!(concatenation.is_some());
            } else {
                let e = build_type::selectable_convert(type_, input, None, Some(&lc), true)
                    .unwrap_err();
                assert!(
                    e.get_message()
                        .contains("doesn't support select concatenation"),
                    "{type_}: {}",
                    e.get_message()
                );
            }
        }
    });
}

#[test]
fn selectable_convert_simplifying_unconditionals_fails_cleanly_on_invalid_concatenation() {
    let lc = label_converter();
    with_eval(|eval| {
        let a = eval.heap().alloc("//a");
        let sv = selector_value(eval, &[("//conditions:default", "'//b'")], "");
        let input = selector_list(eval, &[a, sv]);
        let e =
            build_type::selectable_convert(Type::Label, input, None, Some(&lc), true).unwrap_err();
        assert!(
            e.get_message()
                .contains("type 'label' doesn't support select concatenation")
        );
    });
}

#[test]
fn test_copy_and_lift_starlark_list() {
    let lc = label_converter();
    let attribute = attr("attrName", Type::LabelList)
        .allowed_file_types(FileTypeSet::NoFile)
        .build();
    with_eval(|eval| {
        let starlark_list = eval_expr(eval, "['//a:a1', '//a:a2']");
        let converted = build_type::copy_and_lift_starlark_value(
            "ruleClass",
            &attribute,
            starlark_list,
            Some(&lc),
            eval.heap(),
        )
        .unwrap();
        assert_eq!(converted.get_type(), "list");
        assert_eq!(
            converted.to_repr(),
            "[Label(\"//a:a1\"), Label(\"//a:a2\")]"
        );
    });
}

#[test]
fn test_copy_and_lift_starlark_dict() {
    let lc = label_converter();
    let attribute = attr("attrName", Type::StringDict).build();
    with_eval(|eval| {
        let input_dict = eval_expr(eval, "{'a': 'b', 'c': 'd'}");
        let converted = build_type::copy_and_lift_starlark_value(
            "ruleClass",
            &attribute,
            input_dict,
            Some(&lc),
            eval.heap(),
        )
        .unwrap();
        assert_eq!(converted.get_type(), "dict");
        assert!(converted.equals(input_dict).unwrap());
        assert!(!converted.ptr_eq(input_dict));
    });
}

#[test]
fn test_copy_and_lift_selectable_starlark_value() {
    let lc = label_converter();
    let attribute = attr("attrName", Type::LabelList)
        .allowed_file_types(FileTypeSet::NoFile)
        .build();
    with_eval(|eval| {
        let sv = selector_value(
            eval,
            &[
                ("//conditions:a", "['//a:a1', '//a:a2']"),
                (DEFAULT_CONDITION_KEY, "['//a:a1', '//a:a2']"),
            ],
            "",
        );
        let selectable_input = selector_list(eval, &[sv]);
        let converted = build_type::copy_and_lift_starlark_value(
            "ruleClass",
            &attribute,
            selectable_input,
            Some(&lc),
            eval.heap(),
        )
        .unwrap();
        let selector_list = SelectorList::from_value(converted).unwrap();
        let first = selector_list.get_elements().next().unwrap();
        let dictionary = selector_dictionary(first);
        let reprs: Vec<(String, String)> = dictionary
            .iter()
            .map(|(k, v)| (k.to_repr(), v.to_repr()))
            .collect();
        let expected_labels = "[Label(\"//a:a1\"), Label(\"//a:a2\")]".to_owned();
        assert_eq!(
            reprs,
            vec![
                (
                    "Label(\"//conditions:a\")".to_owned(),
                    expected_labels.clone()
                ),
                (
                    "Label(\"//conditions:default\")".to_owned(),
                    expected_labels
                ),
            ]
        );
    });
}

/// Tests that Type.convert fails on selector inputs.
#[test]
fn test_convert_does_not_accept_selectables() {
    let lc = label_converter();
    with_eval(|eval| {
        let sv = selector_value(eval, &[("//conditions:a", "['//a:a1', '//a:a2']")], "");
        let selectable_input = selector_list(eval, &[sv]);
        let e = Type::LabelList
            .convert(selectable_input, None, Some(&lc))
            .unwrap_err();
        assert!(
            e.get_message()
                .contains("expected value of type 'list(label)'")
        );
    });
}

/// Test for the default condition key label which is not intended to map to an actual target.
#[test]
fn test_default_condition_label() {
    assert!(!BuildTypeSelector::is_default_condition_label(&label(
        "//condition:a"
    )));
    assert!(BuildTypeSelector::is_default_condition_label(&label(
        DEFAULT_CONDITION_KEY
    )));
}

#[test]
fn test_unconditional_selects() {
    assert!(
        !selector(&[("//conditions:a", "'//a:a'")], Type::Label)
            .unwrap()
            .is_unconditional()
    );
    assert!(
        !selector(
            &[
                ("//conditions:a", "'//a:a'"),
                (DEFAULT_CONDITION_KEY, "'//b:b'")
            ],
            Type::Label
        )
        .unwrap()
        .is_unconditional()
    );
    assert!(
        selector(&[(DEFAULT_CONDITION_KEY, "'//b:b'")], Type::Label)
            .unwrap()
            .is_unconditional()
    );
}

#[test]
fn test_selector_value_equals() {
    with_eval(|eval| {
        let a = selector_value(eval, &[("a", "1"), ("b", "2")], "");
        let b = selector_value(eval, &[("b", "2"), ("a", "1")], "");
        let c = selector_value(eval, &[("a", "1"), ("b", "2")], "Match failed");
        let d = selector_value(eval, &[("a", "1"), ("c", "2")], "");
        let e = selector_value(eval, &[("a", "1"), ("b", "3")], "");
        assert!(a.equals(b).unwrap());
        assert!(b.equals(a).unwrap());
        for (i, x) in [a, c, d, e].iter().enumerate() {
            for (j, y) in [a, c, d, e].iter().enumerate() {
                assert_eq!(x.equals(*y).unwrap(), i == j, "{i} {j}");
            }
        }
    });
}

#[test]
fn test_label_list_dict() {
    let lc = label_converter();
    let converted = with_eval(|eval| {
        let foo_bar = eval.heap().alloc(LabelValue(label("//foo:bar")));
        let foo = eval.heap().alloc(":foo");
        let list = eval
            .heap()
            .alloc(starlark::values::list::AllocList(vec![foo, foo_bar]));
        let wiz = eval_expr(eval, "['//bang']");
        let k1 = eval.heap().alloc("foo");
        let k2 = eval.heap().alloc("wiz");
        let input = eval.heap().alloc(starlark::values::dict::AllocDict(vec![
            (k1, list),
            (k2, wiz),
        ]));
        Type::LabelListDict.convert(input, None, Some(&lc)).unwrap()
    });
    let expected: IndexMap<String, Vec<Label>> = [
        ("foo".to_owned(), labels(&["//quux:foo", "//foo:bar"])),
        ("wiz".to_owned(), labels(&["//bang"])),
    ]
    .into_iter()
    .collect();
    assert_eq!(converted, AttrValue::LabelListDict(expected));
    assert_eq!(
        collect_labels(Type::LabelListDict, &converted),
        labels(&["//quux:foo", "//foo:bar", "//bang:bang"])
    );
}

fn label_list_dict(entries: &[(&str, &[&str])]) -> AttrValue {
    AttrValue::LabelListDict(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), labels(v)))
            .collect(),
    )
}

#[test]
fn test_label_list_dict_concat() {
    assert_eq!(
        Type::LabelListDict.concat(vec![]),
        Some(label_list_dict(&[]))
    );
    let expected = label_list_dict(&[("foo", &["//foo", "//bar"]), ("wiz", &["//bang"])]);
    assert_eq!(
        Type::LabelListDict.concat(vec![expected.clone()]),
        Some(expected)
    );
    let map1 = label_list_dict(&[("foo", &["//a", "//b"]), ("bar", &["//c", "//d"])]);
    let map2 = label_list_dict(&[("bar", &["//x", "//y"]), ("baz", &["//z"])]);
    let expected_after_concat = label_list_dict(&[
        ("foo", &["//a", "//b"]),
        ("bar", &["//x", "//y"]),
        ("baz", &["//z"]),
    ]);
    assert_eq!(
        Type::LabelListDict.concat(vec![map1, map2]),
        Some(expected_after_concat)
    );
}

#[test]
fn test_label_list_dict_bad_first_element() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelListDict,
            "{2: ['foo', 'bar'], 'wiz': ['bang']}",
            None,
            Some(&lc)
        ),
        "expected value of type 'string' for dict key element, but got 2 (int)"
    );
}

#[test]
fn test_label_list_dict_bad_second_element() {
    let lc = label_converter();
    assert_eq!(
        convert_err(
            Type::LabelListDict,
            "{'foo': 'bar', 'wiz': ['bang']}",
            None,
            Some(&lc)
        ),
        "expected value of type 'list(label)' for dict value element, but got \"bar\" (string)"
    );
}

#[test]
fn test_label_list_dict_bad_elements1() {
    assert_eq!(
        convert_err(
            Type::LabelListDict,
            "{('foo',): ('bang',), 'wiz': ('bang',)}",
            None,
            None
        ),
        "expected value of type 'string' for dict key element, but got (\"foo\",) (tuple)"
    );
}
