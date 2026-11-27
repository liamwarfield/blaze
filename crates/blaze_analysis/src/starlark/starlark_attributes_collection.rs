//! `ctx.attr`, `ctx.file`, `ctx.files` and `ctx.executable`: port of
//! `analysis/starlark/StarlarkAttributesCollection.java` (the builder that converts a rule's
//! attribute values and prerequisites into Starlark values).
//!
//! The collection object itself (Java's `StarlarkAttributesCollection`, exposed to Starlark only
//! as `ctx.rule` in aspects) is kept as Rust data: the four struct values and the
//! executable-runfiles map.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_cmdline::Label;
use blaze_packages::attribute::attribute::Attribute;
use blaze_packages::rule_class::declared_exec_group::is_valid_identifier;
use blaze_packages::rule_class::rule_class::EXEC_GROUP_COMPATIBLE_WITH_ATTR;
use blaze_packages::types::{AttrValue, LabelClass, Type};
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::{StarlarkInfo, StructProvider};
use indexmap::{IndexMap, IndexSet};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::AllocDict;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, FrozenHeap, FrozenValue, Heap, NoSerialize,
    OwnedFrozenValue, StarlarkValue, Trace, Tracer, Value, ValueLike, starlark_value,
};

use super::starlark_action_factory::{StarlarkActionContext, closed_error};
use super::starlark_toolchain_context::{StarlarkExecGroupCollection, StarlarkToolchainContext};
use crate::toolchains::aspect_base_target_resolved_toolchain_context::ToolchainAspectsProviders;

use super::util::{alloc_immutable_dict, alloc_immutable_list};

use crate::configured_target::ConfiguredTarget;
use crate::configuredtargets::target_to_starlark;
use crate::dormant_dependency::DormantDependency;
use crate::providers::alias_provider::AliasProvider;
use crate::providers::file_provider::FilesToRunProvider;
use crate::rule::prerequisite_artifacts::PrerequisiteArtifacts;
use crate::rule::prerequisites_collection::PrerequisitesCollection;
use blaze_starlark::semantics::StarlarkSemantics;

/// Java `StarlarkAttributesCollection.ERROR_MESSAGE_FOR_NO_ATTR`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:67 StarlarkAttributesCollection.ERROR_MESSAGE_FOR_NO_ATTR
pub(crate) const ERROR_MESSAGE_FOR_NO_ATTR: &str =
    "No attribute '%s' in attr. Make sure you declared a rule attribute with this name.";

/// The values built by [`StarlarkAttributesCollectionBuilder`].
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:57 StarlarkAttributesCollection
pub struct StarlarkAttributesCollection<'v> {
    pub attr_object: Value<'v>,
    pub executable_object: Value<'v>,
    pub file_object: Value<'v>,
    pub files_object: Value<'v>,
    pub executable_runfiles_map: Arc<IndexMap<Artifact, Arc<FilesToRunProvider>>>,
    pub rule_class_name: String,
    /// Java `ruleVariables` (aspects only).
    pub rule_variables: IndexMap<String, String>,
}

/// Allocates a `struct` with the given fields (Java `StructProvider.STRUCT.create(map,
/// errorMessageFormat)`).
// Java: com/google/devtools/build/lib/packages/StructProvider.java:50 StructProvider.create
pub(crate) fn alloc_struct<'v>(
    heap: Heap<'v>,
    fields: Vec<(String, Value<'v>)>,
    error_message_format_for_unknown_field: &str,
) -> Value<'v> {
    let provider = heap.alloc(StructProvider);
    heap.alloc(StarlarkInfo::with_message(
        provider,
        fields,
        error_message_format_for_unknown_field,
    ))
}

/// Java `StarlarkAttributesCollection.Builder`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:179 StarlarkAttributesCollection.Builder
pub struct StarlarkAttributesCollectionBuilder<'a, 'v> {
    prerequisites: &'a PrerequisitesCollection,
    heap: Heap<'v>,
    semantics: &'a StarlarkSemantics,
    attr_builder: IndexMap<String, Value<'v>>,
    executable_builder: IndexMap<String, Value<'v>>,
    executable_runfiles_builder: IndexMap<Artifact, Arc<FilesToRunProvider>>,
    file_builder: IndexMap<String, Value<'v>>,
    files_builder: IndexMap<String, Value<'v>>,
    seen_executables: IndexSet<Artifact>,
    rule_variables_builder: IndexMap<String, String>,
}

/// The configured targets of `prerequisites` as Starlark `Target` values.
fn targets_to_starlark<'v>(
    heap: Heap<'v>,
    prerequisites: &[ConfiguredTarget],
    semantics: &StarlarkSemantics,
) -> Vec<Value<'v>> {
    prerequisites
        .iter()
        .map(|t| target_to_starlark(t, heap, semantics))
        .collect()
}

/// Java `Builder.buildPrequisiteMap`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:198 StarlarkAttributesCollection.Builder.buildPrequisiteMap
fn build_prerequisite_map<'v>(
    prerequisites: &[ConfiguredTarget],
    values: &[Value<'v>],
) -> IndexMap<Label, Value<'v>> {
    let mut map = IndexMap::new();
    for (prereq, value) in prerequisites.iter().zip(values) {
        map.insert(AliasProvider::get_dependency_label(prereq), *value);
    }
    map
}

/// Java `Builder.convertStringToLabelMap`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:208 StarlarkAttributesCollection.Builder.convertStringToLabelMap
pub(crate) fn convert_string_to_label_map<'v>(
    heap: Heap<'v>,
    unconfigured_value: &IndexMap<String, Label>,
    prerequisites: &[ConfiguredTarget],
    semantics: &StarlarkSemantics,
) -> Value<'v> {
    let values = targets_to_starlark(heap, prerequisites, semantics);
    let prerequisite_map = build_prerequisite_map(prerequisites, &values);
    // Java: Dict.immutableCopyOf(builder.buildOrThrow())
    alloc_immutable_dict(
        heap,
        unconfigured_value
            .iter()
            .map(|(key, label)| {
                (
                    heap.alloc_str(key).to_value(),
                    prerequisite_map
                        .get(label)
                        .copied()
                        .unwrap_or_else(Value::new_none),
                )
            })
            .collect(),
    )
}

/// Java `Builder.convertStringToLabelListMap`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:219 StarlarkAttributesCollection.Builder.convertStringToLabelListMap
pub(crate) fn convert_string_to_label_list_map<'v>(
    heap: Heap<'v>,
    unconfigured_value: &IndexMap<String, Vec<Label>>,
    prerequisites: &[ConfiguredTarget],
    semantics: &StarlarkSemantics,
) -> Value<'v> {
    let values = targets_to_starlark(heap, prerequisites, semantics);
    let prerequisite_map = build_prerequisite_map(prerequisites, &values);
    // Java: Dict.immutableCopyOf of StarlarkList.immutableCopyOf(Lists.transform(labels,
    // prerequisiteMap::get)) values.
    alloc_immutable_dict(
        heap,
        unconfigured_value
            .iter()
            .map(|(key, labels)| {
                (
                    heap.alloc_str(key).to_value(),
                    alloc_immutable_list(
                        heap,
                        labels
                            .iter()
                            .map(|l| {
                                prerequisite_map
                                    .get(l)
                                    .copied()
                                    .unwrap_or_else(Value::new_none)
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

/// Java `Builder.shouldIgnore`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:234 StarlarkAttributesCollection.Builder.shouldIgnore
fn should_ignore(a: &Attribute) -> bool {
    let type_ = a.get_type();
    let skyname = a.get_public_name();

    // Some legacy native attribute types do not have a valid Starlark type. Avoid exposing
    // these to Starlark.
    if type_ == Type::Tristate {
        return true;
    }

    // Don't expose invalid attributes via the rule ctx.attr. Ordinarily, this case cannot happen,
    // and currently only applies to subrule attributes
    // TODO: b/293304174 - let subrules explicitly mark attributes as not-visible-to-starlark
    if !is_valid_identifier(&skyname) {
        return true;
    }

    // Don't expose exec_group_compatible_with to Starlark. There is no reason for it to be used
    // by the rule implementation function and its type (LABEL_LIST_DICT) is not available to
    // Starlark.
    if a.get_name() == EXEC_GROUP_COMPATIBLE_WITH_ATTR {
        return true;
    }

    false
}

/// Java `Builder.maybeDirectVal`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:263 StarlarkAttributesCollection.Builder.maybeDirectVal
fn maybe_direct_val<'v>(
    a: &Attribute,
    val: Option<&AttrValue>,
    heap: Heap<'v>,
) -> Option<Value<'v>> {
    let type_ = a.get_type();

    if type_ == Type::DormantLabel {
        return Some(match val {
            None => Value::new_none(),
            Some(AttrValue::Label(l)) => heap.alloc(DormantDependency::new(l.clone())),
            Some(other) => panic!("ClassCastException: DORMANT_LABEL value {other:?}"),
        });
    }

    if type_ == Type::DormantLabelList {
        let labels: &[Label] = match val {
            Some(AttrValue::LabelList(l)) => l,
            // BuildType.DORMANT_LABEL_LIST.cast(null): ImmutableList.copyOf of null throws.
            None => panic!("NullPointerException: DORMANT_LABEL_LIST value null"),
            Some(other) => panic!("ClassCastException: DORMANT_LABEL_LIST value {other:?}"),
        };
        let dormant_deps = labels
            .iter()
            .map(|l| heap.alloc(DormantDependency::new(l.clone())))
            .collect();
        return Some(alloc_immutable_list(heap, dormant_deps));
    }

    if type_.get_label_class() != LabelClass::Dependency {
        // Attribute values should be type safe
        return Some(Attribute::value_to_starlark(val, heap));
    }

    None
}

/// Java `Builder.convertAttributeValueForAspectPropagationFunc(depLabelsSupplier, a, val)`;
/// `None` where Java returns null (the attribute is not exposed).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:293 StarlarkAttributesCollection.Builder.convertAttributeValueForAspectPropagationFunc
pub fn convert_attribute_value_for_aspect_propagation_func<'v>(
    dep_labels_supplier: &dyn Fn() -> Vec<Label>,
    a: &Attribute,
    val: Option<&AttrValue>,
    heap: Heap<'v>,
) -> Option<Value<'v>> {
    if should_ignore(a) {
        return None;
    }

    if let Some(v) = maybe_direct_val(a, val, heap) {
        return Some(v);
    }

    let type_ = a.get_type();

    let prerequisites = dep_labels_supplier();

    // PORT: Java's `|| prerequisites == null || prerequisites.contains(null)`: the supplier's
    // `OrderedSetMultimap.get` never returns null, and only materializing attributes (handled by
    // the first operand) have a null label; the caller drops those.
    if a.is_materializing() {
        return Some(Value::new_none());
    }

    let is_split = a.get_transition_factory().is_split();
    let label_value = |l: &Label| heap.alloc(LabelValue(l.clone()));
    if type_ == Type::Label && !is_split {
        Some(match prerequisites.first() {
            None => Value::new_none(),
            Some(l) => label_value(l),
        })
    } else if type_ == Type::LabelList || (type_ == Type::Label && is_split) {
        // Java: StarlarkList.immutableCopyOf(prerequisites)
        Some(alloc_immutable_list(
            heap,
            prerequisites.iter().map(label_value).collect(),
        ))
    } else if type_ == Type::LabelDictUnary || type_ == Type::LabelKeyedStringDict {
        // return the same map as the labels are not configured to targets
        // PORT: Java returns the Java map; the interpreter converts it on each access with
        // `Starlark.fromJava` (a new mutable dict).
        Some(match val {
            Some(AttrValue::LabelDictUnary(m)) => heap.alloc(AllocDict(
                m.iter()
                    .map(|(k, l)| (heap.alloc_str(k).to_value(), label_value(l))),
            )),
            Some(AttrValue::LabelKeyedStringDict(m)) => heap.alloc(AllocDict(
                m.iter()
                    .map(|(l, v)| (label_value(l), heap.alloc_str(v).to_value())),
            )),
            other => panic!("ClassCastException: {type_:?} value {other:?}"),
        })
    } else if type_ == Type::LabelListDict {
        // The type of the inner lists has to be converted to Starlark.
        let Some(AttrValue::LabelListDict(m)) = val else {
            panic!("ClassCastException: LABEL_LIST_DICT value {val:?}");
        };
        // Java: Dict.immutableCopyOf(Maps.transformValues(..., StarlarkList::immutableCopyOf))
        Some(alloc_immutable_dict(
            heap,
            m.iter()
                .map(|(k, labels)| {
                    (
                        heap.alloc_str(k).to_value(),
                        alloc_immutable_list(heap, labels.iter().map(label_value).collect()),
                    )
                })
                .collect(),
        ))
    } else {
        panic!(
            "IllegalArgumentException: Can't transform attribute {} of type {:?} to a Starlark \
             object",
            a.get_name(),
            type_
        );
    }
}

/// Java `Builder.convertAttributeValue(prerequisiteSupplier, a, val)`; `None` where Java returns
/// null (the attribute is not exposed).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:336 StarlarkAttributesCollection.Builder.convertAttributeValue
pub fn convert_attribute_value<'v>(
    prerequisite_supplier: &dyn Fn() -> Vec<ConfiguredTarget>,
    a: &Attribute,
    val: Option<&AttrValue>,
    heap: Heap<'v>,
    semantics: &StarlarkSemantics,
) -> Option<Value<'v>> {
    if should_ignore(a) {
        return None;
    }

    if let Some(v) = maybe_direct_val(a, val, heap) {
        return Some(v);
    }

    let type_ = a.get_type();
    let is_split = a.get_transition_factory().is_split();

    if type_ == Type::Label && !is_split {
        let prerequisites = prerequisite_supplier();
        Some(match prerequisites.first() {
            None => Value::new_none(),
            Some(p) => target_to_starlark(p, heap, semantics),
        })
    } else if type_ == Type::LabelList || (type_ == Type::Label && is_split) {
        let all_prereq = prerequisite_supplier();
        // Java: StarlarkList.immutableCopyOf(allPrereq)
        Some(alloc_immutable_list(
            heap,
            targets_to_starlark(heap, &all_prereq, semantics),
        ))
    } else if type_ == Type::LabelDictUnary {
        let Some(AttrValue::LabelDictUnary(m)) = val else {
            panic!("ClassCastException: LABEL_DICT_UNARY value {val:?}");
        };
        Some(convert_string_to_label_map(
            heap,
            m,
            &prerequisite_supplier(),
            semantics,
        ))
    } else if type_ == Type::LabelKeyedStringDict {
        let Some(AttrValue::LabelKeyedStringDict(original)) = val else {
            panic!("ClassCastException: LABEL_KEYED_STRING_DICT value {val:?}");
        };
        let all_prereq = prerequisite_supplier();
        let mut entries: Vec<(Value<'v>, Value<'v>)> = Vec::with_capacity(original.len());
        for prereq in &all_prereq {
            let key = target_to_starlark(prereq, heap, semantics);
            let value = match original.get(&AliasProvider::get_dependency_label(prereq)) {
                Some(s) => heap.alloc_str(s).to_value(),
                None => Value::new_none(),
            };
            entries.push((key, value));
        }
        // Java: Dict.immutableCopyOf(builder.buildOrThrow())
        Some(alloc_immutable_dict(heap, entries))
    } else if type_ == Type::LabelListDict {
        let Some(AttrValue::LabelListDict(m)) = val else {
            panic!("ClassCastException: LABEL_LIST_DICT value {val:?}");
        };
        Some(convert_string_to_label_list_map(
            heap,
            m,
            &prerequisite_supplier(),
            semantics,
        ))
    } else {
        panic!(
            "IllegalArgumentException: Can't transform attribute {} of type {:?} to a Starlark \
             object",
            a.get_name(),
            type_
        );
    }
}

impl<'a, 'v> StarlarkAttributesCollectionBuilder<'a, 'v> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:173 StarlarkAttributesCollection.builder
    pub fn new(
        prerequisites: &'a PrerequisitesCollection,
        heap: Heap<'v>,
        semantics: &'a StarlarkSemantics,
    ) -> StarlarkAttributesCollectionBuilder<'a, 'v> {
        StarlarkAttributesCollectionBuilder {
            prerequisites,
            heap,
            semantics,
            attr_builder: IndexMap::new(),
            executable_builder: IndexMap::new(),
            executable_runfiles_builder: IndexMap::new(),
            file_builder: IndexMap::new(),
            files_builder: IndexMap::new(),
            seen_executables: IndexSet::new(),
            rule_variables_builder: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:384 StarlarkAttributesCollection.Builder.addAttribute
    pub fn add_attribute(&mut self, a: &Attribute, val: Option<&AttrValue>) {
        let heap = self.heap;
        let type_ = a.get_type();
        let skyname = a.get_public_name();

        // The first attribute with the same name wins.
        if self.attr_builder.contains_key(&skyname) {
            return;
        }

        let prerequisites = self.prerequisites;
        let name = a.get_name().to_owned();
        let Some(starlark_val) = convert_attribute_value(
            &|| prerequisites.get_prerequisites(&name),
            a,
            val,
            heap,
            self.semantics,
        ) else {
            return;
        };

        self.attr_builder.insert(skyname.clone(), starlark_val);
        if type_.get_label_class() != LabelClass::Dependency {
            return;
        }

        let files = PrerequisiteArtifacts::nested_set(self.prerequisites, a.get_name());
        // Java: files.isEmpty() ? StarlarkList.empty() : StarlarkList.lazyImmutable(files::toList)
        // (both immutable; the lazy list is flattened here, with the same elements and order).
        let files_value = alloc_immutable_list(
            heap,
            files
                .to_list()
                .iter()
                .map(|f| heap.alloc(StarlarkFile::new(f.clone())))
                .collect(),
        );
        self.files_builder.insert(skyname.clone(), files_value);

        if a.is_executable() {
            // In a Starlark-defined rule, only LABEL type attributes (not LABEL_LIST) can have
            // the Executable flag. However, we could be here because we're creating a
            // StarlarkRuleContext for a native rule for builtins injection, in which case we may
            // see an executable LABEL_LIST. In that case omit the attribute as if it weren't
            // executable.
            if type_ == Type::Label {
                let provider = self.prerequisites.get_executable_prerequisite(a.get_name());
                match provider
                    .as_ref()
                    .and_then(|p| p.get_executable().cloned().map(|e| (p.clone(), e)))
                {
                    Some((provider, executable)) => {
                        self.executable_builder.insert(
                            skyname.clone(),
                            heap.alloc(StarlarkFile::new(executable.clone())),
                        );
                        if !self.seen_executables.contains(&executable) {
                            // todo(dslomov,laurentlb): In general, this is incorrect.
                            // We associate the first encountered FilesToRunProvider with
                            // the executable (this provider is later used to build the spawn).
                            // However ideally we should associate a provider with the attribute
                            // name, and pass the correct FilesToRunProvider to the spawn
                            // depending on what attribute is used to access the executable.
                            self.executable_runfiles_builder
                                .insert(executable.clone(), provider);
                            self.seen_executables.insert(executable);
                        }
                    }
                    None => {
                        self.executable_builder
                            .insert(skyname.clone(), Value::new_none());
                    }
                }
            }
        }
        if a.is_single_artifact() {
            // In Starlark only label (not label list) type attributes can have the
            // SingleArtifact flag.
            let artifact = self.prerequisites.get_prerequisite_artifact(a.get_name());
            match artifact {
                Some(artifact) => {
                    self.file_builder
                        .insert(skyname, heap.alloc(StarlarkFile::new(artifact)));
                }
                None => {
                    self.file_builder.insert(skyname, Value::new_none());
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:452 StarlarkAttributesCollection.Builder.putAllRuleVariables
    pub fn put_all_rule_variables(&mut self, var: &IndexMap<String, String>) -> &mut Self {
        for (k, v) in var {
            self.rule_variables_builder.insert(k.clone(), v.clone());
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:457 StarlarkAttributesCollection.Builder.build
    pub fn build(self, rule_class_name: String) -> StarlarkAttributesCollection<'v> {
        let heap = self.heap;
        StarlarkAttributesCollection {
            // Java: StarlarkAttributesCollection.java:81-96 (the constructor).
            attr_object: alloc_struct(
                heap,
                self.attr_builder.into_iter().collect(),
                ERROR_MESSAGE_FOR_NO_ATTR,
            ),
            executable_object: alloc_struct(
                heap,
                self.executable_builder.into_iter().collect(),
                "No attribute '%s' in executable. Make sure there is a label type attribute \
                 marked as 'executable' with this name",
            ),
            file_object: alloc_struct(
                heap,
                self.file_builder.into_iter().collect(),
                "No attribute '%s' in file. Make sure there is a label type attribute marked as \
                 'allow_single_file' with this name",
            ),
            files_object: alloc_struct(
                heap,
                self.files_builder.into_iter().collect(),
                "No attribute '%s' in files. Make sure there is a label or label_list type \
                 attribute with this name",
            ),
            executable_runfiles_map: Arc::new(self.executable_runfiles_builder),
            rule_class_name,
            rule_variables: self.rule_variables_builder,
        }
    }
}

// -------------------------------------------------------------------------------------------
// The Starlark value (`ctx.rule` in aspect implementations)

/// The value of `ctx.rule` in an aspect implementation (Java `StarlarkAttributesCollection`,
/// type `rule_attributes`): information about attributes of a rule an aspect is applied to.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:57 StarlarkAttributesCollection
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkAttributesCollectionValue<'v> {
    /// Java `starlarkRuleContext` (its `checkMutable` and rule context).
    #[allocative(skip)]
    context: StarlarkActionContext,
    attr_object: Value<'v>,
    executable_object: Value<'v>,
    file_object: Value<'v>,
    files_object: Value<'v>,
    rule_class_name: String,
    /// Java `ruleVariables` (an immutable dict).
    rule_variables: Value<'v>,
}

/// `ctx.rule` after the aspect implementation returned. Java's object outlives `close()`: every
/// field fails `checkMutable` except `var`, which Java does not guard.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ClosedStarlarkAttributesCollectionValue {
    rule_label_canonical_name: String,
    #[allocative(skip)]
    rule_variables: FrozenValue,
}

impl<'v> StarlarkAttributesCollectionValue<'v> {
    /// Allocates the collection built by `collection`'s builder, with `rule_variables` as an
    /// immutable dict (Java `ruleVariablesBuilder.buildImmutable()`).
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:70 StarlarkAttributesCollection.<init>
    pub(crate) fn new(
        context: StarlarkActionContext,
        collection: StarlarkAttributesCollection<'v>,
        heap: Heap<'v>,
    ) -> StarlarkAttributesCollectionValue<'v> {
        let rule_variables = alloc_immutable_dict(
            heap,
            collection
                .rule_variables
                .iter()
                .map(|(k, v)| (heap.alloc_str(k).to_value(), heap.alloc_str(v).to_value()))
                .collect(),
        );
        StarlarkAttributesCollectionValue {
            context,
            attr_object: collection.attr_object,
            executable_object: collection.executable_object,
            file_object: collection.file_object,
            files_object: collection.files_object,
            rule_class_name: collection.rule_class_name,
            rule_variables,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:101 StarlarkAttributesCollection.checkMutable
    fn check_mutable(&self, attr_name: &str) -> starlark::Result<()> {
        self.context.check_mutable(&format!("rule.{attr_name}"))
    }
}

impl fmt::Debug for StarlarkAttributesCollectionValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for StarlarkAttributesCollectionValue<'_> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:175 StarlarkAttributesCollection.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<rule collection for {}>",
            self.context.rule_label_canonical_name
        )
    }
}

impl fmt::Display for ClosedStarlarkAttributesCollectionValue {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:175 StarlarkAttributesCollection.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<rule collection for {}>",
            self.rule_label_canonical_name
        )
    }
}

// SAFETY: `trace` visits every Starlark value held.
unsafe impl<'v> Trace<'v> for StarlarkAttributesCollectionValue<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.attr_object.trace(tracer);
        self.executable_object.trace(tracer);
        self.file_object.trace(tracer);
        self.files_object.trace(tracer);
        self.rule_variables.trace(tracer);
    }
}

impl<'v> Freeze for StarlarkAttributesCollectionValue<'v> {
    type Frozen = ClosedStarlarkAttributesCollectionValue;

    fn freeze(self, freezer: &Freezer) -> FreezeResult<ClosedStarlarkAttributesCollectionValue> {
        Ok(ClosedStarlarkAttributesCollectionValue {
            rule_label_canonical_name: self.context.rule_label_canonical_name,
            rule_variables: freezer.freeze(self.rule_variables)?,
        })
    }
}

impl<'v> AllocValue<'v> for StarlarkAttributesCollectionValue<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

#[starlark_value(type = "rule_attributes")]
impl<'v> StarlarkValue<'v> for StarlarkAttributesCollectionValue<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("rule_attributes", rule_attributes_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

#[starlark_value(type = "rule_attributes")]
impl<'v> StarlarkValue<'v> for ClosedStarlarkAttributesCollectionValue {
    type Canonical = StarlarkAttributesCollectionValue<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("rule_attributes", rule_attributes_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.to_string());
    }
}

/// The live collection behind `this`, after Java's `checkMutable("rule." + attr_name)`.
fn live_collection<'v>(
    this: Value<'v>,
    attr_name: &str,
) -> starlark::Result<&'v StarlarkAttributesCollectionValue<'v>> {
    if let Some(c) = this.downcast_ref::<StarlarkAttributesCollectionValue<'v>>() {
        c.check_mutable(attr_name)?;
        return Ok(c);
    }
    if let Some(c) = this.downcast_ref::<ClosedStarlarkAttributesCollectionValue>() {
        return Err(closed_error(
            &format!("rule.{attr_name}"),
            &c.rule_label_canonical_name,
        ));
    }
    Err(super::util::errorf(format!(
        "internal error: expected rule_attributes, got {}",
        this.get_type()
    )))
}

#[starlark_module]
fn rule_attributes_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:106 StarlarkAttributesCollection.getAttr
    #[starlark(attribute)]
    fn attr<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live_collection(this, "attr")?.attr_object)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:112 StarlarkAttributesCollection.getExecutable
    #[starlark(attribute)]
    fn executable<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live_collection(this, "executable")?.executable_object)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:118 StarlarkAttributesCollection.getFile
    #[starlark(attribute)]
    fn file<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live_collection(this, "file")?.file_object)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:124 StarlarkAttributesCollection.getFiles
    #[starlark(attribute)]
    fn files<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        Ok(live_collection(this, "files")?.files_object)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:130 StarlarkAttributesCollection.getRuleClassName
    #[starlark(attribute)]
    fn kind<'v>(this: Value<'v>) -> starlark::Result<String> {
        Ok(live_collection(this, "kind")?.rule_class_name.clone())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:136 StarlarkAttributesCollection.toolchains
    #[starlark(attribute)]
    fn toolchains<'v>(this: Value<'v>) -> starlark::Result<StarlarkToolchainContext> {
        let c = live_collection(this, "toolchains")?;
        let rc = c.context.handle.borrow();
        let Some(base_target_toolchain_contexts) = rc.get_base_target_toolchain_contexts() else {
            return Ok(StarlarkToolchainContext::NotValid);
        };
        Ok(StarlarkToolchainContext::create(
            base_target_toolchain_contexts
                .get_default_toolchain_context()
                .target_description()
                .to_owned(),
            /* resolve_toolchain_data_func= */
            &|l| {
                rc.get_toolchain_target(l)
                    .map(toolchain_aspects_providers_value)
            },
            /* resolved_toolchain_type_labels= */
            rc.get_requested_toolchain_types_labels(),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:154 StarlarkAttributesCollection.execGroups
    #[starlark(attribute)]
    fn exec_groups<'v>(this: Value<'v>) -> starlark::Result<StarlarkExecGroupCollection> {
        let c = live_collection(this, "exec_groups")?;
        let rc = c.context.handle.borrow();
        let Some(base_target_toolchain_contexts) = rc.get_base_target_toolchain_contexts() else {
            return Ok(StarlarkExecGroupCollection::NotValid);
        };
        // Create a thin wrapper around the toolchain collection, to expose the Starlark API.
        Ok(StarlarkExecGroupCollection::create_for_base_target(
            base_target_toolchain_contexts,
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:467 StarlarkAttributesCollection.var
    #[starlark(attribute)]
    fn var<'v>(this: Value<'v>) -> starlark::Result<Value<'v>> {
        if let Some(c) = this.downcast_ref::<StarlarkAttributesCollectionValue<'v>>() {
            return Ok(c.rule_variables);
        }
        if let Some(c) = this.downcast_ref::<ClosedStarlarkAttributesCollectionValue>() {
            return Ok(c.rule_variables.to_value());
        }
        Err(super::util::errorf(format!(
            "internal error: expected rule_attributes, got {}",
            this.get_type()
        )))
    }
}

/// A `ToolchainAspectsProviders` as the frozen value `StarlarkToolchainContext` returns for
/// `ctx.rule.toolchains[type]`.
pub(crate) fn toolchain_aspects_providers_value(
    providers: &ToolchainAspectsProviders,
) -> OwnedFrozenValue {
    let heap = FrozenHeap::new();
    let v: FrozenValue = heap.alloc_simple(providers.clone());
    let heap_ref = heap.into_ref();
    // SAFETY: `v` was allocated on `heap_ref`.
    unsafe { OwnedFrozenValue::new(heap_ref, v) }
}
