//! `CcStarlarkInternal.WrappedStarlarkActionFactory` (what `cc_internal.wrap_link_actions`
//! returns) and `CppLinkActionBuilder.LinkActionConstruction`.
//!
//! PORT: Java's class extends `StarlarkActionFactory` (copy constructor `super(parent)`), so it
//! has every `ctx.actions` method plus `declare_shareable_directory`, and overrides
//! `declare_shareable_artifact`. starlark-rust has no subclassing: the value keeps the parent
//! `actions` value and forwards every other attribute to it (the bound methods act on the same
//! rule context, as the copied Java factory does).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::actions::artifact_root::ArtifactRoot;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_analysis::rule::artifact_roots::to_actions_root;
use blaze_analysis::starlark::RuleContextHandle;
use blaze_analysis::starlark::starlark_action_factory::starlark_action_factory_from_value;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_vfs::PathFragment;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::ValueLike;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, StarlarkValue,
    Trace, Tracer, Value, starlark_value,
};

use super::util::errorf;

/// Provides the action construction context, the configuration and methods for creating
/// intermediate and output artifacts for C++ linking.
// Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:57 CppLinkActionBuilder.LinkActionConstruction
#[derive(Clone)]
pub struct LinkActionConstruction {
    shareable_artifacts: bool,
    context: RuleContextHandle,
    config: Arc<BuildConfigurationValue>,
}

impl LinkActionConstruction {
    // Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:49 LinkActionConstruction.getContext
    pub fn get_context(&self) -> RuleContextHandle {
        self.context
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:53 LinkActionConstruction.getConfig
    pub fn get_config(&self) -> &Arc<BuildConfigurationValue> {
        &self.config
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:82 LinkActionConstruction.getBinDirectory
    pub fn get_bin_directory(&self) -> ArtifactRoot {
        let rc = self.context.borrow();
        let repository = rc
            .get_action_owner()
            .get_label()
            .expect("action owner has a label")
            .get_repository()
            .clone();
        to_actions_root(&self.config.get_bin_directory(&repository))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:80 LinkActionConstruction.create
    pub fn create(&self, root_relative_path: &PathFragment) -> Artifact {
        let root = self.get_bin_directory();
        let rc = self.context.borrow();
        if self.shareable_artifacts {
            rc.get_shareable_artifact(root_relative_path, &root)
        } else {
            rc.get_derived_artifact(root_relative_path, &root)
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:77 LinkActionConstruction.createTreeArtifact
    pub fn create_tree_artifact(&self, root_relative_path: &PathFragment) -> Artifact {
        let root = self.get_bin_directory();
        let rc = self.context.borrow();
        if self.shareable_artifacts {
            rc.get_analysis_environment()
                .get_tree_artifact(root_relative_path, &root)
        } else {
            rc.get_tree_artifact(root_relative_path, &root)
        }
    }
}

/// `CppLinkActionBuilder.newActionConstruction(context, config, shareableArtifacts)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppLinkActionBuilder.java:93 CppLinkActionBuilder.newActionConstruction
pub fn new_action_construction(
    context: RuleContextHandle,
    config: Arc<BuildConfigurationValue>,
    shareable_artifacts: bool,
) -> LinkActionConstruction {
    LinkActionConstruction {
        shareable_artifacts,
        context,
        config,
    }
}

/// The `actions` value `wrap_link_actions` returns.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:443 CcStarlarkInternal.WrappedStarlarkActionFactory
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct WrappedStarlarkActionFactory<'v> {
    parent: Value<'v>,
    #[allocative(skip)]
    construction: LinkActionConstruction,
    /// Java `StarlarkRuleContext.ruleLabelCanonicalName`, for the closed-context error.
    rule_label_canonical_name: String,
}

/// The frozen wrapped factory: the closed parent (every method fails as on a closed `ctx`).
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenWrappedStarlarkActionFactory {
    parent: FrozenValue,
    rule_label_canonical_name: String,
}

impl<'v> WrappedStarlarkActionFactory<'v> {
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:446 WrappedStarlarkActionFactory.<init>
    pub fn new(
        parent: Value<'v>,
        construction: LinkActionConstruction,
    ) -> WrappedStarlarkActionFactory<'v> {
        let rule_label_canonical_name = construction
            .context
            .borrow()
            .get_label()
            .get_canonical_form();
        WrappedStarlarkActionFactory {
            parent,
            construction,
            rule_label_canonical_name,
        }
    }

    /// The `actions` value this factory copies.
    pub fn parent(&self) -> Value<'v> {
        self.parent
    }

    pub fn construction(&self) -> &LinkActionConstruction {
        &self.construction
    }

    /// Java `getRuleContext()` (the copied parent's).
    pub fn get_rule_context(&self) -> RuleContextHandle {
        starlark_action_factory_from_value(self.parent)
            .expect("parent is a live actions value")
            .get_rule_context()
    }
}

impl fmt::Debug for WrappedStarlarkActionFactory<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WrappedStarlarkActionFactory")
    }
}

impl fmt::Display for WrappedStarlarkActionFactory<'_> {
    // Java: StarlarkActionFactory.repr (inherited).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.parent)
    }
}

impl fmt::Debug for FrozenWrappedStarlarkActionFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrozenWrappedStarlarkActionFactory")
    }
}

impl fmt::Display for FrozenWrappedStarlarkActionFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.parent)
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for WrappedStarlarkActionFactory<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.parent.trace(tracer);
    }
}

impl<'v> Freeze for WrappedStarlarkActionFactory<'v> {
    type Frozen = FrozenWrappedStarlarkActionFactory;

    // Java: the rule context the factory uses is nullified when `ctx` is closed.
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenWrappedStarlarkActionFactory> {
        Ok(FrozenWrappedStarlarkActionFactory {
            parent: freezer.freeze(self.parent)?,
            rule_label_canonical_name: self.rule_label_canonical_name,
        })
    }
}

impl<'v> AllocValue<'v> for WrappedStarlarkActionFactory<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "actions")]
impl<'v> StarlarkValue<'v> for WrappedStarlarkActionFactory<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("actions", wrapped_actions_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.parent.get_attr(attribute, heap).ok().flatten()
    }

    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        self.parent.has_attr(attribute, heap)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut names = self.parent.dir_attr();
        names.push("declare_shareable_directory".to_owned());
        names.sort();
        names.dedup();
        names
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

#[starlark_value(type = "actions")]
impl<'v> StarlarkValue<'v> for FrozenWrappedStarlarkActionFactory {
    type Canonical = WrappedStarlarkActionFactory<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("actions", wrapped_actions_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.parent
            .to_value()
            .get_attr(attribute, heap)
            .ok()
            .flatten()
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

/// The live wrapped factory behind `this`.
fn live<'v>(
    this: Value<'v>,
    attr_name: &str,
) -> starlark::Result<&'v WrappedStarlarkActionFactory<'v>> {
    if let Some(w) = this.downcast_ref::<WrappedStarlarkActionFactory<'v>>() {
        return Ok(w);
    }
    if let Some(f) = this.downcast_ref::<FrozenWrappedStarlarkActionFactory>() {
        // Java: StarlarkRuleContext.checkMutable of the closed context.
        return Err(errorf(format!(
            "cannot access field or method '{attr_name}' of rule context for '{}' outside of \
             its own rule implementation function",
            f.rule_label_canonical_name
        )));
    }
    Err(errorf(format!(
        "internal error: expected actions, got {}",
        this.get_type()
    )))
}

#[starlark_module]
fn wrapped_actions_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:452 WrappedStarlarkActionFactory.createShareableArtifact
    fn declare_shareable_artifact<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = pos)] artifact_root: Option<Value<'v>>,
    ) -> starlark::Result<StarlarkFile> {
        let _ = artifact_root;
        let w = live(this, "declare_shareable_artifact")?;
        Ok(StarlarkFile::new(
            w.construction.create(&PathFragment::create(path)),
        ))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:461 WrappedStarlarkActionFactory.createShareableDirectory
    fn declare_shareable_directory<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] path: &str,
    ) -> starlark::Result<StarlarkFile> {
        let w = live(this, "declare_shareable_directory")?;
        Ok(StarlarkFile::new(
            w.construction
                .create_tree_artifact(&PathFragment::create(path)),
        ))
    }
}

/// The rule context of an `actions` value (`ctx.actions` or a wrapped factory): Java
/// `((StarlarkActionFactory) actions).getRuleContext()`.
pub fn actions_rule_context(
    v: Value,
    function: &str,
    param: &str,
) -> starlark::Result<RuleContextHandle> {
    if let Some(f) = starlark_action_factory_from_value(v) {
        return Ok(f.get_rule_context());
    }
    if let Some(w) = v.downcast_ref::<WrappedStarlarkActionFactory>() {
        return Ok(w.get_rule_context());
    }
    Err(errorf(format!(
        "in call to {function}(), parameter '{param}' got value of type '{}', want 'actions'",
        v.get_type()
    )))
}
