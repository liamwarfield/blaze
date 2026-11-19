//! Port of `com.google.devtools.build.lib.shell.LogUtil`.

/// Utilities for logging.
// Java: com/google/devtools/build/lib/shell/LogUtil.java:21 LogUtil
pub struct LogUtil;

const TRUNCATE_STRINGS_AT: usize = 150;

impl LogUtil {
    /// Make a string out of a byte array, and truncate it to a reasonable length. Useful for
    /// preventing logs from becoming excessively large.
    // Java: com/google/devtools/build/lib/shell/LogUtil.java:31 LogUtil.toTruncatedString
    pub fn to_truncated_string(bytes: Option<&[u8]>) -> String {
        let bytes = match bytes {
            None => return String::new(),
            Some([]) => return String::new(),
            Some(b) => b,
        };
        // Yes, we'll use the platform encoding here, and this is one of the rare cases where it
        // makes sense. (The platform encoding of Bazel's server is ISO-8859-1:
        // `-Dfile.encoding=ISO-8859-1`.)
        if bytes.len() > TRUNCATE_STRINGS_AT {
            return format!(
                "{}[... truncated. original size was {} bytes.]",
                blaze_util::string_encoding::latin1_decode(&bytes[..TRUNCATE_STRINGS_AT]),
                bytes.len()
            );
        }
        blaze_util::string_encoding::latin1_decode(bytes).into_owned()
    }
}
