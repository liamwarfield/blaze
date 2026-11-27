//! `_builtins.internal.cc_internal` (Java `CcStarlarkInternal`): the members rules_cc calls while
//! loading (`check_private_api`, `create_header_info`, `create_header_info_with_deps`,
//! `freeze`); every other member is analysis-time and fails loudly when called.
//!
//! Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java,
//! com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java (`HeaderInfo`).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::AllocList;
use starlark::values::none::NoneType;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value};

use crate::builtins::stubs::{
    StubMethod, allowlist_entry_allows, module_of_innermost_enclosing_starlark_function,
};

/// Java `CcStarlarkInternal`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:87 CcStarlarkInternal
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct CcStarlarkInternal;

starlark_simple_value!(CcStarlarkInternal);

impl fmt::Display for CcStarlarkInternal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<cc_internal>")
    }
}

#[starlark_value(type = "cc_internal")]
impl<'v> StarlarkValue<'v> for CcStarlarkInternal {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("cc_internal", cc_internal_methods);
        Some(RES.methods())
    }

    // Java: net/starlark/java/eval/Starlark.java:978 Starlark.getattr
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // The ported methods are found through get_methods first. Any other @StarlarkMethod of
        // the Java class is analysis-time: a stub that fails loudly when called. Other names
        // are not members (`getattr` yields its default, `hasattr` is False).
        if CC_STARLARK_INTERNAL_METHODS.contains(&attribute) {
            Some(heap.alloc(StubMethod::new("cc_internal", attribute)))
        } else {
            None
        }
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr
    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        CC_STARLARK_INTERNAL_METHODS.contains(&attribute)
    }

    // Java: net/starlark/java/eval/Starlark.java:1024 Starlark.dir
    fn dir_attr(&self) -> Vec<String> {
        // The ported methods are listed through get_methods.
        CC_STARLARK_INTERNAL_METHODS
            .iter()
            .filter(|name| !CC_STARLARK_INTERNAL_PORTED_METHODS.contains(name))
            .map(|name| (*name).to_owned())
            .collect()
    }

    // Java: net/starlark/java/eval/Starlark.java:1015 Starlark.getattr
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(super::stubs::no_field_or_method_message(
            "cc_internal",
            attribute,
            CC_STARLARK_INTERNAL_METHODS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        ))
    }
}

/// The methods of `cc_internal_methods` (ported; keep in step with it).
const CC_STARLARK_INTERNAL_PORTED_METHODS: &[&str] = &[
    "check_private_api",
    "freeze",
    "create_header_info",
    "create_header_info_with_deps",
];

/// The `@StarlarkMethod` names of Java `CcStarlarkInternal` (none is a struct field or
/// flag-guarded).
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:87 CcStarlarkInternal
const CC_STARLARK_INTERNAL_METHODS: &[&str] = &[
    "absolute_symlink",
    "actions2ctx_cheat",
    "aspect_class",
    "cc_toolchain_features",
    "cc_toolchain_variables",
    "check_private_api",
    "check_toplevel",
    "collect_per_file_lto_backend_opts",
    "combine_cc_toolchain_variables",
    "compute_output_name_prefix_dir",
    "create_cc_compile_action",
    "create_cc_compile_action_template",
    "create_header_info",
    "create_header_info_with_deps",
    "create_lto_backend_action",
    "create_lto_backend_action_template",
    "declare_compile_output_file",
    "declare_other_output_file",
    "declare_shareable_directory",
    "dynamic_library_soname",
    "dynamic_library_symlink",
    "dynamic_library_symlink2",
    "exec_os",
    "expand_and_tokenize",
    "freeze",
    "get_artifact_name_extension_for_category",
    "get_artifact_name_for_category",
    "get_link_args",
    "intern_seq",
    "intern_string_sequence_variable_value",
    "is_package_headers_checking_mode_set",
    "is_package_headers_checking_mode_set_for_aspect",
    "is_tree_artifact",
    "package_headers_checking_mode",
    "package_headers_checking_mode_for_aspect",
    "per_file_copts",
    "rule_class",
    "solib_symlink_action",
    "stl_computed_default",
    "wrap_link_actions",
];

/// `_builtins.internal.cc_internal`.
pub fn cc_internal() -> CcStarlarkInternal {
    CcStarlarkInternal
}

/// The data of a `HeaderInfo` created while loading.
///
/// PORT: while loading no `Artifact` exists, so every artifact field of a HeaderInfo created
/// then is `None` or empty (Java's casts reject anything else); only the dependency structure is
/// kept.
#[derive(Debug, Allocative)]
pub struct HeaderInfoData {
    deps: Vec<Arc<HeaderInfoData>>,
    merged_deps: Vec<Arc<HeaderInfoData>>,
}

/// Java `CcCompilationContext.HeaderInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:599 CcCompilationContext.HeaderInfo
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct HeaderInfo(pub Arc<HeaderInfoData>);

starlark_simple_value!(HeaderInfo);

impl fmt::Display for HeaderInfo {
    // Java: StarlarkValue.repr default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.rules.cpp.CcCompilationContext$HeaderInfo>",
        )
    }
}

#[starlark_value(type = "HeaderInfo")]
impl<'v> StarlarkValue<'v> for HeaderInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:678 HeaderInfo.getSeparateModule
            "separate_module" | "header_module" | "pic_header_module" | "separate_pic_module" => {
                Some(Value::new_none())
            }
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:708 HeaderInfo.getModularPublicHeaders
            "modular_public_headers"
            | "modular_private_headers"
            | "textual_headers"
            | "separate_module_headers" => Some(heap.alloc(AllocList(Vec::<Value<'v>>::new()))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        self.get_attr(attribute, heap).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        [
            "header_module",
            "modular_private_headers",
            "modular_public_headers",
            "pic_header_module",
            "separate_module",
            "separate_module_headers",
            "separate_pic_module",
            "textual_headers",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }
}

/// Java `Sequence.cast(x, Artifact.class, what)` for loading time: only an empty sequence
/// passes (no `Artifact` exists while loading).
// Java: net/starlark/java/eval/Sequence.java:200 Sequence.cast
fn cast_empty_artifact_sequence<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> anyhow::Result<()> {
    let mut it = x
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("for {}, got {}, want sequence", what, x.get_type()))?;
    if let Some(first) = it.next() {
        return Err(anyhow::anyhow!(
            "at index 0 of {}, got element of type {}, want File",
            what,
            first.get_type()
        ));
    }
    Ok(())
}

/// Java `nullIfNone(x, DerivedArtifact.class)` for loading time: only `None` passes.
fn null_if_none_artifact(x: Value, what: &str) -> anyhow::Result<()> {
    if x.is_none() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "got value of type '{}' for '{}', want 'File'",
        x.get_type(),
        what
    ))
}

/// Java `Sequence.cast(x, HeaderInfo.class, what)`.
fn cast_header_infos<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> anyhow::Result<Vec<Arc<HeaderInfoData>>> {
    let it = x
        .iterate(heap)
        .map_err(|_| anyhow::anyhow!("for {}, got {}, want sequence", what, x.get_type()))?;
    let mut out = Vec::new();
    for (i, v) in it.enumerate() {
        match v.downcast_ref::<HeaderInfo>() {
            Some(h) => out.push(h.0.clone()),
            None => {
                return Err(anyhow::anyhow!(
                    "at index {} of {}, got element of type {}, want HeaderInfo",
                    i,
                    what,
                    v.get_type()
                ));
            }
        }
    }
    Ok(out)
}

#[starlark_module]
fn cc_internal_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:111 CcStarlarkInternal.checkPrivateApi
    fn check_private_api<'v>(
        this: Value<'v>,
        #[starlark(require = named)] allowlist: Value<'v>,
        #[starlark(require = named, default = 1)] depth: i32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let _ = this;
        // This method may be called anywhere from builtins, but not outside (because it's not
        // exposed in cc_common.bzl).
        let Some(bazel_module_context) =
            module_of_innermost_enclosing_starlark_function(eval, depth.max(0) as usize)
        else {
            // The module is null when the call is coming from one of the callbacks passed to
            // execution phase.
            return Ok(NoneType);
        };
        let mut entries: Vec<(String, String)> = Vec::new();
        for item in allowlist
            .iterate(eval.heap())
            .map_err(|e| anyhow::anyhow!("{}", e))?
        {
            let tuple = starlark::values::tuple::TupleRef::from_value(item)
                .ok_or_else(|| anyhow::anyhow!("allowlist: expected a sequence of tuples"))?;
            let get = |i: usize| -> anyhow::Result<String> {
                tuple
                    .content()
                    .get(i)
                    .and_then(|v| v.unpack_str())
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("ClassCastException in allowlist entry"))
            };
            entries.push((get(0)?, get(1)?));
        }
        // Java: BuiltinRestriction.failIfModuleOutsideAllowlist -> failIfLabelOutsideAllowlist.
        let label = &bazel_module_context.label;
        if label.get_repository() == &*blaze_cmdline::repository_name::BUILTINS {
            return Ok(NoneType);
        }
        if !entries.iter().any(|(repo, pkg)| {
            allowlist_entry_allows(repo, pkg, label, &bazel_module_context.repo_mapping)
        }) {
            return Err(anyhow::anyhow!(
                "file '{}' cannot use private API",
                label.get_canonical_form()
            ));
        }
        Ok(NoneType)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:954 CcStarlarkInternal.freeze
    fn freeze<'v>(this: Value<'v>, value: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:954 CcStarlarkInternal.freeze
        // Dict.immutableCopyOf(dict): the dict itself when it is immutable (a key-tracking
        // `ctx.var` keeps its tracking), else an immutable copy of its entries.
        if blaze_starlark::dict::dict_is_immutable(value).is_some() {
            return blaze_starlark::dict::dict_immutable_copy_of(heap, value)
                .map_err(|e| anyhow::anyhow!("{e}"));
        }
        // PORT: Java returns an immutable list; starlark-rust has no immutable list on a mutable
        // heap, so this returns a copy (the module freezes it when it is done).
        if value.get_type() != "string"
            && let Ok(it) = value.iterate(heap)
        {
            let items: Vec<Value<'v>> = it.collect();
            return Ok(heap.alloc(AllocList(items)));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1196 CcStarlarkInternal.createHeaderInfo
    fn create_header_info<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneType)] header_module: Value<'v>,
        #[starlark(require = named, default = NoneType)] pic_header_module: Value<'v>,
        #[starlark(require = named)] modular_public_headers: Option<Value<'v>>,
        #[starlark(require = named)] modular_private_headers: Option<Value<'v>>,
        #[starlark(require = named)] textual_headers: Option<Value<'v>>,
        #[starlark(require = named)] separate_module_headers: Option<Value<'v>>,
        #[starlark(require = named, default = NoneType)] separate_module: Value<'v>,
        #[starlark(require = named, default = NoneType)] separate_pic_module: Value<'v>,
        heap: Heap<'v>,
    ) -> anyhow::Result<HeaderInfo> {
        let _ = this;
        null_if_none_artifact(header_module, "header_module")?;
        null_if_none_artifact(pic_header_module, "pic_header_module")?;
        for (v, what) in [
            (modular_public_headers, "modular_public_headers"),
            (modular_private_headers, "modular_private_headers"),
            (textual_headers, "textual_headers"),
            (separate_module_headers, "separate_module_headers"),
        ] {
            if let Some(v) = v {
                cast_empty_artifact_sequence(v, what, heap)?;
            }
        }
        null_if_none_artifact(separate_module, "separate_module")?;
        null_if_none_artifact(separate_pic_module, "separate_pic_module")?;
        Ok(HeaderInfo(Arc::new(HeaderInfoData {
            deps: Vec::new(),
            merged_deps: Vec::new(),
        })))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:1234 CcStarlarkInternal.createHeaderInfoWithDeps
    fn create_header_info_with_deps<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneType)] header_info: Value<'v>,
        #[starlark(require = named)] deps: Option<Value<'v>>,
        #[starlark(require = named)] merged_deps: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> anyhow::Result<HeaderInfo> {
        let _ = this;
        if header_info.downcast_ref::<HeaderInfo>().is_none() {
            return Err(anyhow::anyhow!(
                "in call to create_header_info_with_deps(), parameter 'header_info' got value \
                 of type '{}', want 'HeaderInfo'",
                header_info.get_type()
            ));
        }
        let deps = match deps {
            Some(d) => cast_header_infos(d, "deps", heap)?,
            None => Vec::new(),
        };
        let merged_deps = match merged_deps {
            Some(d) => cast_header_infos(d, "merged_deps", heap)?,
            None => Vec::new(),
        };
        Ok(HeaderInfo(Arc::new(HeaderInfoData { deps, merged_deps })))
    }
}
