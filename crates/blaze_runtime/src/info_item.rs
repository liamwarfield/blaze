//! `InfoItem`: an item that is returned by `blaze info`.

use crate::command_environment::CommandEnvironment;
use crate::module::AbruptExitException;

/// The configuration a configuration-dependent info item reads.
///
/// PORT: Java passes a memoizing `Supplier<BuildConfigurationValue>`; computing the
/// configuration needs options -> `BuildOptions` -> Skyframe (wave W4), so the supplier is a
/// trait whose Blaze implementation is `unported!` until then.
pub trait ConfigurationSupplier {
    /// `BuildConfigurationValue.getMakeEnvironment()`, in Java's iteration order.
    fn get_make_environment(&mut self) -> Result<Vec<(String, String)>, AbruptExitException>;
    /// `BuildConfigurationValue.getBinDirectory(RepositoryName.MAIN).getRoot().asPath()`.
    fn get_bin_directory(&mut self) -> Result<String, AbruptExitException>;
    /// `BuildConfigurationValue.getGenfilesDirectory(RepositoryName.MAIN)` root path.
    fn get_genfiles_directory(&mut self) -> Result<String, AbruptExitException>;
    /// `BuildConfigurationValue.getTestLogsDirectory(RepositoryName.MAIN)` root path.
    fn get_test_logs_directory(&mut self) -> Result<String, AbruptExitException>;
    /// Forces creation (Java: `configurationSupplier.get()`).
    fn get(&mut self) -> Result<(), AbruptExitException>;
}

/// An item that is returned by `blaze info`.
// Java: com/google/devtools/build/lib/runtime/InfoItem.java:25 InfoItem
pub trait InfoItem: Send + Sync {
    /// The name of the info key.
    // Java: com/google/devtools/build/lib/runtime/InfoItem.java:41 InfoItem.getName
    fn get_name(&self) -> &str;

    /// The help description of the info key.
    // Java: com/google/devtools/build/lib/runtime/InfoItem.java:46 InfoItem.getDescription
    fn get_description(&self) -> &str;

    /// Returns true if this info item requires `CommandEnvironment.syncPackageLoading` to be
    /// called. Virtually all info items do not need it.
    // Java: com/google/devtools/build/lib/runtime/InfoItem.java:56 InfoItem.needsSyncPackageLoading
    fn needs_sync_package_loading(&self) -> bool {
        false
    }

    /// Whether the key is printed when "blaze info" is invoked without arguments.
    // Java: com/google/devtools/build/lib/runtime/InfoItem.java:66 InfoItem.isHidden
    fn is_hidden(&self) -> bool {
        false
    }

    /// Returns the value of the info key. The return value is directly printed to stdout.
    // Java: com/google/devtools/build/lib/runtime/InfoItem.java:71 InfoItem.get
    fn get(
        &self,
        configuration_supplier: &mut dyn ConfigurationSupplier,
        env: &mut CommandEnvironment,
    ) -> Result<Vec<u8>, AbruptExitException>;
}

/// `InfoItem.print`: bytes are returned as they are; anything else is its string form plus a
/// newline.
// Java: com/google/devtools/build/lib/runtime/InfoItem.java:75 InfoItem.print
pub fn print(value: impl std::fmt::Display) -> Vec<u8> {
    // PORT: StringEncoding identity (StringUnsafe.getInternalStringBytes).
    let mut bytes = value.to_string().into_bytes();
    bytes.push(b'\n');
    bytes
}

/// `InfoItem.print` for a `byte[]` value.
pub fn print_bytes(value: Vec<u8>) -> Vec<u8> {
    value
}
