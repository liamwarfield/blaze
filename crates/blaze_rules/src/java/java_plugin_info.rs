//! `rules/java/JavaPluginInfo.java`: provider for users of Java plugins, with its
//! `JavaPluginData`.
//!
//! PORT: Bazel 9 creates `JavaPluginInfo` instances in Starlark (rules_java); the core wraps
//! them for the compile actions, so the native value is a plain record without a Starlark
//! type. `providerType` is the key of the wrapped instance's provider.

use std::sync::LazyLock;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::rule_class::ProviderKey;
use starlark::values::{Heap, Value, ValueLike};

use super::java_output::JavaOutput;
use super::provider_util::{
    info_key, is_struct_impl, main_repo_provider_key, rules_java_provider_key, starlark_type,
};
use super::starlark_info_wrapper::{StarlarkInfoWrapper, struct_get_value};
use crate::cpp::util::errorf;

// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:48 JavaPluginInfo.PROVIDER_NAME
pub const PROVIDER_NAME: &str = "JavaPluginInfo";

/// `JavaPluginInfo.PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:83 JavaPluginInfo.Provider.<init>
pub fn provider_key() -> ProviderKey {
    rules_java_provider_key("java/private:java_info.bzl", PROVIDER_NAME)
}

/// `JavaPluginInfo.RULES_JAVA_PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:76 JavaPluginInfo.RulesJavaProvider.<init>
pub fn rules_java_provider_key_() -> ProviderKey {
    main_repo_provider_key("java/private:java_info.bzl", PROVIDER_NAME)
}

/// Information about a Java plugin, except for whether it generates API.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:129 JavaPluginInfo.JavaPluginData
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaPluginData {
    /// The class that should be passed to javac in order to run the annotation processor this
    /// class represents.
    processor_classes: NestedSet<String>,
    /// The artifacts to add to the runtime classpath for this plugin.
    processor_classpath: NestedSet<Artifact>,
    data: NestedSet<Artifact>,
}

// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:131 JavaPluginInfo.JavaPluginData.EMPTY
static EMPTY_PLUGIN_DATA: LazyLock<JavaPluginData> = LazyLock::new(|| JavaPluginData {
    processor_classes: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
    processor_classpath: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
    data: NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER),
});

impl JavaPluginData {
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:137 JavaPluginInfo.JavaPluginData.create
    pub fn create(
        processor_classes: NestedSet<String>,
        processor_classpath: NestedSet<Artifact>,
        data: NestedSet<Artifact>,
    ) -> JavaPluginData {
        if processor_classes.is_empty() && processor_classpath.is_empty() && data.is_empty() {
            return Self::empty();
        }
        JavaPluginData {
            processor_classes,
            processor_classpath,
            data,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:148 JavaPluginInfo.JavaPluginData.empty
    pub fn empty() -> JavaPluginData {
        EMPTY_PLUGIN_DATA.clone()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:152 JavaPluginInfo.JavaPluginData.merge
    pub fn merge<'a>(plugins: impl IntoIterator<Item = &'a JavaPluginData>) -> JavaPluginData {
        let mut processor_classes = NestedSetBuilder::naive_link_order();
        let mut processor_classpath = NestedSetBuilder::naive_link_order();
        let mut data = NestedSetBuilder::naive_link_order();
        for plugin in plugins {
            processor_classes.add_transitive(plugin.processor_classes());
            processor_classpath.add_transitive(plugin.processor_classpath());
            data.add_transitive(plugin.data());
        }
        Self::create(
            processor_classes.build(),
            processor_classpath.build(),
            data.build(),
        )
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:165 JavaPluginInfo.JavaPluginData.wrap
    pub fn wrap<'v>(obj: Option<Value<'v>>, heap: Heap<'v>) -> starlark::Result<JavaPluginData> {
        if let Some(obj) = obj
            && let Some(data) =
                obj.downcast_ref::<super::java_plugins_flag_alias::JavaPluginDataValue>()
        {
            return Ok(data.0.clone());
        }
        if let Some(obj) = obj
            && is_struct_impl(obj)
        {
            let get = |k: &str| struct_get_value(obj, k, heap).unwrap_or_else(Value::new_none);
            return Ok(Self::create(
                blaze_analysis::rule::depsets::depset_to_strings(
                    get("processor_classes"),
                    "processor_classes",
                )?,
                blaze_analysis::rule::depsets::depset_to_artifacts(
                    get("processor_jars"),
                    "processor_jars",
                )?,
                blaze_analysis::rule::depsets::depset_to_artifacts(
                    get("processor_data"),
                    "processor_data",
                )?,
            ));
        }
        Err(errorf(format!(
            "Should never happen! Got unexpected type: {}",
            obj.map(|o| o.get_type().to_owned())
                .unwrap_or_else(|| "null".to_owned())
        )))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:180 JavaPluginInfo.JavaPluginData.processorClasses
    pub fn processor_classes(&self) -> &NestedSet<String> {
        &self.processor_classes
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:183 JavaPluginInfo.JavaPluginData.processorClasspath
    pub fn processor_classpath(&self) -> &NestedSet<Artifact> {
        &self.processor_classpath
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:185 JavaPluginInfo.JavaPluginData.data
    pub fn data(&self) -> &NestedSet<Artifact> {
        &self.data
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:202 JavaPluginInfo.JavaPluginData.isEmpty
    pub fn is_empty(&self) -> bool {
        self.processor_classes.is_empty()
            && self.processor_classpath.is_empty()
            && self.data.is_empty()
    }
}

/// Provider for users of Java plugins.
// Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:45 JavaPluginInfo
#[derive(Clone, Debug)]
pub struct JavaPluginInfo {
    java_outputs: Vec<JavaOutput>,
    plugins: JavaPluginData,
    api_generating_plugins: JavaPluginData,
    provider_type: ProviderKey,
}

impl JavaPluginInfo {
    /// This wrapped instance is not propagated back to Starlark, so we don't need every type: we
    /// just use the single type that is checked for in tests.
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:58 JavaPluginInfo.wrap
    pub fn wrap<'v>(info: Value<'v>, heap: Heap<'v>) -> starlark::Result<JavaPluginInfo> {
        // Java: PROVIDER.wrap(info)
        // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:108 JavaPluginInfo.Provider.wrap
        if let Some(native) =
            info.downcast_ref::<super::java_plugins_flag_alias::JavaPluginInfoValue>()
        {
            return Ok(native.0.clone());
        }
        if is_struct_impl(info) {
            let w = StarlarkInfoWrapper::new(info, heap);
            let java_outputs_value =
                struct_get_value(info, "java_outputs", heap).unwrap_or_else(Value::new_none);
            if !crate::cpp::util::is_sequence(java_outputs_value) {
                return Err(errorf(format!(
                    "for java_outputs, got {}, want sequence",
                    java_outputs_value.get_type()
                )));
            }
            let java_outputs = JavaOutput::wrap_sequence(
                &crate::cpp::util::sequence_elements(java_outputs_value, heap)?,
                heap,
            )?;
            return Ok(JavaPluginInfo {
                java_outputs,
                plugins: JavaPluginData::wrap(
                    struct_get_value(w.underlying, "plugins", heap),
                    heap,
                )?,
                api_generating_plugins: JavaPluginData::wrap(
                    struct_get_value(w.underlying, "api_generating_plugins", heap),
                    heap,
                )?,
                provider_type: info_key(info).unwrap_or_else(provider_key),
            });
        }
        Err(errorf(format!(
            "got element of type {}, want JavaPluginInfo",
            starlark_type(info)
        )))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:243 JavaPluginInfo.mergeWithoutJavaOutputs
    pub fn merge_without_java_outputs_pair(
        a: &JavaPluginInfo,
        b: &JavaPluginInfo,
    ) -> JavaPluginInfo {
        if a.is_empty() {
            b.clone()
        } else if b.is_empty() {
            a.clone()
        } else {
            Self::merge_without_java_outputs([a, b], a.provider_type.clone())
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:249 JavaPluginInfo.mergeWithoutJavaOutputs
    pub fn merge_without_java_outputs<'a>(
        providers: impl IntoIterator<Item = &'a JavaPluginInfo>,
        provider_type: ProviderKey,
    ) -> JavaPluginInfo {
        let mut plugins = Vec::new();
        let mut api_generating_plugins = Vec::new();
        for provider in providers {
            if !provider.plugins().is_empty() {
                plugins.push(provider.plugins().clone());
            }
            if !provider.api_generating_plugins().is_empty() {
                api_generating_plugins.push(provider.api_generating_plugins().clone());
            }
        }
        if plugins.is_empty() && api_generating_plugins.is_empty() {
            return Self::empty(provider_type);
        }
        JavaPluginInfo {
            java_outputs: Vec::new(),
            plugins: JavaPluginData::merge(&plugins),
            api_generating_plugins: JavaPluginData::merge(&api_generating_plugins),
            provider_type,
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:271 JavaPluginInfo.empty
    pub fn empty(provider_type: ProviderKey) -> JavaPluginInfo {
        // Java: EMPTY_RULES_JAVA when providerType is RULES_JAVA_PROVIDER, else EMPTY.
        let provider_type = if provider_type == rules_java_provider_key_() {
            provider_type
        } else {
            provider_key()
        };
        JavaPluginInfo {
            java_outputs: Vec::new(),
            plugins: JavaPluginData::empty(),
            api_generating_plugins: JavaPluginData::empty(),
            provider_type,
        }
    }

    pub fn get_java_outputs(&self) -> &[JavaOutput] {
        &self.java_outputs
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:278 JavaPluginInfo.plugins
    pub fn plugins(&self) -> &JavaPluginData {
        &self.plugins
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:281 JavaPluginInfo.apiGeneratingPlugins
    pub fn api_generating_plugins(&self) -> &JavaPluginData {
        &self.api_generating_plugins
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:66 JavaPluginInfo.getProvider
    pub fn get_provider(&self) -> &ProviderKey {
        &self.provider_type
    }

    /// Returns true if the provider has no associated data.
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:286 JavaPluginInfo.isEmpty
    pub fn is_empty(&self) -> bool {
        // apiGeneratingPlugins is a subset of plugins, so checking if plugins is empty is
        // sufficient
        self.plugins.is_empty()
    }

    /// Returns true if the provider has any associated annotation processors (regardless of
    /// whether it has a classpath or data).
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:295 JavaPluginInfo.hasProcessors
    pub fn has_processors(&self) -> bool {
        // apiGeneratingPlugins is a subset of plugins, so checking if plugins is empty is
        // sufficient
        !self.plugins.processor_classes().is_empty()
    }

    /// Translates the plugin information from a `JavaInfo` instance.
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginInfo.java:309 JavaPluginInfo.fromStarlarkJavaInfo
    pub fn from_starlark_java_info<'v>(
        java_info: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<JavaPluginInfo> {
        let provider_type = info_key(java_info).unwrap_or_else(provider_key);
        let w = StarlarkInfoWrapper::new(java_info, heap);
        let plugins = JavaPluginData::wrap(struct_get_value(w.underlying, "plugins", heap), heap)?;
        let api_generating_plugins = JavaPluginData::wrap(
            struct_get_value(w.underlying, "api_generating_plugins", heap),
            heap,
        )?;
        if plugins.is_empty() && api_generating_plugins.is_empty() {
            return Ok(Self::empty(provider_type));
        }
        Ok(JavaPluginInfo {
            java_outputs: Vec::new(),
            plugins,
            api_generating_plugins,
            provider_type,
        })
    }
}
