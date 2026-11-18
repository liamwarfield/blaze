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

#[test]
fn repr() {
    assert_eq!(License::no_license().repr(), "[\"none\"]");
    assert_eq!(
        License::parse_license(&eval_as_sequence(&License::no_license().repr())).unwrap(),
        License::no_license()
    );
    let without_exceptions = License::parse_license(&strings(&["notice", "restricted"])).unwrap();
    // License types sorted by LicenseType enum order.
    assert_eq!(without_exceptions.repr(), "[\"restricted\", \"notice\"]");
    assert_eq!(
        License::parse_license(&eval_as_sequence(&without_exceptions.repr())).unwrap(),
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
        with_exceptions.repr(),
        "[\"restricted\", \"notice\", \"exception=//baz:qux\", \"exception=//foo:bar\"]"
    );
    assert_eq!(
        License::parse_license(&eval_as_sequence(&with_exceptions.repr())).unwrap(),
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
