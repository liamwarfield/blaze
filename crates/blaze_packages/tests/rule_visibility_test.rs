//! Port of `src/test/java/com/google/devtools/build/lib/packages/RuleVisibilityTest.java`.

use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_packages::package::PackageGroupsRuleVisibility;
use blaze_packages::package::RuleVisibility;

fn label(label_string: &str) -> Label {
    Label::parse_canonical_unchecked(label_string)
}

fn pkg(label_string: &str) -> PackageIdentifier {
    label(label_string).get_package_identifier().clone()
}

fn rule_visibility(label_strings: &[&str]) -> RuleVisibility {
    let labels: Vec<Label> = label_strings.iter().map(|s| label(s)).collect();
    RuleVisibility::parse_unchecked(&labels)
}

const A: &str = "//a:__pkg__";
// Package group labels are represented differently than __pkg__ labels, so cover both cases.
const B_PG: &str = "//b:pkggroup";
const C: &str = "//c:__pkg__";
const PUBLIC: &str = "//visibility:public";
const PRIVATE: &str = "//visibility:private";

/// The message of a panic (Java: the IllegalArgumentException message).
fn panic_message(f: impl FnOnce()) -> String {
    let result = catch_unwind(AssertUnwindSafe(f));
    let payload = result.expect_err("expected a panic");
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    }
}

#[test]
fn validate_and_simplify_validates() {
    let e1 = RuleVisibility::validate_and_simplify(&[label("//visibility:pirvate")]).unwrap_err();
    assert!(e1.get_message().contains(
        "Invalid visibility label '//visibility:pirvate'; did you mean //visibility:public or \
         //visibility:private?"
    ));

    let e2 = RuleVisibility::validate_and_simplify(&[label(PUBLIC), label("//visibility:pbulic")])
        .unwrap_err();
    assert!(e2.get_message().contains(
        "Invalid visibility label '//visibility:pbulic'; did you mean //visibility:public or \
         //visibility:private?"
    ));
}

#[test]
fn validate_and_simplify_simplifies_public() {
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(A), label(PUBLIC)]).unwrap(),
        vec![label(PUBLIC)]
    );
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(PUBLIC), label(B_PG)]).unwrap(),
        vec![label(PUBLIC)]
    );
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(PUBLIC), label(PRIVATE)]).unwrap(),
        vec![label(PUBLIC)]
    );
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(PUBLIC), label(PUBLIC)]).unwrap(),
        vec![label(PUBLIC)]
    );
}

#[test]
fn validate_and_simplify_simplifies_private() {
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(A), label(PRIVATE)]).unwrap(),
        vec![label(A)]
    );
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(PRIVATE), label(B_PG)]).unwrap(),
        vec![label(B_PG)]
    );
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(PRIVATE), label(PRIVATE)]).unwrap(),
        vec![label(PRIVATE)]
    );
}

#[test]
fn empty_list_canonicalized_to_private() {
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[]).unwrap(),
        vec![label(PRIVATE)]
    );
    let e = panic_message(|| {
        RuleVisibility::parse_unchecked(&[]);
    });
    assert!(e.contains("must not be empty"), "{e}");
}

// TODO(arostovtsev): we ought to uniquify the labels, but that would be an incompatible change
// (affects query output).
#[test]
fn validate_and_simplify_does_not_uniquify() {
    assert_eq!(
        RuleVisibility::validate_and_simplify(&[label(A), label(A)]).unwrap(),
        vec![label(A), label(A)]
    );
}

#[test]
fn package_groups_rule_visibility_create_requires_validated_simplified_non_constant_labels() {
    let e1 = panic_message(|| {
        PackageGroupsRuleVisibility::create(&[]);
    });
    assert!(e1.contains("must not be empty"), "{e1}");
    let e2 = panic_message(|| {
        PackageGroupsRuleVisibility::create(&[label(PUBLIC), label(A)]);
    });
    assert!(e2.contains("must be validated and simplified"), "{e2}");
    let e3 = panic_message(|| {
        PackageGroupsRuleVisibility::create(&[label(A), label(PRIVATE)]);
    });
    assert!(e3.contains("must be validated and simplified"), "{e3}");
    let e4 = panic_message(|| {
        PackageGroupsRuleVisibility::create(&[label(PUBLIC)]);
    });
    assert!(
        e4.contains("must not equal [\"//visibility:public\"] or [\"//visibility:private\"]"),
        "{e4}"
    );
    let e5 = panic_message(|| {
        PackageGroupsRuleVisibility::create(&[label(PRIVATE)]);
    });
    assert!(
        e5.contains("must not equal [\"//visibility:public\"] or [\"//visibility:private\"]"),
        "{e5}"
    );
}

#[test]
fn concatenation() {
    assert_eq!(
        rule_visibility(&[A, B_PG]).concat_with_package(&pkg(C)),
        rule_visibility(&[A, B_PG, C])
    );

    assert_eq!(
        RuleVisibility::Public.concat_with_package(&pkg(C)),
        RuleVisibility::Public
    );

    assert_eq!(
        RuleVisibility::Private.concat_with_package(&pkg(C)),
        rule_visibility(&[C])
    );

    // Duplicates are not added, though they are preserved.
    assert_eq!(
        rule_visibility(&[A, B_PG]).concat_with_package(&pkg(A)),
        rule_visibility(&[A, B_PG])
    );
    assert_eq!(
        rule_visibility(&[A, B_PG, B_PG, A]).concat_with_package(&pkg(A)),
        rule_visibility(&[A, B_PG, B_PG, A])
    );
    assert_eq!(
        rule_visibility(&[A, B_PG, B_PG, A]).concat_with_package(&pkg(C)),
        rule_visibility(&[A, B_PG, B_PG, A, C])
    );
}
