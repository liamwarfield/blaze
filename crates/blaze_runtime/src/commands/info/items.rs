//! The hardwired info items of `blaze info` (one struct per Java class in
//! `com.google.devtools.build.lib.runtime.commands.info`).

use blaze_optdefs::command::build_language_options::{
    self, BuildLanguageOptions, SemanticsValue, StarlarkSemanticsKey, Utf8EnforcementMode,
};
use blaze_optdefs::command::host_jvm_startup_options::HostJvmStartupOptions;
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_vfs::Path;

use crate::command_environment::CommandEnvironment;
use crate::info_item::{ConfigurationSupplier, InfoItem, print};
use crate::local_resources::LocalHostCapacity;
use crate::module::AbruptExitException;
use crate::options_result::OptionsParsingResult;
use crate::version_info::BlazeVersionInfo;

/// The `InfoItem` base-class accessors for a struct with `name`, `description` and `hidden`
/// fields.
macro_rules! info_item_base {
    () => {
        fn get_name(&self) -> &str {
            &self.name
        }

        fn get_description(&self) -> &str {
            &self.description
        }

        fn is_hidden(&self) -> bool {
            self.hidden
        }
    };
}
pub(crate) use info_item_base;

/// Declares an info item struct with the `InfoItem` base fields and its constructor fields.
macro_rules! info_item_struct {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        pub struct $name {
            name: String,
            description: String,
            hidden: bool,
            $($(#[$fm])* $field: $ty,)*
        }
    };
}

/// `String.valueOf(path)` for a nullable path.
fn print_nullable_path(path: Option<&Path>) -> Vec<u8> {
    match path {
        Some(p) => print(p),
        None => print("null"),
    }
}

info_item_struct! {
    /// Info item for the workspace directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkspaceInfoItem.java:25 WorkspaceInfoItem
    WorkspaceInfoItem {}
}

impl WorkspaceInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkspaceInfoItem.java:26 WorkspaceInfoItem.<init>
    pub fn new() -> Self {
        WorkspaceInfoItem {
            name: "workspace".to_string(),
            description: "The working directory of the server.".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for WorkspaceInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkspaceInfoItem.java:31 WorkspaceInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print_nullable_path(
            env.get_runtime().get_workspace().get_workspace(),
        ))
    }
}

info_item_struct! {
    /// Info item for the install_base directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallBaseInfoItem.java:25 InstallBaseInfoItem
    InstallBaseInfoItem {}
}

impl InstallBaseInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallBaseInfoItem.java:26 InstallBaseInfoItem.<init>
    pub fn new() -> Self {
        InstallBaseInfoItem {
            name: "install_base".to_string(),
            description: "The installation base directory.".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for InstallBaseInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallBaseInfoItem.java:31 InstallBaseInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(env.get_runtime().get_workspace().get_install_base()))
    }
}

info_item_struct! {
    /// Info item for the local checksum of the Blaze server.
    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallMd5InfoItem.java:25 InstallMd5InfoItem
    InstallMd5InfoItem {}
}

impl InstallMd5InfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallMd5InfoItem.java:26 InstallMd5InfoItem.<init>
    pub fn new() -> Self {
        InstallMd5InfoItem {
            name: "install_md5".to_string(),
            description: "The logical checksum of the Blaze server.".to_string(),
            hidden: true,
        }
    }
}

impl InfoItem for InstallMd5InfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/InstallMd5InfoItem.java:31 InstallMd5InfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(
            env.get_directories()
                .get_install_md5()
                .expect("NullPointerException: getInstallMD5()"),
        ))
    }
}

info_item_struct! {
    /// Info item for the output_base directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputBaseInfoItem.java:25 OutputBaseInfoItem
    OutputBaseInfoItem {}
}

impl OutputBaseInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputBaseInfoItem.java:26 OutputBaseInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        OutputBaseInfoItem {
            name: "output_base".to_string(),
            description: format!(
                "A directory for shared {} state as well as tool and strategy specific subdirectories.",
                product_name
            ),
            hidden: false,
        }
    }
}

impl InfoItem for OutputBaseInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputBaseInfoItem.java:36 OutputBaseInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(env.get_runtime().get_workspace().get_output_base()))
    }
}

info_item_struct! {
    /// Info item for the execution_root directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ExecutionRootInfoItem.java:25 ExecutionRootInfoItem
    ExecutionRootInfoItem {}
}

impl ExecutionRootInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/ExecutionRootInfoItem.java:26 ExecutionRootInfoItem.<init>
    pub fn new() -> Self {
        ExecutionRootInfoItem {
            name: "execution_root".to_string(),
            description: "A directory that makes all input and output files visible to the build."
                .to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for ExecutionRootInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/ExecutionRootInfoItem.java:34 ExecutionRootInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(
            env.get_directories()
                .get_exec_root(env.get_workspace_name()),
        ))
    }
}

info_item_struct! {
    /// Info item for the output_path directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputPathInfoItem.java:25 OutputPathInfoItem
    OutputPathInfoItem {}
}

impl OutputPathInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputPathInfoItem.java:26 OutputPathInfoItem.<init>
    pub fn new() -> Self {
        OutputPathInfoItem {
            name: "output_path".to_string(),
            description: "Output directory".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for OutputPathInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/OutputPathInfoItem.java:31 OutputPathInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(
            env.get_directories()
                .get_output_path(env.get_workspace_name()),
        ))
    }
}

info_item_struct! {
    /// Info item for the effective current client environment.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ClientEnv.java:24 ClientEnv
    ClientEnv {}
}

impl ClientEnv {
    // Java: com/google/devtools/build/lib/runtime/commands/info/ClientEnv.java:25 ClientEnv.<init>
    pub fn new() -> Self {
        ClientEnv {
            name: "client-env".to_string(),
            description:
                "The specifications that need to be added to the project-specific rc file \
                          to freeze the current client environment"
                    .to_string(),
            hidden: true,
        }
    }
}

impl InfoItem for ClientEnv {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/ClientEnv.java:34 ClientEnv.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        let mut result = String::new();
        for (key, value) in env.get_allowlisted_action_env() {
            // TODO(bazel-team): as the syntax of our rc-files does not support to express
            // new-lines in values, we produce syntax errors if the value of the entry contains a
            // newline character.
            result.push_str(&format!("build --action_env={}={}\n", key, value));
        }
        for (key, value) in env.get_allowlisted_test_env() {
            // TODO(bazel-team): as the syntax of our rc-files does not support to express
            // new-lines in values, we produce syntax errors if the value of the entry contains a
            // newline character.
            result.push_str(&format!("build --test_env={}={}\n", key, value));
        }
        Ok(print(result))
    }
}

info_item_struct! {
    /// Info item for the {blaze,bazel}-bin directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeBinInfoItem.java:26 BlazeBinInfoItem
    BlazeBinInfoItem {}
}

impl BlazeBinInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeBinInfoItem.java:27 BlazeBinInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        BlazeBinInfoItem {
            name: format!("{}-bin", product_name),
            description: "Configuration dependent directory for binaries.".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for BlazeBinInfoItem {
    info_item_base!();

    // This is one of the three (non-hidden) info items that require a configuration, because the
    // corresponding paths contain the short name. Maybe we should recommend using the symlinks
    // or make them hidden by default?
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeBinInfoItem.java:35 BlazeBinInfoItem.get
    fn get(
        &self,
        configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(configuration_supplier.get_bin_directory()?))
    }
}

info_item_struct! {
    /// Info item for the {blaze,bazel}-genfiles directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeGenfilesInfoItem.java:26 BlazeGenfilesInfoItem
    BlazeGenfilesInfoItem {}
}

impl BlazeGenfilesInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeGenfilesInfoItem.java:27 BlazeGenfilesInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        BlazeGenfilesInfoItem {
            name: format!("{}-genfiles", product_name),
            description: "Configuration dependent directory for generated files.".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for BlazeGenfilesInfoItem {
    info_item_base!();

    // This is one of the three (non-hidden) info items that require a configuration, because the
    // corresponding paths contain the short name. Maybe we should recommend using the symlinks
    // or make them hidden by default?
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeGenfilesInfoItem.java:36 BlazeGenfilesInfoItem.get
    fn get(
        &self,
        configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(configuration_supplier.get_genfiles_directory()?))
    }
}

info_item_struct! {
    /// Info item for the {blaze,bazel}-testlogs directory.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeTestlogsInfoItem.java:26 BlazeTestlogsInfoItem
    BlazeTestlogsInfoItem {}
}

impl BlazeTestlogsInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeTestlogsInfoItem.java:27 BlazeTestlogsInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        BlazeTestlogsInfoItem {
            name: format!("{}-testlogs", product_name),
            description: "Configuration dependent directory for logs from a test run.".to_string(),
            hidden: false,
        }
    }
}

impl InfoItem for BlazeTestlogsInfoItem {
    info_item_base!();

    // This is one of the three (non-hidden) info items that require a configuration, because the
    // corresponding paths contain the short name. Maybe we should recommend using the symlinks
    // or make them hidden by default?
    // Java: com/google/devtools/build/lib/runtime/commands/info/BlazeTestlogsInfoItem.java:38 BlazeTestlogsInfoItem.get
    fn get(
        &self,
        configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(configuration_supplier.get_test_logs_directory()?))
    }
}

info_item_struct! {
    /// Info item for release.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ReleaseInfoItem.java:24 ReleaseInfoItem
    ReleaseInfoItem {}
}

impl ReleaseInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/ReleaseInfoItem.java:25 ReleaseInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        ReleaseInfoItem {
            name: "release".to_string(),
            description: format!("{} release identifier", product_name),
            hidden: false,
        }
    }
}

impl InfoItem for ReleaseInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/ReleaseInfoItem.java:30 ReleaseInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(BlazeVersionInfo::instance().get_release_name()))
    }
}

info_item_struct! {
    /// Info item for server_pid.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerPidInfoItem.java:23 ServerPidInfoItem
    ServerPidInfoItem {}
}

impl ServerPidInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerPidInfoItem.java:24 ServerPidInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        ServerPidInfoItem {
            name: "server_pid".to_string(),
            description: format!("{} process id", product_name),
            hidden: false,
        }
    }
}

impl InfoItem for ServerPidInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerPidInfoItem.java:29 ServerPidInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(std::process::id()))
    }
}

info_item_struct! {
    /// Info item for server_log path.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerLogInfoItem.java:27 ServerLogInfoItem
    ServerLogInfoItem {}
}

impl ServerLogInfoItem {
    /// Constructs an info item for the server log path.
    ///
    /// `product_name`: name of the tool whose server log path will be queried.
    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerLogInfoItem.java:35 ServerLogInfoItem.<init>
    pub fn new(product_name: &str) -> Self {
        ServerLogInfoItem {
            name: "server_log".to_string(),
            description: format!("{} server log path", product_name),
            hidden: false,
        }
    }
}

impl InfoItem for ServerLogInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/ServerLogInfoItem.java:40 ServerLogInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        match crate::server_log::get_server_log_path() {
            Ok(path) => Ok(print(path.unwrap_or_default())),
            // PORT: the logger.atWarning line is not written (no server logger).
            Err(_) => Ok(print("UNKNOWN LOG LOCATION")),
        }
    }
}

info_item_struct! {
    /// Info item for package_path.
    // Java: com/google/devtools/build/lib/runtime/commands/info/PackagePathInfoItem.java:28 PackagePathInfoItem
    PackagePathInfoItem {
        /// `commandOptions.getOptions(PackageOptions.class)`, read at construction (the options
        /// are immutable). Outer `None`: Java's `commandOptions` is null; inner `None`: the parser
        /// has no `PackageOptions`.
        command_options: Option<Option<PackageOptions>>,
    }
}

impl PackagePathInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/PackagePathInfoItem.java:31 PackagePathInfoItem.<init>
    pub fn new(command_options: Option<&OptionsParsingResult>) -> Self {
        PackagePathInfoItem {
            name: "package_path".to_string(),
            description: "The search path for resolving package labels.".to_string(),
            hidden: false,
            command_options: command_options.map(|o| o.get_options::<PackageOptions>()),
        }
    }
}

impl InfoItem for PackagePathInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/PackagePathInfoItem.java:37 PackagePathInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        let package_options = self
            .command_options
            .as_ref()
            .expect("NullPointerException: checkNotNull(commandOptions)")
            .as_ref()
            .expect("NullPointerException: PackageOptions");
        Ok(print(package_options.package_path.join(":")))
    }
}

/// `StringUtilities.prettyPrintBytes`: an easy-to-read string approximation of a number of
/// bytes, e.g. "21MB". Note, these are IEEE units, i.e. decimal not binary powers.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:96 StringUtilities.prettyPrintBytes
pub fn pretty_print_bytes(bytes: i64) -> String {
    if (bytes as f64) < 1E4 {
        // up to 10KB
        format!("{}B", bytes)
    } else if (bytes as f64) < 1E7 {
        // up to 10MB
        format!("{}KB", (bytes as f64 / 1E3) as i32)
    } else if (bytes as f64) < 1E11 {
        // up to 100GB
        format!("{}MB", (bytes as f64 / 1E6) as i32)
    } else {
        format!("{}GB", (bytes as f64 / 1E9) as i32)
    }
}

/// `InfoItemUtils.getMemoryUsage`: the heap figures of the server.
///
/// PORT: Blaze has no JVM; the figures are those of ruling D15 (`crate::process_memory`).
// Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemUtils.java:26 InfoItemUtils.getMemoryUsage
pub fn get_memory_usage() -> crate::process_memory::MemoryUsage {
    crate::process_memory::get_heap_memory_usage()
}

/// Defines an info item whose value is computed from the command environment alone.
macro_rules! env_info_item {
    (
        $(#[$m:meta])* $ty:ident, $name:expr, $description:expr, $hidden:expr,
        $(#[$gm:meta])* |$env:ident| $body:expr
    ) => {
        info_item_struct! {
            $(#[$m])*
            $ty {}
        }

        impl $ty {
            pub fn new() -> Self {
                $ty {
                    name: $name.to_string(),
                    description: $description.to_string(),
                    hidden: $hidden,
                }
            }
        }

        impl InfoItem for $ty {
            info_item_base!();

            $(#[$gm])*
            fn get(
                &self,
                _configuration_supplier: &mut dyn ConfigurationSupplier,
                $env: &mut CommandEnvironment,
            ) -> Result<Vec<u8>, AbruptExitException> {
                Ok($body)
            }
        }
    };
}

env_info_item!(
    /// Info item for the used heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeInfoItem.java:24 UsedHeapSizeInfoItem
    UsedHeapSizeInfoItem,
    "used-heap-size",
    "The amount of used memory in bytes. Note that this is not a good indicator of the actual \
     memory use, as it includes any remaining inaccessible memory.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeInfoItem.java:35 UsedHeapSizeInfoItem.get
    |_env| print(pretty_print_bytes(get_memory_usage().get_used()))
);

env_info_item!(
    /// Info item for the used heap size after garbage collection.
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeAfterGcInfoItem.java:27 UsedHeapSizeAfterGcInfoItem
    UsedHeapSizeAfterGcInfoItem,
    "used-heap-size-after-gc",
    "The amount of used memory in bytes after a call to System.gc().",
    true,
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeAfterGcInfoItem.java:36 UsedHeapSizeAfterGcInfoItem.get
    // PORT: `System.gc()` is the allocator's release of free memory (D15); the subtraction of
    // `HeapOffsetHelper.getSizeOfFillerArrayOnHeap` (JVM filler arrays in a heap histogram) is
    // 0: Blaze's heap has none.
    |_env| {
        crate::process_memory::release_free_memory();
        print(pretty_print_bytes(get_memory_usage().get_used()))
    }
);

env_info_item!(
    /// Info item for the committed heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/CommittedHeapSizeInfoItem.java:24 CommittedHeapSizeInfoItem
    CommittedHeapSizeInfoItem,
    "committed-heap-size",
    "The amount of memory in bytes that is committed for the Java virtual machine to use",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/CommittedHeapSizeInfoItem.java:34 CommittedHeapSizeInfoItem.get
    |_env| print(pretty_print_bytes(get_memory_usage().get_committed()))
);

env_info_item!(
    /// Info item for the max heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/MaxHeapSizeInfoItem.java:24 MaxHeapSizeInfoItem
    MaxHeapSizeInfoItem,
    "max-heap-size",
    "The maximum amount of memory in bytes that can be used for memory management.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/MaxHeapSizeInfoItem.java:33 MaxHeapSizeInfoItem.get
    |_env| print(pretty_print_bytes(get_memory_usage().get_max()))
);

env_info_item!(
    /// Info item for the gc-time.
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcTimeInfoItem.java:25 GcTimeInfoItem
    GcTimeInfoItem,
    "gc-time",
    "The approximate accumulated time spend on garbage collection.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcTimeInfoItem.java:31 GcTimeInfoItem.get
    // The documentation is not very clear on what it means to have more than one GC MXBean, so
    // we just sum them up.
    // PORT: Blaze has no garbage collector (D15): the sum over no beans.
    |_env| {
        let gc_time: i64 = 0;
        print(format!("{}ms", gc_time))
    }
);

env_info_item!(
    /// Info item for the gc-count.
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcCountInfoItem.java:25 GcCountInfoItem
    GcCountInfoItem,
    "gc-count",
    "Number of garbage collection runs.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcCountInfoItem.java:31 GcCountInfoItem.get
    // The documentation is not very clear on what it means to have more than one GC MXBean, so
    // we just sum them up.
    // PORT: Blaze has no garbage collector (D15): the sum over no beans.
    |_env| {
        let gc_count: i64 = 0;
        print(format!("{}", gc_count))
    }
);

/// The `release` file of the JDK Bazel's server runs on (`<java.home>/release`), as
/// `KEY="value"` pairs.
///
/// PORT: Blaze has no JVM; ruling D15 has `java-runtime` and `java-vm` describe the server
/// javabase (`JavaHomeInfoItem::get_server_javabase`) from its `release` file. A system property
/// the file does not give keeps the default Java passes to `System.getProperty` (the minimal JDK
/// embedded in Bazel's install base names only `JAVA_VERSION` and `MODULES`).
struct JdkRelease {
    entries: Vec<(String, String)>,
}

impl JdkRelease {
    fn read(env: &CommandEnvironment) -> JdkRelease {
        let java_home = JavaHomeInfoItem::get_server_javabase(env);
        let release = Path::create_from_str(&java_home, env.get_runtime().get_file_system())
            .get_relative_str("release");
        let mut content = Vec::new();
        if let Ok(mut input) = release.get_input_stream() {
            let _ = std::io::Read::read_to_end(&mut input, &mut content);
        }
        Self::parse(&content)
    }

    fn parse(content: &[u8]) -> JdkRelease {
        let text = blaze_util::string_encoding::latin1_decode(content);
        let mut entries = Vec::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                entries.push((key.trim().to_string(), value.to_string()));
            }
        }
        JdkRelease { entries }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `java.runtime.version` and `java.vm.version`: `JAVA_RUNTIME_VERSION`, else `JAVA_VERSION`.
    fn runtime_version(&self) -> Option<&str> {
        self.get("JAVA_RUNTIME_VERSION")
            .or_else(|| self.get("JAVA_VERSION"))
    }

    /// `java.vendor` and `java.vm.vendor`: `IMPLEMENTOR`.
    fn vendor(&self) -> Option<&str> {
        self.get("IMPLEMENTOR")
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaRuntimeInfoItem.java:29 JavaRuntimeInfoItem.get
    fn java_runtime(&self) -> String {
        // The release file has no key for java.runtime.name.
        format!(
            "{} (build {}) by {}",
            "Unknown runtime",
            self.runtime_version().unwrap_or("unknown"),
            self.vendor().unwrap_or("unknown")
        )
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaVirtualMachineInfoItem.java:29 JavaVirtualMachineInfoItem.get
    fn java_vm(&self) -> String {
        // The release file has no key for java.vm.name and java.vm.info.
        format!(
            "{} (build {}, {}) by {}",
            "Unknown VM",
            self.runtime_version().unwrap_or("unknown"),
            "unknown",
            self.vendor().unwrap_or("unknown")
        )
    }
}

env_info_item!(
    /// Info item for the name and version of the Java runtime environment.
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaRuntimeInfoItem.java:23 JavaRuntimeInfoItem
    JavaRuntimeInfoItem,
    "java-runtime",
    "Name and version of the current Java runtime environment.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaRuntimeInfoItem.java:29 JavaRuntimeInfoItem.get
    |env| print(JdkRelease::read(env).java_runtime())
);

env_info_item!(
    /// Info item for the name and version of the Java VM.
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaVirtualMachineInfoItem.java:23 JavaVirtualMachineInfoItem
    JavaVirtualMachineInfoItem,
    "java-vm",
    "Name and version of the current Java virtual machine.",
    false,
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaVirtualMachineInfoItem.java:29 JavaVirtualMachineInfoItem.get
    |env| print(JdkRelease::read(env).java_vm())
);

info_item_struct! {
    /// Info item for the location of the Java runtime.
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaHomeInfoItem.java:25 JavaHomeInfoItem
    JavaHomeInfoItem {}
}

impl JavaHomeInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaHomeInfoItem.java:26 JavaHomeInfoItem.<init>
    pub fn new() -> Self {
        JavaHomeInfoItem {
            name: "java-home".to_string(),
            description: "Location of the current Java runtime.".to_string(),
            hidden: false,
        }
    }

    /// `System.getProperty("java.home")` of Bazel's server JVM: the server javabase the client
    /// chose (`StartupOptions::GetServerJavabaseAndType` in `src/main/cpp/startup_options.cc`):
    /// `--server_javabase` if set, else the embedded JDK `<install_base>/embedded_tools/jdk` if
    /// its `bin/java` is executable, else the system JDK.
    ///
    /// PORT: Blaze has no JVM; this computes the value Bazel's server would report from the
    /// startup options and the install base. The system-JDK fallback is not ported.
    pub(crate) fn get_server_javabase(env: &CommandEnvironment) -> String {
        let server_javabase = env
            .get_runtime()
            .get_startup_options_provider()
            .get_options::<HostJvmStartupOptions>()
            .map(|o| o.server_javabase)
            .unwrap_or_default();
        if !server_javabase.is_empty() {
            if !server_javabase.starts_with('/') {
                // The client makes it absolute against its working directory.
                blaze_util::unported!(
                    "StartupOptions::ProcessArg(--server_javabase)",
                    "relative --server_javabase={}",
                    server_javabase
                );
            }
            return server_javabase;
        }
        let bundled_jre_path = env
            .get_directories()
            .get_install_base()
            .get_relative_str("embedded_tools")
            .get_relative_str("jdk");
        let java = bundled_jre_path.get_relative_str("bin/java");
        let java_c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(
            &*blaze_util::string_encoding::internal_to_platform(java.get_path_string()),
        ))
        .expect("NUL in path");
        // blaze_util::CanExecuteFile: access(X_OK) on a regular file.
        // SAFETY: java_c is a valid NUL-terminated string.
        let can_execute =
            java.is_file() && unsafe { libc::access(java_c.as_ptr(), libc::X_OK) } == 0;
        if can_execute {
            return bundled_jre_path.get_path_string().to_string();
        }
        blaze_util::unported!(
            "StartupOptions::GetSystemJavabase",
            "no embedded JDK at {}",
            bundled_jre_path
        )
    }
}

impl InfoItem for JavaHomeInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaHomeInfoItem.java:31 JavaHomeInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        // StringEncoding.platformToInternal(System.getProperty("java.home")): the javabase is
        // computed from the install base, which already is an internal string; java.home is
        // never null here.
        let java_home = Self::get_server_javabase(env);
        // Tunnel through a Path object in order to normalize the representation of the path.
        let java_home_path = Path::create_from_str(&java_home, env.get_runtime().get_file_system());
        Ok(print(java_home_path.get_path_string()))
    }
}

info_item_struct! {
    /// Info item for the current character encoding settings.
    // Java: com/google/devtools/build/lib/runtime/commands/info/CharacterEncodingInfoItem.java:24 CharacterEncodingInfoItem
    CharacterEncodingInfoItem {}
}

/// `-Dfile.encoding=ISO-8859-1`, which Bazel's client always passes to the server JVM
/// (`GetServerJvmArgs`... `result.push_back("-Dfile.encoding=ISO-8859-1")` in
/// `src/main/cpp/blaze.cc`).
const CLIENT_FILE_ENCODING: &str = "ISO-8859-1";

impl CharacterEncodingInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/CharacterEncodingInfoItem.java:25 CharacterEncodingInfoItem.<init>
    pub fn new() -> Self {
        CharacterEncodingInfoItem {
            name: "character-encoding".to_string(),
            description: "Information about the character encoding used by the running JVM."
                .to_string(),
            hidden: false,
        }
    }

    /// `Charset.defaultCharset().name()`: the canonical name of the charset named by
    /// `file.encoding` (JDK 18+ honours an explicit `-Dfile.encoding`).
    fn default_charset_name(file_encoding: &str) -> &'static str {
        match file_encoding {
            "ISO-8859-1" => "ISO-8859-1",
            "UTF-8" => "UTF-8",
            other => blaze_util::unported!("Charset.defaultCharset", "file.encoding={}", other),
        }
    }

    /// `System.getProperty("sun.jnu.encoding")`: the JVM sets it from `nl_langinfo(CODESET)` of
    /// its locale (`ParseLocale` in the JDK's `java_props_md.c`). On Linux, Bazel's client sets
    /// `LC_ALL` for the server to the first of `en_US.ISO-8859-1`, `C.UTF-8`, `en_US.UTF-8` that
    /// `newlocale` accepts (`GetServerEnvironment`... in `src/main/cpp/blaze.cc`); otherwise the
    /// server inherits the client's locale environment.
    fn sun_jnu_encoding() -> String {
        let mut want_locale: Option<&str> = None;
        for candidate_locale in ["en_US.ISO-8859-1", "C.UTF-8", "en_US.UTF-8"] {
            let c = std::ffi::CString::new(candidate_locale).unwrap();
            // SAFETY: c is a valid NUL-terminated string; the locale is freed right away.
            let locale =
                unsafe { libc::newlocale(libc::LC_CTYPE_MASK, c.as_ptr(), std::ptr::null_mut()) };
            if !locale.is_null() {
                unsafe { libc::freelocale(locale) };
                want_locale = Some(candidate_locale);
                break;
            }
        }
        // With no candidate, the JVM uses the locale from the environment ("").
        let name = std::ffi::CString::new(want_locale.unwrap_or("")).unwrap();
        // SAFETY: as above; nl_langinfo_l's result is copied before the locale is freed.
        let codeset = unsafe {
            let locale = libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut());
            if locale.is_null() {
                String::new()
            } else {
                let p = libc::nl_langinfo_l(libc::CODESET, locale);
                let s = if p.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                };
                libc::freelocale(locale);
                s
            }
        };
        // Convert the bare "646" used on Solaris to a proper IANA name; an empty codeset means
        // ISO8859-1; EUC-JP is remapped on Linux.
        match codeset.as_str() {
            "646" => "ISO646-US".to_string(),
            "" => "ISO8859-1".to_string(),
            "EUC-JP" => "EUC-JP-LINUX".to_string(),
            _ => codeset,
        }
    }
}

impl InfoItem for CharacterEncodingInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/CharacterEncodingInfoItem.java:33 CharacterEncodingInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        // PORT: Blaze has no JVM; the three values are those of Bazel's server JVM, derived from
        // the flags and locale Bazel's client gives it (see CLIENT_FILE_ENCODING and
        // sun_jnu_encoding).
        Ok(print(format!(
            "file.encoding = {}, defaultCharset = {}, sun.jnu.encoding = {}",
            CLIENT_FILE_ENCODING,
            Self::default_charset_name(CLIENT_FILE_ENCODING),
            Self::sun_jnu_encoding()
        )))
    }
}

info_item_struct! {
    /// Info item for the default package. It is deprecated, it still works, when explicitly
    /// requested, but are not shown by default. It prints multi-line messages and thus don't play
    /// well with grep. We don't print them unless explicitly requested.
    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultsPackageInfoItem.java:31 DefaultsPackageInfoItem
    DefaultsPackageInfoItem {}
}

impl DefaultsPackageInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultsPackageInfoItem.java:32 DefaultsPackageInfoItem.<init>
    pub fn new() -> Self {
        DefaultsPackageInfoItem {
            name: "defaults-package".to_string(),
            description: "Obsolete. Retained for backwards compatibility.".to_string(),
            hidden: true,
        }
    }
}

impl InfoItem for DefaultsPackageInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultsPackageInfoItem.java:37 DefaultsPackageInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(""))
    }
}

info_item_struct! {
    /// Info item for the build language. It is deprecated, it still works, when explicitly
    /// requested, but are not shown by default. It prints multi-line messages and thus don't play
    /// well with grep. We don't print them unless explicitly requested.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:59 BuildLanguageInfoItem
    BuildLanguageInfoItem {}
}

impl BuildLanguageInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:61 BuildLanguageInfoItem.<init>
    pub fn new() -> Self {
        BuildLanguageInfoItem {
            name: "build-language".to_string(),
            description: "A protobuffer with the build language structure".to_string(),
            hidden: true,
        }
    }
}

impl BuildLanguageInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:85 BuildLanguageInfoItem.loadStarlarkBuiltins
    fn load_starlark_builtins(
        env: &CommandEnvironment,
        executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
    ) -> Result<
        std::sync::Arc<blaze_sky_loading::starlark_builtins::StarlarkBuiltinsValue>,
        AbruptExitException,
    > {
        use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
        let key = blaze_sky_loading::starlark_builtins::StarlarkBuiltinsKey::key(false);
        let reporter: std::sync::Arc<dyn blaze_events::ExtendedEventHandler> =
            env.get_reporter().clone();
        let result = executor
            .evaluate(
                &[blaze_skyframe::SkyKey::new(key.clone())],
                /* keep_going= */ false,
                blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT,
                reporter,
            )
            .map_err(|_| {
                // InfoCommand.exec: `catch (InterruptedException e)`.
                AbruptExitException::new(crate::exit_code::DetailedExitCode::of(FailureDetail {
                    message: "info interrupted".to_string(),
                    category: Some(Category::Interrupted(failure_details::Interrupted {
                        code: failure_details::interrupted::Code::Interrupted as i32,
                    })),
                }))
            })?;
        if result.has_error() {
            return Err(AbruptExitException::new(
                crate::exit_code::DetailedExitCode::of(FailureDetail {
                    message: "Failed to load Starlark builtins".to_string(),
                    category: Some(Category::InfoCommand(
                        failure_details::InfoCommand::default(),
                    )),
                }),
            ));
        }
        Ok(result
            .get(&key)
            .expect("StarlarkBuiltinsValue of a successful evaluation"))
    }
}

impl InfoItem for BuildLanguageInfoItem {
    info_item_base!();

    // Requires CommandEnvironment.syncPackageLoading to be called in order to initialize the
    // skyframe executor.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:66 BuildLanguageInfoItem.needsSyncPackageLoading
    fn needs_sync_package_loading(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:73 BuildLanguageInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        // checkNotNull(env): by type.
        // PORT: Java reads the workspace's SkyframeExecutor, synced by
        // ensureSyncPackageLoading; Blaze creates the command's executor here.
        let executor = crate::commands::query::create_skyframe_executor(env, None)
            .map_err(|r| AbruptExitException::new(r.get_detailed_exit_code().clone()))?;
        let _after_command = crate::commands::query::AfterCommand(&executor);
        let builtins = Self::load_starlark_builtins(env, &executor)?;
        Ok(crate::info_item::print_bytes(
            super::build_language::get_build_language_definition(
                &super::build_language::get_builtin_rule_classes(
                    &builtins,
                    executor.get_rule_class_provider(),
                    /* include_macro_wrapped_rules= */ true,
                ),
            ),
        ))
    }
}

info_item_struct! {
    /// Info item for the default package path. It is deprecated, it still works, when explicitly
    /// requested, but are not shown by default. It prints multi-line messages and thus don't play
    /// well with grep. We don't print them unless explicitly requested.
    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultPackagePathInfoItem.java:33 DefaultPackagePathInfoItem
    DefaultPackagePathInfoItem {
        /// See [`PackagePathInfoItem`].
        command_options: Option<Option<PackageOptions>>,
    }
}

impl DefaultPackagePathInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultPackagePathInfoItem.java:38 DefaultPackagePathInfoItem.<init>
    pub fn new(command_options: Option<&OptionsParsingResult>) -> Self {
        DefaultPackagePathInfoItem {
            name: "default-package-path".to_string(),
            description: "The default package path".to_string(),
            hidden: true,
            command_options: command_options.map(|o| o.get_options::<PackageOptions>()),
        }
    }
}

impl InfoItem for DefaultPackagePathInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/DefaultPackagePathInfoItem.java:44 DefaultPackagePathInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        let package_options = self
            .command_options
            .as_ref()
            .expect("NullPointerException: checkNotNull(commandOptions)")
            .as_ref()
            .expect("NullPointerException: PackageOptions");
        Ok(print(package_options.package_path.join(":")))
    }
}

info_item_struct! {
    /// Info item for the effective current set of Starlark semantics option values.
    ///
    /// This is hidden because its output is verbose and may be multiline.
    // Java: com/google/devtools/build/lib/runtime/commands/info/StarlarkSemanticsInfoItem.java:31 StarlarkSemanticsInfoItem
    StarlarkSemanticsInfoItem {
        /// `commandOptions.getOptions(BuildLanguageOptions.class)`, read at construction (see
        /// [`PackagePathInfoItem`]).
        command_options: Option<Option<BuildLanguageOptions>>,
    }
}

impl StarlarkSemanticsInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/StarlarkSemanticsInfoItem.java:34 StarlarkSemanticsInfoItem.<init>
    pub fn new(command_options: Option<&OptionsParsingResult>) -> Self {
        StarlarkSemanticsInfoItem {
            name: "starlark-semantics".to_string(),
            description: "The effective set of Starlark semantics option values.".to_string(),
            hidden: true,
            command_options: command_options.map(|o| o.get_options::<BuildLanguageOptions>()),
        }
    }
}

impl InfoItem for StarlarkSemanticsInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/StarlarkSemanticsInfoItem.java:43 StarlarkSemanticsInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        let build_language_options = self
            .command_options
            .as_ref()
            .expect("NullPointerException: commandOptions")
            .as_ref()
            .expect("NullPointerException: BuildLanguageOptions");
        let effective_starlark_semantics = get_effective_starlark_semantics(build_language_options);
        Ok(print(effective_starlark_semantics))
    }
}

/// `SkyframeExecutor.getEffectiveStarlarkSemantics`: `buildLanguageOptions.toStarlarkSemantics()`
/// (Bazel's executor does not override it).
///
/// PORT: Java reads it from the workspace's executor; the method uses nothing of the executor, so
/// it is a function here.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1754 SkyframeExecutor.getEffectiveStarlarkSemantics
pub fn get_effective_starlark_semantics(
    build_language_options: &BuildLanguageOptions,
) -> StarlarkSemanticsMap {
    to_starlark_semantics(build_language_options)
}

/// `StarlarkSemantics` as Java stores it: the entries whose value differs from the key's default,
/// keyed by name (boolean keys keep their `+`/`-` prefix), sorted by key (Java's builder is a
/// `TreeMap<String, Object>`; the key names are ASCII, so byte order is Java's `String` order).
///
/// PORT: `blaze_starlark::StarlarkSemantics` (the typed form the interpreter reads) has no
/// `toString`; this is the key/value form `toString` prints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StarlarkSemanticsMap {
    map: std::collections::BTreeMap<String, SemanticsValue>,
}

/// The non-boolean `StarlarkSemantics.Key`s of `BuildLanguageOptions` (name and default).
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:1168 BuildLanguageOptions (non-booleans)
const NON_BOOLEAN_KEYS: [StarlarkSemanticsKey; 9] = [
    build_language_options::INCOMPATIBLE_DISABLE_TRANSITIONS_OPTIONS,
    build_language_options::EXPERIMENTAL_BUILTINS_BZL_PATH,
    build_language_options::INCOMPATIBLE_AUTOLOAD_EXTERNALLY,
    build_language_options::REPOSITORIES_WITHOUT_AUTOLOAD,
    build_language_options::EXPERIMENTAL_BUILTINS_INJECTION_OVERRIDE,
    build_language_options::INCOMPATIBLE_ENFORCE_STARLARK_UTF8,
    build_language_options::EXPERIMENTAL_STARLARK_TYPES_ALLOWED_PATHS,
    build_language_options::MAX_COMPUTATION_STEPS,
    build_language_options::NESTED_SET_DEPTH_LIMIT,
];

/// `BuildLanguageOptions.toStarlarkSemantics`: `setFlags` into a `StarlarkSemantics.Builder`.
// Java: com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions.java:997 BuildLanguageOptions.toStarlarkSemantics
pub fn to_starlark_semantics(options: &BuildLanguageOptions) -> StarlarkSemanticsMap {
    // This function connects command-line flags to their corresponding StarlarkSemantics keys.
    let mut builder = StarlarkSemanticsMap::default();
    for (key, value) in options.set_flags() {
        match value {
            SemanticsValue::Bool(enabled) => builder.set_bool(key, enabled),
            value => {
                let key = NON_BOOLEAN_KEYS
                    .iter()
                    .find(|k| k.name == key)
                    .unwrap_or_else(|| panic!("no StarlarkSemantics.Key named {}", key));
                builder.set(key, value);
            }
        }
    }
    builder
}

impl StarlarkSemanticsMap {
    /// Sets the value for the specified key.
    // Java: net/starlark/java/eval/StarlarkSemantics.java:152 StarlarkSemantics.Builder.set
    fn set(&mut self, key: &StarlarkSemanticsKey, value: SemanticsValue) {
        if value != (key.default)() {
            self.map.insert(key.name.to_string(), value);
        } else {
            self.map.remove(key.name);
        }
    }

    /// Sets the value for the boolean key, which must have a [+-] prefix.
    // Java: net/starlark/java/eval/StarlarkSemantics.java:163 StarlarkSemantics.Builder.setBool
    fn set_bool(&mut self, name: &str, value: bool) {
        let prefix = name.chars().next().expect("empty key");
        blaze_util::check_argument!(prefix == '+' || prefix == '-');
        let default_value = prefix == '+';
        if value != default_value {
            self.map
                .insert(name.to_string(), SemanticsValue::Bool(value));
        } else {
            self.map.remove(name);
        }
    }
}

/// `String.valueOf` of a semantics value.
fn semantics_value_to_string(value: &SemanticsValue) -> String {
    match value {
        SemanticsValue::Bool(b) => b.to_string(),
        SemanticsValue::String(s) => s.clone(),
        // AbstractCollection.toString
        SemanticsValue::StringList(l) => format!("[{}]", l.join(", ")),
        SemanticsValue::Long(n) => n.to_string(),
        SemanticsValue::Int(n) => n.to_string(),
        // Enum.toString: the constant's name.
        SemanticsValue::Utf8EnforcementMode(mode) => match mode {
            Utf8EnforcementMode::OFF => "OFF",
            Utf8EnforcementMode::WARNING => "WARNING",
            Utf8EnforcementMode::ERROR => "ERROR",
        }
        .to_string(),
    }
}

/// Returns a representation of this StarlarkSemantics' non-default key/value pairs in key order.
// Java: net/starlark/java/eval/StarlarkSemantics.java:233 StarlarkSemantics.toString
impl std::fmt::Display for StarlarkSemanticsMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print "StarlarkSemantics{k=v, ...}", without +/- prefixes.
        let mut buf = String::new();
        buf.push_str("StarlarkSemantics{");
        let mut sep = "";
        for (key, value) in &self.map {
            buf.push_str(sep);
            sep = ", ";
            if key.starts_with('+') || key.starts_with('-') {
                buf.push_str(&key[1..]);
            } else {
                buf.push_str(key);
            }
            buf.push('=');
            buf.push_str(&semantics_value_to_string(value));
        }
        buf.push('}');
        f.write_str(&buf)
    }
}

info_item_struct! {
    /// Info item for persistent worker metrics.
    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkerMetricsInfoItem.java:31 WorkerMetricsInfoItem
    WorkerMetricsInfoItem {}
}

impl WorkerMetricsInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkerMetricsInfoItem.java:32 WorkerMetricsInfoItem.<init>
    pub fn new() -> Self {
        WorkerMetricsInfoItem {
            name: "worker_metrics".to_string(),
            description: "persistent worker metrics".to_string(),
            hidden: true,
        }
    }
}

impl InfoItem for WorkerMetricsInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/WorkerMetricsInfoItem.java:37 WorkerMetricsInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        // PORT: WorkerProcessMetricsCollector.getLiveWorkerMetrics() lists the worker processes
        // this server started; Blaze has no worker strategy, so it never starts one and the list
        // is empty. The non-empty formatting is not ported.
        Ok(print("No persistent workers active."))
    }
}

info_item_struct! {
    /// Info item for auto detected local resources.
    // Java: com/google/devtools/build/lib/runtime/commands/info/LocalResourcesInfoItem.java:25 LocalResourcesInfoItem
    LocalResourcesInfoItem {}
}

impl LocalResourcesInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/LocalResourcesInfoItem.java:26 LocalResourcesInfoItem.<init>
    pub fn new() -> Self {
        LocalResourcesInfoItem {
            name: "local_resources".to_string(),
            description: "Auto detected local resources".to_string(),
            hidden: false,
        }
    }

    /// `String.format("RAM=%dMB, CPU=%.1f", (int) memoryMb, cpuUsage)`.
    /// PORT: Java's `%.1f` rounds HALF_UP, Rust's `{:.1}` rounds half to even; CPU counts are
    /// whole numbers, so the two agree.
    pub fn format(memory_mb: f64, cpu_usage: f64) -> String {
        format!("RAM={}MB, CPU={:.1}", memory_mb as i32, cpu_usage)
    }
}

impl InfoItem for LocalResourcesInfoItem {
    info_item_base!();

    // Java: com/google/devtools/build/lib/runtime/commands/info/LocalResourcesInfoItem.java:31 LocalResourcesInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        let local_resources = LocalHostCapacity::get_local_host_capacity();
        Ok(print(Self::format(
            local_resources.get_memory_mb(),
            local_resources.get_cpu_usage(),
        )))
    }
}

/// Info item for the make environment.
// Java: com/google/devtools/build/lib/runtime/commands/info/MakeInfoItem.java:23 MakeInfoItem
pub struct MakeInfoItem {
    name: String,
    /// The value of the variable (Java stores it in `description`).
    description: String,
    hidden: bool,
    /// What `getDescription()` returns.
    make_description: String,
}

impl MakeInfoItem {
    // Java: com/google/devtools/build/lib/runtime/commands/info/MakeInfoItem.java:24 MakeInfoItem.<init>
    pub fn new(name: &str, description: &str) -> Self {
        MakeInfoItem {
            name: name.to_string(),
            description: description.to_string(),
            hidden: false,
            make_description: format!("Make environment variable '{}'", name),
        }
    }
}

impl InfoItem for MakeInfoItem {
    fn get_name(&self) -> &str {
        &self.name
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/MakeInfoItem.java:29 MakeInfoItem.getDescription
    fn get_description(&self) -> &str {
        &self.make_description
    }

    fn is_hidden(&self) -> bool {
        self.hidden
    }

    // Java: com/google/devtools/build/lib/runtime/commands/info/MakeInfoItem.java:34 MakeInfoItem.get
    fn get(
        &self,
        _configuration_supplier: &mut dyn ConfigurationSupplier,
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        Ok(print(&self.description))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_resources_format_matches_oracle() {
        // Oracle under `systemd-run -p MemoryMax=12G` on a 16-CPU host.
        assert_eq!(
            LocalResourcesInfoItem::format(12288.0, 16.0),
            "RAM=12288MB, CPU=16.0"
        );
        assert_eq!(
            LocalResourcesInfoItem::format(12288.9, 16.0),
            "RAM=12288MB, CPU=16.0"
        );
    }

    #[test]
    fn make_info_item_description() {
        let item = MakeInfoItem::new("CC", "/usr/bin/gcc");
        assert_eq!(item.get_name(), "CC");
        assert_eq!(item.get_description(), "Make environment variable 'CC'");
        assert!(!item.is_hidden());
    }

    // Ported from src/test/java/com/google/devtools/build/lib/util/StringUtilitiesTest.java.
    #[test]
    fn test_pretty_print_bytes() {
        let expected = [
            "2B", "23B", "234B", "2345B", "23KB", "234KB", "2345KB", "23MB", "234MB", "2345MB",
            "23456MB", "234GB", "2345GB", "23456GB",
        ];
        let mut x: f64 = 2.3456;
        for e in expected {
            assert_eq!(pretty_print_bytes(x as i64), e);
            x *= 10.0;
        }
    }

    /// The bounds of each unit.
    #[test]
    fn pretty_print_bytes_units() {
        assert_eq!(pretty_print_bytes(0), "0B");
        assert_eq!(pretty_print_bytes(9999), "9999B");
        assert_eq!(pretty_print_bytes(10000), "10KB");
        assert_eq!(pretty_print_bytes(9_999_999), "9999KB");
        assert_eq!(pretty_print_bytes(10_000_000), "10MB");
        // Oracle: `max-heap-size: 8187MB`.
        assert_eq!(pretty_print_bytes(8_187_281_408), "8187MB");
        assert_eq!(pretty_print_bytes(99_999_999_999), "99999MB");
        assert_eq!(pretty_print_bytes(100_000_000_000), "100GB");
    }

    #[test]
    fn jdk_release_of_the_embedded_jdk() {
        // The `release` file of the JDK embedded in Bazel 9.3.2's install base.
        let release =
            JdkRelease::parse(b"JAVA_VERSION=\"25.0.2\"\nMODULES=\"java.base java.compiler\"\n");
        assert_eq!(
            release.java_runtime(),
            "Unknown runtime (build 25.0.2) by unknown"
        );
        assert_eq!(
            release.java_vm(),
            "Unknown VM (build 25.0.2, unknown) by unknown"
        );
        let full = JdkRelease::parse(
            b"IMPLEMENTOR=\"Azul Systems, Inc.\"\nJAVA_RUNTIME_VERSION=\"25.0.2+10-LTS\"\nJAVA_VERSION=\"25.0.2\"\n",
        );
        assert_eq!(
            full.java_runtime(),
            "Unknown runtime (build 25.0.2+10-LTS) by Azul Systems, Inc."
        );
    }

    #[test]
    fn starlark_semantics_default_options() {
        // `internal_starlark_utf_8_byte_strings` defaults to true while its key
        // (`-internal_bazel_only_utf_8_byte_strings`) defaults to false.
        let semantics = to_starlark_semantics(&BuildLanguageOptions::default());
        assert!(semantics.to_string().starts_with("StarlarkSemantics{"));
        assert!(
            semantics
                .to_string()
                .contains("internal_bazel_only_utf_8_byte_strings=true")
        );
    }

    #[test]
    fn starlark_semantics_to_string_in_key_order() {
        let mut semantics = StarlarkSemanticsMap::default();
        assert_eq!(semantics.to_string(), "StarlarkSemantics{}");
        semantics.set_bool("-print_test_marker", true);
        semantics.set(
            &build_language_options::NESTED_SET_DEPTH_LIMIT,
            SemanticsValue::Int(10),
        );
        semantics.set_bool("+check_bzl_visibility", false);
        semantics.set(
            &build_language_options::INCOMPATIBLE_AUTOLOAD_EXTERNALLY,
            SemanticsValue::StringList(vec!["a".into(), "b".into()]),
        );
        // A value equal to the default removes the entry.
        semantics.set(
            &build_language_options::MAX_COMPUTATION_STEPS,
            SemanticsValue::Long(0),
        );
        semantics.set_bool("+experimental_bzl_visibility", true);
        // TreeMap order of the prefixed names: '+' < '-' < letters.
        assert_eq!(
            semantics.to_string(),
            "StarlarkSemantics{check_bzl_visibility=false, print_test_marker=true, \
             incompatible_autoload_externally=[a, b], nested_set_depth_limit=10}"
        );
    }

    #[test]
    fn character_encoding_values() {
        assert_eq!(
            CharacterEncodingInfoItem::default_charset_name(CLIENT_FILE_ENCODING),
            "ISO-8859-1"
        );
        // Depends on the installed locales; on the oracle's host it is ISO-8859-1.
        assert!(!CharacterEncodingInfoItem::sun_jnu_encoding().is_empty());
    }
}
