//! Port of `com.google.devtools.build.lib.util.OsUtils`.
// PORT: move to blaze_util.

use crate::util::os::OS;

/// Operating system-specific utilities.
// Java: com/google/devtools/build/lib/util/OsUtils.java:20 OsUtils
pub struct OsUtils;

impl OsUtils {
    // Java: com/google/devtools/build/lib/util/OsUtils.java:28 OsUtils.executableExtension
    pub fn executable_extension_for(os: OS) -> &'static str {
        if os == OS::Windows { ".exe" } else { "" }
    }

    /// Returns the extension used for executables on the current platform (.exe for Windows,
    /// empty string for others).
    // Java: com/google/devtools/build/lib/util/OsUtils.java:36 OsUtils.executableExtension
    pub fn executable_extension() -> &'static str {
        OsUtils::executable_extension_for(OS::get_current())
    }
}
