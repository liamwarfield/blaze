//! The hardwired info items of `blaze info` (one struct per Java class in
//! `com.google.devtools.build.lib.runtime.commands.info`).

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

/// Defines a JVM-only info item: Blaze has no JVM, so `get` fails loudly; name, description and
/// hidden flag are Java's (they appear in `help info-keys`).
macro_rules! jvm_info_item {
    ($(#[$m:meta])* $ty:ident, $name:expr, $description:expr, $hidden:expr, $java_get:expr) => {
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

            fn get(
                &self,
                _configuration_supplier: &mut dyn ConfigurationSupplier,
                _env: &mut CommandEnvironment,
            ) -> Result<Vec<u8>, AbruptExitException> {
                blaze_util::unported!($java_get, "no JVM")
            }
        }
    };
}

jvm_info_item!(
    /// Info item for the used heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeInfoItem.java:24 UsedHeapSizeInfoItem
    UsedHeapSizeInfoItem,
    "used-heap-size",
    "The amount of used memory in bytes. Note that this is not a good indicator of the actual \
     memory use, as it includes any remaining inaccessible memory.",
    false,
    "UsedHeapSizeInfoItem.get"
);

jvm_info_item!(
    /// Info item for the used heap size after garbage collection.
    // Java: com/google/devtools/build/lib/runtime/commands/info/UsedHeapSizeAfterGcInfoItem.java:27 UsedHeapSizeAfterGcInfoItem
    UsedHeapSizeAfterGcInfoItem,
    "used-heap-size-after-gc",
    "The amount of used memory in bytes after a call to System.gc().",
    true,
    "UsedHeapSizeAfterGcInfoItem.get"
);

jvm_info_item!(
    /// Info item for the committed heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/CommittedHeapSizeInfoItem.java:24 CommittedHeapSizeInfoItem
    CommittedHeapSizeInfoItem,
    "committed-heap-size",
    "The amount of memory in bytes that is committed for the Java virtual machine to use",
    false,
    "CommittedHeapSizeInfoItem.get"
);

jvm_info_item!(
    /// Info item for the max heap size.
    // Java: com/google/devtools/build/lib/runtime/commands/info/MaxHeapSizeInfoItem.java:24 MaxHeapSizeInfoItem
    MaxHeapSizeInfoItem,
    "max-heap-size",
    "The maximum amount of memory in bytes that can be used for memory management.",
    false,
    "MaxHeapSizeInfoItem.get"
);

jvm_info_item!(
    /// Info item for the gc-time.
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcTimeInfoItem.java:25 GcTimeInfoItem
    GcTimeInfoItem,
    "gc-time",
    "The approximate accumulated time spend on garbage collection.",
    false,
    "GcTimeInfoItem.get"
);

jvm_info_item!(
    /// Info item for the gc-count.
    // Java: com/google/devtools/build/lib/runtime/commands/info/GcCountInfoItem.java:25 GcCountInfoItem
    GcCountInfoItem,
    "gc-count",
    "Number of garbage collection runs.",
    false,
    "GcCountInfoItem.get"
);

jvm_info_item!(
    /// Info item for the name and version of the Java runtime environment.
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaRuntimeInfoItem.java:23 JavaRuntimeInfoItem
    JavaRuntimeInfoItem,
    "java-runtime",
    "Name and version of the current Java runtime environment.",
    false,
    "JavaRuntimeInfoItem.get"
);

jvm_info_item!(
    /// Info item for the name and version of the Java VM.
    // Java: com/google/devtools/build/lib/runtime/commands/info/JavaVirtualMachineInfoItem.java:23 JavaVirtualMachineInfoItem
    JavaVirtualMachineInfoItem,
    "java-vm",
    "Name and version of the current Java virtual machine.",
    false,
    "JavaVirtualMachineInfoItem.get"
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
    fn get_server_javabase(env: &CommandEnvironment) -> String {
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
        _env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException> {
        blaze_util::unported!(
            "BuildLanguageInfoItem.get",
            "needs StarlarkBuiltinsValue and the rule class provider"
        )
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
    StarlarkSemanticsInfoItem {}
}

impl StarlarkSemanticsInfoItem {
    /// PORT: Java keeps `commandOptions` to read `BuildLanguageOptions` in `get`; `get` is not
    /// ported, so the options are not kept.
    // Java: com/google/devtools/build/lib/runtime/commands/info/StarlarkSemanticsInfoItem.java:34 StarlarkSemanticsInfoItem.<init>
    pub fn new(_command_options: Option<&OptionsParsingResult>) -> Self {
        StarlarkSemanticsInfoItem {
            name: "starlark-semantics".to_string(),
            description: "The effective set of Starlark semantics option values.".to_string(),
            hidden: true,
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
        blaze_util::unported!("SkyframeExecutor.getEffectiveStarlarkSemantics")
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
