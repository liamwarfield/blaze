//! Port of `analysis/starlark/BazelBuildApiGlobals.java` (`visibility()`,
//! `configuration_field()`) and the Starlark side of `analysis/starlark/StarlarkLateBoundDefault`.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::RepositoryName;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use super::util::call_stack;
use super::util::errorf;
use super::util::eval_exception;
use super::util::module_context_or_throw;
use super::util::sequence_cast_strings;
use super::util::starlark_type;
use crate::attribute::LateBoundDefault;
use crate::bzl_context::BzlInitThreadContext;
use crate::bzl_context::BzlVisibility;
use crate::target::PackageSpecification;

// -------------------------------------------------------------------------------------------
// StarlarkLateBoundDefault

/// A `@StarlarkConfigurationField` of a configuration fragment (Java reflection over the
/// fragment class).
struct ConfigurationField {
    /// The fragment's Starlark name.
    fragment: &'static str,
    /// The fragment's Java class simple name.
    fragment_class: &'static str,
    name: &'static str,
    default_label: &'static str,
    default_in_tool_repository: bool,
}

/// Every `@StarlarkConfigurationField` of Bazel's fragments.
// Java: com/google/devtools/build/lib/analysis/test/CoverageConfiguration.java:86 CoverageConfiguration.outputGenerator
// Java: com/google/devtools/build/lib/rules/apple/AppleConfiguration.java:320 AppleConfiguration.getXcodeConfigLabel
// Java: com/google/devtools/build/lib/rules/cpp/CppConfiguration.java:309 CppConfiguration
// Java: com/google/devtools/build/lib/rules/java/JavaConfiguration.java:236 JavaConfiguration
// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPythonConfiguration.java:124 BazelPythonConfiguration.getPythonTop
// Java: com/google/devtools/build/lib/rules/android/AndroidConfiguration.java:1386 AndroidConfiguration
const CONFIGURATION_FIELDS: &[ConfigurationField] = &[
    field("coverage", "CoverageConfiguration", "output_generator"),
    ConfigurationField {
        fragment: "apple",
        fragment_class: "AppleConfiguration",
        name: "xcode_config_label",
        // Java: com/google/devtools/build/lib/rules/apple/AppleCommandLineOptions.java:326 AppleCommandLineOptions.DEFAULT_XCODE_VERSION_CONFIG_LABEL
        default_label: "//tools/objc:host_xcodes",
        default_in_tool_repository: true,
    },
    field("cpp", "CppConfiguration", "zipper"),
    field("cpp", "CppConfiguration", "custom_malloc"),
    field("cpp", "CppConfiguration", "fdo_optimize"),
    field("cpp", "CppConfiguration", "fdo_prefetch_hints"),
    field("cpp", "CppConfiguration", "fdo_profile"),
    field("cpp", "CppConfiguration", "cs_fdo_profile"),
    field("cpp", "CppConfiguration", "propeller_optimize"),
    field("cpp", "CppConfiguration", "xbinary_fdo"),
    field("cpp", "CppConfiguration", "memprof_profile"),
    field("cpp", "CppConfiguration", "libc_top"),
    field("cpp", "CppConfiguration", "proto_profile_path"),
    // defaultInToolRepository without a defaultLabel: the default is null.
    field("java", "JavaConfiguration", "launcher"),
    field("java", "JavaConfiguration", "proguard_top"),
    field("java", "JavaConfiguration", "bytecode_optimizer"),
    field(
        "java",
        "JavaConfiguration",
        "java_toolchain_bytecode_optimizer",
    ),
    field(
        "java",
        "JavaConfiguration",
        "local_java_optimization_configuration",
    ),
    field("bazel_py", "BazelPythonConfiguration", "python_top"),
    field(
        "android",
        "AndroidConfiguration",
        "legacy_main_dex_list_generator",
    ),
    field("android", "AndroidConfiguration", "optimizing_dexer"),
    // Java: com/google/devtools/build/lib/rules/objc/J2ObjcConfiguration.java:100 J2ObjcConfiguration.getDeadCodeReport
    field("j2objc", "J2ObjcConfiguration", "dead_code_report"),
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:188 ProtoConfiguration.protoCompiler
    proto_field("proto_compiler", "@bazel_tools//tools/proto:protoc"),
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:197 ProtoConfiguration.protoToolchainForJava
    proto_field(
        "proto_toolchain_for_java",
        "@bazel_tools//tools/proto:java_toolchain",
    ),
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:206 ProtoConfiguration.protoToolchainForJ2objc
    proto_field(
        "proto_toolchain_for_j2objc",
        "@bazel_tools//tools/j2objc:j2objc_proto_toolchain",
    ),
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:215 ProtoConfiguration.protoToolchainForJavaLite
    proto_field(
        "proto_toolchain_for_java_lite",
        "@bazel_tools//tools/proto:javalite_toolchain",
    ),
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:224 ProtoConfiguration.protoToolchainForCc
    proto_field(
        "proto_toolchain_for_cc",
        "@bazel_tools//tools/proto:cc_toolchain",
    ),
    // Java: com/google/devtools/build/lib/rules/python/PythonConfiguration.java:147 PythonConfiguration.getNativeRulesAllowlist
    field("py", "PythonConfiguration", "native_rules_allowlist"),
];

/// A `ProtoConfiguration` field with its `defaultLabel` (Java `ProtoConstants`).
// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:44 ProtoConstants
const fn proto_field(name: &'static str, default_label: &'static str) -> ConfigurationField {
    ConfigurationField {
        fragment: "proto",
        fragment_class: "ProtoConfiguration",
        name,
        default_label,
        default_in_tool_repository: false,
    }
}

const fn field(
    fragment: &'static str,
    fragment_class: &'static str,
    name: &'static str,
) -> ConfigurationField {
    ConfigurationField {
        fragment,
        fragment_class,
        name,
        default_label: "",
        default_in_tool_repository: false,
    }
}

/// The value of `configuration_field()` (Java `StarlarkLateBoundDefault`, a label-valued
/// `LateBoundDefault` reading a fragment field).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:50 StarlarkLateBoundDefault
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkLateBoundDefault {
    #[allocative(skip)]
    late_bound: Arc<LateBoundDefault>,
    fragment_name: String,
    fragment_field_name: String,
}

starlark_simple_value!(StarlarkLateBoundDefault);

impl StarlarkLateBoundDefault {
    /// Java `getFragmentName()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:107 StarlarkLateBoundDefault.getFragmentName
    pub fn get_fragment_name(&self) -> &str {
        &self.fragment_name
    }

    /// Java `getFragmentFieldName()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:115 StarlarkLateBoundDefault.getFragmentFieldName
    pub fn get_fragment_field_name(&self) -> &str {
        &self.fragment_field_name
    }

    /// The model's late-bound default.
    pub fn late_bound(&self) -> &Arc<LateBoundDefault> {
        &self.late_bound
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:70 StarlarkLateBoundDefault.getDefaultLabel
    fn get_default_label(
        f: &ConfigurationField,
        tools_repository: &RepositoryName,
    ) -> Option<Label> {
        if f.default_label.is_empty() {
            return None;
        }
        Some(if f.default_in_tool_repository {
            Label::parse_canonical_unchecked(&format!(
                "{}{}",
                tools_repository.get_name_with_at(),
                f.default_label
            ))
        } else {
            Label::parse_canonical_unchecked(f.default_label)
        })
    }

    /// Java `forConfigurationField(fragmentClass, fragmentFieldName, toolsRepository)`; the
    /// fragment is given by its Starlark name.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:228 StarlarkLateBoundDefault.forConfigurationField
    pub fn for_configuration_field(
        fragment: &str,
        fragment_field_name: &str,
        tools_repository: &RepositoryName,
    ) -> Result<StarlarkLateBoundDefault, String> {
        let Some(f) = CONFIGURATION_FIELDS
            .iter()
            .find(|f| f.fragment == fragment && f.name == fragment_field_name)
        else {
            return Err(format!(
                "invalid configuration field name '{fragment_field_name}' on fragment \
                 '{fragment}'"
            ));
        };
        Ok(StarlarkLateBoundDefault {
            late_bound: Arc::new(
                LateBoundDefault::label_from_target_configuration(
                    f.fragment_class,
                    Self::get_default_label(f, tools_repository),
                )
                // W4-analysis: names the field for the analysis resolver.
                .with_resolver_name(&format!("configuration_field:{}:{}", f.fragment, f.name)),
            ),
            fragment_name: f.fragment.to_owned(),
            fragment_field_name: f.name.to_owned(),
        })
    }
}

impl fmt::Display for StarlarkLateBoundDefault {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkLateBoundDefault.java:121 StarlarkLateBoundDefault.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<late-bound default>")
    }
}

#[starlark_value(type = "LateBoundDefault")]
impl<'v> StarlarkValue<'v> for StarlarkLateBoundDefault {}

/// Java `x instanceof StarlarkLateBoundDefault`: its late-bound default.
pub fn late_bound_default_of(v: Value) -> Option<Arc<LateBoundDefault>> {
    v.downcast_ref::<StarlarkLateBoundDefault>()
        .map(|d| d.late_bound.clone())
}

#[starlark_module]
pub fn register_bazel_build_api_globals(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/BazelBuildApiGlobals.java:43 BazelBuildApiGlobals.visibility
    fn visibility<'v>(
        #[starlark(require = pos)] value: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // Confirm load visibility is enabled. We manually check the experimental flag here
        // because StarlarkMethod.enableOnlyWithFlag doesn't work for top-level builtins.
        if !blaze_starlark::ThreadContext::get(eval)?
            .semantics
            .experimental_bzl_visibility
        {
            return Err(errorf(
                "Use of `visibility()` requires --experimental_bzl_visibility",
            ));
        }

        // Fail if we're not initializing a .bzl module
        let bzl_file = BzlInitThreadContext::from_or_fail(eval, "visibility()")
            .map_err(eval_exception)?
            .bzl_file
            .clone();
        // Fail if we're not called from the top level. (We prohibit calling visibility() from
        // within helper functions because it's more magical / less readable, and it makes it
        // more difficult for static tooling to mechanically find and modify visibility()
        // declarations.)
        let stack = call_stack(eval);
        if !(stack.len() == 2 && stack[0].name == "<toplevel>" && stack[1].name == "visibility") {
            return Err(errorf(
                "load visibility may only be set at the top level, not inside a function",
            ));
        }

        // Fail if the module's visibility is already set.
        if BzlInitThreadContext::from_or_fail(eval, "visibility()")
            .map_err(eval_exception)?
            .bzl_visibility
            .is_some()
        {
            return Err(errorf("load visibility may not be set more than once"));
        }

        let module_context = module_context_or_throw(eval)?;
        let repo_mapping = &module_context.repo_mapping;

        let repo = bzl_file.get_repository();
        let spec_strings: Vec<String> = if let Some(s) = value.unpack_str() {
            // `visibility("public")`, `visibility("private")`, visibility("//pkg")
            vec![s.to_owned()]
        } else if value.get_type() == "list" {
            // `visibility(["//pkg1", "//pkg2", ...])`
            sequence_cast_strings(value, "visibility list")?
        } else {
            return Err(errorf(format!(
                "Invalid visibility: got '{}', want string or list of strings",
                starlark_type(value)
            )));
        };
        let mut specs = Vec::with_capacity(spec_strings.len());
        for spec_string in &spec_strings {
            specs.push(
                PackageSpecification::from_string_for_bzl_visibility(
                    repo_mapping,
                    repo,
                    spec_string,
                )
                .map_err(eval_exception)?,
            );
        }
        let visibility = BzlVisibility::of(specs);
        BzlInitThreadContext::from_or_fail(eval, "visibility()")
            .map_err(eval_exception)?
            .bzl_visibility = Some(visibility);
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/BazelBuildApiGlobals.java:102 BazelBuildApiGlobals.configurationField
    fn configuration_field<'v>(
        fragment: &str,
        name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkLateBoundDefault> {
        let context = BzlInitThreadContext::from_or_fail(eval, "configuration_field()")
            .map_err(eval_exception)?;
        if !context.fragment_names.iter().any(|f| f == fragment) {
            return Err(errorf(format!(
                "invalid configuration fragment name '{fragment}'"
            )));
        }
        let tools_repository = context.tools_repository.clone();
        drop(context);
        StarlarkLateBoundDefault::for_configuration_field(fragment, name, &tools_repository)
            .map_err(errorf)
    }
}
