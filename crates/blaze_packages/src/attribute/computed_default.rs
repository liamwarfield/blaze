//! Ports the default-computing classes of `packages/Attribute.java`: `ComputedDefault`,
//! `StarlarkComputedDefaultTemplate`, `StarlarkComputedDefault`, `LateBoundDefault`,
//! `LabelLateBoundDefault`, `LabelListLateBoundDefault`, and `packages/MaterializingDefault`
//! (placeholder data).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::EventHandler;

use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::eval::call_frozen;
use blaze_starlark::label::LabelValue;
use blaze_starlark::thread::CollectingEvents;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::list_or_tuple::UnpackListOrTuple;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::rule_class::StarlarkCallable;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::Type;

/// The function of a native computed default (Java: an anonymous `ComputedDefault` subclass's
/// `getDefault(AttributeMap)`).
pub type NativeComputedDefaultFn =
    Arc<dyn Fn(&dyn AttributeMap) -> Option<AttrValue> + Send + Sync>;

/// How a [`ComputedDefault`] computes its value.
#[derive(Clone)]
pub enum ComputedDefaultKind {
    /// A native rule's computed default.
    Native(NativeComputedDefaultFn),
    /// Java `Attribute.StarlarkComputedDefault`: values precomputed for every combination of
    /// the dependencies' possible values.
    Starlark(StarlarkComputedDefault),
}

impl fmt::Debug for ComputedDefaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComputedDefaultKind::Native(_) => f.write_str("Native(..)"),
            ComputedDefaultKind::Starlark(s) => f.debug_tuple("Starlark").field(s).finish(),
        }
    }
}

/// A default value computed from other attribute values (Java `Attribute.ComputedDefault`).
/// Stored in attribute slots as `AttrValue::ComputedDefault(Arc<..>)`; equality is identity.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1275 Attribute.ComputedDefault
#[derive(Debug, Clone)]
pub struct ComputedDefault {
    /// Sorted (Java `Ordering.natural().immutableSortedCopy`).
    dependencies: Vec<String>,
    kind: ComputedDefaultKind,
}

impl ComputedDefault {
    /// A native computed default depending on `dependencies`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1298 Attribute.ComputedDefault.<init>
    pub fn new_native(dependencies: Vec<String>, f: NativeComputedDefaultFn) -> Self {
        let mut dependencies = dependencies;
        dependencies.sort();
        ComputedDefault {
            dependencies,
            kind: ComputedDefaultKind::Native(f),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1523 Attribute.StarlarkComputedDefault.<init>
    pub(crate) fn new_starlark(dependencies: Vec<String>, s: StarlarkComputedDefault) -> Self {
        let mut dependencies = dependencies;
        dependencies.sort();
        ComputedDefault {
            dependencies,
            kind: ComputedDefaultKind::Starlark(s),
        }
    }

    pub fn kind(&self) -> &ComputedDefaultKind {
        &self.kind
    }

    /// All values this default can take for `rule`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1303 Attribute.ComputedDefault.getPossibleValues
    pub fn get_possible_values(
        &self,
        type_: Type,
        rule: &dyn RuleOrMacroInstance,
    ) -> Vec<Option<AttrValue>> {
        if let ComputedDefaultKind::Starlark(s) = &self.kind {
            // Java: com/google/devtools/build/lib/packages/Attribute.java:1563 Attribute.StarlarkComputedDefault.getPossibleValues
            let _ = type_;
            return s.lookup_table.iter().map(|(_, v)| v.clone()).collect();
        }
        if self.dependencies.is_empty() {
            let mapper = AggregatingAttributeMapper::of(rule);
            let value = self.get_default(&*mapper.create_map_backed_attribute_map(HashMap::new()));
            return vec![value];
        }
        let result: Result<_, ComputeError<std::convert::Infallible>> =
            compute_values_for_all_combinations(
                &self.dependencies,
                rule,
                &mut |_count| Ok(()),
                &mut |map| Ok(self.get_default(map)),
            );
        match result {
            Ok(table) => table.into_iter().map(|(_, v)| v).collect(),
            Err(ComputeError::Limit(m)) => panic!("{m}"),
            Err(ComputeError::Compute(e)) => match e {},
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1321 Attribute.ComputedDefault.dependencies
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1330 Attribute.ComputedDefault.resolvableWithRawAttributes
    pub fn resolvable_with_raw_attributes(&self) -> bool {
        self.dependencies.is_empty()
    }

    /// The value for the given attribute values.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1334 Attribute.ComputedDefault.getDefault
    pub fn get_default(&self, rule: &dyn AttributeMap) -> Option<AttrValue> {
        match &self.kind {
            ComputedDefaultKind::Native(f) => f(rule),
            ComputedDefaultKind::Starlark(s) => s.get_default(&self.dependencies, rule),
        }
    }
}

/// Java `Attribute.StarlarkComputedDefault`: the lookup table of a precomputed Starlark
/// computed default.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1523 Attribute.StarlarkComputedDefault
#[derive(Debug, Clone)]
pub struct StarlarkComputedDefault {
    dependency_types: Vec<Type>,
    // PORT: Java HashMap<List<Object>, Object>; keys are the dependencies' values in order. Only
    // values() iteration order is observable (getPossibleValues); see getPossibleValues.
    lookup_table: Vec<(Vec<Option<AttrValue>>, Option<AttrValue>)>,
}

impl StarlarkComputedDefault {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1531 Attribute.StarlarkComputedDefault.<init>
    pub(crate) fn new(
        dependency_types: Vec<Type>,
        lookup_table: Vec<(Vec<Option<AttrValue>>, Option<AttrValue>)>,
    ) -> Self {
        StarlarkComputedDefault {
            dependency_types,
            lookup_table,
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1541 Attribute.StarlarkComputedDefault.getDependencyTypes
    pub fn get_dependency_types(&self) -> &[Type] {
        &self.dependency_types
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1545 Attribute.StarlarkComputedDefault.getLookupTable
    pub fn get_lookup_table(&self) -> &[(Vec<Option<AttrValue>>, Option<AttrValue>)] {
        &self.lookup_table
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1550 Attribute.StarlarkComputedDefault.getDefault
    fn get_default(&self, dependencies: &[String], rule: &dyn AttributeMap) -> Option<AttrValue> {
        let key = create_dependency_assignment_tuple(dependencies, rule);
        match self.lookup_table.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => v.clone(),
            None => panic!(
                "IllegalStateException: Error in rule '{}': precomputed value missing for \
                 dependencies: {}. Available keys: {}.",
                rule.describe_rule(),
                tuple_to_string(&key),
                format!(
                    "[{}]",
                    self.lookup_table
                        .iter()
                        .map(|(k, _)| tuple_to_string(k))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            ),
        }
    }
}

/// Java `Attribute.StarlarkComputedDefaultTemplate.CannotPrecomputeDefaultsException`.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1508 Attribute.StarlarkComputedDefaultTemplate.CannotPrecomputeDefaultsException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CannotPrecomputeDefaultsException(pub String);

/// A Starlark-defined computed default before precomputation: the attribute type, the
/// callback and its parameter names (Java `Attribute.StarlarkComputedDefaultTemplate`).
// Java: com/google/devtools/build/lib/packages/Attribute.java:1370 Attribute.StarlarkComputedDefaultTemplate
#[derive(Debug, Clone)]
pub struct StarlarkComputedDefaultTemplate {
    type_: Type,
    /// Java `StarlarkCallbackHelper`: the function and its parameter names.
    callback: StarlarkCallable,
    /// Sorted.
    dependencies: Vec<String>,
    /// Java `StarlarkCallbackHelper.starlarkSemantics`: the semantics of the thread that defined
    /// the callback. Set with [`StarlarkComputedDefaultTemplate::with_semantics`].
    semantics: Option<Arc<StarlarkSemantics>>,
}

impl StarlarkComputedDefaultTemplate {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1375 Attribute.StarlarkComputedDefaultTemplate.<init>
    pub fn new(type_: Type, dependencies: Vec<String>, callback: StarlarkCallable) -> Self {
        let mut dependencies = dependencies;
        dependencies.sort();
        StarlarkComputedDefaultTemplate {
            type_,
            callback,
            dependencies,
            semantics: None,
        }
    }

    /// Sets the semantics the callback runs with (Java `new StarlarkCallbackHelper(callback,
    /// thread.getSemantics())`). Required before rules are created.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCallbackHelper.java StarlarkCallbackHelper.<init>
    pub fn with_semantics(mut self, semantics: Arc<StarlarkSemantics>) -> Self {
        self.semantics = Some(semantics);
        self
    }

    pub fn get_type(&self) -> Type {
        self.type_
    }

    pub fn get_callback(&self) -> StarlarkCallable {
        self.callback
    }

    pub fn get_dependencies(&self) -> &[String] {
        &self.dependencies
    }

    /// Precomputes the default for every combination of the dependencies' possible values in
    /// `rule`; reports and fails on errors.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1389 Attribute.StarlarkComputedDefaultTemplate.computePossibleValues
    pub fn compute_possible_values(
        &self,
        attr: &Attribute,
        rule: &dyn RuleOrMacroInstance,
        event_handler: &dyn EventHandler,
    ) -> Result<ComputedDefault, CannotPrecomputeDefaultsException> {
        let mut caught_eval_exception_if_any: Option<String> = None;
        let mut dependency_types: Vec<Type> = Vec::new();
        let result: Result<Vec<_>, String> = (|| {
            for dependency in &self.dependencies {
                let attribute = rule
                    .get_attribute_provider()
                    .get_attribute_by_name_maybe(dependency);
                let Some(attribute) = attribute else {
                    // Java: AttributeNotFoundException
                    return Err(format!(
                        "No such attribute {} in rule {}",
                        dependency,
                        rule.get_label()
                    ));
                };
                dependency_types.push(attribute.get_type());
            }
            let table = compute_values_for_all_combinations(
                &self.dependencies,
                rule,
                // Java: com/google/devtools/build/lib/packages/Attribute.java:1154 Attribute.FixedComputationLimiter
                &mut |count| {
                    if count > COMPUTED_DEFAULT_MAX_COMBINATIONS {
                        // Java: com/google/devtools/build/lib/packages/Attribute.java:1144 Attribute.TooManyConfigurableAttributesException
                        return Err(format!(
                            "Too many configurable attributes to compute all possible values: \
                             Found more than {COMPUTED_DEFAULT_MAX_COMBINATIONS} possible values."
                        ));
                    }
                    Ok(())
                },
                &mut |map| match self.compute_value(event_handler, map) {
                    Ok(v) => Ok::<_, std::convert::Infallible>(v),
                    Err(ex) => {
                        if caught_eval_exception_if_any.is_none() {
                            caught_eval_exception_if_any = Some(ex);
                        }
                        Ok(None)
                    }
                },
            );
            let table = match table {
                Ok(t) => t,
                Err(ComputeError::Limit(m)) => return Err(m),
                Err(ComputeError::Compute(e)) => match e {},
            };
            if let Some(e) = caught_eval_exception_if_any.take() {
                return Err(e);
            }
            Ok(table)
        })();
        match result {
            Ok(lookup_table) => Ok(ComputedDefault::new_starlark(
                self.dependencies.clone(),
                StarlarkComputedDefault::new(dependency_types, lookup_table),
            )),
            Err(message) => {
                let msg = format!(
                    "Cannot compute default value of attribute '{}' in rule '{}': ",
                    attr.get_public_name(),
                    rule.get_label()
                );
                let error = msg + &message;
                rule.report_error(&error, event_handler);
                Err(CannotPrecomputeDefaultsException(error))
            }
        }
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1451 Attribute.StarlarkComputedDefaultTemplate.computeValue
    fn compute_value(
        &self,
        event_handler: &dyn EventHandler,
        rule: &dyn AttributeMap,
    ) -> Result<Option<AttrValue>, String> {
        // Java: HashMap<String, Object> attrValues of the non-computed attributes.
        let mut attr_values: Vec<(String, AttrValue)> = Vec::new();
        for attr_name in rule.get_attribute_names() {
            let attr = rule
                .get_attribute_definition(&attr_name)
                .expect("attribute of the rule");
            if !attr.has_computed_default() {
                let value = rule.get(&attr_name, attr.get_type());
                if let Some(value) = value {
                    attr_values.push((attr.get_name().to_owned(), value));
                }
            }
        }
        self.invoke_callback(event_handler, attr_values)
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1465 Attribute.StarlarkComputedDefaultTemplate.invokeCallback
    fn invoke_callback(
        &self,
        event_handler: &dyn EventHandler,
        attr_values: Vec<(String, AttrValue)>,
    ) -> Result<Option<AttrValue>, String> {
        let Some(semantics) = &self.semantics else {
            blaze_util::unported!(
                "StarlarkComputedDefaultTemplate without semantics (call with_semantics)"
            )
        };
        // Java: StarlarkCallbackHelper.buildArgumentList: one positional argument per parameter,
        // looked up in the struct of attribute values.
        let names = self.callback.get_parameter_names();
        let mut sorted_names: Vec<&str> = attr_values.iter().map(|(k, _)| k.as_str()).collect();
        sorted_names.sort();
        for name in &names {
            if !attr_values.iter().any(|(k, _)| k == name) {
                // Java: StarlarkInfoWithMessage.getErrorMessageForUnknownField, thrown as an
                // IllegalArgumentException and wrapped in an EvalException by
                // StarlarkCallbackHelper.call.
                return Err(format!(
                    "No such regular (non computed) attribute '{}'.\nAvailable attributes: {}",
                    name,
                    sorted_names.join(", ")
                ));
            }
        }
        let events = Arc::new(CollectingEvents::default());
        let mut ctx = ThreadContext::new(Phase::Loading, semantics.clone(), events.clone());
        // Java: `Label()` in the callback resolves against the module of the innermost Starlark
        // function (the callback's .bzl file).
        // PORT: the callback's own module is not known here, so every loaded .bzl is registered.
        for (file, package_context) in
            crate::bzl_context::BazelModuleContext::all_package_contexts()
        {
            ctx.file_package_contexts.insert(file, package_context);
        }
        let type_ = self.type_;
        let result = call_frozen(
            &self.callback.to_owned_frozen_value(),
            &ctx,
            &EvalOptions::default(),
            |module| {
                let heap = module.heap();
                let positional = names
                    .iter()
                    .map(|n| {
                        let v = attr_values.iter().find(|(k, _)| k == n).map(|(_, v)| v);
                        Attribute::value_to_starlark(v, heap)
                    })
                    .collect();
                (positional, Vec::new())
            },
            |res, _module, _eval| Ok(cast_callback_result(type_, res)),
        );
        // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(eventHandler))
        for e in events.take() {
            let location = e.location.as_ref().map(|l| {
                blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32)
            });
            let event = if e.kind == "DEBUG" {
                blaze_events::Event::debug_at(location, e.message)
            } else {
                blaze_events::Event::warn_at(location, e.message)
            };
            event_handler.handle(&event);
        }
        match result {
            Ok(r) => r,
            Err(e) => Err(e.message),
        }
    }
}

/// The function computing a late-bound default's loading-phase value from the rule (Java
/// `Function<RuleOrMacroInstance, ValueT> defaultValueEvaluator`).
pub type LateBoundDefaultEvaluator =
    Arc<dyn Fn(Option<&dyn RuleOrMacroInstance>) -> Option<AttrValue> + Send + Sync>;

/// The kind of a [`LateBoundDefault`] (Java subclasses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateBoundDefaultKind {
    /// Java `Attribute.LabelLateBoundDefault`.
    Label,
    /// Java `Attribute.LabelListLateBoundDefault`.
    LabelList,
    /// Java `Attribute.AlwaysNullLateBoundDefault`.
    AlwaysNull,
    /// Another `SimpleLateBoundDefault`.
    Other,
}

/// A default resolved during analysis from the configuration (Java
/// `Attribute.LateBoundDefault`). The loading phase only needs its default value; the analysis
/// resolver is not ported yet.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1616 Attribute.LateBoundDefault
#[derive(Clone)]
pub struct LateBoundDefault {
    /// Java `Class<FragmentT>`: the simple name of the configuration fragment class, `None` for
    /// `Void`.
    fragment_class: Option<String>,
    default_value_evaluator: LateBoundDefaultEvaluator,
    kind: LateBoundDefaultKind,
    /// Which Java late-bound default this is, so analysis can resolve it: for
    /// `configuration_field(fragment, name)` `"configuration_field:<fragment>:<name>"`, for
    /// native ones the Java constant (e.g. `"BaseRuleClasses.ACTION_LISTENER"`).
    // W4-analysis: Java resolves through the object's own `resolve` method; the resolver
    // implementations live in blaze_analysis and are looked up by this name.
    resolver_name: Option<String>,
}

impl fmt::Debug for LateBoundDefault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LateBoundDefault")
            .field("fragment_class", &self.fragment_class)
            .field("kind", &self.kind)
            .field("resolver_name", &self.resolver_name)
            .finish_non_exhaustive()
    }
}

impl LateBoundDefault {
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1645 Attribute.LateBoundDefault.<init>
    pub fn new(
        fragment_class: Option<String>,
        default_value_evaluator: LateBoundDefaultEvaluator,
        kind: LateBoundDefaultKind,
    ) -> Self {
        LateBoundDefault {
            fragment_class,
            default_value_evaluator,
            kind,
            resolver_name: None,
        }
    }

    /// Names the analysis resolver of this default (see `resolver_name`).
    // W4-analysis: see the field.
    pub fn with_resolver_name(mut self, name: &str) -> Self {
        self.resolver_name = Some(name.to_owned());
        self
    }

    /// The analysis resolver name (see `with_resolver_name`).
    // W4-analysis: see the field.
    pub fn get_resolver_name(&self) -> Option<&str> {
        self.resolver_name.as_deref()
    }

    /// Java `LateBoundDefault.alwaysNull()`.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1640 Attribute.LateBoundDefault.alwaysNull
    pub fn always_null() -> Self {
        LateBoundDefault::new(
            None,
            Arc::new(|_: Option<&dyn RuleOrMacroInstance>| None),
            LateBoundDefaultKind::AlwaysNull,
        )
    }

    /// Java `LabelLateBoundDefault.fromTargetConfiguration(fragmentClass, defaultValue, resolver)`
    /// (the resolver is not ported).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1730 Attribute.LabelLateBoundDefault.fromTargetConfiguration
    pub fn label_from_target_configuration(
        fragment_class: &str,
        default_value: Option<Label>,
    ) -> Self {
        LateBoundDefault::new(
            Some(fragment_class.to_owned()),
            Arc::new(move |_: Option<&dyn RuleOrMacroInstance>| {
                default_value.clone().map(AttrValue::Label)
            }),
            LateBoundDefaultKind::Label,
        )
    }

    /// Java `LabelListLateBoundDefault.fromTargetConfiguration` / `fromRuleAndAttributesOnly`
    /// (default: the empty list).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1799 Attribute.LabelListLateBoundDefault.fromTargetConfiguration
    pub fn label_list(fragment_class: Option<&str>) -> Self {
        LateBoundDefault::new(
            fragment_class.map(str::to_owned),
            Arc::new(|_: Option<&dyn RuleOrMacroInstance>| Some(AttrValue::LabelList(Vec::new()))),
            LateBoundDefaultKind::LabelList,
        )
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1652 Attribute.LateBoundDefault.getFragmentClass
    pub fn get_fragment_class(&self) -> Option<&str> {
        self.fragment_class.as_deref()
    }

    pub fn kind(&self) -> LateBoundDefaultKind {
        self.kind
    }

    /// The default value used when the configuration is unknown.
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1663 Attribute.LateBoundDefault.getDefault
    pub fn get_default(&self, rule: Option<&dyn RuleOrMacroInstance>) -> Option<AttrValue> {
        (self.default_value_evaluator)(rule)
    }

    /// Resolves the value from the configuration fragment (analysis phase).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1675 Attribute.LateBoundDefault.resolve
    pub fn resolve(&self) -> Option<AttrValue> {
        blaze_util::unported!("Attribute.LateBoundDefault.resolve")
    }
}

/// Java `packages/MaterializingDefault`: a dormant-dependency materializer (placeholder; only
/// its identity matters during loading).
// Java: com/google/devtools/build/lib/packages/MaterializingDefault.java MaterializingDefault
#[derive(Debug, Clone)]
pub struct MaterializingDefault {
    /// The materializer implementation function.
    pub implementation: StarlarkCallable,
}

impl MaterializingDefault {
    /// The value used when the materializer has not run (Java `MaterializingDefault.getDefault`).
    // Java: com/google/devtools/build/lib/packages/MaterializingDefault.java MaterializingDefault.getDefault
    pub fn get_default(&self) -> Option<AttrValue> {
        blaze_util::unported!("MaterializingDefault.getDefault")
    }
}

/// Java `Attribute.ComputationStrategy.createDependencyAssignmentTuple` result keys and the
/// combinations computed by `AggregatingAttributeMapper.visitAttributes`.
pub type AttributeAssignment = HashMap<String, Option<AttrValue>>;

// Java: com/google/devtools/build/lib/packages/Attribute.java:1157 Attribute.FixedComputationLimiter.COMPUTED_DEFAULT_MAX_COMBINATIONS
const COMPUTED_DEFAULT_MAX_COMBINATIONS: i32 = 64;

/// Failure of [`compute_values_for_all_combinations`]: the limiter's or the computation's.
enum ComputeError<E> {
    Limit(String),
    Compute(E),
}

/// Computes `compute` for every combination of the possible values of `dependencies` (Java
/// `ComputationStrategy.computeValuesForAllCombinations`). Entries are keyed by the
/// dependencies' values; a repeated key replaces the earlier value in place.
// PORT: java-hash-order. Java collects into a HashMap and callers iterate its values(); entries
// here are in combination order.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1182 Attribute.ComputationStrategy.computeValuesForAllCombinations
fn compute_values_for_all_combinations<E>(
    dependencies: &[String],
    rule: &dyn RuleOrMacroInstance,
    limiter: &mut dyn FnMut(i32) -> Result<(), String>,
    compute: &mut dyn FnMut(&dyn AttributeMap) -> Result<Option<AttrValue>, E>,
) -> Result<Vec<(Vec<Option<AttrValue>>, Option<AttrValue>)>, ComputeError<E>> {
    let mapper = AggregatingAttributeMapper::of(rule);
    // This will hold every (value1, value2, ..) combination of the declared dependencies.
    let dep_maps = mapper
        .visit_attributes(dependencies, limiter)
        .map_err(ComputeError::Limit)?;
    // For each combination, call compute() on a specialized AttributeMap providing those values.
    let mut value_map: Vec<(Vec<Option<AttrValue>>, Option<AttrValue>)> = Vec::new();
    for dep_map in dep_maps {
        let attr_map = mapper.create_map_backed_attribute_map(dep_map);
        let value = compute(&*attr_map).map_err(ComputeError::Compute)?;
        let key = create_dependency_assignment_tuple(dependencies, &*attr_map);
        match value_map.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => value_map.push((key, value)),
        }
    }
    Ok(value_map)
}

// Java: com/google/devtools/build/lib/packages/Attribute.java:1201 Attribute.ComputationStrategy.createDependencyAssignmentTuple
fn create_dependency_assignment_tuple(
    dependencies: &[String],
    attr_map: &dyn AttributeMap,
) -> Vec<Option<AttrValue>> {
    let mut tuple = Vec::with_capacity(dependencies.len());
    for attr_name in dependencies {
        let attr_type = attr_map
            .get_attribute_type(attr_name)
            .expect("dependency attribute exists");
        tuple.push(attr_map.get(attr_name, attr_type));
    }
    tuple
}

// Java: Iterables.toString of a dependency tuple.
fn tuple_to_string(t: &[Option<AttrValue>]) -> String {
    format!(
        "[{}]",
        t.iter()
            .map(|v| match v {
                Some(v) => crate::attribute::attribute::java_to_string(v),
                None => "null".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Java `type.cast(result == NONE ? type.getDefaultValue() : result)` plus the element check of
/// `invokeCallback`: the callback's Starlark result as an attribute value of `type_`, or the
/// error "expected '<type>', but got '<starlark type>'".
// Java: com/google/devtools/build/lib/packages/Attribute.java:1471 Attribute.StarlarkComputedDefaultTemplate.invokeCallback
fn cast_callback_result(type_: Type, result: Value<'_>) -> Result<Option<AttrValue>, String> {
    if result.is_none() {
        return Ok(type_.get_default_value());
    }
    let mismatch = || format!("expected '{}', but got '{}'", type_, result.get_type());
    let label_of = |v: Value<'_>| v.downcast_ref::<LabelValue>().map(|l| l.0.clone());
    if let Some(elem_type) = type_.get_list_element_type() {
        let Some(list) = UnpackListOrTuple::<Value>::unpack_value(result)
            .ok()
            .flatten()
        else {
            return Err(mismatch());
        };
        let elem_mismatch =
            |elem: Value<'_>| format!("expected '{}', but got '{}'", elem_type, elem.get_type());
        return match type_ {
            Type::StringList | Type::StringSet => {
                let mut out = Vec::new();
                for e in list.items {
                    match e.unpack_str() {
                        Some(s) => out.push(s.to_owned()),
                        None => return Err(elem_mismatch(e)),
                    }
                }
                Ok(Some(if type_ == Type::StringSet {
                    AttrValue::StringSet(out.into_iter().collect())
                } else {
                    AttrValue::StringList(out)
                }))
            }
            Type::IntegerList => {
                let mut out = Vec::new();
                for e in list.items {
                    match e.unpack_i32() {
                        Some(i) => out.push(i),
                        None => return Err(elem_mismatch(e)),
                    }
                }
                Ok(Some(AttrValue::IntList(out)))
            }
            _ => {
                let mut out = Vec::new();
                for e in list.items {
                    match label_of(e) {
                        Some(l) => out.push(l),
                        None => return Err(elem_mismatch(e)),
                    }
                }
                Ok(Some(AttrValue::LabelList(out)))
            }
        };
    }
    match type_ {
        Type::Integer => result
            .unpack_i32()
            .map(|i| Some(AttrValue::Int(i)))
            .ok_or_else(mismatch),
        Type::String | Type::StringNoIntern => result
            .unpack_str()
            .map(|s| Some(AttrValue::String(s.to_owned())))
            .ok_or_else(mismatch),
        Type::Boolean => result
            .unpack_bool()
            .map(|b| Some(AttrValue::Bool(b)))
            .ok_or_else(mismatch),
        Type::Label
        | Type::NodepLabel
        | Type::DormantLabel
        | Type::GenqueryScopeType
        | Type::Output => label_of(result)
            .map(|l| Some(AttrValue::Label(l)))
            .ok_or_else(mismatch),
        t if t.is_dict_type() => {
            // PORT: Java casts to Map without checking entries; entries of the wrong type would
            // fail later. Here they are converted, failing with the same message format.
            let Some(dict) = DictRef::from_value(result) else {
                return Err(mismatch());
            };
            let str_of = |v: Value<'_>| v.unpack_str().map(str::to_owned);
            let strs_of = |v: Value<'_>| -> Option<Vec<String>> {
                let l = UnpackListOrTuple::<Value>::unpack_value(v).ok().flatten()?;
                l.items
                    .iter()
                    .map(|e| e.unpack_str().map(str::to_owned))
                    .collect()
            };
            let labels_of = |v: Value<'_>| -> Option<Vec<blaze_cmdline::Label>> {
                let l = UnpackListOrTuple::<Value>::unpack_value(v).ok().flatten()?;
                l.items.iter().map(|e| label_of(*e)).collect()
            };
            let mut ok = true;
            let out = match t {
                Type::StringDict => AttrValue::StringDict(
                    dict.iter()
                        .filter_map(|(k, v)| {
                            let r = str_of(k).zip(str_of(v));
                            ok &= r.is_some();
                            r
                        })
                        .collect(),
                ),
                Type::StringListDict => AttrValue::StringListDict(
                    dict.iter()
                        .filter_map(|(k, v)| {
                            let r = str_of(k).zip(strs_of(v));
                            ok &= r.is_some();
                            r
                        })
                        .collect(),
                ),
                Type::LabelDictUnary => AttrValue::LabelDictUnary(
                    dict.iter()
                        .filter_map(|(k, v)| {
                            let r = str_of(k).zip(label_of(v));
                            ok &= r.is_some();
                            r
                        })
                        .collect(),
                ),
                Type::LabelKeyedStringDict => AttrValue::LabelKeyedStringDict(
                    dict.iter()
                        .filter_map(|(k, v)| {
                            let r = label_of(k).zip(str_of(v));
                            ok &= r.is_some();
                            r
                        })
                        .collect(),
                ),
                _ => AttrValue::LabelListDict(
                    dict.iter()
                        .filter_map(|(k, v)| {
                            let r = str_of(k).zip(labels_of(v));
                            ok &= r.is_some();
                            r
                        })
                        .collect(),
                ),
            };
            if ok { Ok(Some(out)) } else { Err(mismatch()) }
        }
        _ => blaze_util::unported!(
            "StarlarkComputedDefaultTemplate result",
            "of type {}",
            type_
        ),
    }
}
