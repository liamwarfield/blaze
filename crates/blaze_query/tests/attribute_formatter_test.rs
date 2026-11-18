//! `AttributeFormatter.getAttributeProto` and `ProtoUtils` against attribute records of the
//! oracle's `bazel query --output=streamed_jsonproto //...` on brotli
//! (`~/.cache/blaze/oracle-records/oracle-9.3.2/brotli/raw/query_jsonproto.stdout`, the
//! `//:brotli` cc_binary). Expected strings are copied from that recording.

use blaze_cmdline::Label;
use blaze_packages::target::LabelPrinter;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::json::JsonFormat;
use blaze_query::compat::attribute_formatter::get_attribute_proto;

fn json(name: &str, type_: Type, value: Option<AttrValue>, explicitly_specified: bool) -> String {
    let attr = get_attribute_proto(
        name,
        type_,
        value.as_ref(),
        explicitly_specified,
        /* encode_boolean_and_tri_state_as_integer_and_string= */ true,
        /* source_aspect= */ None,
        /* include_attribute_source_aspects= */ false,
        &LabelPrinter::legacy(),
    );
    JsonFormat::printer()
        .omitting_insignificant_whitespace()
        .print(&attr)
        .unwrap()
}

fn label(s: &str) -> Label {
    Label::parse_canonical(s).unwrap()
}

#[test]
fn boolean_false_is_also_encoded_as_int_and_string() {
    assert_eq!(
        json(
            "$cc_binary",
            Type::Boolean,
            Some(AttrValue::Bool(false)),
            false
        ),
        r#"{"name":"$cc_binary","type":"BOOLEAN","intValue":0,"stringValue":"false","explicitlySpecified":false,"booleanValue":false}"#
    );
}

#[test]
fn boolean_true_explicitly_specified() {
    assert_eq!(
        json(
            "linkstatic",
            Type::Boolean,
            Some(AttrValue::Bool(true)),
            true
        ),
        r#"{"name":"linkstatic","type":"BOOLEAN","intValue":1,"stringValue":"true","explicitlySpecified":true,"booleanValue":true}"#
    );
}

#[test]
fn null_label_has_only_name_type_and_flags() {
    assert_eq!(
        json("$stl", Type::Label, None, false),
        r#"{"name":"$stl","type":"LABEL","explicitlySpecified":false,"nodep":false}"#
    );
}

#[test]
fn label_list() {
    assert_eq!(
        json(
            "$config_dependencies",
            Type::LabelList,
            Some(AttrValue::LabelList(vec![
                label("//:msvc"),
                label("//:clang-cl")
            ])),
            false
        ),
        r#"{"name":"$config_dependencies","type":"LABEL_LIST","stringListValue":["//:msvc","//:clang-cl"],"explicitlySpecified":false,"nodep":false}"#
    );
}

#[test]
fn empty_string_list() {
    assert_eq!(
        json(
            "tags",
            Type::StringList,
            Some(AttrValue::StringList(vec![])),
            false
        ),
        r#"{"name":"tags","type":"STRING_LIST","explicitlySpecified":false,"nodep":false}"#
    );
}

#[test]
fn string() {
    assert_eq!(
        json(
            "name",
            Type::String,
            Some(AttrValue::String("brotli".to_owned())),
            true
        ),
        r#"{"name":"name","type":"STRING","stringValue":"brotli","explicitlySpecified":true,"nodep":false}"#
    );
}

#[test]
fn integer_has_no_nodep() {
    assert_eq!(
        json("stamp", Type::Integer, Some(AttrValue::Int(-1)), false),
        r#"{"name":"stamp","type":"INTEGER","intValue":-1,"explicitlySpecified":false}"#
    );
}

#[test]
fn nodep_label_list_is_a_string_list_with_nodep() {
    assert_eq!(
        json(
            "visibility",
            Type::NodepLabelList,
            Some(AttrValue::LabelList(vec![label("//visibility:public")])),
            false
        ),
        r#"{"name":"visibility","type":"STRING_LIST","stringListValue":["//visibility:public"],"explicitlySpecified":false,"nodep":true}"#
    );
}
