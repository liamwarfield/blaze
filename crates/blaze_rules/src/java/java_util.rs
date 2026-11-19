//! `rules/java/JavaUtil.java`: utility methods for use by Java-related parts of the build
//! system.

use blaze_vfs::PathFragment;

/// Java source roots, used to relativize resource paths and infer main_class values.
// Java: com/google/devtools/build/lib/rules/java/JavaUtil.java:56 JavaUtil.KNOWN_SOURCE_ROOTS
pub const KNOWN_SOURCE_ROOTS: &[&str] = &["java", "javatests", "src", "testsrc"];

/// Finds the index of the segment in a Java path fragment that precedes the source root. Starts
/// from the first "java" or "javatests" or "src" or "testsrc" segment. If the found item was
/// "src", check if this is followed by "main" or "test" and then "java" or "resources" (maven
/// layout). If the found item was "src", or "java"/"javatests" at the first segment, check for a
/// nested root directory (src, java or javatests). A nested root must be followed by
/// (com|net|org), or matching maven structure for nested "src", to be accepted, to avoid false
/// positives.
///
/// Returns the index of the java segment or -1 iff no java segment was found.
// Java: com/google/devtools/build/lib/rules/java/JavaUtil.java:71 JavaUtil.javaSegmentIndex
fn java_segment_index(path: &PathFragment) -> i64 {
    if path.is_absolute() {
        panic!("IllegalArgumentException: path must not be absolute: '{path}'");
    }
    let segments = path.split_to_list_of_segments();
    let Some(mut root_index) = segments
        .iter()
        .position(|s| KNOWN_SOURCE_ROOTS.contains(&s.as_str()))
    else {
        return -1;
    };
    let is_src = segments[root_index] == "src";
    let mut check_maven_index: i64 = if is_src { root_index as i64 } else { -1 };
    if root_index == 0 || is_src {
        // Check for a nested root directory.
        let max = segments.len() as i64 - 2;
        let mut i = root_index as i64 + 1;
        while i <= max {
            let segment = &segments[i as usize];
            if segment == "src" || (is_src && (segment == "javatests" || segment == "java")) {
                let next = &segments[i as usize + 1];
                if next == "com" || next == "org" || next == "net" {
                    // Check for common first element of java package, to avoid false positives.
                    root_index = i as usize;
                } else if segment == "src" {
                    // Also accept maven style src/(main|test)/(java|resources).
                    check_maven_index = i;
                }
                break;
            }
            i += 1;
        }
    }
    // Check for (main|test)/(java|resources) after /src/.
    if check_maven_index >= 0 && check_maven_index + 2 < segments.len() as i64 {
        let m = check_maven_index as usize;
        let next = &segments[m + 1];
        if next == "main" || next == "test" {
            let next = &segments[m + 2];
            if next == "java" || next == "resources" {
                root_index = m + 2;
            }
        }
    }
    root_index as i64
}

/// Given the PathFragment of a Java source file, returns the Java root relative path or `None`
/// if no java root can be determined.
///
/// For example, "{workspace}/java/foo/bar/wiz" and "{workspace}/javatests/foo/bar/wiz" both
/// result in "foo/bar/wiz".
// Java: com/google/devtools/build/lib/rules/java/JavaUtil.java:122 JavaUtil.getJavaPath
pub fn get_java_path(path: &PathFragment) -> Option<PathFragment> {
    let index = java_segment_index(path);
    if index >= 0 {
        return Some(path.sub_fragment_from(index as usize + 1));
    }
    None
}

#[cfg(test)]
mod tests {
    //! Java: src/test/java/com/google/devtools/build/lib/rules/java/JavaUtilTest.java
    use super::*;

    // Java: JavaUtilTest.testGetJavaPath
    #[test]
    fn test_get_java_path() {
        assert_eq!(
            get_java_path(&PathFragment::create("java/com/google/foo/FooModule.java"))
                .unwrap()
                .get_path_string(),
            "com/google/foo/FooModule.java"
        );
        assert!(get_java_path(&PathFragment::create("org/foo/FooUtil.java")).is_none());
    }
}
