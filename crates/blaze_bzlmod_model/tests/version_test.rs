//! Port of `VersionTest`.

use blaze_bzlmod_model::Version;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

#[test]
fn test_empty_beats_everything() {
    assert!(v("") > v("1.0"));
    assert!(v("") > v("1.0+build"));
    assert!(v("") > v("1.0-pre"));
    assert!(v("") > v("1.0-pre+build-kek.lol"));
}

#[test]
fn test_normalized() {
    assert_eq!(v("1.0").get_normalized(), "1.0");
    assert_eq!(v("1.0+build").get_normalized(), "1.0");
    assert_eq!(v("1.0-pre").get_normalized(), "1.0-pre");
    assert_eq!(v("1.0-pre+build-kek.lol").get_normalized(), "1.0-pre");
    assert_eq!(v("1.0+build-notpre").get_normalized(), "1.0");
}

#[test]
fn test_release_version() {
    assert!(v("2.0") > v("1.0"));
    assert!(v("2.0") > v("1.9"));
    assert!(v("11.0") > v("3.0"));
    assert!(v("1.0.1") > v("1.0"));
    assert!(v("1.0.0") > v("1.0"));
    assert_eq!(v("1.0+build2"), v("1.0+build3"));
    assert_eq!(
        v("1.0+build2").cmp(&v("1.0+build3")),
        std::cmp::Ordering::Equal
    );
    assert!(v("1.0") > v("1.0-pre"));
    assert_eq!(v("1.0"), v("1.0+build-notpre"));
}

#[test]
fn test_release_version_with_letters() {
    assert!(v("1.0.patch.3") > v("1.0"));
    assert!(v("1.0.patch.3") > v("1.0.patch.2"));
    assert!(v("1.0.patch.3") < v("1.0.patch.10"));
    assert!(v("1.0.patch3") > v("1.0.patch10"));
    assert!(v("4") < v("a"));
    assert!(v("abc") < v("abd"));
}

#[test]
fn test_prerelease_version() {
    assert!(v("1.0-pre") > v("1.0-are"));
    assert!(v("1.0-3") > v("1.0-2"));
    assert!(v("1.0-pre") < v("1.0-pre.foo"));
    assert!(v("1.0-pre.3") > v("1.0-pre.2"));
    assert!(v("1.0-pre.10") > v("1.0-pre.2"));
    assert!(v("1.0-pre.10a") < v("1.0-pre.2a"));
    assert!(v("1.0-pre.99") < v("1.0-pre.2a"));
    assert!(v("1.0-pre.patch.3") < v("1.0-pre.patch.4"));
    assert!(v("1.0--") < v("1.0----"));
    assert!(v("2.1.1-develop.bcr.20250113215904") > v("2.1.1-develop.bcr.20250113215903"));
}

#[test]
fn test_parse_exception() {
    assert!(Version::parse("-abc").is_err());
    assert!(Version::parse("1_2").is_err());
    assert!(Version::parse("ßážëł").is_err());
    assert!(Version::parse("1.0-pre?").is_err());
    assert!(Version::parse("1.0-11111111111111111111111111111111111111111").is_err());
    assert!(Version::parse("1.0-pre///").is_err());
    assert!(Version::parse("1..0").is_err());
    assert!(Version::parse("1.0-pre..erp").is_err());
}

// Not in VersionTest: the exact ParseException messages from Version.java.
#[test]
fn parse_exception_messages() {
    assert_eq!(
        Version::parse("1_2").unwrap_err().get_message(),
        "bad version (does not match regex): 1_2"
    );
    let e = Version::parse("1..0").unwrap_err();
    assert_eq!(e.get_message(), "error parsing version: 1..0");
    assert_eq!(e.get_cause().unwrap().get_message(), "identifier is empty");
    let e = Version::parse("1.0-11111111111111111111111111111111111111111").unwrap_err();
    assert_eq!(
        e.get_cause().unwrap().get_message(),
        "numeric version segment is too large: 11111111111111111111111111111111111111111"
    );
    // 2^64-1 fits Long.parseUnsignedLong.
    assert!(Version::parse("18446744073709551615").is_ok());
    assert!(Version::parse("18446744073709551616").is_err());
}
