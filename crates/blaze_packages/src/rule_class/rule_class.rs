//! Port of `packages/RuleClass.java`: [`RuleClass`], [`RuleClassBuilder`] (Java
//! `RuleClass.Builder`), [`RuleClassType`], [`RuleClassNamePredicate`],
//! [`ToolchainResolutionMode`], [`AutoExecGroupsMode`].

use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelSyntaxException;
use blaze_events::EventHandler;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeBuilder;
use crate::attribute::AttributeMap;
use crate::attribute::AttributeProvider;
use crate::attribute::CannotPrecomputeDefaultsException;
use crate::attribute::ImplicitOutputsFunction;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::attribute::RawAttributeMapper;
use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::attribute::string_util::join_english_list;
use crate::package::BuildLangTypedAttributeValuesMap;
use crate::package::CallStackNode;
use crate::package::TargetDefinitionContext;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::AdvertisedProviderSetBuilder;
use crate::rule_class::AllowlistChecker;
use crate::rule_class::BuildSetting;
use crate::rule_class::ConfigurationFragmentPolicy;
use crate::rule_class::ConfigurationFragmentPolicyBuilder;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::MissingFragmentPolicy;
use crate::rule_class::RuleClassId;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::target::OutputFileKind;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::LabelClass;
use crate::types::LabelConverter;
use crate::types::Type;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use std::collections::HashMap;

/// Java `RuleClass.NAME_ATTRIBUTE`: `attr("name", STRING_NO_INTERN)`, nonconfigurable,
/// mandatory.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:120 RuleClass.NAME_ATTRIBUTE
pub static NAME_ATTRIBUTE: LazyLock<Attribute> = LazyLock::new(|| {
    attr("name", Type::StringNoIntern)
        .nonconfigurable("All rules have a non-customizable \"name\" attribute")
        .mandatory()
        .build()
});

// Java: com/google/devtools/build/lib/packages/RuleClass.java:126 RuleClass.MAX_ATTRIBUTES
pub const MAX_ATTRIBUTES: usize = 200;
// Java: com/google/devtools/build/lib/packages/RuleClass.java:128 RuleClass.MAX_ATTRIBUTE_NAME_LENGTH
pub const MAX_ATTRIBUTE_NAME_LENGTH: usize = 128;
// Java: com/google/devtools/build/lib/packages/RuleClass.java:145 RuleClass.THIRD_PARTY_PREFIX
pub const THIRD_PARTY_PREFIX: &str = "third_party";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:146 RuleClass.EXPERIMENTAL_PREFIX
pub const EXPERIMENTAL_PREFIX: &str = "experimental";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:149 RuleClass.APPLICABLE_METADATA_ATTR
pub const APPLICABLE_METADATA_ATTR: &str = "package_metadata";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:151 RuleClass.APPLICABLE_METADATA_ATTR_ALT
pub const APPLICABLE_METADATA_ATTR_ALT: &str = "applicable_licenses";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:153 RuleClass.DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME
pub const DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME: &str = "test";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:250 RuleClass.RESTRICTED_ENVIRONMENT_ATTR
pub const RESTRICTED_ENVIRONMENT_ATTR: &str = "restricted_to";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:256 RuleClass.COMPATIBLE_ENVIRONMENT_ATTR
pub const COMPATIBLE_ENVIRONMENT_ATTR: &str = "compatible_with";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:262 RuleClass.TARGET_COMPATIBLE_WITH_ATTR
pub const TARGET_COMPATIBLE_WITH_ATTR: &str = "target_compatible_with";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:268 RuleClass.EXEC_COMPATIBLE_WITH_ATTR
pub const EXEC_COMPATIBLE_WITH_ATTR: &str = "exec_compatible_with";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:274 RuleClass.EXEC_GROUP_COMPATIBLE_WITH_ATTR
pub const EXEC_GROUP_COMPATIBLE_WITH_ATTR: &str = "exec_group_compatible_with";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:280 RuleClass.EXEC_PROPERTIES_ATTR
pub const EXEC_PROPERTIES_ATTR: &str = "exec_properties";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:286 RuleClass.DEFAULT_RESTRICTED_ENVIRONMENT_ATTR
pub const DEFAULT_RESTRICTED_ENVIRONMENT_ATTR: &str = "$restricted_to";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:293 RuleClass.DEFAULT_COMPATIBLE_ENVIRONMENT_ATTR
pub const DEFAULT_COMPATIBLE_ENVIRONMENT_ATTR: &str = "$compatible_with";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:324 RuleClass.CONFIG_SETTING_DEPS_ATTRIBUTE
pub const CONFIG_SETTING_DEPS_ATTRIBUTE: &str = "$config_dependencies";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:667 RuleClass.Builder.STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME
pub const STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME: &str = "build_setting_default";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:669 RuleClass.Builder.STARLARK_BUILD_SETTING_HELP_ATTR_NAME
pub const STARLARK_BUILD_SETTING_HELP_ATTR_NAME: &str = "help";
// Java: com/google/devtools/build/lib/packages/RuleClass.java:671 RuleClass.Builder.BUILD_SETTING_DEFAULT_NONCONFIGURABLE
pub const BUILD_SETTING_DEFAULT_NONCONFIGURABLE: &str =
    "Build setting defaults are referenced during analysis.";

/// Whether a rule uses toolchain resolution (Java `RuleClass.ToolchainResolutionMode`).
// Java: com/google/devtools/build/lib/packages/RuleClass.java:171 RuleClass.ToolchainResolutionMode
#[derive(Clone)]
pub enum ToolchainResolutionMode {
    Enabled,
    Disabled,
    /// A native rule's custom predicate.
    Custom(Arc<dyn Fn(&Rule) -> bool + Send + Sync>),
}

impl ToolchainResolutionMode {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:172 RuleClass.ToolchainResolutionMode.useToolchainResolution
    pub fn use_toolchain_resolution(&self, rule: &Rule) -> bool {
        match self {
            ToolchainResolutionMode::Enabled => true,
            ToolchainResolutionMode::Disabled => false,
            ToolchainResolutionMode::Custom(f) => f(rule),
        }
    }
}

impl fmt::Debug for ToolchainResolutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolchainResolutionMode::Enabled => f.write_str("Enabled"),
            ToolchainResolutionMode::Disabled => f.write_str("Disabled"),
            ToolchainResolutionMode::Custom(_) => f.write_str("Custom(..)"),
        }
    }
}

/// Java `RuleClass.AutoExecGroupsMode`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:179 RuleClass.AutoExecGroupsMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutoExecGroupsMode {
    Disabled,
    Enabled,
    Dynamic,
}

impl AutoExecGroupsMode {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:184 RuleClass.AutoExecGroupsMode.isEnabled
    pub fn is_enabled(
        self,
        attributes: &dyn AttributeMap,
        is_allowed_by_configuration: bool,
    ) -> bool {
        match self {
            AutoExecGroupsMode::Disabled => false,
            AutoExecGroupsMode::Enabled => true,
            AutoExecGroupsMode::Dynamic => {
                if attributes.has("$use_auto_exec_groups") {
                    attributes
                        .get("$use_auto_exec_groups", Type::Boolean)
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                } else {
                    is_allowed_by_configuration
                }
            }
        }
    }
}

/// The type of a rule class: affects allowed names and required attributes (Java
/// `RuleClass.Builder.RuleClassType`).
// Java: com/google/devtools/build/lib/packages/RuleClass.java:337 RuleClass.Builder.RuleClassType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleClassType {
    /// Abstract rules are for rule class inheritance only; names contain `$`.
    Abstract,
    /// Invisible rule classes cannot be instantiated in BUILD files; names contain `$`.
    Invisible,
    /// Normal rules are instantiable by BUILD files.
    Normal,
    /// Rules that are only instantiable by BUILD files (not by legacy macros).
    BuildOnly,
    /// Test rules (name ends in `_test`).
    Test,
    /// Placeholder rules, for Starlark rules whose definitions are not loaded.
    Placeholder,
}

impl RuleClassType {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:346 RuleClass.Builder.RuleClassType.checkName
    pub fn check_name(self, name: &str) {
        match self {
            RuleClassType::Abstract => {
                blaze_util::check_argument!(
                    (name.contains('$') && !is_test_rule_name(name)) || name.is_empty()
                );
            }
            RuleClassType::Invisible => {
                blaze_util::check_argument!(name.contains('$'));
            }
            // Java: com/google/devtools/build/lib/packages/RuleClass.java:414 RuleClass.Builder.RuleClassType.BUILD_ONLY.checkName
            RuleClassType::Normal | RuleClassType::BuildOnly => {
                blaze_util::check_argument!(
                    !is_test_rule_name(name) && matches_rule_name_pattern(name),
                    "Invalid rule name: {}",
                    name
                );
            }
            RuleClassType::Test => {
                blaze_util::check_argument!(
                    is_test_rule_name(name) && matches_rule_name_pattern(name)
                );
            }
            RuleClassType::Placeholder => {
                blaze_util::check_argument!(matches_rule_name_pattern(name), "{}", name);
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:357 RuleClass.Builder.RuleClassType.checkAttributes
    pub fn check_attributes(self, attributes: &IndexMap<String, Attribute>) {
        let (required, kind): (&[(&str, Type)], &str) = match self {
            // No required attributes.
            RuleClassType::Abstract | RuleClassType::Invisible | RuleClassType::Placeholder => {
                return;
            }
            RuleClassType::Normal | RuleClassType::BuildOnly => {
                (REQUIRED_ATTRIBUTES_FOR_NORMAL_RULES, "normal")
            }
            RuleClassType::Test => (REQUIRED_ATTRIBUTES_FOR_TESTS, "test"),
        };
        for (name, type_) in required {
            let Some(present_attribute) = attributes.get(*name) else {
                panic!(
                    "IllegalStateException: Missing mandatory '{name}' attribute in {kind} rule \
                     class."
                );
            };
            if present_attribute.get_type() != *type_ {
                panic!(
                    "IllegalStateException: Mandatory attribute '{name}' in {kind} rule class has \
                     incorrect type (expected {type_})."
                );
            }
        }
    }
}

// Java: com/google/devtools/build/lib/packages/RuleClass.java:675 RuleClass.Builder.REQUIRED_ATTRIBUTES_FOR_NORMAL_RULES
const REQUIRED_ATTRIBUTES_FOR_NORMAL_RULES: &[(&str, Type)] = &[("tags", Type::StringList)];

// Java: com/google/devtools/build/lib/packages/RuleClass.java:679 RuleClass.Builder.REQUIRED_ATTRIBUTES_FOR_TESTS
const REQUIRED_ATTRIBUTES_FOR_TESTS: &[(&str, Type)] = &[
    ("tags", Type::StringList),
    ("size", Type::String),
    ("timeout", Type::String),
    ("flaky", Type::Boolean),
    ("shard_count", Type::Integer),
    ("local", Type::Boolean),
];

/// Java `TargetUtils.isTestRuleName`.
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:67 TargetUtils.isTestRuleName
pub fn is_test_rule_name(name: &str) -> bool {
    name.ends_with("_test")
}

// Java: com/google/devtools/build/lib/packages/RuleClass.java:334 RuleClass.Builder.RULE_NAME_PATTERN
fn matches_rule_name_pattern(name: &str) -> bool {
    // [A-Za-z_][A-Za-z0-9_]*
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl RuleClassType {}

/// Java `RuleClassNamePredicate.PredicateType`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:520 RuleClass.Builder.RuleClassNamePredicate.PredicateType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleClassNamePredicateType {
    Only,
    AllExcept,
    Unspecified,
}

/// A predicate over rule class names (Java `RuleClass.Builder.RuleClassNamePredicate`).
// Java: com/google/devtools/build/lib/packages/RuleClass.java:505 RuleClass.Builder.RuleClassNamePredicate
#[derive(Debug, Clone)]
pub struct RuleClassNamePredicate {
    rule_class_names: IndexSet<String>,
    predicate_type: RuleClassNamePredicateType,
    /// If set, used ONLY for checking overlap (Java `overlappable`, set by `only`).
    overlappable: Option<IndexSet<String>>,
}

// PORT: skipped, equality is PartialEq (never hashed): RuleClass.hashCode, RuleClass.equals
impl PartialEq for RuleClassNamePredicate {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:630 RuleClass.Builder.RuleClassNamePredicate.equals
    fn eq(&self, other: &RuleClassNamePredicate) -> bool {
        self.rule_class_names == other.rule_class_names
            && self.predicate_type == other.predicate_type
    }
}

impl RuleClassNamePredicate {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:564 RuleClass.Builder.RuleClassNamePredicate.only
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:569 RuleClass.Builder.RuleClassNamePredicate.only
    pub fn only(rule_class_names: &[&str]) -> Self {
        let names: IndexSet<String> = rule_class_names.iter().map(|s| (*s).to_owned()).collect();
        RuleClassNamePredicate {
            rule_class_names: names.clone(),
            predicate_type: RuleClassNamePredicateType::Only,
            overlappable: Some(names),
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:573 RuleClass.Builder.RuleClassNamePredicate.allExcept
    pub fn all_except(rule_classes: &[&str]) -> Self {
        let names: IndexSet<String> = rule_classes.iter().map(|s| (*s).to_owned()).collect();
        blaze_util::check_state!(!names.is_empty(), "Use unspecified() instead");
        RuleClassNamePredicate {
            rule_class_names: names,
            predicate_type: RuleClassNamePredicateType::AllExcept,
            overlappable: None,
        }
    }

    /// The "default" sentinel predicate: accepts everything.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:585 RuleClass.Builder.RuleClassNamePredicate.unspecified
    pub fn unspecified() -> Self {
        RuleClassNamePredicate {
            rule_class_names: IndexSet::new(),
            predicate_type: RuleClassNamePredicateType::Unspecified,
            overlappable: None,
        }
    }

    pub fn get_predicate_type(&self) -> RuleClassNamePredicateType {
        self.predicate_type
    }

    pub fn get_rule_class_names(&self) -> &IndexSet<String> {
        &self.rule_class_names
    }

    /// Java `asPredicateOfRuleClass().apply(name)`.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:589 RuleClass.Builder.RuleClassNamePredicate.asPredicateOfRuleClass
    pub fn as_predicate_of_rule_class(&self, rule_class_name: &str) -> bool {
        match self.predicate_type {
            RuleClassNamePredicateType::Only => self.rule_class_names.contains(rule_class_name),
            RuleClassNamePredicateType::AllExcept => {
                !self.rule_class_names.contains(rule_class_name)
            }
            RuleClassNamePredicateType::Unspecified => true,
        }
    }

    /// Java `asPredicateOfRuleClassObject().apply(ruleClass)`.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:593 RuleClass.Builder.RuleClassNamePredicate.asPredicateOfRuleClassObject
    pub fn as_predicate_of_rule_class_object(&self, rule_class: &RuleClass) -> bool {
        self.as_predicate_of_rule_class(rule_class.get_name())
    }

    /// Whether this and `that` are incompatible as rule class and warning predicates.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:605 RuleClass.Builder.RuleClassNamePredicate.consideredOverlapping
    pub fn considered_overlapping(&self, that: &RuleClassNamePredicate) -> bool {
        match (&self.overlappable, &that.overlappable) {
            (Some(a), Some(b)) => a.iter().any(|x| b.contains(x)),
            _ => false,
        }
    }

    /// The Java `toString` of the name predicate (e.g. "a, b or c", "all but ...").
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:620 RuleClass.Builder.RuleClassNamePredicate.toString
    // (the DescribedPredicate descriptions: `joinEnglishList(names)`, `all but ...`)
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:656 RuleClass.Builder.RuleClassNamePredicate.DescribedPredicate.toString
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:640 RuleClass.Builder.RuleClassNamePredicate.DescribedPredicate.apply
    pub fn describe(&self) -> String {
        let names: Vec<&str> = self.rule_class_names.iter().map(String::as_str).collect();
        match self.predicate_type {
            RuleClassNamePredicateType::Only => join_english_list(&names),
            RuleClassNamePredicateType::AllExcept => {
                format!("all but {}", join_english_list(&names))
            }
            // Java: Predicates.alwaysTrue().toString()
            RuleClassNamePredicateType::Unspecified => "Predicates.alwaysTrue()".to_owned(),
        }
    }
}

/// The function computing the option labels a `config_setting` references (Java
/// `RuleClass.optionReferenceFunction`).
pub type OptionReferenceFunction = Arc<dyn Fn(&Rule) -> IndexSet<String> + Send + Sync>;

/// A rule class: its attributes and properties (Java `RuleClass`). Immutable; shared as
/// `Arc<RuleClass>`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:97 RuleClass
pub struct RuleClass {
    rule_class_id: RuleClassId,
    /// The stack of the call to `rule()`.
    callstack: Vec<StackEntry>,
    target_kind: String,
    type_: RuleClassType,
    starlark_parent: Option<Arc<RuleClass>>,
    initializer: Option<StarlarkCallable>,
    label_converter_for_initializer: Option<Arc<LabelConverter>>,
    is_starlark: bool,
    extendable: bool,
    starlark_extension_label: Option<Label>,
    starlark_documentation: Option<String>,
    extendable_allowlist: Option<Label>,
    starlark_testable: bool,
    documented: bool,
    outputs_to_bindir: bool,
    dependency_resolution_rule: bool,
    is_materializer_rule: bool,
    materializer_rule_allows_real_deps: bool,
    is_executable_starlark: bool,
    is_analysis_test: bool,
    has_analysis_test_transition: bool,
    allowlist_checkers: Vec<AllowlistChecker>,
    has_aspects: bool,
    attribute_provider: AttributeProvider,
    implicit_outputs_function: ImplicitOutputsFunction,
    transition_factory: TransitionFactory,
    /// Java `ConfiguredTargetFactory` of a native rule. PORT: analysis (W4); the factory's Java
    /// class simple name until then.
    configured_target_factory: Option<String>,
    advertised_providers: AdvertisedProviderSet,
    configured_target_function: Option<StarlarkCallable>,
    build_setting: Option<BuildSetting>,
    /// Java `ImmutableSet<? extends StarlarkSubruleApi>`: the frozen subrule values.
    subrules: Vec<StarlarkCallable>,
    /// `None` is Java's `NO_OPTION_REFERENCE`.
    option_reference_function: Option<OptionReferenceFunction>,
    rule_definition_environment_label: Option<Label>,
    rule_definition_environment_digest: Option<Vec<u8>>,
    output_file_kind: OutputFileKind,
    configuration_fragment_policy: ConfigurationFragmentPolicy,
    supports_constraint_checking: bool,
    toolchain_types: IndexSet<ToolchainTypeRequirement>,
    toolchain_resolution_mode: ToolchainResolutionMode,
    execution_platform_constraints: IndexSet<Label>,
    declared_exec_groups: IndexMap<String, DeclaredExecGroup>,
    auto_exec_groups_mode: AutoExecGroupsMode,
}

impl fmt::Debug for RuleClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuleClass")
            .field("rule_class_id", &self.rule_class_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RuleClass {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2196 RuleClass.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}

impl RuleClass {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1944 RuleClass.getDefaultImplicitOutputsFunction
    pub fn get_default_implicit_outputs_function(&self) -> &ImplicitOutputsFunction {
        &self.implicit_outputs_function
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1948 RuleClass.getTransitionFactory
    pub fn get_transition_factory(&self) -> &TransitionFactory {
        &self.transition_factory
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1956 RuleClass.getConfiguredTargetFactory
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1952 RuleClass.getConfiguredTargetFactory
    // PORT: skipped, the factory is the analysis crate's dispatch on the factory's name: RuleClass.create
    pub fn get_configured_target_factory(&self) -> Option<&str> {
        self.configured_target_factory.as_deref()
    }

    /// The rule class name (e.g. `cc_library`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1962 RuleClass.getName
    pub fn get_name(&self) -> &str {
        &self.rule_class_id.name
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1966 RuleClass.getStarlarkParent
    pub fn get_starlark_parent(&self) -> Option<&Arc<RuleClass>> {
        self.starlark_parent.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1971 RuleClass.getInitializer
    pub fn get_initializer(&self) -> Option<StarlarkCallable> {
        self.initializer
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1976 RuleClass.getLabelConverterForInitializer
    pub fn get_label_converter_for_initializer(&self) -> Option<&Arc<LabelConverter>> {
        self.label_converter_for_initializer.as_ref()
    }

    /// The stack of the call to `rule()`.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1985 RuleClass.getCallStack
    pub fn get_call_stack(&self) -> &[StackEntry] {
        &self.callstack
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1990 RuleClass.getRuleClassType
    pub fn get_rule_class_type(&self) -> RuleClassType {
        self.type_
    }

    /// The unique key (name for native rules, `<bzl label>%<name>` for Starlark rules).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1995 RuleClass.getKey
    pub fn get_key(&self) -> &str {
        &self.rule_class_id.key
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2000 RuleClass.getRuleClassId
    pub fn get_rule_class_id(&self) -> &RuleClassId {
        &self.rule_class_id
    }

    /// The target kind of this class of rule (e.g. `cc_library rule`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2006 RuleClass.getTargetKind
    pub fn get_target_kind(&self) -> &str {
        &self.target_kind
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2014 RuleClass.getAttributeProvider
    pub fn get_attribute_provider(&self) -> &AttributeProvider {
        &self.attribute_provider
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2030 RuleClass.getAdvertisedProviders
    pub fn get_advertised_providers(&self) -> &AdvertisedProviderSet {
        &self.advertised_providers
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2035 RuleClass.getConfigurationFragmentPolicy
    pub fn get_configuration_fragment_policy(&self) -> &ConfigurationFragmentPolicy {
        &self.configuration_fragment_policy
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2040 RuleClass.supportsConstraintChecking
    pub fn supports_constraint_checking(&self) -> bool {
        self.supports_constraint_checking
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2044 RuleClass.hasAspects
    pub fn has_aspects(&self) -> bool {
        self.has_aspects
    }

    /// Creates a rule of this class in the target definition context, populating and checking
    /// its attributes and outputs. Errors are reported on the context's event handler.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2066 RuleClass.createRule
    pub fn create_rule<'v>(
        self: &Arc<Self>,
        target_definition_context: &TargetDefinitionContext,
        rule_label: Label,
        attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
        fail_on_unknown_attributes: bool,
        callstack: &[StackEntry],
    ) -> Result<Rule, RuleClassCreateRuleError> {
        let event_handler = target_definition_context.get_local_event_handler();

        let mut rule = target_definition_context.create_rule(rule_label, self, callstack);
        self.attribute_provider.populate_rule_attribute_values(
            &mut rule,
            target_definition_context,
            attribute_values,
            fail_on_unknown_attributes,
            self.is_starlark,
        )?;
        RuleClass::check_aspect_allowed_values(&rule, event_handler);
        rule.populate_output_files(
            event_handler,
            target_definition_context.get_package_identifier(),
        )?;
        RuleClass::check_for_duplicate_labels(&rule, event_handler);

        RuleClass::check_for_valid_size_and_timeout_values(&rule, event_handler);
        Ok(rule)
    }

    /// Like [`RuleClass::create_rule`], without some internal checks.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2091 RuleClass.createRuleUnchecked
    pub fn create_rule_unchecked<'v>(
        self: &Arc<Self>,
        target_definition_context: &TargetDefinitionContext,
        rule_label: Label,
        attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
        callstack: &CallStackNode,
        implicit_outputs_function: &ImplicitOutputsFunction,
    ) -> Result<Rule, CannotPrecomputeDefaultsException> {
        let mut rule = target_definition_context.create_rule_at(
            rule_label,
            self,
            callstack.to_location(),
            callstack.next().cloned(),
        );
        self.attribute_provider.populate_rule_attribute_values(
            &mut rule,
            target_definition_context,
            attribute_values,
            true,
            self.is_starlark,
        )?;
        rule.populate_output_files_unchecked(target_definition_context, implicit_outputs_function);
        Ok(rule)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2114 RuleClass.checkForDuplicateLabels
    pub(crate) fn check_for_duplicate_labels(rule: &Rule, event_handler: &dyn EventHandler) {
        let mapper = AggregatingAttributeMapper::of(rule);
        for attribute in rule.get_attribute_provider().get_attributes() {
            let duplicates = mapper.check_for_duplicate_labels(attribute);
            if duplicates.is_empty() {
                continue;
            }
            for label in duplicates {
                rule.report_error(
                    &format!(
                        "Label '{}' is duplicated in the '{}' attribute of rule '{}'",
                        label,
                        attribute.get_name(),
                        rule.get_name()
                    ),
                    event_handler,
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2158 RuleClass.checkAspectAllowedValues
    fn check_aspect_allowed_values(rule: &Rule, event_handler: &dyn EventHandler) {
        if rule.has_aspects() {
            for attr_of_rule in rule.get_attribute_provider().get_attributes() {
                for aspect in attr_of_rule.get_aspects(rule) {
                    for attr_of_aspect in aspect.get_definition().get_attributes().values() {
                        // By this point the AspectDefinition has been created and values assigned.
                        if attr_of_aspect.check_allowed_values() {
                            let allowed_values = attr_of_aspect
                                .get_allowed_values()
                                .expect("allowed values of a checked attribute");
                            let value = attr_of_aspect.get_default_value(None);
                            let Some(value) = value else {
                                blaze_util::unported!(
                                    "RuleClass.checkAspectAllowedValues of a null value"
                                )
                            };
                            if !allowed_values.apply(&value) {
                                if RawAttributeMapper::of(rule)
                                    .is_configurable(attr_of_aspect.get_name())
                                {
                                    rule.report_error(
                                        &format!(
                                            "{}: attribute '{}' has a select() and aspect {} also \
                                             declares '{}'. Aspect attributes don't currently \
                                             support select().",
                                            rule.get_label(),
                                            attr_of_aspect.get_name(),
                                            aspect.get_definition().get_name(),
                                            rule.get_label()
                                        ),
                                        event_handler,
                                    );
                                } else {
                                    rule.report_error(
                                        &format!(
                                            "{}: invalid value in '{}' attribute: {}",
                                            rule.get_label(),
                                            attr_of_aspect.get_name(),
                                            allowed_values.get_error_reason(&value)
                                        ),
                                        event_handler,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2138 RuleClass.checkForValidSizeAndTimeoutValues
    pub(crate) fn check_for_valid_size_and_timeout_values(
        rule: &Rule,
        event_handler: &dyn EventHandler,
    ) {
        if rule
            .get_rule_class_object()
            .get_attribute_provider()
            .has_attr("size", Type::String)
        {
            let size = string_attr(rule, "size");
            if TestSize::get_test_size(&size).is_none() {
                rule.report_error(
                    &format!(
                        "In rule '{}', size '{}' is not a valid size.",
                        rule.get_name(),
                        size
                    ),
                    event_handler,
                );
            }
        }
        if rule
            .get_rule_class_object()
            .get_attribute_provider()
            .has_attr("timeout", Type::String)
        {
            let timeout = string_attr(rule, "timeout");
            if TestTimeout::get_test_timeout(&timeout).is_none() {
                rule.report_error(
                    &format!(
                        "In rule '{}', timeout '{}' is not a valid timeout.",
                        rule.get_name(),
                        timeout
                    ),
                    event_handler,
                );
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2200 RuleClass.isDocumented
    pub fn is_documented(&self) -> bool {
        self.documented
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2210 RuleClass.outputsToBindir
    pub fn outputs_to_bindir(&self) -> bool {
        self.outputs_to_bindir
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2216 RuleClass.getConfiguredTargetFunction
    pub fn get_configured_target_function(&self) -> Option<StarlarkCallable> {
        self.configured_target_function
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2221 RuleClass.getBuildSetting
    pub fn get_build_setting(&self) -> Option<&BuildSetting> {
        self.build_setting.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2226 RuleClass.getOptionReferenceFunction
    pub fn get_option_reference_function(&self) -> Option<&OptionReferenceFunction> {
        self.option_reference_function.as_ref()
    }

    /// The label of the .bzl defining this rule class (`None` for native rules).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2239 RuleClass.getRuleDefinitionEnvironmentLabel
    pub fn get_rule_definition_environment_label(&self) -> Option<&Label> {
        self.rule_definition_environment_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2262 RuleClass.getRuleDefinitionEnvironmentDigest
    pub fn get_rule_definition_environment_digest(&self) -> Option<&[u8]> {
        self.rule_definition_environment_digest.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2268 RuleClass.isStarlark
    pub fn is_starlark(&self) -> bool {
        self.is_starlark
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2285 RuleClass.getStarlarkExtensionLabel
    pub fn get_starlark_extension_label(&self) -> Option<&Label> {
        self.starlark_extension_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2294 RuleClass.getStarlarkDocumentation
    pub fn get_starlark_documentation(&self) -> Option<&str> {
        self.starlark_documentation.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2299 RuleClass.isExtendable
    pub fn is_extendable(&self) -> bool {
        self.extendable
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2304 RuleClass.getExtendableAllowlist
    pub fn get_extendable_allowlist(&self) -> Option<&Label> {
        self.extendable_allowlist.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2309 RuleClass.isStarlarkTestable
    pub fn is_starlark_testable(&self) -> bool {
        self.starlark_testable
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2315 RuleClass.isDependencyResolutionRule
    pub fn is_dependency_resolution_rule(&self) -> bool {
        self.dependency_resolution_rule
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2321 RuleClass.isMaterializerRule
    pub fn is_materializer_rule(&self) -> bool {
        self.is_materializer_rule
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2327 RuleClass.materializerRuleAllowsRealDeps
    pub fn materializer_rule_allows_real_deps(&self) -> bool {
        self.materializer_rule_allows_real_deps
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2332 RuleClass.isExecutableStarlark
    pub fn is_executable_starlark(&self) -> bool {
        self.is_executable_starlark
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2337 RuleClass.isAnalysisTest
    pub fn is_analysis_test(&self) -> bool {
        self.is_analysis_test
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2345 RuleClass.hasAnalysisTestTransition
    pub fn has_analysis_test_transition(&self) -> bool {
        self.has_analysis_test_transition
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2350 RuleClass.getAllowlistCheckers
    pub fn get_allowlist_checkers(&self) -> &[AllowlistChecker] {
        &self.allowlist_checkers
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2360 RuleClass.ignoreLicenses
    pub fn ignore_licenses(&self) -> bool {
        self.attribute_provider.ignore_licenses()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2364 RuleClass.getToolchainTypes
    pub fn get_toolchain_types(&self) -> &IndexSet<ToolchainTypeRequirement> {
        &self.toolchain_types
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2368 RuleClass.useToolchainResolution
    pub fn use_toolchain_resolution(&self, rule: &Rule) -> bool {
        self.toolchain_resolution_mode
            .use_toolchain_resolution(rule)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2372 RuleClass.getExecutionPlatformConstraints
    pub fn get_execution_platform_constraints(&self) -> &IndexSet<Label> {
        &self.execution_platform_constraints
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2376 RuleClass.getDeclaredExecGroups
    pub fn get_declared_exec_groups(&self) -> &IndexMap<String, DeclaredExecGroup> {
        &self.declared_exec_groups
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2380 RuleClass.getAutoExecGroupsMode
    pub fn get_auto_exec_groups_mode(&self) -> AutoExecGroupsMode {
        self.auto_exec_groups_mode
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2384 RuleClass.getOutputFileKind
    pub fn get_output_file_kind(&self) -> OutputFileKind {
        self.output_file_kind
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2404 RuleClass.isPackageMetadataRule
    pub fn is_package_metadata_rule(&self) -> bool {
        is_package_metadata_rule_label(self.rule_definition_environment_label.as_ref())
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:2429 RuleClass.getSubrules
    pub fn get_subrules(&self) -> &[StarlarkCallable] {
        &self.subrules
    }
}

impl RuleClass {
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1796 RuleClass.<init>
    fn new_internal(b: &mut RuleClassBuilder, name: &str, key: &str) -> RuleClass {
        let attributes: Vec<Attribute> = b.attributes.values().cloned().collect();
        // Create the index and collect non-configurable attributes while doing some validation
        // checks.
        blaze_util::check_state!(
            !attributes.is_empty() && attributes[0] == *NAME_ATTRIBUTE,
            "Rule {} does not have name as its first attribute: {:?}",
            name,
            attributes
        );
        let mut attribute_index: HashMap<String, usize> = HashMap::with_capacity(attributes.len());
        let mut public_to_private_names: HashMap<String, &Attribute> =
            HashMap::with_capacity(attributes.len());
        let mut computed_has_aspects = false;
        let mut non_configurable_attributes: Vec<String> = Vec::new();
        for (i, attribute) in attributes.iter().enumerate() {
            let public_name = attribute.get_public_name();
            if let Some(conflicting) =
                public_to_private_names.insert(public_name.clone(), attribute)
            {
                panic!(
                    "IllegalStateException: Rule {}: Attributes {} and {} have an identical public \
                     name: {}",
                    name,
                    attribute.get_name(),
                    conflicting.get_name(),
                    public_name
                );
            }
            computed_has_aspects |= attribute.has_aspects();
            attribute_index.insert(attribute.get_name().to_owned(), i);
            if !attribute.is_configurable() {
                non_configurable_attributes.push(attribute.get_name().to_owned());
            }
        }
        let package_metadata_rule =
            is_package_metadata_rule_label(b.rule_definition_environment_label.as_ref());
        let attribute_provider = AttributeProvider::new(
            attributes.clone(),
            attribute_index,
            Some(non_configurable_attributes),
            name.to_owned(),
            b.ignore_licenses,
        )
        .with_package_metadata_rule(package_metadata_rule);
        let mut subrules: Vec<StarlarkCallable> = Vec::new();
        for s in &b.subrules {
            if !subrules.contains(s) {
                subrules.push(*s);
            }
        }
        RuleClass {
            rule_class_id: RuleClassId::create(name, key),
            callstack: b.callstack.clone(),
            target_kind: format!("{}{}", name, Rule::target_kind_suffix()),
            type_: b.type_,
            starlark_parent: b.starlark_parent.clone(),
            initializer: b.initializer,
            label_converter_for_initializer: b.label_converter_for_initializer.clone(),
            is_starlark: b.starlark,
            extendable: b.extendable.unwrap_or(false),
            starlark_extension_label: b.starlark_extension_label.clone(),
            starlark_documentation: b.starlark_documentation.clone(),
            extendable_allowlist: b.extendable_allowlist.clone(),
            starlark_testable: b.starlark_testable,
            documented: b.documented,
            outputs_to_bindir: b.outputs_to_bindir,
            dependency_resolution_rule: b.dependency_resolution_rule,
            is_materializer_rule: b.is_materializer_rule,
            materializer_rule_allows_real_deps: b.materializer_rule_allows_real_deps,
            is_executable_starlark: b.is_executable_starlark,
            is_analysis_test: b.is_analysis_test,
            has_analysis_test_transition: b.has_analysis_test_transition,
            allowlist_checkers: b.allowlist_checkers.clone(),
            has_aspects: computed_has_aspects,
            attribute_provider,
            implicit_outputs_function: b.implicit_outputs_function.clone(),
            transition_factory: b.transition_factory.clone(),
            configured_target_factory: b.configured_target_factory.clone(),
            advertised_providers: b.advertised_providers.build(),
            configured_target_function: b.configured_target_function,
            build_setting: b.build_setting.clone(),
            subrules,
            option_reference_function: b.option_reference_function.clone(),
            rule_definition_environment_label: b.rule_definition_environment_label.clone(),
            rule_definition_environment_digest: b.rule_definition_environment_digest.clone(),
            output_file_kind: b.output_file_kind,
            configuration_fragment_policy: b.configuration_fragment_policy.build(),
            supports_constraint_checking: b.supports_constraint_checking,
            toolchain_types: b.toolchain_types.clone(),
            toolchain_resolution_mode: b.toolchain_resolution_mode.clone(),
            execution_platform_constraints: b.execution_platform_constraints.clone(),
            declared_exec_groups: b.exec_groups.clone(),
            auto_exec_groups_mode: b.auto_exec_groups_mode,
        }
    }
}

// Java: com/google/devtools/build/lib/packages/RuleClass.java:2404 RuleClass.isPackageMetadataRule
fn is_package_metadata_rule_label(rule_definition_environment_label: Option<&Label>) -> bool {
    // If it was not defined in Starlark, it can not be a new style package metadata rule.
    let Some(label) = rule_definition_environment_label else {
        return false;
    };
    if label.get_repository().get_name() == "rules_license" {
        // For now, we treat all rules in rules_license as potenial metadate rules.
        return true;
    }
    // BEGIN-INTERNAL
    // TODO(aiuto): This is a Google-ism, remove from Bazel.
    let package_name = label.get_package_name();
    if package_name.starts_with("tools/build_defs/license")
        || package_name.starts_with("third_party/rules_license")
    {
        return true;
    }
    // END-INTERNAL
    false
}

// Java: NonconfigurableAttributeMapper.of(rule).get(name, Type.STRING)
fn string_attr(rule: &Rule, name: &str) -> String {
    match NonconfigurableAttributeMapper::of(rule).get(name, Type::String) {
        Some(AttrValue::String(s)) => s,
        other => panic!("ClassCastException: {name} value {other:?}"),
    }
}

/// Errors of [`RuleClass::create_rule`] (Java: `LabelSyntaxException`,
/// `CannotPrecomputeDefaultsException`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum RuleClassCreateRuleError {
    #[error("{0}")]
    LabelSyntax(#[from] LabelSyntaxException),
    #[error("{0}")]
    CannotPrecomputeDefaults(#[from] CannotPrecomputeDefaultsException),
}

/// Builder for [`RuleClass`] (Java `RuleClass.Builder`).
// Java: com/google/devtools/build/lib/packages/RuleClass.java:333 RuleClass.Builder
pub struct RuleClassBuilder {
    name: String,
    callstack: Vec<StackEntry>,
    type_: RuleClassType,
    starlark_parent: Option<Arc<RuleClass>>,
    initializer: Option<StarlarkCallable>,
    label_converter_for_initializer: Option<Arc<LabelConverter>>,
    /// `None` means the default allowlist applies when the rule is extendable.
    extendable: Option<bool>,
    extendable_allowlist: Option<Label>,
    default_extendable_allowlist: Option<Label>,
    starlark: bool,
    starlark_testable: bool,
    documented: bool,
    outputs_to_bindir: bool,
    dependency_resolution_rule: bool,
    is_materializer_rule: bool,
    materializer_rule_allows_real_deps: bool,
    is_executable_starlark: bool,
    is_analysis_test: bool,
    has_analysis_test_transition: bool,
    allowlist_checkers: Vec<AllowlistChecker>,
    ignore_licenses: bool,
    implicit_outputs_function: ImplicitOutputsFunction,
    transition_factory: TransitionFactory,
    configured_target_factory: Option<String>,
    advertised_providers: AdvertisedProviderSetBuilder,
    configured_target_function: Option<StarlarkCallable>,
    build_setting: Option<BuildSetting>,
    subrules: Vec<StarlarkCallable>,
    /// Rule attribute name to user-defined name for subrule attributes.
    subrule_attribute_names: IndexMap<String, String>,
    option_reference_function: Option<OptionReferenceFunction>,
    rule_definition_environment_label: Option<Label>,
    rule_definition_environment_digest: Option<Vec<u8>>,
    starlark_extension_label: Option<Label>,
    starlark_documentation: Option<String>,
    configuration_fragment_policy: ConfigurationFragmentPolicyBuilder,
    supports_constraint_checking: bool,
    attributes: IndexMap<String, Attribute>,
    toolchain_types: IndexSet<ToolchainTypeRequirement>,
    toolchain_resolution_mode: ToolchainResolutionMode,
    execution_platform_constraints: IndexSet<Label>,
    output_file_kind: OutputFileKind,
    exec_groups: IndexMap<String, DeclaredExecGroup>,
    auto_exec_groups_mode: AutoExecGroupsMode,
}

impl RuleClassBuilder {
    /// A builder with all attributes of `parents` (several native abstract rules or a single
    /// Starlark rule).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:760 RuleClass.Builder.<init>
    pub fn new(
        name: &str,
        type_: RuleClassType,
        starlark: bool,
        parents: &[Arc<RuleClass>],
    ) -> Self {
        blaze_util::check_argument!(
            (parents.len() == 1 && parents[0].is_starlark())
                || parents.iter().all(|rule| !rule.is_starlark())
        );
        blaze_util::check_state!(starlark || type_ != RuleClassType::Placeholder, "{}", name);
        let mut b = RuleClassBuilder {
            name: name.to_owned(),
            callstack: Vec::new(),
            type_,
            starlark_parent: None,
            initializer: None,
            label_converter_for_initializer: None,
            extendable: None,
            extendable_allowlist: None,
            default_extendable_allowlist: None,
            starlark,
            starlark_testable: false,
            documented: type_ != RuleClassType::Abstract,
            outputs_to_bindir: true,
            dependency_resolution_rule: false,
            is_materializer_rule: false,
            materializer_rule_allows_real_deps: false,
            is_executable_starlark: false,
            is_analysis_test: false,
            has_analysis_test_transition: false,
            allowlist_checkers: Vec::new(),
            ignore_licenses: false,
            implicit_outputs_function: ImplicitOutputsFunction::None,
            transition_factory: TransitionFactory::NoTransition,
            configured_target_factory: None,
            advertised_providers: AdvertisedProviderSet::builder(),
            configured_target_function: None,
            build_setting: None,
            subrules: Vec::new(),
            subrule_attribute_names: IndexMap::new(),
            option_reference_function: None,
            rule_definition_environment_label: None,
            rule_definition_environment_digest: None,
            starlark_extension_label: None,
            starlark_documentation: None,
            configuration_fragment_policy: ConfigurationFragmentPolicyBuilder::new(),
            supports_constraint_checking: true,
            attributes: IndexMap::new(),
            toolchain_types: IndexSet::new(),
            toolchain_resolution_mode: ToolchainResolutionMode::Enabled,
            execution_platform_constraints: IndexSet::new(),
            output_file_kind: OutputFileKind::File,
            exec_groups: IndexMap::new(),
            auto_exec_groups_mode: AutoExecGroupsMode::Dynamic,
        };
        b.add_attribute(NAME_ATTRIBUTE.clone());
        if parents.len() == 1
            && parents[0].is_starlark()
            && parents[0].get_rule_class_type() != RuleClassType::Abstract
        {
            // the condition removes {@link StarlarkRuleClassFunctions.baseRule} and
            // binaryBaseRule, which are marked as Starlark (because of Stardoc) && abstract at the
            // same time
            b.starlark_parent = Some(parents[0].clone());
            blaze_util::check_argument!(parents[0].is_extendable());
        }

        for parent in parents {
            if parent.is_materializer_rule() {
                b.is_materializer_rule = true;
                if parent.materializer_rule_allows_real_deps {
                    b.materializer_rule_allows_real_deps = true;
                }
            } else if b.is_materializer_rule {
                panic!(
                    "IllegalArgumentException: Inconsistent value of isMaterializerRule among \
                     parents"
                );
            }

            if parent.dependency_resolution_rule {
                b.dependency_resolution_rule = true;
            } else if b.dependency_resolution_rule {
                panic!(
                    "IllegalArgumentException: Inconsistent value of dependencyResolutionRule \
                     among parents"
                );
            }

            b.configuration_fragment_policy
                .include_configuration_fragments_from(parent.get_configuration_fragment_policy());
            b.supports_constraint_checking = parent.supports_constraint_checking;

            b.add_toolchain_types(parent.get_toolchain_types().iter().cloned());
            b.add_execution_platform_constraints(
                parent.get_execution_platform_constraints().iter().cloned(),
            );
            if let Err(duplicate) =
                b.add_exec_groups_checked(parent.get_declared_exec_groups(), false)
            {
                panic!(
                    "IllegalArgumentException: An execution group named '{duplicate}' is \
                     inherited multiple times with different requirements in {name} ruleclass"
                );
            }

            b.auto_exec_groups_mode = parent.get_auto_exec_groups_mode();

            for attribute in parent.get_attribute_provider().get_attributes() {
                let attr_name = attribute.get_name();
                blaze_util::check_argument!(
                    !b.attributes.contains_key(attr_name) || b.attributes[attr_name] == *attribute,
                    "Attribute {} is inherited multiple times in {} ruleclass",
                    attr_name,
                    name
                );
                b.attributes.insert(attr_name.to_owned(), attribute.clone());
            }

            b.allowlist_checkers
                .extend(parent.get_allowlist_checkers().iter().cloned());

            b.advertised_providers
                .add_parent(parent.get_advertised_providers());

            if !parent.get_default_implicit_outputs_function().is_none() {
                if !b.implicit_outputs_function.is_none() {
                    panic!(
                        "IllegalArgumentException: Only a single parent may set implicit outputs"
                    );
                }
                b.implicit_outputs_function =
                    parent.get_default_implicit_outputs_function().clone();
            }
        }
        // TODO(bazel-team): move this testonly attribute setting to somewhere else
        // preferably to some base RuleClass implementation.
        if b.type_ == RuleClassType::Test {
            let mut test_only_attr = attr("testonly", Type::Boolean);
            test_only_attr
                .value(Some(AttrValue::Bool(true)))
                .nonconfigurable("policy decision: this shouldn't depend on the configuration");
            if b.attributes.contains_key("testonly") {
                b.override_(&test_only_attr);
            } else {
                b.add(&test_only_attr);
            }
        }
        b
    }

    /// Builds a Starlark rule class defined in `starlark_extension_label`; its key is
    /// `<label>%<name>`.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:864 RuleClass.Builder.buildStarlark
    pub fn build_starlark(&mut self, name: &str, starlark_extension_label: Label) -> RuleClass {
        blaze_util::check_state!(self.starlark);
        let key = format!("{starlark_extension_label}%{name}");
        self.starlark_extension_label = Some(starlark_extension_label);
        self.build_internal(name, &key)
    }

    /// Builds a native rule class (key = name).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:876 RuleClass.Builder.build
    pub fn build(&mut self) -> RuleClass {
        // For built-ins, name == key
        let name = self.name.clone();
        self.build_internal(&name, &name)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:880 RuleClass.Builder.build
    fn build_internal(&mut self, name: &str, key: &str) -> RuleClass {
        blaze_util::check_argument!(self.name.is_empty() || self.name == name);
        self.type_.check_name(name);

        self.check_attributes(name);

        blaze_util::check_state!(
            (self.type_ == RuleClassType::Abstract)
                == (self.configured_target_factory.is_none()
                    && self.configured_target_function.is_none()),
            "Bad combo for {}: {:?} {:?} {:?}",
            name,
            self.type_,
            self.configured_target_factory,
            self.configured_target_function
        );
        if self.starlark {
            self.assert_starlark_rule_class_has_implementation_function();
            self.assert_starlark_rule_class_has_environment_label();
        }
        if self.type_ == RuleClassType::Placeholder
            && self.rule_definition_environment_digest.is_none()
        {
            panic!("NullPointerException: {}", self.name);
        }

        if let Some(build_setting) = &self.build_setting {
            let type_ = build_setting.get_type();
            let mut default_attr_builder = attr(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, type_);
            default_attr_builder
                .nonconfigurable(BUILD_SETTING_DEFAULT_NONCONFIGURABLE)
                .mandatory();
            self.add(&default_attr_builder);

            let mut help = attr(STARLARK_BUILD_SETTING_HELP_ATTR_NAME, Type::String);
            help.nonconfigurable(BUILD_SETTING_DEFAULT_NONCONFIGURABLE);
            self.add(&help);

            // Build setting rules should opt out of toolchain resolution, since they form part of
            // the configuration.
            self.toolchain_resolution_mode(ToolchainResolutionMode::Disabled);
        }

        if self.starlark
            && (self.type_ == RuleClassType::Normal || self.type_ == RuleClassType::Test)
            && self.outputs_to_bindir
            && !self.starlark_testable
            && !self.is_analysis_test
            && self.build_setting.is_none()
        {
            if self.extendable.is_none() {
                // The rule can be extended, use fallback
                self.extendable = Some(true);
                self.extendable_allowlist = self.default_extendable_allowlist.clone();
            }
        } else {
            // This kind of rule can't be extended
            if self.extendable == Some(true) || self.extendable_allowlist.is_some() {
                panic!("IllegalArgumentException: The rule cannot be extended");
            }
            self.extendable = Some(false);
        }

        RuleClass::new_internal(self, name, key)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:984 RuleClass.Builder.checkAttributes
    fn check_attributes(&self, rule_class_name: &str) {
        blaze_util::check_argument!(
            self.attributes.len() <= MAX_ATTRIBUTES,
            "Rule class {} declared too many attributes ({} > {})",
            rule_class_name,
            self.attributes.len(),
            MAX_ATTRIBUTES
        );

        let mut attributes_not_for_dependency_resolution: Vec<&str> = Vec::new();
        for (attribute_name, attribute) in &self.attributes {
            // Java: StarlarkSubruleApi.getUserDefinedNameIfSubruleAttr
            let attribute_name_length = self
                .subrule_attribute_names
                .get(attribute_name)
                .map(|n| n.chars().count())
                .unwrap_or_else(|| attribute_name.chars().count());

            // TODO(b/151171037): This check would make more sense at Attribute creation time,
            // but the use of unchecked exceptions in these APIs makes it brittle.
            blaze_util::check_argument!(
                attribute_name_length <= MAX_ATTRIBUTE_NAME_LENGTH,
                "Attribute {}.{}'s name is too long ({} > {})",
                rule_class_name,
                attribute_name,
                attribute_name_length,
                MAX_ATTRIBUTE_NAME_LENGTH
            );

            if self.dependency_resolution_rule
                && attribute.get_type().get_label_class() == LabelClass::Dependency
                && !attribute.is_for_dependency_resolution()
            {
                attributes_not_for_dependency_resolution.push(attribute_name);
            }
        }

        if !attributes_not_for_dependency_resolution.is_empty() {
            panic!(
                "IllegalStateException: Rule is available for dependency resolution but some \
                 dependency attributes aren't: {}",
                attributes_not_for_dependency_resolution.join(", ")
            );
        }

        self.type_.check_attributes(&self.attributes);
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1033 RuleClass.Builder.assertStarlarkRuleClassHasImplementationFunction
    fn assert_starlark_rule_class_has_implementation_function(&self) {
        blaze_util::check_state!(
            (self.type_ == RuleClassType::Normal || self.type_ == RuleClassType::Test)
                == self.configured_target_function.is_some(),
            "{:?} {:?}",
            self.type_,
            self.configured_target_function
        );
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1042 RuleClass.Builder.assertStarlarkRuleClassHasEnvironmentLabel
    fn assert_starlark_rule_class_has_environment_label(&self) {
        blaze_util::check_state!(
            (self.type_ == RuleClassType::Normal
                || self.type_ == RuleClassType::Test
                || self.type_ == RuleClassType::Placeholder)
                == self.rule_definition_environment_label.is_some(),
            "Concrete Starlark rule classes can't have null labels: {:?} {:?}",
            self.rule_definition_environment_label,
            self.type_
        );
    }

    /// The user-defined names of subrule attributes, by rule attribute name (Java
    /// `StarlarkSubruleApi.getUserDefinedNameIfSubruleAttr` over the subrules).
    pub fn set_subrule_attribute_names(&mut self, names: IndexMap<String, String>) -> &mut Self {
        self.subrule_attribute_names = names;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1054 RuleClass.Builder.initializer
    pub fn initializer(
        &mut self,
        initializer: StarlarkCallable,
        label_converter: Arc<LabelConverter>,
    ) -> &mut Self {
        self.initializer = Some(initializer);
        self.label_converter_for_initializer = Some(label_converter);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1061 RuleClass.Builder.setExtendableByAllowlist
    pub fn set_extendable_by_allowlist(&mut self, extendable_allowlist: Label) {
        self.extendable = Some(true);
        self.extendable_allowlist = Some(extendable_allowlist);
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1067 RuleClass.Builder.setExtendable
    pub fn set_extendable(&mut self, extendable: bool) {
        self.extendable = Some(extendable);
        self.extendable_allowlist = None;
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1076 RuleClass.Builder.setDefaultExtendableAllowlist
    pub fn set_default_extendable_allowlist(&mut self, extendable_allowlist: Label) {
        self.default_extendable_allowlist = Some(extendable_allowlist);
    }

    /// Fragments are named by their Java class simple name.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1087 RuleClass.Builder.requiresConfigurationFragments
    pub fn requires_configuration_fragments(
        &mut self,
        configuration_fragments: &[&str],
    ) -> &mut Self {
        self.configuration_fragment_policy
            .requires_configuration_fragments(configuration_fragments);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1102 RuleClass.Builder.requiresConfigurationFragmentsByStarlarkModuleName
    pub fn requires_configuration_fragments_by_starlark_module_name(
        &mut self,
        configuration_fragment_names: &[String],
    ) -> &mut Self {
        self.configuration_fragment_policy
            .requires_configuration_fragments_by_starlark_builtin_name(
                configuration_fragment_names,
            );
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1111 RuleClass.Builder.setCallStack
    pub fn set_call_stack(&mut self, callstack: Vec<StackEntry>) -> &mut Self {
        self.callstack = callstack;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1117 RuleClass.Builder.setStarlarkTestable
    pub fn set_starlark_testable(&mut self) -> &mut Self {
        blaze_util::check_state!(
            self.starlark,
            "Cannot set starlarkTestable on a non-Starlark rule"
        );
        self.starlark_testable = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1128 RuleClass.Builder.setMissingFragmentPolicy
    pub fn set_missing_fragment_policy(
        &mut self,
        fragment_class: &str,
        missing_fragment_policy: MissingFragmentPolicy,
    ) -> &mut Self {
        self.configuration_fragment_policy
            .set_missing_fragment_policy(fragment_class, missing_fragment_policy);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1135 RuleClass.Builder.setUndocumented
    pub fn set_undocumented(&mut self) -> &mut Self {
        self.documented = false;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1150 RuleClass.Builder.setOutputToGenfiles
    pub fn set_output_to_genfiles(&mut self) -> &mut Self {
        blaze_util::check_state!(
            self.type_ != RuleClassType::Abstract,
            "Setting not inherited property (output to genrules) of abstract rule class '{}'",
            self.name
        );
        self.outputs_to_bindir = false;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1169 RuleClass.Builder.setImplicitOutputsFunction
    pub fn set_implicit_outputs_function(
        &mut self,
        implicit_outputs_function: ImplicitOutputsFunction,
    ) -> &mut Self {
        blaze_util::check_state!(
            self.type_ != RuleClassType::Abstract,
            "Setting not inherited property (implicit output function) of abstract rule class '{}'",
            self.name
        );
        self.implicit_outputs_function = implicit_outputs_function;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1180 RuleClass.Builder.cfg
    pub fn cfg(&mut self, transition_factory: TransitionFactory) -> &mut Self {
        blaze_util::check_state!(
            self.type_ != RuleClassType::Abstract,
            "Setting not inherited property (cfg) of abstract rule class '{}'",
            self.name
        );
        blaze_util::check_state!(
            self.transition_factory.is_no_transition(),
            "Property cfg has already been set"
        );
        // PORT: transitionType().isCompatibleWith(TransitionType.RULE) is not modelled.
        blaze_util::check_argument!(!transition_factory.is_split());
        self.transition_factory = transition_factory;
        self
    }

    /// Java `factory(ConfiguredTargetFactory)`; the factory's Java class simple name.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1196 RuleClass.Builder.factory
    pub fn factory(&mut self, factory: &str) -> &mut Self {
        self.configured_target_factory = Some(factory.to_owned());
        self
    }

    /// Builtin providers by Java class simple name.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1212 RuleClass.Builder.advertiseProvider
    pub fn advertise_provider(&mut self, providers: &[&str]) -> &mut Self {
        for p in providers {
            self.advertised_providers.add_builtin(p);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1220 RuleClass.Builder.advertiseStarlarkProvider
    pub fn advertise_starlark_provider(
        &mut self,
        starlark_providers: Vec<StarlarkProviderIdentifier>,
    ) -> &mut Self {
        for p in starlark_providers {
            self.advertised_providers.add_starlark(p);
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1232 RuleClass.Builder.canHaveAnyProvider
    pub fn can_have_any_provider(&mut self) -> &mut Self {
        self.advertised_providers.can_have_any_provider();
        self
    }

    /// Adds an attribute; panics if one of that name exists.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1245 RuleClass.Builder.addAttribute
    pub fn add_attribute(&mut self, attribute: Attribute) -> &mut Self {
        if self.attributes.contains_key(attribute.get_name()) {
            panic!(
                "IllegalStateException: There is already a built-in attribute '{}' which cannot \
                 be overridden.",
                attribute.get_name()
            );
        }
        self.attributes
            .insert(attribute.get_name().to_owned(), attribute);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1256 RuleClass.Builder.overrideAttribute
    fn override_attribute(&mut self, attribute: Attribute) {
        let attr_name = attribute.get_name().to_owned();
        blaze_util::check_state!(
            self.attributes.contains_key(&attr_name),
            "No such attribute '{}' to override in ruleclass '{}'.",
            attr_name,
            self.name
        );
        let orig_type = self.attributes[&attr_name].get_type();
        let new_type = attribute.get_type();
        blaze_util::check_state!(
            orig_type == new_type,
            "The type of the new attribute '{}' is different from the original one '{}'.",
            new_type,
            orig_type
        );
        self.attributes.insert(attr_name, attribute);
    }

    /// Builds and adds an attribute (Java `add(Attribute.Builder)`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1283 RuleClass.Builder.add
    pub fn add(&mut self, attr: &AttributeBuilder) -> &mut Self {
        self.add_attribute(attr.build())
    }

    /// Overrides an inherited attribute in a Starlark rule extension (Java
    /// `override(Attribute)`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1301 RuleClass.Builder.override
    pub fn override_starlark(&mut self, attr: &Attribute) -> Result<&mut Self, EvalException> {
        // Java: com/google/devtools/build/lib/packages/RuleClass.java:1294 RuleClass.Builder.failIf
        fn fail_if(condition: bool, message: String) -> Result<(), EvalException> {
            if condition {
                return Err(EvalException::new(message));
            }
            Ok(())
        }
        let parent_attr = self
            .attributes
            .get(attr.get_name())
            .expect("NullPointerException: parent attribute")
            .clone();
        let public_name = parent_attr.get_public_name();
        fail_if(
            !parent_attr.starlark_defined(),
            format!("attribute `{public_name}`: built-in attributes cannot be overridden."),
        )?;
        fail_if(
            !parent_attr.is_public(),
            format!("attribute `{public_name}`: private attributes cannot be overridden."),
        )?;
        fail_if(
            parent_attr.get_type() != Type::LabelList && parent_attr.get_type() != Type::Label,
            format!("attribute `{public_name}`: Only label types maybe be overridden."),
        )?;
        fail_if(
            parent_attr.get_type() != attr.get_type(),
            format!("attribute `{public_name}`: Types of parent and child's attributes mismatch."),
        )?;
        attr.fail_if_not_a_valid_override()?;

        let mut attr_builder = self.copy(attr.get_name());
        if let Some(default) = attr.get_default_value_unchecked() {
            // PORT: Java re-converts the (already converted) default with defaultValue(Object),
            // an identity for label and label-list values.
            attr_builder.value(Some(default.clone()));
        }
        attr_builder.add_aspects(attr.get_aspects_list())?;
        self.override_(&attr_builder);
        Ok(self)
    }

    /// Overrides an inherited attribute (Java `override(Attribute.Builder)`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1336 RuleClass.Builder.override
    pub fn override_(&mut self, attr: &AttributeBuilder) -> &mut Self {
        self.override_attribute(attr.build());
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1342 RuleClass.Builder.contains
    pub fn contains(&self, name: &str) -> bool {
        self.attributes.contains_key(name)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1346 RuleClass.Builder.getAttribute
    pub fn get_attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.get(name)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1351 RuleClass.Builder.getAttributes
    pub fn get_attributes(&self) -> Vec<Attribute> {
        self.attributes.values().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1357 RuleClass.Builder.setConfiguredTargetFunction
    pub fn set_configured_target_function(&mut self, func: StarlarkCallable) -> &mut Self {
        self.configured_target_function = Some(func);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1363 RuleClass.Builder.setBuildSetting
    pub fn set_build_setting(&mut self, build_setting: BuildSetting) -> &mut Self {
        self.build_setting = Some(build_setting);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1369 RuleClass.Builder.setSubrules
    pub fn set_subrules(&mut self, subrules: Vec<StarlarkCallable>) -> &mut Self {
        self.subrules = subrules;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1374 RuleClass.Builder.getSubrules
    pub fn get_subrules(&self) -> &[StarlarkCallable] {
        &self.subrules
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1378 RuleClass.Builder.getParentSubrules
    pub fn get_parent_subrules(&self) -> Vec<StarlarkCallable> {
        let mut builder: Vec<StarlarkCallable> = Vec::new();
        let mut current_parent = self.starlark_parent.clone();
        while let Some(parent) = current_parent {
            // Java adds the direct parent's subrules on every iteration.
            for s in self.starlark_parent.as_ref().unwrap().get_subrules() {
                if !builder.contains(s) {
                    builder.push(*s);
                }
            }
            current_parent = parent.starlark_parent.clone();
        }
        builder
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1392 RuleClass.Builder.setRuleDefinitionEnvironmentLabelAndDigest
    pub fn set_rule_definition_environment_label_and_digest(
        &mut self,
        label: Label,
        digest: Vec<u8>,
    ) -> &mut Self {
        self.rule_definition_environment_label = Some(label);
        self.rule_definition_environment_digest = Some(digest);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1403 RuleClass.Builder.setStarlarkDocumentation
    pub fn set_starlark_documentation(&mut self, starlark_documentation: String) -> &mut Self {
        blaze_util::check_state!(self.starlark, "{}", self.name);
        self.starlark_documentation = Some(starlark_documentation);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1414 RuleClass.Builder.getStarlarkDocumentation
    pub fn get_starlark_documentation(&self) -> Option<&str> {
        self.starlark_documentation.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1418 RuleClass.Builder.getRuleDefinitionEnvironmentLabel
    pub fn get_rule_definition_environment_label(&self) -> Option<&Label> {
        self.rule_definition_environment_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1428 RuleClass.Builder.removeAttribute
    pub fn remove_attribute(&mut self, name: &str) -> &mut Self {
        blaze_util::check_state!(
            self.attributes.contains_key(name),
            "No such attribute '{}' to remove.",
            name
        );
        self.attributes.shift_remove(name);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1440 RuleClass.Builder.setDependencyResolutionRule
    pub fn set_dependency_resolution_rule(&mut self) -> &mut Self {
        self.dependency_resolution_rule = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1447 RuleClass.Builder.setIsMaterializerRule
    pub fn set_is_materializer_rule(&mut self, is_materializer_rule: bool) -> &mut Self {
        self.is_materializer_rule = is_materializer_rule;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1454 RuleClass.Builder.setMaterializerRuleAllowsRealDeps
    pub fn set_materializer_rule_allows_real_deps(&mut self, allow_real_deps: bool) -> &mut Self {
        self.materializer_rule_allows_real_deps = allow_real_deps;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1464 RuleClass.Builder.setExecutableStarlark
    pub fn set_executable_starlark(&mut self) -> &mut Self {
        self.is_executable_starlark = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1471 RuleClass.Builder.setIsAnalysisTest
    pub fn set_is_analysis_test(&mut self) -> &mut Self {
        self.is_analysis_test = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1476 RuleClass.Builder.isAnalysisTest
    pub fn is_analysis_test(&self) -> bool {
        self.is_analysis_test
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1485 RuleClass.Builder.setHasAnalysisTestTransition
    pub fn set_has_analysis_test_transition(&mut self) -> &mut Self {
        self.has_analysis_test_transition = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1492 RuleClass.Builder.addAllowlistChecker
    pub fn add_allowlist_checker(&mut self, allowlist_checker: AllowlistChecker) -> &mut Self {
        self.allowlist_checkers.push(allowlist_checker);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1503 RuleClass.Builder.setIgnoreLicenses
    pub fn set_ignore_licenses(&mut self) -> &mut Self {
        self.ignore_licenses = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1508 RuleClass.Builder.getType
    pub fn get_type(&self) -> RuleClassType {
        self.type_
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1517 RuleClass.Builder.setOutputFileKind
    pub fn set_output_file_kind(&mut self, output_file_kind: OutputFileKind) -> &mut Self {
        self.output_file_kind = output_file_kind;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1529 RuleClass.Builder.compatibleWith
    pub fn compatible_with(&mut self, environments: &[Label]) -> &mut Self {
        let mut a = attr(DEFAULT_COMPATIBLE_ENVIRONMENT_ATTR, Type::LabelList);
        a.value(Some(AttrValue::LabelList(environments.to_vec())));
        self.add(&a)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1545 RuleClass.Builder.restrictedTo
    pub fn restricted_to(
        &mut self,
        first_environment: Label,
        other_environments: &[Label],
    ) -> &mut Self {
        let mut environments = vec![first_environment];
        environments.extend(other_environments.iter().cloned());
        let mut a = attr(DEFAULT_RESTRICTED_ENVIRONMENT_ATTR, Type::LabelList);
        a.value(Some(AttrValue::LabelList(environments)));
        self.add(&a)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1560 RuleClass.Builder.exemptFromConstraintChecking
    pub fn exempt_from_constraint_checking(&mut self, reason: &str) -> &mut Self {
        let _ = reason;
        blaze_util::check_state!(self.supports_constraint_checking);
        self.supports_constraint_checking = false;
        self.attributes.shift_remove(COMPATIBLE_ENVIRONMENT_ATTR);
        self.attributes.shift_remove(RESTRICTED_ENVIRONMENT_ATTR);
        self.attributes.shift_remove(TARGET_COMPATIBLE_WITH_ATTR);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1576 RuleClass.Builder.setOptionReferenceFunctionForConfigSettingOnly
    pub fn set_option_reference_function_for_config_setting_only(
        &mut self,
        option_reference_function: OptionReferenceFunction,
    ) -> &mut Self {
        self.option_reference_function = Some(option_reference_function);
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1587 RuleClass.Builder.addToolchainTypes
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1596 RuleClass.Builder.addToolchainTypes
    pub fn add_toolchain_types(
        &mut self,
        toolchain_types: impl IntoIterator<Item = ToolchainTypeRequirement>,
    ) -> &mut Self {
        self.toolchain_types.extend(toolchain_types);
        self
    }

    /// Adds exec groups; with `override_`, replaces groups of the same name, else panics on a
    /// conflicting duplicate (Java `DuplicateExecGroupError`).
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1606 RuleClass.Builder.addExecGroups
    pub fn add_exec_groups(
        &mut self,
        exec_groups: &IndexMap<String, DeclaredExecGroup>,
        override_: bool,
    ) -> &mut Self {
        if let Err(duplicate) = self.add_exec_groups_checked(exec_groups, override_) {
            // Java: com/google/devtools/build/lib/packages/RuleClass.java:1625 RuleClass.Builder.DuplicateExecGroupError
            // Java: com/google/devtools/build/lib/packages/RuleClass.java:1633 RuleClass.Builder.DuplicateExecGroupError.getDuplicateGroup
            panic!(
                "DuplicateExecGroupError: Multiple execution groups with the same name: \
                 '{duplicate}'."
            );
        }
        self
    }

    /// [`RuleClassBuilder::add_exec_groups`] returning the duplicate group's name instead of
    /// throwing Java's `DuplicateExecGroupError`.
    pub fn add_exec_groups_checked(
        &mut self,
        exec_groups: &IndexMap<String, DeclaredExecGroup>,
        override_: bool,
    ) -> Result<(), String> {
        for (name, group) in exec_groups {
            if self.exec_groups.contains_key(name) && !override_ {
                // If trying to add a new execution group with the same name as a execution group
                // that already exists, check if they are equivalent and error out if not.
                if self.exec_groups[name] != *group {
                    return Err(name.clone());
                }
            } else {
                self.exec_groups.insert(name.clone(), group.clone());
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1639 RuleClass.Builder.hasExecGroup
    pub fn has_exec_group(&self, name: &str) -> bool {
        self.exec_groups.contains_key(name)
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1645 RuleClass.Builder.autoExecGroupsMode
    pub fn auto_exec_groups_mode(
        &mut self,
        auto_exec_groups_mode: AutoExecGroupsMode,
    ) -> &mut Self {
        self.auto_exec_groups_mode = auto_exec_groups_mode;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1656 RuleClass.Builder.toolchainResolutionMode
    pub fn toolchain_resolution_mode(&mut self, mode: ToolchainResolutionMode) -> &mut Self {
        self.toolchain_resolution_mode = mode;
        self
    }

    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1676 RuleClass.Builder.addExecutionPlatformConstraints
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1666 RuleClass.Builder.addExecutionPlatformConstraints
    pub fn add_execution_platform_constraints(
        &mut self,
        constraints: impl IntoIterator<Item = Label>,
    ) -> &mut Self {
        self.execution_platform_constraints.extend(constraints);
        self
    }

    /// A builder replicating the parent's attribute `name`.
    // Java: com/google/devtools/build/lib/packages/RuleClass.java:1687 RuleClass.Builder.copy
    pub fn copy(&self, name: &str) -> AttributeBuilder {
        blaze_util::check_argument!(
            self.attributes.contains_key(name),
            "Attribute {} does not exist in parent rule class.",
            name
        );
        self.attributes[name].clone_builder()
    }
}
