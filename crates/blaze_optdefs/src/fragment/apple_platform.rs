//! `ApplePlatform.PlatformType` string constants (`rules/apple/ApplePlatform.java`).

// Java: com/google/devtools/build/lib/rules/apple/ApplePlatform.java:217 ApplePlatform.PlatformType
/// The former enum PlatformType is being migrated to a Starlark struct PLATFORM_TYPE in
/// builtins_bzl/common/objc/apple_platform.bzl. During the migration, PlatformType has been
/// converted to a static class hosting string constants as Java duplicates of
/// apple_platform.PLATFORM_TYPE.
pub mod apple_platform_platform_type {
    pub const IOS: &str = "ios";
    pub const VISIONOS: &str = "visionos";
    pub const WATCHOS: &str = "watchos";
    pub const TVOS: &str = "tvos";
    pub const MACOS: &str = "macos";
    pub const CATALYST: &str = "catalyst";
}
