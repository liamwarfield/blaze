//! Port of `com.google.devtools.build.lib.rules.cpp.Link`: the link-related enums.
//!
//! PORT: the `FileTypeSet` constants (`SHARED_LIBRARY_FILETYPES`, `ARCHIVE_FILETYPES`, ...) need
//! `CppFileTypes`/`FileTypeSet`, which are not ported yet; their accessors fail loudly.

use crate::artifact_category::ArtifactCategory;
use crate::cpp_action_names::CppActionNames;

/// Java: `Link.SHARED_LIBRARY_FILETYPES`.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:39 Link.SHARED_LIBRARY_FILETYPES
pub fn shared_library_filetypes() -> ! {
    blaze_util::unported!("Link.SHARED_LIBRARY_FILETYPES (needs CppFileTypes)")
}

/// Java: `Link.ARCHIVE_LIBRARY_FILETYPES`.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:55 Link.ARCHIVE_LIBRARY_FILETYPES
pub fn archive_library_filetypes() -> ! {
    blaze_util::unported!("Link.ARCHIVE_LIBRARY_FILETYPES (needs CppFileTypes)")
}

/// Java: `Link.OBJECT_FILETYPES`.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:72 Link.OBJECT_FILETYPES
pub fn object_filetypes() -> ! {
    blaze_util::unported!("Link.OBJECT_FILETYPES (needs CppFileTypes)")
}

/// Whether a particular link target requires PIC code.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:82 Link.Picness
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Picness {
    PIC,
    NOPIC,
}

/// Whether a particular link target linked in statically or dynamically.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:88 Link.LinkerOrArchiver
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkerOrArchiver {
    ARCHIVER,
    LINKER,
}

/// Whether a particular link target is executable.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:96 Link.Executable
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Executable {
    EXECUTABLE,
    NOT_EXECUTABLE,
}

/// Types of ELF files that can be created by the linker (.a, .so, .lo, executable).
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:105 Link.LinkTargetType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkTargetType {
    /// A normal static archive.
    STATIC_LIBRARY,
    /// An objc fully linked static archive.
    OBJC_FULLY_LINKED_ARCHIVE,
    /// An objc executable.
    OBJC_EXECUTABLE,
    /// A static archive with .pic.o object files (compiled with -fPIC).
    PIC_STATIC_LIBRARY,
    /// An interface dynamic library.
    INTERFACE_DYNAMIC_LIBRARY,
    /// A dynamic library built from cc_library srcs.
    NODEPS_DYNAMIC_LIBRARY,
    /// A transitive dynamic library used for distribution.
    DYNAMIC_LIBRARY,
    /// A static archive without removal of unused object files.
    ALWAYS_LINK_STATIC_LIBRARY,
    /// A PIC static archive without removal of unused object files.
    ALWAYS_LINK_PIC_STATIC_LIBRARY,
    /// An executable binary.
    EXECUTABLE,
}

impl LinkTargetType {
    /// Java: `LinkTargetType.values()`.
    pub const VALUES: [LinkTargetType; 10] = [
        LinkTargetType::STATIC_LIBRARY,
        LinkTargetType::OBJC_FULLY_LINKED_ARCHIVE,
        LinkTargetType::OBJC_EXECUTABLE,
        LinkTargetType::PIC_STATIC_LIBRARY,
        LinkTargetType::INTERFACE_DYNAMIC_LIBRARY,
        LinkTargetType::NODEPS_DYNAMIC_LIBRARY,
        LinkTargetType::DYNAMIC_LIBRARY,
        LinkTargetType::ALWAYS_LINK_STATIC_LIBRARY,
        LinkTargetType::ALWAYS_LINK_PIC_STATIC_LIBRARY,
        LinkTargetType::EXECUTABLE,
    ];

    // (linkerOrArchiver, actionName, picness, linkerOutput, executable), as in the Java constants.
    fn spec(
        self,
    ) -> (
        LinkerOrArchiver,
        &'static str,
        Picness,
        ArtifactCategory,
        Executable,
    ) {
        use LinkerOrArchiver::*;
        match self {
            LinkTargetType::STATIC_LIBRARY => (
                ARCHIVER,
                CppActionNames::CPP_LINK_STATIC_LIBRARY,
                Picness::NOPIC,
                ArtifactCategory::STATIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::OBJC_FULLY_LINKED_ARCHIVE => (
                ARCHIVER,
                CppActionNames::OBJC_FULLY_LINK,
                Picness::NOPIC,
                ArtifactCategory::STATIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::OBJC_EXECUTABLE => (
                LINKER,
                CppActionNames::OBJC_EXECUTABLE,
                Picness::NOPIC,
                ArtifactCategory::EXECUTABLE,
                Executable::EXECUTABLE,
            ),
            LinkTargetType::PIC_STATIC_LIBRARY => (
                ARCHIVER,
                CppActionNames::CPP_LINK_STATIC_LIBRARY,
                Picness::PIC,
                ArtifactCategory::STATIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::INTERFACE_DYNAMIC_LIBRARY => (
                LINKER,
                CppActionNames::CPP_LINK_DYNAMIC_LIBRARY,
                Picness::NOPIC, // Actually PIC but it's not indicated in the file name
                ArtifactCategory::INTERFACE_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::NODEPS_DYNAMIC_LIBRARY => (
                LINKER,
                CppActionNames::CPP_LINK_NODEPS_DYNAMIC_LIBRARY,
                Picness::NOPIC, // Actually PIC but it's not indicated in the file name
                ArtifactCategory::DYNAMIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::DYNAMIC_LIBRARY => (
                LINKER,
                CppActionNames::CPP_LINK_DYNAMIC_LIBRARY,
                Picness::NOPIC, // Actually PIC but it's not indicated in the file name
                ArtifactCategory::DYNAMIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::ALWAYS_LINK_STATIC_LIBRARY => (
                ARCHIVER,
                CppActionNames::CPP_LINK_STATIC_LIBRARY,
                Picness::NOPIC,
                ArtifactCategory::ALWAYSLINK_STATIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::ALWAYS_LINK_PIC_STATIC_LIBRARY => (
                ARCHIVER,
                CppActionNames::CPP_LINK_STATIC_LIBRARY,
                Picness::PIC,
                ArtifactCategory::ALWAYSLINK_STATIC_LIBRARY,
                Executable::NOT_EXECUTABLE,
            ),
            LinkTargetType::EXECUTABLE => (
                LINKER,
                CppActionNames::CPP_LINK_EXECUTABLE,
                Picness::NOPIC, // Picness is not indicate in the file name
                ArtifactCategory::EXECUTABLE,
                Executable::EXECUTABLE,
            ),
        }
    }

    /// Returns whether the name of the output file should denote that the code in the file is PIC.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:209 Link.LinkTargetType.picness
    pub fn picness(self) -> Picness {
        self.spec().2
    }

    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:213 Link.LinkTargetType.getPicExtensionWhenApplicable
    pub fn get_pic_extension_when_applicable(self) -> &'static str {
        if self.picness() == Picness::PIC {
            ".pic"
        } else {
            ""
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:217 Link.LinkTargetType.getDefaultExtension
    pub fn get_default_extension(self) -> &'static str {
        self.spec().3.get_default_extension()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:221 Link.LinkTargetType.linkerOrArchiver
    pub fn linker_or_archiver(self) -> LinkerOrArchiver {
        self.spec().0
    }

    /// Returns an `ArtifactCategory` identifying the artifact type this link action emits.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:226 Link.LinkTargetType.getLinkerOutput
    pub fn get_linker_output(self) -> ArtifactCategory {
        self.spec().3
    }

    /// The name of a link action with this LinkTargetType, for the purpose of crosstool feature
    /// selection.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:234 Link.LinkTargetType.getActionName
    pub fn get_action_name(self) -> &'static str {
        self.spec().1
    }

    /// Returns true iff this link type is executable.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:239 Link.LinkTargetType.isExecutable
    pub fn is_executable(self) -> bool {
        self.spec().4 == Executable::EXECUTABLE
    }

    /// Returns true iff this link type is a transitive dynamic library.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:244 Link.LinkTargetType.isTransitiveDynamicLibrary
    pub fn is_transitive_dynamic_library(self) -> bool {
        self == LinkTargetType::DYNAMIC_LIBRARY
    }

    /// Returns true iff this link type is a dynamic library or transitive dynamic library.
    // Java: com/google/devtools/build/lib/rules/cpp/Link.java:249 Link.LinkTargetType.isDynamicLibrary
    pub fn is_dynamic_library(self) -> bool {
        self == LinkTargetType::NODEPS_DYNAMIC_LIBRARY || self == LinkTargetType::DYNAMIC_LIBRARY
    }
}

/// The degree of "staticness" of symbol resolution during linking.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:255 Link.LinkingMode
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkingMode {
    /// Link binaries statically except for system libraries (e.g. `gcc x.o libfoo.a libbar.a
    /// -lm`).
    STATIC,
    /// All libraries are linked dynamically (if a dynamic version is available), e.g. `gcc x.o
    /// libfoo.so libbar.so -lm`.
    DYNAMIC,
}

/// How to pass archives to the linker on the command line.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:271 Link.ArchiveType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveType {
    /// Put the archive itself on the linker command line.
    REGULAR,
    /// Put the object files enclosed by --start-lib / --end-lib on the command line.
    START_END_LIB,
}
