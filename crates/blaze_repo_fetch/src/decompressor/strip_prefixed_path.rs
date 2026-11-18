//! Port of `StripPrefixedPath`.

use blaze_vfs::{Path, PathFragment};

/// Utility class for removing a prefix from an archive's path.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/StripPrefixedPath.java:29 StripPrefixedPath
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripPrefixedPath {
    path_fragment: PathFragment,
    found: bool,
    skip: bool,
}

impl StripPrefixedPath {
    /// If a prefix is given, it will be removed from the entry's path. This also turns absolute
    /// paths into relative paths (e.g., /usr/bin/bash will become usr/bin/bash, same as unzip's
    /// default behavior) and normalizes the paths (foo/../bar////baz will become bar/baz). Note
    /// that this could cause collisions, if a zip file had one entry for bin/some-binary and
    /// another entry for /bin/some-binary.
    ///
    /// Note that the prefix is stripped to move the files up one level, so if you have an entry
    /// "foo/../bar" and a prefix of "foo", the result will be "bar" not "../bar".
    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/StripPrefixedPath.java:45 StripPrefixedPath.maybeDeprefix
    pub fn maybe_deprefix(entry: &[u8], prefix: Option<&str>) -> StripPrefixedPath {
        let mut entry_path = relativize(entry);
        let Some(prefix) = prefix else {
            return StripPrefixedPath {
                path_fragment: entry_path,
                found: false,
                skip: false,
            };
        };
        // Bazel parses Starlark files, which are the ultimate source of prefixes, as Latin-1.
        // PORT: StringEncoding identity.
        let prefix_path = relativize(prefix.as_bytes());
        let mut found = false;
        let skip;
        if entry_path.starts_with(&prefix_path) {
            found = true;
            entry_path = entry_path.relative_to(&prefix_path);
            skip = entry_path.get_path_string().is_empty();
        } else {
            skip = true;
        }
        StripPrefixedPath {
            path_fragment: entry_path,
            found,
            skip,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/StripPrefixedPath.java:93 StripPrefixedPath.maybeDeprefixSymlink
    pub fn maybe_deprefix_symlink(
        raw_target: &[u8],
        prefix: Option<&str>,
        root: &Path,
    ) -> PathFragment {
        let was_absolute = create_path_fragment(raw_target).is_absolute();
        // Strip the prefix from the link path if set.
        let link_path_fragment = Self::maybe_deprefix(raw_target, prefix).path_fragment;
        if was_absolute {
            // Recover the path to an absolute path as maybeDeprefix() relativize the path
            // even if the prefix is not set
            return root.get_relative(&link_path_fragment).as_fragment().clone();
        }
        link_path_fragment
    }

    pub fn get_path_fragment(&self) -> &PathFragment {
        &self.path_fragment
    }

    pub fn found_prefix(&self) -> bool {
        self.found
    }

    pub fn skip(&self) -> bool {
        self.skip
    }
}

/// Normalize the path and, if it is absolute, make it relative (e.g., /foo/bar becomes foo/bar).
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/StripPrefixedPath.java:75 StripPrefixedPath.relativize
fn relativize(path: &[u8]) -> PathFragment {
    let entry_path = create_path_fragment(path);
    if entry_path.is_absolute() {
        entry_path.to_relative()
    } else {
        entry_path
    }
}

/// PORT: Java decodes the raw bytes as Latin-1 (Bazel's internal string form); Rust paths are
/// UTF-8, so valid UTF-8 is taken as is and invalid sequences are replaced.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/StripPrefixedPath.java:111 StripPrefixedPath.createPathFragment
pub fn create_path_fragment(raw_bytes: &[u8]) -> PathFragment {
    PathFragment::create(&String::from_utf8_lossy(raw_bytes))
}
