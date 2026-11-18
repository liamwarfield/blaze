//! Port of `packages/Target.java` as a closed enum (crates/blaze_packages/DESIGN.md
//! "Targets"). Targets do not point back to their `Package`; they hold its
//! [`PackageMetadata`] and [`PackageDeclarations`].

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_starlark::Location;
use blaze_vfs::Path;
use indexmap::IndexSet;

use crate::macros::MacroInstance;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::RequiredProviders;
use crate::target::EnvironmentGroup;
use crate::target::FakeLoadTarget;
use crate::target::InputFile;
use crate::target::OutputFile;
use crate::target::PackageGroup;
use crate::target::Rule;
use crate::target::TargetData;
use crate::types::License;

/// A target in a package (Java `Target` and its implementations).
// Java: com/google/devtools/build/lib/packages/Target.java:29 Target
#[derive(Debug, Clone)]
pub enum Target {
    Rule(Arc<Rule>),
    InputFile(Arc<InputFile>),
    OutputFile(Arc<OutputFile>),
    PackageGroup(Arc<PackageGroup>),
    EnvironmentGroup(Arc<EnvironmentGroup>),
    /// Java `query2/compat/FakeLoadTarget` (query reports loaded .bzl files as targets).
    FakeLoadTarget(Arc<FakeLoadTarget>),
}

impl PartialEq for Target {
    // Java: targets compare by identity.
    fn eq(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Rule(a), Target::Rule(b)) => Arc::ptr_eq(a, b),
            (Target::InputFile(a), Target::InputFile(b)) => Arc::ptr_eq(a, b),
            (Target::OutputFile(a), Target::OutputFile(b)) => Arc::ptr_eq(a, b) || **a == **b,
            (Target::PackageGroup(a), Target::PackageGroup(b)) => Arc::ptr_eq(a, b),
            (Target::EnvironmentGroup(a), Target::EnvironmentGroup(b)) => Arc::ptr_eq(a, b),
            (Target::FakeLoadTarget(a), Target::FakeLoadTarget(b)) => **a == **b,
            _ => false,
        }
    }
}

impl Target {
    fn data(&self) -> &dyn TargetData {
        match self {
            Target::Rule(t) => &**t as &dyn TargetData,
            Target::InputFile(t) => &**t as &dyn TargetData,
            Target::OutputFile(t) => &**t as &dyn TargetData,
            Target::PackageGroup(t) => &**t as &dyn TargetData,
            Target::EnvironmentGroup(t) => &**t as &dyn TargetData,
            Target::FakeLoadTarget(t) => &**t as &dyn TargetData,
        }
    }

    pub fn as_rule(&self) -> Option<&Arc<Rule>> {
        match self {
            Target::Rule(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_input_file(&self) -> Option<&Arc<InputFile>> {
        match self {
            Target::InputFile(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_output_file(&self) -> Option<&Arc<OutputFile>> {
        match self {
            Target::OutputFile(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_package_group(&self) -> Option<&Arc<PackageGroup>> {
        match self {
            Target::PackageGroup(g) => Some(g),
            _ => None,
        }
    }

    pub fn as_fake_load_target(&self) -> Option<&Arc<FakeLoadTarget>> {
        match self {
            Target::FakeLoadTarget(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_environment_group(&self) -> Option<&Arc<EnvironmentGroup>> {
        match self {
            Target::EnvironmentGroup(g) => Some(g),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:63 TargetData.getLabel
    pub fn get_label(&self) -> &Label {
        self.data().get_label()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:58 TargetData.getName
    pub fn get_name(&self) -> &str {
        self.get_label().get_name()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:30 TargetData.getTargetKind
    pub fn get_target_kind(&self) -> String {
        self.data().get_target_kind()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:41 TargetData.getLocation
    pub fn get_location(&self) -> Location {
        self.data().get_location()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:44 TargetData.getRuleClass
    pub fn get_rule_class(&self) -> String {
        self.data().get_rule_class()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:49 TargetData.getRuleTags
    pub fn get_rule_tags(&self) -> IndexSet<String> {
        self.data().get_rule_tags()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:65 TargetData.isRule
    pub fn is_rule(&self) -> bool {
        matches!(self, Target::Rule(_))
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:69 TargetData.isFile
    pub fn is_file(&self) -> bool {
        matches!(self, Target::InputFile(_) | Target::OutputFile(_))
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:73 TargetData.isInputFile
    pub fn is_input_file(&self) -> bool {
        matches!(self, Target::InputFile(_))
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:77 TargetData.isOutputFile
    pub fn is_output_file(&self) -> bool {
        matches!(self, Target::OutputFile(_))
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:83 TargetData.getGeneratingRuleLabel
    pub fn get_generating_rule_label(&self) -> Option<Label> {
        self.data().get_generating_rule_label()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:89 TargetData.getInputPath
    pub fn get_input_path(&self) -> Option<Path> {
        self.data().get_input_path()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:95 TargetData.getDeprecationWarning
    pub fn get_deprecation_warning(&self) -> Option<String> {
        self.data().get_deprecation_warning()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:100 TargetData.isTestOnly
    pub fn is_test_only(&self) -> bool {
        self.data().is_test_only()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:109 TargetData.satisfies
    pub fn satisfies(&self, required: &RequiredProviders) -> bool {
        self.data().satisfies(required)
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:115 TargetData.getTestTimeout
    pub fn get_test_timeout(&self) -> Option<TestTimeout> {
        self.data().get_test_timeout()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:120 TargetData.isForDependencyResolution
    pub fn is_for_dependency_resolution(&self) -> bool {
        self.data().is_for_dependency_resolution()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:124 TargetData.getAdvertisedProviders
    pub fn get_advertised_providers(&self) -> AdvertisedProviderSet {
        self.data().get_advertised_providers()
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:47 Target.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        match self {
            Target::Rule(t) => t.get_package_metadata(),
            Target::InputFile(t) => t.get_package_metadata(),
            Target::OutputFile(t) => t.get_package_metadata(),
            Target::PackageGroup(t) => t.get_package_metadata(),
            Target::EnvironmentGroup(t) => t.get_package_metadata(),
            Target::FakeLoadTarget(t) => t.get_package_metadata(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:50 Target.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        match self {
            Target::Rule(t) => t.get_package_declarations(),
            Target::InputFile(t) => t.get_package_declarations(),
            Target::OutputFile(t) => t.get_package_declarations(),
            Target::PackageGroup(t) => t.get_package_declarations(),
            Target::EnvironmentGroup(t) => t.get_package_declarations(),
            Target::FakeLoadTarget(t) => t.get_package_declarations(),
        }
    }

    /// The innermost symbolic macro that declared this target, if any.
    // PORT: stored on the target instead of looked up in the package.
    // Java: com/google/devtools/build/lib/packages/Target.java:61 Target.getDeclaringMacro
    pub fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>> {
        match self {
            Target::Rule(t) => t.get_declaring_macro(),
            Target::InputFile(t) => t.get_declaring_macro(),
            Target::OutputFile(t) => t.get_generating_rule().get_declaring_macro(),
            Target::PackageGroup(_) | Target::EnvironmentGroup(_) | Target::FakeLoadTarget(_) => {
                None
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:90 Target.getDeclaringPackage
    pub fn get_declaring_package(&self) -> PackageIdentifier {
        // Java: pkg.getDeclaringPackageForTargetIfInMacro(name), else the package itself.
        match self.get_declaring_macro() {
            Some(m) => m.get_definition_package().clone(),
            None => self.get_package_metadata().package_identifier.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:108 Target.isCreatedInSymbolicMacro
    pub fn is_created_in_symbolic_macro(&self) -> bool {
        self.get_declaring_macro().is_some()
    }

    /// The rule associated with this target: the rule itself or an output file's generating
    /// rule; `None` otherwise.
    // Java: com/google/devtools/build/lib/packages/Target.java:128 Target.getAssociatedRule
    pub fn get_associated_rule(&self) -> Option<&Arc<Rule>> {
        match self {
            Target::Rule(r) => Some(r),
            Target::OutputFile(f) => Some(f.get_generating_rule()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:133 Target.getLicense
    pub fn get_license(&self) -> License {
        match self {
            Target::Rule(t) => t.get_license(),
            Target::InputFile(t) => t.get_license(),
            // Java: com/google/devtools/build/lib/packages/FileTarget.java:74 FileTarget.getLicense
            Target::OutputFile(t) => t
                .get_package_declarations()
                .get_package_args()
                .license()
                .cloned()
                .expect("NullPointerException: package license"),
            Target::PackageGroup(t) => t.get_license(),
            Target::EnvironmentGroup(t) => t.get_license(),
            // Java: com/google/devtools/build/lib/query2/compat/FakeLoadTarget.java:74 FakeLoadTarget.getLicense
            Target::FakeLoadTarget(_) => panic!("UnsupportedOperationException"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:155 Target.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        match self {
            Target::Rule(t) => t.get_raw_visibility(),
            Target::InputFile(t) => t.get_raw_visibility(),
            Target::OutputFile(t) => t.get_raw_visibility(),
            Target::PackageGroup(t) => t.get_raw_visibility(),
            Target::EnvironmentGroup(t) => t.get_raw_visibility(),
            Target::FakeLoadTarget(t) => t.get_raw_visibility(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:166 Target.getDefaultVisibility
    pub fn get_default_visibility(&self) -> RuleVisibility {
        match self {
            // Java: com/google/devtools/build/lib/packages/Rule.java:695 Rule.getDefaultVisibility
            Target::Rule(r) => r.get_default_visibility(),
            // Java: com/google/devtools/build/lib/packages/OutputFile.java:64 OutputFile.getDefaultVisibility
            Target::OutputFile(f) => f.get_default_visibility(),
            _ => {
                if self.is_created_in_symbolic_macro() {
                    RuleVisibility::Private
                } else {
                    self.get_package_declarations()
                        .get_package_args()
                        .default_visibility()
                        .cloned()
                        .expect("NullPointerException: default visibility")
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:186 Target.getVisibility
    pub fn get_visibility(&self) -> RuleVisibility {
        match self {
            // Java: com/google/devtools/build/lib/packages/OutputFile.java:73 OutputFile.getVisibility
            Target::OutputFile(f) => f.get_visibility(),
            // Java: com/google/devtools/build/lib/packages/PackageGroup.java:156 PackageGroup.getVisibility
            Target::PackageGroup(g) => g.get_visibility(),
            // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:312 EnvironmentGroup.getVisibility
            Target::EnvironmentGroup(g) => g.get_visibility(),
            _ => match self.get_raw_visibility() {
                Some(result) => result,
                None => self.get_default_visibility(),
            },
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:198 Target.getVisibilityDependencyLabels
    pub fn get_visibility_dependency_labels(&self) -> Vec<Label> {
        match self {
            // Java: com/google/devtools/build/lib/packages/Rule.java:713 Rule.getVisibilityDependencyLabels
            Target::Rule(r) => r.get_visibility_dependency_labels(),
            // Java: com/google/devtools/build/lib/packages/OutputFile.java:82 OutputFile.getVisibilityDependencyLabels
            Target::OutputFile(f) => f.get_visibility_dependency_labels(),
            _ => self.get_visibility().get_dependency_labels(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:209 Target.getVisibilityDeclaredLabels
    pub fn get_visibility_declared_labels(&self) -> Vec<Label> {
        match self {
            // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:567 RuleOrMacroInstance.getVisibilityDeclaredLabels
            Target::Rule(r) => {
                crate::target::RuleOrMacroInstance::get_visibility_declared_labels(&**r)
            }
            // Java: com/google/devtools/build/lib/packages/OutputFile.java:92 OutputFile.getVisibilityDeclaredLabels
            Target::OutputFile(f) => {
                crate::target::RuleOrMacroInstance::get_visibility_declared_labels(
                    &**f.get_generating_rule(),
                )
            }
            _ => self.get_visibility().get_declared_labels(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:224 Target.getActualVisibility
    pub fn get_actual_visibility(&self) -> RuleVisibility {
        let visibility = self.get_visibility();
        let instantiating_loc = match self.get_declaring_macro() {
            None => self.get_package_metadata().package_identifier.clone(),
            Some(declaring_macro) => declaring_macro.get_definition_package().clone(),
        };
        visibility.concat_with_package(&instantiating_loc)
    }

    // Java: com/google/devtools/build/lib/packages/Target.java:235 Target.isConfigurable
    pub fn is_configurable(&self) -> bool {
        match self {
            Target::Rule(_) => true,
            Target::InputFile(t) => t.is_configurable(),
            Target::OutputFile(t) => t.is_configurable(),
            Target::PackageGroup(t) => t.is_configurable(),
            Target::EnvironmentGroup(t) => t.is_configurable(),
            Target::FakeLoadTarget(t) => t.is_configurable(),
        }
    }

    /// The label in display form (apparent repo names in the main repo).
    // Java: com/google/devtools/build/lib/packages/Target.java:243 Target.getDisplayFormLabel
    pub fn get_display_form_label(&self) -> String {
        let label = self.get_label();
        let mapping = if label.get_repository().is_main() {
            Some(&self.get_package_metadata().repository_mapping)
        } else {
            None
        };
        label.get_display_form(mapping)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Rule(t) => fmt::Display::fmt(&**t, f),
            Target::InputFile(t) => fmt::Display::fmt(&**t, f),
            Target::OutputFile(t) => fmt::Display::fmt(&**t, f),
            Target::PackageGroup(t) => fmt::Display::fmt(&**t, f),
            Target::EnvironmentGroup(t) => fmt::Display::fmt(&**t, f),
            Target::FakeLoadTarget(t) => fmt::Display::fmt(&**t, f),
        }
    }
}
