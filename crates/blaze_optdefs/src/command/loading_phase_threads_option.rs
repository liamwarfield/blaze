//! Transcription of `com/google/devtools/build/lib/runtime/LoadingPhaseThreadsOption.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/LoadingPhaseThreadsOption.java:26 LoadingPhaseThreadsOption
options! {
    /// Defines the --loading_phase_threads option which is used by multiple commands.
    pub struct LoadingPhaseThreadsOption {
        #[option(
            name = "loading_phase_threads",
            default_value = "auto",
            converter = LoadingPhaseThreadCountConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "Number of parallel threads to use for the loading/analysis phase.",
                "Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". \"auto\" sets a reasonable default based on ",
                "host resources. Must be at least 1.",
            ),
        )]
        pub threads: i32,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const LOADING_PHASE_THREADS_OPTION_OPTION_NAMES: &[&str] = &["loading_phase_threads"];

    #[test]
    fn loading_phase_threads_option_option_names() {
        super::super::test_util::check_option_names(
            LoadingPhaseThreadsOption::class(),
            LOADING_PHASE_THREADS_OPTION_OPTION_NAMES,
        );
    }

    #[test]
    fn loading_phase_threads_option_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<LoadingPhaseThreadsOption>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

use super::resource_converter::{host_cpus_supplier, resource_integer_converter};

// Java: com/google/devtools/build/lib/util/TestType.java:48 TestType.isInTest
/// Whether this process runs under a test (`TEST_TMPDIR` is set), read once.
pub fn is_in_test() -> bool {
    static IN_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // Java: TestType.getTestTypeFromEnvVars (TestType.java:38)
    *IN_TEST.get_or_init(|| std::env::var_os("TEST_TMPDIR").is_some())
}

resource_integer_converter!(
    // Java: com/google/devtools/build/lib/runtime/LoadingPhaseThreadsOption.java:47 LoadingPhaseThreadsOption.LoadingPhaseThreadCountConverter
    /// A converter for loading phase thread count. Takes FLAG_SYNTAX. Caps at 20 for tests.
    pub LoadingPhaseThreadCountConverter =>
        ResourceIntegerConverter::new(host_cpus_supplier, 1, i32::MAX)
            .with_check_and_limit(|c, value| check_and_limit(c, value, is_in_test()))
);

// Java: com/google/devtools/build/lib/runtime/LoadingPhaseThreadsOption.java:59 LoadingPhaseThreadsOption.LoadingPhaseThreadCountConverter.checkAndLimit
fn check_and_limit(
    c: &ResourceIntegerConverter,
    mut value: i32,
    in_test: bool,
) -> Result<i32, OptionsParsingException> {
    // Cap thread count while running tests. Test cases are typically small and large thread
    // pools vying for a relatively small number of CPU cores may induce non-optimal
    // performance.
    if in_test {
        value = value.min(20);
        // PORT: Java logs "Running under a test; loading_phase_threads capped at %d".
    }
    c.default_check_and_limit(value)
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::local_host_capacity::{LocalHostResources, set_local_host_capacity};
    use super::super::test_util::parse;
    use super::*;

    fn capacity(cpu_usage: f64) {
        set_local_host_capacity(Some(LocalHostResources {
            memory_mb: 1.0,
            cpu_usage,
        }));
    }

    /// The converter as Java's tests see it: running under a test (TEST_TMPDIR is set by
    /// `bazel test`).
    fn converter_in_test() -> ResourceIntegerConverter {
        ResourceIntegerConverter::new(host_cpus_supplier, 1, i32::MAX)
            .with_check_and_limit(|c, value| check_and_limit(c, value, true))
    }

    // Ported from src/test/java/com/google/devtools/build/lib/runtime/LoadingPhaseThreadsConverterTest.java.

    #[test]
    fn test_auto_loading_phase_threads_uses_hardware_settings() {
        capacity(7.0);
        assert_eq!(converter_in_test().convert_int("auto").unwrap(), 7);
        set_local_host_capacity(None);
    }

    #[test]
    fn test_auto_loading_phase_threads_capped_for_tests() {
        capacity(123.0);
        assert_eq!(converter_in_test().convert_int("auto").unwrap(), 20);
        set_local_host_capacity(None);
    }

    #[test]
    fn test_explicit_loading_phase_threads_capped_for_tests() {
        assert_eq!(converter_in_test().convert_int("200").unwrap(), 20);
    }

    #[test]
    fn test_explicit_loading_phase_threads_must_be_at_least1() {
        let thrown = LoadingPhaseThreadCountConverter::inner()
            .convert_int("0")
            .unwrap_err();
        assert!(thrown.get_message().contains("must be at least 1"));
    }

    // Blaze-specific.
    #[test]
    fn threads() {
        capacity(3.0);
        assert_eq!(
            LoadingPhaseThreadCountConverter::inner()
                .convert_int("auto")
                .unwrap(),
            3
        );
        set_local_host_capacity(None);
        assert_eq!(
            parse::<LoadingPhaseThreadsOption>(&["--loading_phase_threads=3"]).threads,
            3
        );
    }
}
