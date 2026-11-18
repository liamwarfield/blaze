//! `module_ctx`: the object passed to the implementation function of module extensions.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java (a subclass of
//! `StarlarkBaseExternalContext`). The base context is `crate::starlark::base_external_context`;
//! the Starlark handle is `crate::starlark::context::ModuleCtx`, whose methods are the base
//! methods plus [`module_ctx_methods`].
//!
//! PORT: starlark-rust attributes (Java `structField = true` methods) cannot reach the thread's
//! payload, so the values behind `modules` and `facts` live in [`ModuleCtxFields`], held by the
//! `ModuleCtx` handle (field `fields`). They are allocated once on a frozen heap, like Java's
//! immutable objects.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use blaze_bzlmod_model::ModuleExtensionId;
use blaze_cmdline::RepositoryName;
use blaze_repo_fetch::downloader::download_manager::DownloadManager;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::Path;
use indexmap::IndexMap;
use starlark::environment::MethodsBuilder;
use starlark::starlark_module;
use starlark::values::FrozenHeapRef;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneType;

use super::facts::cast_string_keyed_dict;
use super::module_extension_metadata::ModuleExtensionMetadata;
use super::runnable_extension::ModuleExtensionEvaluationProgress;
use super::starlark_bazel_module::TypeCheckedTag;
use crate::env_handle::SkyEnv;
use crate::repository::repo_recorded_input::wrap_env_vars;
use crate::starlark::base_external_context::StarlarkBaseExternalContext;
use crate::starlark::context::ModuleCtx;

fn eval_error(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The values behind the struct fields of `module_ctx`, shared by the `ModuleCtx` handle.
pub struct ModuleCtxFields {
    /// Keeps `modules` and `facts` (and everything they reference) alive.
    pub frozen_heap: FrozenHeapRef,
    /// A frozen list of `bazel_module` values (Java `StarlarkList<StarlarkBazelModule>`).
    pub modules: FrozenValue,
    /// The `Facts` value.
    pub facts: FrozenValue,
    pub root_module_has_non_dev_dependency: bool,
    /// `extensionId.isolationKey().isPresent()`.
    pub is_isolated: bool,
    /// `--experimental_isolated_extension_usages` (enables `is_isolated`).
    pub experimental_isolated_extension_usages: bool,
}

impl fmt::Debug for ModuleCtxFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleCtxFields")
            .field(
                "root_module_has_non_dev_dependency",
                &self.root_module_has_non_dev_dependency,
            )
            .field("is_isolated", &self.is_isolated)
            .finish_non_exhaustive()
    }
}

/// The `module_ctx` state in the thread payload (`ExternalContextKind::ModuleExtension`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:51 ModuleExtensionContext
pub struct ModuleExtensionContext {
    extension_id: ModuleExtensionId,
    fields: Arc<ModuleCtxFields>,
}

impl ModuleExtensionContext {
    /// Java's constructor: creates the base context and this one, then records the inputs to
    /// the extension that are known prior to evaluation.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:57 ModuleExtensionContext.<init>
    pub fn create(
        working_directory: Path,
        directories: LoadingDirectories,
        env: SkyEnv,
        repo_env: Arc<IndexMap<String, String>>,
        nonstrict_repo_env: Arc<IndexMap<String, String>>,
        download_manager: Option<Arc<DownloadManager>>,
        timeout_scaling: f64,
        process_wrapper: Option<Path>,
        starlark_semantics: Arc<StarlarkSemantics>,
        extension_id: ModuleExtensionId,
        fields: Arc<ModuleCtxFields>,
        static_env_vars: &BTreeMap<String, Option<String>>,
        static_repo_mapping_entries: &[(RepositoryName, String, RepositoryName)],
    ) -> (StarlarkBaseExternalContext, ModuleExtensionContext) {
        // PORT: RepositoryRemoteExecutor is not ported (remote execution of repository rules).
        let mut base = StarlarkBaseExternalContext::new(
            working_directory,
            directories,
            env,
            repo_env,
            nonstrict_repo_env,
            download_manager,
            timeout_scaling,
            process_wrapper,
            starlark_semantics,
            ModuleExtensionEvaluationProgress::module_extension_evaluation_context_string(
                &extension_id,
            ),
            /* allow_watching_paths_outside_workspace= */ false,
        );
        // Record inputs to the extension that are known prior to evaluation.
        for (input, value) in wrap_env_vars(static_env_vars) {
            base.record_input_with_value(input, value);
        }
        // repoMappingRecorder.record(staticRepoMappingEntries)
        for (from_repo, apparent_name, canonical_name) in static_repo_mapping_entries {
            base.record_repo_mapping(from_repo, apparent_name, canonical_name);
        }
        (
            base,
            ModuleExtensionContext {
                extension_id,
                fields,
            },
        )
    }

    pub fn extension_id(&self) -> &ModuleExtensionId {
        &self.extension_id
    }

    pub fn fields(&self) -> &Arc<ModuleCtxFields> {
        &self.fields
    }

    /// The contents of the working directory are purely ephemeral, only the repos instantiated
    /// by the extension are considered its results.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:98 ModuleExtensionContext.shouldDeleteWorkingDirectoryOnClose
    pub fn should_delete_working_directory_on_close(&self, _successful: bool) -> bool {
        true
    }

    /// Maybe we can some day support remote execution, but not today.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:105 ModuleExtensionContext.isRemotable
    pub fn is_remotable(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:111 ModuleExtensionContext.getRemoteExecProperties
    pub fn get_remote_exec_properties(&self) -> anyhow::Result<IndexMap<String, String>> {
        Ok(IndexMap::new())
    }
}

/// The `tag` parameter (`allowedTypes = {@ParamType(type = TypeCheckedTag.class)}`).
// Java: net/starlark/java/eval/BuiltinFunction.java:405 BuiltinFunction.getArgumentVector
fn unpack_tag<'v>(function: &str, tag: Value<'v>) -> starlark::Result<&'v TypeCheckedTag> {
    tag.downcast_ref::<TypeCheckedTag>().ok_or_else(|| {
        eval_error(format!(
            "in call to {function}(), parameter 'tag' got value of type '{}', want \
             'bazel_module_tag'",
            tag.get_type()
        ))
    })
}

/// The `module_ctx` methods of Java `ModuleExtensionContext` (the base methods are
/// `base_external_context_methods`).
#[starlark_module]
pub fn module_ctx_methods(builder: &mut MethodsBuilder) {
    /// A list of all the Bazel modules in the external dependency graph that use this module
    /// extension, each of which is a `bazel_module` object that exposes all the tags it specified
    /// for this extension. The iteration order of this dictionary is guaranteed to be the same as
    /// breadth-first search starting from the root module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:124 ModuleExtensionContext.getModules
    #[starlark(attribute)]
    fn modules<'v>(this: &ModuleCtx, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        heap.add_reference(&this.fields.frozen_heap);
        Ok(this.fields.modules.to_value())
    }

    /// The JSON-like dict returned by a previous execution of this extension in the `facts`
    /// parameter of `extension_metadata` or else `{}`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:141 ModuleExtensionContext.getFacts
    #[starlark(attribute)]
    fn facts<'v>(this: &ModuleCtx, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        heap.add_reference(&this.fields.frozen_heap);
        Ok(this.fields.facts.to_value())
    }

    /// Returns whether the given tag was specified on the result of a `use_extension` call with
    /// `devDependency = True`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:159 ModuleExtensionContext.isDevDependency
    fn is_dev_dependency<'v>(
        this: &ModuleCtx,
        #[starlark(require = pos)] tag: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = this;
        Ok(unpack_tag("is_dev_dependency", tag)?.is_dev_dependency())
    }

    /// Returns an opaque `sort_key` object for the given tag, which can be compared with other
    /// `sort_key` objects to determine the relative order of tags, even across tag classes.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:182 ModuleExtensionContext.getTagSortKey
    fn tag_sort_key<'v>(
        this: &ModuleCtx,
        #[starlark(require = pos)] tag: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        Ok(unpack_tag("tag_sort_key", tag)?.get_sort_key())
    }

    /// Whether the root module uses this extension as a non-dev dependency.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:203 ModuleExtensionContext.rootModuleHasNonDevDependency
    #[starlark(attribute)]
    fn root_module_has_non_dev_dependency(this: &ModuleCtx) -> starlark::Result<bool> {
        Ok(this.fields.root_module_has_non_dev_dependency)
    }

    /// Constructs an opaque object that can be returned from the module extension's
    /// implementation function to provide metadata about the repositories generated by the
    /// extension to Bazel.
    ///
    /// PORT: Java's parameter type checks (`sequence of strings`, `string` or `None`; `bool`;
    /// `dict`) give starlark-rust's or the casts' messages here.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:310 ModuleExtensionContext.extensionMetadata
    fn extension_metadata<'v>(
        this: &ModuleCtx,
        #[starlark(require = named, default = NoneType)] root_module_direct_deps: Value<'v>,
        #[starlark(require = named, default = NoneType)] root_module_direct_dev_deps: Value<'v>,
        #[starlark(require = named, default = false)] reproducible: bool,
        #[starlark(require = named)] facts: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<ModuleExtensionMetadata> {
        let _ = this;
        // defaultValue = "{}"
        let facts = facts.unwrap_or_else(|| {
            heap.alloc(starlark::values::dict::AllocDict(Vec::<(
                Value<'v>,
                Value<'v>,
            )>::new()))
        });
        cast_string_keyed_dict(facts, "facts").map_err(eval_error)?;
        ModuleExtensionMetadata::create(
            root_module_direct_deps,
            root_module_direct_dev_deps,
            reproducible,
            facts,
        )
        .map_err(eval_error)
    }
}

/// `module_ctx.is_isolated`: whether this particular usage of the extension had `isolate = True`
/// specified and is thus isolated from all other usages. Only available with
/// `--experimental_isolated_extension_usages` (`enableOnlyWithFlag`): without the flag the field
/// does not exist, so `getattr(module_ctx, "is_isolated", default)` gives the default and
/// `hasattr` is false. Served by `ModuleCtx`'s `get_attr`/`has_attr`/`dir_attr`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:195 ModuleExtensionContext.isIsolated
pub fn is_isolated(fields: &ModuleCtxFields) -> Option<bool> {
    fields
        .experimental_isolated_extension_usages
        .then_some(fields.is_isolated)
}
