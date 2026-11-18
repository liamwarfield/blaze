//! Targets: ports of `packages/Target.java`, `TargetData.java`, `Rule.java`,
//! `RuleOrMacroInstance.java` (attribute storage), `InputFile.java` (+ its subclasses),
//! `OutputFile.java`, `FileTarget.java`, `PackageGroup.java`, `PackageSpecification.java`
//! (+ `PackageGroupContents`), `EnvironmentGroup.java`, `EnvironmentLabels.java`,
//! `TargetUtils.java` (+ `TestTargetUtils`), `LabelVisitationUtils.java`, `LabelPrinter.java` and
//! `query2/compat/FakeLoadTarget.java`.

pub mod environment_group;
pub mod fake_load_target;
pub mod input_file;
pub mod label_printer;
pub mod label_visitation_utils;
pub mod output_file;
pub mod package_group;
pub mod package_specification;
pub mod rule;
pub mod rule_or_macro_instance;
#[allow(clippy::module_inception)]
pub mod target;
pub mod target_data;
pub mod target_utils;

pub use environment_group::EnvironmentGroup;
pub use environment_group::EnvironmentLabels;
pub use fake_load_target::FakeLoadTarget;
pub use input_file::InputFile;
pub use input_file::InputFileKind;
pub use label_printer::LabelPrinter;
pub use output_file::OutputFile;
pub use output_file::OutputFileKind;
pub use package_group::PackageGroup;
pub use package_specification::InvalidPackageSpecificationException;
pub use package_specification::PackageGroupContents;
pub use package_specification::PackageSpecification;
pub use rule::IS_EXECUTABLE_ATTRIBUTE_NAME;
pub use rule::Rule;
pub use rule::RuleOutput;
pub use rule_or_macro_instance::AttributeContainer;
pub use rule_or_macro_instance::ImplicitTestsRef;
pub use rule_or_macro_instance::RuleOrMacroInstance;
pub use target::Target;
pub use target_data::TargetData;
