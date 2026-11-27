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
    /// The Java class of a native computed default (`getClass().getName()`, usually an
    /// anonymous class such as `BaseRuleClasses$2`), which `StarlarkValue.repr` prints.
    java_class_name: Option<&'static str>,
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
            java_class_name: None,
        }
    }

    /// Names the Java class this native computed default ports (`getClass().getName()`).
    pub fn with_java_class_name(mut self, java_class_name: &'static str) -> Self {
        self.java_class_name = Some(java_class_name);
        self
    }

    /// The Java class name set by [`ComputedDefault::with_java_class_name`].
    pub fn java_class_name(&self) -> Option<&'static str> {
        self.java_class_name
    }

    // Java: com/google/devtools/build/lib/packages/Attribute.java:1523 Attribute.StarlarkComputedDefault.<init>
    pub(crate) fn new_starlark(dependencies: Vec<String>, s: StarlarkComputedDefault) -> Self {
        let mut dependencies = dependencies;
        dependencies.sort();
        ComputedDefault {
            dependencies,
            kind: ComputedDefaultKind::Starlark(s),
            java_class_name: None,
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
    // PORT: java-hash-order. Java `HashMap<List<Object>, Object>`; keys are the dependencies'
    // values in order. Its iteration order is observable through `values()` (getPossibleValues:
    // the possible values query prints and the order labels are visited in) and `keySet()` (the
    // "Available keys" message), so the entries are kept in Java's iteration order (see
    // `starlark_lookup_table_order`).
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
                 dependencies: {}. Available keys: [{}].",
                rule.describe_rule(),
                tuple_to_string(&key),
                self.lookup_table
                    .iter()
                    .map(|(k, _)| tuple_to_string(k))
                    .collect::<Vec<_>>()
                    .join(", ")
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
            Ok(value_map) => Ok(ComputedDefault::new_starlark(
                self.dependencies.clone(),
                // Java: `lookupTable = new HashMap<>(strategy.computeValuesForAllCombinations(..))`.
                StarlarkComputedDefault::new(
                    dependency_types,
                    starlark_lookup_table_order(value_map),
                ),
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
        // Java: `Label()` in the callback resolves against the module of the innermost Starlark
        // function (the callback's .bzl file); blaze_starlark reads it from that module.
        let ctx = ThreadContext::new(Phase::Loading, semantics.clone(), events.clone());
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

/// Java `StarlarkAttrModule.StarlarkMaterializer`: the Starlark materializer function of a
/// dormant-dependency attribute, with the semantics it runs under. Resolving it (calling the
/// function with the rule's attributes and prerequisites) is analysis
/// (`DependencyProducer.evaluateMaterializerTargets`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:219 StarlarkAttrModule.StarlarkMaterializer
#[derive(Debug, Clone)]
pub struct StarlarkMaterializer {
    pub type_: Type,
    pub semantics: Arc<StarlarkSemantics>,
    /// The materializer implementation function.
    pub implementation: StarlarkCallable,
}

impl StarlarkMaterializer {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:226 StarlarkAttrModule.StarlarkMaterializer.<init>
    pub fn new(
        type_: Type,
        semantics: Arc<StarlarkSemantics>,
        implementation: StarlarkCallable,
    ) -> Self {
        StarlarkMaterializer {
            type_,
            semantics,
            implementation,
        }
    }
}

/// Java `packages/MaterializingDefault`: the default of a materializing (dormant-dependency)
/// attribute.
// Java: com/google/devtools/build/lib/packages/MaterializingDefault.java:31 MaterializingDefault
#[derive(Debug, Clone)]
pub struct MaterializingDefault {
    type_: Type,
    /// Java `resolver` (a `StarlarkMaterializer`; its analysis context class is `ImmutableMap`).
    resolver: StarlarkMaterializer,
}

impl MaterializingDefault {
    // Java: com/google/devtools/build/lib/packages/MaterializingDefault.java:46 MaterializingDefault.<init>
    pub fn new(type_: Type, resolver: StarlarkMaterializer) -> Self {
        blaze_util::check_argument!(type_ == Type::Label || type_ == Type::LabelList);
        MaterializingDefault { type_, resolver }
    }

    /// The value used when the materializer has not run: the type's default.
    // Java: com/google/devtools/build/lib/packages/MaterializingDefault.java:57 MaterializingDefault.getDefault
    pub fn get_default(&self) -> Option<AttrValue> {
        self.type_.get_default_value()
    }

    /// The materializer (for `MaterializingDefault.resolve` at analysis).
    pub fn get_resolver(&self) -> &StarlarkMaterializer {
        &self.resolver
    }

    /// The materializer implementation function.
    pub fn implementation(&self) -> StarlarkCallable {
        self.resolver.implementation
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

/// The entries of a `Map<List<Object>, T>` keyed by dependency assignment tuples.
type DependencyAssignmentEntries = Vec<(Vec<Option<AttrValue>>, Option<AttrValue>)>;

/// Java `Object.hashCode()` of one attribute value of a dependency assignment tuple (`null`
/// hashes as 0 in `List.hashCode()`).
///
/// A value whose Java hash code is an identity hash (enums: `TriState`, the license types of
/// a `License`) or belongs to a default-computing object cannot be replayed and fails loudly.
fn attr_value_java_hash_code(value: Option<&AttrValue>) -> i32 {
    use blaze_query_engine::JavaHashCode;
    // Java (JDK): java/util/AbstractList.java AbstractList.hashCode (the `List.hashCode()`
    // contract, which ArrayList and ImmutableList follow).
    fn list_hash(hashes: impl Iterator<Item = i32>) -> i32 {
        let mut hash_code: i32 = 1;
        for e in hashes {
            hash_code = 31i32.wrapping_mul(hash_code).wrapping_add(e);
        }
        hash_code
    }
    // Java (JDK): java/util/AbstractMap.java AbstractMap.hashCode (the `Map.hashCode()` contract:
    // the sum of `key.hashCode() ^ value.hashCode()` over the entries), and
    // java/util/AbstractSet.java AbstractSet.hashCode (the sum of the elements' hash codes).
    fn sum_hash(hashes: impl Iterator<Item = i32>) -> i32 {
        let mut h: i32 = 0;
        for e in hashes {
            h = h.wrapping_add(e);
        }
        h
    }
    // Java: net/starlark/java/eval/StarlarkInt.java:216 StarlarkInt.Int32.hashCode
    fn starlark_int_hash(v: i32) -> i32 {
        0x316c5239i32.wrapping_mul(v) ^ 0x67c4a7d5
    }
    let Some(value) = value else {
        return 0;
    };
    match value {
        // `Type.STRING`: java.lang.String.
        AttrValue::String(s) => s.java_hash_code(),
        // `Type.INTEGER`: StarlarkInt (Int32).
        AttrValue::Int(i) => starlark_int_hash(*i),
        // `Type.BOOLEAN`: java.lang.Boolean.hashCode() is 1231 for true and 1237 for false.
        AttrValue::Bool(b) => {
            if *b {
                1231
            } else {
                1237
            }
        }
        // Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
        AttrValue::Label(l) => l.java_hash_code(),
        AttrValue::StringList(l) => list_hash(l.iter().map(|s| s.java_hash_code())),
        AttrValue::IntList(l) => list_hash(l.iter().map(|i| starlark_int_hash(*i))),
        AttrValue::LabelList(l) => list_hash(l.iter().map(|l| l.java_hash_code())),
        AttrValue::StringSet(set) => sum_hash(set.iter().map(|s| s.java_hash_code())),
        AttrValue::StringDict(d) => sum_hash(
            d.iter()
                .map(|(k, v)| k.java_hash_code() ^ v.java_hash_code()),
        ),
        AttrValue::StringListDict(d) => sum_hash(
            d.iter()
                .map(|(k, v)| k.java_hash_code() ^ list_hash(v.iter().map(|s| s.java_hash_code()))),
        ),
        AttrValue::LabelDictUnary(d) => sum_hash(
            d.iter()
                .map(|(k, v)| k.java_hash_code() ^ v.java_hash_code()),
        ),
        AttrValue::LabelKeyedStringDict(d) => sum_hash(
            d.iter()
                .map(|(k, v)| k.java_hash_code() ^ v.java_hash_code()),
        ),
        AttrValue::LabelListDict(d) => sum_hash(
            d.iter()
                .map(|(k, v)| k.java_hash_code() ^ list_hash(v.iter().map(|l| l.java_hash_code()))),
        ),
        AttrValue::License(_)
        | AttrValue::TriState(_)
        | AttrValue::SelectorList(_)
        | AttrValue::ComputedDefault(_)
        | AttrValue::StarlarkComputedDefaultTemplate(_)
        | AttrValue::LateBound(_)
        | AttrValue::Materializer(_) => blaze_util::unported!(
            "Attribute.ComputationStrategy.computeValuesForAllCombinations",
            "the iteration order of Java's HashMap of dependency assignments needs the Java \
             hashCode() of {:?}, which is an identity hash",
            value
        ),
    }
}

/// Java `List<Object>.hashCode()` of a dependency assignment tuple (an `ArrayList`).
// Java (JDK): java/util/AbstractList.java AbstractList.hashCode
fn dependency_assignment_tuple_java_hash_code(tuple: &[Option<AttrValue>]) -> i32 {
    let mut hash_code: i32 = 1;
    for e in tuple {
        hash_code = 31i32
            .wrapping_mul(hash_code)
            .wrapping_add(attr_value_java_hash_code(e.as_ref()));
    }
    hash_code
}

/// The iteration order of a `java.util.HashMap` made by `new HashMap<>(initial_capacity)` and
/// filled with `entries` (distinct keys) in order. A map of fewer than two entries has only one
/// order, so no hash code is needed for it.
// PORT: java-hash-order.
fn java_hash_map_order_of_assignments(
    initial_capacity: i32,
    entries: DependencyAssignmentEntries,
) -> DependencyAssignmentEntries {
    if entries.len() < 2 {
        return entries;
    }
    blaze_query_engine::java_concurrent_hash_map::java_hash_map_order_by(
        blaze_query_engine::java_concurrent_hash_map::java_hash_map_table_size_for(
            initial_capacity,
        ),
        entries,
        |key| dependency_assignment_tuple_java_hash_code(key),
    )
}

/// Java `Maps.capacity(expectedSize)` of Guava 33.5.0 (the version Bazel 9.3.2 pins in
/// MODULE.bazel): the `initialCapacity` `Maps.newHashMapWithExpectedSize` passes to `HashMap`.
// Java (Guava): com/google/common/collect/Maps.java Maps.capacity
fn guava_maps_capacity(expected_size: usize) -> i32 {
    if expected_size < 3 {
        return expected_size as i32 + 1;
    }
    // `(int) Math.ceil(expectedSize / 0.75)`; expectedSize is far below Ints.MAX_POWER_OF_TWO.
    (expected_size as f64 / 0.75).ceil() as i32
}

/// Java `new HashMap<>(valueMap)` of `StarlarkComputedDefaultTemplate.computePossibleValues`:
/// the lookup table's entries in its iteration order, given `value_map` in the iteration order
/// of the map it is copied from.
///
/// `HashMap(Map)` calls `putMapEntries`, which sizes the empty table for
/// `(int) Math.ceil(s / (double) loadFactor)` (JDK 19 and later; Bazel 9.3.2 embeds JDK 25.0.2,
/// on which the resulting table lengths 2, 4, 4, 8, 8, 8, 16 for 1 to 7 entries were measured)
/// and then puts the entries in the source map's iteration order.
// PORT: java-hash-order.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1436 Attribute.StarlarkComputedDefaultTemplate.computePossibleValues
// Java (JDK): java/util/HashMap.java HashMap.putMapEntries
fn starlark_lookup_table_order(
    value_map: DependencyAssignmentEntries,
) -> DependencyAssignmentEntries {
    let s = value_map.len();
    let t = (s as f64 / 0.75).ceil() as i32;
    java_hash_map_order_of_assignments(t, value_map)
}

/// Computes `compute` for every combination of the possible values of `dependencies` (Java
/// `ComputationStrategy.computeValuesForAllCombinations`). Entries are keyed by the
/// dependencies' values; a repeated key replaces the earlier value in place.
// PORT: java-hash-order. Java collects into `Maps.newHashMapWithExpectedSize(depMaps.size())` and
// callers iterate it (`values()` in ComputedDefault.getPossibleValues, the copy constructor in
// StarlarkComputedDefaultTemplate.computePossibleValues); the entries are returned in that
// map's iteration order.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1182 Attribute.ComputationStrategy.computeValuesForAllCombinations
fn compute_values_for_all_combinations<E>(
    dependencies: &[String],
    rule: &dyn RuleOrMacroInstance,
    limiter: &mut dyn FnMut(i32) -> Result<(), String>,
    compute: &mut dyn FnMut(&dyn AttributeMap) -> Result<Option<AttrValue>, E>,
) -> Result<DependencyAssignmentEntries, ComputeError<E>> {
    let mapper = AggregatingAttributeMapper::of(rule);
    // This will hold every (value1, value2, ..) combination of the declared dependencies.
    let dep_maps = mapper
        .visit_attributes(dependencies, limiter)
        .map_err(ComputeError::Limit)?;
    // For each combination, call compute() on a specialized AttributeMap providing those values.
    let expected_size = dep_maps.len();
    let mut value_map: DependencyAssignmentEntries = Vec::new();
    for dep_map in dep_maps {
        let attr_map = mapper.create_map_backed_attribute_map(dep_map);
        let value = compute(&*attr_map).map_err(ComputeError::Compute)?;
        let key = create_dependency_assignment_tuple(dependencies, &*attr_map);
        // HashMap.put of a key already present replaces the value and keeps the entry's place.
        match value_map.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => value_map.push((key, value)),
        }
    }
    Ok(java_hash_map_order_of_assignments(
        guava_maps_capacity(expected_size),
        value_map,
    ))
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

#[cfg(test)]
mod java_hash_order_tests {
    use super::*;

    // The expected hash codes and orders were printed by the JDK 25 that Bazel 9.3.2 embeds, with
    // the pinned server jar on the class path, for
    //   Map<List<Object>, Object> valueMap = Maps.newHashMapWithExpectedSize(n);  // puts in order
    //   Map<List<Object>, Object> lookupTable = new HashMap<>(valueMap);
    // as `ComputationStrategy.computeValuesForAllCombinations` and
    // `StarlarkComputedDefaultTemplate.computePossibleValues` build them, with keys made of
    // String, StarlarkInt, Boolean, Label and ImmutableList values (and null).
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1221 Attribute.ComputationStrategy.computeValuesForAllCombinations
    // Java: com/google/devtools/build/lib/packages/Attribute.java:1436 Attribute.StarlarkComputedDefaultTemplate.computePossibleValues

    fn s(v: &str) -> Option<AttrValue> {
        Some(AttrValue::String(v.to_owned()))
    }

    fn label(v: &str) -> Label {
        Label::parse_canonical_unchecked(v)
    }

    fn strings(v: &[&str]) -> Option<AttrValue> {
        Some(AttrValue::StringList(
            v.iter().map(|s| (*s).to_owned()).collect(),
        ))
    }

    /// Both maps, as Java builds them from `n` combinations whose keys are `keys` in order
    /// (a repeated key replaces the value in place): the keys of `valueMap` and of
    /// `lookupTable` with the index of the combination each holds.
    fn orders(n: usize, keys: Vec<Vec<Option<AttrValue>>>) -> (Vec<String>, Vec<String>) {
        assert_eq!(n, keys.len());
        let mut entries: DependencyAssignmentEntries = Vec::new();
        for (i, key) in keys.into_iter().enumerate() {
            let value = Some(AttrValue::Int(i as i32));
            match entries.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }
        let show = |entries: &DependencyAssignmentEntries| -> Vec<String> {
            entries
                .iter()
                .map(|(k, v)| {
                    let Some(AttrValue::Int(i)) = v else {
                        panic!("not an index")
                    };
                    format!("{}=v{}", tuple_to_string(k), i)
                })
                .collect()
        };
        let value_map = java_hash_map_order_of_assignments(guava_maps_capacity(n), entries);
        let shown_value_map = show(&value_map);
        let lookup_table = starlark_lookup_table_order(value_map);
        (shown_value_map, show(&lookup_table))
    }

    fn hash(key: Vec<Option<AttrValue>>) -> i32 {
        dependency_assignment_tuple_java_hash_code(&key)
    }

    #[test]
    fn tuple_hash_codes_match_java() {
        assert_eq!(hash(vec![s("a")]), 128);
        assert_eq!(hash(vec![s("eta")]), 100785);
        assert_eq!(hash(vec![s("zeta")]), 3735287);
        assert_eq!(hash(vec![s("theta")]), 110327485);
        // StarlarkInt.Int32.hashCode.
        assert_eq!(hash(vec![Some(AttrValue::Int(0))]), 1740941300);
        assert_eq!(hash(vec![Some(AttrValue::Int(3))]), -209694307);
        assert_eq!(hash(vec![Some(AttrValue::Int(-1))]), -1453913551);
        assert_eq!(hash(vec![Some(AttrValue::Int(1000000))]), 2119886772);
        // Boolean.hashCode.
        assert_eq!(hash(vec![Some(AttrValue::Bool(false))]), 1268);
        assert_eq!(hash(vec![Some(AttrValue::Bool(true))]), 1262);
        // Label.hashCode.
        assert_eq!(hash(vec![Some(AttrValue::Label(label("//q:m")))]), 928966);
        assert_eq!(hash(vec![Some(AttrValue::Label(label("//p:z")))]), 929368);
        // Two-element tuples.
        assert_eq!(hash(vec![s("x"), Some(AttrValue::Bool(true))]), 5912);
        assert_eq!(hash(vec![s("w"), Some(AttrValue::Bool(false))]), 5887);
        // List values.
        assert_eq!(hash(vec![strings(&[])]), 32);
        assert_eq!(hash(vec![strings(&["a", "b"])]), 4097);
        assert_eq!(hash(vec![strings(&["b", "a"])]), 4127);
        assert_eq!(
            hash(vec![Some(AttrValue::LabelList(vec![
                label("//p:z"),
                label("//p:y")
            ]))]),
            29739745
        );
        assert_eq!(
            hash(vec![Some(AttrValue::LabelList(vec![label("//p:a")]))]),
            928624
        );
        assert_eq!(hash(vec![Some(AttrValue::IntList(vec![2, 1]))]), -183670011);
        assert_eq!(hash(vec![Some(AttrValue::IntList(vec![7]))]), 1043503256);
        // null.
        assert_eq!(hash(vec![None]), 31);
        // An internal-form (Latin-1 holding UTF-8) string: the bytes C3 A9.
        assert_eq!(hash(vec![s("\u{c3}\u{a9}")]), 6245);
    }

    #[test]
    fn string_keys_iterate_in_java_hash_map_order() {
        let (value_map, lookup_table) = orders(
            5,
            vec![
                vec![s("zeta")],
                vec![s("eta")],
                vec![s("theta")],
                vec![s("iota")],
                vec![s("kappa")],
            ],
        );
        let expected = vec![
            "[eta]=v1",
            "[iota]=v3",
            "[kappa]=v4",
            "[theta]=v2",
            "[zeta]=v0",
        ];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    #[test]
    fn int_keys_iterate_in_java_hash_map_order() {
        let (value_map, lookup_table) = orders(
            4,
            vec![
                vec![Some(AttrValue::Int(3))],
                vec![Some(AttrValue::Int(-1))],
                vec![Some(AttrValue::Int(0))],
                vec![Some(AttrValue::Int(1000000))],
            ],
        );
        let expected = vec!["[0]=v2", "[3]=v0", "[-1]=v1", "[1000000]=v3"];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    #[test]
    fn label_keys_iterate_in_java_hash_map_order() {
        let (value_map, lookup_table) = orders(
            3,
            vec![
                vec![Some(AttrValue::Label(label("//p:z")))],
                vec![Some(AttrValue::Label(label("//p:a")))],
                vec![Some(AttrValue::Label(label("//q:m")))],
            ],
        );
        let expected = vec!["[//q:m]=v2", "[//p:z]=v0", "[//p:a]=v1"];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    #[test]
    fn two_dependencies_iterate_in_java_hash_map_order() {
        let t = || Some(AttrValue::Bool(true));
        let f = || Some(AttrValue::Bool(false));
        let (value_map, lookup_table) = orders(
            6,
            vec![
                vec![s("x"), t()],
                vec![s("x"), f()],
                vec![s("y"), t()],
                vec![s("y"), f()],
                vec![s("w"), t()],
                vec![s("w"), f()],
            ],
        );
        let expected = vec![
            "[x, true]=v0",
            "[w, true]=v4",
            "[y, false]=v3",
            "[x, false]=v1",
            "[y, true]=v2",
            "[w, false]=v5",
        ];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    #[test]
    fn list_and_null_keys_iterate_in_java_hash_map_order() {
        let (value_map, lookup_table) = orders(
            3,
            vec![
                vec![strings(&["b", "a"])],
                vec![strings(&[])],
                vec![strings(&["a", "b"])],
            ],
        );
        let expected = vec!["[[]]=v1", "[[a, b]]=v2", "[[b, a]]=v0"];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);

        let (value_map, lookup_table) = orders(2, vec![vec![None], vec![s("s")]]);
        let expected = vec!["[s]=v1", "[null]=v0"];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    // Two select branches with the same value give the same key: HashMap.put replaces the value
    // and keeps the entry; the copy is then sized for 3 entries, not 6.
    #[test]
    fn repeated_keys_keep_their_place_and_take_the_last_value() {
        let (value_map, lookup_table) = orders(
            6,
            vec![
                vec![s("zeta")],
                vec![s("eta")],
                vec![s("zeta")],
                vec![s("theta")],
                vec![s("eta")],
                vec![s("theta")],
            ],
        );
        let expected = vec!["[eta]=v4", "[theta]=v5", "[zeta]=v2"];
        assert_eq!(value_map, expected);
        assert_eq!(lookup_table, expected);
    }

    // Java: enum hash codes are identity hashes, so the order of a HashMap keyed by them is
    // not reproducible.
    #[test]
    #[should_panic(expected = "Attribute.ComputationStrategy.computeValuesForAllCombinations")]
    fn tristate_keys_fail_loudly_when_the_order_depends_on_them() {
        orders(
            2,
            vec![
                vec![Some(AttrValue::TriState(crate::types::TriState::Yes))],
                vec![Some(AttrValue::TriState(crate::types::TriState::No))],
            ],
        );
    }

    // A map of one entry has one order: no hash code is needed.
    #[test]
    fn a_single_combination_needs_no_hash_code() {
        let (value_map, lookup_table) = orders(
            1,
            vec![vec![Some(AttrValue::TriState(
                crate::types::TriState::Auto,
            ))]],
        );
        assert_eq!(value_map.len(), 1);
        assert_eq!(lookup_table.len(), 1);
    }
}
