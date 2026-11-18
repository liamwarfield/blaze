//! `bazel_module` (the elements of `module_ctx.modules`), its `tags` structure, and the
//! type-checked tags in it.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java and
//! TypeCheckedTag.java.
//!
//! PORT: Java's objects are immutable and compared by identity. Blaze allocates them, and the
//! attribute values of the tags, once per extension evaluation on a frozen heap (see
//! `super::module_extension_context::ModuleCtxFields`), so they are immutable and keep their
//! identity too. Java's `Structure.getErrorMessageForUnknownField` ("unknown tag class foo",
//! "unknown attribute foo") has no hook in starlark-rust; unknown fields give starlark-rust's
//! message.

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_bzlmod_model::AbridgedModule;
use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::ExternalDepsCode;
use blaze_bzlmod_model::ExternalDepsException;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::Tag;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryMapping;
use blaze_options::spelling::SpellChecker;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_packages::types::label_converter::SharedRepoMappingRecorder;
use blaze_starlark::error::StackEntry;
use blaze_starlark::label::LabelValue;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::environment::Module;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::FrozenHeap;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::AllocDict;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::attr_values::attribute_value_to_starlark;
use crate::bzlmod::module_extension::ModuleExtension;
use crate::bzlmod::tag_class::TagClass;
use crate::repository::attribute_utils::type_check_attr_values;

fn eval_error(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Converts an event location to a Starlark location (Java uses one `Location` class).
pub(crate) fn starlark_location(location: &blaze_events::Location) -> blaze_starlark::Location {
    blaze_starlark::Location {
        file: location.file().to_owned(),
        line: location.line().max(0) as usize,
        column: location.column().max(0) as usize,
    }
}

/// Allocates a stored attribute value on a frozen heap (Java: the immutable Starlark object).
pub(crate) fn alloc_frozen_attribute_value(
    value: &AttributeValue,
    heap: &FrozenHeap,
) -> FrozenValue {
    match value {
        AttributeValue::None => FrozenValue::new_none(),
        AttributeValue::Bool(b) => FrozenValue::new_bool(*b),
        AttributeValue::Int(i) => heap.alloc(*i),
        AttributeValue::String(s) => heap.alloc(s.as_str()),
        AttributeValue::Label(l) => heap.alloc(LabelValue(l.clone())),
        AttributeValue::List(items) => heap.alloc(
            items
                .iter()
                .map(|i| alloc_frozen_attribute_value(i, heap))
                .collect::<Vec<FrozenValue>>(),
        ),
        AttributeValue::Dict(entries) => heap.alloc(AllocDict(
            entries
                .iter()
                .map(|(k, v)| {
                    (
                        alloc_frozen_attribute_value(k, heap),
                        alloc_frozen_attribute_value(v, heap),
                    )
                })
                .collect::<Vec<(FrozenValue, FrozenValue)>>(),
        )),
    }
}

/// Identity hash of an immutable object (Java `Object.hashCode` of a value that is hashable
/// because `isImmutable()` is true).
fn write_identity_hash<T>(this: &T, hasher: &mut StarlarkHasher) {
    (this as *const T as usize).hash(hasher);
}

// ---------------------------------------------------------------------------------------------
// TypeCheckedTag.SortKey

/// An opaque object that can be used to sort tags in the order they are defined across all tag
/// classes within a module file and across modules in BFS order.
// Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:46 TypeCheckedTag.SortKey
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct SortKey {
    pub module_index: i32,
    pub tag_index: i32,
}

starlark_simple_value!(SortKey);

impl Ord for SortKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:56 TypeCheckedTag.SortKey.compareTo
    fn cmp(&self, other: &Self) -> Ordering {
        self.module_index
            .cmp(&other.module_index)
            .then_with(|| self.tag_index.cmp(&other.tag_index))
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for SortKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:61 TypeCheckedTag.SortKey.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<sort_key>")
    }
}

#[starlark_value(type = "sort_key")]
impl<'v> StarlarkValue<'v> for SortKey {
    // PORT: Java's `debugPrint` (used by `print()`); starlark-rust's `print()` uses `str()`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:66 TypeCheckedTag.SortKey.debugPrint
    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&format!(
            "<sort_key module={} tag={}>",
            self.module_index, self.tag_index
        ));
    }

    // Record equals.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other.downcast_ref::<SortKey>().is_some_and(|o| o == self))
    }

    fn compare(&self, other: Value<'v>) -> starlark::Result<Ordering> {
        match other.downcast_ref::<SortKey>() {
            Some(o) => Ok(self.cmp(o)),
            None => Err(eval_error(format!(
                "unsupported comparison: sort_key <=> {}",
                other.get_type()
            ))),
        }
    }

    // Record hashCode.
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.module_index.hash(hasher);
        self.tag_index.hash(hasher);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// TypeCheckedTag

/// A [`Tag`] whose attribute values have been type-checked against the attribute schema defined
/// in the [`TagClass`] (type `bazel_module_tag`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:39 TypeCheckedTag
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct TypeCheckedTag {
    #[allocative(skip)]
    tag_class: Arc<TagClass>,
    /// The type-checked values, in the order of the tag class's attributes, on the frozen heap.
    #[allocative(skip)]
    attr_values: Vec<FrozenValue>,
    dev_dependency: bool,
    /// The `SortKey` value on the frozen heap.
    #[allocative(skip)]
    sort_key: FrozenValue,

    // The properties below are only used for error reporting.
    #[allocative(skip)]
    location: blaze_events::Location,
    tag_class_name: String,
}

starlark_simple_value!(TypeCheckedTag);

impl TypeCheckedTag {
    /// Creates a [`TypeCheckedTag`], allocating its values on `heap`.
    ///
    /// PORT: the tag's stored attribute values are allocated on a temporary heap for
    /// `AttributeUtils.typeCheckAttrValues`, which type-checks Starlark values.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:97 TypeCheckedTag.create
    pub fn create(
        tag_class: &Arc<TagClass>,
        tag: &Tag,
        label_converter: &LabelConverter,
        module_display_string: &str,
        module_index: i32,
        tag_index: i32,
        heap: &FrozenHeap,
    ) -> Result<TypeCheckedTag, ExternalDepsException> {
        let call_stack = [StackEntry {
            name: "<toplevel>".to_owned(),
            location: Some(starlark_location(&tag.location)),
            source_line: None,
        }];
        let attr_values = Module::with_temp_heap(|module| {
            let temp_heap = module.heap();
            let kwargs: Vec<(String, Value)> = tag
                .attribute_values
                .attributes()
                .iter()
                .map(|(k, v)| (k.clone(), attribute_value_to_starlark(v, temp_heap)))
                .collect();
            type_check_attr_values(
                &tag_class.attributes,
                &tag_class.attribute_indices,
                &kwargs,
                label_converter,
                ExternalDepsCode::BadModule,
                &call_stack,
                &format!("'{}' tag", tag.tag_name),
                &format!("to the {module_display_string}"),
            )
        })?;
        Ok(TypeCheckedTag {
            tag_class: tag_class.clone(),
            attr_values: attr_values
                .iter()
                .map(|v| alloc_frozen_attribute_value(v, heap))
                .collect(),
            dev_dependency: tag.dev_dependency,
            sort_key: heap.alloc(SortKey {
                module_index,
                tag_index,
            }),
            location: tag.location.clone(),
            tag_class_name: tag.tag_name.clone(),
        })
    }

    /// Whether the tag was specified on an extension proxy created with
    /// `dev_dependency=True`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:129 TypeCheckedTag.isDevDependency
    pub fn is_dev_dependency(&self) -> bool {
        self.dev_dependency
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:153 TypeCheckedTag.getSortKey
    pub fn get_sort_key<'v>(&self) -> Value<'v> {
        self.sort_key.to_value()
    }

    /// `getValue(name)`: `None` for unknown attributes (Java null).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:140 TypeCheckedTag.getValue
    pub fn get_value<'v>(&self, name: &str) -> Option<Value<'v>> {
        let attr_index = *self.tag_class.attribute_indices.get(name)?;
        Some(self.attr_values[attr_index].to_value())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:149 TypeCheckedTag.getFieldNames
    pub fn get_field_names(&self) -> Vec<String> {
        self.tag_class.attribute_indices.keys().cloned().collect()
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:159 TypeCheckedTag.getErrorMessageForUnknownField
    pub fn get_error_message_for_unknown_field(&self, field: &str) -> String {
        format!(
            "unknown attribute {}{}",
            field,
            SpellChecker::did_you_mean(field, self.tag_class.attribute_indices.keys())
        )
    }
}

impl fmt::Debug for TypeCheckedTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TypeCheckedTag('{}' at {})",
            self.tag_class_name, self.location
        )
    }
}

impl fmt::Display for TypeCheckedTag {
    // The default `StarlarkValue.repr`.
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<unknown object com.google.devtools.build.lib.bazel.bzlmod.TypeCheckedTag>")
    }
}

#[starlark_value(type = "bazel_module_tag")]
impl<'v> StarlarkValue<'v> for TypeCheckedTag {
    // PORT: Java's `debugPrint` (used by `print()` and `fail()`); starlark-rust's `print()` uses
    // `str()`, so `str(tag)` also gives this form here.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TypeCheckedTag.java:164 TypeCheckedTag.debugPrint
    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&format!(
            "'{}' tag at {}",
            self.tag_class_name, self.location
        ));
    }

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.get_value(attribute)
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.tag_class.attribute_indices.contains_key(attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.get_field_names()
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        write_identity_hash(self, hasher);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// StarlarkBazelModule.Tags

/// Contains the tags in a module for the module extension currently being processed (type
/// `bazel_module_tags`): a field for each tag class of the extension, whose value is the list of
/// tags of that class.
// Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:66 StarlarkBazelModule.Tags
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct BazelModuleTags {
    /// Frozen lists of [`TypeCheckedTag`]s, in Java's `HashMap` iteration order.
    #[allocative(skip)]
    type_checked_tags: IndexMap<String, FrozenValue>,
}

starlark_simple_value!(BazelModuleTags);

impl BazelModuleTags {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:91 StarlarkBazelModule.Tags.getErrorMessageForUnknownField
    pub fn get_error_message_for_unknown_field(&self, field: &str) -> String {
        format!("unknown tag class {field}")
    }
}

impl fmt::Debug for BazelModuleTags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BazelModuleTags({:?})",
            self.type_checked_tags.keys().collect::<Vec<_>>()
        )
    }
}

impl fmt::Display for BazelModuleTags {
    // The default `StarlarkValue.repr`.
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.StarlarkBazelModule$Tags>",
        )
    }
}

#[starlark_value(type = "bazel_module_tags")]
impl<'v> StarlarkValue<'v> for BazelModuleTags {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:80 StarlarkBazelModule.Tags.getValue
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.type_checked_tags.get(attribute).map(|v| v.to_value())
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        self.type_checked_tags.contains_key(attribute)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:85 StarlarkBazelModule.Tags.getFieldNames
    fn dir_attr(&self) -> Vec<String> {
        self.type_checked_tags.keys().cloned().collect()
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        write_identity_hash(self, hasher);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// StarlarkBazelModule

/// A Starlark object representing a Bazel module in the external dependency graph (type
/// `bazel_module`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:45 StarlarkBazelModule
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkBazelModule {
    name: String,
    version: String,
    /// The [`BazelModuleTags`] value on the frozen heap.
    #[allocative(skip)]
    tags: FrozenValue,
    is_root_module: bool,
}

starlark_simple_value!(StarlarkBazelModule);

impl fmt::Debug for StarlarkBazelModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StarlarkBazelModule({}@{})", self.name, self.version)
    }
}

impl fmt::Display for StarlarkBazelModule {
    // The default `StarlarkValue.repr`.
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.bzlmod.StarlarkBazelModule>",
        )
    }
}

#[starlark_value(type = "bazel_module")]
impl<'v> StarlarkValue<'v> for StarlarkBazelModule {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("bazel_module", bazel_module_methods);
        Some(RES.methods())
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        write_identity_hash(self, hasher);
        Ok(())
    }
}

#[starlark_module]
fn bazel_module_methods(builder: &mut MethodsBuilder) {
    /// The name of the module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:166 StarlarkBazelModule.getName
    #[starlark(attribute)]
    fn name(this: &StarlarkBazelModule) -> starlark::Result<String> {
        Ok(this.name.clone())
    }

    /// The version of the module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:171 StarlarkBazelModule.getVersion
    #[starlark(attribute)]
    fn version(this: &StarlarkBazelModule) -> starlark::Result<String> {
        Ok(this.version.clone())
    }

    /// The tags in the module related to the module extension currently being processed.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:176 StarlarkBazelModule.getTags
    #[starlark(attribute)]
    fn tags<'v>(this: &StarlarkBazelModule) -> starlark::Result<Value<'v>> {
        Ok(this.tags.to_value())
    }

    /// Whether this module is the root module.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:184 StarlarkBazelModule.isRoot
    #[starlark(attribute)]
    fn is_root(this: &StarlarkBazelModule) -> starlark::Result<bool> {
        Ok(this.is_root_module)
    }
}

/// Java `String.hashCode`.
fn java_string_hash_code(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// The iteration order of a `java.util.HashMap<String, _>` filled with `keys` in order (default
/// capacity and load factor): indices into `keys`.
///
/// PORT: java-hash-order. Entries iterate by bucket (`(h ^ (h >>> 16)) & (capacity - 1)`), then
/// in insertion order within a bucket (resizing keeps that relative order). Treeified buckets (8+
/// collisions in a table of 64+ buckets) are not modelled.
pub(crate) fn java_hash_map_order(keys: &[String]) -> Vec<usize> {
    let mut capacity: usize = 16;
    while keys.len() > capacity / 4 * 3 {
        capacity *= 2;
    }
    let mut order: Vec<(usize, usize)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let h = java_string_hash_code(k) as u32;
            let spread = h ^ (h >> 16);
            ((spread as usize) & (capacity - 1), i)
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, i)| i).collect()
}

/// Creates a new `bazel_module` object representing the given [`AbridgedModule`], with its scope
/// limited to the given [`ModuleExtension`]. It'll be populated with the tags present in the
/// given [`ModuleExtensionUsage`]. Any labels present in tags will be converted using the given
/// [`RepositoryMapping`]. The object (and everything it contains) is allocated on `heap`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/StarlarkBazelModule.java:109 StarlarkBazelModule.create
pub fn create(
    module: &AbridgedModule,
    extension: &ModuleExtension,
    repo_mapping: &RepositoryMapping,
    usage: Option<&ModuleExtensionUsage>,
    repo_mapping_recorder: SharedRepoMappingRecorder,
    module_index: i32,
    heap: &FrozenHeap,
) -> Result<FrozenValue, ExternalDepsException> {
    let label_converter = LabelConverter::with_recorder(
        &PackageIdentifier::create(
            repo_mapping.context_repo().clone(),
            PathFragment::empty_fragment(),
        ),
        repo_mapping.clone(),
        repo_mapping_recorder,
    );
    let empty = Vec::new();
    let tags = usage.map(|u| &u.tags).unwrap_or(&empty);
    // Java HashMap<String, ArrayList<TypeCheckedTag>>.
    let tag_class_names: Vec<String> = extension.tag_classes.keys().cloned().collect();
    let mut type_checked_tags: IndexMap<String, Vec<FrozenValue>> = tag_class_names
        .iter()
        .map(|n| (n.clone(), Vec::new()))
        .collect();
    for (tag_index, tag) in tags.iter().enumerate() {
        let Some(tag_class) = extension.tag_classes.get(&tag.tag_name) else {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!(
                    "The module extension defined at {} does not have a tag class named {}, but \
                     its use is attempted at {}{}",
                    extension.location,
                    tag.tag_name,
                    tag.location,
                    SpellChecker::did_you_mean(&tag.tag_name, extension.tag_classes.keys())
                ),
            ));
        };

        // Now we need to type-check the attribute values and convert them into "build language
        // types" (for example, String to Label).
        let type_checked_tag = TypeCheckedTag::create(
            tag_class,
            tag,
            &label_converter,
            &module.key.to_display_string(),
            module_index,
            tag_index as i32,
            heap,
        )?;
        type_checked_tags
            .get_mut(&tag.tag_name)
            .expect("every tag class has an entry")
            .push(heap.alloc(type_checked_tag));
    }
    // Maps.transformValues(HashMap, StarlarkList::immutableCopyOf), copied into an ImmutableMap:
    // the HashMap's iteration order.
    let mut ordered: IndexMap<String, FrozenValue> = IndexMap::new();
    for i in java_hash_map_order(&tag_class_names) {
        let name = &tag_class_names[i];
        let list = heap.alloc(type_checked_tags[name].clone());
        ordered.insert(name.clone(), list);
    }
    let tags_value = heap.alloc(BazelModuleTags {
        type_checked_tags: ordered,
    });
    Ok(heap.alloc(StarlarkBazelModule {
        name: module.name.clone(),
        version: module.version.get_normalized().to_owned(),
        tags: tags_value,
        is_root_module: module.key.is_root(),
    }))
}
