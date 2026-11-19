//! Port of src/test/java/com/google/devtools/common/options/OptionFilterDescriptionsTest.java.
//! Tests that we have descriptions for every option tag.

use std::collections::BTreeSet;

use blaze_options::option_filter_descriptions::OptionFilterDescriptions;
use blaze_options::{OptionDocumentationCategory, OptionEffectTag, OptionMetadataTag};

// Java: OptionFilterDescriptionsTest.documentationOrderIncludesAllDocumentedCategories
#[test]
fn documentation_order_includes_all_documented_categories() {
    // Expect the documentation order to include everything but the undocumented category.
    let mut doc_order_plus_undocumented: Vec<OptionDocumentationCategory> =
        OptionFilterDescriptions::DOCUMENTATION_ORDER.to_vec();
    doc_order_plus_undocumented.push(OptionDocumentationCategory::UNDOCUMENTED);

    let mut values = OptionDocumentationCategory::VALUES.to_vec();
    values.sort();
    doc_order_plus_undocumented.sort();
    assert_eq!(values, doc_order_plus_undocumented);
}

// Java: OptionFilterDescriptionsTest.optionDocumentationCategoryDescriptionsContainsAllCategories
#[test]
fn option_documentation_category_descriptions_contains_all_categories() {
    // Check that we have a description for all valid option categories.
    let option_category_descriptions =
        OptionFilterDescriptions::get_option_categories_enum_description();

    assert_eq!(
        OptionDocumentationCategory::VALUES
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        option_category_descriptions.keys().copied().collect()
    );
    assert_eq!(
        OptionDocumentationCategory::VALUES.len(),
        option_category_descriptions.len()
    );
}

// Java: OptionFilterDescriptionsTest.optionEffectTagDescriptionsContainsAllTags
#[test]
fn option_effect_tag_descriptions_contains_all_tags() {
    // Check that we have a description for all valid option tags.
    let option_effect_tag_description =
        OptionFilterDescriptions::get_option_effect_tag_description("blaze");

    assert_eq!(
        OptionEffectTag::VALUES
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        option_effect_tag_description.keys().copied().collect()
    );
    assert_eq!(
        OptionEffectTag::VALUES.len(),
        option_effect_tag_description.len()
    );
}

// Java: OptionFilterDescriptionsTest.optionMetadataTagDescriptionsContainsAllTags
#[test]
fn option_metadata_tag_descriptions_contains_all_tags() {
    // Check that we have a description for all valid option tags.
    let option_metadata_tag_description =
        OptionFilterDescriptions::get_option_metadata_tag_description("blaze");

    assert_eq!(
        OptionMetadataTag::VALUES
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        option_metadata_tag_description.keys().copied().collect()
    );
    assert_eq!(
        OptionMetadataTag::VALUES.len(),
        option_metadata_tag_description.len()
    );
}
