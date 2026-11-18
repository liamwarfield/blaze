//! `_builtins.internal.py_builtins`: ports of `rules/python/PyBuiltins.java` (the Starlark
//! methods, and its `CopyWithoutCachingAction`) and `bazel/rules/python/BazelPyBuiltins.java`
//! (the Bazel subclass with its `__init__.py` empty-files supplier).
//!
//! At load time the value behaves like W3's stand-in (`blaze_packages::builtins::stubs::py_builtins`):
//! `@_builtins//:common/python/py_internal.bzl` only reads the object into a module global; every
//! method runs at analysis time, inside a rule implementation.

use std::any::Any;
use std::cell::Ref;
use std::fmt;
use std::sync::{Arc, LazyLock};

use allocative::Allocative;
use blaze_actions::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::actions::fingerprint_util::FingerprintPathExt;
use blaze_actions::actions::path_mapper::FileRoot;
use blaze_actions::analysis::actions::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, NewDeterministicWriterError,
};
use blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingManifestAction;
use blaze_actions::analysis::runfiles::{EmptyFilesSupplierRef, Runfiles, RunfilesBuilder};
use blaze_actions::analysis::source_manifest_action::{ManifestType, SourceManifestAction};
use blaze_actions::analysis::starlark::starlark_file::{
    StarlarkFile, StarlarkFileRoot, artifact_from_value,
};
use blaze_actions::{ActionRef, Artifact};
use blaze_analysis::configuredtargets::{
    get_files_to_run_provider, get_provider, target_from_starlark,
};
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::config_util;
use blaze_analysis::starlark::starlark_rule_context::rule_context_from_ctx;
use blaze_analysis::starlark::starlark_runfiles::{runfiles_from_value, runfiles_to_value};
use blaze_analysis::starlark::thread_payload::register_action;
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::list::ListRef;
use starlark::values::none::NoneType;
use starlark::values::tuple::TupleRef;
use starlark::values::{
    NoSerialize, OwnedFrozenValue, StarlarkValue, Value, ValueLike, starlark_value,
};

use super::python_utils::PythonUtilsGetInitPyFiles;

// -------------------------------------------------------------------------------------------
// BazelPyBuiltins

/// `UUID.fromString("0211a192-1b1e-40e6-80e9-7352360b12b1")` as (most, least) significant bits.
// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPyBuiltins.java:26 BazelPyBuiltins.GUID
const BAZEL_PY_BUILTINS_GUID: (i64, i64) =
    (0x0211a1921b1e40e6_u64 as i64, 0x80e97352360b12b1_u64 as i64);

/// The supplier Bazel's `py_builtins` installs (a singleton: `Runfiles.Builder.merge` compares
/// suppliers by identity).
// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPyBuiltins.java:29 BazelPyBuiltins.GET_INIT_PY_FILES
pub static GET_INIT_PY_FILES: LazyLock<EmptyFilesSupplierRef> = LazyLock::new(|| {
    Arc::new(PythonUtilsGetInitPyFiles::new(
        |_source| false,
        BAZEL_PY_BUILTINS_GUID,
    ))
});

/// `new BazelPyBuiltins()`.
// Java: com/google/devtools/build/lib/bazel/rules/python/BazelPyBuiltins.java:32 BazelPyBuiltins.<init>
pub fn bazel_py_builtins() -> PyBuiltins {
    PyBuiltins::new(GET_INIT_PY_FILES.clone())
}

/// `_builtins.internal.py_builtins` (Java `BazelPyBuiltins`), frozen, for the analysis-capable
/// builtins environment (`blaze_rules::bazel_environment`).
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:310 BazelRuleClassProvider.PYTHON_RULES
pub fn py_builtins_value() -> OwnedFrozenValue {
    blaze_packages::builtins::env::freeze_value(|m| m.heap().alloc(bazel_py_builtins()))
}

// -------------------------------------------------------------------------------------------
// PyBuiltins

/// Bridge to allow builtins bzl code to call Java code.
// Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:57 PyBuiltins
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct PyBuiltins {
    #[allocative(skip)]
    empty_files_supplier: EmptyFilesSupplierRef,
}

starlark_simple_value!(PyBuiltins);

// Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:58 PyBuiltins.NAME
pub const NAME: &str = "py_builtins";

/// Java's default `StarlarkValue.repr` of the Bazel subclass.
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
const PY_BUILTINS_REPR: &str =
    "<unknown object com.google.devtools.build.lib.bazel.rules.python.BazelPyBuiltins>";

impl PyBuiltins {
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:62 PyBuiltins.<init>
    pub fn new(empty_files_supplier: EmptyFilesSupplierRef) -> PyBuiltins {
        PyBuiltins {
            empty_files_supplier,
        }
    }
}

impl fmt::Debug for PyBuiltins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PyBuiltins")
    }
}

impl fmt::Display for PyBuiltins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PY_BUILTINS_REPR)
    }
}

#[starlark_value(type = "py_builtins")]
impl<'v> StarlarkValue<'v> for PyBuiltins {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("py_builtins", py_builtins_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(PY_BUILTINS_REPR);
    }
}

// -------------------------------------------------------------------------------------------
// Parameter conversion (the checks Starlark's method calling performs for typed Java
// parameters).

/// Java `Starlark.errorf(...)`.
fn errorf(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The error of a Java-typed parameter that got a value of another type
/// (`net/starlark/java/eval/BuiltinFunction.java` `checkParamValue`).
// Java: net/starlark/java/eval/BuiltinFunction.java:330 BuiltinFunction.getArgumentVector
pub(crate) fn param_type_error(method: &str, param: &str, v: Value, want: &str) -> starlark::Error {
    errorf(format!(
        "in call to {method}(), parameter '{param}' got value of type '{}', want '{want}'",
        v.get_type()
    ))
}

/// A `StarlarkRuleContext` parameter: the rule context behind `ctx` (Java
/// `starlarkCtx.getRuleContext()`).
pub(crate) fn ctx_param<'v>(
    method: &str,
    ctx: Value<'v>,
) -> starlark::Result<Ref<'v, RuleContext>> {
    if ctx.get_type() != "ctx" {
        return Err(param_type_error(method, "ctx", ctx, "ctx"));
    }
    rule_context_from_ctx(ctx)
}

/// A `Runfiles` parameter.
fn runfiles_param(method: &str, param: &str, v: Value) -> starlark::Result<Runfiles> {
    runfiles_from_value(v).ok_or_else(|| param_type_error(method, param, v, "runfiles"))
}

/// An `Artifact` parameter.
fn artifact_param(method: &str, param: &str, v: Value) -> starlark::Result<Artifact> {
    artifact_from_value(v).ok_or_else(|| param_type_error(method, param, v, "File"))
}

/// `Sequence.cast(targets, TransitiveInfoCollection.class, "targets")` after the `Sequence`
/// parameter check.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
fn targets_param(
    method: &str,
    targets: Value,
) -> starlark::Result<Vec<blaze_analysis::configured_target::ConfiguredTarget>> {
    let elems: Vec<Value> = if let Some(l) = ListRef::from_value(targets) {
        l.content().to_vec()
    } else if let Some(t) = TupleRef::from_value(targets) {
        t.content().to_vec()
    } else {
        return Err(param_type_error(method, "targets", targets, "sequence"));
    };
    let mut result = Vec::with_capacity(elems.len());
    for (i, elem) in elems.into_iter().enumerate() {
        match target_from_starlark(elem) {
            Some(t) => result.push(t),
            None => {
                return Err(errorf(format!(
                    "at index {i} of targets, got element of type {}, want Target",
                    elem.get_type()
                )));
            }
        }
    }
    Ok(result)
}

// -------------------------------------------------------------------------------------------
// Methods

#[starlark_module]
fn py_builtins_methods(builder: &mut MethodsBuilder) {
    /// Tells if bzlmod is enabled
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:72 PyBuiltins.isBzlmodEnabled
    fn is_bzlmod_enabled<'v>(this: &PyBuiltins, ctx: Value<'v>) -> starlark::Result<bool> {
        let _ = this;
        if ctx.get_type() != "ctx" {
            return Err(param_type_error("is_bzlmod_enabled", "ctx", ctx, "ctx"));
        }
        Ok(true)
    }

    /// Efficiently checks if the depset is a singleton.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:87 PyBuiltins.isSingletonDepset
    fn is_singleton_depset<'v>(
        this: &PyBuiltins,
        #[starlark(require = pos)] value: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = this;
        let Some(depset) = Depset::from_value(value) else {
            return Err(param_type_error(
                "is_singleton_depset",
                "value",
                value,
                "depset",
            ));
        };
        // Java `depset.getSet().isSingleton()`: the set's children are a single element.
        // PORT: blaze_starlark's depset does not expose its node shape; rebuilding the NestedSet
        // keeps the same structure (singleton nodes stay singletons), so `is_singleton` of the
        // rebuilt set answers the same question. The element mapping only needs to be total.
        let mut index = 0usize;
        let set = depset.to_nested_set(&mut |_| {
            index += 1;
            Ok(index)
        })?;
        Ok(set.is_singleton())
    }

    /// Return true if subject matches pattern; pattern is implicitly anchored with ^ and $
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:98 PyBuiltins.regexMatch
    fn regex_match<'v>(
        this: &PyBuiltins,
        #[starlark(require = pos)] subject: &str,
        #[starlark(require = pos)] pattern: &str,
    ) -> starlark::Result<bool> {
        let _ = this;
        // Java `subject.matches(pattern)` (java.util.regex, whole-input match).
        // PORT: the regex crate implements the common subset of java.util.regex; a pattern it
        // cannot compile is not ported (Java would either accept it or crash with a
        // PatternSyntaxException).
        match regex::Regex::new(&format!("^(?:{pattern})$")) {
            Ok(re) => Ok(re.is_match(subject)),
            Err(e) => blaze_util::unported!(
                "String.matches",
                "pattern {pattern:?} is not supported by the regex crate: {e}"
            ),
        }
    }

    /// Get the name of the rule for the given ctx
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:108 PyBuiltins.getRuleName
    fn get_rule_name<'v>(this: &PyBuiltins, ctx: Value<'v>) -> starlark::Result<String> {
        let _ = this;
        let rule_context = ctx_param("get_rule_name", ctx)?;
        Ok(rule_context.get_rule().get_rule_class().to_owned())
    }

    /// Get the name of the OS Bazel itself is running on.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:116 PyBuiltins.getCurrentOsName
    fn get_current_os_name(this: &PyBuiltins) -> starlark::Result<String> {
        let _ = this;
        Ok(blaze_shell::util::os::OS::get_current()
            .get_canonical_name()
            .to_owned())
    }

    /// Given a label, return a runfiles path that includes the repository directory
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:126 PyBuiltins.getLabelRepoRunfilesPath
    fn get_label_repo_runfiles_path<'v>(
        this: &PyBuiltins,
        label: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = this;
        let Some(label) = label.downcast_ref::<LabelValue>() else {
            return Err(param_type_error(
                "get_label_repo_runfiles_path",
                "label",
                label,
                "Label",
            ));
        };
        Ok(label
            .0
            .get_package_identifier()
            .get_runfiles_path()
            .get_path_string()
            .to_owned())
    }

    // TODO(bazel-team): Remove this once rules are switched to using execpath semanatics for the
    // $(location) function. See https://github.com/bazelbuild/bazel/issues/15294
    /// Expands $(location) and makevar references. Note that $(location) performs rootpath
    /// (runfiles-relative) expansion, not execpath expansion.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:162 PyBuiltins.expandLocationAndMakeVariables
    fn expand_location_and_make_variables<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] attribute_name: &str,
        #[starlark(require = named)] expression: &str,
        #[starlark(require = named)] targets: Value<'v>,
    ) -> starlark::Result<String> {
        let _ = this;
        const METHOD: &str = "expand_location_and_make_variables";
        let rule_context = ctx_param(METHOD, ctx)?;
        let targets = targets_param(METHOD, targets)?;

        let mut builder: IndexMap<Label, Vec<Artifact>> = IndexMap::new();
        for current in &targets {
            // This logic is basically a copy of how LocationExpander.java treats the data
            // attribute.
            let files_to_run = get_files_to_run_provider(current);
            let files_to_build = get_provider::<FileProvider>(current)
                .expect("NullPointerException: target without FileProvider")
                .get_files_to_build();
            let artifacts: Vec<Artifact> = match files_to_run {
                None => files_to_build.to_list().to_vec(),
                Some(files_to_run) => match files_to_run.get_executable() {
                    None => files_to_build.to_list().to_vec(),
                    Some(executable) => vec![executable.clone()],
                },
            };
            let label = AliasProvider::get_dependency_label(current);
            // Java `ImmutableMap.Builder.buildOrThrow` rejects duplicate keys.
            if builder.contains_key(&label) {
                panic!("IllegalArgumentException: Multiple entries with same key: {label}");
            }
            builder.insert(label, artifacts);
        }

        Ok(rule_context
            .get_expander_with_label_map(builder)
            .with_data_locations() // Enables $(location) expansion.
            .expand(Some(attribute_name), expression))
    }

    /// Write a repo_mapping file for the given runfiles
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:202 PyBuiltins.repoMappingAction
    fn create_repo_mapping_manifest<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] runfiles: Value<'v>,
        #[starlark(require = named)] output: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        const METHOD: &str = "create_repo_mapping_manifest";
        let rule_context = ctx_param(METHOD, ctx)?;
        let runfiles = runfiles_param(METHOD, "runfiles", runfiles)?;
        let repo_mapping_manifest = artifact_param(METHOD, "output", output)?;
        let action: ActionRef = Arc::new(RepoMappingManifestAction::new(
            rule_context.get_action_owner(),
            repo_mapping_manifest,
            rule_context
                .get_transitive_packages_for_runfile_repo_mapping_manifest()
                .expect(
                    "NullPointerException: transitive packages for the runfiles repo mapping \
                     manifest are not tracked",
                )
                .clone(),
            runfiles.get_artifacts().clone(),
            runfiles.get_symlinks(),
            runfiles.get_root_symlinks().clone(),
            rule_context.get_workspace_name().to_owned(),
            config_util::core_options(rule_context.get_configuration()).compact_repo_mapping,
        ));
        drop(rule_context);
        // Java: ruleContext.getAnalysisEnvironment().registerAction(...)
        register_action(eval, action)?;
        Ok(NoneType)
    }

    /// Create a runfiles that generates missing __init__.py files using Java and the internal
    /// EmptyFilesProvider interface
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:237 PyBuiltins.mergeRunfilesWithGeneratedInitsEmptyFilesSupplier
    fn merge_runfiles_with_generated_inits_empty_files_supplier<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] runfiles: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        const METHOD: &str = "merge_runfiles_with_generated_inits_empty_files_supplier";
        let rule_context = ctx_param(METHOD, ctx)?;
        let runfiles = runfiles_param(METHOD, "runfiles", runfiles)?;
        if runfiles.is_empty() {
            // The Runfiles merge functions have an optimization to detect an empty runfiles and
            // return a singleton. Unfortunately, this optimization considers an empty runfiles
            // with a emptyFilesSupplier as empty, so then drops it when returning the singleton.
            // To work around this, require that there is *something* in the runfiles.
            return Err(errorf("input runfiles cannot be empty"));
        }
        // Java: starlarkCtx.getWorkspaceName() is ruleContext.getWorkspaceName().
        let merged = RunfilesBuilder::new(rule_context.get_workspace_name())
            .set_empty_files_supplier(this.empty_files_supplier.clone())
            .merge(&runfiles)
            .build();
        drop(rule_context);
        Ok(runfiles_to_value(eval.heap(), merged))
    }

    /// Declare a file that always reports it is unchanged.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:260 PyBuiltins.declareConstantMetadataFile
    fn declare_constant_metadata_file<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] name: &str,
        #[starlark(require = named)] root: Value<'v>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = this;
        let rule_context = ctx_param("declare_constant_metadata_file", ctx)?;
        // Java: (ArtifactRoot) rootUnchecked
        let root = match root.downcast_ref::<StarlarkFileRoot>() {
            Some(StarlarkFileRoot(FileRoot::Artifact(root))) => root.clone(),
            _ => panic!(
                "ClassCastException: class {} cannot be cast to class \
                 com.google.devtools.build.lib.actions.ArtifactRoot",
                root.get_type()
            ),
        };
        let artifact = rule_context
            .get_analysis_environment()
            .get_constant_metadata_artifact(
                &rule_context
                    .get_package_directory()
                    .get_relative(&PathFragment::create(name)),
                &root,
            );
        Ok(StarlarkFile::new(artifact))
    }

    /// Create a manifest of the files in runfiles
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:278 PyBuiltins.createRunfilesManifest
    fn create_sources_only_manifest<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] runfiles: Value<'v>,
        #[starlark(require = named)] output: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        const METHOD: &str = "create_sources_only_manifest";
        let rule_context = ctx_param(METHOD, ctx)?;
        let runfiles = runfiles_param(METHOD, "runfiles", runfiles)?;
        let output = artifact_param(METHOD, "output", output)?;
        let action: ActionRef = Arc::new(SourceManifestAction::new(
            Arc::new(ManifestType::SOURCES_ONLY),
            rule_context.get_action_owner(),
            output,
            runfiles,
            /* repo_mapping_manifest= */ None,
            // Java: ruleContext.getConfiguration().remotableSourceManifestActions()
            config_util::core_options(rule_context.get_configuration())
                .remotable_source_manifest_actions,
        ));
        drop(rule_context);
        // Java: ruleContext.getAnalysisEnvironment().registerAction(...)
        register_action(eval, action)?;
        Ok(NoneType)
    }

    /// Copy one file to another, but with action caching disabled.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:301 PyBuiltins.copyWithoutCaching
    fn copy_without_caching<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] read_from: Value<'v>,
        #[starlark(require = named)] write_to: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        const METHOD: &str = "copy_without_caching";
        let rule_context = ctx_param(METHOD, ctx)?;
        let read_from = artifact_param(METHOD, "read_from", read_from)?;
        let write_to = artifact_param(METHOD, "write_to", write_to)?;
        let action: ActionRef = Arc::new(CopyWithoutCachingAction::new(
            rule_context.get_action_owner(),
            read_from,
            write_to,
        ));
        drop(rule_context);
        // Java: ruleContext.registerAction(...)
        register_action(eval, action)?;
        Ok(NoneType)
    }

    // TODO(b/253059598): Remove support for this; https://github.com/bazelbuild/bazel/issues/16455
    /// Tells if any action listeners are enabled. This is to prevent registering extra actions
    /// unless necessary
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:387 PyBuiltins.areActionListenersEnabled
    fn are_action_listeners_enabled<'v>(
        this: &PyBuiltins,
        #[starlark(require = pos)] ctx: Value<'v>,
    ) -> starlark::Result<bool> {
        let _ = this;
        let rule_context = ctx_param("are_action_listeners_enabled", ctx)?;
        // Java: BuildConfigurationValue.getActionListeners() is options.actionListeners.
        Ok(!config_util::core_options(rule_context.get_configuration())
            .action_listeners
            .is_empty())
    }

    // TODO(b/253059598): Remove support for this; https://github.com/bazelbuild/bazel/issues/16455
    /// Adds an extra pseudo action for (deprecated) extra actions support
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:409 PyBuiltins.addPyExtraActionPseudoAction
    fn add_py_extra_pseudo_action<'v>(
        this: &PyBuiltins,
        #[starlark(require = named)] ctx: Value<'v>,
        #[starlark(require = named)] dependency_transitive_python_sources: Value<'v>,
    ) -> starlark::Result<NoneType> {
        let _ = this;
        const METHOD: &str = "add_py_extra_pseudo_action";
        let _rule_context = ctx_param(METHOD, ctx)?;
        let Some(depset) = Depset::from_value(dependency_transitive_python_sources) else {
            return Err(param_type_error(
                METHOD,
                "dependency_transitive_python_sources",
                dependency_transitive_python_sources,
                "depset",
            ));
        };
        // Java: Depset.cast(..., Artifact.class, "dependency_transitive_python_sources")
        // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:262 Depset.cast
        let element_type = depset.element_type();
        if element_type != "empty" && element_type != "File" {
            return Err(errorf(format!(
                "for 'dependency_transitive_python_sources', got a depset of '{element_type}', \
                 expected a depset of 'File'"
            )));
        }
        // Java: PyCommon.registerPyExtraActionPseudoAction builds a PyPseudoAction (extra
        // actions), which is not ported.
        blaze_util::unported!("PyCommon.registerPyExtraActionPseudoAction")
    }
}

// -------------------------------------------------------------------------------------------
// CopyWithoutCachingAction

// Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:311 CopyWithoutCachingAction.GUID
const COPY_WITHOUT_CACHING_GUID: &str = "67513fa7-3824-493b-aeab-95a8b778ea07";

/// Copies one file to another, with action caching disabled.
// Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:310 PyBuiltins.CopyWithoutCachingAction
pub struct CopyWithoutCachingAction {
    base: AbstractAction,
}

impl CopyWithoutCachingAction {
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:313 CopyWithoutCachingAction.<init>
    pub fn new(owner: ActionOwner, read_from: Artifact, write_to: Artifact) -> Self {
        CopyWithoutCachingAction {
            base: abstract_file_write_action::new_abstract_file_write_action(
                owner,
                NestedSetBuilder::create(Order::STABLE_ORDER, [read_from]),
                write_to,
            ),
        }
    }

    fn primary_input(&self) -> Artifact {
        self.get_primary_input()
            .expect("CopyWithoutCachingAction has one input")
    }
}

impl fmt::Debug for CopyWithoutCachingAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CopyWithoutCachingAction")
            .field("base", &self.base)
            .finish()
    }
}

impl Action for CopyWithoutCachingAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:368 CopyWithoutCachingAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "PyCopyWithoutCaching".to_string()
    }

    // NOTE: This method is effectively unused because executeUnconditionally=true.
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:334 CopyWithoutCachingAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(COPY_WITHOUT_CACHING_GUID);
        fp.add_path(&self.primary_input().get_path());
        fp.add_path(&self.get_primary_output().get_path());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:344 CopyWithoutCachingAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "executeUnconditionally: {}\nGUID: {}\nreadFrom: {}\nwriteTo: {}\n",
            self.execute_unconditionally(),
            COPY_WITHOUT_CACHING_GUID,
            self.primary_input().get_exec_path_string(),
            self.get_primary_output().get_exec_path_string()
        ))
    }

    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:323 CopyWithoutCachingAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:328 CopyWithoutCachingAction.isVolatile
    fn is_volatile(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:361 CopyWithoutCachingAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!(
            "Copying {} to {} (uncachable action)",
            self.primary_input().get_exec_path_string(),
            self.get_primary_output().get_exec_path_string()
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }
}

impl AbstractFileWriteAction for CopyWithoutCachingAction {
    // Java: com/google/devtools/build/lib/rules/python/PyBuiltins.java:318 CopyWithoutCachingAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        // Java: out -> ctx.getInputPath(getPrimaryInput()).getInputStream().transferTo(out);
        // needs the ActionExecutionContext (execution).
        blaze_util::unported!("CopyWithoutCachingAction.newDeterministicWriter")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bazel_guid_matches_uuid_string() {
        // UUID.fromString("0211a192-1b1e-40e6-80e9-7352360b12b1")
        assert_eq!(
            format!(
                "{:016x}{:016x}",
                BAZEL_PY_BUILTINS_GUID.0 as u64, BAZEL_PY_BUILTINS_GUID.1 as u64
            ),
            "0211a1921b1e40e680e97352360b12b1"
        );
    }

    #[test]
    fn get_init_py_files_is_a_singleton() {
        assert!(Arc::ptr_eq(&GET_INIT_PY_FILES, &GET_INIT_PY_FILES.clone()));
        let a = bazel_py_builtins();
        let b = bazel_py_builtins();
        assert!(Arc::ptr_eq(
            &a.empty_files_supplier,
            &b.empty_files_supplier
        ));
    }

    #[test]
    fn repr_is_java_default() {
        assert_eq!(bazel_py_builtins().to_string(), PY_BUILTINS_REPR);
    }

    // PythonStarlarkApiTest / PyBinaryConfiguredTargetTest exercise py_builtins through rule
    // analysis.
    #[test]
    #[ignore = "needs analysis test harness (BuildViewTestCase)"]
    fn library_sandwich() {}
}
