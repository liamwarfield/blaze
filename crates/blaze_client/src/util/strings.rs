//! The client's strings at the operating system boundary.
//!
//! The C++ client keeps every string (argv, environment, file names, file contents, messages) as
//! a `std::string` of raw bytes. Blaze keeps them as internal strings (docs/research/
//! 07-byte-strings-rules.md): one char per byte. Bytes enter through [`from_platform_bytes`]
//! and leave through [`cstr`] (system calls) or [`console_bytes`] (streams).

use std::borrow::Cow;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;

use blaze_util::string_encoding::{
    internal_to_platform, latin1_encode_lossy, platform_to_internal,
};

/// The NUL-terminated platform string of an internal string (the `c_str()` of the C++
/// `std::string`); empty if it holds a NUL, as before.
pub(crate) fn cstr(s: &str) -> CString {
    CString::new(internal_to_platform(s).as_bytes()).unwrap_or_default()
}

/// The internal string of bytes a system call returned (a `std::string` built from them).
pub(crate) fn from_platform_bytes(bytes: &[u8]) -> String {
    platform_to_internal(OsStr::from_bytes(bytes)).into_owned()
}

/// The bytes the C++ writes for an internal string to a stream (`std::cerr << s`): its raw bytes.
pub(crate) fn console_bytes(s: &str) -> Cow<'_, [u8]> {
    latin1_encode_lossy(s)
}
