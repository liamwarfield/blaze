//! OS process related utilities.
//!
//! Use `std::process::id()` to get the PID of the current process.

/// The real group ID of the current process.
///
/// PORT: Java throws `UnsupportedOperationException` on Windows; Blaze runs on Linux only.
// Java: com/google/devtools/build/lib/util/ProcessUtils.java:36 ProcessUtils.getgid
pub fn getgid() -> i32 {
    // SAFETY: getgid has no preconditions and cannot fail.
    unsafe { libc::getgid() as i32 }
}

/// The real user ID of the current process.
// Java: com/google/devtools/build/lib/util/ProcessUtils.java:49 ProcessUtils.getuid
pub fn getuid() -> i32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_of_this_process() {
        // SAFETY: as above.
        assert_eq!(getuid(), unsafe { libc::getuid() } as i32);
        assert_eq!(getgid(), unsafe { libc::getgid() } as i32);
    }
}
