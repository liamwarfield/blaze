//! Ports `packages/InputFile.java`, `VisibilityLicenseSpecifiedInputFile.java`,
//! `PrivateVisibilityInputFile.java` and the parts of `FileTarget.java` input files use.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_starlark::Location;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;

use crate::macros::MacroInstance;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::target::TargetData;
use crate::types::License;

/// The Java subclass of an [`InputFile`].
#[derive(Debug, Clone, PartialEq)]
pub enum InputFileKind {
    /// Plain `InputFile`.
    Plain,
    /// Java `VisibilityLicenseSpecifiedInputFile` (from `exports_files`).
    // Java: com/google/devtools/build/lib/packages/VisibilityLicenseSpecifiedInputFile.java:24 VisibilityLicenseSpecifiedInputFile
    VisibilityLicenseSpecified {
        visibility: Option<RuleVisibility>,
        license: Option<License>,
    },
    /// Java `PrivateVisibilityInputFile` (implicitly created inputs with
    /// `--incompatible_no_implicit_file_export`).
    // Java: com/google/devtools/build/lib/packages/PrivateVisibilityInputFile.java:21 PrivateVisibilityInputFile
    PrivateVisibility,
}

/// A source file of a package (Java `InputFile`).
// Java: com/google/devtools/build/lib/packages/InputFile.java:42 InputFile
#[derive(Debug, Clone)]
pub struct InputFile {
    label: Label,
    package_metadata: Arc<PackageMetadata>,
    package_declarations: Arc<PackageDeclarations>,
    /// PORT: stored on the target (see Rule).
    declaring_macro: Option<Arc<MacroInstance>>,
    location: Location,
    kind: InputFileKind,
}

impl InputFile {
    // Java: com/google/devtools/build/lib/packages/InputFile.java:47 InputFile.<init>
    pub fn new(
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
        label: Label,
        location: Location,
        kind: InputFileKind,
    ) -> Self {
        InputFile {
            label,
            package_metadata,
            package_declarations,
            declaring_macro: None,
            location,
            kind,
        }
    }

    /// Records the innermost symbolic macro the file was declared in (Java: the package's
    /// `targetsToDeclaringMacro` entry).
    pub fn with_declaring_macro(mut self, declaring_macro: Option<Arc<MacroInstance>>) -> Self {
        self.declaring_macro = declaring_macro;
        self
    }

    pub fn kind(&self) -> &InputFileKind {
        &self.kind
    }

    pub fn get_label(&self) -> &Label {
        &self.label
    }

    pub fn get_name(&self) -> &str {
        self.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/FileTarget.java:37 FileTarget.getFilename
    pub fn get_filename(&self) -> &str {
        self.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:63 InputFile.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:68 InputFile.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    pub fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.declaring_macro.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:72 InputFile.isVisibilitySpecified
    pub fn is_visibility_specified(&self) -> bool {
        match &self.kind {
            InputFileKind::Plain => false,
            // Java: com/google/devtools/build/lib/packages/VisibilityLicenseSpecifiedInputFile.java:41 VisibilityLicenseSpecifiedInputFile.isVisibilitySpecified
            InputFileKind::VisibilityLicenseSpecified { visibility, .. } => visibility.is_some(),
            // Java: com/google/devtools/build/lib/packages/PrivateVisibilityInputFile.java:28 PrivateVisibilityInputFile.isVisibilitySpecified
            InputFileKind::PrivateVisibility => true,
        }
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:77 InputFile.isBuildFile
    pub fn is_build_file(&self) -> bool {
        self.label == self.package_metadata.build_file_label
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:83 InputFile.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        match &self.kind {
            InputFileKind::Plain => None,
            // Java: com/google/devtools/build/lib/packages/VisibilityLicenseSpecifiedInputFile.java:47 VisibilityLicenseSpecifiedInputFile.getRawVisibility
            InputFileKind::VisibilityLicenseSpecified { visibility, .. } => visibility.clone(),
            // Java: com/google/devtools/build/lib/packages/PrivateVisibilityInputFile.java:33 PrivateVisibilityInputFile.getRawVisibility
            InputFileKind::PrivateVisibility => Some(RuleVisibility::Private),
        }
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:88 InputFile.isConfigurable
    pub fn is_configurable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:92 InputFile.isLicenseSpecified
    pub fn is_license_specified(&self) -> bool {
        match &self.kind {
            // Java: com/google/devtools/build/lib/packages/VisibilityLicenseSpecifiedInputFile.java:52 VisibilityLicenseSpecifiedInputFile.isLicenseSpecified
            InputFileKind::VisibilityLicenseSpecified { license, .. } => {
                license.as_ref().is_some_and(License::is_specified)
            }
            _ => false,
        }
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:97 InputFile.getLicense
    pub fn get_license(&self) -> License {
        match &self.kind {
            // Java: com/google/devtools/build/lib/packages/VisibilityLicenseSpecifiedInputFile.java:57 VisibilityLicenseSpecifiedInputFile.getLicense
            InputFileKind::VisibilityLicenseSpecified { license, .. } => match license {
                Some(license) => license.clone(),
                None => self
                    .package_declarations
                    .get_package_args()
                    .license()
                    .cloned()
                    .expect("NullPointerException: package license"),
            },
            _ => License::no_license(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:108 InputFile.getPath
    pub fn get_path(&self) -> Path {
        self.package_metadata
            .get_package_directory()
            .get_relative_str(self.label.get_name())
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:116 InputFile.getExecPath
    pub fn get_exec_path(&self, sibling_repository_layout: bool) -> PathFragment {
        self.label
            .get_repository()
            .get_exec_path(sibling_repository_layout)
            .get_relative_str(self.label.get_package_name())
            .get_relative_str(self.label.get_name())
    }

    /// Java `InputFile.targetKind()`: `"source file"`.
    // Java: com/google/devtools/build/lib/packages/InputFile.java:141 InputFile.targetKind
    pub fn target_kind() -> &'static str {
        "source file"
    }
}

impl fmt::Display for InputFile {
    // Java: com/google/devtools/build/lib/packages/FileTarget.java:57 FileTarget.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", InputFile::target_kind(), self.label)
    }
}

impl TargetData for InputFile {
    // Java: com/google/devtools/build/lib/packages/InputFile.java:125 InputFile.getTargetKind
    fn get_target_kind(&self) -> String {
        InputFile::target_kind().to_owned()
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:136 InputFile.getLocation
    fn get_location(&self) -> Location {
        self.location.clone()
    }

    fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/packages/FileTarget.java:52 FileTarget.isFile
    fn is_file(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:146 InputFile.isInputFile
    fn is_input_file(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/packages/InputFile.java:151 InputFile.getInputPath
    fn get_input_path(&self) -> Option<Path> {
        Some(self.get_path())
    }
}
