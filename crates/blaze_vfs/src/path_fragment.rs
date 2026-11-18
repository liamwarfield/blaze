//! Port of `PathFragment` and `PathSegmentIterator`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use crate::os_path_policy::{HOST_POLICY, NORMALIZED, OsPathPolicy};

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:60 PathFragment.OS
const OS: &crate::os_path_policy::UnixOsPathPolicy = &HOST_POLICY;

/// The path separator.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:65 PathFragment.SEPARATOR_CHAR
pub const SEPARATOR_CHAR: char = '/';
const SEPARATOR: u8 = b'/';

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:116 PathFragment.ADDITIONAL_SEPARATOR_CHAR
fn additional_separator_char() -> u8 {
    OS.additional_separator()
}

static EMPTY_FRAGMENT: LazyLock<PathFragment> = LazyLock::new(|| PathFragment {
    normalized_path: Arc::from(""),
    drive_str_length: 0,
});

/// A normalized path, absolute or relative, using the host (Unix) path style (Java
/// `PathFragment`).
///
/// Strings are normalized with '.' and '..' removed and resolved (if possible), any multiple
/// slashes ('/') removed, and any trailing slash also removed. All paths are case-sensitive.
///
/// The value is immutable and cheap to clone (one `Arc<str>`). Equality, hashing and ordering are
/// those of the normalized string; ordering is byte-wise, which equals Java's `String.compareTo`
/// on Bazel's internal (Latin-1 coded raw UTF-8) strings.
///
/// Windows-style absolute paths (`C:/...`, Java `WindowsStyleAbsolutePathFragment`) are not
/// ported.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:58 PathFragment
#[derive(Clone)]
pub struct PathFragment {
    normalized_path: Arc<str>,
    /// 0 for relative paths (Java `RelativePathFragment`), 1 for Unix-style absolute paths (Java
    /// `UnixStyleAbsolutePathFragment`).
    drive_str_length: u8,
}

/// Indicates that a path fragment's base name had invalid characters.
// Java: com/google/devtools/build/lib/vfs/PathFragment.java:960 PathFragment.InvalidBaseNameException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidBaseNameException {
    message: String,
}

impl InvalidBaseNameException {
    /// The exception message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for InvalidBaseNameException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidBaseNameException {}

impl PathFragment {
    /// The empty relative path fragment (Java `PathFragment.EMPTY_FRAGMENT`).
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:63 PathFragment.EMPTY_FRAGMENT
    pub fn empty_fragment() -> PathFragment {
        EMPTY_FRAGMENT.clone()
    }

    /// Creates a new normalized path fragment.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:125 PathFragment.create
    pub fn create(path: &str) -> PathFragment {
        Self::create_internal(path, OS)
    }

    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:133 PathFragment.createInternal
    fn create_internal(path: &str, os_path_policy: &dyn OsPathPolicy) -> PathFragment {
        if path.is_empty() {
            return Self::empty_fragment();
        }
        let normalization_level = os_path_policy.needs_to_normalize(path);
        let normalized_path = if normalization_level != NORMALIZED {
            os_path_policy.normalize(path, normalization_level)
        } else {
            path.to_owned()
        };
        let drive_str_length = os_path_policy.get_drive_str_length(&normalized_path);
        Self::make_path_fragment(Arc::from(normalized_path), drive_str_length)
    }

    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:146 PathFragment.makePathFragment
    fn make_path_fragment(normalized_path: Arc<str>, drive_str_length: usize) -> PathFragment {
        match drive_str_length {
            0 | 1 => PathFragment {
                normalized_path,
                drive_str_length: drive_str_length as u8,
            },
            // 3 would be WindowsStyleAbsolutePathFragment, which is not ported.
            _ => panic!(
                "normalizedPath: {}, driveStrLength: {}",
                normalized_path, drive_str_length
            ),
        }
    }

    fn make(normalized_path: &str, drive_str_length: usize) -> PathFragment {
        if normalized_path.is_empty() && drive_str_length == 0 {
            return Self::empty_fragment();
        }
        Self::make_path_fragment(Arc::from(normalized_path), drive_str_length)
    }

    /// Creates a new path fragment, where the caller promises that the path is normalized.
    ///
    /// WARNING! Make sure the path fragment is in fact already normalized.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:164 PathFragment.createAlreadyNormalized
    pub fn create_already_normalized(normalized_path: &str) -> PathFragment {
        if normalized_path.is_empty() {
            Self::empty_fragment()
        } else {
            Self::make_path_fragment(
                Arc::from(normalized_path),
                OS.get_drive_str_length(normalized_path),
            )
        }
    }

    /// Returns the normalized path string.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:175 PathFragment.getPathString
    pub fn get_path_string(&self) -> &str {
        &self.normalized_path
    }

    /// Returns the normalized path string as a shared `Arc<str>`.
    pub fn get_path_arc(&self) -> &Arc<str> {
        &self.normalized_path
    }

    /// Whether this is the empty fragment.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:179 PathFragment.isEmpty
    pub fn is_empty(&self) -> bool {
        self.normalized_path.is_empty()
    }

    /// Returns 0 for relative paths (e.g. "a/b") and 1 for Unix-style absolute paths (e.g. "/a/b").
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:187 PathFragment.getDriveStrLength
    pub fn get_drive_str_length(&self) -> usize {
        self.drive_str_length as usize
    }

    /// Returns true if both values share the same underlying allocation (Java reference
    /// equality, `isSameInstanceAs`).
    pub fn ptr_eq(a: &PathFragment, b: &PathFragment) -> bool {
        Arc::ptr_eq(&a.normalized_path, &b.normalized_path)
    }

    fn bytes(&self) -> &[u8] {
        self.normalized_path.as_bytes()
    }

    fn last_index_of_separator(&self) -> Option<usize> {
        self.bytes().iter().rposition(|&c| c == SEPARATOR)
    }

    /// Returns the last segment. For the mount name ('/') or the empty fragment, the empty string
    /// is returned.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:240 PathFragment.getBaseName
    pub fn get_base_name(&self) -> &str {
        let drive = self.get_drive_str_length();
        match self.last_index_of_separator() {
            Some(last_separator) if last_separator >= drive => {
                &self.normalized_path[last_separator + 1..]
            }
            _ => &self.normalized_path[drive..],
        }
    }

    /// Returns a path fragment formed by resolving `other` relative to this path. If `other` is
    /// absolute it is returned untouched (the same instance).
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:254 PathFragment.getRelative
    pub fn get_relative(&self, other: &PathFragment) -> PathFragment {
        if self.is_empty() || other.is_absolute() {
            return other.clone();
        }
        // The path fragment is already normal, use cheaper normalization check.
        let other_str = other.get_path_string();
        self.get_relative_impl(
            other_str,
            other.get_drive_str_length(),
            OS.needs_to_normalize_suffix(other_str),
        )
    }

    /// Returns a path fragment formed by resolving the string `other` relative to this path.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:270 PathFragment.getRelative
    pub fn get_relative_str(&self, other: &str) -> PathFragment {
        self.get_relative_impl(
            other,
            OS.get_drive_str_length(other),
            OS.needs_to_normalize(other),
        )
    }

    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:275 PathFragment.getRelative
    fn get_relative_impl(
        &self,
        other: &str,
        other_drive_str_length: usize,
        normalization_level: i32,
    ) -> PathFragment {
        if self.is_empty() {
            return Self::create(other);
        }
        if other.is_empty() {
            return self.clone();
        }
        // This is an absolute path, simply return it
        if other_drive_str_length > 0 {
            let normalized_path = if normalization_level != NORMALIZED {
                OS.normalize(other, normalization_level)
            } else {
                other.to_owned()
            };
            return Self::make_path_fragment(Arc::from(normalized_path), other_drive_str_length);
        }
        let mut new_path = if self.normalized_path.len() == self.get_drive_str_length() {
            format!("{}{}", self.normalized_path, other)
        } else {
            format!("{}/{}", self.normalized_path, other)
        };
        if normalization_level != NORMALIZED {
            new_path = OS.normalize(&new_path, normalization_level);
        }
        Self::make(&new_path, self.get_drive_str_length())
    }

    /// Whether `path` is relative and already normalized.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:303 PathFragment.isNormalizedRelativePath
    pub fn is_normalized_relative_path(path: &str) -> bool {
        let drive_str_length = OS.get_drive_str_length(path);
        let normalization_level = OS.needs_to_normalize(path);
        drive_str_length == 0 && normalization_level == NORMALIZED
    }

    /// Whether `path` contains a separator.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:309 PathFragment.containsSeparator
    pub fn contains_separator(path: &str) -> bool {
        path.as_bytes().contains(&SEPARATOR)
    }

    /// Returns this path with `base_name` appended.
    ///
    /// Panics (Java `IllegalArgumentException`) if `base_name` is empty, `.`, `..` or contains a
    /// separator.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:313 PathFragment.getChild
    pub fn get_child(&self, base_name: &str) -> PathFragment {
        check_base_name(base_name);
        let new_path = if self.normalized_path.len() == self.get_drive_str_length() {
            format!("{}{}", self.normalized_path, base_name)
        } else {
            format!("{}/{}", self.normalized_path, base_name)
        };
        Self::make_path_fragment(Arc::from(new_path), self.get_drive_str_length())
    }

    /// Returns the parent directory. For a single relative segment this is the empty fragment;
    /// for the root ('/') or the empty fragment it is `None`.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:331 PathFragment.getParentDirectory
    pub fn get_parent_directory(&self) -> Option<PathFragment> {
        let last_separator = self.last_index_of_separator();
        let drive = self.get_drive_str_length();
        // For absolute paths we need to specially handle when we hit root
        // Relative paths can't hit this path as driveStrLength == 0
        if drive > 0 {
            if last_separator.is_none_or(|ls| ls < drive) {
                if self.normalized_path.len() > drive {
                    return Some(Self::make(&self.normalized_path[..drive], drive));
                } else {
                    return None;
                }
            }
        } else if last_separator.is_none() {
            if !self.is_empty() {
                return Some(Self::empty_fragment());
            } else {
                return None;
            }
        }
        let last_separator = last_separator.expect("checked above");
        Some(Self::make(&self.normalized_path[..last_separator], drive))
    }

    /// Returns this fragment relative to `base`.
    ///
    /// Panics (Java `IllegalArgumentException`) if this is not under `base`, in particular when
    /// the two are not both absolute or both relative.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:369 PathFragment.relativeTo
    pub fn relative_to(&self, base: &PathFragment) -> PathFragment {
        match self.try_relative_to(base) {
            Ok(p) => p,
            Err(message) => panic!("{}", message),
        }
    }

    /// Like [`PathFragment::relative_to`], but returns the `IllegalArgumentException` message
    /// instead of panicking.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:369 PathFragment.relativeTo
    pub fn try_relative_to(&self, base: &PathFragment) -> Result<PathFragment, String> {
        if self.is_absolute() != base.is_absolute() {
            return Err("Cannot relativize an absolute and a non-absolute path pair".to_owned());
        }
        let base_path = base.get_path_string();
        if !self.normalized_path.starts_with(base_path) {
            return Err(format!(
                "Path '{}' is not under '{}', cannot relativize",
                self, base
            ));
        }
        let bn = base_path.len();
        if bn == 0 {
            return Ok(self.clone());
        }
        if self.normalized_path.len() == bn {
            return Ok(Self::empty_fragment());
        }
        let last_slash_index = if base_path.as_bytes()[bn - 1] == b'/' {
            bn - 1
        } else {
            bn
        };
        if self.bytes()[last_slash_index] != b'/' {
            return Err(format!(
                "Path '{}' is not under '{}', cannot relativize",
                self, base
            ));
        }
        Ok(Self::make(&self.normalized_path[last_slash_index + 1..], 0))
    }

    /// [`PathFragment::relative_to`] with a string base.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:401 PathFragment.relativeTo
    pub fn relative_to_str(&self, base: &str) -> PathFragment {
        self.relative_to(&PathFragment::create(base))
    }

    /// Returns true iff `other` is an ancestor of this path (or equal to it). An absolute path can
    /// never be an ancestor of a relative path, and vice versa.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:412 PathFragment.startsWith
    pub fn starts_with(&self, other: &PathFragment) -> bool {
        let other_len = other.normalized_path.len();
        if other_len > self.normalized_path.len() {
            return false;
        }
        if self.get_drive_str_length() != other.get_drive_str_length() {
            return false;
        }
        if !self.normalized_path.starts_with(&*other.normalized_path) {
            return false;
        }
        self.normalized_path.len() == other_len
            || other_len == self.get_drive_str_length()
            || self.bytes()[other_len] == SEPARATOR
    }

    /// Returns true iff `other` is an ancestor of this path, ignoring case.
    ///
    /// Java compares UTF-16 code units with `String.regionMatches(ignoreCase=true, ...)`; this
    /// compares Unicode scalar values with the same simple (single-char) case mappings.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:435 PathFragment.startsWithIgnoringCase
    pub fn starts_with_ignoring_case(&self, other: &PathFragment) -> bool {
        // Drive strings are ASCII only and hence can be checked without conversion to Unicode.
        if self.get_drive_str_length() != other.get_drive_str_length() {
            return false;
        }
        let this_chars: Vec<char> = self.normalized_path.chars().collect();
        let other_chars: Vec<char> = other.normalized_path.chars().collect();
        if other_chars.len() > this_chars.len() {
            return false;
        }
        if !this_chars
            .iter()
            .zip(other_chars.iter())
            .all(|(&a, &b)| chars_equal_ignoring_case(a, b))
        {
            return false;
        }
        this_chars.len() == other_chars.len()
            || other.normalized_path.len() == self.get_drive_str_length()
            || this_chars[other_chars.len()] == SEPARATOR_CHAR
    }

    /// Returns true iff `other`, considered as a list of path segments, is relative and a suffix
    /// of this, or both are absolute and equal.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:462 PathFragment.endsWith
    pub fn ends_with(&self, other: &PathFragment) -> bool {
        let other_len = other.normalized_path.len();
        let len = self.normalized_path.len();
        if other_len > len {
            return false;
        }
        if other.is_absolute() {
            return self == other;
        }
        if !self.normalized_path.ends_with(&*other.normalized_path) {
            return false;
        }
        len == other_len || other.is_empty() || self.bytes()[len - other_len - 1] == SEPARATOR
    }

    /// Whether this is an absolute path.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:479 PathFragment.isAbsolute
    pub fn is_absolute(&self) -> bool {
        self.get_drive_str_length() > 0
    }

    /// Whether the string `path` is absolute.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:483 PathFragment.isAbsolute
    pub fn is_absolute_str(path: &str) -> bool {
        OS.get_drive_str_length(path) > 0
    }

    /// Returns the number of segments in this path, excluding the drive string.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:525 PathFragment.segmentCount
    pub fn segment_count(&self) -> usize {
        let n = self.normalized_path.len();
        let drive = self.get_drive_str_length();
        let mut segment_count = 0;
        let mut i = drive;
        while i < n {
            if self.bytes()[i] == SEPARATOR {
                segment_count += 1;
            }
            i += 1;
        }
        // Add last segment if one exists.
        if i > drive {
            segment_count += 1;
        }
        segment_count
    }

    /// Whether this path consists of a single segment, excluding the drive string.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:548 PathFragment.isSingleSegment
    pub fn is_single_segment(&self) -> bool {
        self.normalized_path.len() > self.get_drive_str_length() && !self.is_multi_segment()
    }

    /// Whether this path consists of multiple segments, excluding the drive string.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:559 PathFragment.isMultiSegment
    pub fn is_multi_segment(&self) -> bool {
        self.bytes()[self.get_drive_str_length().min(self.bytes().len())..].contains(&SEPARATOR)
    }

    /// Returns the specified segment.
    ///
    /// Panics (Java `IllegalArgumentException`: "Illegal segment index: N") if out of range.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:569 PathFragment.getSegment
    pub fn get_segment(&self, index: usize) -> &str {
        let n = self.normalized_path.len();
        let drive = self.get_drive_str_length();
        let b = self.bytes();
        let mut segment_count = 0;
        let mut i = drive;
        while i < n && segment_count < index {
            if b[i] == SEPARATOR {
                segment_count += 1;
            }
            i += 1;
        }
        let starti = i;
        while i < n {
            if b[i] == SEPARATOR {
                break;
            }
            i += 1;
        }
        // Add last segment if one exists.
        if i > drive {
            segment_count += 1;
        }
        let endi = i;
        if index >= segment_count {
            panic!("Illegal segment index: {}", index);
        }
        &self.normalized_path[starti..endi]
    }

    /// Returns the sub fragment of segments `[begin_index, end_index)`. If the path is absolute
    /// and `begin_index` is zero, the result is absolute.
    ///
    /// Panics (Java `IndexOutOfBoundsException`) on invalid indices.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:614 PathFragment.subFragment
    pub fn sub_fragment(&self, begin_index: usize, end_index: usize) -> PathFragment {
        if begin_index > end_index {
            panic!(
                "path: {}, beginIndex: {} endIndex: {}",
                self, begin_index, end_index
            );
        }
        self.sub_fragment_impl(begin_index, end_index as i64)
    }

    /// Returns the sub fragment of segments from `begin_index` to the end.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:622 PathFragment.subFragment
    pub fn sub_fragment_from(&self, begin_index: usize) -> PathFragment {
        self.sub_fragment_impl(begin_index, -1)
    }

    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:630 PathFragment.subFragmentImpl
    fn sub_fragment_impl(&self, begin_index: usize, end_index: i64) -> PathFragment {
        let n = self.normalized_path.len();
        let drive = self.get_drive_str_length();
        let b = self.bytes();
        let mut segment_index: i64 = 0;
        let begin = begin_index as i64;
        let mut i = drive;
        while i < n && segment_index < begin {
            if b[i] == SEPARATOR {
                segment_index += 1;
            }
            i += 1;
        }
        let mut starti = i;
        if segment_index < end_index {
            while i < n {
                if b[i] == SEPARATOR {
                    segment_index += 1;
                    if segment_index == end_index {
                        break;
                    }
                }
                i += 1;
            }
        } else if end_index == -1 {
            i = n;
        }
        let mut endi = i;
        // Add last segment if one exists for verification
        if i == n && i > drive {
            segment_index += 1;
        }
        if begin > segment_index || end_index > segment_index {
            panic!(
                "path: {}, beginIndex: {} endIndex: {}",
                self, begin_index, end_index
            );
        }
        // If beginIndex is 0, we include the drive string.
        let mut drive_str_length = 0;
        if begin_index == 0 {
            starti = 0;
            drive_str_length = drive;
            endi = endi.max(drive_str_length);
        }
        Self::make(&self.normalized_path[starti..endi], drive_str_length)
    }

    /// Strip `num_components` leading components.
    ///
    /// Panics (Java `IllegalArgumentException`) if `num_components` is negative.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:672 PathFragment.stripComponents
    pub fn strip_components(&self, num_components: i32) -> PathFragment {
        if num_components == 0 {
            return self.clone();
        }
        if num_components < 0 {
            panic!("Invalid number of components ({})", num_components);
        }
        if num_components as usize >= self.segment_count() {
            return Self::empty_fragment();
        }
        self.sub_fragment_from(num_components as usize)
    }

    /// Lazily yields the segments of this path.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:693 PathFragment.segments
    pub fn segments(&self) -> PathSegmentIterator<'_> {
        PathSegmentIterator::create(&self.normalized_path, self.get_drive_str_length())
    }

    /// Splits this path fragment into a list of segments.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:703 PathFragment.splitToListOfSegments
    pub fn split_to_list_of_segments(&self) -> Vec<String> {
        let mut segments = Vec::with_capacity(self.segment_count());
        let drive = self.get_drive_str_length();
        let mut nexti = drive;
        let n = self.normalized_path.len();
        for i in drive..n {
            if self.bytes()[i] == SEPARATOR {
                segments.push(self.normalized_path[nexti..i].to_owned());
                nexti = i + 1;
            }
        }
        // Add last segment if one exists.
        if nexti < n {
            segments.push(self.normalized_path[nexti..].to_owned());
        }
        segments
    }

    /// Returns the path string, or '.' if the path is empty.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:721 PathFragment.getSafePathString
    pub fn get_safe_path_string(&self) -> &str {
        if !self.is_empty() {
            &self.normalized_path
        } else {
            "."
        }
    }

    /// Returns the path string in a form a shell recognizes as a path: "." for the empty path and
    /// "./x" for single-segment relative paths.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:735 PathFragment.getCallablePathString
    pub fn get_callable_path_string(&self) -> String {
        if self.is_absolute() {
            self.normalized_path.to_string()
        } else if self.is_empty() {
            ".".to_owned()
        } else if !self.bytes().contains(&SEPARATOR) {
            format!(".{}{}", SEPARATOR_CHAR, self.normalized_path)
        } else {
            self.normalized_path.to_string()
        }
    }

    /// [`PathFragment::get_callable_path_string`] post-processed for the execution OS policy.
    /// Only the Unix policy exists in Blaze.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:755 PathFragment.getCallablePathStringForOs
    pub fn get_callable_path_string_for_os(&self, execution_os: &dyn OsPathPolicy) -> String {
        execution_os.post_process_path_string_for_execution(&self.get_callable_path_string())
    }

    /// Returns the file extension, excluding the period, or "" if there is none.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:762 PathFragment.getFileExtension
    pub fn get_file_extension(&self) -> &str {
        let n = self.normalized_path.len();
        let drive = self.get_drive_str_length();
        let mut i = n;
        while i > drive + 1 {
            i -= 1;
            let c = self.bytes()[i];
            if c == b'.' {
                return &self.normalized_path[i + 1..n];
            } else if c == SEPARATOR {
                break;
            }
        }
        ""
    }

    /// Returns the parent directory with `new_name` resolved relative to it, or `None` if this
    /// has zero segments.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:783 PathFragment.replaceName
    pub fn replace_name(&self, new_name: &str) -> Option<PathFragment> {
        self.get_parent_directory()
            .map(|parent| parent.get_relative_str(new_name))
    }

    /// Returns the drive ("/") of an absolute path. Panics if relative.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:793 PathFragment.getDriveStr
    pub fn get_drive_str(&self) -> &str {
        assert!(self.is_absolute());
        &self.normalized_path[..self.get_drive_str_length()]
    }

    /// Returns a relative fragment with the same segments. Panics if relative.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:802 PathFragment.toRelative
    pub fn to_relative(&self) -> PathFragment {
        assert!(self.is_absolute());
        Self::make(&self.normalized_path[self.get_drive_str_length()..], 0)
    }

    /// Returns true if this path contains uplevel references "..". Since path fragments are
    /// normalized, the uplevel reference can only be at the start.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:813 PathFragment.containsUplevelReferences
    pub fn contains_uplevel_references(&self) -> bool {
        // Path is normalized, so any ".." would have to be the first segment.
        self.normalized_path.starts_with("..")
            && (self.normalized_path.len() == 2 || self.bytes()[2] == SEPARATOR)
    }

    /// Returns true if the passed path string contains uplevel references "..".
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:825 PathFragment.containsUplevelReferences
    pub fn contains_uplevel_references_str(path: &str) -> bool {
        !is_normalized_impl(path, false)
    }

    /// Returns true if the passed path does not contain ".." or "." segments.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:836 PathFragment.isNormalized
    pub fn is_normalized(path: &str) -> bool {
        is_normalized_impl(path, true)
    }

    /// Panics (Java `IllegalArgumentException`) if `paths` contains any path that is equal to
    /// `starting_with_path` or not beneath it.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:913 PathFragment.checkAllPathsAreUnder
    pub fn check_all_paths_are_under<'a>(
        paths: impl IntoIterator<Item = &'a PathFragment>,
        starting_with_path: &PathFragment,
    ) {
        for path in paths {
            assert!(
                path != starting_with_path && path.starts_with(starting_with_path),
                "{} is not beneath {}",
                path,
                starting_with_path
            );
        }
    }

    /// The string used by file type matchers.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:925 PathFragment.filePathForFileTypeMatcher
    pub fn file_path_for_file_type_matcher(&self) -> &str {
        &self.normalized_path
    }

    /// Checks that `base_name` contains no separator.
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:948 PathFragment.checkSeparators
    pub fn check_separators(base_name: &str) -> Result<(), InvalidBaseNameException> {
        if base_name.as_bytes().contains(&SEPARATOR) {
            return Err(InvalidBaseNameException {
                message: format!("must not contain {}", SEPARATOR_CHAR),
            });
        }
        let additional = additional_separator_char();
        if additional != 0 && base_name.as_bytes().contains(&additional) {
            return Err(InvalidBaseNameException {
                message: format!("must not contain {}", additional as char),
            });
        }
        Ok(())
    }

    /// Compares two path fragments lexicographically as sequences of path segments, so that the
    /// children of a path sort directly after it (Java `PathFragment.HIERARCHICAL_COMPARATOR`).
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java:84 PathFragment.HIERARCHICAL_COMPARATOR
    pub fn hierarchical_compare(p1: &PathFragment, p2: &PathFragment) -> Ordering {
        // Bazel's Strings contain raw UTF-8 bytes (see StringEncoding), which can be compared
        // byte-by-byte.
        let b1 = p1.bytes();
        let b2 = p2.bytes();
        // This is based on String.compareTo for the case of two Latin-1 coders.
        let k = match b1.iter().zip(b2.iter()).position(|(a, b)| a != b) {
            Some(k) => k,
            None => {
                if b1.len() == b2.len() {
                    return Ordering::Equal;
                }
                b1.len().min(b2.len())
            }
        };
        if k >= b1.len() {
            // b1 is a prefix of b2.
            return Ordering::Less;
        }
        if k >= b2.len() {
            // b2 is a prefix of b1.
            return Ordering::Greater;
        }
        let c1 = b1[k];
        let c2 = b2[k];
        if c1 == b'/' {
            // Sort a/b/c before a/b-c.
            return Ordering::Less;
        }
        if c2 == b'/' {
            // Sort a/b-c after a/b/c.
            return Ordering::Greater;
        }
        c1.cmp(&c2)
    }
}

// Java String.regionMatches(ignoreCase=true): chars match if equal, or their upper cases are
// equal, or the lower cases of their upper cases are equal.
fn chars_equal_ignoring_case(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let ua = to_upper_simple(a);
    let ub = to_upper_simple(b);
    if ua == ub {
        return true;
    }
    to_lower_simple(ua) == to_lower_simple(ub)
}

fn to_upper_simple(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

fn to_lower_simple(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:840 PathFragment.NormalizedImplState
#[derive(Clone, Copy, PartialEq, Eq)]
enum NormalizedImplState {
    /// No particular state, eg. an 'a' or 'L' character
    Base,
    /// We just saw a separator
    Separator,
    /// We just saw a dot after a separator
    Dot,
    /// We just saw two dots after a separator
    DotDot,
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:847 PathFragment.isNormalizedImpl
fn is_normalized_impl(path: &str, look_for_same_level_references: bool) -> bool {
    use NormalizedImplState::*;
    // Starting state is equivalent to having just seen a separator
    let mut state = Separator;
    for &c in path.as_bytes() {
        let is_separator = OS.is_separator(c);
        match state {
            Base => {
                state = if is_separator { Separator } else { Base };
            }
            Separator => {
                state = if is_separator {
                    Separator
                } else if c == b'.' {
                    Dot
                } else {
                    Base
                };
            }
            Dot => {
                if is_separator {
                    if look_for_same_level_references {
                        // "." segment found
                        return false;
                    }
                    state = Separator;
                } else if c == b'.' {
                    state = DotDot;
                } else {
                    state = Base;
                }
            }
            DotDot => {
                if is_separator {
                    // ".." segment found
                    return false;
                } else {
                    state = Base;
                }
            }
        }
    }
    // The character just after the string is equivalent to a separator
    match state {
        Dot => {
            if look_for_same_level_references {
                // "." segment found
                return false;
            }
        }
        DotDot => return false,
        _ => {}
    }
    true
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:934 PathFragment.checkBaseName
fn check_base_name(base_name: &str) {
    if base_name.is_empty() {
        panic!("Child must not be empty string ('')");
    }
    if base_name == "." || base_name == ".." {
        panic!("baseName must not be '{}'", base_name);
    }
    if let Err(e) = PathFragment::check_separators(base_name) {
        panic!("baseName {}: '{}'", e.message, base_name);
    }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:493 PathFragment.equals
impl PartialEq for PathFragment {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.normalized_path, &other.normalized_path)
            || self.normalized_path == other.normalized_path
    }
}

impl Eq for PathFragment {}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:501 PathFragment.hashCode
impl Hash for PathFragment {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.normalized_path.hash(state);
    }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:514 PathFragment.compareTo
impl Ord for PathFragment {
    fn cmp(&self, other: &Self) -> Ordering {
        self.normalized_path
            .as_bytes()
            .cmp(other.normalized_path.as_bytes())
    }
}

impl PartialOrd for PathFragment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Java: com/google/devtools/build/lib/vfs/PathFragment.java:488 PathFragment.toString
impl fmt::Display for PathFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized_path)
    }
}

impl fmt::Debug for PathFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PathFragment({:?})", &*self.normalized_path)
    }
}

impl AsRef<str> for PathFragment {
    fn as_ref(&self) -> &str {
        &self.normalized_path
    }
}

/// Lazily iterates over the segments of a normalized path string (Java `PathSegmentIterator`).
///
/// Java throws `NoSuchElementException("No more segments: <path>")` past the end; the Rust
/// iterator returns `None`.
// Java: com/google/devtools/build/lib/vfs/PathSegmentIterator.java:25 PathSegmentIterator
#[derive(Clone, Debug)]
pub struct PathSegmentIterator<'a> {
    normalized_path: &'a str,
    start: usize,
}

impl<'a> PathSegmentIterator<'a> {
    /// Creates an iterator over `normalized_path`, skipping the first `drive_str_length` bytes.
    // Java: com/google/devtools/build/lib/vfs/PathSegmentIterator.java:27 PathSegmentIterator.create
    pub fn create(normalized_path: &'a str, drive_str_length: usize) -> PathSegmentIterator<'a> {
        PathSegmentIterator {
            normalized_path,
            start: if normalized_path.len() > drive_str_length {
                drive_str_length
            } else {
                normalized_path.len()
            },
        }
    }
}

impl<'a> Iterator for PathSegmentIterator<'a> {
    type Item = &'a str;

    // Java: com/google/devtools/build/lib/vfs/PathSegmentIterator.java:47 PathSegmentIterator.next
    fn next(&mut self) -> Option<&'a str> {
        // Java: com/google/devtools/build/lib/vfs/PathSegmentIterator.java:42 PathSegmentIterator.hasNext
        if self.start >= self.normalized_path.len() {
            return None;
        }
        let b = self.normalized_path.as_bytes();
        let mut end = self.start + 1;
        while end < b.len() && b[end] != SEPARATOR {
            end += 1;
        }
        let segment = &self.normalized_path[self.start..end];
        self.start = end + 1;
        Some(segment)
    }
}
