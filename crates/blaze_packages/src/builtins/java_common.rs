//! `_builtins.internal.java_common_internal_do_not_use` (Java `JavaStarlarkCommon`): the flag
//! accessors rules_java calls while loading; every other member is analysis-time and fails
//! loudly when called.
//!
//! Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java,
//! com/google/devtools/build/lib/packages/BuiltinRestriction.java.

use std::fmt;

use allocative::Allocative;
use blaze_starlark::ThreadContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::builtins::stubs::{StubMethod, allowlist_entry_allows};
use crate::bzl_context::BazelModuleContext;

/// The "default" allowlist for restricted APIs added to aid the Java to Starlark migration
/// (apparent repository name, package prefix).
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:35 BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST
pub const INTERNAL_STARLARK_API_ALLOWLIST: &[(&str, &str)] = &[
    // Testing
    ("", "test"),
    ("", "bazel_internal/test_rules"),
    // BuildInfo
    ("", "tools/build_defs/build_info"),
    ("bazel_tools", "tools/build_defs/build_info"),
    // Android rules
    ("", "bazel_internal/test_rules/cc"),
    ("", "tools/build_defs/android"),
    ("", "third_party/bazel_rules/rules_android"),
    ("rules_android", ""),
    ("build_bazel_rules_android", ""),
    // Apple rules
    ("", "third_party/apple_crosstool"),
    ("", "third_party/cpptoolchains/portable_llvm/build_defs"),
    ("", "third_party/bazel_rules/rules_apple"),
    ("rules_apple", ""),
    ("build_bazel_rules_apple", ""),
    // Cc rules
    ("", "third_party/bazel_rules/rules_cc"),
    ("", "tools/build_defs/cc"),
    ("rules_cc", ""),
    // Java rules
    ("", "third_party/bazel_rules/rules_java/java"),
    ("rules_java", "java"),
    // Rust rules
    ("", "third_party/bazel_rules/rules_rust/rust/private"),
    ("", "third_party/crubit"),
    ("rules_rust", "rust/private"),
    // CUDA rules
    ("", "third_party/gpus/cuda"),
    // Packaging rules
    ("", "tools/build_defs/packaging"),
    // Go rules
    ("", "tools/build_defs/go"),
    // Proto rules
    ("", "third_party/protobuf"),
    ("protobuf", ""),
    ("com_google_protobuf", ""),
    // Shell rules
    ("rules_shell", ""),
];

/// Java `BuiltinRestriction.failIfCalledOutsideDefaultAllowlist`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:181 BuiltinRestriction.failIfCalledOutsideDefaultAllowlist
pub fn fail_if_called_outside_default_allowlist(eval: &Evaluator) -> anyhow::Result<()> {
    // Java: BazelModuleContext.ofInnermostBzlOrThrow.
    let ctx = BazelModuleContext::of_innermost_bzl(eval)
        .expect("NullPointerException: no BazelModuleContext for the innermost .bzl");
    let label = &ctx.label;
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:214 BuiltinRestriction.isNotAllowed
    if label.get_repository() == &*blaze_cmdline::repository_name::BUILTINS {
        return Ok(());
    }
    if INTERNAL_STARLARK_API_ALLOWLIST
        .iter()
        .any(|(repo, pkg)| allowlist_entry_allows(repo, pkg, label, &ctx.repo_mapping))
    {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "file '{}' cannot use private API",
        label.get_canonical_form()
    ))
}

/// Java `JavaStarlarkCommon`.
// Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:73 JavaStarlarkCommon
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct JavaStarlarkCommon;

starlark_simple_value!(JavaStarlarkCommon);

impl fmt::Display for JavaStarlarkCommon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<java_common>")
    }
}

#[starlark_value(type = "java_common")]
impl<'v> StarlarkValue<'v> for JavaStarlarkCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("java_common", java_common_methods);
        Some(RES.methods())
    }

    // Java: net/starlark/java/eval/Starlark.java:978 Starlark.getattr
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // The ported methods are found through get_methods first. Any other @StarlarkMethod of
        // the Java class is analysis-time: a stub that fails loudly when called (a struct field:
        // when read). Other names are not members (`getattr` yields its default, `hasattr` is
        // False).
        if JAVA_COMMON_STRUCT_FIELDS.contains(&attribute) {
            blaze_util::unported!(
                "JavaStarlarkCommon",
                "the value of the struct field java_common.{}",
                attribute
            );
        }
        if JAVA_COMMON_METHODS.contains(&attribute) {
            Some(heap.alloc(StubMethod::new(
                "java_common_internal_do_not_use",
                attribute,
            )))
        } else {
            None
        }
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        // A struct field is not read.
        JAVA_COMMON_STRUCT_FIELDS.contains(&attribute) || JAVA_COMMON_METHODS.contains(&attribute)
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir
    fn dir_attr(&self) -> Vec<String> {
        // The ported methods are listed through get_methods.
        JAVA_COMMON_STRUCT_FIELDS
            .iter()
            .chain(JAVA_COMMON_METHODS.iter())
            .filter(|name| !JAVA_COMMON_PORTED_METHODS.contains(name))
            .map(|name| (*name).to_owned())
            .collect()
    }

    // Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(super::stubs::no_field_or_method_message(
            "java_common",
            attribute,
            JAVA_COMMON_STRUCT_FIELDS
                .iter()
                .chain(JAVA_COMMON_METHODS.iter())
                .map(|name| (*name).to_owned())
                .collect(),
        ))
    }
}

/// The methods of `java_common_methods` (ported; keep in step with it).
const JAVA_COMMON_PORTED_METHODS: &[&str] = &[
    "_google_legacy_api_enabled",
    "_incompatible_java_info_merge_runtime_module_flags",
    "incompatible_disable_non_executable_java_binary",
];

/// The `@StarlarkMethod(structField = true)` names of Java `JavaCommonApi` (implemented by
/// `JavaStarlarkCommon`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:602 JavaCommonApi.getJavaToolchainProvider
const JAVA_COMMON_STRUCT_FIELDS: &[&str] =
    &["BootClassPathInfo", "JavaRuntimeInfo", "JavaToolchainInfo"];

/// The other `@StarlarkMethod` names of Java `JavaCommonApi` (none is flag-guarded).
// Java: com/google/devtools/build/lib/starlarkbuildapi/java/JavaCommonApi.java:47 JavaCommonApi
const JAVA_COMMON_METHODS: &[&str] = &[
    "_check_java_toolchain_is_declared_on_rule",
    "_google_legacy_api_enabled",
    "_incompatible_java_info_merge_runtime_module_flags",
    "check_provider_instances",
    "collect_native_deps_dirs",
    "compile",
    "create_compilation_action",
    "create_header_compilation_action",
    "default_javac_opts",
    "expand_java_opts",
    "get_runtime_classpath_for_archive",
    "incompatible_disable_non_executable_java_binary",
    "merge",
    "pack_sources",
    "run_ijar",
    "stamp_jar",
    "target_kind",
    "tokenize_javacopts",
];

/// `_builtins.internal.java_common_internal_do_not_use`.
pub fn java_common_internal() -> JavaStarlarkCommon {
    JavaStarlarkCommon
}

#[starlark_module]
fn java_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:383 JavaStarlarkCommon.isLegacyGoogleApiEnabled
    fn _google_legacy_api_enabled<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let _ = this;
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(ThreadContext::get(eval)?
            .semantics
            .experimental_google_legacy_api)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:389 JavaStarlarkCommon.isJavaInfoMergeRuntimeModuleFlagsEnabled
    fn _incompatible_java_info_merge_runtime_module_flags<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let _ = this;
        fail_if_called_outside_default_allowlist(eval)?;
        Ok(ThreadContext::get(eval)?
            .semantics
            .incompatible_java_info_merge_runtime_module_flags)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaStarlarkCommon.java:398 JavaStarlarkCommon.incompatibleDisableNonExecutableJavaBinary
    fn incompatible_disable_non_executable_java_binary<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let _ = this;
        Ok(ThreadContext::get(eval)?
            .semantics
            .incompatible_disable_non_executable_java_binary)
    }
}
