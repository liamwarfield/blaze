//! Port of `com.google.devtools.build.lib.util.DescribableExecutionUnit`.

use blaze_cmdline::Label;
use indexmap::IndexMap;

/// Something executable that can be described by
/// `CommandFailureUtils::describe_command_failure`.
// Java: com/google/devtools/build/lib/util/DescribableExecutionUnit.java:24 DescribableExecutionUnit
pub trait DescribableExecutionUnit {
    // Java: com/google/devtools/build/lib/util/DescribableExecutionUnit.java:27 DescribableExecutionUnit.getTargetDescription
    fn get_target_description(&self) -> Option<String> {
        None
    }

    /// Returns the command (the first element) and its arguments.
    fn get_arguments(&self) -> Vec<String>;

    /// Returns the initial environment of the process. If `None`, the environment is inherited
    /// from the parent process.
    fn get_environment(&self) -> Option<IndexMap<String, String>>;

    /// Returns the Label of the execution platform for the command, if any.
    // Java: com/google/devtools/build/lib/util/DescribableExecutionUnit.java:42 DescribableExecutionUnit.getExecutionPlatformLabel
    fn get_execution_platform_label(&self) -> Option<Label> {
        None
    }

    /// Returns the configuration hash for this command, if any.
    // Java: com/google/devtools/build/lib/util/DescribableExecutionUnit.java:48 DescribableExecutionUnit.getConfigurationChecksum
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }

    fn get_mnemonic(&self) -> String;
}
