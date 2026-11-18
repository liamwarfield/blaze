//! The loading-phase subset of `analysis/ConfiguredRuleClassProvider.java` (with its
//! `Builder`) and `bazel/rules/BazelRuleClassProvider.java` with the rule sets it installs.
//!
//! What loading needs: the native rule classes by name in registration order, their
//! `BuiltinRuleFunction`s (the BUILD / `native` rule symbols), the tools repository
//! (`@bazel_tools`), the prelude label and the runfiles prefix. The analysis-phase parts of the
//! rule sets (configuration fragments, Starlark bootstraps and builtins internals, trimming
//! transitions, native aspects, action environment) belong to W4 and are not registered here.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::RepositoryName;
use indexmap::IndexMap;

use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::SharedRuleDefinition;
use crate::native_rules::base_rule_classes::EmptyRule;
use crate::native_rules::base_rule_classes::MakeVariableExpandingRule;
use crate::native_rules::base_rule_classes::NativeActionCreatingRule;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::native_rules::cc_rules::CcLibcTopAlias;
use crate::native_rules::cc_rules::CcToolchainAliasRule;
use crate::native_rules::config_rule_classes::ConfigBaseRule;
use crate::native_rules::config_rule_classes::ConfigFeatureFlagRule;
use crate::native_rules::config_rule_classes::ConfigSettingRule;
use crate::native_rules::generic_rules::AliasRule;
use crate::native_rules::generic_rules::EnvironmentRule;
use crate::native_rules::generic_rules::FilegroupRule;
use crate::native_rules::generic_rules::GenQueryRule;
use crate::native_rules::generic_rules::LabelBuildFlagRule;
use crate::native_rules::generic_rules::LabelBuildSettingRule;
use crate::native_rules::generic_rules::StarlarkDocExtractRule;
use crate::native_rules::generic_rules::TestSuiteRule;
use crate::native_rules::genrule::BazelGenRuleRule;
use crate::native_rules::genrule::GenRuleBaseRule;
use crate::native_rules::java_rules::ActionListenerRule;
use crate::native_rules::java_rules::ExtraActionRule;
use crate::native_rules::java_rules::JavaPluginsFlagAliasRule;
use crate::native_rules::platform_rules::ConstraintSettingRule;
use crate::native_rules::platform_rules::ConstraintValueRule;
use crate::native_rules::platform_rules::PlatformRule;
use crate::native_rules::platform_rules::ToolchainRule;
use crate::native_rules::toolchain_type::ToolchainTypeRule;
use crate::package::BuiltinRuleFunction;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;

/// Builder of a [`RuleClassProvider`] (Java `ConfiguredRuleClassProvider.Builder`).
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:141 ConfiguredRuleClassProvider.Builder
pub struct RuleClassProviderBuilder {
    prelude_label: Option<Label>,
    runfiles_prefix: Option<String>,
    tools_repository: Option<RepositoryName>,
    builtins_bzl_package_path_in_source: Option<String>,
    /// Java `ruleDefinitionMap` before `build()`: definitions by Java class name, in
    /// `addRuleDefinition` order.
    rule_definitions: IndexMap<String, SharedRuleDefinition>,
}

impl RuleClassProviderBuilder {
    pub fn new() -> Self {
        RuleClassProviderBuilder {
            prelude_label: None,
            runfiles_prefix: None,
            tools_repository: None,
            builtins_bzl_package_path_in_source: None,
            rule_definitions: IndexMap::new(),
        }
    }

    /// Java `setPrelude(preludeLabelString)`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:180 ConfiguredRuleClassProvider.Builder.setPrelude
    pub fn set_prelude(&mut self, prelude_label_string: &str) -> &mut Self {
        blaze_util::check_argument!(
            prelude_label_string.starts_with("//"),
            "Prelude label '{}' must start with '//'",
            prelude_label_string
        );
        // We're parsing this label as if it's in the main repository but it will actually get
        // massaged into a label in the repository where the package being loaded resides.
        self.prelude_label = Some(Label::parse_canonical(prelude_label_string).unwrap_or_else(
            |e| {
                panic!(
                    "Prelude label '{}' is invalid: {}",
                    prelude_label_string,
                    e.get_message()
                )
            },
        ));
        self
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:198 ConfiguredRuleClassProvider.Builder.setRunfilesPrefix
    pub fn set_runfiles_prefix(&mut self, runfiles_prefix: &str) -> &mut Self {
        self.runfiles_prefix = Some(runfiles_prefix.to_owned());
        self
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:204 ConfiguredRuleClassProvider.Builder.setToolsRepository
    pub fn set_tools_repository(&mut self, tools_repository: RepositoryName) -> &mut Self {
        self.tools_repository = Some(tools_repository);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:248 ConfiguredRuleClassProvider.Builder.setBuiltinsBzlPackagePathInSource
    pub fn set_builtins_bzl_package_path_in_source(&mut self, path: &str) -> &mut Self {
        self.builtins_bzl_package_path_in_source = Some(path.to_owned());
        self
    }

    /// Registers a rule definition; its ancestors must be registered too before `build()`.
    // PORT: Java also adds the definition's node and ancestor edges to a dependency graph; the
    // graph is rebuilt from the definitions in `build()`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:260 ConfiguredRuleClassProvider.Builder.addRuleDefinition
    pub fn add_rule_definition(
        &mut self,
        rule_definition: impl RuleDefinition + 'static,
    ) -> &mut Self {
        let definition: SharedRuleDefinition = Arc::new(rule_definition);
        self.rule_definitions
            .insert(definition.definition_class(), definition);
        self
    }

    /// Commits every definition, ancestors first, and builds the provider.
    // PORT: Java commits in `dependencyGraph.getTopologicalOrder()`, a reverse DFS post-order
    // over hash-ordered nodes keyed by `Class` identity, so its order is not deterministic.
    // Blaze commits in registration order, committing a definition's ancestors (in declared
    // order) just before it when they are not committed yet. Every ancestor precedes its
    // descendants in both orders.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:525 ConfiguredRuleClassProvider.Builder.build
    pub fn build(&self) -> RuleClassProvider {
        let tools_repository = self
            .tools_repository
            .clone()
            .expect("setToolsRepository must be called");
        let mut committer = Committer {
            definitions: &self.rule_definitions,
            env: ToolsEnv(tools_repository.clone()),
            rule_map: IndexMap::new(),
            rule_class_map: IndexMap::new(),
            rule_definition_map: IndexMap::new(),
            in_progress: Vec::new(),
        };
        for definition_class in self.rule_definitions.keys() {
            committer.commit(definition_class);
        }
        let rule_class_map = committer.rule_class_map;
        let rule_functions = crate::package::rule_factory::build_rule_functions(&rule_class_map);
        RuleClassProvider {
            prelude_label: self.prelude_label.clone(),
            runfiles_prefix: self.runfiles_prefix.clone(),
            tools_repository,
            builtins_bzl_package_path_in_source: self.builtins_bzl_package_path_in_source.clone(),
            rule_class_map,
            rule_definition_map: committer.rule_definition_map,
            rule_functions,
        }
    }
}

struct ToolsEnv(RepositoryName);

impl RuleDefinitionEnvironment for ToolsEnv {
    fn get_tools_repository(&self) -> RepositoryName {
        self.0.clone()
    }
}

struct Committer<'a> {
    definitions: &'a IndexMap<String, SharedRuleDefinition>,
    env: ToolsEnv,
    /// Java `ruleMap`: definition class -> rule class.
    rule_map: IndexMap<String, Arc<RuleClass>>,
    rule_class_map: IndexMap<String, Arc<RuleClass>>,
    rule_definition_map: IndexMap<String, SharedRuleDefinition>,
    in_progress: Vec<String>,
}

impl Committer<'_> {
    fn commit(&mut self, definition_class: &str) {
        if self.rule_map.contains_key(definition_class) {
            return;
        }
        let Some(instance) = self.definitions.get(definition_class).cloned() else {
            panic!("addRuleDefinition(new {definition_class}()) should be called before build()");
        };
        blaze_util::check_state!(
            !self.in_progress.iter().any(|c| c == definition_class),
            "cycle in rule definition ancestors at {}",
            definition_class
        );
        self.in_progress.push(definition_class.to_owned());
        for ancestor in instance.get_metadata().ancestors {
            self.commit(&ancestor);
        }
        self.in_progress.pop();
        self.commit_rule_definition(definition_class, &instance);
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:455 ConfiguredRuleClassProvider.Builder.commitRuleDefinition
    fn commit_rule_definition(&mut self, definition_class: &str, instance: &SharedRuleDefinition) {
        let metadata = instance.get_metadata();
        blaze_util::check_argument!(
            !self.rule_class_map.contains_key(&metadata.name),
            "The rule {} was committed already, use another name",
            metadata.name
        );

        let ancestors = &metadata.ancestors;

        blaze_util::check_argument!(
            (metadata.type_ == RuleClassType::Abstract) ^ metadata.factory_class.is_some()
        );

        let mut ancestor_classes: Vec<Arc<RuleClass>> = Vec::with_capacity(ancestors.len());
        for ancestor in ancestors {
            match self.rule_map.get(ancestor) {
                Some(rc) => ancestor_classes.push(rc.clone()),
                // Ancestors should have been initialized by now
                None => panic!(
                    "Ancestor {} of {} is not initialized",
                    ancestor, metadata.name
                ),
            }
        }

        let mut builder =
            RuleClassBuilder::new(&metadata.name, metadata.type_, false, &ancestor_classes);
        // Java: `builder.factory(factory)`, null for abstract rules.
        if let Some(factory) = metadata.factory_class {
            builder.factory(factory);
        }
        let rule_class = Arc::new(instance.build(&mut builder, &self.env));
        self.rule_map
            .insert(definition_class.to_owned(), rule_class.clone());
        self.rule_class_map
            .insert(rule_class.get_name().to_owned(), rule_class.clone());
        self.rule_definition_map
            .insert(rule_class.get_name().to_owned(), instance.clone());
    }
}

/// The native rule classes and BUILD-file symbols Blaze loads with (Java
/// `ConfiguredRuleClassProvider`, loading-phase subset).
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:87 ConfiguredRuleClassProvider
pub struct RuleClassProvider {
    prelude_label: Option<Label>,
    runfiles_prefix: Option<String>,
    tools_repository: RepositoryName,
    builtins_bzl_package_path_in_source: Option<String>,
    rule_class_map: IndexMap<String, Arc<RuleClass>>,
    rule_definition_map: IndexMap<String, SharedRuleDefinition>,
    rule_functions: IndexMap<String, BuiltinRuleFunction>,
}

impl RuleClassProvider {
    /// Bazel's provider (Java `BazelRuleClassProvider.create()`).
    // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:206 BazelRuleClassProvider.create
    pub fn create_bazel() -> RuleClassProvider {
        let mut builder = RuleClassProviderBuilder::new();
        bazel_rule_class_provider_setup(&mut builder);
        builder.build()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:730 ConfiguredRuleClassProvider.getPreludeLabel
    pub fn get_prelude_label(&self) -> Option<&Label> {
        self.prelude_label.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:740 ConfiguredRuleClassProvider.getRunfilesPrefix
    pub fn get_runfiles_prefix(&self) -> Option<&str> {
        self.runfiles_prefix.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:745 ConfiguredRuleClassProvider.getToolsRepository
    pub fn get_tools_repository(&self) -> &RepositoryName {
        &self.tools_repository
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:757 ConfiguredRuleClassProvider.getBuiltinsBzlPackagePathInSource
    pub fn get_builtins_bzl_package_path_in_source(&self) -> Option<&str> {
        self.builtins_bzl_package_path_in_source.as_deref()
    }

    /// Every native rule class (abstract ones included) by name, in commit order.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:762 ConfiguredRuleClassProvider.getRuleClassMap
    pub fn get_rule_class_map(&self) -> &IndexMap<String, Arc<RuleClass>> {
        &self.rule_class_map
    }

    /// The definition a rule class was built from.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:810 ConfiguredRuleClassProvider.getRuleClassDefinition
    pub fn get_rule_class_definition(
        &self,
        rule_class_name: &str,
    ) -> Option<&SharedRuleDefinition> {
        self.rule_definition_map.get(rule_class_name)
    }

    /// The native rule symbols of BUILD files and `native` (Java
    /// `BazelStarlarkEnvironment.ruleFunctions`, from `RuleFactory.buildRuleFunctions`).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:667 ConfiguredRuleClassProvider.<init>
    pub fn get_rule_functions(&self) -> &IndexMap<String, BuiltinRuleFunction> {
        &self.rule_functions
    }
}

impl RuleDefinitionEnvironment for RuleClassProvider {
    fn get_tools_repository(&self) -> RepositoryName {
        self.tools_repository.clone()
    }
}

/// Java `BazelRuleClassProvider.setup(builder)`: the tools repository, builtins path and the
/// `RULE_SETS` in order.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:213 BazelRuleClassProvider.setup
pub fn bazel_rule_class_provider_setup(builder: &mut RuleClassProviderBuilder) {
    builder.set_tools_repository(blaze_cmdline::repository_name::BAZEL_TOOLS.clone());
    // PORT: the builtins_bzl.zip resource is the install base's embedded builtins (agent D).
    builder.set_builtins_bzl_package_path_in_source("src/main/starlark/builtins_bzl");

    // Java: BazelRuleClassProvider.RULE_SETS
    bazel_setup(builder);
    core_rules(builder);
    generic_rules(builder);
    config_rules(builder);
    platform_rules(builder);
    // PROTO_RULES: configuration fragment and `proto_common_do_not_use` only (W4).
    cc_rules(builder);
    java_rules(builder);
    // ANDROID_RULES, PYTHON_RULES: configuration fragments and Starlark toplevels only (W4).
    objc_rules(builder);
    // TestingSupportRules, PACKAGING_RULES: Starlark bootstraps and toplevels only (W4).
    toolchain_rules(builder);
}

/// Java `BazelRuleClassProvider.BAZEL_SETUP` (loading-phase part).
// PORT: `package()` is a BUILD toplevel registered by `build::register_build_globals`;
// shell executables, the prerequisite validator, action environment and universal fragments
// are analysis (W4).
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:224 BazelRuleClassProvider.BAZEL_SETUP
fn bazel_setup(builder: &mut RuleClassProviderBuilder) {
    builder
        .set_prelude("//tools/build_rules:prelude_bazel")
        .set_runfiles_prefix("_main");
}

// Java: com/google/devtools/build/lib/rules/core/CoreRules.java:32 CoreRules.init
fn core_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(NativeBuildRule);
    builder.add_rule_definition(NativeActionCreatingRule);
    builder.add_rule_definition(MakeVariableExpandingRule);
}

// Java: com/google/devtools/build/lib/bazel/rules/GenericRules.java:42 GenericRules.init
fn generic_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(EnvironmentRule);

    builder.add_rule_definition(AliasRule);
    builder.add_rule_definition(FilegroupRule);
    builder.add_rule_definition(TestSuiteRule);
    // Java: GenQueryRule.register(builder)
    builder.add_rule_definition(GenQueryRule);
    builder.add_rule_definition(LabelBuildSettingRule);
    builder.add_rule_definition(LabelBuildFlagRule);
    builder.add_rule_definition(StarlarkDocExtractRule);
}

// Java: com/google/devtools/build/lib/rules/config/ConfigRules.java:37 ConfigRules.init
fn config_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(ConfigBaseRule);
    builder.add_rule_definition(ConfigSettingRule);
    builder.add_rule_definition(ConfigFeatureFlagRule);
}

// Java: com/google/devtools/build/lib/rules/platform/PlatformRules.java:34 PlatformRules.init
fn platform_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(ConstraintSettingRule);
    builder.add_rule_definition(ConstraintValueRule);
    builder.add_rule_definition(PlatformRule);

    builder.add_rule_definition(ToolchainRule);
}

// Java: com/google/devtools/build/lib/bazel/rules/CcRules.java:39 CcRules.init
fn cc_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(EmptyRule::new("cc_toolchain"));
    builder.add_rule_definition(EmptyRule::new("cc_toolchain_suite"));
    builder.add_rule_definition(CcToolchainAliasRule);
    builder.add_rule_definition(CcLibcTopAlias);
    builder.add_rule_definition(EmptyRule::new("cc_binary"));
    builder.add_rule_definition(EmptyRule::new("cc_shared_library"));
    builder.add_rule_definition(EmptyRule::new("cc_static_library"));
    builder.add_rule_definition(EmptyRule::new("cc_test"));
    builder.add_rule_definition(EmptyRule::new("cc_library"));
    builder.add_rule_definition(EmptyRule::new("cc_import"));
    builder.add_rule_definition(EmptyRule::new("fdo_profile"));
    builder.add_rule_definition(EmptyRule::new("fdo_prefetch_hints"));
    builder.add_rule_definition(EmptyRule::new("memprof_profile"));
    builder.add_rule_definition(EmptyRule::new("propeller_optimize"));
}

// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:73 JavaRules.coreBzlLabel
fn core_bzl_label(rule_name: &str) -> String {
    format!("@rules_java//java:{rule_name}.bzl")
}

// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:77 JavaRules.toolchainBzlLabel
fn toolchain_bzl_label(rule_name: &str) -> String {
    format!("@rules_java//java/toolchains:{rule_name}.bzl")
}

// Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:38 JavaRules.init
fn java_rules(builder: &mut RuleClassProviderBuilder) {
    for name in [
        "java_binary",
        "java_library",
        "java_import",
        "java_test",
        "java_plugin",
    ] {
        builder.add_rule_definition(EmptyRule::with_bzl_load_label(
            name,
            Some(&core_bzl_label(name)),
        ));
    }
    for name in [
        "java_toolchain",
        "java_package_configuration",
        "java_runtime",
    ] {
        builder.add_rule_definition(EmptyRule::with_bzl_load_label(
            name,
            Some(&toolchain_bzl_label(name)),
        ));
    }
    builder.add_rule_definition(JavaPluginsFlagAliasRule);

    builder.add_rule_definition(ExtraActionRule);
    builder.add_rule_definition(ActionListenerRule);
}

// Java: com/google/devtools/build/lib/bazel/rules/ObjcRules.java:36 ObjcRules.init
fn objc_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(EmptyRule::new("objc_import"));
    builder.add_rule_definition(EmptyRule::new("objc_library"));
}

// Java: com/google/devtools/build/lib/bazel/rules/ToolchainRules.java:33 ToolchainRules.init
fn toolchain_rules(builder: &mut RuleClassProviderBuilder) {
    builder.add_rule_definition(ToolchainTypeRule);
    builder.add_rule_definition(GenRuleBaseRule);
    builder.add_rule_definition(BazelGenRuleRule);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The rule classes `bazel info build-language` lists at the pin (oracle, Bazel 9.3.2):
    // every non-abstract native rule class.
    const ORACLE_BUILD_LANGUAGE_RULES: &[&str] = &[
        "action_listener",
        "alias",
        "cc_binary",
        "cc_import",
        "cc_libc_top_alias",
        "cc_library",
        "cc_shared_library",
        "cc_static_library",
        "cc_test",
        "cc_toolchain",
        "cc_toolchain_alias",
        "cc_toolchain_suite",
        "config_feature_flag",
        "config_setting",
        "constraint_setting",
        "constraint_value",
        "environment",
        "extra_action",
        "fdo_prefetch_hints",
        "fdo_profile",
        "filegroup",
        "genquery",
        "genrule",
        "java_binary",
        "java_import",
        "java_library",
        "java_package_configuration",
        "java_plugin",
        "java_plugins_flag_alias",
        "java_runtime",
        "java_test",
        "java_toolchain",
        "label_flag",
        "label_setting",
        "memprof_profile",
        "objc_import",
        "objc_library",
        "platform",
        "propeller_optimize",
        "starlark_doc_extract",
        "test_suite",
        "toolchain",
        "toolchain_type",
    ];

    #[test]
    fn bazel_rule_classes_match_oracle_build_language() {
        let provider = RuleClassProvider::create_bazel();
        let mut names: Vec<&str> = provider
            .get_rule_class_map()
            .values()
            .filter(|rc| rc.get_rule_class_type() != RuleClassType::Abstract)
            .map(|rc| rc.get_name())
            .collect();
        names.sort();
        assert_eq!(names, ORACLE_BUILD_LANGUAGE_RULES);
        assert_eq!(provider.get_rule_functions().len(), 43);
        assert_eq!(provider.get_tools_repository().get_name(), "bazel_tools");
    }

    #[test]
    fn tools_label_uses_tools_repository() {
        let provider = RuleClassProvider::create_bazel();
        assert_eq!(
            provider
                .get_tools_label("//tools/genrule:genrule-setup.sh")
                .get_canonical_form(),
            "@@bazel_tools//tools/genrule:genrule-setup.sh"
        );
    }

    /// Java `ProtoUtils.getDiscriminatorFromType` (the `Attribute.Discriminator` name).
    // Java: com/google/devtools/build/lib/packages/ProtoUtils.java:80 ProtoUtils.getDiscriminatorFromType
    fn discriminator(t: crate::types::Type) -> &'static str {
        use crate::types::Type;
        match t {
            Type::Integer => "INTEGER",
            Type::Label | Type::GenqueryScopeType => "LABEL",
            Type::NodepLabel => "STRING",
            Type::LabelList | Type::GenqueryScopeTypeList | Type::DormantLabel => "LABEL_LIST",
            Type::NodepLabelList | Type::DormantLabelList => "STRING_LIST",
            Type::String | Type::StringNoIntern => "STRING",
            Type::StringList => "STRING_LIST",
            Type::StringSet => "DISTRIBUTION_SET",
            Type::Output => "OUTPUT",
            Type::OutputList => "OUTPUT_LIST",
            Type::License => "LICENSE",
            Type::StringDict => "STRING_DICT",
            Type::LabelDictUnary => "LABEL_DICT_UNARY",
            Type::StringListDict => "STRING_LIST_DICT",
            Type::LabelListDict => "LABEL_LIST_DICT",
            Type::Boolean => "BOOLEAN",
            Type::Tristate => "TRISTATE",
            Type::IntegerList => "INTEGER_LIST",
            Type::LabelKeyedStringDict => "LABEL_KEYED_STRING_DICT",
        }
    }

    /// The text of `BuildLanguageInfoItem.convertAttrValue(t, v)` as
    /// `scripts/w3/native-rule-attrs dump` prints it.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:157 BuildLanguageInfoItem.convertAttrValue
    fn fmt_attr_value(t: crate::types::Type, v: &crate::types::AttrValue) -> String {
        use crate::types::AttrValue;
        use crate::types::TriState;
        let q = |s: &str| format!("\"{s}\"");
        match v {
            AttrValue::String(s) => q(s),
            AttrValue::Int(i) => i.to_string(),
            AttrValue::Bool(b) => b.to_string(),
            AttrValue::TriState(t) => match t {
                TriState::Yes => "1",
                TriState::No => "0",
                TriState::Auto => "-1",
            }
            .to_owned(),
            AttrValue::Label(l) => q(&l.get_canonical_form()),
            AttrValue::StringList(l) => {
                format!("[{}]", l.iter().map(|s| q(s)).collect::<Vec<_>>().join(","))
            }
            AttrValue::LabelList(l) => format!(
                "[{}]",
                l.iter()
                    .map(|s| q(&s.get_canonical_form()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            AttrValue::StringDict(d) => format!(
                "{{{}}}",
                d.iter()
                    .map(|(k, v)| format!("{}:{}", q(k), q(v)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            AttrValue::License(_) => "<empty>".to_owned(),
            other => panic!("unexpected default {other:?} of type {t}"),
        }
    }

    /// The `bazel info build-language` lines of the provider's native rule classes, in the
    /// format of `scripts/w3/native-rule-attrs dump`.
    // Java: com/google/devtools/build/lib/runtime/commands/info/BuildLanguageInfoItem.java:109 BuildLanguageInfoItem.getBuildLanguageDefinition
    fn build_language_dump(provider: &RuleClassProvider) -> Vec<String> {
        let mut rule_classes: Vec<&Arc<RuleClass>> =
            provider.get_rule_class_map().values().collect();
        rule_classes.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        let b = |x: bool| if x { "1" } else { "0" };
        let mut out = Vec::new();
        for rule_class in &rule_classes {
            if rule_class.get_rule_class_type() == RuleClassType::Abstract {
                continue;
            }
            out.push(format!("rule {}", rule_class.get_name()));
            let mut attrs: Vec<_> = rule_class
                .get_attribute_provider()
                .get_attributes()
                .to_vec();
            attrs.sort_by(|x, y| x.get_name().cmp(y.get_name()));
            for attr in &attrs {
                let t = attr.get_type();
                let mut line = format!(
                    "  {} {} mandatory={} allow_empty={} single_file={} executable={} configurable={}",
                    attr.get_name(),
                    discriminator(t),
                    b(attr.is_mandatory()),
                    b(!attr.is_non_empty()),
                    b(attr.is_single_artifact()),
                    b(attr.is_executable()),
                    b(attr.is_configurable()),
                );
                if t.is_label_type() {
                    line.push_str(&format!(
                        " nodep={}",
                        b(t.get_label_class() == crate::types::LabelClass::NondepReference)
                    ));
                    let predicate = attr.get_allowed_rule_classes_for_labels();
                    if attr.is_strict_label_checking_enabled()
                        && predicate.get_predicate_type()
                            != crate::rule_class::RuleClassNamePredicateType::Unspecified
                    {
                        let allowed: Vec<&str> = rule_classes
                            .iter()
                            .filter(|rc| attr.get_allowed_rule_class_object_predicate(&***rc))
                            .filter(|rc| rc.get_rule_class_type() != RuleClassType::Abstract)
                            .map(|rc| rc.get_name())
                            .collect();
                        line.push_str(&format!(" allowed={}", allowed.join(",")));
                    } else {
                        line.push_str(" allowed=ANY");
                    }
                }
                // Encode default value, if simple.
                if let Some(v) = attr.get_default_value_unchecked()
                    && !v.is_computed_default()
                    && !matches!(v, crate::types::AttrValue::StarlarkComputedDefaultTemplate(_))
                    && !v.is_late_bound()
                    // PORT: Java compares with `==` (identity); the type's default values are
                    // shared singletons, so value equality is equivalent here.
                    && Some(v) != t.get_default_value().as_ref()
                {
                    line.push_str(&format!(" default={}", fmt_attr_value(t, v)));
                }
                out.push(line);
            }
        }
        out
    }

    /// Every native rule class has Bazel 9.3.2's attributes, types, flags and simple
    /// defaults (oracle: `bazel info build-language`, decoded by
    /// `scripts/w3/native-rule-attrs dump` into testdata/build_language.oracle.txt).
    #[test]
    fn native_rule_attributes_match_oracle_build_language() {
        let expected: Vec<&str> = include_str!("testdata/build_language.oracle.txt")
            .lines()
            .collect();
        let actual = build_language_dump(&RuleClassProvider::create_bazel());
        let mut diffs = Vec::new();
        let e: std::collections::BTreeSet<&str> = expected.iter().copied().collect();
        let a: std::collections::BTreeSet<&str> = actual.iter().map(String::as_str).collect();
        for missing in e.difference(&a) {
            diffs.push(format!("- {missing}"));
        }
        for extra in a.difference(&e) {
            diffs.push(format!("+ {extra}"));
        }
        assert!(
            diffs.is_empty(),
            "build-language differs:\n{}",
            diffs.join("\n")
        );
        assert_eq!(actual, expected);
    }
}
