//! `ctx.toolchains` and `ctx.exec_groups`: ports of
//! `analysis/starlark/StarlarkToolchainContext.java` and
//! `analysis/starlark/StarlarkExecGroupCollection.java`.
//!
//! PORT: Java's `StarlarkToolchainContext` holds a function that resolves a toolchain type to
//! its `ToolchainInfo` on each index; the lookup is pure, so the values are resolved when the
//! context is created and held as frozen values.
//!
//! PORT: indexing with a string converts it with `LabelConverter.forBzlEvaluatingThread`
//! (StarlarkToolchainContext.java:106), the package context of the innermost .bzl on the call
//! stack. starlark-rust's `at` and `is_in` get no evaluator (no call stack), so the context is
//! fixed when `ctx.toolchains` / `ctx.exec_groups` is read: the .bzl that defines the
//! implementation function being run ([`implementation_package_context`]; the subrule's
//! implementation for a subrule context). That is Java's answer whenever the string key is
//! converted in a function of that .bzl. It differs when the conversion happens in a function of
//! another .bzl with another repository, repository mapping or package (oracle scenario
//! scripts/w4/scenarios/ctx-toolchains-string, step `//:helper`); an exact port needs the
//! evaluator in the index operators of the vendored crate (request in the wave report).

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::{Label, LabelPackageContext};
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_packages::rule_class::declared_exec_group::{
    DEFAULT_EXEC_GROUP_NAME, is_valid_identifier,
};
use blaze_packages::rule_class::rule_class::RuleClass;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_starlark::defining_module::DefModule;
use blaze_starlark::label::LabelValue;
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::{Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value};

use super::util::{alloc_immutable_list, errorf};
use crate::platform::toolchain_type_label_from_value;
use crate::toolchains::{ResolvedToolchainContext, ToolchainCollection};

/// The package context used to convert string toolchain type keys (see the module docs): Java
/// `BazelModuleContext.of(fn.getModule()).packageContext()` of the implementation function of
/// `rule_class`, the innermost .bzl on the call stack while that function's own body runs.
/// `None` when the implementation is not a Starlark function of a .bzl module.
// Java: com/google/devtools/build/lib/packages/LabelConverter.java:41 LabelConverter.forBzlEvaluatingThread
// Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:163 BazelModuleContext.ofInnermostBzlOrThrow
pub(crate) fn implementation_package_context(
    rule_class: &RuleClass,
) -> Option<LabelPackageContext> {
    let implementation = rule_class.get_configured_target_function()?.value();
    // The implementation comes from a frozen .bzl module, whose extra value is its
    // BazelModuleContext (Java `Module.getClientData()`).
    let module = DefModule::of_function(implementation.to_value())?;
    let extra_value = module.frozen_extra_value()?;
    BazelModuleContext::of_module_extra_value(extra_value.to_value())
        .map(|module_context| module_context.package_context())
}

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
        #[allocative(skip)]
        package_context: Option<LabelPackageContext>,
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
        package_context: Option<LabelPackageContext>,
    ) -> StarlarkToolchainContext {
        let resolved = resolved_toolchain_type_labels
            .iter()
            .map(|l| (l.clone(), resolve_toolchain_data_func(l)))
            .collect();
        StarlarkToolchainContext::Valid {
            target_description,
            resolved_toolchain_type_labels,
            resolved,
            package_context,
        }
    }

    /// The Starlark toolchain context of a resolved toolchain context (Java
    /// `StarlarkToolchainContext.create(ctx.targetDescription(), ctx::forToolchainType,
    /// ctx.requestedToolchainTypeLabels().keySet())`).
    pub fn of_resolved(
        toolchain_context: &ResolvedToolchainContext,
        package_context: Option<LabelPackageContext>,
    ) -> StarlarkToolchainContext {
        StarlarkToolchainContext::create(
            toolchain_context.target_description().to_owned(),
            &|l| toolchain_context.for_toolchain_type(l),
            toolchain_context
                .requested_toolchain_type_labels()
                .keys()
                .cloned()
                .collect(),
            package_context,
        )
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:96 StarlarkToolchainContext.transformKey
    fn transform_key(&self, key: Value) -> starlark::Result<Label> {
        if let Some(label) = key.downcast_ref::<LabelValue>() {
            return Ok(label.label().clone());
        }
        if let Some(label) = toolchain_type_label_from_value(key) {
            return Ok(label);
        }
        if let Some(s) = key.unpack_str() {
            let package_context = match self {
                StarlarkToolchainContext::Valid {
                    package_context: Some(pc),
                    ..
                } => pc.clone(),
                _ => blaze_util::unported!(
                    "LabelConverter.forBzlEvaluatingThread",
                    "no package context to convert toolchain type '{}'",
                    s
                ),
            };
            let converter = LabelConverter::from_package_context(package_context);
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
    fn contains_key(&self, key: Value) -> starlark::Result<bool> {
        match self {
            // Java: TOOLCHAINS_NOT_VALID.containsKey
            StarlarkToolchainContext::NotValid => Ok(false),
            StarlarkToolchainContext::Valid {
                resolved_toolchain_type_labels,
                ..
            } => {
                let toolchain_type_label = self.transform_key(key)?;
                Ok(resolved_toolchain_type_labels.contains(&toolchain_type_label))
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkToolchainContext.java:116 StarlarkToolchainContext.getIndex
    fn get_index<'v>(&self, key: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
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
        let toolchain_type_label = self.transform_key(key)?;
        if !self.contains_key(key)? {
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
        self.get_index(index, heap)
    }

    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        self.contains_key(other)
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
        package_context: Option<LabelPackageContext>,
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
                        std::sync::Arc::new(StarlarkToolchainContext::of_resolved(
                            tc,
                            package_context.clone(),
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
            package_context,
        } => StarlarkToolchainContext::Valid {
            target_description: target_description.clone(),
            resolved_toolchain_type_labels: resolved_toolchain_type_labels.clone(),
            resolved: resolved.clone(),
            package_context: package_context.clone(),
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
