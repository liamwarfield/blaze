//! Port of `src/main/cpp/util/errors_posix.cc`.

use std::ffi::CStr;

/// The current `errno`.
pub fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Sets `errno` (C++ code assigns `errno` directly in a few places).
pub fn set_errno(value: i32) {
    // SAFETY: __errno_location returns a valid pointer to this thread's errno.
    unsafe {
        *libc::__errno_location() = value;
    }
}

/// `strerror(errnum)`.
pub fn strerror(errnum: i32) -> String {
    // SAFETY: strerror returns a pointer to a NUL-terminated static or thread-local string.
    unsafe {
        let p = libc::strerror(errnum);
        if p.is_null() {
            return String::new();
        }
        // The C++ uses strerror's raw bytes (locale-encoded).
        super::strings::from_platform_bytes(CStr::from_ptr(p).to_bytes())
    }
}

// Java: src/main/cpp/util/errors_posix.cc:27 GetLastErrorString
pub fn get_last_error_string() -> String {
    let saved_errno = errno();
    format!("(error: {}): {}", saved_errno, strerror(saved_errno))
}
