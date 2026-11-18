//! Packages and package construction: ports of `packages/Package.java` (with `Metadata`,
//! `Declarations`, `Builder`), `Packageoid.java`, `TargetDefinitionContext.java`,
//! `TargetRecorder.java`, `RuleFactory.java`, `PackageArgs.java`, `RuleVisibility.java`,
//! `PackageGroupsRuleVisibility.java`, `CallStack.java`, `BuildFileName.java`, `Globber.java`,
//! `PackageValidator.java`, `TargetSuggester.java`, `TestSuiteImplicitTestsAccumulator.java`.

pub mod build_file_name;
pub mod call_stack;
pub mod globber;
#[allow(clippy::module_inception)]
pub mod package;
pub mod package_args;
pub mod package_validator;
pub mod rule_factory;
pub mod rule_visibility;
pub mod target_definition_context;
pub mod target_recorder;
pub mod target_suggester;
pub mod test_suite_implicit_tests_accumulator;

pub use build_file_name::BuildFileName;
pub use call_stack::CallStackNode;
pub use globber::BadGlobException;
pub use globber::Globber;
pub use globber::GlobberError;
pub use globber::GlobberOperation;
pub use globber::GlobberToken;
pub use package::ConfigSettingVisibilityPolicy;
pub use package::Package;
pub use package::PackageDeclarations;
pub use package::PackageLimits;
pub use package::PackageMetadata;
pub use package::PackageSettings;
pub use package_args::FeatureSet;
pub use package_args::PackageArgs;
pub use package_args::PackageArgsBuilder;
pub use package_validator::InvalidPackageException;
pub use package_validator::NoopValidator;
pub use package_validator::PackageValidator;
pub use rule_factory::BuildLangTypedAttributeValuesMap;
pub use rule_factory::BuiltinRuleFunction;
pub use rule_factory::CreateAndAddRuleError;
pub use rule_factory::InvalidRuleException;
pub use rule_visibility::PackageGroupsRuleVisibility;
pub use rule_visibility::RuleVisibility;
pub use target_definition_context::FromOrFailMode;
pub use target_definition_context::PackageBuilderError;
pub use target_definition_context::TargetDefinitionContext;
pub use target_recorder::MACRO_NAMING_RULES;
pub use target_recorder::MacroFrame;
pub use target_recorder::MacroNamespaceViolationException;
pub use target_recorder::NameConflictException;
pub use target_recorder::TargetRecorder;

/// Java `Package.Builder`: the package builder is the [`TargetDefinitionContext`].
pub type PackageBuilder = TargetDefinitionContext;
