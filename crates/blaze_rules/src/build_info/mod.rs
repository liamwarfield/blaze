//! Build info at analysis time (agent PY; crates/blaze_analysis/DESIGN.md): the body of
//! `ctx.actions.transform_info_file` / `transform_version_file` (Java
//! `StarlarkActionFactory.transformBuildInfoFile`), which `@bazel_tools//tools/build_defs/
//! build_info:bazel_cc_build_info.bzl` calls to make the `TranslateBuildInfo` actions
//! (`BuildInfoFileWriteAction`) over the workspace status artifacts `bazel-out/stable-status.txt`
//! (`ctx.info_file`) and `bazel-out/volatile-status.txt` (`ctx.version_file`).
//!
//! The Starlark methods themselves are agent S's (`crate::starlark::starlark_action_factory` of
//! blaze_analysis). Java builds and registers the action inside the method; in Blaze the action
//! holds the translation function as an `OwnedFrozenValue`, which only exists once the rule's
//! module is frozen (crates/blaze_analysis/DESIGN.md, "Actions created from Starlark are
//! finished after freezing"). So [`transform_build_info_file`] does everything Java does at call
//! time and returns the output artifact plus a [`BuildInfoFileWriteActionSpec`]; S keeps the
//! function value alive, records a pending action at the call's position in the registration
//! order, and after freezing calls [`BuildInfoFileWriteActionSpec::build`] with the frozen
//! function.
//!
//! blaze_analysis cannot depend on this crate, so S reaches it through a hook the lead installs
//! with [`transform_build_info_file_deferred`] (a plain `fn` with no blaze_rules types).

use std::sync::Arc;

use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::analysis::actions::build_info_file_write_action::BuildInfoFileWriteAction;
use blaze_actions::{ActionRef, Artifact, ArtifactRoot};
use blaze_analysis::rule::RuleContext;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_vfs::PathFragment;
use starlark::values::{OwnedFrozenValue, Value};

/// Everything `new BuildInfoFileWriteAction(...)` takes except the translation callback, which
/// is frozen later.
// Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:72 BuildInfoFileWriteAction.<init>
#[derive(Debug, Clone)]
pub struct BuildInfoFileWriteActionSpec {
    owner: ActionOwner,
    input: Artifact,
    output: Artifact,
    template: Artifact,
    is_volatile: bool,
    /// `BazelModuleContext.of(translationCallback.getModule()).bzlTransitiveDigest()`.
    bzl_transitive_digest: Vec<u8>,
    /// `thread.getSemantics()`.
    semantics: Arc<blaze_starlark::StarlarkSemantics>,
}

impl BuildInfoFileWriteActionSpec {
    /// The action's output (the artifact `transform_*_file` returns).
    pub fn get_output(&self) -> &Artifact {
        &self.output
    }

    /// `new BuildInfoFileWriteAction(owner, input, output, translationFunc, template, isVolatile,
    /// semantics)` with the frozen translation function (the value that was passed as
    /// `transform_func`).
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:519 StarlarkActionFactory.transformBuildInfoFile
    pub fn build(self, translation_callback: OwnedFrozenValue) -> ActionRef {
        Arc::new(BuildInfoFileWriteAction::new(
            self.owner,
            self.input,
            self.output,
            translation_callback,
            self.template,
            self.is_volatile,
            self.bzl_transitive_digest,
            self.semantics,
        ))
    }
}

/// `StarlarkActionFactory.transformBuildInfoFile` up to (excluding) `registerAction`: declares
/// the output (`getConstantMetadataArtifact` for the volatile file, `getDerivedArtifact`
/// otherwise) under the rule's package directory and describes the `BuildInfoFileWriteAction`
/// that reads `stable-status.txt` / `volatile-status.txt`.
///
/// `new_file_root` is the action factory's `newFileRoot()`. `transform_func` must be a Starlark
/// `def` (Java casts it to `StarlarkFunction`), `function_module` is
/// `BazelModuleContext.of(translationFunc.getModule())` (the module context of the module
/// `transform_func` was defined in), `template` is the `template` File.
///
/// The caller (`transform_info_file` / `transform_version_file`, after
/// `BuiltinRestriction.failIfCalledOutsideDefaultAllowlist`) registers
/// `spec.build(frozen transform_func)` in this call's place in the registration order and returns
/// the artifact as a `File`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:500 StarlarkActionFactory.transformBuildInfoFile
pub fn transform_build_info_file<'v>(
    rule_context: &RuleContext,
    new_file_root: &ArtifactRoot,
    transform_func: Value<'v>,
    function_module: Option<Arc<BazelModuleContext>>,
    template: &Artifact,
    output_file_name: &str,
    is_volatile: bool,
) -> (Artifact, BuildInfoFileWriteActionSpec) {
    let template_file = template.clone();
    let fragment = rule_context
        .get_package_directory()
        .get_relative(&PathFragment::create(output_file_name));
    let build_info_file = if is_volatile {
        rule_context
            .get_analysis_environment()
            .get_constant_metadata_artifact(&fragment, new_file_root)
    } else {
        rule_context.get_derived_artifact(&fragment, new_file_root)
    };
    // Java: StarlarkFunction translationFunc = (StarlarkFunction) transformFuncObject;
    if transform_func.get_type() != "function" {
        panic!(
            "ClassCastException: class {} cannot be cast to class \
             net.starlark.java.eval.StarlarkFunction",
            transform_func.get_type()
        );
    }
    // Java reads the digest in BuildInfoFileWriteAction.computeKey from the function's module.
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:196 BuildInfoFileWriteAction.computeKey
    let bzl_transitive_digest = function_module
        .expect("NullPointerException: no BazelModuleContext for the translation function's module")
        .bzl_transitive_digest
        .clone();
    let analysis_environment = rule_context.get_analysis_environment();
    let input = if is_volatile {
        analysis_environment
            .get_volatile_workspace_status_artifact()
            .expect("MissingDepException: workspace status")
    } else {
        analysis_environment
            .get_stable_workspace_status_artifact()
            .expect("MissingDepException: workspace status")
    };
    let spec = BuildInfoFileWriteActionSpec {
        owner: rule_context.get_action_owner(),
        input,
        output: build_info_file.clone(),
        template: template_file,
        is_volatile,
        bzl_transitive_digest,
        // Java: thread.getSemantics() (the rule's analysis thread).
        semantics: rule_context.get_starlark_semantics().clone(),
    };
    (build_info_file, spec)
}

/// The finishing step of [`transform_build_info_file`]: builds the `BuildInfoFileWriteAction`
/// from the frozen translation function.
pub use blaze_analysis::starlark::starlark_action_factory::BuildInfoFileWriteActionBuilder;

/// [`transform_build_info_file`] with the spec boxed as a closure, for the hook blaze_analysis
/// (agent S) calls from `transform_info_file` (`is_volatile = false`) and
/// `transform_version_file` (`is_volatile = true`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkActionFactory.java:500 StarlarkActionFactory.transformBuildInfoFile
pub fn transform_build_info_file_deferred<'v>(
    rule_context: &RuleContext,
    new_file_root: &ArtifactRoot,
    transform_func: Value<'v>,
    function_module: Option<Arc<BazelModuleContext>>,
    template: &Artifact,
    output_file_name: &str,
    is_volatile: bool,
) -> (Artifact, BuildInfoFileWriteActionBuilder) {
    let (artifact, spec) = transform_build_info_file(
        rule_context,
        new_file_root,
        transform_func,
        function_module,
        template,
        output_file_name,
        is_volatile,
    );
    (
        artifact,
        Box::new(move |translation_callback| spec.build(translation_callback)),
    )
}
