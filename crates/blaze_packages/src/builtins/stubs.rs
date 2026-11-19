//! Stand-ins for the Java objects Bazel registers for `@_builtins` and as top-level .bzl symbols
//! (`addStarlarkBuiltinsInternal`, `addBzlToplevel`), whose members are only used at analysis
//! time.
//!
//! A stub has exactly the fields and methods of its Java class (the `@StarlarkMethod` names),
//! so `hasattr`, `getattr(x, name, default)`, `dir` and a missing attribute behave as in Java
//! (`Starlark.hasattr`, `Starlark.getattr`, `Starlark.dir`). A struct field that is read while
//! loading has its value; reading any other struct field fails loudly (`unported!`). A method
//! reads as a [`StubMethod`]; calling it fails loudly, since those methods belong to analysis.
//!
//! Java: com/google/devtools/build/lib/bazel/rules/cpp/BazelCcModule.java (`cc_common`),
//! rules/cpp/CcStarlarkInternal.java (`cc_internal`), rules/objc/AppleStarlarkCommon.java
//! (`apple_common`), bazel/rules/python/BazelPyBuiltins.java (`py_builtins`),
//! rules/java/JavaStarlarkCommon.java (`java_common_internal_do_not_use`),
//! rules/android/AndroidStarlarkCommon.java (`android_common`).

use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::bzl_context::BazelModuleContext;

/// A struct field (`@StarlarkMethod(structField = true)`) of a stub object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum StubField {
    /// A `structField = true` method returning `void`/`None`.
    None,
    /// A struct field whose value is not ported: the field exists (`hasattr`, `dir`), reading
    /// it fails loudly. The payload names the Java method.
    Unported(&'static str),
}

/// A Java builtin object whose analysis-time members are not ported.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StubObject {
    /// Java `@StarlarkBuiltin(name = ...)` of the object's class (`Starlark.type(x)`).
    pub type_name: &'static str,
    /// The `@StarlarkMethod(structField = true)` names of the Java class.
    pub struct_fields: &'static [(&'static str, StubField)],
    /// The other `@StarlarkMethod` names of the Java class.
    pub methods: &'static [&'static str],
}

starlark_simple_value!(StubObject);

impl fmt::Display for StubObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.type_name)
    }
}

/// Java `Starlark.getattr`'s error for a value that is not a `Structure`:
/// `"'%s' value has no field or method '%s'%s"` with `SpellChecker.didYouMean(name, dir(x))`.
// Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
pub(crate) fn no_field_or_method_message(
    type_name: &str,
    attribute: &str,
    mut dir: Vec<String>,
) -> String {
    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir (a TreeSet: sorted).
    dir.sort();
    format!(
        "'{}' value has no field or method '{}'{}",
        type_name,
        attribute,
        crate::attribute::string_util::did_you_mean(attribute, &dir)
    )
}

// PORT: starlark-rust needs a static type name; Java reports each class's own name
// (`type_name`), which the missing-attribute message uses.
#[starlark_value(type = "builtins_stub")]
impl<'v> StarlarkValue<'v> for StubObject {
    // Java: net/starlark/java/eval/Starlark.java:978 Starlark.getattr
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // StarlarkMethod-annotated field or method?
        for (name, field) in self.struct_fields {
            if *name == attribute {
                // `method.callField(x, semantics, mu)`.
                return Some(match field {
                    StubField::None => Value::new_none(),
                    StubField::Unported(java_method) => blaze_util::unported!(
                        *java_method,
                        "the value of the struct field {}.{}",
                        self.type_name,
                        attribute
                    ),
                });
            }
        }
        if self.methods.contains(&attribute) {
            // `new BuiltinFunction(x, name, method)`.
            return Some(heap.alloc(StubMethod {
                owner: self.type_name,
                name: attribute.to_owned(),
            }));
        }
        // Not a Structure: getattr yields the default, or the "no field or method" error.
        None
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        // `CallUtils.getAnnotatedMethods(semantics, x.getClass()).containsKey(name)`; a struct
        // field is not called.
        self.struct_fields
            .iter()
            .any(|(name, _)| *name == attribute)
            || self.methods.contains(&attribute)
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir
    fn dir_attr(&self) -> Vec<String> {
        self.struct_fields
            .iter()
            .map(|(n, _)| (*n).to_owned())
            .chain(self.methods.iter().map(|n| (*n).to_owned()))
            .collect()
    }

    // Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(no_field_or_method_message(
            self.type_name,
            attribute,
            self.dir_attr(),
        ))
    }
}

/// An analysis-time method of a [`StubObject`].
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StubMethod {
    owner: &'static str,
    name: String,
}

starlark_simple_value!(StubMethod);

impl StubMethod {
    pub fn new(owner: &'static str, name: &str) -> Self {
        StubMethod {
            owner,
            name: name.to_owned(),
        }
    }
}

impl fmt::Display for StubMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<built-in method {} of {} value>", self.name, self.owner)
    }
}

#[starlark_value(type = "builtin_function_or_method")]
impl<'v> StarlarkValue<'v> for StubMethod {
    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &Arguments<'v, '_>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        blaze_util::unported!(format!("{}.{}", self.owner, self.name))
    }
}

// The `@StarlarkMethod(structField = true)` members of `CcModuleApi` (none is flag-guarded).
const CC_COMMON_FIELDS: &[(&str, StubField)] = &[
    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:56 CcModuleApi.getCcToolchainProvider
    (
        "CcToolchainInfo",
        StubField::Unported("CcModuleApi.getCcToolchainProvider"),
    ),
    // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:65 CcModuleApi.compilerFlagExists
    ("do_not_use_tools_cpp_compiler_present", StubField::None),
];

// The other `@StarlarkMethod` members of `CcModuleApi`, in declaration order (`CcModule` and
// `BazelCcModule` add none; none is flag-guarded).
// Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:75 CcModuleApi
const CC_COMMON_METHODS: &[&str] = &[
    "compile",
    "link",
    "configure_features",
    "create_lto_compilation_context",
    "create_compilation_outputs",
    "merge_compilation_outputs",
    "get_tool_for_action",
    "get_execution_requirements",
    "is_enabled",
    "action_is_enabled",
    "get_memory_inefficient_command_line",
    "get_environment_variables",
    "create_compile_variables",
    "create_link_variables",
    "empty_variables",
    "create_library_to_link",
    "create_linker_input",
    "check_experimental_cc_shared_library",
    "incompatible_disable_objc_library_transition",
    "add_go_exec_groups_to_binary_rules",
    "create_linking_context",
    "merge_cc_infos",
    "create_compilation_context",
    "create_module_map",
    "legacy_cc_flags_make_variable_do_not_use",
    "create_cc_toolchain_config_info",
    "create_linking_context_from_compilation_outputs",
    "merge_compilation_contexts",
    "get_tool_requirement_for_action",
    "create_extra_link_time_library",
];

/// `_builtins.internal.cc_common` (Java `BazelCcModule`).
pub fn cc_common() -> StubObject {
    StubObject {
        type_name: "cc_common",
        struct_fields: CC_COMMON_FIELDS,
        methods: CC_COMMON_METHODS,
    }
}

/// The module of the `depth`-th innermost Starlark function on the call stack (Java
/// `Module.ofInnermostEnclosingStarlarkFunction(thread, depth)` with its client data).
// Java: net/starlark/java/eval/StarlarkThread.java:530 StarlarkThread.getInnermostEnclosingStarlarkFunction
pub fn module_of_innermost_enclosing_starlark_function<'v>(
    eval: &Evaluator<'v, '_, '_>,
    depth: usize,
) -> Option<std::sync::Arc<BazelModuleContext>> {
    BazelModuleContext::of_innermost_enclosing_starlark_function(eval, depth)
}

/// Java `BuiltinRestriction.AllowlistEntry.allows`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:124 BuiltinRestriction.AllowlistEntry.allows
pub(crate) fn allowlist_entry_allows(
    apparent_repo_name: &str,
    package_prefix: &str,
    label: &blaze_cmdline::Label,
    repo_mapping: &blaze_cmdline::RepositoryMapping,
) -> bool {
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:129 BuiltinRestriction.AllowlistEntry.reposMatch
    let given_name = label.get_repository();
    let repos_match = if given_name.is_main() {
        // The main repository may be one of the allowlisted rulesets, in which case we need to
        // fall back to interpreting allowedName as the apparent repo name.
        apparent_repo_name == blaze_cmdline::repository_name::MAIN.get_name()
            || repo_mapping.get(apparent_repo_name).is_main()
    } else if *given_name == *blaze_cmdline::repository_name::BAZEL_TOOLS {
        apparent_repo_name == blaze_cmdline::repository_name::BAZEL_TOOLS.get_name()
    } else {
        // allowedName is a module name and givenName is a real canonical repo name, so it
        // belongs to any version of that module if and only if it contains <allowedName>+ as a
        // prefix.
        given_name
            .get_name()
            .starts_with(&format!("{}+", apparent_repo_name))
    };
    repos_match
        && label
            .get_package_fragment()
            .starts_with(&blaze_vfs::PathFragment::create(package_prefix))
}

// The `@StarlarkMethod(structField = true)` members of `AppleCommonApi` (`AppleStarlarkCommon`
// adds no member; none is flag-guarded).
const APPLE_COMMON_FIELDS: &[(&str, StubField)] = &[
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:47 AppleCommonApi.getPlatformTypeStruct
    (
        "platform_type",
        StubField::Unported("AppleCommonApi.getPlatformTypeStruct"),
    ),
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:64 AppleCommonApi.getPlatformStruct
    (
        "platform",
        StubField::Unported("AppleCommonApi.getPlatformStruct"),
    ),
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:85 AppleCommonApi.getXcodeVersionPropertiesConstructor
    (
        "XcodeProperties",
        StubField::Unported("AppleCommonApi.getXcodeVersionPropertiesConstructor"),
    ),
    // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:100 AppleCommonApi.getXcodeVersionConfigConstructor
    (
        "XcodeVersionConfig",
        StubField::Unported("AppleCommonApi.getXcodeVersionConfigConstructor"),
    ),
];

// The other `@StarlarkMethod` members of `AppleCommonApi`, in declaration order.
// Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleCommonApi.java:40 AppleCommonApi
const APPLE_COMMON_METHODS: &[&str] = &[
    "apple_toolchain",
    "apple_host_system_env",
    "target_apple_env",
    "dotted_version",
];

/// `_builtins.internal.apple_common` (Java `AppleStarlarkCommon`).
pub fn apple_common() -> StubObject {
    StubObject {
        type_name: "apple_common",
        struct_fields: APPLE_COMMON_FIELDS,
        methods: APPLE_COMMON_METHODS,
    }
}

// The `@StarlarkMethod` members of `PyBuiltins` other than `regex_match`, in declaration order
// (`BazelPyBuiltins` adds none; none is a struct field or flag-guarded).
// Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:66 PyBuiltins
const PY_BUILTINS_STUB_METHODS: &[&str] = &[
    "is_bzlmod_enabled",
    "is_singleton_depset",
    "get_rule_name",
    "get_current_os_name",
    "get_label_repo_runfiles_path",
    "expand_location_and_make_variables",
    "create_repo_mapping_manifest",
    "merge_runfiles_with_generated_inits_empty_files_supplier",
    "declare_constant_metadata_file",
    "create_sources_only_manifest",
    "copy_without_caching",
    "are_action_listeners_enabled",
    "add_py_extra_pseudo_action",
];

/// `_builtins.internal.py_builtins` (Java `BazelPyBuiltins`) while loading: `regex_match` (a pure
/// function rules_python's `py_internal` also calls from repository rules and module
/// extensions) is ported; every other member of the Java class is an analysis-time
/// [`StubMethod`]. The analysis environment (blaze_rules `bazel_environment`) replaces this
/// object with the full port.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct PyBuiltinsLoading;

starlark_simple_value!(PyBuiltinsLoading);

impl fmt::Display for PyBuiltinsLoading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<py_builtins>")
    }
}

#[starlark_value(type = "py_builtins")]
impl<'v> StarlarkValue<'v> for PyBuiltinsLoading {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("py_builtins", py_builtins_loading_methods);
        Some(RES.methods())
    }

    // Java: net/starlark/java/eval/Starlark.java:978 Starlark.getattr
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // Methods are found through get_methods first (`regex_match`); the other members of the
        // Java class are analysis-time. Any other name is not an attribute.
        if PY_BUILTINS_STUB_METHODS.contains(&attribute) {
            return Some(heap.alloc(StubMethod {
                owner: "py_builtins",
                name: attribute.to_owned(),
            }));
        }
        None
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        PY_BUILTINS_STUB_METHODS.contains(&attribute)
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir
    fn dir_attr(&self) -> Vec<String> {
        PY_BUILTINS_STUB_METHODS
            .iter()
            .map(|n| (*n).to_owned())
            .collect()
    }

    // Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        let mut dir = self.dir_attr();
        dir.push("regex_match".to_owned());
        Some(no_field_or_method_message("py_builtins", attribute, dir))
    }
}

#[starlark_module]
fn py_builtins_loading_methods(builder: &mut MethodsBuilder) {
    /// Return true if subject matches pattern; pattern is implicitly anchored with ^ and $
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:98 PyBuiltins.regexMatch
    fn regex_match<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] subject: &str,
        #[starlark(require = pos)] pattern: &str,
    ) -> anyhow::Result<bool> {
        let _ = this;
        // Java `subject.matches(pattern)` (java.util.regex, whole-input match), through
        // `blaze_util::java_regex` (its module docs list what is `unported!`). The subject is an
        // internal string, matched by its Latin-1 chars as in Java.
        // PORT: a pattern Java rejects throws the unchecked `PatternSyntaxException` out of the
        // builtin (a crash in Bazel); here it panics with Java's message.
        match blaze_util::java_regex::Pattern::compile(pattern, 0) {
            Ok(compiled) => Ok(compiled.matches(subject)),
            Err(message) => panic!("java.util.regex.PatternSyntaxException: {message}"),
        }
    }
}

/// `_builtins.internal.py_builtins` (Java `BazelPyBuiltins`), loading-phase form.
pub fn py_builtins() -> PyBuiltinsLoading {
    PyBuiltinsLoading
}

// The `@StarlarkMethod` members of `AndroidStarlarkCommon`, in declaration order (no struct
// field; none is flag-guarded).
// Java: com/google/devtools/build/lib/rules/android/AndroidStarlarkCommon.java:52 AndroidStarlarkCommon
const ANDROID_COMMON_METHODS: &[&str] = &["resource_source_directory", "create_dex_merger_actions"];

/// `android_common` (Java `AndroidStarlarkCommon`).
pub fn android_common() -> StubObject {
    StubObject {
        type_name: "android_common",
        struct_fields: &[],
        methods: ANDROID_COMMON_METHODS,
    }
}

// PORT: the former stand-ins `package_specification_info()` (`PackageSpecificationInfo`) and
// `BazelProtoCommon` (`proto_common_do_not_use`) are gone: the objects are ported in
// `starlark_defs::builtin_providers` and registered from there
// (`register_rule_class_provider_bzl_toplevels`).

#[cfg(test)]
mod tests {
    use starlark::environment::{GlobalsBuilder, Module};
    use starlark::eval::Evaluator;
    use starlark::syntax::{AstModule, Dialect};
    use starlark::values::Value;

    use super::*;

    /// Evaluates the expression `source` with the stubs bound to their Starlark names and
    /// returns its repr, or the error message.
    fn eval(source: &str) -> Result<String, String> {
        let mut builder = GlobalsBuilder::standard();
        builder.set("cc_common", cc_common());
        builder.set("apple_common", apple_common());
        builder.set("android_common", android_common());
        builder.set("py_builtins", py_builtins());
        let globals = builder.build();
        Module::with_temp_heap(|module| {
            let ast = AstModule::parse("stubs_test.bzl", source.to_owned(), &Dialect::Standard)
                .map_err(|e| format!("{e}"))?;
            let mut eval = Evaluator::new(&module);
            let value: Value = eval
                .eval_module(ast, &globals)
                .map_err(|e| format!("{e}"))?;
            Ok(value.to_repr())
        })
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    //   return (x instanceof Structure && ((Structure) x).getValue(name) != null)
    //       || CallUtils.getAnnotatedMethods(semantics, x.getClass()).containsKey(name);
    #[test]
    fn hasattr_is_true_only_for_the_java_class_members() {
        // AndroidStarlarkCommon has exactly two @StarlarkMethods.
        assert_eq!(
            eval("hasattr(android_common, 'resource_source_directory')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(android_common, 'create_dex_merger_actions')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(android_common, 'anything')").unwrap(),
            "False"
        );
        // CcModuleApi.
        assert_eq!(eval("hasattr(cc_common, 'compile')").unwrap(), "True");
        assert_eq!(
            eval("hasattr(cc_common, 'do_not_use_tools_cpp_compiler_present')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(cc_common, 'CcToolchainInfo')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(cc_common, 'implementation_deps_allowed_by_allowlist')").unwrap(),
            "False"
        );
        // AppleCommonApi.
        assert_eq!(
            eval("hasattr(apple_common, 'dotted_version')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(apple_common, 'platform_type')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(apple_common, 'new_objc_provider')").unwrap(),
            "False"
        );
        // PyBuiltins.
        assert_eq!(eval("hasattr(py_builtins, 'regex_match')").unwrap(), "True");
        assert_eq!(
            eval("hasattr(py_builtins, 'get_rule_name')").unwrap(),
            "True"
        );
        assert_eq!(
            eval("hasattr(py_builtins, 'no_such_thing')").unwrap(),
            "False"
        );
    }

    // Java: net/starlark/java/eval/Starlark.java:1011 Starlark.getattr
    //   } else if (defaultValue != null) {
    //     return defaultValue;
    //   }
    #[test]
    fn getattr_with_default_yields_the_default_for_other_names() {
        assert_eq!(
            eval("getattr(android_common, 'anything', 42)").unwrap(),
            "42"
        );
        assert_eq!(
            eval("getattr(cc_common, 'no_such_method', None)").unwrap(),
            "None"
        );
        assert_eq!(eval("getattr(apple_common, 'Objc', 'd')").unwrap(), "\"d\"");
        assert_eq!(eval("getattr(py_builtins, 'nope', 1)").unwrap(), "1");
        // A member is found, not defaulted.
        assert_eq!(
            eval("getattr(android_common, 'create_dex_merger_actions', 42) == 42").unwrap(),
            "False"
        );
        // CcModuleApi.compilerFlagExists is a `void` struct field: None.
        assert_eq!(
            eval("getattr(cc_common, 'do_not_use_tools_cpp_compiler_present', 42)").unwrap(),
            "None"
        );
    }

    // Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
    //   throw Starlark.errorf(
    //       "'%s' value has no field or method '%s'%s",
    //       Starlark.type(x), name, SpellChecker.didYouMean(name, dir(mu, semantics, x)));
    #[test]
    fn a_missing_attribute_is_javas_no_field_or_method_error() {
        let error = eval("android_common.anything").unwrap_err();
        assert!(
            error.contains("'android_common' value has no field or method 'anything'"),
            "{error}"
        );
        let error = eval("getattr(cc_common, 'compil')").unwrap_err();
        assert!(
            error.contains(
                "'cc_common' value has no field or method 'compil' (did you mean 'compile'?)"
            ),
            "{error}"
        );
        let error = eval("py_builtins.regex_matc").unwrap_err();
        assert!(
            error.contains(
                "'py_builtins' value has no field or method 'regex_matc' (did you mean \
                 'regex_match'?)"
            ),
            "{error}"
        );
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir (sorted names of the
    // annotated methods).
    #[test]
    fn dir_lists_the_java_class_members() {
        assert_eq!(
            eval("dir(android_common)").unwrap(),
            "[\"create_dex_merger_actions\", \"resource_source_directory\"]"
        );
        assert_eq!(
            eval("dir(apple_common)").unwrap(),
            "[\"XcodeProperties\", \"XcodeVersionConfig\", \"apple_host_system_env\", \
             \"apple_toolchain\", \"dotted_version\", \"platform\", \"platform_type\", \
             \"target_apple_env\"]"
        );
        assert_eq!(eval("len(dir(cc_common))").unwrap(), "32");
        assert_eq!(eval("len(dir(py_builtins))").unwrap(), "14");
    }

    // The analysis-time methods are not ported: calling one fails loudly.
    #[test]
    #[should_panic(expected = "android_common.create_dex_merger_actions")]
    fn calling_a_stub_method_fails_loudly() {
        let _ = eval("android_common.create_dex_merger_actions()");
    }

    // A struct field whose value is not ported fails loudly when read (but not for hasattr).
    #[test]
    #[should_panic(expected = "CcModuleApi.getCcToolchainProvider")]
    fn reading_an_unported_struct_field_fails_loudly() {
        let _ = eval("cc_common.CcToolchainInfo");
    }
}
