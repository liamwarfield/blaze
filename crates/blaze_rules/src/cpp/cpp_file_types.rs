//! The `CppFileTypes` matchers the C++ natives use (`util/FileType` semantics on Linux:
//! `hasExtension(path, ext)` is `path.endsWith(ext)`).

use blaze_actions::Artifact;
use blaze_vfs::PathFragment;

/// A `FileType`: a predicate over the path string (`filePathForFileTypeMatcher`).
// Java: com/google/devtools/build/lib/util/FileType.java:33 FileType
pub struct FileType {
    apply: fn(&str) -> bool,
}

impl FileType {
    // Java: com/google/devtools/build/lib/util/FileType.java:154 FileType.matches
    pub fn matches(&self, path: &str) -> bool {
        (self.apply)(path)
    }

    /// Java `fileType.matches(pathFragment)`.
    pub fn matches_path(&self, path: &PathFragment) -> bool {
        (self.apply)(path.get_path_string())
    }

    /// Java `artifact.isFileType(fileType)`.
    // Java: com/google/devtools/build/lib/actions/Artifact.java:416 Artifact.isFileType
    pub fn matches_artifact(&self, artifact: &Artifact) -> bool {
        (self.apply)(artifact.file_path_for_file_type_matcher())
    }
}

/// Java `FileType.hasExtension(path, ext)` (non-Windows).
// Java: com/google/devtools/build/lib/util/FileType.java:115 FileType.hasExtension
// PORT: skipped, Windows only (case-insensitive branch).
fn has_extension(path: &str, ext: &str) -> bool {
    path.ends_with(ext)
}

fn has_any_extension(path: &str, extensions: &[&str]) -> bool {
    extensions.iter().any(|e| has_extension(path, e))
}

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:31 CppFileTypes.CPP_SOURCE
pub static CPP_SOURCE: FileType = FileType {
    apply: |p| {
        [".cc", ".cpp", ".cxx", ".c++", ".C", ".cu", ".cl"]
            .iter()
            .any(|e| p.ends_with(e))
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:53 CppFileTypes.C_SOURCE
pub static C_SOURCE: FileType = FileType {
    apply: |p| p.ends_with(".c"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:68 CppFileTypes.OBJC_SOURCE
pub static OBJC_SOURCE: FileType = FileType {
    apply: |p| has_extension(p, ".m"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:69 CppFileTypes.OBJCPP_SOURCE
pub static OBJCPP_SOURCE: FileType = FileType {
    apply: |p| has_extension(p, ".mm"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:70 CppFileTypes.CLIF_INPUT_PROTO
pub static CLIF_INPUT_PROTO: FileType = FileType {
    apply: |p| has_extension(p, ".ipb"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:86 CppFileTypes.CPP_HEADER
pub static CPP_HEADER: FileType = FileType {
    apply: |p| {
        has_any_extension(
            p,
            &[
                ".h", ".hh", ".hpp", ".ipp", ".hxx", ".h++", ".inc", ".inl", ".tlh", ".tli", ".H",
                ".tcc",
            ],
        )
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:93 CppFileTypes.CPP_TEXTUAL_INCLUDE
pub static CPP_TEXTUAL_INCLUDE: FileType = FileType {
    apply: |p| has_extension(p, ".inc"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:127 CppFileTypes.ASSEMBLER_WITH_C_PREPROCESSOR
pub static ASSEMBLER_WITH_C_PREPROCESSOR: FileType = FileType {
    apply: |p| p.ends_with(".S"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:143 CppFileTypes.PIC_ASSEMBLER
pub static PIC_ASSEMBLER: FileType = FileType {
    apply: |p| has_extension(p, ".pic.s") && p.ends_with(".s"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:159 CppFileTypes.ASSEMBLER
pub static ASSEMBLER: FileType = FileType {
    apply: |p| (p.ends_with(".s") && !PIC_ASSEMBLER.matches(p)) || has_extension(p, ".asm"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:279 CppFileTypes.CPP_MODULE_MAP
pub static CPP_MODULE_MAP: FileType = FileType {
    apply: |p| has_extension(p, ".cppmap"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:280 CppFileTypes.CPP_MODULE
pub static CPP_MODULE: FileType = FileType {
    apply: |p| has_any_extension(p, &[".pcm", ".gcm", ".ifc"]),
};

/// Java `CppFileTypes.headerDiscoveryRequired(source)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:299 CppFileTypes.headerDiscoveryRequired
pub fn header_discovery_required(source: &Artifact) -> bool {
    let file_name = source.get_filename();
    !ASSEMBLER.matches(file_name)
        && !PIC_ASSEMBLER.matches(file_name)
        && !CPP_MODULE.matches(file_name)
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/rules/cpp/CppFileTypesTest.java
    use super::*;

    #[test]
    #[ignore = "needs CppFileTypes.OBJECT_FILE / PIC_OBJECT_FILE (not used by the compile natives)"]
    fn test_two_dot_extensions() {}

    #[test]
    #[ignore = "needs CppFileTypes.RUST_RLIB (not used by the compile natives)"]
    fn test_rlib() {}

    #[test]
    #[ignore = "needs CppFileTypes.SHARED_LIBRARY / VERSIONED_SHARED_LIBRARY (ported in blaze_actions::rules::cpp::solib_symlink_action)"]
    fn test_versioned_shared_libraries() {}

    #[test]
    fn test_case_sensitive_assembly_files() {
        assert!(ASSEMBLER_WITH_C_PREPROCESSOR.matches("foo.S"));
        assert!(!ASSEMBLER_WITH_C_PREPROCESSOR.matches("foo.s"));
        assert!(PIC_ASSEMBLER.matches("foo.pic.s"));
        assert!(!PIC_ASSEMBLER.matches("foo.pic.S"));
        assert!(ASSEMBLER.matches("foo.s"));
        assert!(ASSEMBLER.matches("foo.asm"));
        assert!(!ASSEMBLER.matches("foo.pic.s"));
        assert!(!ASSEMBLER.matches("foo.S"));
    }

    #[test]
    #[ignore = "needs Link.SHARED_LIBRARY_FILETYPES and friends (blaze_cc_features::link is unported)"]
    fn test_no_extension_libraries() {}

    #[test]
    fn test_case_sensitive_c_files() {
        assert!(C_SOURCE.matches("foo.c"));
        assert!(!CPP_SOURCE.matches("foo.c"));
        assert!(!C_SOURCE.matches("foo.C"));
        assert!(CPP_SOURCE.matches("foo.C"));
    }

    #[test]
    fn source_and_header_types() {
        assert!(CPP_SOURCE.matches("re2/re2.cc"));
        assert!(C_SOURCE.matches("c/common/constants.c"));
        assert!(CPP_HEADER.matches("re2/re2.h"));
        assert!(CPP_TEXTUAL_INCLUDE.matches("x.inc"));
        assert!(CPP_MODULE_MAP.matches("bazel-out/k8-fastbuild/bin/re2.cppmap"));
        assert!(CPP_MODULE.matches("a.pcm"));
    }
}
