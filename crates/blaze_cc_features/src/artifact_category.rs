//! Port of `com.google.devtools.build.lib.rules.cpp.ArtifactCategory`.

use std::fmt;

/// A category of artifacts that are candidate input/output to an action, for which the toolchain
/// can select a single artifact.
// Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:23 ArtifactCategory
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtifactCategory {
    STATIC_LIBRARY,
    ALWAYSLINK_STATIC_LIBRARY,
    DYNAMIC_LIBRARY,
    EXECUTABLE,
    INTERFACE_LIBRARY,
    PIC_FILE,
    INCLUDED_FILE_LIST,
    SERIALIZED_DIAGNOSTICS_FILE,
    OBJECT_FILE,
    PIC_OBJECT_FILE,
    CPP_MODULE,
    CPP_MODULES_INFO,
    CPP_MODULES_DDI,
    CPP_MODULES_MODMAP,
    CPP_MODULES_MODMAP_INPUT,
    GENERATED_ASSEMBLY,
    PROCESSED_HEADER,
    GENERATED_HEADER,
    PREPROCESSED_C_SOURCE,
    PREPROCESSED_CPP_SOURCE,
    COVERAGE_DATA_FILE,
    // A matched-clif protobuf. Typically in binary format, but could be text depending on
    // the options passed to the clif_matcher.
    CLIF_OUTPUT_PROTO,
}

impl ArtifactCategory {
    /// Java: `ArtifactCategory.values()`, in declaration order.
    pub const VALUES: [ArtifactCategory; 22] = [
        ArtifactCategory::STATIC_LIBRARY,
        ArtifactCategory::ALWAYSLINK_STATIC_LIBRARY,
        ArtifactCategory::DYNAMIC_LIBRARY,
        ArtifactCategory::EXECUTABLE,
        ArtifactCategory::INTERFACE_LIBRARY,
        ArtifactCategory::PIC_FILE,
        ArtifactCategory::INCLUDED_FILE_LIST,
        ArtifactCategory::SERIALIZED_DIAGNOSTICS_FILE,
        ArtifactCategory::OBJECT_FILE,
        ArtifactCategory::PIC_OBJECT_FILE,
        ArtifactCategory::CPP_MODULE,
        ArtifactCategory::CPP_MODULES_INFO,
        ArtifactCategory::CPP_MODULES_DDI,
        ArtifactCategory::CPP_MODULES_MODMAP,
        ArtifactCategory::CPP_MODULES_MODMAP_INPUT,
        ArtifactCategory::GENERATED_ASSEMBLY,
        ArtifactCategory::PROCESSED_HEADER,
        ArtifactCategory::GENERATED_HEADER,
        ArtifactCategory::PREPROCESSED_C_SOURCE,
        ArtifactCategory::PREPROCESSED_CPP_SOURCE,
        ArtifactCategory::COVERAGE_DATA_FILE,
        ArtifactCategory::CLIF_OUTPUT_PROTO,
    ];

    /// Java: `name()` / `toString()`.
    pub fn name(self) -> &'static str {
        match self {
            ArtifactCategory::STATIC_LIBRARY => "STATIC_LIBRARY",
            ArtifactCategory::ALWAYSLINK_STATIC_LIBRARY => "ALWAYSLINK_STATIC_LIBRARY",
            ArtifactCategory::DYNAMIC_LIBRARY => "DYNAMIC_LIBRARY",
            ArtifactCategory::EXECUTABLE => "EXECUTABLE",
            ArtifactCategory::INTERFACE_LIBRARY => "INTERFACE_LIBRARY",
            ArtifactCategory::PIC_FILE => "PIC_FILE",
            ArtifactCategory::INCLUDED_FILE_LIST => "INCLUDED_FILE_LIST",
            ArtifactCategory::SERIALIZED_DIAGNOSTICS_FILE => "SERIALIZED_DIAGNOSTICS_FILE",
            ArtifactCategory::OBJECT_FILE => "OBJECT_FILE",
            ArtifactCategory::PIC_OBJECT_FILE => "PIC_OBJECT_FILE",
            ArtifactCategory::CPP_MODULE => "CPP_MODULE",
            ArtifactCategory::CPP_MODULES_INFO => "CPP_MODULES_INFO",
            ArtifactCategory::CPP_MODULES_DDI => "CPP_MODULES_DDI",
            ArtifactCategory::CPP_MODULES_MODMAP => "CPP_MODULES_MODMAP",
            ArtifactCategory::CPP_MODULES_MODMAP_INPUT => "CPP_MODULES_MODMAP_INPUT",
            ArtifactCategory::GENERATED_ASSEMBLY => "GENERATED_ASSEMBLY",
            ArtifactCategory::PROCESSED_HEADER => "PROCESSED_HEADER",
            ArtifactCategory::GENERATED_HEADER => "GENERATED_HEADER",
            ArtifactCategory::PREPROCESSED_C_SOURCE => "PREPROCESSED_C_SOURCE",
            ArtifactCategory::PREPROCESSED_CPP_SOURCE => "PREPROCESSED_CPP_SOURCE",
            ArtifactCategory::COVERAGE_DATA_FILE => "COVERAGE_DATA_FILE",
            ArtifactCategory::CLIF_OUTPUT_PROTO => "CLIF_OUTPUT_PROTO",
        }
    }

    /// Java: `ArtifactCategory.valueOf(name)`; `None` where Java throws
    /// `IllegalArgumentException("No enum constant ...")`.
    pub fn value_of(name: &str) -> Option<ArtifactCategory> {
        ArtifactCategory::VALUES
            .iter()
            .copied()
            .find(|c| c.name() == name)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:56 ArtifactCategory.<init>
    fn spec(self) -> (&'static str, &'static [&'static str]) {
        // (defaultPrefix, [defaultExtension, extraAllowedExtensions...])
        match self {
            ArtifactCategory::STATIC_LIBRARY => ("lib", &[".a", ".lib"]),
            ArtifactCategory::ALWAYSLINK_STATIC_LIBRARY => ("lib", &[".lo", ".lo.lib"]),
            ArtifactCategory::DYNAMIC_LIBRARY => {
                ("lib", &[".so", ".dylib", ".dll", ".pyd", ".wasm"])
            }
            ArtifactCategory::EXECUTABLE => ("", &["", ".exe", ".wasm"]),
            ArtifactCategory::INTERFACE_LIBRARY => ("lib", &[".ifso", ".tbd", ".if.lib", ".lib"]),
            ArtifactCategory::PIC_FILE => ("", &[".pic"]),
            ArtifactCategory::INCLUDED_FILE_LIST => ("", &[".d"]),
            ArtifactCategory::SERIALIZED_DIAGNOSTICS_FILE => ("", &[".dia"]),
            ArtifactCategory::OBJECT_FILE => ("", &[".o", ".obj"]),
            ArtifactCategory::PIC_OBJECT_FILE => ("", &[".pic.o"]),
            ArtifactCategory::CPP_MODULE => ("", &[".pcm", ".gcm", ".ifc"]),
            ArtifactCategory::CPP_MODULES_INFO => ("", &[".CXXModules.json"]),
            ArtifactCategory::CPP_MODULES_DDI => ("", &[".ddi"]),
            ArtifactCategory::CPP_MODULES_MODMAP => ("", &[".modmap"]),
            ArtifactCategory::CPP_MODULES_MODMAP_INPUT => ("", &[".modmap.input"]),
            ArtifactCategory::GENERATED_ASSEMBLY => ("", &[".s", ".asm"]),
            ArtifactCategory::PROCESSED_HEADER => ("", &[".processed"]),
            ArtifactCategory::GENERATED_HEADER => ("", &[".h"]),
            ArtifactCategory::PREPROCESSED_C_SOURCE => ("", &[".i"]),
            ArtifactCategory::PREPROCESSED_CPP_SOURCE => ("", &[".ii"]),
            ArtifactCategory::COVERAGE_DATA_FILE => ("", &[".gcno"]),
            ArtifactCategory::CLIF_OUTPUT_PROTO => ("", &[".opb"]),
        }
    }

    /// Returns the name of the category.
    // Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:70 ArtifactCategory.getCategoryName
    pub fn get_category_name(self) -> String {
        self.name().to_lowercase()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:74 ArtifactCategory.getDefaultPrefix
    pub fn get_default_prefix(self) -> &'static str {
        self.spec().0
    }

    // Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:78 ArtifactCategory.getDefaultExtension
    pub fn get_default_extension(self) -> &'static str {
        self.spec().1[0]
    }

    // Java: com/google/devtools/build/lib/rules/cpp/ArtifactCategory.java:82 ArtifactCategory.getAllowedExtensions
    pub fn get_allowed_extensions(self) -> &'static [&'static str] {
        self.spec().1
    }
}

impl fmt::Display for ArtifactCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
