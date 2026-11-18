//! The parts of `java.io.File` (Unix flavour) that `lib/shell` and `lib/util` rely on: path
//! normalization, `getParent`, `getName`, `isAbsolute`, `getAbsolutePath` and the
//! `File(File, String)` constructor.
// PORT: move to blaze_util (java.io.File emulation shared by ports that use File).

use std::fmt;

/// Port of `java.io.File`: an abstract pathname, normalized like `java.io.UnixFileSystem`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JavaFile {
    path: String,
}

// Java: java/io/UnixFileSystem.java normalize (JDK 21)
fn normalize(pathname: &str) -> String {
    if !pathname.contains("//") && !pathname.ends_with('/') {
        return pathname.to_string();
    }
    let bytes = pathname.as_bytes();
    let mut n = bytes.len();
    while n > 0 && bytes[n - 1] == b'/' {
        n -= 1;
    }
    if n == 0 {
        return if bytes.is_empty() {
            String::new()
        } else {
            "/".to_string()
        };
    }
    let mut out = String::with_capacity(n);
    let mut prev_slash = false;
    for c in pathname[..n].chars() {
        if c == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        out.push(c);
    }
    out
}

// Java: java/io/UnixFileSystem.java resolve(String, String) (JDK 21)
fn resolve(parent: &str, child: &str) -> String {
    if child.is_empty() {
        return parent.to_string();
    }
    if child.starts_with('/') {
        if parent == "/" {
            return child.to_string();
        }
        return format!("{parent}{child}");
    }
    if parent == "/" {
        return format!("{parent}{child}");
    }
    format!("{parent}/{child}")
}

impl JavaFile {
    /// `new File(String pathname)`.
    pub fn new(pathname: &str) -> JavaFile {
        JavaFile {
            path: normalize(pathname),
        }
    }

    /// `new File(File parent, String child)`; `parent` may be null.
    pub fn new_child(parent: Option<&JavaFile>, child: &str) -> JavaFile {
        match parent {
            Some(parent) => {
                if parent.path.is_empty() {
                    // Java: defaultParent is "/" on Unix.
                    JavaFile {
                        path: resolve("/", &normalize(child)),
                    }
                } else {
                    JavaFile {
                        path: resolve(&parent.path, &normalize(child)),
                    }
                }
            }
            None => JavaFile::new(child),
        }
    }

    /// `File.getPath()`.
    pub fn get_path(&self) -> &str {
        &self.path
    }

    fn prefix_length(&self) -> usize {
        if self.path.starts_with('/') { 1 } else { 0 }
    }

    /// `File.isAbsolute()`.
    pub fn is_absolute(&self) -> bool {
        self.prefix_length() != 0
    }

    /// `File.getParent()`.
    pub fn get_parent(&self) -> Option<String> {
        let prefix_length = self.prefix_length();
        match self.path.rfind('/') {
            Some(index) if index >= prefix_length => Some(self.path[..index].to_string()),
            _ => {
                if prefix_length > 0 && self.path.len() > prefix_length {
                    Some(self.path[..prefix_length].to_string())
                } else {
                    None
                }
            }
        }
    }

    /// `File.getName()`.
    pub fn get_name(&self) -> &str {
        let prefix_length = self.prefix_length();
        match self.path.rfind('/') {
            Some(index) if index >= prefix_length => &self.path[index + 1..],
            _ => &self.path[prefix_length..],
        }
    }

    /// `File.getAbsolutePath()`: resolves a relative path against `user.dir` (the process's
    /// working directory).
    pub fn get_absolute_path(&self) -> String {
        if self.is_absolute() {
            return self.path.clone();
        }
        // PORT: `user.dir` is the JVM's startup working directory; Blaze never changes its working
        // directory, so the current one is the same.
        let user_dir = std::env::current_dir()
            .expect("user.dir")
            .to_string_lossy()
            .into_owned();
        resolve(&normalize(&user_dir), &self.path)
    }

    /// `File.exists()`.
    pub fn exists(&self) -> bool {
        std::fs::metadata(&self.path).is_ok()
    }

    /// `File.delete()`.
    pub fn delete(&self) -> bool {
        match std::fs::symlink_metadata(&self.path) {
            Ok(m) if m.is_dir() => std::fs::remove_dir(&self.path).is_ok(),
            Ok(_) => std::fs::remove_file(&self.path).is_ok(),
            Err(_) => false,
        }
    }
}

impl fmt::Display for JavaFile {
    /// `File.toString()` is `getPath()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_like_java() {
        assert_eq!(JavaFile::new("a//b/").get_path(), "a/b");
        assert_eq!(JavaFile::new("///").get_path(), "/");
        assert_eq!(JavaFile::new("").get_path(), "");
        assert_eq!(JavaFile::new("/foo").get_parent().as_deref(), Some("/"));
        assert_eq!(JavaFile::new("/").get_parent(), None);
        assert_eq!(JavaFile::new("foo").get_parent(), None);
        assert_eq!(JavaFile::new("a/b").get_parent().as_deref(), Some("a"));
        assert_eq!(JavaFile::new("/bin/sh").get_name(), "sh");
        assert_eq!(JavaFile::new("sh").get_name(), "sh");
        assert_eq!(
            JavaFile::new_child(Some(&JavaFile::new("/w/d")), "rel/bin").get_path(),
            "/w/d/rel/bin"
        );
    }
}
