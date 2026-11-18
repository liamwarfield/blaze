//! Transcription of `com/google/devtools/build/lib/runtime/HostJvmStartupOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/HostJvmStartupOptions.java:30 HostJvmStartupOptions
options! {
    /// Options that will be evaluated by the blaze client startup code only.
    ///
    /// The only reason we have this interface is that we'd like to print a nice
    /// help page for the client startup options. These options do not affect the
    /// server's behavior in any way.
    pub struct HostJvmStartupOptions {
        // NOTE: purely decorative! See BlazeServerStartupOptions.
        #[option(
            name = "server_javabase",
            default_value = "",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            value_help = "<jvm path>",
            help = "Path to the JVM used to execute Bazel itself.",
        )]
        pub server_javabase: String,

        // NOTE: purely decorative!  See BlazeServerStartupOptions.
        #[option(
            name = "host_jvm_args",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            value_help = "<jvm_arg>",
            help = "Flags to pass to the JVM executing Blaze.",
        )]
        pub host_jvm_args: Vec<String>,

        // NOTE: purely decorative!  See BlazeServerStartupOptions.
        #[option(
            name = "host_jvm_debug",
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            expansion = ["--host_jvm_args=-agentlib:jdwp=transport=dt_socket,server=y,address=5005"],
            help = concat!(
                "Convenience option to add some additional JVM startup flags, which cause ",
                "the JVM to wait during startup until you connect from a JDWP-compliant debugger ",
                "(like Eclipse) to port 5005.",
            ),
        )]
        pub host_jvm_debug: Void,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const HOST_JVM_STARTUP_OPTIONS_OPTION_NAMES: &[&str] =
        &["server_javabase", "host_jvm_args", "host_jvm_debug"];

    #[test]
    fn host_jvm_startup_options_option_names() {
        super::super::test_util::check_option_names(
            HostJvmStartupOptions::class(),
            HOST_JVM_STARTUP_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn host_jvm_startup_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<HostJvmStartupOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn host_jvm_debug_expansion() {
        let o = parse::<HostJvmStartupOptions>(&["--host_jvm_args=-Xmx1g", "--host_jvm_debug"]);
        assert_eq!(
            o.host_jvm_args,
            vec![
                "-Xmx1g",
                "-agentlib:jdwp=transport=dt_socket,server=y,address=5005"
            ]
        );
    }
}
