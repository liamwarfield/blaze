//! Port of `packages/TargetData.java`: the target data available without the package (Java
//! uses it for serialized targets; here every target implements it).

use blaze_cmdline::Label;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_starlark::Location;
use blaze_vfs::Path;
use indexmap::IndexSet;

use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::RequiredProviders;

/// Java `TargetData`.
// Java: com/google/devtools/build/lib/packages/TargetData.java:24 TargetData
pub trait TargetData {
    /// The kind of target, e.g. `cc_library rule`, `source file`, `generated file`.
    // Java: com/google/devtools/build/lib/packages/TargetData.java:30 TargetData.getTargetKind
    fn get_target_kind(&self) -> String;

    // Java: com/google/devtools/build/lib/packages/TargetData.java:41 TargetData.getLocation
    fn get_location(&self) -> Location;

    /// The rule class name for rules, `""` otherwise.
    // Java: com/google/devtools/build/lib/packages/TargetData.java:44 TargetData.getRuleClass
    fn get_rule_class(&self) -> String {
        String::new()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:49 TargetData.getRuleTags
    fn get_rule_tags(&self) -> IndexSet<String> {
        IndexSet::new()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:58 TargetData.getName
    fn get_name(&self) -> &str {
        self.get_label().get_name()
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:63 TargetData.getLabel
    fn get_label(&self) -> &Label;

    // Java: com/google/devtools/build/lib/packages/TargetData.java:65 TargetData.isRule
    fn is_rule(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:69 TargetData.isFile
    fn is_file(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:73 TargetData.isInputFile
    fn is_input_file(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:77 TargetData.isOutputFile
    fn is_output_file(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:83 TargetData.getGeneratingRuleLabel
    fn get_generating_rule_label(&self) -> Option<Label> {
        None
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:89 TargetData.getInputPath
    fn get_input_path(&self) -> Option<Path> {
        None
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:95 TargetData.getDeprecationWarning
    fn get_deprecation_warning(&self) -> Option<String> {
        None
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:100 TargetData.isTestOnly
    fn is_test_only(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:109 TargetData.satisfies
    fn satisfies(&self, required: &RequiredProviders) -> bool {
        let _ = required;
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:115 TargetData.getTestTimeout
    fn get_test_timeout(&self) -> Option<TestTimeout> {
        None
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:120 TargetData.isForDependencyResolution
    fn is_for_dependency_resolution(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/TargetData.java:124 TargetData.getAdvertisedProviders
    fn get_advertised_providers(&self) -> AdvertisedProviderSet {
        AdvertisedProviderSet::empty()
    }
}
