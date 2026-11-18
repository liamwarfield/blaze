//! Port of `analysis/platform/PlatformConstants.java`: hardcoded constants used by the platforms
//! system.

use blaze_cmdline::Label;

// Java: com/google/devtools/build/lib/analysis/platform/PlatformConstants.java:23 PlatformConstants.INTERNAL_PLATFORM
pub fn internal_platform() -> Label {
    Label::parse_canonical_unchecked("@bazel_tools//tools:internal_platform")
}

/// The label of the toolchain type to add to the default "test" exec group.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformConstants.java:27 PlatformConstants.DEFAULT_TEST_TOOLCHAIN_TYPE
pub fn default_test_toolchain_type() -> Label {
    Label::parse_canonical_unchecked("@bazel_tools//tools/test:default_test_toolchain_type")
}
