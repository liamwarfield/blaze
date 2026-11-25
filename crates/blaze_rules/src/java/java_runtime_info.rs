//! `rules/java/JavaRuntimeInfo.java`: information about the Java runtime used by the `java_*`
//! rules (a wrapped instance of rules_java's `JavaRuntimeInfo` provider).

use blaze_actions::Artifact;
use blaze_collect::NestedSet;
use blaze_packages::rule_class::ProviderKey;
use blaze_vfs::PathFragment;
use starlark::values::{Heap, Value};

use super::provider_util::{
    info_key, is_starlark_info_with_schema, main_repo_provider_key, rules_java_provider_key,
    starlark_type,
};
use super::starlark_info_wrapper::StarlarkInfoWrapper;
use crate::cpp::util::errorf;

/// The bzl file of the provider, relative to the rules_java repository.
const BZL: &str = "java/common/rules:java_runtime.bzl";

/// `JavaRuntimeInfo.PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:139 JavaRuntimeInfo.Provider.<init>
// Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:147 JavaRuntimeInfo.Provider.<init>
pub fn provider_key() -> ProviderKey {
    rules_java_provider_key(BZL, "JavaRuntimeInfo")
}

/// `JavaRuntimeInfo.RULES_JAVA_PROVIDER.getKey()`.
// Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:132 JavaRuntimeInfo.RulesJavaProvider.<init>
pub fn rules_java_provider_key_() -> ProviderKey {
    main_repo_provider_key(BZL, "JavaRuntimeInfo")
}

/// Information about the Java runtime used by the `java_*` rules.
// Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:42 JavaRuntimeInfo
#[derive(Clone, Copy)]
pub struct JavaRuntimeInfo<'v> {
    wrapper: StarlarkInfoWrapper<'v>,
}

impl<'v> JavaRuntimeInfo<'v> {
    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:50 JavaRuntimeInfo.forHost
    pub fn for_host(
        rule_context: &blaze_analysis::rule::RuleContext,
        eval: &starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<JavaRuntimeInfo<'v>> {
        super::java_toolchain_provider::JavaToolchainProvider::from_rule_context(rule_context, eval)
            .unwrap_or_else(|| panic!("NullPointerException: JavaToolchainProvider.from"))
            .get_java_runtime()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:54 JavaRuntimeInfo.from
    pub fn from_toolchain_type(
        rule_context: &blaze_analysis::rule::RuleContext,
        java_runtime_toolchain_type: &blaze_cmdline::Label,
        eval: &starlark::eval::Evaluator<'v, '_, '_>,
    ) -> Option<JavaRuntimeInfo<'v>> {
        let toolchain_info = rule_context
            .get_toolchain_info(java_runtime_toolchain_type)
            .map(|i| eval.heap().access_owned_frozen_value(&i));
        Self::from_toolchain_info(rule_context, toolchain_info, eval.heap())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:60 JavaRuntimeInfo.from
    pub fn from_attribute(
        rule_context: &blaze_analysis::rule::RuleContext,
        attribute_name: &str,
        eval: &starlark::eval::Evaluator<'v, '_, '_>,
    ) -> Option<JavaRuntimeInfo<'v>> {
        use blaze_packages::attribute::AttributeMap;
        if !rule_context
            .attributes()
            .has_typed(attribute_name, blaze_packages::types::Type::Label)
        {
            return None;
        }
        let prerequisite = rule_context.get_prerequisite(attribute_name)?;
        let toolchain_info = blaze_analysis::configuredtargets::get_info(
            &prerequisite,
            &blaze_packages::starlark_defs::platform_common::TOOLCHAIN_INFO.key(),
        )
        .map(|i| eval.heap().access_owned_frozen_value(&i));
        Self::from_toolchain_info(rule_context, toolchain_info, eval.heap())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:74 JavaRuntimeInfo.from
    fn from_toolchain_info(
        rule_context: &blaze_analysis::rule::RuleContext,
        toolchain_info: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> Option<JavaRuntimeInfo<'v>> {
        use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
        if let Some(toolchain_info) = toolchain_info {
            let java_runtime = super::starlark_info_wrapper::struct_get_value(
                toolchain_info,
                "java_runtime",
                heap,
            );
            let result = match java_runtime {
                Some(v) if !v.is_none() => Self::wrap(v, heap),
                _ => panic!("NullPointerException: ToolchainInfo.java_runtime"),
            };
            match result {
                Ok(r) => return Some(r),
                Err(e) => {
                    rule_context.rule_error(&format!(
                        "There was an error reading the Java runtime: {}",
                        e
                    ));
                    return None;
                }
            }
        }
        rule_context.rule_error("The selected Java runtime is not a JavaRuntimeInfo");
        None
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:94 JavaRuntimeInfo.wrap
    pub fn wrap(info: Value<'v>, heap: Heap<'v>) -> starlark::Result<JavaRuntimeInfo<'v>> {
        let key = info_key(info);
        if key == Some(provider_key()) || key == Some(rules_java_provider_key_()) {
            // Java: JavaRuntimeInfo.Provider.wrap
            // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:152 JavaRuntimeInfo.Provider.wrap
            if is_starlark_info_with_schema(info) {
                return Ok(JavaRuntimeInfo {
                    wrapper: StarlarkInfoWrapper::new(info, heap),
                });
            }
            return Err(errorf(format!(
                "got value of type '{}', want 'JavaRuntimeInfo'",
                starlark_type(info)
            )));
        }
        Err(errorf(format!(
            "expected JavaRuntimeInfo, got: {}",
            key.map(|k| k.to_string())
                .unwrap_or_else(|| "null".to_owned())
        )))
    }

    /// All input artifacts in the javabase.
    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:106 JavaRuntimeInfo.javaBaseInputs
    pub fn java_base_inputs(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.wrapper.get_underlying_nested_set_artifacts("files")
    }

    /// The root directory of the Java installation.
    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:111 JavaRuntimeInfo.javaHome
    pub fn java_home(&self) -> starlark::Result<Option<String>> {
        self.wrapper.get_underlying_string("java_home")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:115 JavaRuntimeInfo.javaBinaryExecPathFragment
    pub fn java_binary_exec_path_fragment(&self) -> starlark::Result<PathFragment> {
        let s = self
            .wrapper
            .get_underlying_string("java_executable_exec_path")?
            .unwrap_or_else(|| panic!("NullPointerException: java_executable_exec_path"));
        Ok(PathFragment::create(&s))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:119 JavaRuntimeInfo.javaBinaryRunfilesPathFragment
    pub fn java_binary_runfiles_path_fragment(&self) -> starlark::Result<PathFragment> {
        let s = self
            .wrapper
            .get_underlying_string("java_executable_runfiles_path")?
            .unwrap_or_else(|| panic!("NullPointerException: java_executable_runfiles_path"));
        Ok(PathFragment::create(&s))
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:123 JavaRuntimeInfo.hermeticStaticLibs
    pub fn hermetic_static_libs(&self) -> starlark::Result<Vec<Value<'v>>> {
        self.wrapper.get_underlying_sequence("hermetic_static_libs")
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaRuntimeInfo.java:127 JavaRuntimeInfo.version
    pub fn version(&self) -> starlark::Result<i32> {
        match self.wrapper.get_underlying_value("version") {
            None => panic!("NullPointerException: version"),
            Some(v) => match v.unpack_i32() {
                Some(i) => Ok(i),
                None => Err(StarlarkInfoWrapper::cast_error("version", v, "int")),
            },
        }
    }
}
