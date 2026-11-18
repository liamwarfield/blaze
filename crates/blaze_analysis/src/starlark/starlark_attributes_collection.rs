//! `ctx.attr`, `ctx.file`, `ctx.files` and `ctx.executable`: port of
//! `analysis/starlark/StarlarkAttributesCollection.java` (the builder that converts a rule's
//! attribute values and prerequisites into Starlark values).
//!
//! The collection object itself (Java's `StarlarkAttributesCollection`, exposed to Starlark only
//! as `ctx.rule` in aspects) is kept as Rust data: the four struct values and the
//! executable-runfiles map.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_cmdline::Label;
use blaze_packages::attribute::attribute::Attribute;
use blaze_packages::rule_class::declared_exec_group::is_valid_identifier;
use blaze_packages::rule_class::rule_class::EXEC_GROUP_COMPATIBLE_WITH_ATTR;
use blaze_packages::types::{AttrValue, LabelClass, Type};
use blaze_starlark::structs::{StarlarkInfo, StructProvider};
use indexmap::{IndexMap, IndexSet};
use starlark::values::{Heap, Value};

use super::util::{alloc_immutable_dict, alloc_immutable_list};

use crate::configured_target::ConfiguredTarget;
use crate::configuredtargets::target_to_starlark;
use crate::providers::alias_provider::AliasProvider;
use crate::providers::file_provider::FilesToRunProvider;
use crate::rule::prerequisite_artifacts::PrerequisiteArtifacts;
use crate::rule::prerequisites_collection::PrerequisitesCollection;

/// Java `StarlarkAttributesCollection.ERROR_MESSAGE_FOR_NO_ATTR`.
// PORT: Java builds `ctx.attr` with `StructProvider.STRUCT.create(attrs, ERROR_MESSAGE_FOR_NO_ATTR)`
// (a `StarlarkInfoWithMessage`), so `ctx.attr.nosuch` fails with this text. blaze_starlark's
// `StarlarkInfo` has no unknown-field format yet (request: `StarlarkInfo::with_message`), so the
// error is "'struct' value has no field or method 'nosuch'" plus the same "Available
// attributes" line. An error either way; only the first sentence of the message differs.
#[allow(dead_code)]
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
pub(crate) fn alloc_struct<'v>(heap: Heap<'v>, fields: Vec<(String, Value<'v>)>) -> Value<'v> {
    let provider = heap.alloc(StructProvider);
    heap.alloc(StarlarkInfo::new(provider, fields))
}

/// Java `StarlarkAttributesCollection.Builder`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:179 StarlarkAttributesCollection.Builder
pub struct StarlarkAttributesCollectionBuilder<'a, 'v> {
    prerequisites: &'a PrerequisitesCollection,
    heap: Heap<'v>,
    attr_builder: IndexMap<String, Value<'v>>,
    executable_builder: IndexMap<String, Value<'v>>,
    executable_runfiles_builder: IndexMap<Artifact, Arc<FilesToRunProvider>>,
    file_builder: IndexMap<String, Value<'v>>,
    files_builder: IndexMap<String, Value<'v>>,
    seen_executables: IndexSet<Artifact>,
    rule_variables_builder: IndexMap<String, String>,
}

/// The configured targets of `prerequisites` as Starlark `Target` values.
fn targets_to_starlark<'v>(heap: Heap<'v>, prerequisites: &[ConfiguredTarget]) -> Vec<Value<'v>> {
    prerequisites
        .iter()
        .map(|t| target_to_starlark(t, heap))
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
) -> Value<'v> {
    let values = targets_to_starlark(heap, prerequisites);
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
) -> Value<'v> {
    let values = targets_to_starlark(heap, prerequisites);
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
        return match val {
            None => Some(Value::new_none()),
            Some(_) => blaze_util::unported!("DormantDependency"),
        };
    }

    if type_ == Type::DormantLabelList {
        blaze_util::unported!("DormantDependency");
    }

    if type_.get_label_class() != LabelClass::Dependency {
        // Attribute values should be type safe
        return Some(Attribute::value_to_starlark(val, heap));
    }

    None
}

/// Java `Builder.convertAttributeValue(prerequisiteSupplier, a, val)`; `None` where Java returns
/// null (the attribute is not exposed).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:336 StarlarkAttributesCollection.Builder.convertAttributeValue
pub fn convert_attribute_value<'v>(
    prerequisite_supplier: &dyn Fn() -> Vec<ConfiguredTarget>,
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
    let is_split = a.get_transition_factory().is_split();

    if type_ == Type::Label && !is_split {
        let prerequisites = prerequisite_supplier();
        Some(match prerequisites.first() {
            None => Value::new_none(),
            Some(p) => target_to_starlark(p, heap),
        })
    } else if type_ == Type::LabelList || (type_ == Type::Label && is_split) {
        let all_prereq = prerequisite_supplier();
        // Java: StarlarkList.immutableCopyOf(allPrereq)
        Some(alloc_immutable_list(
            heap,
            targets_to_starlark(heap, &all_prereq),
        ))
    } else if type_ == Type::LabelDictUnary {
        let Some(AttrValue::LabelDictUnary(m)) = val else {
            panic!("ClassCastException: LABEL_DICT_UNARY value {val:?}");
        };
        Some(convert_string_to_label_map(
            heap,
            m,
            &prerequisite_supplier(),
        ))
    } else if type_ == Type::LabelKeyedStringDict {
        let Some(AttrValue::LabelKeyedStringDict(original)) = val else {
            panic!("ClassCastException: LABEL_KEYED_STRING_DICT value {val:?}");
        };
        let all_prereq = prerequisite_supplier();
        let mut entries: Vec<(Value<'v>, Value<'v>)> = Vec::with_capacity(original.len());
        for prereq in &all_prereq {
            let key = target_to_starlark(prereq, heap);
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
    ) -> StarlarkAttributesCollectionBuilder<'a, 'v> {
        StarlarkAttributesCollectionBuilder {
            prerequisites,
            heap,
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
        let Some(starlark_val) =
            convert_attribute_value(&|| prerequisites.get_prerequisites(&name), a, val, heap)
        else {
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
            attr_object: alloc_struct(heap, self.attr_builder.into_iter().collect()),
            executable_object: alloc_struct(heap, self.executable_builder.into_iter().collect()),
            file_object: alloc_struct(heap, self.file_builder.into_iter().collect()),
            files_object: alloc_struct(heap, self.files_builder.into_iter().collect()),
            executable_runfiles_map: Arc::new(self.executable_runfiles_builder),
            rule_class_name,
            rule_variables: self.rule_variables_builder,
        }
    }
}
