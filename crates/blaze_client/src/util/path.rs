//! Port of `src/main/cpp/util/path.cc`, `path_posix.cc` and the `Path` class of
//! `path_platform.h`.

use super::exit_code;
use super::file::get_cwd;
use super::logging::bazel_die;

const PATH_POSIX_CC: &str = "src/main/cpp/util/path_posix.cc";

// Java: src/main/cpp/util/path.cc:22 Dirname
pub fn dirname(path: &str) -> String {
    split_path(path).0
}

// Java: src/main/cpp/util/path.cc:24 Basename
pub fn basename(path: &str) -> String {
    split_path(path).1
}

// Java: src/main/cpp/util/path.cc:26 JoinPath
pub fn join_path(path1: &str, path2: &str) -> String {
    if path1.is_empty() {
        // "" + "/bar"
        return path2.to_string();
    }
    if path1.ends_with('/') {
        if let Some(rest) = path2.strip_prefix('/') {
            // foo/ + /bar
            format!("{}{}", path1, rest)
        } else {
            // foo/ + bar
            format!("{}{}", path1, path2)
        }
    } else if path2.starts_with('/') {
        // foo + /bar
        format!("{}{}", path1, path2)
    } else {
        // foo + bar
        format!("{}/{}", path1, path2)
    }
}

// Java: src/main/cpp/util/path_posix.cc:30 ConvertPath
pub fn convert_path(path: &str) -> String {
    path.to_string()
}

// Java: src/main/cpp/util/path_posix.cc:32 PathAsJvmFlag
pub fn path_as_jvm_flag(path: &str) -> String {
    path.to_string()
}

// Java: src/main/cpp/util/path_posix.cc:34 ArePathsEquivalent
pub fn are_paths_equivalent(a: &Path, b: &Path) -> bool {
    a == b
}

// Java: src/main/cpp/util/path_posix.cc:38 SplitPath
pub fn split_path(path: &str) -> (String, String) {
    let Some(pos) = path.rfind('/') else {
        // Handle the case with no '/' in 'path'.
        return (String::new(), path.to_string());
    };
    // Handle the case with a single leading '/' in 'path'.
    if pos == 0 {
        return (path[..1].to_string(), path[1..].to_string());
    }
    (path[..pos].to_string(), path[pos + 1..].to_string())
}

// Java: src/main/cpp/util/path_posix.cc:51 IsDevNull
pub fn is_dev_null(path: Option<&str>) -> bool {
    match path {
        Some(p) => {
            // strncmp("/dev/null\0", path, 10) == 0: `path` must be exactly "/dev/null" up to its
            // first NUL.
            let p = match p.find('\0') {
                Some(i) => &p[..i],
                None => p,
            };
            !p.is_empty() && p == "/dev/null"
        }
        None => false,
    }
}

// Java: src/main/cpp/util/path_posix.cc:55 IsRootDirectory
pub fn is_root_directory(path: &str) -> bool {
    path.len() == 1 && path.as_bytes()[0] == b'/'
}

// Java: src/main/cpp/util/path_posix.cc:59 IsRootDirectory
pub fn is_root_directory_path(path: &Path) -> bool {
    is_root_directory(&path.as_native_path())
}

// Java: src/main/cpp/util/path_posix.cc:63 IsAbsolute
pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

// Java: src/main/cpp/util/path_posix.cc:67 MakeAbsolute
pub fn make_absolute(path: &str) -> String {
    if is_absolute(path) || path.is_empty() {
        return path.to_string();
    }
    join_path(&get_cwd(), path)
}

// Java: src/main/cpp/util/path_posix.cc:75 ResolveEnvvars
pub fn resolve_envvars(path: &str) -> String {
    let mut result = path.to_string();
    let mut start = 0;
    while let Some(s) = result[start..].find("${").map(|i| i + start) {
        // Just match to the next }
        let Some(end) = result[s + 1..].find('}').map(|i| i + s + 1) else {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                PATH_POSIX_CC,
                82,
                format!(
                    "ResolveEnvvars({}): incomplete variable at position {}",
                    path, s
                ),
            );
        };
        // Extract the variable name
        let name = result[s + 2..end].to_string();
        // Get the value from the environment
        let value = crate::platform::get_env(&name);
        result.replace_range(s..=end, &value);
        start = s + value.len();
    }
    result
}

// Java: src/main/cpp/util/path_posix.cc:97 MakeAbsoluteAndResolveEnvvars
pub fn make_absolute_and_resolve_envvars(path: &str) -> String {
    make_absolute(&resolve_envvars(path))
}

// Java: src/main/cpp/util/path_posix.cc:101 NormalizeAbsPath
fn normalize_abs_path(p: &str) -> String {
    if !p.starts_with('/') {
        return String::new();
    }
    let mut segments: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == ".." {
            segments.pop();
        } else if seg != "." {
            segments.push(seg);
        }
    }
    if segments.is_empty() {
        "/".to_string()
    } else {
        let mut r = String::new();
        for s in &segments {
            r.push('/');
            r.push_str(s);
        }
        if p.ends_with('/') {
            r.push('/');
        }
        r
    }
}

// Java: src/main/cpp/util/path_posix.cc:136 TestOnly_NormalizeAbsPath
pub fn test_only_normalize_abs_path(s: &str) -> String {
    normalize_abs_path(s)
}

// Java: src/main/cpp/util/path_platform.h:24 Path
/// An absolute, normalized path (or the empty path).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path {
    path: String,
}

impl Path {
    // Java: src/main/cpp/util/path_posix.cc:140 Path::Path
    pub fn new(path: &str) -> Path {
        Path {
            path: normalize_abs_path(&make_absolute(path)),
        }
    }

    // Java: src/main/cpp/util/path_platform.h:32 Path::IsEmpty
    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    // Java: src/main/cpp/util/path_posix.cc:146 Path::IsNull
    pub fn is_null(&self) -> bool {
        self.path == "/dev/null"
    }

    // Java: src/main/cpp/util/path_posix.cc:148 Path::Contains
    pub fn contains_char(&self, c: char) -> bool {
        self.path.contains(c)
    }

    // Java: src/main/cpp/util/path_posix.cc:152 Path::Contains
    pub fn contains(&self, s: &str) -> bool {
        self.path.contains(s)
    }

    // Java: src/main/cpp/util/path_posix.cc:156 Path::GetRelative
    pub fn get_relative(&self, r: &str) -> Path {
        Path::new(&join_path(&self.path, r))
    }

    // Java: src/main/cpp/util/path_posix.cc:160 Path::Canonicalize
    pub fn canonicalize(&self) -> Path {
        Path::new(&super::file::make_canonical(&self.path))
    }

    // Java: src/main/cpp/util/path_posix.cc:162 Path::GetBaseName
    pub fn get_base_name(&self) -> String {
        split_path(&self.path).1
    }

    // Java: src/main/cpp/util/path_posix.cc:164 Path::GetParent
    pub fn get_parent(&self) -> Path {
        Path::new(&split_path(&self.path).0)
    }

    // Java: src/main/cpp/util/path_posix.cc:166 Path::AsPrintablePath
    pub fn as_printable_path(&self) -> String {
        self.path.clone()
    }

    // Java: src/main/cpp/util/path_posix.cc:168 Path::AsJvmArgument
    pub fn as_jvm_argument(&self) -> String {
        self.path.clone()
    }

    // Java: src/main/cpp/util/path_posix.cc:170 Path::AsCommandLineArgument
    pub fn as_command_line_argument(&self) -> String {
        self.path.clone()
    }

    // Java: src/main/cpp/util/path_platform.h:67 Path::AsNativePath
    pub fn as_native_path(&self) -> String {
        self.path.clone()
    }

    /// The path as a `&str` (no copy).
    pub fn as_str(&self) -> &str {
        &self.path
    }
}
