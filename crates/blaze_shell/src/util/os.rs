//! Port of `com.google.devtools.build.lib.util.OS`.
// PORT: move to blaze_util.

use std::fmt;
use std::sync::OnceLock;

/// Detects the running operating system and returns a describing enum value.
// Java: com/google/devtools/build/lib/util/OS.java:21 OS
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OS {
    Darwin,
    Freebsd,
    Openbsd,
    Linux,
    Windows,
    Unknown,
}

impl OS {
    /// `OS.values()`, in declaration order.
    pub const VALUES: [OS; 6] = [
        OS::Darwin,
        OS::Freebsd,
        OS::Openbsd,
        OS::Linux,
        OS::Windows,
        OS::Unknown,
    ];

    // Java: com/google/devtools/build/lib/util/OS.java:39 OS.getCanonicalName
    pub fn get_canonical_name(self) -> &'static str {
        match self {
            OS::Darwin => "osx",
            OS::Freebsd => "freebsd",
            OS::Openbsd => "openbsd",
            OS::Linux => "linux",
            OS::Windows => "windows",
            OS::Unknown => "unknown",
        }
    }

    fn detection_name(self) -> &'static str {
        match self {
            OS::Darwin => "Mac OS X",
            OS::Freebsd => "FreeBSD",
            OS::Openbsd => "OpenBSD",
            OS::Linux => "Linux",
            OS::Windows => "Windows",
            OS::Unknown => "",
        }
    }

    /// The current operating system.
    // Java: com/google/devtools/build/lib/util/OS.java:53 OS.getCurrent
    pub fn get_current() -> OS {
        static HOST_SYSTEM: OnceLock<OS> = OnceLock::new();
        *HOST_SYSTEM.get_or_init(OS::determine_current_os)
    }

    // Java: com/google/devtools/build/lib/util/OS.java:57 OS.isPosixCompatible
    pub fn is_posix_compatible() -> bool {
        matches!(
            OS::get_current(),
            OS::Darwin | OS::Freebsd | OS::Openbsd | OS::Linux
        )
    }

    /// `System.getProperty("os.version")`: the kernel release (`uname -r`) on Linux.
    // Java: com/google/devtools/build/lib/util/OS.java:61 OS.getVersion
    pub fn get_version() -> String {
        // SAFETY: uname fills a zeroed struct.
        let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
        // SAFETY: valid pointer.
        if unsafe { libc::uname(&mut uts) } != 0 {
            return String::new();
        }
        // SAFETY: release is NUL-terminated.
        unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    /// The JVM's `os.name` for the platform this binary was built for.
    // PORT: Java also honours the `blaze.os` system property (a test hook to fake another OS);
    // Blaze has no system properties, so only `os.name` is used.
    fn os_name() -> Option<&'static str> {
        match std::env::consts::OS {
            "linux" => Some("Linux"),
            "macos" => Some("Mac OS X"),
            "freebsd" => Some("FreeBSD"),
            "openbsd" => Some("OpenBSD"),
            "windows" => Some("Windows"),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/util/OS.java:67 OS.determineCurrentOs
    fn determine_current_os() -> OS {
        let Some(os_name) = OS::os_name() else {
            return OS::Unknown;
        };
        for os in OS::VALUES {
            // Windows have many names, all starting with "Windows".
            if os_name.starts_with(os.detection_name()) {
                return os;
            }
        }
        OS::Unknown
    }
}

impl fmt::Display for OS {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_canonical_name())
    }
}
