//! Port of `lib/sandbox/LinuxSandboxUtil.java`: utility functions for the `linux-sandbox`
//! embedded tool.

use std::io;

use blaze_protos::failure_details::sandbox::Code;
use blaze_vfs::Path;

use super::fs_utils;
use super::sandbox_helpers::{self, INACCESSIBLE_HELPER_DIR, INACCESSIBLE_HELPER_FILE};
use crate::actions::UserExecException;
use crate::exec::bin_tools::BinTools;

// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:29 LinuxSandboxUtil.LINUX_SANDBOX
// PORT: OsUtils.executableExtension() is "" on Linux.
const LINUX_SANDBOX: &str = "linux-sandbox";

/// Returns whether using the `linux-sandbox` is supported in the command environment.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:32 LinuxSandboxUtil.isSupported
pub fn is_supported(bin_tools: &BinTools) -> bool {
    // We can only use the linux-sandbox if the linux-sandbox exists in the embedded tools.
    // This might not always be the case, e.g. while bootstrapping.
    get_linux_sandbox(bin_tools).is_some()
}

/// Returns the path of the `linux-sandbox` binary, or `None` if it doesn't exist.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:39 LinuxSandboxUtil.getLinuxSandbox
pub fn get_linux_sandbox(bin_tools: &BinTools) -> Option<Path> {
    bin_tools.get_embedded_path(LINUX_SANDBOX)
}

/// This method does the following things:
///
/// - If mount source does not exist on the host system, throw an error message
/// - If mount target exists, check whether the source and target are of the same type
/// - If mount target does not exist on the host system, throw an error message
///
/// `bind_mounts`: the bind mounts map with target as key and source as value.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:55 LinuxSandboxUtil.validateBindMounts
pub fn validate_bind_mounts<'a>(
    bind_mounts: impl IntoIterator<Item = (&'a Path, &'a Path)>,
) -> Result<(), UserExecException> {
    for (target, source) in bind_mounts {
        // Mount source should exist in the file system
        if !source.exists() {
            return Err(UserExecException::new(
                sandbox_helpers::create_failure_detail(
                    &format!("Mount source '{}' does not exist.", source),
                    Code::MountSourceDoesNotExist,
                ),
            ));
        }
        // If target exists, but is not of the same type as the source, then we cannot mount it.
        if target.exists() {
            let are_both_directories = source.is_directory() && target.is_directory();
            let is_source_file = source.is_file() || source.is_symbolic_link();
            let is_target_file = target.is_file() || target.is_symbolic_link();
            let are_both_files = is_source_file && is_target_file;
            if !(are_both_directories || are_both_files) {
                // Source and target are not of the same type; we cannot mount it.
                return Err(UserExecException::new(
                    sandbox_helpers::create_failure_detail(
                        &format!(
                            "Mount target '{}' is a {} but mount source '{}' is a {}, they must be the same type.",
                            target,
                            if is_target_file { "file" } else { "directory" },
                            source,
                            if is_source_file { "file" } else { "directory" }
                        ),
                        Code::MountSourceTargetTypeMismatch,
                    ),
                ));
            }
        } else {
            // Mount target should exist in the file system
            return Err(UserExecException::new(
                sandbox_helpers::create_failure_detail(
                    &format!(
                        "Mount target '{}' does not exist. Bazel only supports bind mounting on top of existing files/directories. Please create an empty file or directory at the mount target path according to the type of mount source.",
                        target
                    ),
                    Code::MountTargetDoesNotExist,
                ),
            ));
        }
    }
    Ok(())
}

/// Returns a newly created inaccessible file underneath the given directory.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:100 LinuxSandboxUtil.getInaccessibleHelperFile
pub fn get_inaccessible_helper_file(sandbox_base: &Path) -> io::Result<Path> {
    // The order of the permissions settings calls matters, see
    // https://github.com/bazelbuild/bazel/issues/16364
    let inaccessible_helper_file = sandbox_base.get_relative_str(INACCESSIBLE_HELPER_FILE);
    fs_utils::touch_file(&inaccessible_helper_file)?;
    inaccessible_helper_file.set_executable(false)?;
    inaccessible_helper_file.set_writable(false)?;
    inaccessible_helper_file.set_readable(false)?;
    Ok(inaccessible_helper_file)
}

/// Returns a newly created inaccessible directory underneath the given directory.
// Java: com/google/devtools/build/lib/sandbox/LinuxSandboxUtil.java:112 LinuxSandboxUtil.getInaccessibleHelperDir
pub fn get_inaccessible_helper_dir(sandbox_base: &Path) -> io::Result<Path> {
    // The order of the permissions settings calls matters, see
    // https://github.com/bazelbuild/bazel/issues/16364
    let inaccessible_helper_dir = sandbox_base.get_relative_str(INACCESSIBLE_HELPER_DIR);
    inaccessible_helper_dir.create_directory()?;
    inaccessible_helper_dir.set_executable(false)?;
    inaccessible_helper_dir.set_writable(false)?;
    inaccessible_helper_dir.set_readable(false)?;
    Ok(inaccessible_helper_dir)
}
