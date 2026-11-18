//! Stand-ins for the Java objects Bazel registers for `@_builtins` and as top-level .bzl symbols
//! (`addStarlarkBuiltinsInternal`, `addBzlToplevel`), whose members are only used at analysis
//! time.
//!
//! A stub has the Java object's struct fields that are read while loading. Reading any other
//! attribute yields a [`StubMethod`]; calling it fails loudly (`unported!`), since those methods
//! belong to analysis, which is not ported yet.
//!
//! Java: com/google/devtools/build/lib/bazel/rules/cpp/BazelCcModule.java (`cc_common`),
//! rules/cpp/CcStarlarkInternal.java (`cc_internal`), rules/objc/AppleStarlarkCommon.java
//! (`apple_common`), bazel/rules/python/BazelPyBuiltins.java (`py_builtins`),
//! rules/java/JavaStarlarkCommon.java (`java_common_internal_do_not_use`),
//! rules/android/AndroidStarlarkCommon.java (`android_common`),
//! rules/proto/BazelProtoCommon.java (`proto_common_do_not_use`).

use std::fmt;

use allocative::Allocative;
use blaze_starlark::ThreadContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::bzl_context::BazelModuleContext;

/// A struct field of a stub object that is readable while loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Allocative)]
pub enum StubField {
    /// A `structField = true` method returning `void`/`None`.
    None,
}

/// A Java builtin object whose analysis-time members are not ported.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StubObject {
    /// Java `@StarlarkBuiltin(name = ...)` of the object's class.
    pub type_name: &'static str,
    pub struct_fields: &'static [(&'static str, StubField)],
}

starlark_simple_value!(StubObject);

impl fmt::Display for StubObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.type_name)
    }
}

// PORT: starlark-rust needs a static type name; Java reports each class's own name.
#[starlark_value(type = "builtins_stub")]
impl<'v> StarlarkValue<'v> for StubObject {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        for (name, field) in self.struct_fields {
            if *name == attribute {
                return Some(match field {
                    StubField::None => Value::new_none(),
                });
            }
        }
        Some(heap.alloc(StubMethod {
            owner: self.type_name,
            name: attribute.to_owned(),
        }))
    }

    fn dir_attr(&self) -> Vec<String> {
        self.struct_fields
            .iter()
            .map(|(n, _)| (*n).to_owned())
            .collect()
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

// Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcModuleApi.java:65 CcModuleApi.compilerFlagExists
const CC_COMMON_FIELDS: &[(&str, StubField)] =
    &[("do_not_use_tools_cpp_compiler_present", StubField::None)];

/// `_builtins.internal.cc_common` (Java `BazelCcModule`).
pub fn cc_common() -> StubObject {
    StubObject {
        type_name: "cc_common",
        struct_fields: CC_COMMON_FIELDS,
    }
}

/// The module of the `depth`-th innermost Starlark function on the call stack (Java
/// `Module.ofInnermostEnclosingStarlarkFunction(thread, depth)` with its client data).
///
/// Each frame records where its function was called from; a recorded position lies in the code
/// of the enclosing Starlark function (or the file's top level), so the positions from the top
/// of the stack down name the files of the active Starlark functions, innermost first.
// PORT: move to blaze_starlark (StarlarkThread.getInnermostEnclosingStarlarkFunction).
// Java: net/starlark/java/eval/StarlarkThread.java:530 StarlarkThread.getInnermostEnclosingStarlarkFunction
pub fn module_of_innermost_enclosing_starlark_function(
    eval: &Evaluator,
    depth: usize,
) -> Option<std::sync::Arc<BazelModuleContext>> {
    let mut depth = depth;
    for n in 0..eval.call_stack_count() {
        let Some(location) = eval.call_stack_nth_location(n) else {
            continue;
        };
        if depth == 0 {
            return BazelModuleContext::of_filename(location.filename());
        }
        depth -= 1;
    }
    None
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

/// `_builtins.internal.apple_common` (Java `AppleStarlarkCommon`).
pub fn apple_common() -> StubObject {
    StubObject {
        type_name: "apple_common",
        struct_fields: &[],
    }
}

/// `_builtins.internal.py_builtins` (Java `BazelPyBuiltins`) while loading: `regex_match` (a pure
/// function rules_python's `py_internal` also calls from repository rules and module
/// extensions) is ported; every other member is an analysis-time [`StubMethod`]. The analysis
/// environment (blaze_rules `bazel_environment`) replaces this object with the full port.
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

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // Methods are found through get_methods first; any other member is analysis-time.
        Some(heap.alloc(StubMethod {
            owner: "py_builtins",
            name: attribute.to_owned(),
        }))
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
        // Java `subject.matches(pattern)` (java.util.regex, whole-input match).
        // PORT: the regex crate implements the common subset of java.util.regex. Java's `(?U)`
        // (UNICODE_CHARACTER_CLASS) is the regex crate's default for `\w` etc., while the regex
        // crate's `(?U)` swaps greediness, which cannot change whether the whole input matches;
        // so `(?U)` is dropped. A pattern the crate cannot compile is not ported.
        let translated = pattern.replace("(?U)", "");
        match regex::Regex::new(&format!("^(?:{translated})$")) {
            Ok(re) => Ok(re.is_match(subject)),
            Err(e) => blaze_util::unported!(
                "String.matches",
                "pattern {pattern:?} is not supported by the regex crate: {e}"
            ),
        }
    }
}

/// `_builtins.internal.py_builtins` (Java `BazelPyBuiltins`), loading-phase form.
pub fn py_builtins() -> PyBuiltinsLoading {
    PyBuiltinsLoading
}

/// `android_common` (Java `AndroidStarlarkCommon`).
pub fn android_common() -> StubObject {
    StubObject {
        type_name: "android_common",
        struct_fields: &[],
    }
}

/// `PackageSpecificationInfo` (Java `PackageSpecificationProvider.PROVIDER`).
///
/// PORT: the native provider belongs with the package-group targets (A2); until it exists this
/// stand-in only lets `@_builtins` re-export the symbol.
pub fn package_specification_info() -> StubObject {
    StubObject {
        type_name: "PackageSpecificationInfo",
        struct_fields: &[],
    }
}

/// `proto_common_do_not_use` (Java `BazelProtoCommon`). Its flag accessor is called while
/// loading `@_builtins`; its other methods are analysis-time.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BazelProtoCommon;

starlark_simple_value!(BazelProtoCommon);

impl fmt::Display for BazelProtoCommon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<proto_common>")
    }
}

#[starlark_value(type = "proto_common")]
impl<'v> StarlarkValue<'v> for BazelProtoCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("proto_common", bazel_proto_common_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // Methods are found through get_methods first; any other member is analysis-time.
        Some(heap.alloc(StubMethod {
            owner: "proto_common",
            name: attribute.to_owned(),
        }))
    }
}

/// Java `BuiltinRestriction.failIfCalledOutsideAllowlist(thread, ImmutableSet.of())`: only
/// `@_builtins` code may call.
///
/// PORT: move to blaze_packages BuiltinRestriction when it is ported.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:168 BuiltinRestriction.failIfCalledOutsideAllowlist
fn fail_if_called_outside_builtins_allowlist(eval: &Evaluator) -> anyhow::Result<()> {
    let ctx = BazelModuleContext::of_innermost_bzl(eval)
        .ok_or_else(|| anyhow::anyhow!("NullPointerException: not called from a .bzl file"))?;
    if ctx.label.get_repository().get_name() == "_builtins" {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "file '{}' cannot use private API",
        ctx.label.get_canonical_form()
    ))
}

#[starlark_module]
fn bazel_proto_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/proto/BazelProtoCommon.java:45 BazelProtoCommon.getDefineProtoToolchains
    fn incompatible_enable_proto_toolchain_resolution<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let _ = this;
        fail_if_called_outside_builtins_allowlist(eval)?;
        let cx = ThreadContext::get(eval)?;
        Ok(cx.semantics.incompatible_enable_proto_toolchain_resolution)
    }
}
