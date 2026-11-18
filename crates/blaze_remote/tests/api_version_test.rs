#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/ApiVersionTest.java`: tests for
//! `ApiVersion`.

use std::cmp::Ordering;

use blaze_remote::api_version::ApiVersion;
use blaze_remote::reapi::SemVer;

#[test]
fn test_to_string() {
    assert_eq!(ApiVersion::new(0, 0, 0, "v1test").to_string(), "v1test");
    assert_eq!(ApiVersion::new(1, 2, 3, "v1test").to_string(), "v1test");
    assert_eq!(ApiVersion::new(2, 0, 0, "").to_string(), "2.0");
    assert_eq!(ApiVersion::new(2, 1, 0, "").to_string(), "2.1");
    assert_eq!(ApiVersion::new(10, 0, 3, "").to_string(), "10.0.3");
}

#[test]
fn test_compare_to() {
    assert_eq!(
        ApiVersion::new(0, 0, 0, "v1test").compare_to(&ApiVersion::new(0, 0, 0, "v1test")),
        Ordering::Equal
    );
    assert_eq!(
        ApiVersion::new(0, 0, 0, "v1test").compare_to(&ApiVersion::new(0, 1, 0, "")),
        Ordering::Less
    );
    assert_eq!(
        ApiVersion::new(0, 0, 1, "").compare_to(&ApiVersion::new(1, 0, 0, "v1test")),
        Ordering::Greater
    );
    assert_eq!(
        ApiVersion::new(1, 0, 0, "").compare_to(&ApiVersion::new(2, 0, 0, "")),
        Ordering::Less
    );
    assert_eq!(
        ApiVersion::new(2, 0, 0, "").compare_to(&ApiVersion::new(1, 0, 0, "")),
        Ordering::Greater
    );
    assert_eq!(
        ApiVersion::new(2, 1, 0, "").compare_to(&ApiVersion::new(2, 2, 0, "")),
        Ordering::Less
    );
    assert_eq!(
        ApiVersion::new(2, 2, 0, "").compare_to(&ApiVersion::new(2, 1, 0, "")),
        Ordering::Greater
    );
    assert_eq!(
        ApiVersion::new(2, 2, 1, "").compare_to(&ApiVersion::new(2, 2, 2, "")),
        Ordering::Less
    );
    assert_eq!(
        ApiVersion::new(2, 2, 2, "").compare_to(&ApiVersion::new(2, 1, 1, "")),
        Ordering::Greater
    );
    assert_eq!(
        ApiVersion::new(2, 2, 2, "").compare_to(&ApiVersion::new(2, 2, 2, "")),
        Ordering::Equal
    );
}

#[test]
fn test_from_to_semver() {
    let semvers = [
        SemVer {
            major: 2,
            ..Default::default()
        },
        SemVer {
            major: 2,
            minor: 1,
            patch: 3,
            ..Default::default()
        },
        SemVer {
            prerelease: "v1test".to_string(),
            ..Default::default()
        },
    ];
    for sm in &semvers {
        assert_eq!(&ApiVersion::from_sem_ver(sm).to_sem_ver(), sm);
    }
}
