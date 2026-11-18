//! Port of `analysis/BaseRuleClasses.java`: the attributes shared by (almost) every rule, the
//! abstract native base rules, and `EmptyRule`.
//!
//! `common_core_and_starlark_attributes`, `exec_properties_attribute`, the computed defaults,
//! `TIMEOUT_DEFAULT`, the coverage/run-under late-bound defaults and
//! `get_test_runtime_label_list` are shared with the Starlark base rules of
//! `StarlarkRuleClassFunctions` (agent B, `starlark_defs/`), which call them from here.

use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::Label;
use indexmap::IndexMap;

use crate::attribute::AttributeMap;
use crate::attribute::ComputedDefault;
use crate::attribute::LateBoundDefault;
use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::constraint_value_info_id;
use crate::native_rules::exec_transition_factory;
use crate::native_rules::template_variable_info_id;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::rule_class::rule_class::APPLICABLE_METADATA_ATTR;
use crate::rule_class::rule_class::COMPATIBLE_ENVIRONMENT_ATTR;
use crate::rule_class::rule_class::CONFIG_SETTING_DEPS_ATTRIBUTE;
use crate::rule_class::rule_class::EXEC_COMPATIBLE_WITH_ATTR;
use crate::rule_class::rule_class::EXEC_GROUP_COMPATIBLE_WITH_ATTR;
use crate::rule_class::rule_class::EXEC_PROPERTIES_ATTR;
use crate::rule_class::rule_class::RESTRICTED_ENVIRONMENT_ATTR;
use crate::rule_class::rule_class::TARGET_COMPATIBLE_WITH_ATTR;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::Type;

/// Java `ConstraintConstants.ENVIRONMENT_RULE`.
// Java: com/google/devtools/build/lib/analysis/constraints/ConstraintConstants.java:28 ConstraintConstants.ENVIRONMENT_RULE
pub const ENVIRONMENT_RULE: &str = "environment";

/// Java `BaseRuleClasses.testonlyDefault`: the package's `default_testonly`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:68 BaseRuleClasses.testonlyDefault
pub static TESTONLY_DEFAULT: LazyLock<Arc<ComputedDefault>> = LazyLock::new(|| {
    Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|rule: &dyn AttributeMap| {
            rule.get_package_args()
                .default_test_only()
                .map(AttrValue::Bool)
        }),
    ))
});

/// Java `BaseRuleClasses.deprecationDefault`: the package's `default_deprecation`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:77 BaseRuleClasses.deprecationDefault
pub static DEPRECATION_DEFAULT: LazyLock<Arc<ComputedDefault>> = LazyLock::new(|| {
    Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|rule: &dyn AttributeMap| {
            rule.get_package_args()
                .default_deprecation()
                .map(|d| AttrValue::String(d.to_owned()))
        }),
    ))
});

/// Java `BaseRuleClasses.TIMEOUT_DEFAULT`: the default timeout of the test's `size`, or
/// `"illegal"`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:86 BaseRuleClasses.TIMEOUT_DEFAULT
pub static TIMEOUT_DEFAULT: LazyLock<Arc<ComputedDefault>> = LazyLock::new(|| {
    Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|rule: &dyn AttributeMap| {
            let size = rule.get("size", Type::String);
            let size = size.as_ref().and_then(AttrValue::as_str).unwrap_or("");
            if let Some(timeout) = test_size_default_timeout(size) {
                return Some(AttrValue::String(timeout.to_owned()));
            }
            Some(AttrValue::String("illegal".to_owned()))
        }),
    ))
});

/// `TestSize.getTestSize(attr).getDefaultTimeout().toString()`.
// PORT: move to packages/TestSize + TestTimeout when ported (no owner yet).
// Java: com/google/devtools/build/lib/packages/TestSize.java:70 TestSize.getTestSize
fn test_size_default_timeout(attr: &str) -> Option<&'static str> {
    if attr != attr.to_lowercase() {
        return None;
    }
    // Java: com/google/devtools/build/lib/packages/TestSize.java:28 TestSize (SMALL(SHORT),
    // MEDIUM(MODERATE), LARGE(LONG), ENORMOUS(ETERNAL)); TestTimeout.toString is lower case.
    match attr {
        "small" => Some("short"),
        "medium" => Some("moderate"),
        "large" => Some("long"),
        "enormous" => Some("eternal"),
        _ => None,
    }
}

/// Java `BaseRuleClasses.packageMetadataDefault`: the package's `default_package_metadata`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:102 BaseRuleClasses.packageMetadataDefault
pub static PACKAGE_METADATA_DEFAULT: LazyLock<Arc<ComputedDefault>> = LazyLock::new(|| {
    Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|rule: &dyn AttributeMap| {
            rule.get_package_args()
                .default_package_metadata()
                .map(|l| AttrValue::LabelList(l.to_vec()))
        }),
    ))
});

/// Java `BaseRuleClasses.ACTION_LISTENER` (resolved from the configuration's action listeners
/// during analysis).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:120 BaseRuleClasses.ACTION_LISTENER
pub static ACTION_LISTENER: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    Arc::new(LateBoundDefault::label_list(Some(
        "BuildConfigurationValue",
    )))
});

/// Java `BaseRuleClasses.DEFAULT_COVERAGE_SUPPORT_VALUE`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:125 BaseRuleClasses.DEFAULT_COVERAGE_SUPPORT_VALUE
pub const DEFAULT_COVERAGE_SUPPORT_VALUE: &str = "//tools/test:coverage_support";

/// Java `BaseRuleClasses.coverageSupportAttribute(defaultValue)`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:131 BaseRuleClasses.coverageSupportAttribute
pub fn coverage_support_attribute(default_value: Label) -> Arc<LateBoundDefault> {
    Arc::new(LateBoundDefault::label_from_target_configuration(
        "TestConfiguration",
        Some(default_value),
    ))
}

/// Java `BaseRuleClasses.DEFAULT_COVERAGE_REPORT_GENERATOR_VALUE`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:137 BaseRuleClasses.DEFAULT_COVERAGE_REPORT_GENERATOR_VALUE
pub const DEFAULT_COVERAGE_REPORT_GENERATOR_VALUE: &str = "//tools/test:coverage_report_generator";

/// Java `BaseRuleClasses.coverageReportGeneratorAttribute(defaultValue)`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:145 BaseRuleClasses.coverageReportGeneratorAttribute
pub fn coverage_report_generator_attribute(default_value: Label) -> Arc<LateBoundDefault> {
    Arc::new(LateBoundDefault::label_from_target_configuration(
        "CoverageConfiguration",
        Some(default_value),
    ))
}

/// Java `BaseRuleClasses.RUN_UNDER_EXEC_CONFIG` (null outside analysis).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:162 BaseRuleClasses.RUN_UNDER_EXEC_CONFIG
pub static RUN_UNDER_EXEC_CONFIG: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    Arc::new(LateBoundDefault::label_from_target_configuration(
        "BuildConfigurationValue",
        None,
    ))
});

/// Java `BaseRuleClasses.RUN_UNDER_TARGET_CONFIG` (null outside analysis).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:184 BaseRuleClasses.RUN_UNDER_TARGET_CONFIG
pub static RUN_UNDER_TARGET_CONFIG: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    Arc::new(LateBoundDefault::label_from_target_configuration(
        "BuildConfigurationValue",
        None,
    ))
});

// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:198 BaseRuleClasses.TOOLS_TEST_RUNTIME_TARGET_PATTERN
const TOOLS_TEST_RUNTIME_TARGET_PATTERN: &str = "//tools/test:runtime";

/// The `$test_runtime` default: `[<tools repo>//tools/test:runtime]`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:202 BaseRuleClasses.getTestRuntimeLabelList
pub fn get_test_runtime_label_list(env: &dyn RuleDefinitionEnvironment) -> Vec<Label> {
    vec![Label::parse_canonical_unchecked(&format!(
        "{}{}",
        env.get_tools_repository(),
        TOOLS_TEST_RUNTIME_TARGET_PATTERN
    ))]
}

/// Java `BaseRuleClasses.TAGGED_TRIMMING_ATTR`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:220 BaseRuleClasses.TAGGED_TRIMMING_ATTR
pub const TAGGED_TRIMMING_ATTR: &str = "transitive_configs";

/// Adds the attributes shared by the native and Starlark base rules.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:226 BaseRuleClasses.commonCoreAndStarlarkAttributes
pub fn common_core_and_starlark_attributes(
    builder: &mut RuleClassBuilder,
) -> &mut RuleClassBuilder {
    builder
        // The visibility attribute is special: it is a nodep label, and loading the
        // necessary package groups is handled by {@link LabelVisitor#visitTargetVisibility}.
        // Package groups always have the null configuration so that they are not duplicated
        // needlessly.
        .add(
            attr("visibility", Type::NodepLabelList)
                .order_independent()
                .cfg(exec_transition_factory())
                .nonconfigurable(
                    "special attribute integrated more deeply into Bazel's core logic",
                ),
        )
        .add(
            attr(TAGGED_TRIMMING_ATTR, Type::NodepLabelList)
                .order_independent()
                .nonconfigurable("Used in determining configuration"),
        )
        .add(
            attr("deprecation", Type::String)
                .value_computed_default(DEPRECATION_DEFAULT.clone())
                .nonconfigurable("Used in core loading phase logic with no access to configs"),
        )
        .add(
            attr("tags", Type::StringList)
                .order_independent()
                .taggable()
                .nonconfigurable("low-level attribute, used in TargetUtils without configurations"),
        )
        .add(
            attr("generator_name", Type::String)
                .undocumented("internal")
                .nonconfigurable("static structure of a rule"),
        )
        .add(
            attr("generator_function", Type::String)
                .undocumented("internal")
                .nonconfigurable("static structure of a rule"),
        )
        .add(
            attr("generator_location", Type::String)
                .undocumented("internal")
                .nonconfigurable("static structure of a rule"),
        )
        .add(
            attr("testonly", Type::Boolean)
                .value_computed_default(TESTONLY_DEFAULT.clone())
                .nonconfigurable("policy decision: rules testability should be consistent"),
        )
        .add(attr("features", Type::StringList).order_independent())
        .add(
            attr(":action_listener", Type::LabelList)
                .cfg(exec_transition_factory())
                .value_late_bound(ACTION_LISTENER.clone()),
        )
        .add(
            attr(COMPATIBLE_ENVIRONMENT_ATTR, Type::LabelList)
                .allowed_rule_classes(&[ENVIRONMENT_RULE])
                .cfg(TransitionFactory::no_config_transition())
                .allowed_file_types(FileTypeSet::NoFile)
                .dont_check_constraints()
                .nonconfigurable(
                    "special logic for constraints and select: see ConstraintSemantics",
                ),
        )
        .add(
            attr(RESTRICTED_ENVIRONMENT_ATTR, Type::LabelList)
                .allowed_rule_classes(&[ENVIRONMENT_RULE])
                .cfg(TransitionFactory::no_config_transition())
                .allowed_file_types(FileTypeSet::NoFile)
                .dont_check_constraints()
                .nonconfigurable(
                    "special logic for constraints and select: see ConstraintSemantics",
                ),
        )
        .add(
            attr(CONFIG_SETTING_DEPS_ATTRIBUTE, Type::LabelList)
                .nonconfigurable("stores configurability keys"),
        )
        .add(
            attr(APPLICABLE_METADATA_ATTR, Type::LabelList)
                .value_computed_default(PACKAGE_METADATA_DEFAULT.clone())
                .cfg(TransitionFactory::no_config_transition())
                .allowed_file_types(FileTypeSet::NoFile)
                .dont_check_constraints()
                .nonconfigurable("applicable_metadata is not configurable"),
        )
        .add(attr("aspect_hints", Type::LabelList).allowed_file_types(FileTypeSet::NoFile))
}

/// Adds `exec_properties` (a string dict defaulting to `{}`).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:301 BaseRuleClasses.execPropertiesAttribute
pub fn exec_properties_attribute(builder: &mut RuleClassBuilder) -> &mut RuleClassBuilder {
    builder.add(
        attr(EXEC_PROPERTIES_ATTR, Type::StringDict)
            .value(Some(AttrValue::StringDict(IndexMap::new()))),
    )
}

/// Ancestor of every native rule in BUILD files (Java `BaseRuleClasses.NativeBuildRule`).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:318 BaseRuleClasses.NativeBuildRule
pub struct NativeBuildRule;

impl NativeBuildRule {
    /// Java `BaseRuleClasses.NativeBuildRule.class.getName()`.
    pub const CLASS: &'static str = "BaseRuleClasses.NativeBuildRule";
}

impl RuleDefinition for NativeBuildRule {
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:320 BaseRuleClasses.NativeBuildRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        common_core_and_starlark_attributes(builder)
            .add(
                attr("licenses", Type::License)
                    .nonconfigurable("Used in core loading phase logic with no access to configs"),
            )
            // TODO: b/148549967 - Remove for Bazel 9.0
            .add(attr("distribs", Type::StringList).nonconfigurable("deprecated - no op"))
            // Any rule that provides its own meaning for the "target_compatible_with" attribute
            // has to be excluded in `IncompatibleTargetChecker`.
            .add(
                attr(TARGET_COMPATIBLE_WITH_ATTR, Type::LabelList)
                    .mandatory_builtin_providers(vec!["ConfigMatchingProvider".to_owned()])
                    .mandatory_providers(vec![constraint_value_info_id()])
                    // This should be configurable to allow for complex types of restrictions.
                    .tool(
                        "target_compatible_with exists for constraint checking, not to create an \
                         actual dependency",
                    )
                    .allowed_file_types(FileTypeSet::NoFile),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:343 BaseRuleClasses.NativeBuildRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("$native_build_rule").type_(RuleClassType::Abstract)
    }

    fn definition_class(&self) -> String {
        Self::CLASS.to_owned()
    }
}

/// A rule with a `toolchains` attribute for Make variables (Java
/// `BaseRuleClasses.MakeVariableExpandingRule`).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:352 BaseRuleClasses.MakeVariableExpandingRule
pub struct MakeVariableExpandingRule;

impl MakeVariableExpandingRule {
    pub const CLASS: &'static str = "BaseRuleClasses.MakeVariableExpandingRule";
}

impl RuleDefinition for MakeVariableExpandingRule {
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:354 BaseRuleClasses.MakeVariableExpandingRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            // Documented in
            // com/google/devtools/build/docgen/templates/attributes/common/toolchains.html.
            .add(
                attr("toolchains", Type::LabelList)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers(vec![template_variable_info_id()])
                    .dont_check_constraints(),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:367 BaseRuleClasses.MakeVariableExpandingRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("$make_variable_expanding_rule")
            .type_(RuleClassType::Abstract)
    }

    fn definition_class(&self) -> String {
        Self::CLASS.to_owned()
    }
}

/// Ancestor of every native BUILD rule that creates actions (Java
/// `BaseRuleClasses.NativeActionCreatingRule`).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:382 BaseRuleClasses.NativeActionCreatingRule
pub struct NativeActionCreatingRule;

impl NativeActionCreatingRule {
    pub const CLASS: &'static str = "BaseRuleClasses.NativeActionCreatingRule";
}

impl RuleDefinition for NativeActionCreatingRule {
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:384 BaseRuleClasses.NativeActionCreatingRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .add(attr("deps", Type::LabelList).legacy_allow_any_file_type())
            .add(
                attr("data", Type::LabelList)
                    .allowed_file_types(FileTypeSet::AnyFile)
                    .dont_check_constraints(),
            )
            .add(
                attr(EXEC_PROPERTIES_ATTR, Type::StringDict)
                    .value(Some(AttrValue::StringDict(IndexMap::new()))),
            )
            .add(
                attr(EXEC_COMPATIBLE_WITH_ATTR, Type::LabelList)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable("Used in toolchain resolution")
                    .tool(
                        "exec_compatible_with exists for constraint checking, not to create an \
                         actual dependency",
                    )
                    .value(Some(AttrValue::LabelList(Vec::new()))),
            )
            .add(
                attr(EXEC_GROUP_COMPATIBLE_WITH_ATTR, Type::LabelListDict)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable("Used in toolchain resolution")
                    .tool(
                        "exec_group_compatible_with exists for constraint checking, not to create \
                         an actual dependency",
                    )
                    .value(Some(AttrValue::LabelListDict(IndexMap::new()))),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:412 BaseRuleClasses.NativeActionCreatingRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("$native_buildable_rule")
            .type_(RuleClassType::Abstract)
            .ancestors(&[NativeBuildRule::CLASS])
    }

    fn definition_class(&self) -> String {
        Self::CLASS.to_owned()
    }
}

/// Java `TargetUtils.isTestRuleName(name)`.
// PORT: move to packages/TargetUtils when ported (no owner yet).
// Java: com/google/devtools/build/lib/packages/TargetUtils.java:67 TargetUtils.isTestRuleName
pub fn is_test_rule_name(name: &str) -> bool {
    name.ends_with("_test")
}

/// A placeholder for a native rule removed in favour of a Starlark rule in builtins (Java
/// `BaseRuleClasses.EmptyRule`; each Java use is its own anonymous subclass).
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:429 BaseRuleClasses.EmptyRule
pub struct EmptyRule {
    name: String,
    bzl_load_file: Option<String>,
}

impl EmptyRule {
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:433 BaseRuleClasses.EmptyRule.<init>
    pub fn new(name: &str) -> EmptyRule {
        Self::with_bzl_load_label(name, None)
    }

    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:433 BaseRuleClasses.EmptyRule.<init>
    pub fn with_bzl_load_label(name: &str, bzl_load_label: Option<&str>) -> EmptyRule {
        EmptyRule {
            name: name.to_owned(),
            bzl_load_file: bzl_load_label.map(str::to_owned),
        }
    }
}

impl RuleDefinition for EmptyRule {
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:443 BaseRuleClasses.EmptyRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        if builder.get_type() == RuleClassType::Test {
            builder
                .add(
                    attr("size", Type::String).nonconfigurable(
                        "policy decision: should be consistent across configurations",
                    ),
                )
                .add(
                    attr("timeout", Type::String).nonconfigurable(
                        "policy decision: should be consistent across configurations",
                    ),
                )
                .add(&attr("flaky", Type::Boolean))
                .add(&attr("shard_count", Type::Integer))
                .add(&attr("local", Type::Boolean));
        }
        builder
            .remove_attribute("deps")
            .remove_attribute("data")
            .add_attribute(
                attr("$bzl_load_label", Type::String)
                    .value(self.bzl_load_file.clone().map(AttrValue::String))
                    .build(),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:464 BaseRuleClasses.EmptyRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(&self.name)
            .type_(if is_test_rule_name(&self.name) {
                RuleClassType::Test
            } else {
                RuleClassType::Normal
            })
            .ancestors(&[NativeActionCreatingRule::CLASS])
            .factory_class("EmptyRuleConfiguredTargetFactory")
    }

    fn definition_class(&self) -> String {
        // PORT: Java's anonymous subclass `new EmptyRule("<name>") {}`.
        format!("BaseRuleClasses.EmptyRule({})", self.name)
    }
}
