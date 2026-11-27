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
    // Java: com/google/devtools/build/lib/actions/Artifact.java:421 Artifact.isFileType
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

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:272 CppFileTypes.COVERAGE_NOTES
pub static COVERAGE_NOTES: FileType = FileType {
    apply: |p| has_extension(p, ".gcno"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:281 CppFileTypes.OBJC_MODULE_MAP
pub static OBJC_MODULE_MAP: FileType = FileType {
    apply: |p| has_extension(p, "module.modulemap"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:71 CppFileTypes.CLIF_OUTPUT_PROTO
pub static CLIF_OUTPUT_PROTO: FileType = FileType {
    apply: |p| has_extension(p, ".opb"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:72 CppFileTypes.BC_SOURCE
pub static BC_SOURCE: FileType = FileType {
    apply: |p| has_extension(p, ".bc"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:90 CppFileTypes.PCH
pub static PCH: FileType = FileType {
    apply: |p| has_extension(p, ".pch"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:95 CppFileTypes.PIC_PREPROCESSED_C
pub static PIC_PREPROCESSED_C: FileType = FileType {
    apply: |p| has_extension(p, ".pic.i"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:96 CppFileTypes.PREPROCESSED_C
pub static PREPROCESSED_C: FileType = FileType {
    apply: |p| has_extension(p, ".i") && !PIC_PREPROCESSED_C.matches(p),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:110 CppFileTypes.PIC_PREPROCESSED_CPP
pub static PIC_PREPROCESSED_CPP: FileType = FileType {
    apply: |p| has_extension(p, ".pic.ii"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:111 CppFileTypes.PREPROCESSED_CPP
pub static PREPROCESSED_CPP: FileType = FileType {
    apply: |p| has_extension(p, ".ii") && !PIC_PREPROCESSED_CPP.matches(p),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:174 CppFileTypes.PIC_ARCHIVE
pub static PIC_ARCHIVE: FileType = FileType {
    apply: |p| has_extension(p, ".pic.a"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:175 CppFileTypes.ARCHIVE
pub static ARCHIVE: FileType = FileType {
    apply: |p| {
        if PIC_ARCHIVE.matches(p) || ALWAYS_LINK_LIBRARY.matches(p) || has_extension(p, ".if.lib") {
            return false;
        }
        has_any_extension(p, &[".a", ".lib"])
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:195 CppFileTypes.ALWAYS_LINK_PIC_LIBRARY
pub static ALWAYS_LINK_PIC_LIBRARY: FileType = FileType {
    apply: |p| has_extension(p, ".pic.lo"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:196 CppFileTypes.ALWAYS_LINK_LIBRARY
pub static ALWAYS_LINK_LIBRARY: FileType = FileType {
    apply: |p| {
        (has_extension(p, ".lo") && !ALWAYS_LINK_PIC_LIBRARY.matches(p))
            || has_extension(p, ".lo.lib")
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:212 CppFileTypes.PIC_OBJECT_FILE
pub static PIC_OBJECT_FILE: FileType = FileType {
    apply: |p| has_extension(p, ".pic.o"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:213 CppFileTypes.OBJECT_FILE
pub static OBJECT_FILE: FileType = FileType {
    apply: |p| (has_extension(p, ".o") && !PIC_OBJECT_FILE.matches(p)) || has_extension(p, ".obj"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:229 CppFileTypes.RUST_RLIB
pub static RUST_RLIB: FileType = FileType {
    apply: |p| has_extension(p, ".rlib"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:232 CppFileTypes.LTO_INDEXING_OBJECT_FILE
pub static LTO_INDEXING_OBJECT_FILE: FileType = FileType {
    apply: |p| has_extension(p, ".indexing.o"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:235 CppFileTypes.LTO_IMPORTS_FILE
pub static LTO_IMPORTS_FILE: FileType = FileType {
    apply: |p| has_extension(p, ".imports"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:239 CppFileTypes.LTO_INDEXING_ANALYSIS_FILE
pub static LTO_INDEXING_ANALYSIS_FILE: FileType = FileType {
    apply: |p| has_extension(p, ".thinlto.bc"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:243 CppFileTypes.SHARED_LIBRARY
pub static SHARED_LIBRARY: FileType = FileType {
    apply: |p| {
        has_any_extension(
            p,
            &[".so", ".dylib", ".dll", ".pyd", ".wasm", ".tgt", ".vpi"],
        )
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:246 CppFileTypes.UNIX_SHARED_LIBRARY
pub static UNIX_SHARED_LIBRARY: FileType = FileType {
    apply: |p| has_any_extension(p, &[".so", ".dylib"]),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:247 CppFileTypes.INTERFACE_SHARED_LIBRARY
pub static INTERFACE_SHARED_LIBRARY: FileType = FileType {
    apply: |p| has_any_extension(p, &[".ifso", ".tbd", ".lib", ".dll.a"]),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:249 CppFileTypes.LINKER_SCRIPT
pub static LINKER_SCRIPT: FileType = FileType {
    apply: |p| has_any_extension(p, &[".ld", ".lds", ".ldscript"]),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:252 CppFileTypes.WINDOWS_DEF_FILE
pub static WINDOWS_DEF_FILE: FileType = FileType {
    apply: |p| has_extension(p, ".def"),
};

// Matches shared libraries with version names in the extension, i.e.
// libmylib.so.2 or libmylib.so.2.10 or libmylib.so.1a_b35.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:256 CppFileTypes.VERSIONED_SHARED_LIBRARY_PATTERN
static VERSIONED_SHARED_LIBRARY_PATTERN: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        // Java's \d and \w are ASCII-only without UNICODE_CHARACTER_CLASS; `.` excludes line
        // terminators, as the regex crate's `.` excludes `\n`.
        regex::Regex::new(r"^.+\.((so)|(dylib))(\.[0-9][0-9A-Za-z_]*)+$").expect("a valid pattern")
    });

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:258 CppFileTypes.VERSIONED_SHARED_LIBRARY
pub static VERSIONED_SHARED_LIBRARY: FileType = FileType {
    apply: |p| {
        // Because regex matching can be slow, we first do a quick check for ".so." and ".dylib."
        // substring before risking the full-on regex match. This should eliminate the performance
        // hit on practically every non-qualifying file type.
        if !p.contains(".so.") && !p.contains(".dylib.") {
            return false;
        }
        VERSIONED_SHARED_LIBRARY_PATTERN.is_match(p)
    },
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:273 CppFileTypes.GCC_AUTO_PROFILE
pub static GCC_AUTO_PROFILE: FileType = FileType {
    apply: |p| has_extension(p, ".afdo"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:274 CppFileTypes.XBINARY_PROFILE
pub static XBINARY_PROFILE: FileType = FileType {
    apply: |p| has_extension(p, ".xfdo"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:275 CppFileTypes.LLVM_PROFILE
pub static LLVM_PROFILE: FileType = FileType {
    apply: |p| has_extension(p, ".profdata"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:276 CppFileTypes.LLVM_PROFILE_RAW
pub static LLVM_PROFILE_RAW: FileType = FileType {
    apply: |p| has_extension(p, ".profraw"),
};

// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:277 CppFileTypes.LLVM_PROFILE_ZIP
pub static LLVM_PROFILE_ZIP: FileType = FileType {
    apply: |p| has_extension(p, ".zip"),
};

/// Java `FileType.NO_EXTENSION`: no `.` after the last `/`.
// Java: com/google/devtools/build/lib/util/FileType.java:34 FileType.NO_EXTENSION
pub static NO_EXTENSION: FileType = FileType {
    apply: |p| {
        let last_slash_index = p.rfind('/').map_or(0, |i| i + 1);
        !p[last_slash_index..].contains('.')
    },
};

/// A `FileTypeSet`: matches when any of its types matches.
// Java: com/google/devtools/build/lib/util/FileTypeSet.java:29 FileTypeSet
pub struct FileTypeSet(&'static [&'static FileType]);

impl FileTypeSet {
    // Java: com/google/devtools/build/lib/util/FileTypeSet.java:111 FileTypeSet.matches
    pub fn matches(&self, path: &str) -> bool {
        self.0.iter().any(|t| t.matches(path))
    }
}

/// These file are supposed to be added using `addLibrary()` calls to `CppLinkAction` but will
/// never be expanded to their constituent `.o` files.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:39 Link.SHARED_LIBRARY_FILETYPES
pub static SHARED_LIBRARY_FILETYPES: FileTypeSet = FileTypeSet(&[
    &SHARED_LIBRARY,
    &VERSIONED_SHARED_LIBRARY,
    &INTERFACE_SHARED_LIBRARY,
    &NO_EXTENSION,
]);

// Java: com/google/devtools/build/lib/rules/cpp/Link.java:46 Link.ONLY_SHARED_LIBRARY_FILETYPES
pub static ONLY_SHARED_LIBRARY_FILETYPES: FileTypeSet =
    FileTypeSet(&[&SHARED_LIBRARY, &VERSIONED_SHARED_LIBRARY, &NO_EXTENSION]);

// Java: com/google/devtools/build/lib/rules/cpp/Link.java:52 Link.ONLY_INTERFACE_LIBRARY_FILETYPES
pub static ONLY_INTERFACE_LIBRARY_FILETYPES: FileTypeSet =
    FileTypeSet(&[&INTERFACE_SHARED_LIBRARY]);

// Java: com/google/devtools/build/lib/rules/cpp/Link.java:55 Link.ARCHIVE_LIBRARY_FILETYPES
pub static ARCHIVE_LIBRARY_FILETYPES: FileTypeSet = FileTypeSet(&[
    &ARCHIVE,
    &PIC_ARCHIVE,
    &ALWAYS_LINK_LIBRARY,
    &ALWAYS_LINK_PIC_LIBRARY,
    &RUST_RLIB,
    &NO_EXTENSION,
]);

// Java: com/google/devtools/build/lib/rules/cpp/Link.java:64 Link.ARCHIVE_FILETYPES
pub static ARCHIVE_FILETYPES: FileTypeSet =
    FileTypeSet(&[&ARCHIVE, &PIC_ARCHIVE, &RUST_RLIB, &NO_EXTENSION]);

/// The set of object files.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:72 Link.OBJECT_FILETYPES
pub static OBJECT_FILETYPES: FileTypeSet = FileTypeSet(&[
    &OBJECT_FILE,
    &PIC_OBJECT_FILE,
    &CLIF_OUTPUT_PROTO,
    &BC_SOURCE,
]);

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

    // Java: src/test/java/com/google/devtools/build/lib/rules/cpp/CppFileTypesTest.java:74 CppFileTypesTest.testNoExtensionLibraries
    #[test]
    fn test_no_extension_libraries() {
        assert!(SHARED_LIBRARY_FILETYPES.matches("someframework"));
        assert!(ONLY_SHARED_LIBRARY_FILETYPES.matches("someframework"));
        assert!(ARCHIVE_LIBRARY_FILETYPES.matches("someframework"));
        assert!(ARCHIVE_FILETYPES.matches("someframework"));
    }

    /// Not a Java test: the versioned shared library pattern and the archive exclusions.
    #[test]
    fn versioned_shared_libraries_and_archives() {
        assert!(VERSIONED_SHARED_LIBRARY.matches("libmylib.so.2"));
        assert!(VERSIONED_SHARED_LIBRARY.matches("libmylib.so.2.10"));
        assert!(VERSIONED_SHARED_LIBRARY.matches("libmylib.so.1a_b35"));
        assert!(!VERSIONED_SHARED_LIBRARY.matches("libmylib.so"));
        assert!(!VERSIONED_SHARED_LIBRARY.matches("libmylib.so.a"));
        assert!(ARCHIVE.matches("libx.a"));
        assert!(!ARCHIVE.matches("libx.pic.a"));
        assert!(!ARCHIVE.matches("libx.lo.lib"));
        assert!(OBJECT_FILE.matches("x.o"));
        assert!(!OBJECT_FILE.matches("x.pic.o"));
        assert!(!SHARED_LIBRARY_FILETYPES.matches("x.a"));
    }

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
