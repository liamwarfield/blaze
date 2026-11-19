//! `ctx`: port of `analysis/starlark/StarlarkRuleContext.java`
//! (`starlarkbuildapi/StarlarkRuleContextApi.java`), including `ctx.outputs` (Java's nested
//! `Outputs`) and `ctx.split_attr`.
//!
//! Java nullifies the context (`close()`) when the implementation function returns; later
//! accesses fail with "cannot access field or method '...' of rule context for '...' outside of
//! its own rule implementation function". Here the context is a complex Starlark value that
//! freezes into [`ClosedStarlarkRuleContext`] (and `ctx.actions` / `ctx.outputs` into their
//! closed forms), so a `ctx` that escapes into a provider behaves the same.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::actions::path_mapper::FileRoot;
use blaze_actions::analysis::runfiles::{ConflictPolicy, Runfiles, RunfilesBuilder};
use blaze_actions::analysis::starlark::starlark_file::{
    StarlarkFile, StarlarkFileRoot, artifact_from_value,
};
use blaze_build_config::build_options::{JavaCollectionClass, StarlarkOptionValue};
use blaze_cmdline::{Label, LabelPackageContext};
use blaze_collect::Order;
use blaze_packages::aspect::AspectDescriptor;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::implicit_outputs_function::ImplicitOutputsFunction;
use blaze_packages::rule_class::rule_class::{RuleClass, STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME};
use blaze_packages::types::{LabelClass, Type};
use blaze_shell::ShellUtils;
use blaze_starlark::depset::Depset;
use blaze_starlark::label::LabelValue;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::collections::Hashed;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::dict::{AllocDict, DictRef};
use starlark::values::list::AllocList;
use starlark::values::none::NoneType;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, Heap, NoSerialize, StarlarkValue, Trace, Tracer,
    Value, ValueLike, starlark_value,
};

use super::fragment_collection::FragmentCollection;
use super::starlark_action_factory::{
    SharedRuleContextState, StarlarkActionContext, StarlarkActionFactory, closed_error,
    context_repr,
};
use super::starlark_attributes_collection::{
    StarlarkAttributesCollectionBuilder, StarlarkAttributesCollectionValue, alloc_struct,
    convert_string_to_label_list_map, convert_string_to_label_map,
};
use super::starlark_configuration::{StarlarkConfiguration, is_code_coverage_enabled};
use super::starlark_runfiles::{StarlarkRunfiles, symlink_entry_from_value};
use super::starlark_toolchain_context::{StarlarkExecGroupCollection, StarlarkToolchainContext};
use super::thread_payload::RuleContextHandle;
use super::util::{
    alloc_immutable_dict, alloc_immutable_list, check_bool_param, depset_cast_artifacts, errorf,
    fail_if_called_outside_default_allowlist, is_sequence, param_type_error,
    sequence_cast_artifacts, sequence_cast_strings,
};
use crate::configured_target::ConfiguredTarget;
use crate::configuredtargets::{target_from_starlark, target_to_starlark};
use crate::platform::ConstraintValueInfo;
use crate::providers::alias_provider::AliasProvider;
use crate::providers::file_provider::FileProvider;
use crate::providers::file_provider::{FilesToRunProvider, RunfilesProvider};
use crate::rule::configuration_make_variable_context::{
    ConfigurationMakeVariableContext, MapBackedMakeVariableSupplier,
};
use crate::rule::location_expander::LocationExpander;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use crate::rule::{RuleContext, RuleErrorException};

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:131 StarlarkRuleContext.EXECUTABLE_OUTPUT_NAME
const EXECUTABLE_OUTPUT_NAME: &str = "executable";

// Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:34 ConfigurationTransition.PATCH_TRANSITION_KEY
const PATCH_TRANSITION_KEY: &str = "";

/// The `ctx` value of a rule implementation (type `ctx`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:127 StarlarkRuleContext
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkRuleContext<'v> {
    #[allocative(skip)]
    context: StarlarkActionContext,
    actions: Value<'v>,
    fragments: Value<'v>,
    attr: Value<'v>,
    executable: Value<'v>,
    file: Value<'v>,
    files: Value<'v>,
    split_attr: Option<Value<'v>>,
    outputs: Option<Value<'v>>,
    /// Java `ruleAttributesCollection` (`ctx.rule`; aspects only).
    rule_attributes_collection: Option<Value<'v>>,
    /// Java `aspectDescriptor`: the aspect for which the context is created, or `None` if it is
    /// for a rule.
    #[allocative(skip)]
    aspect_descriptor: Option<AspectDescriptor>,
    /// Java `cachedMakeVariables` (`ctx.var`).
    #[allocative(skip)]
    cached_make_variables: Cell<Option<Value<'v>>>,
    /// The current rule class under evaluation (in case of extended rule this changes to parent
    /// when ctx.super is called).
    #[allocative(skip)]
    rule_class_under_evaluation: RefCell<Arc<RuleClass>>,
    /// Was super called in the context of current parent, it's set to false each time
    /// ruleClassUnderEvaluation changes, and it's expected to be set to true when ctx.super is
    /// called.
    #[allocative(skip)]
    super_called: Cell<bool>,
    /// Counter for calls to `ctx.resolve_command` with a long command.
    #[allocative(skip)]
    resolve_command_script_counter: Cell<i32>,
}

/// `ctx` after the rule implementation returned (Java: a nullified `StarlarkRuleContext`).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ClosedStarlarkRuleContext {
    rule_label_canonical_name: String,
    is_for_aspect: bool,
}

impl fmt::Debug for StarlarkRuleContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StarlarkRuleContext({})",
            self.context.rule_label_canonical_name
        )
    }
}

impl fmt::Display for StarlarkRuleContext<'_> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:587 StarlarkRuleContext.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.context.repr())
    }
}

impl fmt::Display for ClosedStarlarkRuleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&context_repr(
            self.is_for_aspect,
            &self.rule_label_canonical_name,
        ))
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for StarlarkRuleContext<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.actions.trace(tracer);
        self.fragments.trace(tracer);
        self.attr.trace(tracer);
        self.executable.trace(tracer);
        self.file.trace(tracer);
        self.files.trace(tracer);
        self.split_attr.trace(tracer);
        self.outputs.trace(tracer);
        self.rule_attributes_collection.trace(tracer);
        let mut v = self.cached_make_variables.get();
        v.trace(tracer);
        self.cached_make_variables.set(v);
    }
}

impl<'v> Freeze for StarlarkRuleContext<'v> {
    type Frozen = ClosedStarlarkRuleContext;

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:464 StarlarkRuleContext.close
    // (the nullified object; the `super` check runs in `close` below).
    fn freeze(self, _freezer: &Freezer) -> FreezeResult<ClosedStarlarkRuleContext> {
        Ok(ClosedStarlarkRuleContext {
            rule_label_canonical_name: self.context.rule_label_canonical_name,
            is_for_aspect: self.context.is_for_aspect,
        })
    }
}

impl<'v> AllocValue<'v> for StarlarkRuleContext<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "ctx")]
impl<'v> StarlarkValue<'v> for StarlarkRuleContext<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("ctx", ctx_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkRuleContextApi.java:149 StarlarkRuleContextApi.callParent
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if attribute == "super" {
            return Some(heap.alloc(CtxSuper));
        }
        None
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "super"
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.context.repr());
    }
}

#[starlark_value(type = "ctx")]
impl<'v> StarlarkValue<'v> for ClosedStarlarkRuleContext {
    type Canonical = StarlarkRuleContext<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("ctx", ctx_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

/// The live context behind `this`, after Java's `checkMutable(attr_name)`.
fn live<'v>(this: Value<'v>, attr_name: &str) -> starlark::Result<&'v StarlarkRuleContext<'v>> {
    let ctx = live_unchecked(this, attr_name)?;
    ctx.context.check_mutable(attr_name)?;
    Ok(ctx)
}

/// The live context behind `this`, for the methods Java does not guard with `checkMutable`
/// (the closed context fails like `checkMutable`, where Java would dereference null).
fn live_unchecked<'v>(
    this: Value<'v>,
    attr_name: &str,
) -> starlark::Result<&'v StarlarkRuleContext<'v>> {
    if let Some(ctx) = this.downcast_ref::<StarlarkRuleContext<'v>>() {
        return Ok(ctx);
    }
    if let Some(c) = this.downcast_ref::<ClosedStarlarkRuleContext>() {
        return Err(closed_error(attr_name, &c.rule_label_canonical_name));
    }
    Err(errorf(format!(
        "internal error: expected ctx, got {}",
        this.get_type()
    )))
}

/// The live `ctx` value of `v` (Java `(StarlarkRuleContext) ctx`), for natives such as
/// `cc_internal` and `py_builtins` that receive `ctx`.
pub fn starlark_rule_context_from_value<'v>(v: Value<'v>) -> Option<&'v StarlarkRuleContext<'v>> {
    v.downcast_ref::<StarlarkRuleContext<'v>>()
}

/// The rule context behind a live `ctx` value (Java `((StarlarkRuleContext) ctx).getRuleContext()`),
/// borrowed; drop the guard before calling back into Starlark. A closed `ctx` fails like
/// `checkMutable`.
pub fn rule_context_from_ctx<'v>(
    ctx: Value<'v>,
) -> starlark::Result<std::cell::Ref<'v, RuleContext>> {
    let ctx = live_unchecked(ctx, "getRuleContext")?;
    Ok(ctx.context.handle.borrow())
}

impl<'v> StarlarkRuleContext<'v> {
    /// Creates a new StarlarkRuleContext wrapping the rule context behind `handle`.
    ///
    /// `aspect_descriptor` is the aspect for which the context is created, or `None` if it is
    /// for a rule.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:191 StarlarkRuleContext.<init>
    pub(crate) fn create(
        handle: RuleContextHandle,
        aspect_descriptor: Option<AspectDescriptor>,
        heap: Heap<'v>,
    ) -> Result<StarlarkRuleContext<'v>, RuleErrorException> {
        let is_for_aspect = aspect_descriptor.is_some();
        let shared = Arc::new(SharedRuleContextState::default());
        let (rule_label_canonical_name, rule) = {
            let rc = handle.borrow();
            (rc.get_label().get_canonical_form(), rc.get_rule().clone())
        };
        let fragments = heap.alloc(FragmentCollection::new(&handle.borrow()));
        let rule_class_under_evaluation = rule.get_rule_class_object().clone();
        let is_executable = rule_class_under_evaluation.is_executable_starlark();

        let attributes_collection;
        let split_attr;
        let outputs_object: Option<Vec<(String, Value<'v>)>>;
        let rule_attributes_collection;
        if aspect_descriptor.is_none() {
            let attributes: Vec<_> = rule
                .get_rule_class_object()
                .get_attribute_provider()
                .get_attributes()
                .iter()
                .filter(|attribute| attribute.get_name() != "aspect_hints")
                .cloned()
                .collect();

            // Populate ctx.outputs.
            let mut outputs: IndexMap<String, Value<'v>> = IndexMap::new();
            // These getters do some computational work to return a view, so ensure we only do
            // it once.
            let explicit_out_map = rule.get_explicit_output_file_map();
            let implicit_out_map = rule.get_starlark_implicit_output_file_map();
            let add_output = |key: &str,
                              value: Value<'v>,
                              outputs: &mut IndexMap<String, Value<'v>>|
             -> Result<(), RuleErrorException> {
                // TODO(bazel-team): We should reject outputs whose key is not an identifier.
                // Today this is allowed, and the resulting ctx.outputs value can be retrieved
                // using getattr().
                if outputs.contains_key(key) || (is_executable && EXECUTABLE_OUTPUT_NAME == key) {
                    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:369 Outputs.addOutput
                    return Err(handle.borrow_mut().throw_with_rule_error(&format!(
                        "Multiple outputs with the same key: {key}"
                    )));
                }
                outputs.insert(key.to_owned(), value);
                Ok(())
            };
            // Add the explicit outputs -- values of attributes of type OUTPUT or OUTPUT_LIST.
            // We must iterate over the attribute definitions, and not just the entries in the
            // explicitOutMap, because the latter omits empty output attributes, which must still
            // generate None or [] fields in the struct.
            for a in &attributes {
                // Skip non-output attrs.
                let attr_name = a.get_name();
                let type_ = a.get_type();
                if type_.get_label_class() != LabelClass::Output {
                    continue;
                }

                // Grab all associated outputs.
                // Java: StarlarkList<Artifact> artifacts = StarlarkList.immutableCopyOf(...)
                let mut artifacts: Vec<Artifact> = Vec::new();
                if let Some(output_files) = explicit_out_map.get(attr_name) {
                    for output_file in output_files {
                        artifacts.push(handle.borrow_mut().create_output_artifact_for(output_file));
                    }
                }

                // For singular output attributes, unwrap sole element or else use None for arity
                // mismatch.
                if type_ == Type::Output {
                    if artifacts.len() == 1 {
                        let artifact = artifacts.pop().unwrap();
                        add_output(
                            attr_name,
                            heap.alloc(StarlarkFile::new(artifact)),
                            &mut outputs,
                        )?;
                    } else {
                        add_output(attr_name, Value::new_none(), &mut outputs)?;
                    }
                } else if type_ == Type::OutputList {
                    let list = alloc_immutable_list(
                        heap,
                        artifacts
                            .into_iter()
                            .map(|a| heap.alloc(StarlarkFile::new(a)))
                            .collect(),
                    );
                    add_output(attr_name, list, &mut outputs)?;
                } else {
                    return Err(handle.borrow_mut().throw_with_rule_error(&format!(
                        "Attribute {attr_name} has unexpected output type {type_:?}"
                    )));
                }
            }
            // Add the implicit outputs. In the case where the rule has a native-defined implicit
            // outputs function, nothing is added. Note that Rule ensures that Starlark-defined
            // implicit output keys don't conflict with output attribute names.
            // TODO(bazel-team): Also see about requiring the key to be a valid Starlark
            // identifier.
            for (key, output_file) in &implicit_out_map {
                let artifact = handle.borrow_mut().create_output_artifact_for(output_file);
                add_output(key, heap.alloc(StarlarkFile::new(artifact)), &mut outputs)?;
            }

            outputs_object = Some(outputs.into_iter().collect());

            // Populate ctx.attr.
            attributes_collection = {
                let rc = handle.borrow();
                let semantics = rc.get_starlark_semantics().clone();
                let mut builder = StarlarkAttributesCollectionBuilder::new(
                    rc.get_rule_prerequisites_collection(),
                    heap,
                    &semantics,
                );
                for attribute in &attributes {
                    let value = rc
                        .attributes()
                        .get(attribute.get_name(), attribute.get_type());
                    builder.add_attribute(attribute, value.as_ref());
                }
                builder.build(rule.get_rule_class().to_owned())
            };
            split_attr = Some(build_split_attribute_info(
                &attributes,
                &handle.borrow(),
                heap,
            ));
            rule_attributes_collection = None;
        } else {
            // ASPECT
            outputs_object = None;
            let rc = handle.borrow();
            let semantics = rc.get_starlark_semantics().clone();
            let main_aspect = rc
                .get_main_aspect()
                .expect("an aspect context has a main aspect")
                .clone();
            let attributes: Vec<_> = main_aspect
                .get_definition()
                .get_attributes()
                .values()
                .cloned()
                .collect();

            let mut aspect_builder = StarlarkAttributesCollectionBuilder::new(
                rc.get_main_aspect_prerequisites_collection(),
                heap,
                &semantics,
            );
            for attribute in &attributes {
                let default_value = aspect_attribute_default_value(attribute, rc.attributes());
                aspect_builder.add_attribute(attribute, default_value.as_ref());
            }
            attributes_collection = aspect_builder.build(rule.get_rule_class().to_owned());

            split_attr = None;
            let mut rule_builder = StarlarkAttributesCollectionBuilder::new(
                rc.get_rule_prerequisites_collection(),
                heap,
                &semantics,
            );
            match rc
                .get_base_target_configuration_make_variable_context()
                .collect_make_variables()
            {
                // Java: collectMakeVariables().buildImmutable()
                Ok(make_variables) => {
                    rule_builder.put_all_rule_variables(&make_variables);
                }
                Err(_) => {
                    return Err(rc.throw_with_rule_error("Exception expanding template variables"));
                }
            }

            for attribute in rule
                .get_rule_class_object()
                .get_attribute_provider()
                .get_attributes()
                .iter()
            {
                let value = rc
                    .attributes()
                    .get(attribute.get_name(), attribute.get_type());
                rule_builder.add_attribute(attribute, value.as_ref());
            }
            for aspect in rc.get_aspects() {
                if *aspect == main_aspect {
                    // Aspect's own attributes are in <code>attributesCollection</code>.
                    continue;
                }
                for attribute in aspect.get_definition().get_attributes().values() {
                    let default_value = aspect_attribute_default_value(attribute, rc.attributes());
                    rule_builder.add_attribute(attribute, default_value.as_ref());
                }
            }

            rule_attributes_collection = Some(rule_builder.build(rule.get_rule_class().to_owned()));
        }

        let context = StarlarkActionContext {
            handle,
            shared,
            rule_label_canonical_name,
            is_for_aspect,
            executable_runfiles_map: attributes_collection.executable_runfiles_map.clone(),
            subrule: None,
        };
        let outputs_value = outputs_object.map(|outputs| {
            heap.alloc(CtxOutputs {
                context: context.clone(),
                is_executable,
                outputs,
                executable_created: Cell::new(false),
            })
        });
        let rule_attributes_collection_value = rule_attributes_collection.map(|collection| {
            heap.alloc(StarlarkAttributesCollectionValue::new(
                context.clone(),
                collection,
                heap,
            ))
        });
        let actions = heap.alloc(StarlarkActionFactory::new(context.clone()));
        Ok(StarlarkRuleContext {
            context,
            actions,
            fragments,
            attr: attributes_collection.attr_object,
            executable: attributes_collection.executable_object,
            file: attributes_collection.file_object,
            files: attributes_collection.files_object,
            split_attr,
            outputs: outputs_value,
            rule_attributes_collection: rule_attributes_collection_value,
            aspect_descriptor,
            cached_make_variables: Cell::new(None),
            rule_class_under_evaluation: RefCell::new(rule_class_under_evaluation),
            super_called: Cell::new(false),
            resolve_command_script_counter: Cell::new(0),
        })
    }

    /// Java `getRuleContext()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:597 StarlarkRuleContext.getRuleContext
    pub fn get_rule_context(&self) -> RuleContextHandle {
        self.context.handle
    }

    /// The action context `ctx.actions` uses.
    pub fn get_action_context(&self) -> &StarlarkActionContext {
        &self.context
    }

    /// Java `getRuleClassUnderEvaluation()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:309 StarlarkRuleContext.getRuleClassUnderEvaluation
    pub fn get_rule_class_under_evaluation(&self) -> Arc<RuleClass> {
        self.rule_class_under_evaluation.borrow().clone()
    }

    /// Java `isExecutable()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:440 StarlarkRuleContext.isExecutable
    pub fn is_executable(&self) -> bool {
        self.context
            .handle
            .borrow()
            .get_rule()
            .get_rule_class_object()
            .is_executable_starlark()
    }

    /// Java `isDefaultExecutableCreated()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:444 StarlarkRuleContext.isDefaultExecutableCreated
    pub fn is_default_executable_created(&self) -> bool {
        self.outputs
            .and_then(|o| o.downcast_ref::<CtxOutputs>())
            .is_some_and(|o| o.executable_created.get())
    }

    /// Java `isForAspect()`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:929 StarlarkRuleContext.isForAspect
    pub fn is_for_aspect(&self) -> bool {
        self.context.is_for_aspect
    }

    /// Java `getAspectDescriptor()`: the aspect this context is for (`None` for a rule).
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:500 StarlarkRuleContext.getAspectDescriptor
    pub fn get_aspect_descriptor(&self) -> Option<&AspectDescriptor> {
        self.aspect_descriptor.as_ref()
    }

    /// Java `setLockedForSubrule(lockedBy)`: the id of the locking subrule context, 0 for
    /// null.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:313 StarlarkRuleContext.setLockedForSubrule
    pub(crate) fn set_locked_for_subrule(&self, locked_by: u64) {
        self.context
            .shared
            .locked_by
            .store(locked_by, std::sync::atomic::Ordering::Relaxed);
    }

    /// Java `getLockedForSubrule()`: the id of the locking subrule context, 0 for null.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:317 StarlarkRuleContext.getLockedForSubrule
    pub(crate) fn get_locked_for_subrule(&self) -> u64 {
        self.context
            .shared
            .locked_by
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Java `close()`'s check: an extended rule must call `super`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:464 StarlarkRuleContext.close
    pub(crate) fn close(&self) {
        // Check super was called
        if self
            .rule_class_under_evaluation
            .borrow()
            .get_starlark_parent()
            .is_some()
            && !self.super_called.get()
            && !self.is_for_aspect()
        {
            self.context
                .handle
                .borrow_mut()
                .rule_error("'super' was not called.");
        }
    }

    /// Java `lookedUpVariables()`: the variables read through `ctx.var` (only when tracking
    /// required fragments).
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:774 StarlarkRuleContext.lookedUpVariables
    pub fn looked_up_variables(&self) -> Option<IndexSet<String>> {
        if !self
            .context
            .handle
            .borrow()
            .should_include_required_config_fragments_provider()
        {
            return None;
        }
        // Java: ImmutableKeyTrackingDict.getAccessedKeys
        blaze_util::unported!("StarlarkRuleContext.lookedUpVariables")
    }

    /// Java `getExecutableRunfiles(executable, what)`.
    pub fn get_executable_runfiles(
        &self,
        executable: &Artifact,
        what: &str,
    ) -> starlark::Result<Option<Arc<FilesToRunProvider>>> {
        self.context.get_executable_runfiles(executable, what)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:786 StarlarkRuleContext.getRequestedToolchainTypeLabelsFromAutoExecGroups
    pub(crate) fn get_requested_toolchain_type_labels_from_auto_exec_groups(
        rule_context: &RuleContext,
    ) -> IndexSet<Label> {
        let toolchain_contexts = rule_context
            .get_toolchain_contexts()
            .expect("toolchain contexts");
        let mut result = IndexSet::new();
        for exec_group_name in toolchain_contexts.get_exec_group_names() {
            if !blaze_packages::rule_class::DeclaredExecGroup::is_automatic(exec_group_name) {
                continue;
            }
            if let Some(tc) = toolchain_contexts.get_toolchain_context(exec_group_name) {
                for label in tc.requested_toolchain_type_labels().keys() {
                    result.insert(label.clone());
                }
            }
        }
        result
    }

    /// Java `expandMakeVariables(attributeName, command, additionalSubstitutionsMap)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:960 StarlarkRuleContext.expandMakeVariables
    fn expand_make_variables_with(
        &self,
        attribute_name: &str,
        command: &str,
        additional_substitutions_map: IndexMap<String, String>,
    ) -> String {
        let rc = self.context.handle.borrow();
        // Java: an anonymous ConfigurationMakeVariableContext whose lookupVariable consults
        // additionalSubstitutionsMap first: a supplier ahead of all others.
        let make_variable_context = ConfigurationMakeVariableContext::with_extra_suppliers(
            rc.get_rule().get_package_declarations(),
            rc.get_configuration(),
            rc.get_default_template_variable_providers(),
            vec![Box::new(MapBackedMakeVariableSupplier::new(
                additional_substitutions_map,
            ))],
        );
        rc.get_expander_for(Arc::new(make_variable_context))
            .expand(Some(attribute_name), command)
    }
}

/// The value of an aspect attribute for `ctx.attr` / `ctx.rule.attr` (Java
/// `attribute.getDefaultValue(null)`, with a `ComputedDefault` evaluated against
/// `ruleContext.attributes()`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:279 StarlarkRuleContext.<init>
fn aspect_attribute_default_value(
    attribute: &blaze_packages::attribute::attribute::Attribute,
    rule_attributes: &dyn AttributeMap,
) -> Option<blaze_packages::types::AttrValue> {
    let default_value = attribute.get_default_value(None);
    if let Some(blaze_packages::types::AttrValue::ComputedDefault(computed_default)) =
        &default_value
    {
        return computed_default.get_default(rule_attributes);
    }
    default_value
}

/// Java `buildSplitAttributeInfo(attributes, ruleContext)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:507 StarlarkRuleContext.buildSplitAttributeInfo
fn build_split_attribute_info<'v>(
    attributes: &[blaze_packages::attribute::attribute::Attribute],
    rule_context: &RuleContext,
    heap: Heap<'v>,
) -> Value<'v> {
    let mut split_attr_infos: Vec<(String, Value<'v>)> = Vec::new();
    for attr in attributes {
        if !attr.get_transition_factory().is_split() {
            continue;
        }
        let split_prereqs = rule_context
            .get_rule_prerequisites_collection()
            .get_split_prerequisites(attr.get_name());

        let mut split_prereqs_map: Vec<(Value<'v>, Value<'v>)> = Vec::new();
        for (key, prereqs) in &split_prereqs {
            // Skip a split with an empty dependency list.
            // TODO(jungjw): Figure out exactly which cases trigger this and see if this can be
            // made more error-proof.
            if prereqs.is_empty() {
                continue;
            }

            let targets: Vec<ConfiguredTarget> = prereqs
                .iter()
                .map(|p| p.get_configured_target().clone())
                .collect();
            let value = if attr.get_type() == Type::Label {
                blaze_util::check_state!(prereqs.len() == 1);
                target_to_starlark(&targets[0], heap, rule_context.get_starlark_semantics())
            } else if attr.get_type() == Type::LabelDictUnary {
                match rule_context
                    .attributes()
                    .get(attr.get_name(), Type::LabelDictUnary)
                {
                    Some(blaze_packages::types::AttrValue::LabelDictUnary(m)) => {
                        convert_string_to_label_map(
                            heap,
                            &m,
                            &targets,
                            rule_context.get_starlark_semantics(),
                        )
                    }
                    other => panic!("ClassCastException: LABEL_DICT_UNARY value {other:?}"),
                }
            } else if attr.get_type() == Type::LabelListDict {
                match rule_context
                    .attributes()
                    .get(attr.get_name(), Type::LabelListDict)
                {
                    Some(blaze_packages::types::AttrValue::LabelListDict(m)) => {
                        convert_string_to_label_list_map(
                            heap,
                            &m,
                            &targets,
                            rule_context.get_starlark_semantics(),
                        )
                    }
                    other => panic!("ClassCastException: LABEL_LIST_DICT value {other:?}"),
                }
            } else {
                // Java: StarlarkList.immutableCopyOf(...)
                alloc_immutable_list(
                    heap,
                    targets
                        .iter()
                        .map(|t| target_to_starlark(t, heap, rule_context.get_starlark_semantics()))
                        .collect(),
                )
            };

            match key {
                Some(k) if k != PATCH_TRANSITION_KEY => {
                    split_prereqs_map.push((heap.alloc_str(k).to_value(), value));
                }
                _ => {
                    // If the split transition is not in effect, then the key will be missing
                    // since there's nothing to key on because the dependencies aren't split and
                    // getSplitPrerequisites() behaves like getPrerequisites(). This also means
                    // there should be only one entry in the map. Use None in Starlark to
                    // represent this.
                    blaze_util::check_state!(split_prereqs.len() == 1);
                    split_prereqs_map.push((Value::new_none(), value));
                }
            }
        }

        // Java: Dict.immutableCopyOf(splitPrereqsMap)
        split_attr_infos.push((
            attr.get_public_name(),
            alloc_immutable_dict(heap, split_prereqs_map),
        ));
    }

    // Java: StarlarkRuleContext.java:579 (no space after "split_attr.", as in Java).
    alloc_struct(
        heap,
        split_attr_infos,
        "No attribute '%s' in split_attr.This attribute is not defined with a split \
         configuration.",
    )
}

/// Experimental: Calls parent's implementation function and returns its providers.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:612 StarlarkRuleContext.callParent
fn call_parent<'v>(
    this: Value<'v>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let ctx = live(this, "super()")?;
    if ctx.is_for_aspect() {
        return Err(errorf("Can't use 'super' call in an aspect."));
    }
    let parent = ctx
        .rule_class_under_evaluation
        .borrow()
        .get_starlark_parent()
        .cloned();
    let Some(parent) = parent else {
        return Err(errorf("Can't use 'super' call, the rule has no parent."));
    };
    if ctx.super_called.get() {
        return Err(errorf("'super' called the second time."));
    }

    let previous_class_under_evaluation = ctx.get_rule_class_under_evaluation();
    *ctx.rule_class_under_evaluation.borrow_mut() = parent.clone();

    ctx.super_called.set(false);
    let raw_providers = super::starlark_rule_configured_target_util::eval_rule(
        ctx.context.handle,
        &parent,
        this,
        eval,
    );
    if parent.get_starlark_parent().is_some() && !ctx.super_called.get() {
        ctx.context.handle.borrow_mut().rule_error(&format!(
            "in {} rule: 'super' was not called.",
            parent.get_name()
        ));
    }
    *ctx.rule_class_under_evaluation.borrow_mut() = previous_class_under_evaluation;

    let Some(raw_providers) = raw_providers else {
        return Err(errorf("Error evaluating parent rule."));
    };

    // Normalize the return type
    let raw_providers = if crate::info::is_info(raw_providers) {
        // Either an old-style struct or a single declared provider (not in a list)
        if crate::info::info_provider_key(raw_providers)
            == Some(blaze_packages::starlark_defs::builtin_providers::STRUCT.key())
        {
            return Err(errorf(
                "Parent rule returned struct providers. Rules returning struct providers \
                 can't be extended.",
            ));
        }
        eval.heap().alloc(AllocList([raw_providers]))
    } else if raw_providers.is_none() {
        // Java: StarlarkList.empty()
        alloc_immutable_list(eval.heap(), Vec::new())
    } else {
        raw_providers
    };
    ctx.super_called.set(true);
    Ok(raw_providers)
}

/// `ctx.super`: a callable bound to the running rule's context.
///
/// PORT: the Starlark method is named `super`, a Rust keyword that `#[starlark_module]` cannot
/// use as a function name, so `ctx` exposes it through `get_attr`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct CtxSuper;

starlark::starlark_simple_value!(CtxSuper);

impl fmt::Display for CtxSuper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<built-in method super of ctx value>")
    }
}

#[starlark_value(type = "builtin_function_or_method")]
impl<'v> StarlarkValue<'v> for CtxSuper {
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &starlark::eval::Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        args.no_positional_args(eval.heap())?;
        args.no_named_args()?;
        let ctx = super::thread_payload::get_starlark_rule_context(eval).ok_or_else(|| {
            errorf("super can only be called from a rule or aspect implementation")
        })?;
        call_parent(ctx, eval)
    }
}

// -------------------------------------------------------------------------------------------
// ctx.outputs

/// The value of `ctx.outputs` (Java `StarlarkRuleContext.Outputs`, type `ctx.outputs`).
///
/// Note: There is only one `Outputs` object per rule context, so default (object identity)
/// equals and hashCode suffice.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:353 StarlarkRuleContext.Outputs
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct CtxOutputs<'v> {
    #[allocative(skip)]
    context: StarlarkActionContext,
    is_executable: bool,
    outputs: Vec<(String, Value<'v>)>,
    #[allocative(skip)]
    executable_created: Cell<bool>,
}

/// `ctx.outputs` after the rule implementation returned.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ClosedCtxOutputs {
    rule_label_canonical_name: String,
}

impl fmt::Debug for CtxOutputs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CtxOutputs")
    }
}

impl<'v> CtxOutputs<'v> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:384 Outputs.getFieldNames
    fn get_field_names(&self) -> Vec<String> {
        let mut result = Vec::new();
        if self.is_executable && self.executable_created.get() {
            result.push(EXECUTABLE_OUTPUT_NAME.to_owned());
        }
        result.extend(self.outputs.iter().map(|(k, _)| k.clone()));
        result
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:398 Outputs.getValue
    fn get_value(&self, name: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if self.is_executable && EXECUTABLE_OUTPUT_NAME == name {
            self.executable_created.set(true);
            // createOutputArtifact() will cache the created artifact.
            let artifact = self.context.handle.borrow_mut().create_output_artifact();
            return Some(heap.alloc(StarlarkFile::new(artifact)));
        }
        self.outputs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| *v)
    }
}

impl fmt::Display for CtxOutputs<'_> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:417 Outputs.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.context.is_immutable() {
            return write!(
                f,
                "ctx.outputs(for {})",
                self.context.rule_label_canonical_name
            );
        }
        f.write_str("ctx.outputs(")?;
        // Sort by field name to ensure deterministic output.
        let mut fields = self.get_field_names();
        fields.sort();
        let mut first = true;
        for field in fields {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            f.write_str(&field)?;
            f.write_str(" = ")?;
            if self.is_executable && field == EXECUTABLE_OUTPUT_NAME {
                // Java: getValue("executable") returns the (cached) default executable.
                let artifact = self.context.handle.borrow_mut().create_output_artifact();
                f.write_str(&StarlarkFile::new(artifact).to_string())?;
            } else if let Some((_, v)) = self.outputs.iter().find(|(k, _)| *k == field) {
                f.write_str(&v.to_repr())?;
            }
        }
        f.write_str(")")
    }
}

impl fmt::Display for ClosedCtxOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ctx.outputs(for {})", self.rule_label_canonical_name)
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for CtxOutputs<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        for (_, v) in &mut self.outputs {
            v.trace(tracer);
        }
    }
}

impl<'v> Freeze for CtxOutputs<'v> {
    type Frozen = ClosedCtxOutputs;

    fn freeze(self, _freezer: &Freezer) -> FreezeResult<ClosedCtxOutputs> {
        Ok(ClosedCtxOutputs {
            rule_label_canonical_name: self.context.rule_label_canonical_name,
        })
    }
}

impl<'v> AllocValue<'v> for CtxOutputs<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "ctx.outputs")]
impl<'v> StarlarkValue<'v> for CtxOutputs<'v> {
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }

    /// Rust callers without an evaluator: `None` where Java's `getValue` throws.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if self.context.is_immutable() {
            return None;
        }
        self.get_value(attribute, heap)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:394 Outputs.getValue
    fn get_attr_fallible(
        &self,
        attribute: Hashed<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        // Java: checkMutable()
        if self.context.is_immutable() {
            return Err(errorf(outputs_check_mutable_message(
                &self.context.rule_label_canonical_name,
            )));
        }
        Ok(self.get_value(attribute.key(), eval.heap()))
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr (`getValue(name) != null`,
    // so `hasattr(ctx.outputs, "executable")` creates the default executable like a read does,
    // and a locked `ctx.outputs` fails)
    fn has_attr_fallible(
        &self,
        attribute: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        Ok(self
            .get_attr_fallible(Hashed::new(attribute), eval)?
            .is_some())
    }

    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        self.get_attr(attribute, heap).is_some()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:408 Outputs.getErrorMessageForUnknownField
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(outputs_error_message_for_unknown_field(attribute))
    }

    fn dir_attr(&self) -> Vec<String> {
        self.get_field_names()
    }
}

#[starlark_value(type = "ctx.outputs")]
impl<'v> StarlarkValue<'v> for ClosedCtxOutputs {
    type Canonical = CtxOutputs<'v>;

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:441 Outputs.checkMutable
    // (every `getValue` of a closed `ctx.outputs` fails)
    fn get_attr_fallible(
        &self,
        _attribute: Hashed<&str>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Option<Value<'v>>> {
        Err(errorf(outputs_check_mutable_message(
            &self.rule_label_canonical_name,
        )))
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr (`getValue` throws)
    fn has_attr_fallible(
        &self,
        _attribute: &str,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        Err(errorf(outputs_check_mutable_message(
            &self.rule_label_canonical_name,
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:408 Outputs.getErrorMessageForUnknownField
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        Some(outputs_error_message_for_unknown_field(attribute))
    }
}

/// The message of Java `Outputs.checkMutable`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:441 Outputs.checkMutable
fn outputs_check_mutable_message(rule_label_canonical_name: &str) -> String {
    format!(
        "cannot access outputs of rule '{rule_label_canonical_name}' outside of its own rule \
         implementation function"
    )
}

/// Java `Outputs.getErrorMessageForUnknownField`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:408 Outputs.getErrorMessageForUnknownField
fn outputs_error_message_for_unknown_field(name: &str) -> String {
    format!(
        "No attribute '{name}' in outputs. Make sure you declared a rule output with this name."
    )
}

// -------------------------------------------------------------------------------------------
// Runfiles.Builder.addRunfiles(ruleContext, mapping)

/// Java `RunfilesProvider.DATA_RUNFILES` / `DEFAULT_RUNFILES`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunfilesMapping {
    Default,
    Data,
}

// Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:48 RunfilesProvider.DEFAULT_RUNFILES
// Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:67 RunfilesProvider.DATA_RUNFILES
fn apply_mapping(mapping: RunfilesMapping, input: &ConfiguredTarget) -> Runfiles {
    match input.get_provider::<RunfilesProvider>() {
        Some(provider) => match mapping {
            RunfilesMapping::Default => provider.get_default_runfiles().clone(),
            RunfilesMapping::Data => provider.get_data_runfiles().clone(),
        },
        None => Runfiles::empty(),
    }
}

/// Java `Runfiles.Builder.getPrerequisites(ruleContext, attributeName)`.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:795 Runfiles.Builder.getPrerequisites
fn runfiles_get_prerequisites(
    rule_context: &RuleContext,
    attribute_name: &str,
) -> Vec<ConfiguredTarget> {
    let defined = rule_context
        .get_rule()
        .get_attribute_definition(attribute_name)
        .is_some_and(|a| a.get_type() == Type::LabelList);
    if defined {
        rule_context.get_prerequisites(attribute_name)
    } else {
        Vec::new()
    }
}

/// Java `Runfiles.Builder.addRunfiles(ruleContext, mapping)`.
// PORT: move to blaze_actions' RunfilesBuilder::add_runfiles once it can see RuleContext.
// Java: com/google/devtools/build/lib/analysis/Runfiles.java:669 Runfiles.Builder.addRunfiles
fn add_runfiles(
    builder: &mut RunfilesBuilder,
    rule_context: &RuleContext,
    mapping: RunfilesMapping,
) {
    // Java: addDataDeps(ruleContext)
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:699 Runfiles.Builder.addDataDeps
    let always_include_files_to_build_in_data =
        super::starlark_configuration::core_options(rule_context.get_configuration())
            .always_include_files_to_build_in_data;
    for target in runfiles_get_prerequisites(rule_context, "data") {
        // Java: addTarget(target, DATA_RUNFILES, alwaysIncludeFilesToBuildInData) =
        // addTargetIncludingFileTargets
        // Java: com/google/devtools/build/lib/analysis/Runfiles.java:742 Runfiles.Builder.addTargetIncludingFileTargets
        if target.get_provider::<RunfilesProvider>().is_none() {
            // RuleConfiguredTarget implements RunfilesProvider, so this will only be called on
            // FileConfiguredTarget instances.
            builder.add_transitive_artifacts(
                target
                    .get_provider::<FileProvider>()
                    .expect("FileProvider")
                    .get_files_to_build(),
            );
            continue;
        }
        if always_include_files_to_build_in_data {
            // Ensure that `DefaultInfo.files` of Starlark rules is merged in so that native rules
            // interoperate well with idiomatic Starlark rules..
            // https://bazel.build/extending/rules#runfiles_features_to_avoid
            // Internal tests fail if the order of filesToBuild is preserved.
            let mut b = blaze_collect::NestedSetBuilder::stable_order();
            b.add_transitive(
                target
                    .get_provider::<FileProvider>()
                    .expect("FileProvider")
                    .get_files_to_build(),
            );
            builder.add_transitive_artifacts(&b.build());
        }
        builder.merge(&apply_mapping(RunfilesMapping::Data, &target));
    }
    // Java: addNonDataDeps(ruleContext, mapping)
    // Java: com/google/devtools/build/lib/analysis/Runfiles.java:707 Runfiles.Builder.addNonDataDeps
    let non_data_deps = runfiles_get_prerequisites(rule_context, "srcs")
        .into_iter()
        .chain(runfiles_get_prerequisites(rule_context, "deps"));
    for target in non_data_deps {
        // Java: addTargetExceptFileTargets(target, mapping)
        builder.merge(&apply_mapping(mapping, &target));
    }
}

/// `isNonEmptyDict(o)`.
fn is_non_empty_dict(o: Value) -> bool {
    DictRef::from_value(o).is_some_and(|d| !d.is_empty())
}

/// `isNonEmptyDepset(o)`.
fn is_non_empty_depset(o: Value) -> bool {
    Depset::from_value(o).is_some_and(|d| !d.is_empty())
}

/// `((Depset) x).getSet(SymlinkEntry.class)`.
fn symlink_entries(
    x: Value,
) -> starlark::Result<blaze_collect::NestedSet<blaze_actions::analysis::symlink_entry::SymlinkEntry>>
{
    let d = Depset::from_value(x).expect("depset");
    if !d.is_empty() && d.element_type() != "SymlinkEntry" {
        return Err(errorf(format!(
            "got a depset of '{}', expected a depset of 'SymlinkEntry'",
            d.element_type()
        )));
    }
    // Java: com/google/devtools/build/lib/collect/nestedset/Depset.java:207 Depset.getSet
    // (the depset's one NestedSet, see rule::depsets).
    crate::rule::depsets::depset_to_nested_set_shared(x, d, &mut |v: Value| {
        symlink_entry_from_value(v).ok_or_else(|| errorf("depset element is not a SymlinkEntry"))
    })
}

/// `Dict.cast(x, String.class, Artifact.class, what)`.
fn dict_cast_string_artifact(x: Value, what: &str) -> starlark::Result<Vec<(String, Artifact)>> {
    let d = DictRef::from_value(x).expect("dict");
    let mut result = Vec::new();
    for (k, v) in d.iter() {
        match (k.unpack_str(), artifact_from_value(v)) {
            (Some(k), Some(a)) => result.push((k.to_owned(), a)),
            _ => {
                return Err(errorf(format!(
                    "got dict<{}, {}> for '{what}', want dict<string, File>",
                    k.get_type(),
                    v.get_type()
                )));
            }
        }
    }
    Ok(result)
}

/// `StarlarkRuleContext.makeLabelMap(knownLabels, locationsPrefersExecutable)`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1281 StarlarkRuleContext.makeLabelMap
pub fn make_label_map(
    known_labels: &[ConfiguredTarget],
    locations_prefers_executable: bool,
) -> starlark::Result<IndexMap<Label, Vec<Artifact>>> {
    let mut targets_map: IndexMap<Label, Vec<Artifact>> = IndexMap::new();
    for current in known_labels {
        let label = AliasProvider::get_dependency_label(current);
        if targets_map.contains_key(&label) {
            return Err(errorf(format!(
                "Label {} is found more than once in 'targets' list.",
                blaze_starlark::printer::quote(&label.to_string())
            )));
        }

        let files_to_run_provider = current.get_provider::<FilesToRunProvider>();
        let files_to_build = current
            .get_provider::<FileProvider>()
            .expect("FileProvider")
            .get_files_to_build();
        // Only use the executable if requesting the files to build via a singleton location
        // function would fail to ensure backwards compatibility (this function used to always
        // return the files to build). The case of a plural location function is potentially
        // breaking, but gated behind locationsPrefersExecutable.
        // Avoid flattening a nested set if the first branch is taken.
        let expansion = match files_to_run_provider.and_then(|p| p.get_executable()) {
            Some(executable) if locations_prefers_executable && !files_to_build.is_singleton() => {
                vec![executable.clone()]
            }
            _ => files_to_build.to_list().to_vec(),
        };
        targets_map.insert(label, expansion);
    }
    Ok(targets_map)
}

/// Converts a Starlark `build_setting_value` (Java `Object`) to a Starlark value, as the
/// interpreter converts a `@StarlarkMethod` result (`Starlark.fromJava`): a `java.util.List`
/// becomes a new mutable list, a `java.util.Set` a new mutable set; a `StarlarkList` or `Tuple`
/// stored by a transition is returned as it is (frozen with the transition's thread).
// Java: net/starlark/java/eval/Starlark.java:214 Starlark.fromJava
fn starlark_option_to_value<'v>(v: &StarlarkOptionValue, heap: Heap<'v>) -> Value<'v> {
    match v {
        StarlarkOptionValue::Bool(b) => Value::new_bool(*b),
        StarlarkOptionValue::Int(i) => heap.alloc(*i),
        StarlarkOptionValue::Str(s) => heap.alloc_str(s).to_value(),
        StarlarkOptionValue::Label(l) => heap.alloc(LabelValue(l.clone())),
        // PORT: Starlark.fromJava rejects a FeatureFlagValue (not a Starlark value); not reached
        // by any ported caller.
        StarlarkOptionValue::FeatureFlag(v) => {
            blaze_util::unported!("Starlark.fromJava", "FeatureFlagValue {}", v)
        }
        StarlarkOptionValue::List(items) => {
            let elems: Vec<Value<'v>> = items
                .iter()
                .map(|i| starlark_option_to_value(i, heap))
                .collect();
            match items.class() {
                JavaCollectionClass::List => heap.alloc(AllocList(elems)),
                JavaCollectionClass::StarlarkList => super::util::alloc_immutable_list(heap, elems),
                JavaCollectionClass::Tuple => {
                    heap.alloc(starlark::values::tuple::AllocTuple(elems))
                }
                JavaCollectionClass::Set | JavaCollectionClass::StarlarkSet => {
                    starlark::values::set::alloc_set(heap, elems)
                        .expect("build setting values are hashable")
                }
            }
        }
    }
}

/// The targets of a Starlark sequence of `Target`s (Java `Sequence.cast(targets,
/// TransitiveInfoCollection.class, what)`).
fn sequence_cast_targets<'v>(
    x: Value<'v>,
    what: &str,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Vec<ConfiguredTarget>> {
    let mut result = Vec::new();
    for (i, elem) in super::util::sequence_elements(x, eval)?
        .into_iter()
        .enumerate()
    {
        match target_from_starlark(elem) {
            Some(t) => result.push(t),
            None => {
                return Err(errorf(format!(
                    "at index {i} of {what}, got element of type {}, want Target",
                    elem.get_type()
                )));
            }
        }
    }
    Ok(result)
}

#[starlark_module]
fn ctx_methods(builder: &mut MethodsBuilder) {
    /// Contains methods for declaring output files and the actions that produce them.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:602 StarlarkRuleContext.actions
    #[starlark(attribute)]
    fn actions<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live_unchecked(this, "actions")?;
        // ruleContext will be null when this StarlarkRuleContext is frozen. Accessing
        // ctx.actions when frozen will throw other errors, so just ignore this for materializer
        // rules.
        if ctx
            .context
            .handle
            .borrow()
            .get_rule()
            .get_rule_class_object()
            .is_materializer_rule()
        {
            return Err(errorf("ctx.actions is not available in materializer rules"));
        }
        Ok(ctx.actions)
    }

    /// For rules with _skylark_testable set to True, this returns an Actions provider.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:668 StarlarkRuleContext.createdActions
    fn created_actions<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "created_actions")?;
        if ctx
            .context
            .handle
            .borrow()
            .get_rule()
            .get_rule_class_object()
            .is_starlark_testable()
        {
            blaze_util::unported!("ActionsProvider.create");
        }
        Ok(Value::new_none())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:678 StarlarkRuleContext.getAttr
    #[starlark(attribute)]
    fn attr<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "attr")?.attr)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:684 StarlarkRuleContext.getSplitAttr
    #[starlark(attribute)]
    fn split_attr<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "split_attr")?;
        match ctx.split_attr {
            Some(v) => Ok(v),
            None => Err(errorf(
                "'split_attr' is available only in rule implementations",
            )),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:694 StarlarkRuleContext.getExecutable
    #[starlark(attribute)]
    fn executable<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "executable")?.executable)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:701 StarlarkRuleContext.getFile
    #[starlark(attribute)]
    fn file<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "file")?.file)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:708 StarlarkRuleContext.getFiles
    #[starlark(attribute)]
    fn files<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "files")?.files)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:714 StarlarkRuleContext.getWorkspaceName
    #[starlark(attribute)]
    fn workspace_name<'v>(this: Value<'v>) -> starlark::Result<String> {
        let ctx = live(this, "workspace_name")?;
        Ok(ctx.context.handle.borrow().get_workspace_name().to_owned())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:720 StarlarkRuleContext.getLabel
    #[starlark(attribute)]
    fn label<'v>(this: Value<'v>) -> starlark::Result<LabelValue> {
        let ctx = live(this, "label")?;
        Ok(LabelValue(ctx.context.handle.borrow().get_label().clone()))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:726 StarlarkRuleContext.getFragments
    #[starlark(attribute)]
    fn fragments<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live(this, "fragments")?.fragments)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:732 StarlarkRuleContext.getConfiguration
    #[starlark(attribute)]
    fn configuration<'v>(this: Value<'v>) -> starlark::Result<StarlarkConfiguration> {
        let ctx = live(this, "configuration")?;
        Ok(StarlarkConfiguration(
            ctx.context.handle.borrow().get_configuration().clone(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:738 StarlarkRuleContext.getBuildSettingValue
    #[starlark(attribute)]
    fn build_setting_value<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live_unchecked(this, "build_setting_value")?;
        let rc = ctx.context.handle.borrow();
        let rule_class = rc.get_rule().get_rule_class_object().clone();
        let Some(build_setting) = rule_class.get_build_setting() else {
            return Err(errorf(format!(
                "attempting to access 'build_setting_value' of non-build setting {}",
                ctx.context.rule_label_canonical_name
            )));
        };
        let starlark_flag_settings = rc.get_configuration().get_options().get_starlark_options();
        if let Some(v) = starlark_flag_settings.get(rc.get_label()) {
            return Ok(starlark_option_to_value(v, heap));
        }
        let default_value = rc.attributes().get(
            STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME,
            build_setting.get_type(),
        );
        let default_value = blaze_packages::attribute::attribute::Attribute::value_to_starlark(
            default_value.as_ref(),
            heap,
        );
        Ok(if build_setting.allows_multiple() {
            heap.alloc(AllocList([default_value]))
        } else {
            default_value
        })
    }

    /// Returns true if code coverage instrumentation should be generated when performing
    /// compilation actions for this rule or, if `target` is provided, the rule specified by that
    /// Target.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:760 StarlarkRuleContext.instrumentCoverage
    fn coverage_instrumented<'v>(
        this: Value<'v>,
        #[starlark(default = NoneType)] target: Value<'v>,
    ) -> starlark::Result<bool> {
        let ctx = live(this, "coverage_instrumented")?;
        let rc = ctx.context.handle.borrow();
        let config = rc.get_configuration();
        if !is_code_coverage_enabled(config) {
            return Ok(false);
        }
        if target.is_none() {
            return Ok(
                crate::test::instrumented_files_collector::should_include_local_sources(
                    config,
                    rc.get_label(),
                    rc.is_test_target(),
                ),
            );
        }
        let Some(target) = target_from_starlark(target) else {
            return Err(errorf(format!(
                "in call to coverage_instrumented(), parameter 'target' got value of type '{}', \
                 want 'Target or NoneType'",
                target.get_type()
            )));
        };
        Ok(target
            .get(&blaze_packages::starlark_defs::builtin_providers::INSTRUMENTED_FILES_INFO.key())
            .is_some()
            && crate::test::instrumented_files_collector::should_include_local_sources_for_target(
                config, &target,
            ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:775 StarlarkRuleContext.getFeatures
    // Java returns an `ImmutableList<String>`, which the interpreter converts with
    // `Starlark.fromJava(result, thread.mutability())` (net/starlark/java/eval/MethodDescriptor.java:315,
    // Starlark.java:214 `StarlarkList.copyOf(mutability, list)`): a new list on each access that
    // the rule may mutate (oracle: scripts/w4/scenarios/ctx-immutable, features_append). The
    // same holds for `disabled_features`, `aspect_ids` and `build_setting_value`.
    #[starlark(attribute)]
    fn features<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "features")?;
        let rc = ctx.context.handle.borrow();
        Ok(heap.alloc(AllocList(
            rc.get_features()
                .iter()
                .map(|f| heap.alloc_str(f).to_value()),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:781 StarlarkRuleContext.getDisabledFeatures
    #[starlark(attribute)]
    fn disabled_features<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "disabled_features")?;
        let rc = ctx.context.handle.borrow();
        Ok(heap.alloc(AllocList(
            rc.get_disabled_features()
                .iter()
                .map(|f| heap.alloc_str(f).to_value()),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:787 StarlarkRuleContext.getBinDirectory
    #[starlark(attribute)]
    fn bin_dir<'v>(this: Value<'v>) -> starlark::Result<StarlarkFileRoot> {
        let ctx = live(this, "bin_dir")?;
        let rc = ctx.context.handle.borrow();
        let root = rc
            .get_configuration()
            .get_bin_directory(rc.get_rule().get_repository());
        Ok(StarlarkFileRoot(FileRoot::Artifact(
            crate::rule::artifact_roots::to_actions_root(&root),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:793 StarlarkRuleContext.getGenfilesDirectory
    #[starlark(attribute)]
    fn genfiles_dir<'v>(this: Value<'v>) -> starlark::Result<StarlarkFileRoot> {
        let ctx = live(this, "genfiles_dir")?;
        let rc = ctx.context.handle.borrow();
        let root = rc
            .get_configuration()
            .get_genfiles_directory(rc.get_rule().get_repository());
        Ok(StarlarkFileRoot(FileRoot::Artifact(
            crate::rule::artifact_roots::to_actions_root(&root),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:799 StarlarkRuleContext.outputs
    #[starlark(attribute)]
    fn outputs<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "outputs")?;
        match ctx.outputs {
            Some(v) => Ok(v),
            None => Err(errorf("'outputs' is not defined")),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:808 StarlarkRuleContext.rule
    #[starlark(attribute)]
    fn rule<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "rule")?;
        if !ctx.is_for_aspect() {
            return Err(errorf("'rule' is only available in aspect implementations"));
        }
        Ok(ctx
            .rule_attributes_collection
            .expect("an aspect context has a rule attributes collection"))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:817 StarlarkRuleContext.aspectIds
    #[starlark(attribute)]
    fn aspect_ids<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "aspect_ids")?;
        if !ctx.is_for_aspect() {
            return Err(errorf(
                "'aspect_ids' is only available in aspect implementations",
            ));
        }
        let rc = ctx.context.handle.borrow();
        Ok(heap.alloc(AllocList(
            rc.get_aspect_descriptors()
                .iter()
                .map(|d| heap.alloc_str(&d.get_description()).to_value()),
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:831 StarlarkRuleContext.var
    #[starlark(attribute)]
    fn var<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        get_var(this, heap)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:802 StarlarkRuleContext.toolchains
    #[starlark(attribute)]
    fn toolchains<'v>(this: Value<'v>) -> starlark::Result<StarlarkToolchainContext> {
        let ctx = live(this, "toolchains")?;
        let rc = ctx.context.handle.borrow();
        let Some(toolchain_context) = rc.get_toolchain_context() else {
            return Ok(StarlarkToolchainContext::NotValid);
        };
        if rc.use_auto_exec_groups() {
            Ok(StarlarkToolchainContext::create(
                /* target_description= */ toolchain_context.target_description().to_owned(),
                /* resolve_toolchain_data_func= */ &|l| rc.get_toolchain_info(l),
                /* resolved_toolchain_type_labels= */
                StarlarkRuleContext::get_requested_toolchain_type_labels_from_auto_exec_groups(&rc),
            ))
        } else {
            Ok(StarlarkToolchainContext::of_resolved(toolchain_context))
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:831 StarlarkRuleContext.targetPlatformHasConstraint
    fn target_platform_has_constraint<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] constraint_value: &ConstraintValueInfo,
    ) -> starlark::Result<bool> {
        let ctx = live_unchecked(this, "target_platform_has_constraint")?;
        Ok(ctx
            .context
            .handle
            .borrow()
            .target_platform_has_constraint(constraint_value))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:836 StarlarkRuleContext.execGroups
    #[starlark(attribute)]
    fn exec_groups<'v>(this: Value<'v>) -> starlark::Result<StarlarkExecGroupCollection> {
        let ctx = live_unchecked(this, "exec_groups")?;
        let rc = ctx.context.handle.borrow();
        // Create a thin wrapper around the toolchain collection, to expose the Starlark API.
        match rc.get_toolchain_contexts() {
            Some(toolchain_contexts) => Ok(StarlarkExecGroupCollection::create(toolchain_contexts)),
            None => blaze_util::unported!(
                "StarlarkExecGroupCollection.create",
                "rule context without toolchain contexts"
            ),
        }
    }

    /// Splits a shell command into a list of tokens.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:847 StarlarkRuleContext.tokenize
    fn tokenize<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] option: &str,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let _ctx = live(this, "tokenize")?;
        let mut options: Vec<String> = Vec::new();
        if let Err(e) = ShellUtils::tokenize(&mut options, option) {
            return Err(errorf(format!(
                "{} while tokenizing '{}'",
                e.get_message(),
                option
            )));
        }
        // Java: StarlarkList.immutableCopyOf(options)
        Ok(alloc_immutable_list(
            heap,
            options
                .iter()
                .map(|o| heap.alloc_str(o).to_value())
                .collect(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:863 StarlarkRuleContext.checkPlaceholders
    fn check_placeholders<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] template: &str,
        #[starlark(require = pos)] allowed_placeholders: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ctx = live(this, "check_placeholders")?;
        let mut actual_place_holders: Vec<String> = Vec::new();
        let allowed_placeholder_set: IndexSet<String> =
            sequence_cast_strings(allowed_placeholders, "allowed_placeholders", eval)?
                .into_iter()
                .collect();
        ImplicitOutputsFunction::create_placeholder_substitution_format_string(
            template,
            &mut actual_place_holders,
        );
        for placeholder in &actual_place_holders {
            if !allowed_placeholder_set.contains(placeholder) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// **Deprecated.** Use `ctx.var` to access the variables instead.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:879 StarlarkRuleContext.expandMakeVariables
    fn expand_make_variables<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] attribute_name: &str,
        #[starlark(require = pos)] command: &str,
        #[starlark(require = pos)] additional_substitutions: Value<'v>,
    ) -> starlark::Result<String> {
        let ctx = live(this, "expand_make_variables")?;
        let additional_substitutions_map = super::util::dict_cast_string_string(
            additional_substitutions,
            "additional_substitutions",
        )?;
        Ok(ctx.expand_make_variables_with(attribute_name, command, additional_substitutions_map))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:995 StarlarkRuleContext.getStableWorkspaceStatus
    #[starlark(attribute)]
    fn info_file<'v>(this: Value<'v>) -> starlark::Result<StarlarkFile> {
        let ctx = live(this, "info_file")?;
        Ok(StarlarkFile::new(
            ctx.context
                .handle
                .borrow()
                .get_analysis_environment()
                .get_stable_workspace_status_artifact()
                .expect("MissingDepException: workspace status"),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1001 StarlarkRuleContext.getVolatileWorkspaceStatus
    #[starlark(attribute)]
    fn version_file<'v>(this: Value<'v>) -> starlark::Result<StarlarkFile> {
        let ctx = live(this, "version_file")?;
        Ok(StarlarkFile::new(
            ctx.context
                .handle
                .borrow()
                .get_analysis_environment()
                .get_volatile_workspace_status_artifact()
                .expect("MissingDepException: workspace status"),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1007 StarlarkRuleContext.getBuildFileRelativePath
    #[starlark(attribute)]
    fn build_file_path<'v>(this: Value<'v>) -> starlark::Result<String> {
        let ctx = live(this, "build_file_path")?;
        // Java: checkDeprecated("ctx.label.package + '/BUILD'", "ctx.build_file_path", semantics)
        if ctx
            .context
            .get_starlark_semantics()
            .incompatible_stop_exporting_build_file_path
        {
            return Err(errorf(
                "Use ctx.label.package + '/BUILD' instead of ctx.build_file_path.\nUse \
                 --incompatible_stop_exporting_build_file_path=false to temporarily disable this \
                 check.",
            ));
        }
        // Package pkg = ruleContext.getRule().getPackage();
        // return pkg.getSourceRoot().get().relativize(pkg.getBuildFile().asPath()).getPathString();
        let handle = ctx.context.handle.borrow();
        let metadata = handle.get_rule().get_package_metadata();
        Ok(metadata
            .source_root
            .relativize(&metadata.build_filename.as_path())
            .get_path_string()
            .to_owned())
    }

    /// Expands all `$(location ...)` templates in the given string.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1029 StarlarkRuleContext.expandLocation
    fn expand_location<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] input: &str,
        #[starlark(default = AllocList::EMPTY)] targets: Value<'v>,
        #[starlark(require = named, default = false)] short_paths: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        let ctx = live(this, "expand_location")?;
        if !is_sequence(targets) {
            return Err(errorf(format!(
                "in call to expand_location(), parameter 'targets' got value of type '{}', want \
                 'sequence'",
                targets.get_type()
            )));
        }
        let prefers_executable = ctx
            .context
            .get_starlark_semantics()
            .incompatible_locations_prefers_executable;
        let label_map = make_label_map(
            &sequence_cast_targets(targets, "targets", eval)?,
            prefers_executable,
        )?;
        let rc = ctx.context.handle.borrow();
        let expander = if !short_paths {
            LocationExpander::with_exec_paths(&rc, label_map)
        } else {
            fail_if_called_outside_default_allowlist(eval)?;
            LocationExpander::with_runfiles_paths(&rc, Some(label_map))
        };
        // PORT: Java's `catch (IllegalStateException ise) { throw new EvalException(ise); }`
        // has no counterpart; the Rust expander reports its errors as rule errors.
        Ok(expander.expand(input))
    }

    /// Creates a runfiles object.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1058 StarlarkRuleContext.runfiles
    fn runfiles<'v>(
        this: Value<'v>,
        #[starlark(default = AllocList::EMPTY)] files: Value<'v>,
        #[starlark(default = NoneType)] transitive_files: Value<'v>,
        #[starlark(default = false)] collect_data: Value<'v>,
        #[starlark(default = false)] collect_default: Value<'v>,
        #[starlark(default = AllocDict::EMPTY)] symlinks: Value<'v>,
        #[starlark(default = AllocDict::EMPTY)] root_symlinks: Value<'v>,
        #[starlark(require = named, default = false)] skip_conflict_checking: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkRunfiles> {
        // The interpreter checks every argument against its `@Param` before the method runs
        // (net/starlark/java/eval/BuiltinFunction.java:405), in declaration order.
        // Java: com/google/devtools/build/lib/starlarkbuildapi/StarlarkRuleContextApi.java:469 StarlarkRuleContextApi.runfiles
        if !is_sequence(files) {
            return Err(param_type_error("runfiles", "files", files, "sequence"));
        }
        if !(transitive_files.is_none() || Depset::from_value(transitive_files).is_some()) {
            return Err(param_type_error(
                "runfiles",
                "transitive_files",
                transitive_files,
                "depset or NoneType",
            ));
        }
        let collect_data = check_bool_param("runfiles", "collect_data", collect_data)?;
        let collect_default = check_bool_param("runfiles", "collect_default", collect_default)?;
        for (name, value) in [("symlinks", symlinks), ("root_symlinks", root_symlinks)] {
            if !(DictRef::from_value(value).is_some() || Depset::from_value(value).is_some()) {
                return Err(param_type_error("runfiles", name, value, "dict or depset"));
            }
        }

        if skip_conflict_checking {
            fail_if_called_outside_default_allowlist(eval)?;
        }
        let ctx = live(this, "runfiles")?;
        let rc = ctx.context.handle.borrow();
        let mut builder = RunfilesBuilder::new(rc.get_workspace_name());
        let mut check_conflicts = false;
        // Java: Starlark.truth(collectData)
        if collect_data {
            add_runfiles(&mut builder, &rc, RunfilesMapping::Data);
        }
        if collect_default {
            add_runfiles(&mut builder, &rc, RunfilesMapping::Default);
        }
        let files_list = sequence_cast_artifacts(files, "files", eval)?;
        if !files_list.is_empty() {
            for artifact in &files_list {
                if artifact.is_runfiles_tree() {
                    // Java: Preconditions.checkArgument in Runfiles.Builder.addArtifact
                    return Err(errorf(format!(
                        "could not add all 'files': unexpected runfiles tree artifact: {artifact}"
                    )));
                }
            }
            builder.add_artifacts(files_list);
        }
        if !transitive_files.is_none() {
            let transitive_artifacts = depset_cast_artifacts(transitive_files, "transitive_files")?;

            // Runfiles uses compile order. Check that the given transitive_files depset is
            // compatible.
            if !Order::COMPILE_ORDER.is_compatible(transitive_artifacts.get_order()) {
                return Err(errorf(format!(
                    "order '{}' is invalid for transitive_files",
                    transitive_artifacts.get_order().get_starlark_name()
                )));
            }
            builder.add_transitive_artifacts(&transitive_artifacts);
        }
        if is_non_empty_depset(symlinks) {
            // If Starlark code directly manipulates symlinks, activate more stringent validity
            // checking.
            check_conflicts = true;
            builder.add_symlinks(&symlink_entries(symlinks)?);
        } else if is_non_empty_dict(symlinks) {
            check_conflicts = true;
            for (k, v) in dict_cast_string_artifact(symlinks, "symlinks")? {
                builder.add_symlink(PathFragment::create(&k), v);
            }
        }
        if is_non_empty_depset(root_symlinks) {
            check_conflicts = true;
            builder.add_root_symlinks(&symlink_entries(root_symlinks)?);
        } else if is_non_empty_dict(root_symlinks) {
            check_conflicts = true;
            for (k, v) in dict_cast_string_artifact(root_symlinks, "root_symlinks")? {
                builder.add_root_symlink(PathFragment::create(&k), v);
            }
        }
        let mut runfiles = builder.build();
        if check_conflicts && !skip_conflict_checking {
            runfiles = runfiles.set_conflict_policy(ConflictPolicy::ERROR);
        }
        Ok(StarlarkRunfiles(runfiles))
    }

    /// *(Experimental)* Returns a tuple `(inputs, command, empty list)` of the list of resolved
    /// inputs and the argv list for the resolved command.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1140 StarlarkRuleContext.resolveCommand
    fn resolve_command<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = "")] command: &str,
        #[starlark(require = named, default = NoneType)] attribute: Value<'v>,
        #[starlark(require = named, default = false)] expand_locations: bool,
        #[starlark(require = named, default = NoneType)] make_variables: Value<'v>,
        #[starlark(require = named, default = AllocList::EMPTY)] tools: Value<'v>,
        #[starlark(require = named, default = AllocDict::EMPTY)] label_dict: Value<'v>,
        #[starlark(require = named, default = AllocDict::EMPTY)] execution_requirements: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "resolve_command")?;
        let _ = (
            ctx,
            command,
            attribute,
            expand_locations,
            make_variables,
            tools,
            label_dict,
            execution_requirements,
            &ctx.resolve_command_script_counter,
        );
        // Java: CommandHelper.builder(ruleContext)...buildCommandLine(...)
        blaze_util::unported!("StarlarkRuleContext.resolveCommand")
    }

    /// Returns a tuple `(inputs, empty list)` of the depset of resolved inputs required to run
    /// the tools.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1200 StarlarkRuleContext.resolveTools
    fn resolve_tools<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = AllocList::EMPTY)] tools: Value<'v>,
    ) -> starlark::Result<Value<'v>> {
        let ctx = live(this, "resolve_tools")?;
        // Java: checkResolveToolsAllowed()
        if ctx
            .context
            .get_starlark_semantics()
            .incompatible_disallow_ctx_resolve_tools
        {
            return Err(errorf(
                "Pass an executable or tools argument to ctx.actions.run or ctx.actions.run_shell \
                 instead of calling ctx.resolve_tools.\nUse \
                 --noincompatible_disallow_ctx_resolve_tools to temporarily disable this check.",
            ));
        }
        let _ = tools;
        // Java: CommandHelper.builder(ruleContext).addToolDependencies(...).getResolvedTools()
        blaze_util::unported!("StarlarkRuleContext.resolveTools")
    }

    /// Converts the input string into a Label object, in the context of the package where the
    /// target being analyzed is defined.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:1226 StarlarkRuleContext.packageRelativeLabel
    fn package_relative_label<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] input: Value<'v>,
    ) -> starlark::Result<LabelValue> {
        let ctx = live(this, "package_relative_label")?;
        if let Some(label) = input.downcast_ref::<LabelValue>() {
            return Ok(label.clone());
        }
        let Some(s) = input.unpack_str() else {
            return Err(errorf(format!(
                "in call to package_relative_label(), parameter 'input' got value of type '{}', \
                 want 'string or Label'",
                input.get_type()
            )));
        };
        let rc = ctx.context.handle.borrow();
        let package_context = LabelPackageContext::of(
            rc.get_label().get_package_identifier(),
            rc.get_rule()
                .get_package_metadata()
                .repository_mapping
                .clone(),
        );
        Label::parse_with_package_context(s, &package_context)
            .map(LabelValue)
            .map_err(|e| {
                errorf(format!(
                    "invalid label in ctx.package_relative_label: {}",
                    e.get_message()
                ))
            })
    }
}

/// Java `StarlarkRuleContext.getStarlarkSemantics()`.
pub fn get_starlark_semantics(ctx: &StarlarkRuleContext) -> Arc<blaze_starlark::StarlarkSemantics> {
    ctx.context.get_starlark_semantics()
}

/// `ctx.var` (Java `starlarkRuleContext.var()`), also for natives: the make variables as an
/// immutable dict, computed once.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:831 StarlarkRuleContext.var
pub fn get_var<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
    let ctx = live(this, "var")?;
    if let Some(v) = ctx.cached_make_variables.get() {
        return Ok(v);
    }
    let vars = {
        let rc = ctx.context.handle.borrow_mut();
        if rc.should_include_required_config_fragments_provider() {
            // When tracking required fragments, use a key-tracking dict to support
            // lookedUpVariables().
            blaze_util::unported!("Dict.Builder.buildImmutableWithKeyTracking");
        }
        rc.get_configuration_make_variable_context()
            .collect_make_variables()
            .map_err(|e| errorf(e.0))?
    };
    // Java: vars.buildImmutable()
    let dict = alloc_immutable_dict(
        heap,
        vars.iter()
            .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
            .collect(),
    );
    ctx.cached_make_variables.set(Some(dict));
    Ok(dict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_context_repr() {
        let c = ClosedStarlarkRuleContext {
            rule_label_canonical_name: "//foo:bar".to_owned(),
            is_for_aspect: false,
        };
        assert_eq!(c.to_string(), "<rule context for //foo:bar>");
        let o = ClosedCtxOutputs {
            rule_label_canonical_name: "//foo:bar".to_owned(),
        };
        assert_eq!(o.to_string(), "ctx.outputs(for //foo:bar)");
    }

    // Expected text: the oracle (Bazel 9.3.2) on scripts/w4/scenarios/ctx-errors, target
    // outputs_unknown.
    #[test]
    fn outputs_unknown_field_message() {
        assert_eq!(
            outputs_error_message_for_unknown_field("nosuch"),
            "No attribute 'nosuch' in outputs. Make sure you declared a rule output with this \
             name."
        );
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleContext.java:441 Outputs.checkMutable
    // (`getValue` and so `getattr` with a default and `hasattr` fail on a closed ctx.outputs).
    #[test]
    fn closed_outputs_report_check_mutable() {
        let o = ClosedCtxOutputs {
            rule_label_canonical_name: "//foo:bar".to_owned(),
        };
        starlark::environment::Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            let expected = "cannot access outputs of rule '//foo:bar' outside of its own rule \
                            implementation function";
            let e = o
                .get_attr_fallible(Hashed::new("out"), &mut eval)
                .unwrap_err();
            assert_eq!(e.to_string(), expected);
            let e = o.has_attr_fallible("out", &mut eval).unwrap_err();
            assert_eq!(e.to_string(), expected);
        });
    }
}
