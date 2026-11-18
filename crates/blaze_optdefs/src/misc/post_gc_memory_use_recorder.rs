//! Port of `metrics/PostGCMemoryUseRecorder.java` (the `OptionsBase` subclass and what it uses).

use blaze_options::options;

// Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:270 PostGCMemoryUseRecorder.GcAfterBuildModule.Options
options! {
    /// Command options for forcing a GC after a build.
    pub struct PostGCMemoryUseRecorderGcAfterBuildModuleOptions {
        #[option(
            name = "experimental_force_gc_after_build",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If true calls System.gc() after a build to try and get a post-gc peak heap",
                " measurement.",
            ),
        )]
        pub experimental_force_gc_after_build: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<PostGCMemoryUseRecorderGcAfterBuildModuleOptions>(&[
            "experimental_force_gc_after_build",
        ]);
    }

    #[test]
    fn parses_flag() {
        let o = parse::<PostGCMemoryUseRecorderGcAfterBuildModuleOptions>(&[
            "--experimental_force_gc_after_build",
        ])
        .unwrap();
        assert!(o.experimental_force_gc_after_build);
    }
}
