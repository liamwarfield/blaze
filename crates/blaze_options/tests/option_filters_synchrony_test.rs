//! Port of src/test/java/com/google/devtools/common/options/OptionFiltersSynchronyTest.java.
//!
//! This test makes sure that the two java filtering enums, OptionMetadataTag and
//! OptionEffectTag, are kept in sync with the matching proto.

use blaze_options::{OptionEffectTag, OptionMetadataTag};
use blaze_protos::options as proto;

// Java: OptionFiltersSynchronyTest.optionEffectTags
#[test]
fn option_effect_tags() {
    // PORT: prost enums have no UNRECOGNIZED value, so the sizes are compared directly.
    let proto_values: Vec<i32> = (-1..64)
        .filter(|n| proto::OptionEffectTag::try_from(*n).is_ok())
        .collect();
    assert_eq!(proto_values.len(), OptionEffectTag::VALUES.len());

    // Now go through each and check that the names are equal.
    for java_tag in OptionEffectTag::VALUES {
        let proto_tag = proto::OptionEffectTag::try_from(java_tag.get_value());

        // First check that the tag exists with this value, then that the names are equal.
        let proto_tag = proto_tag.unwrap_or_else(|_| {
            panic!(
                "OptionEffectTag {} does not have a proto equivalent with the same value",
                java_tag.name()
            )
        });
        assert_eq!(
            java_tag.name(),
            proto_tag.as_str_name(),
            "OptionEffectTag {} does not have the same name as the proto equivalent {}",
            java_tag.name(),
            proto_tag.as_str_name()
        );
    }
}

// Java: OptionFiltersSynchronyTest.optionMetadataTags
#[test]
fn option_metadata_tags() {
    // PORT: prost enums have no UNRECOGNIZED value, so the sizes are compared directly.
    let proto_values: Vec<i32> = (-1..64)
        .filter(|n| proto::OptionMetadataTag::try_from(*n).is_ok())
        .collect();
    assert_eq!(proto_values.len(), OptionMetadataTag::VALUES.len());

    // Now go through each and check that the names are equal.
    for java_tag in OptionMetadataTag::VALUES {
        let proto_tag = proto::OptionMetadataTag::try_from(java_tag.get_value());

        // First check that the tag exists with this value, then that the names are equal.
        let proto_tag = proto_tag.unwrap_or_else(|_| {
            panic!(
                "OptionMetadataTag {} does not have a proto equivalent with the same value",
                java_tag.name()
            )
        });
        assert_eq!(
            java_tag.name(),
            proto_tag.as_str_name(),
            "OptionMetadataTag {} does not have the same name as the proto equivalent {}",
            java_tag.name(),
            proto_tag.as_str_name()
        );
    }
}
