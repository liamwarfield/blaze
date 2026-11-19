//! `ctx.toolchains` and `ctx.exec_groups`: ports of
//! `analysis/starlark/StarlarkToolchainContext.java` and
//! `analysis/starlark/StarlarkExecGroupCollection.java`.
//!
//! PORT: Java's `StarlarkToolchainContext` holds a function that resolves a toolchain type to
//! its `ToolchainInfo` on each index; the lookup is pure, so the values are resolved when the
//! context is created and held as frozen values.
//!
//! Indexing with a string converts it with `LabelConverter.forBzlEvaluatingThread`
//! (StarlarkToolchainContext.java:106): the package context of the innermost .bzl on the call
//! stack, read through the evaluator (`at_with_eval` / `is_in_with_eval`, starlark-rust patch
//! 0027).

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_packages::rule_class::declared_exec_group::{
    DEFAULT_EXEC_GROUP_NAME, is_valid_identifier,
};
use blaze_packages::types::label_converter::LabelConverter;
use blaze_starlark::label::LabelValue;
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value};

use super::util::{alloc_immutable_list, errorf};
use crate::platform::toolchain_type_label_from_value;
use crate::toolchains::aspect_base_target_resolved_toolchain_context::AspectBaseTargetResolvedToolchainContext;
use crate::toolchains::{ResolvedToolchainContext, ToolchainCollection};

/// The value of `ctx.toolchains` (type `ToolchainContext`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:40 StarlarkToolchainContext
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub enum StarlarkToolchainContext {
    /// Java `StarlarkToolchainContext.TOOLCHAINS_NOT_VALID`.
    NotValid,
    Valid {
        target_description: String,
        /// Java `resolvedToolchainTypeLabels()`.
        #[allocative(skip)]
        resolved_toolchain_type_labels: IndexSet<Label>,
        /// Java `resolveToolchainFunc().apply(label)` for each resolved label.
        #[allocative(skip)]
        resolved: IndexMap<Label, Option<OwnedFrozenValue>>,
    },
}

starlark_simple_value!(StarlarkToolchainContext);

impl fmt::Debug for StarlarkToolchainContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for StarlarkToolchainContext {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:89 StarlarkToolchainContext.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // PORT: TOOLCHAINS_NOT_VALID is an anonymous class: Java's default repr names it.
            StarlarkToolchainContext::NotValid => f.write_str(
                "<unknown object \
                 com.google.devtools.build.lib.analysis.starlark.StarlarkToolchainContext$1>",
            ),
            StarlarkToolchainContext::Valid {
                resolved_toolchain_type_labels,
                ..
            } => {
                f.write_str("<toolchain_context.resolved_labels: ")?;
                let labels: Vec<String> = resolved_toolchain_type_labels
                    .iter()
                    .map(|l| l.to_string())
                    .collect();
                f.write_str(&labels.join(", "))?;
                f.write_str(">")
            }
        }
    }
}

impl StarlarkToolchainContext {
    /// Java `StarlarkToolchainContext.create(targetDescription, resolveToolchainDataFunc,
    /// resolvedToolchainTypeLabels)`, with the function applied to every label.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:64 StarlarkToolchainContext.create
    pub fn create(
        target_description: String,
        resolve_toolchain_data_func: &dyn Fn(&Label) -> Option<OwnedFrozenValue>,
        resolved_toolchain_type_labels: IndexSet<Label>,
    ) -> StarlarkToolchainContext {
        let resolved = resolved_toolchain_type_labels
            .iter()
            .map(|l| (l.clone(), resolve_toolchain_data_func(l)))
            .collect();
        StarlarkToolchainContext::Valid {
            target_description,
            resolved_toolchain_type_labels,
            resolved,
        }
    }

    /// The Starlark toolchain context of a resolved toolchain context (Java
    /// `StarlarkToolchainContext.create(ctx.targetDescription(), ctx::forToolchainType,
    /// ctx.requestedToolchainTypeLabels().keySet())`).
    pub fn of_resolved(toolchain_context: &ResolvedToolchainContext) -> StarlarkToolchainContext {
        StarlarkToolchainContext::create(
            toolchain_context.target_description().to_owned(),
            &|l| toolchain_context.for_toolchain_type(l),
            toolchain_context
                .requested_toolchain_type_labels()
                .keys()
                .cloned()
                .collect(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:96 StarlarkToolchainContext.transformKey
    fn transform_key(&self, key: Value, eval: Option<&Evaluator>) -> starlark::Result<Label> {
        if let Some(label) = key.downcast_ref::<LabelValue>() {
            return Ok(label.label().clone());
        }
        if let Some(label) = toolchain_type_label_from_value(key) {
            return Ok(label);
        }
        if let Some(s) = key.unpack_str() {
            let Some(eval) = eval else {
                // Only Rust callers index without an evaluator.
                blaze_util::unported!(
                    "StarlarkToolchainContext.transformKey",
                    "string toolchain type '{}' without a Starlark thread",
                    s
                )
            };
            // Java: LabelConverter.forBzlEvaluatingThread(starlarkThread):
            // BazelModuleContext.ofInnermostBzlOrThrow (Preconditions.checkNotNull).
            let module_context = BazelModuleContext::of_innermost_bzl(eval)
                .expect("NullPointerException: BazelModuleContext.ofInnermostBzlOrThrow");
            let converter = LabelConverter::from_package_context(module_context.package_context());
            return converter.convert(s).map_err(|e| {
                errorf(format!(
                    "Unable to parse toolchain label '{s}': {}",
                    e.get_message()
                ))
            });
        }
        Err(errorf(format!(
            "Toolchains only supports indexing by toolchain type, got {} instead",
            key.get_type()
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:138 StarlarkToolchainContext.containsKey
    fn contains_key(&self, key: Value, eval: Option<&Evaluator>) -> starlark::Result<bool> {
        match self {
            // Java: TOOLCHAINS_NOT_VALID.containsKey
            StarlarkToolchainContext::NotValid => Ok(false),
            StarlarkToolchainContext::Valid {
                resolved_toolchain_type_labels,
                ..
            } => {
                let toolchain_type_label = self.transform_key(key, eval)?;
                Ok(resolved_toolchain_type_labels.contains(&toolchain_type_label))
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:116 StarlarkToolchainContext.getIndex
    fn get_index<'v>(
        &self,
        key: Value<'v>,
        heap: Heap<'v>,
        eval: Option<&Evaluator>,
    ) -> starlark::Result<Value<'v>> {
        let StarlarkToolchainContext::Valid {
            target_description,
            resolved_toolchain_type_labels,
            resolved,
            ..
        } = self
        else {
            // Java: TOOLCHAINS_NOT_VALID.getIndex
            return Err(errorf("Toolchains are not valid in this context"));
        };
        let toolchain_type_label = self.transform_key(key, eval)?;
        if !self.contains_key(key, eval)? {
            // TODO(bazel-configurability): The list of available toolchain types is confusing in
            // the presence of aliases, since it only contains the actual label, not the alias
            // passed to the rule definition.
            let labels: Vec<String> = resolved_toolchain_type_labels
                .iter()
                .map(|l| l.to_string())
                .collect();
            return Err(errorf(format!(
                "In {}, toolchain type {} was requested but only types [{}] are configured",
                target_description,
                toolchain_type_label,
                labels.join(", ")
            )));
        }
        match resolved.get(&toolchain_type_label).and_then(|v| v.as_ref()) {
            None => Ok(Value::new_none()),
            Some(toolchain_info) => Ok(heap.access_owned_frozen_value(toolchain_info)),
        }
    }
}

#[starlark_value(type = "ToolchainContext")]
impl<'v> StarlarkValue<'v> for StarlarkToolchainContext {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("ToolchainContext", toolchain_context_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        self.get_index(index, heap, None)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:116 StarlarkToolchainContext.getIndex
    // (StarlarkIndexable.Threaded: the thread converts string keys)
    fn at_with_eval(
        &self,
        index: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        self.get_index(index, heap, Some(eval))
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        self.contains_key(other, None)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:138 StarlarkToolchainContext.containsKey
    fn is_in_with_eval(
        &self,
        other: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        self.contains_key(other, Some(eval))
    }
}

#[starlark_module]
fn toolchain_context_methods(builder: &mut MethodsBuilder) {
    /// Returns the resolved toolchain type labels.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:79 StarlarkToolchainContext.toolchainTypes
    fn toolchain_types<'v>(
        this: &StarlarkToolchainContext,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let labels: Vec<Label> = match this {
            StarlarkToolchainContext::NotValid => Vec::new(),
            StarlarkToolchainContext::Valid {
                resolved_toolchain_type_labels,
                ..
            } => resolved_toolchain_type_labels.iter().cloned().collect(),
        };
        // Java: StarlarkList.immutableCopyOf(resolvedToolchainTypeLabels())
        Ok(alloc_immutable_list(
            heap,
            labels
                .into_iter()
                .map(|l| heap.alloc(LabelValue(l)))
                .collect(),
        ))
    }
}

// -------------------------------------------------------------------------------------------
// StarlarkExecGroupCollection

/// The value of `ctx.exec_groups` (type `ExecGroupCollection`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:42 StarlarkExecGroupCollection
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub enum StarlarkExecGroupCollection {
    /// Java `EXEC_GROUP_COLLECTION_NOT_VALID`.
    NotValid,
    Valid {
        /// `toolchainCollection().getExecGroupNames()`, in the collection's order.
        exec_group_names: Vec<String>,
        /// `toolchainCollection().getDefaultToolchainContext().targetDescription()`.
        default_target_description: String,
        /// The toolchain context of each exec group (`None` where
        /// `getToolchainContext(name)` is null).
        #[allocative(skip)]
        contexts: IndexMap<String, Option<std::sync::Arc<StarlarkToolchainContext>>>,
    },
}

starlark_simple_value!(StarlarkExecGroupCollection);

impl fmt::Debug for StarlarkExecGroupCollection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for StarlarkExecGroupCollection {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:128 StarlarkExecGroupCollection.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // PORT: EXEC_GROUP_COLLECTION_NOT_VALID is an anonymous class.
            StarlarkExecGroupCollection::NotValid => f.write_str(
                "<unknown object \
                 com.google.devtools.build.lib.analysis.starlark.StarlarkExecGroupCollection$1>",
            ),
            StarlarkExecGroupCollection::Valid { .. } => {
                write!(
                    f,
                    "<ctx.exec_groups: {}>",
                    self.get_scrubbed_exec_groups().join(", ")
                )
            }
        }
    }
}

/// Java `StarlarkExecGroupCollection.isValidGroupName`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:84 StarlarkExecGroupCollection.isValidGroupName
pub fn is_valid_group_name(exec_group_name: &str) -> bool {
    exec_group_name != DEFAULT_EXEC_GROUP_NAME && is_valid_identifier(exec_group_name)
}

impl StarlarkExecGroupCollection {
    /// Java `StarlarkExecGroupCollection.create(toolchainCollection)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:72 StarlarkExecGroupCollection.create
    pub fn create(
        toolchain_collection: &ToolchainCollection<ResolvedToolchainContext>,
    ) -> StarlarkExecGroupCollection {
        let exec_group_names: Vec<String> = toolchain_collection
            .get_exec_group_names()
            .into_iter()
            .map(|s| s.to_owned())
            .collect();
        let contexts = exec_group_names
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    toolchain_collection
                        .get_toolchain_context(name)
                        .map(|tc| std::sync::Arc::new(StarlarkToolchainContext::of_resolved(tc))),
                )
            })
            .collect();
        StarlarkExecGroupCollection::Valid {
            exec_group_names,
            default_target_description: toolchain_collection
                .get_default_toolchain_context()
                .target_description()
                .to_owned(),
            contexts,
        }
    }

    /// Java `StarlarkExecGroupCollection.create(toolchainCollection)` for the base target's
    /// toolchain contexts of an aspect (`ctx.rule.exec_groups`): each exec group's
    /// `StarlarkToolchainContext` resolves a toolchain type to its `ToolchainAspectsProviders`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:72 StarlarkExecGroupCollection.create
    pub fn create_for_base_target(
        toolchain_collection: &ToolchainCollection<AspectBaseTargetResolvedToolchainContext>,
    ) -> StarlarkExecGroupCollection {
        let exec_group_names: Vec<String> = toolchain_collection
            .get_exec_group_names()
            .into_iter()
            .map(|s| s.to_owned())
            .collect();
        let contexts = exec_group_names
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    toolchain_collection.get_toolchain_context(name).map(|tc| {
                        std::sync::Arc::new(StarlarkToolchainContext::create(
                            tc.target_description().to_owned(),
                            &|l| {
                                tc.for_toolchain_type(l)
                                    .map(super::starlark_attributes_collection::toolchain_aspects_providers_value)
                            },
                            tc.requested_toolchain_type_labels().keys().cloned().collect(),
                        ))
                    }),
                )
            })
            .collect();
        StarlarkExecGroupCollection::Valid {
            exec_group_names,
            default_target_description: toolchain_collection
                .get_default_toolchain_context()
                .target_description()
                .to_owned(),
            contexts,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:135 StarlarkExecGroupCollection.getScrubbedExecGroups
    fn get_scrubbed_exec_groups(&self) -> Vec<String> {
        let StarlarkExecGroupCollection::Valid {
            exec_group_names, ..
        } = self
        else {
            return Vec::new();
        };
        let mut groups: Vec<String> = exec_group_names
            .iter()
            .filter(|g| g.as_str() != DEFAULT_EXEC_GROUP_NAME)
            .cloned()
            .collect();
        groups.sort();
        groups
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:117 StarlarkExecGroupCollection.castGroupName
    fn cast_group_name(key: Value) -> starlark::Result<String> {
        match key.unpack_str() {
            Some(s) => Ok(s.to_owned()),
            None => Err(errorf(format!(
                "exec groups only support indexing by exec group name, got {} of type {} instead",
                key.to_repr(),
                key.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:89 StarlarkExecGroupCollection.containsKey
    fn contains_key(&self, key: Value) -> starlark::Result<bool> {
        match self {
            StarlarkExecGroupCollection::NotValid => Ok(false),
            StarlarkExecGroupCollection::Valid {
                exec_group_names, ..
            } => {
                let group = Self::cast_group_name(key)?;
                Ok(DEFAULT_EXEC_GROUP_NAME != group && exec_group_names.contains(&group))
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:100 StarlarkExecGroupCollection.getIndex
    fn get_index(&self, key: Value) -> starlark::Result<StarlarkExecGroupContext> {
        let StarlarkExecGroupCollection::Valid {
            default_target_description,
            contexts,
            ..
        } = self
        else {
            return Err(errorf("exec_groups are not valid in this context"));
        };
        let exec_group = Self::cast_group_name(key)?;
        if !self.contains_key(key)? {
            return Err(errorf(format!(
                "In {}, unrecognized exec group '{}' requested. Available exec groups: [{}]",
                default_target_description,
                exec_group,
                self.get_scrubbed_exec_groups().join(", ")
            )));
        }
        match contexts.get(&exec_group).and_then(|c| c.clone()) {
            None => Ok(StarlarkExecGroupContext(std::sync::Arc::new(
                StarlarkToolchainContext::NotValid,
            ))),
            Some(toolchains) => Ok(StarlarkExecGroupContext(toolchains)),
        }
    }
}

#[starlark_value(type = "ExecGroupCollection")]
impl<'v> StarlarkValue<'v> for StarlarkExecGroupCollection {
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }

    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(self.get_index(index)?))
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        self.contains_key(other)
    }
}

/// The value of `ctx.exec_groups[name]` (type `ExecGroupContext`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:148 StarlarkExecGroupCollection.StarlarkExecGroupContext
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkExecGroupContext(#[allocative(skip)] std::sync::Arc<StarlarkToolchainContext>);

starlark_simple_value!(StarlarkExecGroupContext);

impl fmt::Debug for StarlarkExecGroupContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StarlarkExecGroupContext")
    }
}

impl fmt::Display for StarlarkExecGroupContext {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:155 StarlarkExecGroupContext.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<exec_group_context>")
    }
}

#[starlark_value(type = "ExecGroupContext")]
impl<'v> StarlarkValue<'v> for StarlarkExecGroupContext {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("ExecGroupContext", exec_group_context_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str("<exec_group_context>");
    }
}

#[starlark_module]
fn exec_group_context_methods(builder: &mut MethodsBuilder) {
    /// Toolchains required for this exec group.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:148 StarlarkExecGroupContext.toolchains
    #[starlark(attribute)]
    fn toolchains<'v>(
        this: &StarlarkExecGroupContext,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(clone_toolchain_context(&this.0)))
    }
}

fn clone_toolchain_context(c: &StarlarkToolchainContext) -> StarlarkToolchainContext {
    match c {
        StarlarkToolchainContext::NotValid => StarlarkToolchainContext::NotValid,
        StarlarkToolchainContext::Valid {
            target_description,
            resolved_toolchain_type_labels,
            resolved,
        } => StarlarkToolchainContext::Valid {
            target_description: target_description.clone(),
            resolved_toolchain_type_labels: resolved_toolchain_type_labels.clone(),
            resolved: resolved.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_group_name_rejects_default_and_non_identifiers() {
        assert!(is_valid_group_name("test"));
        assert!(is_valid_group_name("_foo1"));
        assert!(!is_valid_group_name("default-exec-group"));
        assert!(!is_valid_group_name(
            "@bazel_tools//tools/cpp:toolchain_type"
        ));
        assert!(!is_valid_group_name("1abc"));
        assert!(!is_valid_group_name(""));
    }
}
