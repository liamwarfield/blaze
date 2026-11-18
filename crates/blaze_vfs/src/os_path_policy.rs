//! Port of `OsPathPolicy` and `UnixOsPathPolicy`.
//!
//! Only the Unix policy is ported. `WindowsOsPathPolicy` (drive letters, backslash separators,
//! case-insensitivity) is Windows-only and deliberately not ported: Blaze targets Linux x86-64.

/// Path is normalized.
// Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:24 OsPathPolicy.NORMALIZED
pub const NORMALIZED: i32 = 0;
/// Path requires normalization.
// Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:25 OsPathPolicy.NEEDS_NORMALIZE
pub const NEEDS_NORMALIZE: i32 = 1;

/// The differences in path style between OSs (Java `OsPathPolicy`).
///
/// Strings are Bazel "internal" strings: in Rust, UTF-8 `str` examined byte by byte, which is how
/// Bazel's Latin-1-coded internal strings behave.
// Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:23 OsPathPolicy
pub trait OsPathPolicy: Send + Sync {
    /// Returns required normalization level, passed to [`OsPathPolicy::normalize`].
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:28 OsPathPolicy.needsToNormalize
    fn needs_to_normalize(&self, path: &str) -> i32;

    /// Returns the required normalization level if an already normalized string is concatenated
    /// with another normalized path fragment.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:36 OsPathPolicy.needsToNormalizeSuffix
    fn needs_to_normalize_suffix(&self, normalized_suffix: &str) -> i32;

    /// Normalizes the passed string according to the passed normalization level.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:43 OsPathPolicy.normalize
    fn normalize(&self, path: &str, normalization_level: i32) -> String;

    /// Returns the length of the mount, eg. 1 for unix '/'. 0 for relative paths.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:50 OsPathPolicy.getDriveStrLength
    fn get_drive_str_length(&self, path: &str) -> usize;

    /// Returns whether the unnormalized character c is a separator.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:53 OsPathPolicy.isSeparator
    fn is_separator(&self, c: u8) -> bool;

    /// Returns an additional character besides '/' for which `is_separator` is true. 0 means there
    /// is no such additional character.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:59 OsPathPolicy.additionalSeparator
    fn additional_separator(&self) -> u8;

    /// Modifies the given string to be suitable for execution on the OS represented by this
    /// policy.
    // Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:64 OsPathPolicy.postProcessPathStringForExecution
    fn post_process_path_string_for_execution(&self, callable_path_string: &str) -> String;
}

/// The policy for the OS of the machine running the server (Java `OsPathPolicy.HOST_POLICY`).
/// Always Unix in Blaze.
// Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:71 OsPathPolicy.HOST_POLICY
pub static HOST_POLICY: UnixOsPathPolicy = UnixOsPathPolicy;

/// Normalizes any '.' and '..' in-place in the segment array by shifting other segments to the
/// front. Returns the remaining number of items.
// Java: com/google/devtools/build/lib/vfs/OsPathPolicy.java:94 OsPathPolicy.Utils.removeRelativePaths
pub(crate) fn remove_relative_paths(
    segments: &mut [&str],
    starti: usize,
    is_absolute: bool,
) -> usize {
    let mut segment_count: usize = 0;
    let mut shift: usize = starti;
    let n = segments.len();
    for i in starti..n {
        let segment = segments[i];
        match segment {
            "." => {
                shift += 1;
                continue;
            }
            ".." => {
                if segment_count > 0 && segments[segment_count - 1] != ".." {
                    // Remove the last segment, if there is one and it is not "..". This
                    // means that the resulting path can still contain ".."
                    // segments at the beginning.
                    segment_count -= 1;
                    shift += 2;
                    continue;
                } else if is_absolute {
                    // If this is absolute, then just pop it the ".." off and remain at root
                    shift += 1;
                    continue;
                }
                // Fall through to add it to the segment list (Java `switch` fallthrough).
            }
            _ => {}
        }
        segment_count += 1;
        if shift > 0 {
            segments[i - shift] = segments[i];
        }
    }
    segment_count
}

/// The Unix path policy (Java `UnixOsPathPolicy`).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnixOsPathPolicy;

impl OsPathPolicy for UnixOsPathPolicy {
    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:27 UnixOsPathPolicy.needsToNormalize
    fn needs_to_normalize(&self, path: &str) -> i32 {
        let mut dot_count = 0;
        let mut prev_char: u8 = 0;
        for &c in path.as_bytes() {
            if c == b'\\' {
                return NEEDS_NORMALIZE;
            }
            if c == b'/' {
                if prev_char == b'/' {
                    return NEEDS_NORMALIZE;
                }
                if dot_count == 1 || dot_count == 2 {
                    return NEEDS_NORMALIZE;
                }
            }
            dot_count = if c == b'.' { dot_count + 1 } else { 0 };
            prev_char = c;
        }
        if prev_char == b'/' || dot_count == 1 || dot_count == 2 {
            return NEEDS_NORMALIZE;
        }
        NORMALIZED
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:54 UnixOsPathPolicy.needsToNormalizeSuffix
    fn needs_to_normalize_suffix(&self, normalized_suffix: &str) -> i32 {
        // We already know that normalizedSuffix is normalized, so we only need to check the start
        // of the string. The only way there can be a non-normalized path is if it starts with "..".
        if normalized_suffix.starts_with("..") {
            NEEDS_NORMALIZE
        } else {
            NORMALIZED
        }
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:62 UnixOsPathPolicy.normalize
    fn normalize(&self, path: &str, normalization_level: i32) -> String {
        if normalization_level == NORMALIZED {
            return path.to_owned();
        }
        if path.is_empty() {
            return path.to_owned();
        }
        let is_absolute = path.as_bytes()[0] == b'/';
        let mut sb = String::with_capacity(path.len());
        if is_absolute {
            sb.push('/');
        }
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let segment_count = remove_relative_paths(&mut segments, 0, is_absolute);
        for segment in segments.iter().take(segment_count) {
            sb.push_str(segment);
            sb.push('/');
        }
        if segment_count > 0 {
            sb.pop();
        }
        sb
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:87 UnixOsPathPolicy.getDriveStrLength
    fn get_drive_str_length(&self, path: &str) -> usize {
        if path.is_empty() {
            return 0;
        }
        if path.as_bytes()[0] == b'/' { 1 } else { 0 }
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:95 UnixOsPathPolicy.isSeparator
    fn is_separator(&self, c: u8) -> bool {
        c == b'/'
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:100 UnixOsPathPolicy.additionalSeparator
    fn additional_separator(&self) -> u8 {
        0
    }

    // Java: com/google/devtools/build/lib/vfs/UnixOsPathPolicy.java:105 UnixOsPathPolicy.postProcessPathStringForExecution
    fn post_process_path_string_for_execution(&self, callable_path_string: &str) -> String {
        callable_path_string.to_owned()
    }
}
