//! Port of `analysis/starlark/StarlarkRuleClassFunctions.java`: `rule()`,
//! `materializer_rule()`, `exec_group()`, the base rule classes of Starlark rules and the
//! `StarlarkRuleFunction` value that `rule()` returns. `aspect()`, `subrule()` and `macro()` are
//! in their own modules.

use std::fmt;
use std::hash::Hash;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use indexmap::IndexMap;
use indexmap::IndexSet;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use super::attr_module::DescriptorRef;
use super::attr_module::as_descriptor;
use super::attr_module::get_starlark_provider_identifiers;
use super::attr_module::is_starlark_function;
use super::config::BuildSettingValue;
use super::config::convert_config;
use super::config::default_exec_factory;
use super::config::exec_factory;
use super::subrule;
use super::util::call_stack;
use super::util::caller_location;
use super::util::dict_cast;
use super::util::errorf;
use super::util::eval_exception;
use super::util::is_valid_identifier;
use super::util::label_converter_for_bzl;
use super::util::param_type_error;
use super::util::sequence_cast;
use super::util::sequence_cast_strings;
use super::util::sequence_elements;
use super::util::starlark_type;
use super::util::trim_doc_string;
use super::value_cell::PendingValue;
use super::value_cell::resolve;
use crate::attribute::Attribute;
use crate::attribute::AttributeBuilder;
use crate::attribute::ImplicitOutputsFunction;
use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::bzl_context::BzlInitThreadContext;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::base_rule_classes;
use crate::package::BuildLangTypedAttributeValuesMap;
use crate::package::TargetDefinitionContext;
use crate::rule_class::AllowlistChecker;
use crate::rule_class::DeclaredExecGroup;
use crate::rule_class::LocationCheck;
use crate::rule_class::MissingFragmentPolicy;
use crate::rule_class::ProviderKey;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::rule_class::RuleFunction;
use crate::rule_class::StarlarkCallable;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::rule_class::ToolchainTypeRequirement;
use crate::rule_class::rule_class::DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME;
use crate::rule_class::rule_class::EXEC_COMPATIBLE_WITH_ATTR;
use crate::rule_class::rule_class::EXEC_GROUP_COMPATIBLE_WITH_ATTR;
use crate::rule_class::rule_class::EXEC_PROPERTIES_ATTR;
use crate::rule_class::rule_class::TARGET_COMPATIBLE_WITH_ATTR;
use crate::target::IS_EXECUTABLE_ATTRIBUTE_NAME;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::LabelClass;
use crate::types::LabelConverter;
use crate::types::Type;

// -------------------------------------------------------------------------------------------
// Constants

/// Java `StarlarkRuleClassFunctions.ALLOWLIST_RULE_EXTENSION_API` (`BuiltinRestriction`
/// entries: (repository name, package prefix)).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:243 StarlarkRuleClassFunctions.ALLOWLIST_RULE_EXTENSION_API
pub const ALLOWLIST_RULE_EXTENSION_API: &[(&str, &str)] = &[
    ("", "initializer_testing"),
    ("", "extend_rule_testing"),
    ("", "subrule_testing"),
];

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:249 StarlarkRuleClassFunctions.ALLOWLIST_RULE_EXTENSION_API_EXPERIMENTAL
pub const ALLOWLIST_RULE_EXTENSION_API_EXPERIMENTAL: &[(&str, &str)] = &[
    ("", "initializer_testing/builtins"),
    ("", "third_party/bazel_rules/rules_cc"),
    ("rules_cc", ""),
];

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:255 StarlarkRuleClassFunctions.COMMON_ATTRIBUTES_NAME
const COMMON_ATTRIBUTES_NAME: &str = "common";

// Java: com/google/devtools/build/lib/packages/FunctionSplitTransitionAllowlist.java:21 FunctionSplitTransitionAllowlist
const FUNCTION_TRANSITION_NAME: &str = "function_transition";
const FUNCTION_TRANSITION_ATTRIBUTE_NAME: &str = "$allowlist_function_transition";
const FUNCTION_TRANSITION_LABEL_STR: &str = "//tools/allowlists/function_transition_allowlist";
// Java: com/google/devtools/build/lib/analysis/DormantDependency.java:38 DormantDependency
const DORMANT_DEPENDENCY_NAME: &str = "dormant_dependency";
const DORMANT_DEPENDENCY_ALLOWLIST_ATTRIBUTE_NAME: &str = "$allowlist_dormant_dependency";
const DORMANT_DEPENDENCY_ALLOWLIST_LABEL_STR: &str =
    "//tools/allowlists/dormant_dependency_allowlist";

fn allowlist_checker(attr: &str, message: &str, location_check: LocationCheck) -> AllowlistChecker {
    AllowlistChecker {
        allowlist_attr: attr.to_owned(),
        error_message: message.to_owned(),
        location_check,
        attribute_set_trigger: None,
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2052 StarlarkRuleClassFunctions.FUNCTION_TRANSITION_ALLOWLIST_CHECKER
fn function_transition_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        FUNCTION_TRANSITION_NAME,
        "Non-allowlisted use of Starlark transition",
        LocationCheck::InstanceOrDefinition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2060 StarlarkRuleClassFunctions.DORMANT_DEPENDENCY_ALLOWLIST_CHECKER
fn dormant_dependency_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        DORMANT_DEPENDENCY_NAME,
        "Non-allowlisted use of dormant dependencies",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2068 StarlarkRuleClassFunctions.EXTEND_RULE_ALLOWLIST_CHECKER
fn extend_rule_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "extend_rule",
        "Non-allowlisted attempt to extend a rule.",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2076 StarlarkRuleClassFunctions.EXTEND_RULE_API_ALLOWLIST_CHECKER
fn extend_rule_api_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "extend_rule_api",
        "Non-allowlisted attempt to use extend rule APIs.",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2084 StarlarkRuleClassFunctions.INITIALIZER_ALLOWLIST_CHECKER
fn initializer_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "initializer",
        "Non-allowlisted attempt to use initializer.",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2092 StarlarkRuleClassFunctions.SUBRULES_ALLOWLIST_CHECKER
fn subrules_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "subrules",
        "Non-allowlisted attempt to use subrules.",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2100 StarlarkRuleClassFunctions.SKIP_VALIDATIONS_ALLOWLIST_CHECKER
fn skip_validations_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "skip_validations",
        "Non-allowlisted use of skip_validations",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2108 StarlarkRuleClassFunctions.MATERIALIZER_RULE_ALLOWLIST_CHECKER
fn materializer_rule_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "materializer_rule",
        "Non-allowlisted use of materializer rule",
        LocationCheck::Definition,
    )
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2116 StarlarkRuleClassFunctions.MATERIALIZER_RULE_REAL_DEPS_ALLOWLIST_CHECKER
fn materializer_rule_real_deps_allowlist_checker() -> AllowlistChecker {
    allowlist_checker(
        "materializer_rule_real_deps",
        "Non-allowlisted use of real deps in materializer target",
        LocationCheck::Definition,
    )
}

/// A builtin provider key (Java `BuiltinProvider.Key`).
fn builtin_provider_id(name: &str) -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Builtin {
        name: name.to_owned(),
        provider_class: name.to_owned(),
    })
}

/// Java `BuiltinRestriction.failIfCalledOutsideAllowlist`: the innermost .bzl's label, checked
/// under that module's own repo mapping (`failIfModuleOutsideAllowlist`).
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:169 BuiltinRestriction.failIfCalledOutsideAllowlist
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:190 BuiltinRestriction.failIfModuleOutsideAllowlist
pub fn fail_if_called_outside_allowlist(
    eval: &Evaluator,
    allowlist: &[(&str, &str)],
) -> starlark::Result<()> {
    let module = super::util::module_context_or_throw(eval)?;
    fail_if_label_outside_allowlist(module.label(), &module.repo_mapping, allowlist)
}

/// Java `BuiltinRestriction.failIfLabelOutsideAllowlist`. Each allowlist entry is an
/// `AllowlistEntry` (apparent repository name, package prefix).
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:199 BuiltinRestriction.failIfLabelOutsideAllowlist
pub fn fail_if_label_outside_allowlist(
    label: &Label,
    repo_mapping: &RepositoryMapping,
    allowlist: &[(&str, &str)],
) -> starlark::Result<()> {
    if is_not_allowed(label, repo_mapping, allowlist) {
        return Err(errorf(format!(
            "file '{}' cannot use private API",
            label.get_canonical_form()
        )));
    }
    Ok(())
}

/// Java `BuiltinRestriction.isNotAllowed`: true if the label is neither in the builtins
/// repository nor in a package or subpackage of an allowlist entry.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:211 BuiltinRestriction.isNotAllowed
fn is_not_allowed(
    label: &Label,
    repo_mapping: &RepositoryMapping,
    allowlist: &[(&str, &str)],
) -> bool {
    if *label.get_repository() == *blaze_cmdline::repository_name::BUILTINS {
        return false;
    }
    !allowlist
        .iter()
        .any(|(apparent_repo_name, package_prefix)| {
            allowlist_entry_allows(apparent_repo_name, package_prefix, label, repo_mapping)
        })
}

/// Java `BuiltinRestriction.AllowlistEntry.allows`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:124 BuiltinRestriction.AllowlistEntry.allows
fn allowlist_entry_allows(
    apparent_repo_name: &str,
    package_prefix: &str,
    label: &Label,
    repo_mapping: &RepositoryMapping,
) -> bool {
    allowlist_entry_repos_match(apparent_repo_name, label.get_repository(), repo_mapping)
        && label
            .get_package_fragment()
            .starts_with(&blaze_vfs::PathFragment::create(package_prefix))
}

/// Java `BuiltinRestriction.AllowlistEntry.reposMatch`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:129 BuiltinRestriction.AllowlistEntry.reposMatch
fn allowlist_entry_repos_match(
    allowed_name: &str,
    given_name: &RepositoryName,
    repo_mapping: &RepositoryMapping,
) -> bool {
    if *given_name == *blaze_cmdline::repository_name::MAIN {
        // The main repository may be one of the allowlisted rulesets, in which case we need to
        // fall back to interpreting allowedName as the apparent repo name. This is not a
        // performance concern since:
        // * In Bazel, the main repo is not expected to use private API unless it is one of the
        //   allowlisted rulesets. For these rulesets, it is acceptable to pay the cost of a
        //   failed RepositoryMapping lookup, which is expensive because it uses SpellChecker to
        //   construct error messages. The only other case in which this cost is paid is if the
        //   main repo attempts to use private APIs and subsequently fails.
        // * In Blaze, the repo mapping is always trivial and lookups are cheap.
        return allowed_name == blaze_cmdline::repository_name::MAIN.get_name()
            || repo_mapping.get(allowed_name).is_main();
    }
    if *given_name == *blaze_cmdline::repository_name::BAZEL_TOOLS {
        return allowed_name == blaze_cmdline::repository_name::BAZEL_TOOLS.get_name();
    }
    // allowedName is a module name and givenName is a real canonical repo name, so it belongs
    // to any version of that module if and only if it contains <allowedName>+ as a prefix.
    given_name
        .get_name()
        .starts_with(&format!("{allowed_name}+"))
}

// -------------------------------------------------------------------------------------------
// Base rule classes

/// The `RuleDefinitionEnvironment` of a .bzl evaluation (Java: the rule class provider).
struct BzlRuleDefinitionEnvironment {
    tools_repository: RepositoryName,
}

impl RuleDefinitionEnvironment for BzlRuleDefinitionEnvironment {
    fn get_tools_repository(&self) -> RepositoryName {
        self.tools_repository.clone()
    }
}

fn catch_illegal_state<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = if let Some(s) = p.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = p.downcast_ref::<&str>() {
            (*s).to_owned()
        } else {
            "unknown error".to_owned()
        };
        // Java prints the exception message; check_state! prefixes it with the class name.
        msg.strip_prefix("IllegalStateException: ")
            .or_else(|| msg.strip_prefix("IllegalArgumentException: "))
            .unwrap_or(&msg)
            .to_owned()
    })
}

/// Parent rule class for non-executable non-test Starlark rules.
// TODO(bazel-team): Remove the code duplication (BaseRuleClasses and this class).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:159 StarlarkRuleClassFunctions.baseRule
pub static BASE_RULE: LazyLock<Arc<RuleClass>> = LazyLock::new(|| {
    let mut builder = RuleClassBuilder::new("$base_rule", RuleClassType::Abstract, true, &[]);
    builder.add(&attr("expect_failure", Type::String));
    base_rule_classes::common_core_and_starlark_attributes(&mut builder);
    // TODO(skylark-team): Allow Starlark rules to extend native rules and remove duplication.
    builder
        .add(
            attr("toolchains", Type::LabelList)
                .allowed_file_types(FileTypeSet::NoFile)
                .mandatory_providers(vec![builtin_provider_id("TemplateVariableInfo")])
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
        .add(
            attr(TARGET_COMPATIBLE_WITH_ATTR, Type::LabelList)
                .mandatory_builtin_providers(vec!["ConfigMatchingProvider".to_owned()])
                .mandatory_providers(vec![builtin_provider_id("ConstraintValueInfo")])
                // This should be configurable to allow for complex types of restrictions.
                .tool(
                    "target_compatible_with exists for constraint checking, not to create an \
                     actual dependency",
                )
                .allowed_file_types(FileTypeSet::NoFile),
        );
    Arc::new(builder.build())
});

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:197 StarlarkRuleClassFunctions.dependencyResolutionBaseRule
pub static DEPENDENCY_RESOLUTION_BASE_RULE: LazyLock<Arc<RuleClass>> = LazyLock::new(|| {
    let mut builder = RuleClassBuilder::new(
        "$dependency_resolution_base_rule",
        RuleClassType::Abstract,
        true,
        std::slice::from_ref(&*BASE_RULE),
    );
    builder
        .set_dependency_resolution_rule()
        .remove_attribute(":action_listener")
        .remove_attribute("aspect_hints")
        .remove_attribute("toolchains")
        .remove_attribute(EXEC_COMPATIBLE_WITH_ATTR)
        .remove_attribute(EXEC_GROUP_COMPATIBLE_WITH_ATTR)
        .remove_attribute(TARGET_COMPATIBLE_WITH_ATTR)
        .remove_attribute("compatible_with")
        .remove_attribute("restricted_to")
        .remove_attribute("$config_dependencies")
        // Dependency resolution rules can't have package defaults.
        .remove_attribute("package_metadata");
    Arc::new(builder.build())
});

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:214 StarlarkRuleClassFunctions.materializerBaseRule
pub static MATERIALIZER_BASE_RULE: LazyLock<Arc<RuleClass>> = LazyLock::new(|| {
    let mut builder = RuleClassBuilder::new(
        "$materializer_base_rule",
        RuleClassType::Abstract,
        true,
        std::slice::from_ref(&*BASE_RULE),
    );
    builder
        .set_is_materializer_rule(true)
        .remove_attribute(":action_listener")
        .remove_attribute("aspect_hints")
        .remove_attribute("toolchains")
        .remove_attribute(EXEC_COMPATIBLE_WITH_ATTR)
        .remove_attribute(EXEC_GROUP_COMPATIBLE_WITH_ATTR)
        .remove_attribute(TARGET_COMPATIBLE_WITH_ATTR)
        .remove_attribute("compatible_with")
        .remove_attribute("restricted_to")
        // Materializer rules can't have package defaults, in particular because materializer
        // rules can't have dependencies on non-dependency-resolution-rules or dependencies
        // through non-dormant attributes.
        .remove_attribute("package_metadata");
    Arc::new(builder.build())
});

/// Parent rule class for executable non-test Starlark rules.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:232 StarlarkRuleClassFunctions.binaryBaseRule
pub static BINARY_BASE_RULE: LazyLock<Arc<RuleClass>> = LazyLock::new(|| {
    let mut builder = RuleClassBuilder::new(
        "$binary_base_rule",
        RuleClassType::Abstract,
        true,
        std::slice::from_ref(&*BASE_RULE),
    );
    builder
        .add(&attr("args", Type::StringList))
        .add(&attr("output_licenses", Type::StringList))
        .add_attribute(
            attr(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
                .value(Some(AttrValue::Bool(true)))
                .nonconfigurable("Called from RunCommand.isExecutable, which takes a Target")
                .build(),
        );
    Arc::new(builder.build())
});

/// Java `RuleClass.DEFAULT_TEST_RUNNER_EXEC_GROUP`.
// Java: com/google/devtools/build/lib/packages/RuleClass.java:161 RuleClass.DEFAULT_TEST_RUNNER_EXEC_GROUP
fn default_test_runner_exec_group() -> DeclaredExecGroup {
    // Java: com/google/devtools/build/lib/analysis/platform/PlatformConstants.java:27 PlatformConstants.DEFAULT_TEST_TOOLCHAIN_TYPE
    let toolchain_type =
        Label::parse_canonical_unchecked("@bazel_tools//tools/test:default_test_toolchain_type");
    let mut toolchain_types_map = IndexMap::new();
    toolchain_types_map.insert(
        toolchain_type.clone(),
        ToolchainTypeRequirement {
            toolchain_type,
            mandatory: false,
            ignore_if_invalid: false,
        },
    );
    DeclaredExecGroup {
        toolchain_types_map,
        exec_compatible_with: IndexSet::new(),
        copy_from_default: false,
    }
}

/// Parent rule class for test Starlark rules.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:258 StarlarkRuleClassFunctions.getTestBaseRule
pub fn get_test_base_rule(env: &dyn RuleDefinitionEnvironment) -> Arc<RuleClass> {
    let tools_repository = env.get_tools_repository();
    let tools_label = |s: &str| Label::parse_canonical_unchecked(&format!("{tools_repository}{s}"));
    let test_runner = || exec_factory(DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME);
    let mut builder = RuleClassBuilder::new(
        "$test_base_rule",
        RuleClassType::Abstract,
        true,
        std::slice::from_ref(&*BASE_RULE),
    );
    builder
        .requires_configuration_fragments(&["TestConfiguration"])
        // TestConfiguration only needed to create TestAction and TestProvider
        // Only necessary at top-level and can be skipped if trimmed.
        .set_missing_fragment_policy("TestConfiguration", MissingFragmentPolicy::Ignore)
        .add(
            attr("size", Type::String)
                .value(Some(AttrValue::String("medium".to_owned())))
                .taggable()
                .nonconfigurable("used in loading phase rule validation logic"),
        )
        .add(
            attr("timeout", Type::String)
                .taggable()
                .nonconfigurable("policy decision: should be consistent across configurations")
                .value_computed_default(base_rule_classes::TIMEOUT_DEFAULT.clone()),
        )
        .add(
            attr("flaky", Type::Boolean)
                .value(Some(AttrValue::Bool(false)))
                .taggable()
                .nonconfigurable("taggable - called in Rule.getRuleTags"),
        )
        .add(attr("shard_count", Type::Integer).value(Some(AttrValue::Int(-1))))
        .add(
            attr("local", Type::Boolean)
                .value(Some(AttrValue::Bool(false)))
                .taggable()
                .nonconfigurable(
                    "policy decision: this should be consistent across configurations",
                ),
        )
        .add(&attr("args", Type::StringList))
        // Input files for every test action
        .add(
            attr("$test_wrapper", Type::Label)
                .cfg(test_runner())
                .single_artifact()
                .value(Some(AttrValue::Label(tools_label(
                    "//tools/test:test_wrapper",
                )))),
        )
        .add(
            attr("$xml_writer", Type::Label)
                .cfg(test_runner())
                .single_artifact()
                .value(Some(AttrValue::Label(tools_label(
                    "//tools/test:xml_writer",
                )))),
        )
        .add(
            attr("$test_runtime", Type::LabelList)
                .cfg(test_runner())
                // Getting this default value through the getTestRuntimeLabelList helper ensures
                // we reuse the same ImmutableList<Label> instance for each $test_runtime attr.
                .value(Some(AttrValue::LabelList(
                    base_rule_classes::get_test_runtime_label_list(env),
                ))),
        )
        .add(
            attr("$test_setup_script", Type::Label)
                .cfg(test_runner())
                .single_artifact()
                .value(Some(AttrValue::Label(tools_label(
                    "//tools/test:test_setup",
                )))),
        )
        .add(
            attr("$xml_generator_script", Type::Label)
                .cfg(test_runner())
                .single_artifact()
                .value(Some(AttrValue::Label(tools_label(
                    "//tools/test:test_xml_generator",
                )))),
        )
        .add(
            attr("$collect_coverage_script", Type::Label)
                .cfg(test_runner())
                .value(Some(AttrValue::Label(tools_label(
                    "//tools/test:collect_coverage",
                )))),
        )
        // Input files for test actions collecting code coverage
        .add(
            attr(":coverage_support", Type::Label)
                .cfg(test_runner())
                .value_late_bound(base_rule_classes::coverage_support_attribute(tools_label(
                    base_rule_classes::DEFAULT_COVERAGE_SUPPORT_VALUE,
                ))),
        )
        // Used in the one-per-build coverage report generation action.
        .add(
            attr(":coverage_report_generator", Type::Label)
                .cfg(test_runner())
                .value_late_bound(base_rule_classes::coverage_report_generator_attribute(
                    tools_label(base_rule_classes::DEFAULT_COVERAGE_REPORT_GENERATOR_VALUE),
                )),
        )
        // See similar definitions in BaseRuleClasses for context.
        .add(
            attr(":run_under_exec_config", Type::Label)
                .cfg(test_runner())
                .value_late_bound(base_rule_classes::RUN_UNDER_EXEC_CONFIG.clone())
                .skip_prereq_validator_check(),
        )
        .add(
            attr(":run_under_target_config", Type::Label)
                .value_late_bound(base_rule_classes::RUN_UNDER_TARGET_CONFIG.clone())
                .skip_prereq_validator_check(),
        )
        .add_attribute(
            attr(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
                .value(Some(AttrValue::Bool(true)))
                .nonconfigurable("Called from RunCommand.isExecutable, which takes a Target")
                .build(),
        );

    if env.get_network_allowlist_for_tests().is_some() {
        blaze_util::unported!("Allowlist.getAttributeFromAllowlistName(\"external_network\")");
    }

    Arc::new(builder.build())
}

// -------------------------------------------------------------------------------------------
// Helpers shared with aspect() / subrule() / macro()

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1280 StarlarkRuleClassFunctions.checkAttributeName
pub fn check_attribute_name(name: &str) -> starlark::Result<()> {
    if !is_valid_identifier(name) {
        return Err(errorf(format!(
            "attribute name `{name}` is not a valid identifier."
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1286 StarlarkRuleClassFunctions.attrObjectToAttributesList
pub fn attr_object_to_attributes_list<'v>(
    attrs: Value<'v>,
) -> starlark::Result<Vec<(String, DescriptorRef<'v>)>> {
    let mut attributes = Vec::new();
    let entries = dict_cast(
        attrs,
        "string",
        |k| k.unpack_str().is_some(),
        "Attribute",
        |v| as_descriptor(v).is_some(),
        "attrs",
    )?;
    for (k, v) in entries {
        let name = k.unpack_str().unwrap();
        let attr_descriptor = as_descriptor(v).unwrap();
        let source = attr_descriptor.get_value_source();
        check_attribute_name(name)?;
        let attr_name = source
            .convert_to_native_name(name)
            .map_err(eval_exception)?;
        attributes.push((attr_name, attr_descriptor));
    }
    Ok(attributes)
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1302 StarlarkRuleClassFunctions.parseLabels
pub fn parse_labels(
    inputs: Value,
    label_converter: &crate::types::LabelConverter,
    attribute_name: &str,
) -> starlark::Result<IndexSet<Label>> {
    let mut parsed_labels = IndexSet::new();
    if sequence_elements(inputs).is_some_and(|e| e.is_empty()) {
        return Ok(parsed_labels);
    }
    for input in sequence_cast_strings(inputs, attribute_name)? {
        match label_converter.convert(&input) {
            Ok(label) => {
                parsed_labels.insert(label);
            }
            Err(e) => {
                return Err(errorf(format!(
                    "Unable to parse label '{input}' in attribute '{attribute_name}': {}",
                    e.get_message()
                )));
            }
        }
    }
    Ok(parsed_labels)
}

/// Java `ToolchainTypeRequirement.strictest`.
// Java: com/google/devtools/build/lib/analysis/config/ToolchainTypeRequirement.java:59 ToolchainTypeRequirement.strictest
fn strictest(
    first: ToolchainTypeRequirement,
    second: ToolchainTypeRequirement,
) -> ToolchainTypeRequirement {
    assert_eq!(
        first.toolchain_type, second.toolchain_type,
        "Cannot use strictest() for two instances with different type labels."
    );
    if first.mandatory {
        return first;
    }
    if second.mandatory {
        return second;
    }
    first
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2232 StarlarkRuleClassFunctions.parseToolchainType
fn parse_toolchain_type(
    raw_toolchain: Value,
    label_converter: &crate::types::LabelConverter,
) -> starlark::Result<ToolchainTypeRequirement> {
    // Handle actual ToolchainTypeRequirement objects.
    if let Some(t) = super::config_common::toolchain_type_requirement(raw_toolchain) {
        return Ok(t);
    }

    // Handle Label-like objects.
    let mut toolchain_label = None;
    if let Some(l) = raw_toolchain.downcast_ref::<blaze_starlark::label::LabelValue>() {
        toolchain_label = Some(l.label().clone());
    } else if let Some(s) = raw_toolchain.unpack_str() {
        match label_converter.convert(s) {
            Ok(l) => toolchain_label = Some(l),
            Err(e) => {
                return Err(errorf(format!(
                    "Unable to parse toolchain_type label '{s}': {}",
                    e.get_message()
                )));
            }
        }
    }

    if let Some(toolchain_label) = toolchain_label {
        return Ok(ToolchainTypeRequirement {
            toolchain_type: toolchain_label,
            mandatory: true,
            ignore_if_invalid: false,
        });
    }

    // It's not a valid type.
    // PORT: Java prints the Java class simple name of the value.
    Err(errorf(format!(
        "'toolchains' takes a toolchain_type, Label, or String, but instead got a {}",
        java_class_simple_name(raw_toolchain)
    )))
}

/// The Java class simple name of a Starlark value of a core type, as Java's
/// `getClass().getSimpleName()` would print it.
fn java_class_simple_name(v: Value) -> String {
    match v.get_type() {
        "int" => "StarlarkInt".to_owned(),
        "bool" => "Boolean".to_owned(),
        "NoneType" => "NoneType".to_owned(),
        "list" => "StarlarkList".to_owned(),
        "tuple" => "Tuple".to_owned(),
        "dict" => "Dict".to_owned(),
        "function" => "StarlarkFunction".to_owned(),
        "struct" => "StarlarkInfo".to_owned(),
        other => other.to_owned(),
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2214 StarlarkRuleClassFunctions.parseToolchainTypes
pub fn parse_toolchain_types(
    raw_toolchains: Value,
    label_converter: &crate::types::LabelConverter,
) -> starlark::Result<IndexSet<ToolchainTypeRequirement>> {
    let mut toolchain_types: IndexMap<Label, ToolchainTypeRequirement> = IndexMap::new();
    let raw = sequence_cast(raw_toolchains, "Object", |_| true, "toolchains")?;
    for raw_toolchain in raw {
        let mut toolchain_type = parse_toolchain_type(raw_toolchain, label_converter)?;
        let type_label = toolchain_type.toolchain_type.clone();
        if let Some(previous) = toolchain_types.get(&type_label) {
            // Keep the one with the strictest requirements.
            toolchain_type = strictest(previous.clone(), toolchain_type);
        }
        toolchain_types.insert(type_label, toolchain_type);
    }
    Ok(toolchain_types.into_values().collect())
}

/// Java `StarlarkExecGroupCollection.isValidGroupName`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:78 StarlarkExecGroupCollection.isValidGroupName
fn is_valid_group_name(exec_group_name: &str) -> bool {
    exec_group_name != crate::rule_class::DEFAULT_EXEC_GROUP_NAME
        && is_valid_identifier(exec_group_name)
}

/// `Dict.cast(execGroups, String.class, DeclaredExecGroup.class, "exec_group")` and the group
/// name checks of `rule()`/`aspect()`.
pub fn parse_exec_groups(raw: Value) -> starlark::Result<IndexMap<String, DeclaredExecGroup>> {
    let entries = dict_cast(
        raw,
        "string",
        |k| k.unpack_str().is_some(),
        "exec_group",
        |v| v.downcast_ref::<ExecGroupValue>().is_some(),
        "exec_group",
    )?;
    let mut groups = IndexMap::new();
    for (k, v) in entries {
        let group = k.unpack_str().unwrap();
        // TODO(b/151742236): document this in the param documentation.
        if !is_valid_group_name(group) {
            return Err(errorf(format!(
                "Exec group name '{group}' is not a valid name."
            )));
        }
        groups.insert(
            group.to_owned(),
            v.downcast_ref::<ExecGroupValue>().unwrap().0.clone(),
        );
    }
    Ok(groups)
}

// -------------------------------------------------------------------------------------------
// exec_group()

/// The Starlark value of `exec_group()` (Java `DeclaredExecGroup` implements `ExecGroupApi`).
#[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExecGroupValue(#[allocative(skip)] pub DeclaredExecGroup);

starlark_simple_value!(ExecGroupValue);

impl fmt::Display for ExecGroupValue {
    // PORT: DeclaredExecGroup has no repr; Starlark's default for a value without one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<exec_group>")
    }
}

#[starlark_value(type = "exec_group")]
impl<'v> StarlarkValue<'v> for ExecGroupValue {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<ExecGroupValue>()
            .is_some_and(|o| o == self))
    }
}

// -------------------------------------------------------------------------------------------
// StarlarkRuleFunction

/// The mutable state of a rule function: the builder until export, the rule class after.
pub struct RuleFunctionState {
    builder: Mutex<Option<RuleClassBuilder>>,
    rule_class: OnceLock<Arc<RuleClass>>,
    definition_location: Location,
    /// Java's identity token of an unexported rule function.
    symbol: u64,
    /// `(bzl label, exported name)` once exported.
    exported: OnceLock<(Label, String)>,
}

impl fmt::Debug for RuleFunctionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuleFunctionState")
            .field("exported", &self.exported.get())
            .finish_non_exhaustive()
    }
}

impl RuleFunctionState {
    /// Java `definitionLocation`: where `rule()` was called.
    pub fn get_definition_location(&self) -> &Location {
        &self.definition_location
    }

    /// Java `getRuleClass()` (`None` before export or after a failed export).
    pub fn rule_class(&self) -> Option<&Arc<RuleClass>> {
        self.rule_class.get()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1762 StarlarkRuleFunction.getName
    pub fn get_name(&self) -> String {
        match self.rule_class.get() {
            Some(rc) => rc.get_name().to_owned(),
            None => "unexported rule".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1991 StarlarkRuleFunction.isExported
    pub fn is_exported(&self) -> bool {
        self.exported.get().is_some()
    }
}

/// A callable Starlark object representing a Starlark-defined rule, which may be invoked during
/// package construction time to instantiate the rule (Java `StarlarkRuleFunction`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1724 StarlarkRuleClassFunctions.StarlarkRuleFunction
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkRuleFunctionGen<V: ValueLifetimeless> {
    /// The parent rule function (Java keeps only its RuleClass).
    parent: Option<V>,
    /// The declared subrules (Java keeps them in the builder / rule class).
    subrules: Vec<V>,
    /// Values referred to by handles in the rule class (empty once frozen).
    pending: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    handles: Vec<StarlarkCallable>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    state: Arc<RuleFunctionState>,
}

starlark_complex_value!(pub StarlarkRuleFunction);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkRuleFunctionGen<Value<'v>>>
    for StarlarkRuleFunctionGen<FrozenValue>
{
}

impl<'v> Freeze for StarlarkRuleFunction<'v> {
    type Frozen = FrozenStarlarkRuleFunction;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkRuleFunction> {
        for (value, handle) in self.pending.into_iter().zip(self.handles) {
            PendingValue { value, handle }.freeze(freezer)?;
        }
        Ok(StarlarkRuleFunctionGen {
            parent: match self.parent {
                Some(p) => Some(p.freeze(freezer)?),
                None => None,
            },
            subrules: self.subrules.freeze(freezer)?,
            pending: Vec::new(),
            handles: Vec::new(),
            state: self.state,
        })
    }
}

/// The state of a rule function value, frozen or not (Java `instanceof StarlarkRuleFunction`).
pub fn rule_function_state(v: Value) -> Option<Arc<RuleFunctionState>> {
    if let Some(r) = StarlarkRuleFunction::from_value(v) {
        return Some(r.state.clone());
    }
    v.downcast_ref::<FrozenStarlarkRuleFunction>()
        .map(|r| r.state.clone())
}

fn rule_function_subrules<'v>(v: Value<'v>) -> Vec<Value<'v>> {
    if let Some(r) = StarlarkRuleFunction::from_value(v) {
        return r.subrules.clone();
    }
    match v.downcast_ref::<FrozenStarlarkRuleFunction>() {
        Some(r) => r.subrules.iter().map(|s| s.to_value()).collect(),
        None => Vec::new(),
    }
}

impl<'v, V: ValueLike<'v>> StarlarkRuleFunctionGen<V> {
    pub fn state(&self) -> &Arc<RuleFunctionState> {
        &self.state
    }
}

impl<'v, V: ValueLike<'v>> RuleFunction for StarlarkRuleFunctionGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1985 StarlarkRuleFunction.getRuleClass
    fn get_rule_class(&self) -> Arc<RuleClass> {
        self.state
            .rule_class
            .get()
            .cloned()
            .expect("IllegalStateException: rule class not exported")
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkRuleFunctionGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1999 StarlarkRuleFunction.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.state.is_exported() {
            match self.state.rule_class.get() {
                Some(rc) => write!(f, "<rule {}>", rc.get_name()),
                // PORT: Java's getRuleClass() fails here (export reported an error).
                None => write!(f, "<rule>"),
            }
        } else {
            write!(f, "<rule>")
        }
    }
}

/// Java `StarlarkRuleFunction.getLegacyAnyTypeAttrs`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1518 StarlarkRuleClassFunctions.getLegacyAnyTypeAttrs
fn get_legacy_any_type_attrs(rule_class: &RuleClass) -> IndexSet<String> {
    let attr = rule_class
        .get_attribute_provider()
        .get_attribute_by_name_maybe("$legacy_any_type_attrs");
    match attr {
        Some(attr) if attr.get_type() == Type::StringList => {
            match attr.get_default_value_unchecked() {
                Some(AttrValue::StringList(l)) => l.iter().cloned().collect(),
                _ => IndexSet::new(),
            }
        }
        _ => IndexSet::new(),
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1902 StarlarkRuleFunction.validateRulePropagatedAspects
fn validate_rule_propagated_aspects(rule_class: &RuleClass) -> starlark::Result<()> {
    for attribute in rule_class.get_attribute_provider().get_attributes() {
        attribute
            .validate_rule_propagated_aspects_parameters(rule_class)
            .map_err(eval_exception)?;
    }
    Ok(())
}

/// The thread state of `StarlarkRuleFunction.call` while the rule's initializers run: the
/// `TargetDefinitionContext` is off the thread (so initializers cannot call load-time
/// functions) and the package's `LabelConverter` is a thread-local (for
/// `native.package_relative_label()` and `select()` in an initializer). Dropping it is Java's
/// `finally` block: it runs on every exit path, including errors and panics.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1789 StarlarkRuleFunction.call
struct InitializerThreadState<'a> {
    thread: &'a ThreadContext,
    target_definition_context: Option<Box<dyn std::any::Any>>,
}

impl<'a> InitializerThreadState<'a> {
    fn enter(thread: &'a ThreadContext, label_converter: Arc<LabelConverter>) -> Self {
        // Temporarily remove `targetDefinitionContext` from the thread to prevent calls to load
        // time functions. Mutating values in initializers is mostly not a problem, because the
        // attribute values are copied before calling the initializers (<-TODO) and before they
        // are set on the target. Exception is a legacy case allowing arbitrary type of parameter
        // values. In that case the values may be mutated by the initializer, but they are still
        // copied when set on the target.
        let target_definition_context = thread.clear_payload();
        // Allow access to the LabelConverter to support native.package_relative_label() in an
        // initializer.
        thread.set_thread_local::<Arc<LabelConverter>>(Some(label_converter));
        // PORT: skipped, `thread.setUncheckedExceptionContext(() -> "an initializer")` (crash
        // report context only).
        InitializerThreadState {
            thread,
            target_definition_context,
        }
    }
}

impl Drop for InitializerThreadState<'_> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1880 StarlarkRuleFunction.call
    fn drop(&mut self) {
        self.thread.set_thread_local::<Arc<LabelConverter>>(None);
        self.thread
            .restore_payload(self.target_definition_context.take());
    }
}

/// Java `StarlarkRuleFunction.call`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1767 StarlarkRuleFunction.call
fn call_rule_function<'v>(
    state: &RuleFunctionState,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    if args.len()? != args.names_map()?.len() {
        return Err(errorf("Unexpected positional arguments"));
    }
    let Some(rule_class) = state.rule_class.get().cloned() else {
        return Err(errorf(
            "Invalid rule class hasn't been exported by a bzl file",
        ));
    };
    // PORT: Java keeps the `targetDefinitionContext` reference across the initializers; here
    // the builder is the thread's payload, so what the initializer loop needs from it (its
    // label converter and main repo mapping) is taken now.
    let (package_label_converter, main_repo_mapping) = {
        let target_definition_context =
            TargetDefinitionContext::from_or_fail_participle(eval, "a rule", "instantiated")
                .map_err(eval_exception)?;
        (
            target_definition_context.get_label_converter_arc().clone(),
            target_definition_context
                .get_main_repository_mapping()
                .clone(),
        )
    };
    let thread = ThreadContext::get(eval)?;

    validate_rule_propagated_aspects(&rule_class)?;

    let legacy_any_type_attrs = get_legacy_any_type_attrs(&rule_class);

    let mut kwargs: IndexMap<String, Value<'v>> = args
        .names_map()?
        .into_iter()
        .map(|(k, v)| (k.as_str().to_owned(), v))
        .collect();

    let heap = eval.heap();
    // Java's `try { ... } finally { ... }`: the state is restored when this guard drops.
    let initializer_thread_state =
        InitializerThreadState::enter(thread, package_label_converter.clone());
    // We call all the initializers of the rule and its ancestor rules, proceeding from child to
    // ancestor, so each initializer can transform the attributes it knows about in turn.
    let mut current_rule_class = Some(rule_class.clone());
    while let Some(current) = current_rule_class {
        let Some(initializer) = current.get_initializer() else {
            current_rule_class = current.get_starlark_parent().cloned();
            continue;
        };

        // You might feel tempted to inspect the signature of the initializer function. The
        // temptation might come from handling default values, making them work for better for
        // the users. The less magic the better. Do not give in those temptations!
        let mut initializer_kwargs: Vec<(String, Value<'v>)> = Vec::new();
        for attr in current.get_attribute_provider().get_attributes() {
            if ((attr.is_public() && attr.starlark_defined()) || attr.get_name() == "name")
                && let Some(value) = kwargs.get(attr.get_name()).copied()
            {
                if value.is_none() {
                    continue;
                }
                let reified_value = if legacy_any_type_attrs.contains(attr.get_name()) {
                    value
                } else {
                    crate::types::build_type::copy_and_lift_starlark_value(
                        current.get_name(),
                        attr,
                        value,
                        Some(package_label_converter.as_ref()),
                        heap,
                    )
                    .map_err(|e| errorf(e.to_string()))?
                };
                initializer_kwargs.push((attr.get_name().to_owned(), reified_value));
            }
        }
        let initializer_fn = resolve(initializer)
            .expect("rule initializer used before its module was frozen")
            .to_value();
        let named: Vec<(&str, Value<'v>)> = initializer_kwargs
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        let ret = eval.eval_function(initializer_fn, &[], &named)?;
        let new_kwargs: Vec<(Value<'v>, Value<'v>)> = if ret.is_none() {
            Vec::new()
        } else {
            dict_cast(
                ret,
                "string",
                |k| k.unpack_str().is_some(),
                "Object",
                |_| true,
                "rule's initializer return value",
            )?
        };

        for (arg, value) in new_kwargs {
            let arg = arg.unpack_str().unwrap();
            if arg == "name" {
                let old = kwargs.get("name").copied();
                if !old.is_some_and(|o| o.equals(value).unwrap_or(false)) {
                    return Err(errorf("Initializer can't change the name of the target"));
                }
                continue;
            }
            check_attribute_name(arg)?;
            if arg.starts_with('_') {
                // allow setting private attributes from initializers in builtins
                let definition_label = current
                    .get_rule_definition_environment_label()
                    .expect("Starlark rule class has a definition label");
                fail_if_label_outside_allowlist(
                    definition_label,
                    &main_repo_mapping,
                    ALLOWLIST_RULE_EXTENSION_API_EXPERIMENTAL,
                )?;
            }
            let native_name = match arg.strip_prefix('_') {
                Some(rest) => format!("${rest}"),
                None => arg.to_owned(),
            };
            let attr = current
                .get_attribute_provider()
                .get_attribute_by_name_maybe(&native_name);
            if let Some(attr) = attr
                && !attr.starlark_defined()
            {
                return Err(errorf(format!(
                    "Initializer can only set Starlark defined attributes, not '{arg}'"
                )));
            }
            let reified_value = match attr {
                Some(attr)
                    if !value.is_none() && !legacy_any_type_attrs.contains(attr.get_name()) =>
                {
                    // Reify to the location of the initializer definition (except for outputs)
                    if attr.get_type() == Type::Output || attr.get_type() == Type::OutputList {
                        crate::types::build_type::copy_and_lift_starlark_value(
                            current.get_name(),
                            attr,
                            value,
                            Some(package_label_converter.as_ref()),
                            heap,
                        )
                        .map_err(|e| errorf(e.to_string()))?
                    } else {
                        crate::types::build_type::copy_and_lift_starlark_value(
                            current.get_name(),
                            attr,
                            value,
                            current
                                .get_label_converter_for_initializer()
                                .map(|c| c.as_ref()),
                            heap,
                        )
                        .map_err(|e| errorf(e.to_string()))?
                    }
                }
                _ => value,
            };
            kwargs.insert(native_name, reified_value);
        }
        current_rule_class = current.get_starlark_parent().cloned();
    }
    // Java's `finally`: `thread.setThreadLocal(LabelConverter.class, null)` and
    // `targetDefinitionContext.storeInThread(thread)`.
    drop(initializer_thread_state);

    let attribute_values = BuildLangTypedAttributeValuesMap::new(kwargs);
    let fail_on_unknown_attributes = thread.semantics.incompatible_fail_on_unknown_attributes;
    let callstack = call_stack(eval);
    let mut target_definition_context =
        TargetDefinitionContext::from_or_fail_participle(eval, "a rule", "instantiated")
            .map_err(eval_exception)?;
    crate::package::rule_factory::create_and_add_rule(
        &mut target_definition_context,
        &rule_class,
        &attribute_values,
        fail_on_unknown_attributes,
        &callstack,
    )
    .map_err(|e| errorf(e.to_string()))?;
    Ok(Value::new_none())
}

#[starlark_value(type = "rule")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkRuleFunctionGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        call_rule_function(&self.state, args, eval)
    }

    /// The name in tracebacks and "Error in <name>:" (Java `StarlarkCallable.getName`).
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1762 StarlarkRuleFunction.getName
    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        match self.state.rule_class.get() {
            Some(rule_class) => rule_class.get_name().to_owned(),
            None => "unexported rule".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2008 StarlarkRuleFunction.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match rule_function_state(other) {
            Some(o) => match (self.state.exported.get(), o.exported.get()) {
                (Some(a), Some(b)) => a == b,
                _ => Arc::ptr_eq(&self.state, &o),
            },
            None => false,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2016 StarlarkRuleFunction.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.state.exported.get() {
            Some(e) => e.hash(hasher),
            None => self.state.symbol.hash(hasher),
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1911 StarlarkRuleFunction.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if self.state.exported.get().is_some() {
            return Ok(());
        }
        let Some(ctx) = BzlInitThreadContext::from_or_null(eval) else {
            // PORT: Java exports analysis_test rules from BUILD files (AnalysisTestKey);
            // testing.analysis_test is unported.
            return Ok(());
        };
        let starlark_label = ctx.bzl_file.clone();
        drop(ctx);
        let parent_subrules = match self.parent {
            Some(p) => rule_function_subrules(p.to_value()),
            None => Vec::new(),
        };
        let subrules: Vec<Value<'v>> = self.subrules.iter().map(|s| s.to_value()).collect();
        export_rule_function(
            &self.state,
            &starlark_label,
            variable_name,
            &subrules,
            &parent_subrules,
        )
    }
}

/// The export body of `StarlarkRuleFunction.export`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1911 StarlarkRuleFunction.export
fn export_rule_function(
    state: &RuleFunctionState,
    starlark_label: &Label,
    rule_class_name: &str,
    subrules: &[Value],
    parent_subrules: &[Value],
) -> starlark::Result<()> {
    let mut guard = state.builder.lock();
    let Some(mut builder) = guard.take() else {
        return Ok(());
    };
    drop(guard);
    let _ = state
        .exported
        .set((starlark_label.clone(), rule_class_name.to_owned()));
    // PORT: Java reports these errors as events at the rule() call and continues; Blaze fails
    // the export (the assignment) with the same message.
    let errorf_at = |msg: String| errorf(msg);

    if (builder.get_type() == RuleClassType::Test)
        != base_rule_classes::is_test_rule_name(rule_class_name)
    {
        return Err(errorf_at(format!(
            "Invalid rule class name '{rule_class_name}', test rule class names must end with \
             '_test' and other rule classes must not"
        )));
    }

    // lift the subrule attributes to the rule class as if they were declared there, this lets
    // us exploit dependency resolution for "free"
    let subrules_not_in_parents: Vec<Value> = subrules
        .iter()
        .copied()
        .filter(|s| {
            !parent_subrules
                .iter()
                .any(|p| subrule::subrule_equals(*s, *p))
        })
        .collect();
    let subrule_attributes = subrule::discover_attributes(&subrules_not_in_parents)
        .map_err(|e| errorf_at(e.to_string()))?;
    for (name, descriptor) in subrule_attributes {
        let attr = descriptor.build(&name);
        catch_illegal_state(|| {
            builder.add_attribute(attr);
        })
        .map_err(|m| errorf_at(format!("cannot add attribute: {m}")))?;
    }

    builder.set_subrule_attribute_names(subrule::subrule_attribute_names(subrules));

    let rule_class =
        catch_illegal_state(|| builder.build_starlark(rule_class_name, starlark_label.clone()))
            .map_err(errorf_at)?;
    let _ = state.rule_class.set(Arc::new(rule_class));
    Ok(())
}

// -------------------------------------------------------------------------------------------
// createRule

fn check_and_add_allowlist_if_necessary(
    builder: &mut RuleClassBuilder,
    env: &dyn RuleDefinitionEnvironment,
    uses_functionality: bool,
    bzl_file_label: &Label,
    allowlist_checker: AllowlistChecker,
    description: &str,
    attribute_factory: fn(&dyn RuleDefinitionEnvironment) -> AttributeBuilder,
    attribute_name: &str,
    label_str: &str,
) -> starlark::Result<()> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1195 StarlarkRuleClassFunctions.checkAndAddAllowlistIfNecessary
    let label = Label::parse_canonical_unchecked(label_str);
    let mut has_allowlist = false;
    // Check for existence of the allowlist attribute.
    if builder.contains(attribute_name) {
        let attr = builder.get_attribute(attribute_name).unwrap();
        if !attr.get_type().is_label_type() {
            return Err(errorf(format!(
                "{} attribute must be a label type",
                Attribute::get_starlark_name(attribute_name)
            )));
        }
        let Some(AttrValue::Label(default_label)) = attr.get_default_value_unchecked() else {
            return Err(errorf(format!(
                "{} attribute must have a default value",
                Attribute::get_starlark_name(attribute_name)
            )));
        };
        // Check the label value for package and target name, to make sure this works properly
        // in Bazel where it is expected to be found under @bazel_tools.
        if !(default_label.get_package_name() == label.get_package_name()
            && default_label.get_name() == label.get_name())
        {
            return Err(errorf(format!(
                "{} attribute ({}) does not have the expected value {}",
                Attribute::get_starlark_name(attribute_name),
                default_label,
                label
            )));
        }
        has_allowlist = true;
    }
    if uses_functionality {
        if bzl_file_label.get_repository().get_name() != "_builtins" {
            if !has_allowlist {
                // add the allowlist automatically
                builder.add(&attribute_factory(env));
            }
            builder.add_allowlist_checker(allowlist_checker);
        }
    } else if has_allowlist {
        return Err(errorf(format!(
            "Unused {description} allowlist: {} {:?}",
            builder
                .get_rule_definition_environment_label()
                .map(|l| l.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            builder.get_type()
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1173 StarlarkRuleClassFunctions.createStarlarkFunctionTransitionAllowlistAttribute
fn create_starlark_function_transition_allowlist_attribute(
    env: &dyn RuleDefinitionEnvironment,
) -> AttributeBuilder {
    let mut b = attr(FUNCTION_TRANSITION_ATTRIBUTE_NAME, Type::Label);
    b.cfg(default_exec_factory())
        .mandatory_builtin_providers(vec!["PackageSpecificationProvider".to_owned()])
        .value(Some(AttrValue::Label(
            env.get_tools_label(FUNCTION_TRANSITION_LABEL_STR),
        )));
    b
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1181 StarlarkRuleClassFunctions.createDormantDependencyAllowlistAttribute
fn create_dormant_dependency_allowlist_attribute(
    env: &dyn RuleDefinitionEnvironment,
) -> AttributeBuilder {
    let mut b = attr(DORMANT_DEPENDENCY_ALLOWLIST_ATTRIBUTE_NAME, Type::Label);
    b.cfg(default_exec_factory())
        .mandatory_builtin_providers(vec!["PackageSpecificationProvider".to_owned()]);
    b.set_property_flag("FOR_DEPENDENCY_RESOLUTION")
        .and_then(|b| b.set_property_flag("FOR_DEPENDENCY_RESOLUTION_EXPLICITLY_SET"))
        .expect("IllegalStateException");
    b.value(Some(AttrValue::Label(
        env.get_tools_label(DORMANT_DEPENDENCY_ALLOWLIST_LABEL_STR),
    )));
    b
}

/// An allowlist attribute `$allowlist_<name>` with `default` (the shape Java repeats in
/// createRule).
fn allowlist_attr(name: &str, default: Label) -> AttributeBuilder {
    let mut b = attr(name, Type::Label);
    b.cfg(default_exec_factory())
        .mandatory_builtin_providers(vec!["PackageSpecificationProvider".to_owned()])
        .value(Some(AttrValue::Label(default)));
    b
}

/// The arguments of `rule()` after `rule()`'s own checks (Java `createRule` parameters).
struct CreateRuleArgs<'v> {
    parent: Option<(Value<'v>, Arc<RuleClass>)>,
    extendable_unchecked: Option<Value<'v>>,
    implementation: Value<'v>,
    initializer: Option<Value<'v>>,
    test: bool,
    attrs: Value<'v>,
    implicit_outputs: Value<'v>,
    executable: bool,
    output_to_genfiles: bool,
    fragments: Value<'v>,
    starlark_testable: bool,
    toolchains: Value<'v>,
    doc: Option<&'v str>,
    provides_arg: Value<'v>,
    dependency_resolution_rule: bool,
    is_materializer_rule: bool,
    allow_materializer_rule_real_deps: bool,
    exec_compatible_with: Value<'v>,
    analysis_test: bool,
    build_setting: Value<'v>,
    cfg: Value<'v>,
    exec_groups: Value<'v>,
    subrules_unchecked: Value<'v>,
}

/// Returns a new callable representing a Starlark-defined rule.
///
/// Note that the bzlFile and transitiveDigest params correspond to the outermost .bzl file being
/// evaluated, not the one in which rule() is called.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:717 StarlarkRuleClassFunctions.createRule
fn create_rule<'v>(
    eval: &mut Evaluator<'v, '_, '_>,
    env: &dyn RuleDefinitionEnvironment,
    bzl_file: &Label,
    transitive_digest: &[u8],
    label_converter: crate::types::LabelConverter,
    a: CreateRuleArgs<'v>,
) -> starlark::Result<StarlarkRuleFunction<'v>> {
    let semantics = ThreadContext::get(eval)?.semantics.clone();
    let mut pending: Vec<PendingValue<'v>> = Vec::new();
    let is_builtins = bzl_file.get_repository().get_name() == "_builtins";

    // analysis_test=true implies test=true.
    let test = a.test || a.analysis_test;

    let type_ = if test {
        RuleClassType::Test
    } else {
        RuleClassType::Normal
    };

    let parent = a.parent.as_ref().map(|(_, rc)| rc.clone());
    let mut builder = if a.is_materializer_rule {
        if parent.is_some() {
            return Err(errorf("materializer rules cannot have a parent"));
        }
        let mut b = RuleClassBuilder::new(
            "",
            type_,
            true,
            std::slice::from_ref(&*MATERIALIZER_BASE_RULE),
        );
        b.set_materializer_rule_allows_real_deps(a.allow_materializer_rule_real_deps);
        b
    } else if a.dependency_resolution_rule {
        if parent.is_some() {
            return Err(errorf(
                "rules used in dependency resolution cannot have a parent",
            ));
        }
        RuleClassBuilder::new(
            "",
            type_,
            true,
            std::slice::from_ref(&*DEPENDENCY_RESOLUTION_BASE_RULE),
        )
    } else if let Some(parent) = &parent {
        if parent.is_dependency_resolution_rule() {
            return Err(errorf("dependency resolution rules cannot be parents"));
        }
        if parent.is_materializer_rule() {
            return Err(errorf("materializer rules cannot be parents"));
        }
        // We'll set the name later, pass the empty string for now.
        RuleClassBuilder::new("", type_, true, std::slice::from_ref(parent))
    } else {
        // We'll set the name later, pass the empty string for now.
        let base_parent = if test {
            get_test_base_rule(env)
        } else if a.executable {
            BINARY_BASE_RULE.clone()
        } else {
            BASE_RULE.clone()
        };
        RuleClassBuilder::new("", type_, true, &[base_parent])
    };

    let label_converter = Arc::new(label_converter);
    if let Some(initializer) = a.initializer {
        let p = PendingValue::new(initializer, eval);
        builder.initializer(p.handle, label_converter.clone());
        pending.push(p);
    }

    builder.set_default_extendable_allowlist(
        env.get_tools_label("//tools/allowlists/extend_rule_allowlist"),
    );
    match a.extendable_unchecked {
        Some(e) if e.unpack_bool().is_some() => builder.set_extendable(e.unpack_bool().unwrap()),
        Some(e) if e.unpack_str().is_some() => {
            let s = e.unpack_str().unwrap();
            match label_converter.convert(s) {
                Ok(l) => builder.set_extendable_by_allowlist(l),
                Err(err) => {
                    return Err(errorf(format!(
                        "Unable to parse label '{s}': {}",
                        err.get_message()
                    )));
                }
            }
        }
        Some(e)
            if e.downcast_ref::<blaze_starlark::label::LabelValue>()
                .is_some() =>
        {
            builder.set_extendable_by_allowlist(
                e.downcast_ref::<blaze_starlark::label::LabelValue>()
                    .unwrap()
                    .label()
                    .clone(),
            )
        }
        Some(e) if !e.is_none() => {
            return Err(errorf(format!(
                "parameter 'extendable': expected bool, str or Label, but got '{}'",
                starlark_type(e)
            )));
        }
        _ => {}
    }

    // Verify the child against parent's allowlist
    if let Some(parent) = &parent
        && let Some(parent_allowlist) = parent.get_extendable_allowlist()
        && !is_builtins
    {
        builder.add_allowlist_checker(extend_rule_allowlist_checker());
        let allowlist_attr = allowlist_attr("$allowlist_extend_rule", parent_allowlist.clone());
        if builder.contains("$allowlist_extend_rule") {
            // the allowlist already exist if this is the second extension of the rule
            // in this case we need to override the allowlist with the one in the direct parent
            builder.override_(&allowlist_attr);
        } else {
            builder.add(&allowlist_attr);
        }
    }

    if parent.is_some() && !semantics.experimental_rule_extension_api && !is_builtins {
        builder.add_allowlist_checker(extend_rule_api_allowlist_checker());
        if !builder.contains("$allowlist_extend_rule_api") {
            builder.add(&allowlist_attr(
                "$allowlist_extend_rule_api",
                env.get_tools_label(
                    "//tools/allowlists/extend_rule_allowlist:extend_rule_api_allowlist",
                ),
            ));
        }
    }

    if a.initializer.is_some() && !semantics.experimental_rule_extension_api && !is_builtins {
        builder.add_allowlist_checker(initializer_allowlist_checker());
        if !builder.contains("$allowlist_initializer") {
            // the allowlist already exist if this is an extended rule
            builder.add(&allowlist_attr(
                "$allowlist_initializer",
                env.get_tools_label("//tools/allowlists/initializer_allowlist"),
            ));
        }
    }

    let subrule_values = sequence_cast(
        a.subrules_unchecked,
        "Subrule",
        |s| subrule::is_subrule(s),
        "subrules",
    )?;
    if !subrule_values.is_empty() && !semantics.experimental_rule_extension_api && !is_builtins {
        builder.add_allowlist_checker(subrules_allowlist_checker());
        if !builder.contains("$allowlist_subrules") {
            // the allowlist already exist if this is an extended rule
            builder.add(&allowlist_attr(
                "$allowlist_subrules",
                env.get_tools_label("//tools/allowlists/subrules_allowlist"),
            ));
        }
    }

    if a.is_materializer_rule {
        builder.add_allowlist_checker(materializer_rule_allowlist_checker());
        if !builder.contains("$allowlist_materializer_rule") {
            // the allowlist already exists if this is an extended rule
            builder.add(&allowlist_attr(
                "$allowlist_materializer_rule",
                env.get_tools_label("//tools/allowlists/materializer_rule_allowlist"),
            ));
        }

        if a.allow_materializer_rule_real_deps {
            builder.add_allowlist_checker(materializer_rule_real_deps_allowlist_checker());
            if !builder.contains("$allowlist_materializer_rule_real_deps") {
                // the allowlist already exists if this is an extended rule
                builder.add(&allowlist_attr(
                    "$allowlist_materializer_rule_real_deps",
                    env.get_tools_label(
                        "//tools/allowlists/materializer_rule_allowlist\
                         :materializer_rule_real_deps_allowlist",
                    ),
                ));
            }
        }
    }

    if a.executable || test {
        builder.set_executable_starlark();
    }

    // Get the callstack, sans the last entry, which is the builtin 'rule' callable itself.
    let mut callstack = call_stack(eval);
    callstack.pop();
    builder.set_call_stack(callstack);

    let attributes = attr_object_to_attributes_list(a.attrs)?;

    if a.starlark_testable {
        builder.set_starlark_testable();
    }
    if a.analysis_test {
        builder.set_is_analysis_test();
    }

    let mut has_starlark_defined_transition = false;
    let mut propagates_aspects = false;
    let mut has_materializers = false;
    let mut dormant_attributes: Vec<String> = Vec::new();

    for (name, descriptor) in &attributes {
        let mut attr = descriptor.build(name);
        pending.extend(descriptor.pending_values().iter().copied());
        let is_dependency = attr.get_type().get_label_class() == LabelClass::Dependency;

        if a.dependency_resolution_rule && attr.is_materializing() {
            return Err(errorf(format!(
                "attribute '{name}' has a materializer which is not allowed on rules for \
                 dependency resolution"
            )));
        }

        if a.dependency_resolution_rule && is_dependency {
            if !attr.is_for_dependency_resolution()
                && attr.for_dependency_resolution_explicitly_set()
            {
                return Err(errorf(format!(
                    "attribute '{name}' is explicitly marked as not for dependency resolution, \
                     which is disallowed on rules for dependency resolution"
                )));
            }
            let mut b = attr.clone_builder();
            b.set_property_flag("FOR_DEPENDENCY_RESOLUTION")
                .map_err(eval_exception)?
                .nonconfigurable("On a rule used in dependency resolution");
            attr = b.build();
        }

        // "configurable" may only be user-set for symbolic macros, not rules.
        if attr.configurable_attr_was_user_set() {
            return Err(errorf(format!(
                "attribute '{name}' has the 'configurable' argument set, which is not allowed in \
                 rule definitions"
            )));
        }
        if attr.skip_validations() {
            // This is mitigation for internal Blaze builds, and not planned to be a Bazel
            // feature, and therefore has no extendable allowlists.
            if !builder.contains("$allowlist_skip_validations") {
                builder.add(&allowlist_attr(
                    "$allowlist_skip_validations",
                    Label::parse_canonical_unchecked(
                        "//tools/allowlists/skip_validations_allowlist",
                    ),
                ));
                builder.add_allowlist_checker(skip_validations_allowlist_checker());
            }
        }

        if attr.get_aspects_list().has_aspects() {
            propagates_aspects = true;
        }

        has_starlark_defined_transition |= attr.has_starlark_defined_transition();
        if attr.has_analysis_test_transition() {
            if !builder.is_analysis_test() {
                return Err(errorf(
                    "Only rule definitions with analysis_test=True may have attributes with \
                     analysis_test_transition transitions",
                ));
            }
            builder.set_has_analysis_test_transition();
        }

        if attr.get_type() == Type::DormantLabel || attr.get_type() == Type::DormantLabelList {
            dormant_attributes.push(name.clone());
        }

        if attr.is_materializing() {
            has_materializers = true;
        }

        let result = if builder.contains(attr.get_name()) {
            match catch_illegal_state(|| builder.override_starlark(&attr).map(|_| ())) {
                Ok(r) => r.map_err(eval_exception),
                Err(m) => Err(errorf(format!("cannot add attribute: {m}"))),
            }
        } else {
            catch_illegal_state(|| {
                builder.add_attribute(attr.clone());
            })
            .map_err(|m| errorf(format!("cannot add attribute: {m}")))
        };
        result?;
    }

    // the set of subrules is stored in the rule class, primarily for validating that a rule
    // class declared the subrule when using it.
    builder.add_toolchain_types(subrule::discover_toolchains(&subrule_values));
    let mut subrule_handles = Vec::new();
    for s in &subrule_values {
        let p = PendingValue::new(*s, eval);
        subrule_handles.push(p.handle);
        pending.push(p);
    }
    builder.set_subrules(subrule_handles);

    if !a.implicit_outputs.is_none() {
        if is_starlark_function(a.implicit_outputs) {
            let p = PendingValue::new(a.implicit_outputs, eval);
            builder.set_implicit_outputs_function(ImplicitOutputsFunction::StarlarkWithCallback {
                callback: p.handle,
            });
            pending.push(p);
        } else {
            let map = super::util::dict_cast_string_string(
                a.implicit_outputs,
                "implicit outputs of the rule class",
            )?;
            builder.set_implicit_outputs_function(ImplicitOutputsFunction::StarlarkWithMap(map));
        }
    }

    if a.output_to_genfiles {
        builder.set_output_to_genfiles();
    }

    builder.requires_configuration_fragments_by_starlark_module_name(&sequence_cast_strings(
        a.fragments,
        "fragments",
    )?);
    let implementation = PendingValue::new(a.implementation, eval);
    builder.set_configured_target_function(implementation.handle);
    pending.push(implementation);

    // The rule definition's label and transitive digest typically come from the context of the
    // .bzl file being initialized.
    //
    // Note that if rule() was called via a helper function (a meta-macro), the label and digest
    // of the .bzl file of the innermost stack frame might not be the same as that of the
    // outermost frame. In this case we really do want the outermost, in order to ensure that the
    // digest includes the code that determines the helper function's argument values.
    builder.set_rule_definition_environment_label_and_digest(
        bzl_file.clone(),
        transitive_digest.to_vec(),
    );

    builder.add_toolchain_types(parse_toolchain_types(a.toolchains, &label_converter)?);

    if !a.exec_groups.is_none() {
        let override_ = parent.is_some();
        let exec_group_dict = parse_exec_groups(a.exec_groups)?;
        builder.add_exec_groups(&exec_group_dict, override_);
    }
    if test && !builder.has_exec_group(DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME) {
        let mut groups = IndexMap::new();
        groups.insert(
            DEFAULT_TEST_RUNNER_EXEC_GROUP_NAME.to_owned(),
            default_test_runner_exec_group(),
        );
        builder.add_exec_groups(&groups, false);
    }

    if !a.build_setting.is_none() && !a.cfg.is_none() {
        return Err(errorf(
            "Build setting rules cannot use the `cfg` param to apply transitions to themselves.",
        ));
    }
    if !a.build_setting.is_none() {
        let Some(bs) = a.build_setting.downcast_ref::<BuildSettingValue>() else {
            return Err(param_type_error(
                "rule",
                "build_setting",
                a.build_setting,
                "BuildSetting or NoneType",
            ));
        };
        builder.set_build_setting(bs.0.clone());
    }

    let converted = convert_config(eval, a.cfg)?;
    let mut transition_factory = converted.factory;
    pending.extend(converted.pending);
    // PORT: StarlarkExposedRuleTransitionFactory (native Android transitions) is not exposed.
    if let Some(parent) = &parent {
        transition_factory = compose_transition_factories(
            transition_factory,
            parent.get_transition_factory().clone(),
        );
    }
    // Check if the transition has any Starlark code.
    has_starlark_defined_transition |= transition_has_starlark(&transition_factory);
    builder.cfg(transition_factory);

    check_and_add_allowlist_if_necessary(
        &mut builder,
        env,
        a.dependency_resolution_rule || has_materializers,
        bzl_file,
        dormant_dependency_allowlist_checker(),
        "dormant dependency",
        create_dormant_dependency_allowlist_attribute,
        DORMANT_DEPENDENCY_ALLOWLIST_ATTRIBUTE_NAME,
        DORMANT_DEPENDENCY_ALLOWLIST_LABEL_STR,
    )?;

    check_and_add_allowlist_if_necessary(
        &mut builder,
        env,
        has_starlark_defined_transition,
        bzl_file,
        function_transition_allowlist_checker(),
        "function-based split transition",
        create_starlark_function_transition_allowlist_attribute,
        FUNCTION_TRANSITION_ATTRIBUTE_NAME,
        FUNCTION_TRANSITION_LABEL_STR,
    )?;

    if a.dependency_resolution_rule {
        if !subrule_values.is_empty() {
            return Err(errorf(
                "Rules that can be required for materializers cannot have subrules",
            ));
        }
        if !sequence_elements(a.toolchains)
            .unwrap_or_default()
            .is_empty()
        {
            return Err(errorf(
                "Rules that can be required for materializers cannot depend on toolchains",
            ));
        }
        if propagates_aspects {
            return Err(errorf(
                "Rules that can be required for materializes cannot propagate aspects",
            ));
        }
    }

    if !dormant_attributes.is_empty() && !a.dependency_resolution_rule && !a.is_materializer_rule {
        return Err(errorf(format!(
            "Has dormant attributes ({}) but is not marked as allowed in materializers",
            dormant_attributes
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let provides = get_starlark_provider_identifiers(
        &sequence_cast(a.provides_arg, "Provider", |_| true, "provides")?,
        "provides",
    )?;
    builder.advertise_starlark_provider(provides);

    if !sequence_elements(a.exec_compatible_with)
        .unwrap_or_default()
        .is_empty()
    {
        builder.add_execution_platform_constraints(parse_labels(
            a.exec_compatible_with,
            &label_converter,
            "exec_compatible_with",
        )?);
    }

    if let Some(doc) = a.doc {
        builder.set_starlark_documentation(trim_doc_string(doc));
    }

    let symbol = ThreadContext::get(eval)?.next_symbol();
    Ok(StarlarkRuleFunctionGen {
        parent: a.parent.map(|(v, _)| v),
        subrules: subrule_values,
        handles: pending.iter().map(|p| p.handle).collect(),
        pending: pending.iter().map(|p| p.value).collect(),
        state: Arc::new(RuleFunctionState {
            builder: Mutex::new(Some(builder)),
            rule_class: OnceLock::new(),
            definition_location: caller_location(eval),
            symbol,
            exported: OnceLock::new(),
        }),
    })
}

/// Java `ComposingTransitionFactory.ofUnchecked(transitionFactory, parentFactory)`.
// PORT: A1's TransitionFactory has no composition variant (requested); composition with no
// transition yields the other factory, as Java's ComposingTransitionFactory.of does.
// Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java ComposingTransitionFactory.ofUnchecked
fn compose_transition_factories(
    transition_factory1: TransitionFactory,
    transition_factory2: TransitionFactory,
) -> TransitionFactory {
    if transition_factory1.is_no_transition() {
        return transition_factory2;
    }
    if transition_factory2.is_no_transition() {
        return transition_factory1;
    }
    blaze_util::unported!("ComposingTransitionFactory.ofUnchecked (two real transitions)")
}

/// Java `StarlarkTransitionCheckingVisitor`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2263 StarlarkRuleClassFunctions.StarlarkTransitionCheckingVisitor
fn transition_has_starlark(factory: &TransitionFactory) -> bool {
    matches!(factory, TransitionFactory::Starlark { .. })
}

/// A view of a rule function value's rule class, for `rule(parent=...)` and
/// `macro(inherit_attrs=...)`: `Err(())` if it is not a Starlark rule function.
pub fn rule_function_rule_class(v: Value) -> Option<Arc<RuleFunctionState>> {
    rule_function_state(v)
}

// -------------------------------------------------------------------------------------------
// The builtins

fn bool_or_unbound(func: &str, param: &str, v: Option<Value>) -> starlark::Result<Option<bool>> {
    match v {
        None => Ok(None),
        Some(v) => match v.unpack_bool() {
            Some(b) => Ok(Some(b)),
            None => Err(param_type_error(func, param, v, "bool")),
        },
    }
}

fn empty_list<'v>(eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
    eval.heap().alloc(Vec::<Value>::new())
}

fn empty_dict<'v>(eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
    eval.heap()
        .alloc(starlark::values::dict::AllocDict(std::iter::empty::<(
            Value,
            Value,
        )>()))
}

fn optional_str<'v>(
    func: &str,
    param: &str,
    v: Option<Value<'v>>,
) -> starlark::Result<Option<&'v str>> {
    match v {
        None => Ok(None),
        Some(v) if v.is_none() => Ok(None),
        Some(v) => match v.unpack_str() {
            Some(s) => Ok(Some(s)),
            None => Err(param_type_error(func, param, v, "string or NoneType")),
        },
    }
}

#[starlark_module]
pub fn register_rule_class_functions(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:549 StarlarkRuleClassFunctions.rule
    fn rule<'v>(
        implementation: Value<'v>,
        #[starlark(require = named)] test: Option<Value<'v>>,
        #[starlark(require = named)] attrs: Option<Value<'v>>,
        #[starlark(require = named)] outputs: Option<Value<'v>>,
        #[starlark(require = named)] executable: Option<Value<'v>>,
        #[starlark(require = named, default = false)] output_to_genfiles: bool,
        #[starlark(require = named)] fragments: Option<Value<'v>>,
        #[starlark(require = named)] host_fragments: Option<Value<'v>>,
        #[starlark(require = named, default = false)] _skylark_testable: bool,
        #[starlark(require = named)] toolchains: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named)] provides: Option<Value<'v>>,
        #[starlark(require = named, default = false)] dependency_resolution_rule: bool,
        #[starlark(require = named)] exec_compatible_with: Option<Value<'v>>,
        #[starlark(require = named, default = false)] analysis_test: bool,
        #[starlark(require = named)] build_setting: Option<Value<'v>>,
        #[starlark(require = named)] cfg: Option<Value<'v>>,
        #[starlark(require = named)] exec_groups: Option<Value<'v>>,
        #[starlark(require = named)] initializer: Option<Value<'v>>,
        #[starlark(require = named)] parent: Option<Value<'v>>,
        #[starlark(require = named)] extendable: Option<Value<'v>>,
        #[starlark(require = named)] subrules: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkRuleFunction<'v>> {
        if !is_starlark_function(implementation) {
            return Err(param_type_error(
                "rule",
                "implementation",
                implementation,
                "function",
            ));
        }
        let test_unchecked = bool_or_unbound("rule", "test", test)?;
        // Java: `@Param(name = "attrs", allowedTypes = {@ParamType(type = Dict.class)})`
        // (lib/starlarkbuildapi/StarlarkRuleFunctionsApi.java:267), checked by the interpreter
        // before the method body, in parameter order.
        if let Some(attrs) = attrs
            && starlark::values::dict::DictRef::from_value(attrs).is_none()
        {
            return Err(param_type_error("rule", "attrs", attrs, "dict"));
        }
        let executable_unchecked = bool_or_unbound("rule", "executable", executable)?;
        let doc = optional_str("rule", "doc", doc)?;
        let outputs = outputs.unwrap_or_else(Value::new_none);
        if !outputs.is_none()
            && ThreadContext::get(eval)?
                .semantics
                .incompatible_no_rule_outputs_param
        {
            // Java: @Param(disableWithFlag = INCOMPATIBLE_NO_RULE_OUTPUTS_PARAM)
            return Err(errorf(
                "parameter 'outputs' is deprecated and will be removed soon. It may be \
                 temporarily re-enabled by setting --incompatible_no_rule_outputs_param=false",
            ));
        }
        let attrs = attrs.unwrap_or_else(|| empty_dict(eval));
        let fragments = fragments.unwrap_or_else(|| empty_list(eval));
        let host_fragments = host_fragments.unwrap_or_else(|| empty_list(eval));
        let toolchains = toolchains.unwrap_or_else(|| empty_list(eval));
        let provides = provides.unwrap_or_else(|| empty_list(eval));
        let exec_compatible_with = exec_compatible_with.unwrap_or_else(|| empty_list(eval));
        let build_setting = build_setting.unwrap_or_else(Value::new_none);
        let cfg = cfg.unwrap_or_else(Value::new_none);
        let exec_groups = exec_groups.unwrap_or_else(Value::new_none);
        let initializer = initializer.filter(|v| !v.is_none());
        let parent_unchecked = parent.unwrap_or_else(Value::new_none);
        let subrules = subrules.unwrap_or_else(|| empty_list(eval));

        // Ensure we're initializing a .bzl file, which also means we have a
        // RuleDefinitionEnvironment.
        let (bzl_file, transitive_digest, tools_repository) = {
            let bazel_context =
                BzlInitThreadContext::from_or_fail(eval, "rule()").map_err(eval_exception)?;
            (
                bazel_context.bzl_file.clone(),
                bazel_context.transitive_digest.clone(),
                bazel_context.tools_repository.clone(),
            )
        };

        let parent_rule;
        let executable_flag;
        let test_flag;

        if parent_unchecked.is_none() {
            parent_rule = None;
            executable_flag = executable_unchecked.unwrap_or(false);
            test_flag = test_unchecked.unwrap_or(false);
        } else {
            let Some(parent_state) = rule_function_state(parent_unchecked) else {
                return Err(errorf(format!(
                    "Parent needs to be a Starlark rule, was {}",
                    starlark_type(parent_unchecked)
                )));
            };
            // Assuming parent is already exported.
            let Some(parent_rc) = parent_state.rule_class().cloned() else {
                return Err(errorf("Please export the parent rule before extending it."));
            };
            executable_flag = parent_rc.is_executable_starlark();
            test_flag = parent_rc.get_rule_class_type() == RuleClassType::Test;

            if !parent_rc.is_extendable() {
                return Err(errorf(format!(
                    "The rule '{}' is not extendable. Only Starlark rules not using deprecated \
                     features (like implicit outputs, output to genfiles) may be extended. \
                     Special rules like analysis tests or rules using build_settings cannot be \
                     extended.",
                    parent_rc.get_name()
                )));
            }
            if executable_unchecked.is_some() {
                return Err(errorf("Omit executable parameter when extending rules."));
            }
            if test_unchecked.is_some() {
                return Err(errorf("Omit test parameter when extending rules."));
            }
            if !outputs.is_none() {
                return Err(errorf(
                    "implicit_outputs is not supported when extending rules (deprecated).",
                ));
            }
            if !sequence_elements(host_fragments)
                .unwrap_or_default()
                .is_empty()
            {
                return Err(errorf(
                    "host_fragments are not supported when extending rules (deprecated).",
                ));
            }
            if output_to_genfiles {
                return Err(errorf(
                    "output_to_genfiles are not supported when extending rules (deprecated).",
                ));
            }
            if _skylark_testable {
                return Err(errorf(
                    "_skylark_testable is not supported when extending rules.",
                ));
            }
            if analysis_test {
                return Err(errorf(
                    "analysis_test is not supported when extending rules.",
                ));
            }
            if !build_setting.is_none() {
                return Err(errorf(
                    "build_setting is not supported when extending rules.",
                ));
            }
            parent_rule = Some((parent_unchecked, parent_rc));
        }

        let label_converter = label_converter_for_bzl(eval)?;
        let env = BzlRuleDefinitionEnvironment { tools_repository };

        create_rule(
            eval,
            &env,
            &bzl_file,
            &transitive_digest,
            label_converter,
            CreateRuleArgs {
                parent: parent_rule,
                extendable_unchecked: extendable,
                implementation,
                initializer,
                test: test_flag,
                attrs,
                implicit_outputs: outputs,
                executable: executable_flag,
                output_to_genfiles,
                fragments,
                starlark_testable: _skylark_testable,
                toolchains,
                doc,
                provides_arg: provides,
                dependency_resolution_rule,
                is_materializer_rule: false,
                allow_materializer_rule_real_deps: false,
                exec_compatible_with,
                analysis_test,
                build_setting,
                cfg,
                exec_groups,
                subrules_unchecked: subrules,
            },
        )
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:660 StarlarkRuleClassFunctions.materializerRule
    fn materializer_rule<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] attrs: Option<Value<'v>>,
        #[starlark(require = named)] doc: Option<Value<'v>>,
        #[starlark(require = named, default = false)] allow_real_deps: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkRuleFunction<'v>> {
        if !is_starlark_function(implementation) {
            return Err(param_type_error(
                "materializer_rule",
                "implementation",
                implementation,
                "function",
            ));
        }
        let doc = optional_str("materializer_rule", "doc", doc)?;
        let attrs = attrs.unwrap_or_else(|| empty_dict(eval));
        // Ensure we're initializing a .bzl file, which also means we have a
        // RuleDefinitionEnvironment.
        let (bzl_file, transitive_digest, tools_repository) = {
            let bazel_context =
                BzlInitThreadContext::from_or_fail(eval, "rule()").map_err(eval_exception)?;
            (
                bazel_context.bzl_file.clone(),
                bazel_context.transitive_digest.clone(),
                bazel_context.tools_repository.clone(),
            )
        };
        let label_converter = label_converter_for_bzl(eval)?;
        let env = BzlRuleDefinitionEnvironment { tools_repository };
        let empty = empty_list(eval);
        create_rule(
            eval,
            &env,
            &bzl_file,
            &transitive_digest,
            label_converter,
            CreateRuleArgs {
                parent: None,
                extendable_unchecked: None,
                implementation,
                initializer: None,
                test: false,
                attrs,
                implicit_outputs: Value::new_none(),
                executable: false,
                output_to_genfiles: false,
                fragments: empty,
                starlark_testable: false,
                toolchains: empty,
                doc,
                provides_arg: empty,
                dependency_resolution_rule: false,
                is_materializer_rule: true,
                allow_materializer_rule_real_deps: allow_real_deps,
                exec_compatible_with: empty,
                analysis_test: false,
                build_setting: Value::new_none(),
                cfg: Value::new_none(),
                exec_groups: Value::new_none(),
                subrules_unchecked: empty,
            },
        )
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2146 StarlarkRuleClassFunctions.execGroup
    fn exec_group<'v>(
        #[starlark(require = named)] toolchains: Option<Value<'v>>,
        #[starlark(require = named)] exec_compatible_with: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<ExecGroupValue> {
        let toolchains = toolchains.unwrap_or_else(|| empty_list(eval));
        let exec_compatible_with = exec_compatible_with.unwrap_or_else(|| empty_list(eval));
        let label_converter = label_converter_for_bzl(eval)?;
        let toolchain_types = parse_toolchain_types(toolchains, &label_converter)?;
        let constraints = parse_labels(
            exec_compatible_with,
            &label_converter,
            "exec_compatible_with",
        )?;
        let mut toolchain_types_map = IndexMap::new();
        for t in toolchain_types {
            toolchain_types_map.insert(t.toolchain_type.clone(), t);
        }
        Ok(ExecGroupValue(DeclaredExecGroup {
            toolchain_types_map,
            exec_compatible_with: constraints,
            copy_from_default: false,
        }))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:2161 StarlarkRuleClassFunctions.subrule
    fn subrule<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] attrs: Option<Value<'v>>,
        #[starlark(require = named)] toolchains: Option<Value<'v>>,
        #[starlark(require = named)] fragments: Option<Value<'v>>,
        #[starlark(require = named)] subrules: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<subrule::StarlarkSubrule<'v>> {
        if !is_starlark_function(implementation) {
            return Err(param_type_error(
                "subrule",
                "implementation",
                implementation,
                "function",
            ));
        }
        let attrs_unchecked = attrs.unwrap_or_else(|| empty_dict(eval));
        let toolchains_unchecked = toolchains.unwrap_or_else(|| empty_list(eval));
        let fragments_unchecked = fragments.unwrap_or_else(|| empty_list(eval));
        let subrules_unchecked = subrules.unwrap_or_else(|| empty_list(eval));

        let attrs = dict_cast(
            attrs_unchecked,
            "string",
            |k| k.unpack_str().is_some(),
            "Attribute",
            |v| as_descriptor(v).is_some(),
            "attrs",
        )?;
        let fragments =
            super::util::sequence_noneable_cast_strings(fragments_unchecked, "fragments")?;
        for (attr_name, descriptor) in &attrs {
            let attr_name = attr_name.unpack_str().unwrap();
            let descriptor = as_descriptor(*descriptor).unwrap();
            let transition_factory = descriptor.get_transition_factory();
            if !transition_factory.is_no_transition() && !transition_factory.is_tool() {
                return Err(errorf(format!(
                    "bad cfg for attribute '{attr_name}': subrules may only have target/exec \
                     attributes."
                )));
            }
            check_attribute_name(attr_name)?;
            let type_ = descriptor.get_type();
            if !attr_name.starts_with('_') {
                return Err(errorf(format!(
                    "illegal attribute name '{attr_name}': subrules may only define private \
                     attributes (whose names begin with '_')."
                )));
            } else if descriptor.get_value_source()
                == crate::attribute::AttributeValueSource::ComputedDefault
            {
                return Err(errorf(format!(
                    "illegal default value for attribute '{attr_name}': subrules cannot define \
                     computed defaults."
                )));
            } else if !descriptor.has_default() {
                return Err(errorf(format!(
                    "for attribute '{attr_name}': no default value specified"
                )));
            } else if type_ != Type::Label && type_ != Type::LabelList {
                return Err(errorf(format!(
                    "bad type for attribute '{attr_name}': subrule attributes may only be label \
                     or lists of labels."
                )));
            }
        }
        let label_converter = label_converter_for_bzl(eval)?;
        let toolchains = parse_toolchain_types(toolchains_unchecked, &label_converter)?;
        if toolchains.len() > 1 {
            return Err(errorf(format!(
                "subrules may require at most 1 toolchain, got: [{}]",
                toolchains
                    .iter()
                    .map(|t| format!("{t:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let subrule_values = sequence_cast(
            subrules_unchecked,
            "Subrule",
            |s| subrule::is_subrule(s),
            "subrules",
        )?;
        let symbol = ThreadContext::get(eval)?.next_symbol();
        Ok(subrule::StarlarkSubrule::new(
            implementation,
            attrs
                .into_iter()
                .map(|(k, v)| (k.unpack_str().unwrap().to_owned(), v))
                .collect(),
            toolchains,
            fragments.into_iter().collect(),
            subrule_values,
            symbol,
        ))
    }
}

/// The macro() `inherit_attrs` value "common": the attributes of the base rule.
pub fn common_attributes() -> Vec<Attribute> {
    BASE_RULE.get_attribute_provider().get_attributes().to_vec()
}

/// Java `COMMON_ATTRIBUTES_NAME`.
pub fn is_common_attributes_name(s: &str) -> bool {
    s == COMMON_ATTRIBUTES_NAME
}

/// `NoneType` re-export for callers building None results.
pub type StarlarkNone = NoneType;
