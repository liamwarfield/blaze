//! Ports of `src/test/java/com/google/devtools/build/lib/packages/LicenseTest.java` and
//! `LabelConverterTest.java`.

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_vfs::PathFragment;

use crate::types::LabelConverter;
use crate::types::License;
use crate::types::LicenseType;
use crate::types::Type;
use crate::types::test_util::eval_expr;
use crate::types::test_util::with_eval;

#[test]
fn test_least_restrictive() {
    assert_eq!(
        License::least_restrictive(&[LicenseType::Restricted]),
        LicenseType::Restricted
    );
    assert_eq!(
        License::least_restrictive(&[LicenseType::Restricted, LicenseType::ByExceptionOnly]),
        LicenseType::Restricted
    );
    assert_eq!(
        License::least_restrictive(&[]),
        LicenseType::ByExceptionOnly
    );
}

/// Evaluates a string as a Starlark expression returning a sequence of strings.
fn eval_as_sequence(string: &str) -> Vec<String> {
    with_eval(|eval| {
        let v = eval_expr(eval, string);
        crate::types::type_::into_string_list(Type::StringList.convert(v, None, None).unwrap())
    })
}

fn strings(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| (*s).to_owned()).collect()
}

/// Java `Starlark.repr(license)`: the repr of the License as a Starlark value.
fn starlark_repr(l: &License) -> String {
    starlark::values::Heap::temp(|heap| heap.alloc(l.clone()).to_repr())
}

#[test]
fn repr() {
    assert_eq!(starlark_repr(&License::no_license()), "[\"none\"]");
    assert_eq!(
        License::parse_license(&eval_as_sequence(&starlark_repr(&License::no_license()))).unwrap(),
        License::no_license()
    );
    let without_exceptions = License::parse_license(&strings(&["notice", "restricted"])).unwrap();
    // License types sorted by LicenseType enum order.
    assert_eq!(
        starlark_repr(&without_exceptions),
        "[\"restricted\", \"notice\"]"
    );
    assert_eq!(
        License::parse_license(&eval_as_sequence(&starlark_repr(&without_exceptions))).unwrap(),
        without_exceptions
    );
    let with_exceptions = License::parse_license(&strings(&[
        "notice",
        "restricted",
        "exception=//foo:bar",
        "exception=//baz:qux",
    ]))
    .unwrap();
    // Exceptions sorted alphabetically.
    assert_eq!(
        starlark_repr(&with_exceptions),
        "[\"restricted\", \"notice\", \"exception=//baz:qux\", \"exception=//foo:bar\"]"
    );
    assert_eq!(
        License::parse_license(&eval_as_sequence(&starlark_repr(&with_exceptions))).unwrap(),
        with_exceptions
    );
}

// Java: LabelConverterTest.convertLabel
#[test]
fn convert_label() {
    let base_package = PackageIdentifier::create(
        RepositoryName::create("quux").unwrap(),
        PathFragment::create("baz"),
    );
    let converter = LabelConverter::new(
        &base_package,
        RepositoryMapping::create(
            [("orig_repo", RepositoryName::create("new_repo").unwrap())],
            MAIN.clone(),
        ),
    );
    assert_eq!(
        converter.convert("@orig_repo//foo:bar").unwrap(),
        Label::parse_canonical("@new_repo//foo:bar").unwrap()
    );
    assert_eq!(
        converter.convert("//foo:bar").unwrap(),
        Label::parse_canonical("@quux//foo:bar").unwrap()
    );
    assert_eq!(
        converter.convert(":bar").unwrap(),
        Label::parse_canonical("@quux//baz:bar").unwrap()
    );
}

// Not a Java test: `LicenseType.valueOf(str.toUpperCase(Locale.ENGLISH))` maps the chars of the
// internal string, so the byte DF (U+00DF, `ß`, written `\337` in a BUILD file) upper-cases to
// `SS`; `é` (C3 A9) stays non-ASCII and is invalid.
#[test]
fn parse_license_upper_cases_latin1_chars_as_java() {
    assert_eq!(
        License::parse_license(&["permi\u{df}ive".to_owned()]).unwrap(),
        License::parse_license(&["permissive".to_owned()]).unwrap()
    );
    assert_eq!(
        License::parse_license(&["notic\u{c3}\u{a9}".to_owned()])
            .unwrap_err()
            .0,
        "invalid license type: 'notic\u{c3}\u{a9}'"
    );
}
