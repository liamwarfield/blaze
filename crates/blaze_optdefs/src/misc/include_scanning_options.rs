//! Port of `includescanning/IncludeScanningOptions.java` (the `OptionsBase` subclass and what it uses).

use std::sync::Arc;

use blaze_options::{ConversionContext, Converter, OptionsParsingException, Value, options};

use super::resource_converter::{ResourceConverterIntegerConverter, host_cpus_supplier};

// Java: com/google/devtools/build/lib/includescanning/IncludeScanningOptions.java:32 IncludeScanningOptions.ParallelismConverter
/// Converter for scanning parallelism threads: Takes `FLAG_SYNTAX` 0 disables scanning
/// parallelism.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParallelismConverter;

impl ParallelismConverter {
    // Java: com/google/devtools/build/lib/includescanning/IncludeScanningOptions.java:33 IncludeScanningOptions.ParallelismConverter.<init>
    fn inner() -> ResourceConverterIntegerConverter {
        ResourceConverterIntegerConverter::new(Arc::new(host_cpus_supplier), 0, i32::MAX)
    }
}

impl Converter for ParallelismConverter {
    fn convert(
        &self,
        input: Option<&str>,
        c: Option<&ConversionContext>,
    ) -> Result<Option<Value>, OptionsParsingException> {
        Self::inner().convert(input, c)
    }
    fn get_type_description(&self) -> String {
        Self::inner().get_type_description()
    }
}

// Java: com/google/devtools/build/lib/includescanning/IncludeScanningOptions.java:26 IncludeScanningOptions
options! {
    /// Command options specific to include scanning.
    pub struct IncludeScanningOptions {
        #[option(
            name = "experimental_inmemory_dotincludes_files",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "false",
            help = concat!(
                "If enabled, searching for '#include' lines in generated header files will not ",
                "touch local disk. This makes include scanning of C++ files less disk-intensive.",
            ),
        )]
        pub in_memory_includes_files: bool,

        #[option(
            name = "experimental_remote_include_extraction_size_threshold",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "1000000",
            help = "Run remotable C++ include extraction remotely if the file size in bytes exceeds this.",
        )]
        pub experimental_remote_extraction_threshold: i32,

        #[option(
            name = "experimental_include_scanning_parallelism",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "80",
            converter = ParallelismConverter,
            help = concat!(
                "Configures the size of the thread pool used for include scanning. Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". 0 means to disable parallelism and to just rely on the build graph parallelism ",
                "for concurrency. ",
                " \"auto\" means to use a reasonable value derived from the machine's hardware",
                " profile (e.g. the number of processors).",
            ),
        )]
        pub include_scanning_parallelism: i32,

        #[option(
            name = "experimental_reuse_include_scanning_threads",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, EXECUTION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            default_value = "false",
            help = "If enabled core threads of include scanner pool will not die during execution.",
        )]
        pub experimental_reuse_include_scanning_threads: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::test_support::{check_options_class, parse};

    #[test]
    fn options_class_matches_java() {
        check_options_class::<IncludeScanningOptions>(&[
            "experimental_inmemory_dotincludes_files",
            "experimental_remote_include_extraction_size_threshold",
            "experimental_include_scanning_parallelism",
            "experimental_reuse_include_scanning_threads",
        ]);
    }

    #[test]
    fn parses_flags() {
        let o = parse::<IncludeScanningOptions>(&["--experimental_include_scanning_parallelism=0"])
            .unwrap();
        assert_eq!(o.include_scanning_parallelism, 0);
        assert_eq!(
            parse::<IncludeScanningOptions>(&["--experimental_include_scanning_parallelism=-1"])
                .unwrap_err()
                .get_message(),
            "While parsing option --experimental_include_scanning_parallelism=-1: Value \
             '(-1.000000)' must be at least 0.000000."
        );
    }
}
