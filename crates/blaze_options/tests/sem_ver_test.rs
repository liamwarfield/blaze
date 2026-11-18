//! Port of src/test/cpp/sem_ver_test.cc.

// The C++ test checks the negated operators literally.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use blaze_options::rc::sem_ver::SemVer;

// Java: sem_ver_test.cc SemVerTest.ComparisonOperators
#[test]
fn comparison_operators() {
    // In order of precedence.
    let test_versions = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.2.0",
        "1.2.3-456",
        "1.2.3-456.789",
        "1.2.3-456-789",
        "1.2.3-456a",
        "1.2.3-pre",
        "1.2.3-pre.1",
        "1.2.3-zzz",
        "1.2.3",
    ];
    for (i, is) in test_versions.iter().enumerate() {
        for (j, js) in test_versions.iter().enumerate() {
            let iv = SemVer::parse(is).unwrap_or_else(|| panic!("{}", is));
            let jv = SemVer::parse(js).unwrap_or_else(|| panic!("{}", js));
            let message = format!("comparing '{}' with '{}'", is, js);
            if is == js {
                assert!(iv == jv, "{}", message);
                assert!(!(iv != jv), "{}", message);
            } else if i < j {
                assert!(iv < jv, "{}", message);
                assert!(!(iv >= jv), "{}", message);
            } else {
                assert!(iv > jv, "{}", message);
                assert!(!(iv <= jv), "{}", message);
            }
        }
    }
}

fn p(s: &str) -> SemVer {
    SemVer::parse(s).unwrap()
}

// Java: sem_ver_test.cc SemVerTest.NextMajorVersion
#[test]
fn next_major_version() {
    assert_eq!(p("0.0.0").next_major_version(), p("1.0.0"));
    assert_eq!(p("8.0.0").next_major_version(), p("9.0.0"));
    assert_eq!(p("8.2.4").next_major_version(), p("9.0.0"));
    assert_eq!(p("8.2.4-beta").next_major_version(), p("9.0.0"));
}

// Java: sem_ver_test.cc SemVerTest.NextMinorVersion
#[test]
fn next_minor_version() {
    assert_eq!(p("0.0.0").next_minor_version(), p("0.1.0"));
    assert_eq!(p("8.0.0").next_minor_version(), p("8.1.0"));
    assert_eq!(p("8.2.4").next_minor_version(), p("8.3.0"));
    assert_eq!(p("8.2.4-beta").next_minor_version(), p("8.3.0"));
}
