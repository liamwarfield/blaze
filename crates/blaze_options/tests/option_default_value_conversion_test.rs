//! Port of src/test/java/com/google/devtools/common/options/OptionDefaultValueConversionTest.java.
//!
//! PORT: Java scans every `@Option` field on Bazel's production classpath. This crate only
//! contains `ClientOptions` (and the test-only `TestOptions`), so the check is narrowed to
//! those; the bulk-transcription wave should register each new options class here.

mod common;

use blaze_options::OptionsClass;
use blaze_options::definition::OptionsClassRef;
use blaze_options::rc::client_options::ClientOptions;
use common::test_options::TestOptions;

fn all_options_classes() -> Vec<OptionsClassRef> {
    vec![ClientOptions::class(), TestOptions::class()]
}

// Java: OptionDefaultValueConversionTest.shouldConvertDefaultValue
#[test]
fn should_convert_default_value() {
    for class in all_options_classes() {
        for option_definition_under_test in class.definitions() {
            if let Err(e) = option_definition_under_test.try_get_default_value(None) {
                panic!("{}: {}", option_definition_under_test, e);
            }
        }
    }
}
