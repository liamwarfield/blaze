//! Tests of `TestXmlOutputParser` (no Java test exists): documents whose expected results come
//! from Bazel's own `TestXmlOutputParser` run on the oracle's embedded JDK 25.0.2
//! (`~/.cache/blaze/work/exec-fix-1/xmlprobe/`: `gen/make_cases*.py` writes the documents,
//! `src/XmlProbe.java` runs the parser from the oracle's `A-server.jar`, and
//! `gen/make_rust_tests.py` writes this file). `None` is a document Java rejects (null or a
//! `TestXmlOutputParserException`, which `TestStrategy.parseTestResult` both turn into null).
//! Generated; edit the generator.

use blaze_exec::exec::test_xml_output_parser::TestXmlOutputParser;
use blaze_protos::blaze::TestCase;
use blaze_protos::blaze::test_case::{Status, Type};

/// (name, run duration in ms, type, status) of each node, in pre-order.
type Node = (Option<&'static str>, Option<i64>, Type, Status);

fn preorder(test_case: &TestCase, out: &mut Vec<(Option<String>, Option<i64>, Type, Status)>) {
    out.push((
        test_case.name.clone(),
        test_case.run_duration_millis,
        test_case.r#type(),
        test_case.status(),
    ));
    for child in &test_case.child {
        preorder(child, out);
    }
}

fn check(document: &[u8], expected: Option<&[Node]>) {
    let result = TestXmlOutputParser::new()
        .parse_xml_into_test_result(&mut &document[..])
        .ok()
        .flatten();
    let actual = result.map(|test_case| {
        let mut nodes = Vec::new();
        preorder(&test_case, &mut nodes);
        nodes
    });
    let expected = expected.map(|nodes| {
        nodes
            .iter()
            .map(|(name, millis, type_, status)| {
                (name.map(str::to_string), *millis, *type_, *status)
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(actual, expected);
}

fn parse(document: &[u8]) {
    let _ = TestXmlOutputParser::new().parse_xml_into_test_result(&mut &document[..]);
}

#[test]
fn oracle_ascii_high() {
    check(
        b"<?xml version=\"1.0\" encoding=\"US-ASCII\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_amp_dangling() {
    check(
        b"<testsuites><testsuite name=\"a & b\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_gt() {
    check(
        b"<testsuites><testsuite name=\"a>b\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("a>b"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_attr_single_quote() {
    check(
        b"<testsuites><testsuite name='s\"q'/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s\"q"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_attr_unknown_entity() {
    check(
        b"<testsuites><testsuite name=\"a&nbsp;b\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_ws() {
    check(
        b"<testsuites><testsuite name=\"a\x09b\x0ac\x0d\x0ad  e\"><testcase name=\"x&#10;y&#9;z&#13;w\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("a b c d  e"), None, Type::TestSuite, Status::Passed),
        (Some("x\u{a}y\u{9}z\u{d}w"), None, Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_base() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_bogus_encoding() {
    check(
        b"<?xml version=\"1.0\" encoding=\"bogus\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_bound_ns() {
    check(
        b"<testsuites xmlns:x=\"urn:x\"><testsuite name=\"s\"><x:testcase name=\"t\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), None, Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_cdata_end_in_text() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a ]]> b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_cdata_in_body() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out><![CDATA[a <&> \x01 b]]></system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_cdata_ok() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out><![CDATA[a <&> b]]></system-out></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_charref_0() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a &#0; b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_charref_1b() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a &#x1B; b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_charref_ok() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a &#x41;&lt;&amp;&gt;&quot;&apos; b</system-out></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_comment_dashes() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><!-- a -- b --></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_comment_end_dash() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><!-- a ---></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_decl_not_first() {
    check(
        b"\x0a<?xml version=\"1.0\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_decodeerr_after_root() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>\x0a\xff\xff\xff",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_doctype_plain() {
    check(
        b"<!DOCTYPE testsuites><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_doctype_plain_nbsp() {
    check(
        b"<!DOCTYPE testsuites><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>&nbsp;</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_dup_attr() {
    check(
        b"<testsuites><testsuite name=\"s\" name=\"t\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_dup_ns_attr() {
    check(
        b"<testsuites xmlns:p=\"u\" xmlns:q=\"u\"><testsuite name=\"s\"><foo p:x=\"1\" q:x=\"2\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_empty() {
    check(b"", None);
}

#[test]
fn oracle_esc_char() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a \x1b b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_fffe_char() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>\xef\xbf\xbe</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_latin1() {
    check(
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9st\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}st"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_latin1_alias() {
    check(
        b"<?xml version=\"1.0\" encoding=\"latin1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9st\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}st"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_latin1_c1() {
    check(
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\x85x\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{85}x"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_latin1_lower() {
    check(
        b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9st\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}st"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_lt_in_attr() {
    check(b"<testsuites><testsuite name=\"a<b\"/></testsuites>", None);
}

#[test]
fn oracle_name_bad_char() {
    check(
        b"<testsuites><testsuite name=\"s\"><a\"b/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_name_digit_start() {
    check(
        b"<testsuites><testsuite name=\"s\"><1a/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_nbsp() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a &nbsp; b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_no_ws_attrs() {
    check(
        b"<testsuites><testsuite name=\"s\"time=\"1\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_nonascii_elem_name() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><f\xc3\xa9e/></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_only_comment() {
    check(b"<!-- x -->", None);
}

#[test]
fn oracle_pi_stylesheet() {
    check(
        b"<?xml version=\"1.0\"?><?xml-stylesheet href=\"a\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_pi_xml_inside() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><?XML foo?></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_root_other_then_suite() {
    check(
        b"<foo><testsuite name=\"s\"><testcase name=\"t\"/></testsuite></foo>",
        Some(&[
            (Some("s"), None, Type::TestSuite, Status::Passed),
            (Some("t"), None, Type::TestCase, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_text_before_root() {
    check(
        b"abc<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_trailing_garbage() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>garbage << & \x00\xff",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_two_colons() {
    check(
        b"<testsuites xmlns:a=\"u\"><testsuite name=\"s\"><a:b:c/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_two_roots() {
    check(
        b"<foo/><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_unbound_attr() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><foo x:a=\"1\"/></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_unbound_elem() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><x:foo/></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_utf16_decl_on_ascii() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF-16\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_utf16be_bom() {
    check(
        b"\xfe\xff\x00<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9N-\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}\u{4e2d}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16be_nobom_nodecl() {
    check(
        b"\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>",
        None,
    );
}

#[test]
fn oracle_utf16le_bom() {
    check(
        b"\xff\xfe<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00-N\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}\u{4e2d}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16le_nobom() {
    check(
        b"<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf8_bad_in_body() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"><system-out>a \xff b</system-out></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_utf8_bad_in_name() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_utf8_bom() {
    check(
        b"\xef\xbb\xbf<testsuites><testsuite name=\"s\"><testcase name=\"t\xc3\xa9\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf8_nodecl_nonascii() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\xc3\xa9\xf0\x9f\x98\x80\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}\u{1f600}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

// Java parses this document; the port fails loudly (XML 1.1).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_version_11() {
    parse(b"<?xml version=\"1.1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>");
}

#[test]
fn oracle_version_missing() {
    check(
        b"<?xml encoding=\"UTF-8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_ws_before_root() {
    check(
        b" \x0a\x09<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"></testcase></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_ws_end_tag() {
    check(
        b"<testsuites><testsuite name=\"s\"></testsuite  ></testsuites >",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_xml_prefix_attr() {
    check(
        b"<testsuites xml:lang=\"en\"><testsuite name=\"s\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_xmlns_empty_prefixed() {
    check(
        b"<testsuites xmlns:a=\"\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_charref_bad() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"a&#xZZ;b\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_charref_ffff() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"a&#xFFFF;b\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_attr_charref_supp() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"a&#x10000;b\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("a\u{10000}b"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_attr_no_value() {
    check(b"<testsuites><testsuite name/></testsuites>", None);
}

#[test]
fn oracle_attr_xmlns_prefixed_use() {
    check(
        b"<testsuites xmlns:p=\"urn:x\"><testsuite p:name=\"s\" name=\"t\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("t"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_colon_start_name() {
    check(
        b"<testsuites><testsuite name=\"s\"><:a/></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_cr_lf_in_text() {
    check(
        b"<testsuites>\x0d\x0a<testsuite name=\"s\"/>\x0d</testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_decl_no_space() {
    check(
        b"<?xml version=\"1.0\"encoding=\"UTF-8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_decl_order_bad() {
    check(
        b"<?xml encoding=\"UTF-8\" version=\"1.0\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_decl_single_quotes() {
    check(
        b"<?xml version='1.0' encoding='ISO-8859-1' ?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_decl_standalone() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_decl_standalone_bad() {
    check(
        b"<?xml version=\"1.0\" standalone=\"maybe\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_decl_version_12() {
    check(
        b"<?xml version=\"1.2\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_default_ns() {
    check(
        b"<testsuites xmlns=\"urn:x\"><testsuite name=\"s\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_doctype_after_root() {
    check(
        b"<testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites><!DOCTYPE x>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

// Java parses this document; the port fails loudly (document type definition).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_doctype_internal() {
    parse(b"<!DOCTYPE testsuites [<!ENTITY nbsp \"&#160;\">]><testsuites><testsuite name=\"a&nbsp;b\"/></testsuites>");
}

#[test]
fn oracle_elem_xmlns_prefix() {
    check(b"<testsuites><xmlns:a/></testsuites>", None);
}

#[test]
fn oracle_empty_attr_value() {
    check(
        b"<testsuites><testsuite name=\"\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some(""), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_enc_csisolatin1() {
    check(
        b"<?xml version=\"1.0\" encoding=\"CSISOLATIN1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_enc_l1() {
    check(
        b"<?xml version=\"1.0\" encoding=\"L1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_enc_ascii() {
    check(
        b"<?xml version=\"1.0\" encoding=\"ASCII\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_enc_bad_name_chars() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF 8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_enc_iso8859_1_java() {
    check(
        b"<?xml version=\"1.0\" encoding=\"ISO8859_1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_enc_utf8_lower() {
    check(
        b"<?xml version=\"1.0\" encoding=\"utf-8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xc3\xa9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_enc_utf8_nodash() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF8\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xc3\xa9\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

// Java parses this document; the port fails loudly (encoding windows-1252).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_enc_windows1252() {
    parse(b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\x80\" time=\"1.5\"/></testsuite></testsuites>");
}

#[test]
fn oracle_end_tag_mismatch() {
    check(
        b"<testsuites><testsuite name=\"s\"></testsuites></testsuite>",
        None,
    );
}

#[test]
fn oracle_eq_spaces() {
    check(
        b"<testsuites><testsuite name = \"s\" /></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_nel_in_attr_latin1() {
    check(
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"a\x85b\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("a\u{85}b"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_pi_colon() {
    check(
        b"<testsuites><testsuite name=\"s\"><?a:b c?><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_pi_no_ws() {
    check(
        b"<testsuites><testsuite name=\"s\"><?ab?><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_supp_name() {
    check(
        b"<testsuites><testsuite name=\"s\"><x\xf0\x90\x80\x80 a=\"1\"/><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_text_cr_only_lines() {
    check(
        b"<testsuites>\x0d<testsuite name=\"a\x0db\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("a b"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

// Java parses this document; the port fails loudly (UCS-4).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_ucs4() {
    parse(b"\x00\x00\x00<\x00\x00\x00t\x00\x00\x00e\x00\x00\x00s\x00\x00\x00t\x00\x00\x00s\x00\x00\x00u\x00\x00\x00i\x00\x00\x00t\x00\x00\x00e\x00\x00\x00s\x00\x00\x00/\x00\x00\x00>");
}

#[test]
fn oracle_utf16_unpaired() {
    check(
        b"\xff\xfe<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\x00\xd8x\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        None,
    );
}

#[test]
fn oracle_utf16be_decl_on_ascii() {
    check(
        b"<?xml version=\"1.0\" encoding=\"UTF-16BE\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_utf16le_bom_nodecl() {
    check(
        b"\xff\xfe<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16le_nobom_decl_noenc() {
    check(
        b"<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16le_nobom_decl_utf16le() {
    check(
        b"<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00L\x00E\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16lebom_decl_utf16be() {
    check(
        b"\xff\xfe<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00B\x00E\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        None,
    );
}

#[test]
fn oracle_utf16lebom_decl_utf16le() {
    check(
        b"\xff\xfe<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x001\x006\x00L\x00E\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf16lebom_decl_utf8() {
    check(
        b"\xff\xfe<\x00?\x00x\x00m\x00l\x00 \x00v\x00e\x00r\x00s\x00i\x00o\x00n\x00=\x00\"\x001\x00.\x000\x00\"\x00 \x00e\x00n\x00c\x00o\x00d\x00i\x00n\x00g\x00=\x00\"\x00U\x00T\x00F\x00-\x008\x00\"\x00?\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00<\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00s\x00\"\x00>\x00<\x00t\x00e\x00s\x00t\x00c\x00a\x00s\x00e\x00 \x00n\x00a\x00m\x00e\x00=\x00\"\x00t\x00\xe9\x00\"\x00 \x00t\x00i\x00m\x00e\x00=\x00\"\x001\x00.\x005\x00\"\x00/\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00>\x00<\x00/\x00t\x00e\x00s\x00t\x00s\x00u\x00i\x00t\x00e\x00s\x00>\x00",
        None,
    );
}

#[test]
fn oracle_utf8bom_decl_latin1() {
    check(
        b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xe9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{e9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_utf8bom_decl_latin1_utf8bytes() {
    check(
        b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><testsuites><testsuite name=\"s\"><testcase name=\"t\xc3\xa9\" time=\"1.5\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
        (Some("t\u{c3}\u{a9}"), Some(1500), Type::TestCase, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_xml_prefix_rebind() {
    check(
        b"<testsuites xmlns:xml=\"urn:x\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_xmlns_prefix_bind() {
    check(
        b"<testsuites xmlns:xmlns=\"urn:x\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_xmlpi_nospace() {
    check(
        b"<?xml?><testsuites><testsuite name=\"s\"><testcase name=\"t\" time=\"1.5\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_attr_xmlns_alone_empty() {
    check(
        b"<testsuites xmlns=\"\"><testsuite name=\"s\"/></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_bad_charref_upper_x() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\">&#X41;</testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_bind_xml_uri() {
    check(
        b"<testsuites xmlns:p=\"http://www.w3.org/XML/1998/namespace\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_bind_xmlns_uri() {
    check(
        b"<testsuites xmlns:p=\"http://www.w3.org/2000/xmlns/\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_cdata_at_root() {
    check(b"<![CDATA[x]]><testsuites/>", None);
}

#[test]
fn oracle_q_charref_huge() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\">&#99999999999;</testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_charref_in_prolog() {
    check(b"&#32;<testsuites/>", None);
}

#[test]
fn oracle_q_charref_leading_zeros() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\">&#0000065;</testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_colon_only_elem() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><:/></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_comment_before_decl() {
    check(b"<!-- c --><?xml version=\"1.0\"?><testsuites/>", None);
}

#[test]
fn oracle_q_comment_in_doctype_pos() {
    check(
        b"<!DOCTYPE testsuites><!DOCTYPE testsuites><testsuites/>",
        None,
    );
}

#[test]
fn oracle_q_comment_three_dashes_start() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><!---x--></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_default_xml_ns() {
    check(
        b"<testsuites xmlns=\"http://www.w3.org/XML/1998/namespace\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_doctype_lowercase() {
    check(b"<!doctype testsuites><testsuites/>", None);
}

// Java parses this document; the port fails loudly (document type definition).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_q_doctype_system() {
    parse(b"<!DOCTYPE testsuites SYSTEM \"nonexistent.dtd\"><testsuites><testsuite name=\"s\"/></testsuites>");
}

#[test]
fn oracle_q_empty_comment() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><!----></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_end_no_name() {
    check(b"<testsuites><testsuite name=\"s\"></></testsuites>", None);
}

#[test]
fn oracle_q_end_tag_other_prefix() {
    check(
        b"<testsuites xmlns:a=\"urn:a\" xmlns:b=\"urn:a\"><testsuite name=\"s\"><a:x></b:x></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_leading_colon_attr() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><f :x=\"1\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_q_leading_colon_end() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><:x></:x></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_leading_colon_unbound_like() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><:b:c/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_lt_space_name() {
    check(b"<testsuites>< testsuite name=\"s\"/></testsuites>", None);
}

#[test]
fn oracle_q_ns_scope_ends() {
    check(
        b"<testsuites><testsuite name=\"s\"><x xmlns:p=\"u\"/><p:y/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_pi_target_xml_prefix() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><?xmlfoo bar?></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_q_prefix_dash_local() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><a:-b/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_prefix_digit_local() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><a:1b/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_testsuite_prefixed_root() {
    check(
        b"<a:testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"/></a:testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_text_rbracket_gt() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\">]></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_text_rbrackets() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\">]]]></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_trailing_colon_attr() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><f a:=\"1\"/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_trailing_colon_elem() {
    check(
        b"<testsuites xmlns:a=\"urn:a\"><testsuite name=\"s\"><a:/></testsuite></testsuites>",
        None,
    );
}

#[test]
fn oracle_q_unprefixed_and_prefixed_same_local() {
    check(
        b"<testsuites xmlns:a=\"u\"><testsuite name=\"s\"><f x=\"1\" a:x=\"2\"/></testsuite></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_q_ws_before_gt_in_start() {
    check(
        b"<testsuites ><testsuite name=\"s\" ></testsuite></testsuites>",
        Some(&[
            (None, None, Type::TestSuite, Status::Passed),
            (Some("s"), None, Type::TestSuite, Status::Passed),
        ]),
    );
}

#[test]
fn oracle_q_ws_before_slash() {
    check(b"<testsuites><testsuite name=\"s\" / ></testsuites>", None);
}

#[test]
fn oracle_q_xml_prefix_correct() {
    check(
        b"<testsuites xmlns:xml=\"http://www.w3.org/XML/1998/namespace\"><testsuite name=\"s\"/></testsuites>",
        Some(&[
        (None, None, Type::TestSuite, Status::Passed),
        (Some("s"), None, Type::TestSuite, Status::Passed),
    ]),
    );
}

#[test]
fn oracle_q_xmlns_attr_dup() {
    check(
        b"<testsuites xmlns:a=\"u\" xmlns:a=\"v\"><testsuite name=\"s\"/></testsuites>",
        None,
    );
}

// Java parses this document; the port fails loudly (document type definition).
#[test]
#[should_panic(expected = "not ported yet")]
fn oracle_int_noentity() {
    parse(b"<!DOCTYPE testsuites [<!ELEMENT testsuites ANY>]><testsuites><testsuite name=\"ab\"/></testsuites>");
}
