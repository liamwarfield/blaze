//! The implementation of `java_plugins_flag_alias` (Java
//! `rules/java/JavaPluginsFlagAliasRule.JavaPluginsFlagAlias`): provides the plugins specified
//! by the `--plugin` flag, as native `JavaPluginInfo` instances (`JavaPluginInfo` is a
//! `NativeInfo` whose provider is a `StarlarkProviderWrapper` of rules_java's `JavaPluginInfo`).
//!
//! The rule class is `blaze_packages::native_rules::java_rules::JavaPluginsFlagAliasRule`.

use std::fmt;

use allocative::Allocative;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::{InfoProviderKey, freeze_native_info};
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::depsets::{artifacts_to_depset, depset_value, strings_to_depset};
use blaze_analysis::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_starlark::structs::{DeeplyImmutable, StructureMarker};
use starlark::any::ProvidesStaticType;
use starlark::environment::Module;
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value,
};

use super::java_plugin_info::{self, JavaPluginData, JavaPluginInfo};

/// The labels allowed to instantiate the private rule.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:42 JavaPluginsFlagAliasRule.ALLOWLISTED_LABELS
fn allowlisted_labels() -> [Label; 2] {
    [
        Label::parse_canonical_unchecked("//tools/jdk:java_plugins_flag_alias"),
        Label::parse_canonical_unchecked("@bazel_tools//tools/jdk:java_plugins_flag_alias"),
    ]
}

/// Implementation of the 'java_plugins_flag_alias' rule. Provides plugins specified by the
/// --plugin flag.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:74 JavaPluginsFlagAliasRule.JavaPluginsFlagAlias
pub struct JavaPluginsFlagAlias;

impl RuleConfiguredTargetFactory for JavaPluginsFlagAlias {
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:76 JavaPluginsFlagAliasRule.JavaPluginsFlagAlias.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        if !allowlisted_labels().contains(rule_context.get_label()) {
            rule_context.rule_error(&format!(
                "Rule {} cannot use private rule",
                rule_context.get_label()
            ));
            return Ok(None);
        }

        let mut plugins = get_prerequisites(rule_context, &java_plugin_info::provider_key())?;
        if plugins.is_empty() {
            plugins =
                get_prerequisites(rule_context, &java_plugin_info::rules_java_provider_key_())?;
        }
        let java_plugin_info =
            JavaPluginInfo::merge_without_java_outputs(&plugins, java_plugin_info::provider_key());
        let rules_java_provider_info = JavaPluginInfo::merge_without_java_outputs(
            &plugins,
            java_plugin_info::rules_java_provider_key_(),
        );
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_starlark_declared_provider(freeze_native_info(JavaPluginInfoValue(
                java_plugin_info,
            )))
            .add_starlark_declared_provider(freeze_native_info(JavaPluginInfoValue(
                rules_java_provider_info,
            )))
            .add_provider(RunfilesProvider::empty());
        match builder.build() {
            Ok(t) => Ok(t),
            Err(RuleConfiguredTargetBuildError::ActionConflict(m)) => {
                Err(CreateConfiguredTargetError::ActionConflict(m))
            }
            Err(RuleConfiguredTargetBuildError::Interrupted) => {
                Err(CreateConfiguredTargetError::Interrupted)
            }
        }
    }
}

/// `ruleContext.getRulePrerequisitesCollection().getPrerequisites(":java_plugins", provider)`:
/// the wrapped provider of every prerequisite that has it.
// Java: com/google/devtools/build/lib/analysis/PrerequisitesCollection.java PrerequisitesCollection.getPrerequisites
fn get_prerequisites(
    rule_context: &RuleContext,
    key: &ProviderKey,
) -> Result<Vec<JavaPluginInfo>, CreateConfiguredTargetError> {
    let mut result = Vec::new();
    for prerequisite in rule_context
        .get_rule_prerequisites_collection()
        .get_prerequisites(":java_plugins")
    {
        let Some(info) = blaze_analysis::configuredtargets::get_info(&prerequisite, key) else {
            continue;
        };
        // Java: StarlarkProviderWrapper.wrap (a RuleErrorException is reported by the caller).
        if let Some(native) = info.value().downcast_ref::<JavaPluginInfoValue>() {
            result.push(native.0.clone());
            continue;
        }
        let wrapped = Module::with_temp_heap(|module| {
            // SAFETY: `info` keeps the frozen heap alive for the duration of the closure.
            let value = unsafe { info.unchecked_frozen_value() }.to_value();
            JavaPluginInfo::wrap(value, module.heap()).map_err(|e| e.to_string())
        });
        match wrapped {
            Ok(w) => result.push(w),
            Err(message) => {
                return Err(rule_context.throw_with_rule_error(&message).into());
            }
        }
    }
    Ok(result)
}

/// A native `JavaPluginInfo` (Java's `AutoValue_JavaPluginInfo`), as Starlark sees it.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:45 JavaPluginInfo
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaPluginInfoValue(#[allocative(skip)] pub JavaPluginInfo);

starlark_simple_value!(JavaPluginInfoValue);

impl fmt::Display for JavaPluginInfoValue {
    // Java: StructImpl.repr: the provider's printable name and its fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(...)", java_plugin_info::PROVIDER_NAME)
    }
}

#[starlark_value(type = "JavaPluginInfo")]
impl<'v> StarlarkValue<'v> for JavaPluginInfoValue {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaPluginInfoApi.java:29 JavaPluginInfoApi.plugins
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "plugins" => Some(heap.alloc(JavaPluginDataValue(self.0.plugins().clone()))),
            // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaPluginInfoApi.java:40 JavaPluginInfoApi.apiGeneratingPlugins
            "api_generating_plugins" => {
                Some(heap.alloc(JavaPluginDataValue(self.0.api_generating_plugins().clone())))
            }
            // Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaPluginInfoApi.java:80 JavaPluginInfoApi.getJavaOutputs
            "java_outputs" => {
                if !self.0.get_java_outputs().is_empty() {
                    // Native JavaOutput values are not Starlark values in Blaze; the flag alias
                    // (the only creator of native JavaPluginInfo) has none.
                    blaze_util::unported!(
                        "JavaPluginInfo.getJavaOutputs",
                        "native JavaOutput values"
                    );
                }
                Some(starlark::values::list::alloc_immutable_list(heap, &[]))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            "plugins" | "api_generating_plugins" | "java_outputs"
        )
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "api_generating_plugins".to_owned(),
            "java_outputs".to_owned(),
            "plugins".to_owned(),
        ]
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        // Java: getProvider() is the provider type it was created with (PROVIDER or
        // RULES_JAVA_PROVIDER).
        demand.provide_value(InfoProviderKey(self.0.get_provider().clone()));
        demand.provide_value(StructureMarker { native_info: true });
        demand.provide_value(DeeplyImmutable);
    }
}

/// `JavaPluginInfo.JavaPluginData` as Starlark sees it (a `StructApi`).
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:129 JavaPluginInfo.JavaPluginData
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaPluginDataValue(#[allocative(skip)] pub JavaPluginData);

starlark_simple_value!(JavaPluginDataValue);

impl fmt::Display for JavaPluginDataValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("struct(...)")
    }
}

#[starlark_value(type = "JavaPluginData")]
impl<'v> StarlarkValue<'v> for JavaPluginDataValue {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:188 JavaPluginInfo.JavaPluginData.getProcessorJarsForStarlark
            "processor_jars" => Some(depset_value(&artifacts_to_depset(
                self.0.processor_classpath(),
            ))),
            // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:193 JavaPluginInfo.JavaPluginData.getProcessorClassesForStarlark
            "processor_classes" => {
                Some(depset_value(&strings_to_depset(self.0.processor_classes())))
            }
            // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:198 JavaPluginInfo.JavaPluginData.getProcessorDataForStarlark
            "processor_data" => Some(depset_value(&artifacts_to_depset(self.0.data()))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            "processor_jars" | "processor_classes" | "processor_data"
        )
    }

    fn dir_attr(&self) -> Vec<String> {
        vec![
            "processor_classes".to_owned(),
            "processor_data".to_owned(),
            "processor_jars".to_owned(),
        ]
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(StructureMarker { native_info: false });
        demand.provide_value(DeeplyImmutable);
    }
}
