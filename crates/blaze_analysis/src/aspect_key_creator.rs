//! Port of `skyframe/AspectKeyCreator.java` (`AspectBaseKey`, `AspectKey` with its two
//! subclasses, `TopLevelAspectsKey`), `skyframe/TopLevelAspectsValue.java`,
//! `skyframe/LoadAspectsKey.java` and `skyframe/LoadAspectsValue.java`.
//!
//! PORT: Java interns these keys (`SkyKeyInterner`); equality is structural either way, so the
//! interners are dropped. Java caches each key's `hashCode`; the Rust `Hash` impls hash the same
//! fields `equals` compares.
//!
//! PORT: Bazel 9.3.2 has no `BuildTopLevelAspectsDetailsFunction` (its key and value were
//! replaced by `LoadAspectsKey` / `LoadAspectsValue`, ported here).

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::ActionLookupKey;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_packages::aspect::{Aspect, AspectClass, AspectDescriptor, AspectParameters};
use blaze_skyframe::{Key, SkyFunctionName};
use blaze_util::check_argument;
use indexmap::IndexMap;

use crate::aspect_value::AspectValue;
use crate::configured_target_key::ConfiguredTargetKey;

/// `SkyFunctions.ASPECT`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:101 SkyFunctions.ASPECT
pub const ASPECT: SkyFunctionName = SkyFunctionName("ASPECT");

/// `SkyFunctions.TOP_LEVEL_ASPECTS`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:102 SkyFunctions.TOP_LEVEL_ASPECTS
pub const TOP_LEVEL_ASPECTS: SkyFunctionName = SkyFunctionName("TOP_LEVEL_ASPECTS");

/// `SkyFunctions.LOAD_ASPECTS`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:104 SkyFunctions.LOAD_ASPECTS
pub const LOAD_ASPECTS: SkyFunctionName = SkyFunctionName("LOAD_ASPECTS");

/// The class responsible for creating the various types of AspectKeys.
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:40 AspectKeyCreator
pub struct AspectKeyCreator;

impl AspectKeyCreator {
    /// Java `createAspectKey(AspectDescriptor, ConfiguredTargetKey)` (no base keys).
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:44 AspectKeyCreator.createAspectKey
    pub fn create_aspect_key_no_base_keys(
        aspect_descriptor: AspectDescriptor,
        base_configured_target_key: ConfiguredTargetKey,
    ) -> AspectKey {
        Self::create_aspect_key(
            aspect_descriptor,
            /* base_keys= */ Vec::new(),
            base_configured_target_key,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:50 AspectKeyCreator.createAspectKey
    pub fn create_aspect_key(
        aspect_descriptor: AspectDescriptor,
        base_keys: Vec<AspectKey>,
        base_configured_target_key: ConfiguredTargetKey,
    ) -> AspectKey {
        AspectKey::create_aspect_key(base_configured_target_key, base_keys, aspect_descriptor)
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:57 AspectKeyCreator.createTopLevelAspectsKey
    pub fn create_top_level_aspects_key(
        top_level_aspects_classes: Vec<AspectClass>,
        target_label: Label,
        configuration: Option<&BuildConfigurationValue>,
        top_level_aspects_parameters: IndexMap<String, String>,
    ) -> TopLevelAspectsKey {
        TopLevelAspectsKey::create_internal(
            top_level_aspects_classes,
            target_label.clone(),
            ConfiguredTargetKey::builder()
                .set_label(target_label)
                .set_configuration(configuration)
                .build(),
            top_level_aspects_parameters,
        )
    }
}

/// Free-function form of [`AspectKeyCreator::create_aspect_key`].
pub fn create_aspect_key(
    aspect_descriptor: AspectDescriptor,
    base_keys: Vec<AspectKey>,
    base_configured_target_key: ConfiguredTargetKey,
) -> AspectKey {
    AspectKeyCreator::create_aspect_key(aspect_descriptor, base_keys, base_configured_target_key)
}

/// Free-function form of [`AspectKeyCreator::create_top_level_aspects_key`].
pub fn create_top_level_aspects_key(
    top_level_aspects_classes: Vec<AspectClass>,
    target_label: Label,
    configuration: Option<&BuildConfigurationValue>,
    top_level_aspects_parameters: IndexMap<String, String>,
) -> TopLevelAspectsKey {
    AspectKeyCreator::create_top_level_aspects_key(
        top_level_aspects_classes,
        target_label,
        configuration,
        top_level_aspects_parameters,
    )
}

/// Common superclass for [`AspectKey`] and [`TopLevelAspectsKey`].
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:70 AspectKeyCreator.AspectBaseKey
pub trait AspectBaseKey: ActionLookupKey {
    /// Returns the key for the base configured target for this aspect.
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:80 AspectKeyCreator.AspectBaseKey.getBaseConfiguredTargetKey
    fn get_base_configured_target_key(&self) -> &ConfiguredTargetKey;
}

/// Java `ImmutableMap<String, String>.toString()`: `{a=b, c=d}`.
fn map_to_string(map: &IndexMap<String, String>) -> String {
    let entries: Vec<String> = map.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{{{}}}", entries.join(", "))
}

/// Java `List<T>.toString()`: `[a, b]`.
fn list_to_string<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    let parts: Vec<String> = items.iter().map(f).collect();
    format!("[{}]", parts.join(", "))
}

/// Hashes an `ImmutableMap<String, String>` consistently with its order-insensitive equality.
fn hash_unordered_map<H: Hasher>(map: &IndexMap<String, String>, state: &mut H) {
    let mut entries: Vec<(&String, &String)> = map.iter().collect();
    entries.sort();
    entries.len().hash(state);
    for (k, v) in entries {
        k.hash(state);
        v.hash(state);
    }
}

fn default_hash<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Represents an aspect applied to a particular target.
///
/// PORT: Java's two subclasses, `SimpleAspectKey` (no base keys; most aspects) and
/// `AspectKeyWithBaseAspects`, are one type distinguished by whether `base_keys` is empty, which
/// is exactly how Java's factory chooses the subclass. `Debug` prints the subclass's simple name
/// as Java's `toString` does.
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:101 AspectKeyCreator.AspectKey
#[derive(Clone)]
pub struct AspectKey(Arc<AspectKeyInner>);

struct AspectKeyInner {
    base_configured_target_key: ConfiguredTargetKey,
    aspect_descriptor: AspectDescriptor,
    /// Empty for Java's `SimpleAspectKey`; sorted for `AspectKeyWithBaseAspects`.
    base_keys: Vec<AspectKey>,
}

impl AspectKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:116 AspectKeyCreator.AspectKey.createAspectKey
    fn create_aspect_key(
        base_configured_target_key: ConfiguredTargetKey,
        base_keys: Vec<AspectKey>,
        aspect_descriptor: AspectDescriptor,
    ) -> AspectKey {
        if base_keys.is_empty() {
            return AspectKey(Arc::new(AspectKeyInner {
                base_configured_target_key,
                aspect_descriptor,
                base_keys: Vec::new(),
            }));
        }
        // Keep the list of `baseKeys` sorted to avoid running the same aspect twice because of
        // different `baseKeys` order even if the `AspectKey` objects in the list are the same.
        let mut sorted_base_keys = base_keys;
        // Java: ImmutableList.sortedCopyOf (a stable sort) comparing the aspect class name, then
        // (for aspects that appear more than once) the parameters' string representation.
        sorted_base_keys.sort_by(|a, b| {
            a.get_aspect_class()
                .get_name()
                .cmp(b.get_aspect_class().get_name())
                .then_with(|| {
                    a.get_parameters()
                        .to_string()
                        .cmp(&b.get_parameters().to_string())
                })
        });
        AspectKey(Arc::new(AspectKeyInner {
            base_configured_target_key,
            aspect_descriptor,
            base_keys: sorted_base_keys,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:147 AspectKeyCreator.AspectKey.getBaseKeys
    pub fn get_base_keys(&self) -> &[AspectKey] {
        &self.0.base_keys
    }

    /// Java `getDescription()` (also `getDescription(LabelPrinter)`, which ignores the printer).
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:276 AspectKeyCreator.AspectKey.SimpleAspectKey.getDescription
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:299 AspectKeyCreator.AspectKey.AspectKeyWithBaseAspects.getDescription
    pub fn get_description(&self) -> String {
        if self.0.base_keys.is_empty() {
            format!(
                "{} of {}",
                self.get_aspect_class().get_name(),
                self.get_label()
            )
        } else {
            format!(
                "{} on top of {}",
                self.get_aspect_class().get_name(),
                list_to_string(&self.0.base_keys, AspectKey::get_description)
            )
        }
    }

    /// Gets the name of the aspect that would be returned by the corresponding value's
    /// `aspectValue.getAspect().getAspectClass().getName()`, if the value could be produced.
    ///
    /// Only needed for reporting errors in BEP when the key's AspectValue fails evaluation.
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:167 AspectKeyCreator.AspectKey.getAspectName
    pub fn get_aspect_name(&self) -> String {
        self.0.aspect_descriptor.get_description()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:172 AspectKeyCreator.AspectKey.getLabel
    pub fn get_label(&self) -> &Label {
        self.0.base_configured_target_key.get_label()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:186 AspectKeyCreator.AspectKey.getAspectClass
    pub fn get_aspect_class(&self) -> &AspectClass {
        self.0.aspect_descriptor.get_aspect_class()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:191 AspectKeyCreator.AspectKey.getParameters
    pub fn get_parameters(&self) -> &AspectParameters {
        self.0.aspect_descriptor.get_parameters()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:195 AspectKeyCreator.AspectKey.getAspectDescriptor
    pub fn get_aspect_descriptor(&self) -> &AspectDescriptor {
        &self.0.aspect_descriptor
    }

    /// Returns the key of the configured target of the aspect; that is, the configuration in
    /// which the aspect will be evaluated.
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:205 AspectKeyCreator.AspectKey.getConfigurationKey
    pub fn get_configuration_key(&self) -> Option<&BuildConfigurationKey> {
        self.0.base_configured_target_key.get_configuration_key()
    }

    /// Returns the key for the base configured target for this aspect.
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:80 AspectKeyCreator.AspectBaseKey.getBaseConfiguredTargetKey
    pub fn get_base_configured_target_key(&self) -> &ConfiguredTargetKey {
        &self.0.base_configured_target_key
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:223 AspectKeyCreator.AspectKey.prettyPrint
    pub fn pretty_print(&self) -> String {
        // PORT: Java's `getLabel() == null` branch ("null") cannot occur: a ConfiguredTargetKey
        // always has a label.
        let base_keys_string = if self.0.base_keys.is_empty() {
            String::new()
        } else {
            format!(
                " (over {})",
                list_to_string(&self.0.base_keys, |k| format!("{k:?}"))
            )
        };
        format!(
            "{} with aspect {}{}",
            self.get_label(),
            self.0.aspect_descriptor.get_aspect_class().get_name(),
            base_keys_string
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:249 AspectKeyCreator.AspectKey.withLabel
    pub fn with_label(&self, label: &Label) -> AspectKey {
        let new_base_keys: Vec<AspectKey> = self
            .0
            .base_keys
            .iter()
            .map(|k| k.with_label(label))
            .collect();

        AspectKey::create_aspect_key(
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(
                    self.0
                        .base_configured_target_key
                        .get_configuration_key()
                        .cloned(),
                )
                .build(),
            new_base_keys,
            self.0.aspect_descriptor.clone(),
        )
    }

    /// Java's subclass simple name (`MoreObjects.toStringHelper(this)`).
    fn java_class_simple_name(&self) -> &'static str {
        if self.0.base_keys.is_empty() {
            "SimpleAspectKey"
        } else {
            "AspectKeyWithBaseAspects"
        }
    }
}

impl PartialEq for AspectKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:210 AspectKeyCreator.AspectKey.equals
    fn eq(&self, other: &AspectKey) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.base_keys == other.0.base_keys
            && self.0.base_configured_target_key == other.0.base_configured_target_key
            && self.0.aspect_descriptor == other.0.aspect_descriptor
    }
}

impl Eq for AspectKey {}

impl Hash for AspectKey {
    // Java: HashCodes.hashObjects(baseConfiguredTargetKey, [sortedBaseKeys,] aspectDescriptor).
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.base_configured_target_key.hash(state);
        self.0.base_keys.hash(state);
        self.0.aspect_descriptor.hash(state);
    }
}

impl fmt::Debug for AspectKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:236 AspectKeyCreator.AspectKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{{baseConfiguredTargetKey={:?}, aspectDescriptor={}",
            self.java_class_simple_name(),
            self.0.base_configured_target_key,
            self.0.aspect_descriptor
        )?;
        if !self.0.base_keys.is_empty() {
            write!(
                f,
                ", baseKeys={}",
                list_to_string(&self.0.base_keys, |k| format!("{k:?}"))
            )?;
        }
        f.write_str("}")
    }
}

impl fmt::Display for AspectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Key for AspectKey {
    type Value = AspectValue;
    const FUNCTION: SkyFunctionName = ASPECT;
}

impl AspectBaseKey for AspectKey {
    fn get_base_configured_target_key(&self) -> &ConfiguredTargetKey {
        &self.0.base_configured_target_key
    }
}

impl ActionLookupKey for AspectKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:172 AspectKeyCreator.AspectKey.getLabel
    fn get_label(&self) -> Option<Label> {
        Some(AspectKey::get_label(self).clone())
    }

    fn get_configuration_checksum(&self) -> Option<String> {
        self.0
            .base_configured_target_key
            .get_configuration_checksum()
            .map(str::to_owned)
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:157 AspectKeyCreator.AspectKey.functionName
    fn function_name(&self) -> &'static str {
        ASPECT.0
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<AspectKey>()
            .is_some_and(|o| o == self)
    }

    fn key_hash(&self) -> u64 {
        default_hash(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The key for top level aspects specified by `--aspects` option and their parameters specified
/// by `--aspects_parameters` applied on a top level target.
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:313 AspectKeyCreator.TopLevelAspectsKey
#[derive(Clone)]
pub struct TopLevelAspectsKey(Arc<TopLevelAspectsKeyInner>);

struct TopLevelAspectsKeyInner {
    top_level_aspects_classes: Vec<AspectClass>,
    target_label: Label,
    base_configured_target_key: ConfiguredTargetKey,
    top_level_aspects_parameters: IndexMap<String, String>,
}

impl TopLevelAspectsKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:322 AspectKeyCreator.TopLevelAspectsKey.createInternal
    pub fn create_internal(
        top_level_aspects_classes: Vec<AspectClass>,
        target_label: Label,
        base_configured_target_key: ConfiguredTargetKey,
        top_level_aspects_parameters: IndexMap<String, String>,
    ) -> TopLevelAspectsKey {
        TopLevelAspectsKey(Arc::new(TopLevelAspectsKeyInner {
            top_level_aspects_classes,
            target_label,
            base_configured_target_key,
            top_level_aspects_parameters,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:358 AspectKeyCreator.TopLevelAspectsKey.getConfigurationKey
    pub fn get_configuration_key(&self) -> Option<&BuildConfigurationKey> {
        self.0.base_configured_target_key.get_configuration_key()
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:362 AspectKeyCreator.TopLevelAspectsKey.getTopLevelAspectsClasses
    pub fn get_top_level_aspects_classes(&self) -> &[AspectClass] {
        &self.0.top_level_aspects_classes
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:366 AspectKeyCreator.TopLevelAspectsKey.getTopLevelAspectsParameters
    pub fn get_top_level_aspects_parameters(&self) -> &IndexMap<String, String> {
        &self.0.top_level_aspects_parameters
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:371 AspectKeyCreator.TopLevelAspectsKey.getLabel
    pub fn get_label(&self) -> &Label {
        &self.0.target_label
    }

    /// Returns the key for the base configured target for these aspects.
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:80 AspectKeyCreator.AspectBaseKey.getBaseConfiguredTargetKey
    pub fn get_base_configured_target_key(&self) -> &ConfiguredTargetKey {
        &self.0.base_configured_target_key
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:375 AspectKeyCreator.TopLevelAspectsKey.getDescription
    pub fn get_description(&self) -> String {
        // PORT: `AspectClass.toString()` (the Starlark class's name; see AspectClass Display).
        format!(
            "{} with parameters {} on {}",
            list_to_string(&self.0.top_level_aspects_classes, |c| c.to_string()),
            map_to_string(&self.0.top_level_aspects_parameters),
            self.0.target_label
        )
    }
}

impl PartialEq for TopLevelAspectsKey {
    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:382 AspectKeyCreator.TopLevelAspectsKey.equals
    fn eq(&self, other: &TopLevelAspectsKey) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.target_label == other.0.target_label
            && self.0.base_configured_target_key == other.0.base_configured_target_key
            && self.0.top_level_aspects_classes == other.0.top_level_aspects_classes
            // Java: ImmutableMap.equals ignores entry order (as IndexMap's PartialEq does).
            && self.0.top_level_aspects_parameters == other.0.top_level_aspects_parameters
    }
}

impl Eq for TopLevelAspectsKey {}

impl Hash for TopLevelAspectsKey {
    // Java: HashCodes.hashObjects(topLevelAspectsClasses, targetLabel, baseConfiguredTargetKey,
    // topLevelAspectsParameters).
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.top_level_aspects_classes.hash(state);
        self.0.target_label.hash(state);
        self.0.base_configured_target_key.hash(state);
        hash_unordered_map(&self.0.top_level_aspects_parameters, state);
    }
}

impl fmt::Debug for TopLevelAspectsKey {
    // PORT: Java `TopLevelAspectsKey` does not override `toString()` (`Object.toString()`, not
    // reproducible); the description stands for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TopLevelAspectsKey{{{}}}", self.get_description())
    }
}

impl fmt::Display for TopLevelAspectsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Key for TopLevelAspectsKey {
    type Value = TopLevelAspectsValue;
    const FUNCTION: SkyFunctionName = TOP_LEVEL_ASPECTS;
}

impl AspectBaseKey for TopLevelAspectsKey {
    fn get_base_configured_target_key(&self) -> &ConfiguredTargetKey {
        &self.0.base_configured_target_key
    }
}

impl ActionLookupKey for TopLevelAspectsKey {
    fn get_label(&self) -> Option<Label> {
        Some(self.0.target_label.clone())
    }

    fn get_configuration_checksum(&self) -> Option<String> {
        self.0
            .base_configured_target_key
            .get_configuration_checksum()
            .map(str::to_owned)
    }

    // Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:353 AspectKeyCreator.TopLevelAspectsKey.functionName
    fn function_name(&self) -> &'static str {
        TOP_LEVEL_ASPECTS.0
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<TopLevelAspectsKey>()
            .is_some_and(|o| o == self)
    }

    fn key_hash(&self) -> u64 {
        default_hash(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// SkyValue for `TopLevelAspectsKey`: wraps the `AspectValue`s of the top level aspects applied
/// on the same top level target.
// Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:30 TopLevelAspectsValue
#[derive(Clone, Debug)]
pub struct TopLevelAspectsValue {
    top_level_aspects_map: IndexMap<AspectKey, Arc<AspectValue>>,
}

impl TopLevelAspectsValue {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:33 TopLevelAspectsValue.<init>
    pub fn new(top_level_aspects_map: IndexMap<AspectKey, Arc<AspectValue>>) -> Self {
        TopLevelAspectsValue {
            top_level_aspects_map,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:37 TopLevelAspectsValue.getTopLevelAspectsMap
    pub fn get_top_level_aspects_map(&self) -> &IndexMap<AspectKey, Arc<AspectValue>> {
        &self.top_level_aspects_map
    }

    /// Java `getActions()`: a `TopLevelAspectsValue` owns no actions.
    // Java: com/google/devtools/build/lib/skyframe/TopLevelAspectsValue.java:42 TopLevelAspectsValue.getActions
    pub fn get_actions(&self) -> &[blaze_actions::ActionRef] {
        &[]
    }
}

/// `SkyKey` for loading the top-level aspects (`--aspects`) and their parameters.
// Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:32 LoadAspectsKey
#[derive(Clone)]
pub struct LoadAspectsKey(Arc<LoadAspectsKeyInner>);

struct LoadAspectsKeyInner {
    top_level_aspects_classes: Vec<AspectClass>,
    top_level_aspects_parameters: IndexMap<String, String>,
}

impl LoadAspectsKey {
    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:39 LoadAspectsKey.create
    pub fn create(
        top_level_aspects_classes: Vec<AspectClass>,
        top_level_aspects_parameters: IndexMap<String, String>,
    ) -> LoadAspectsKey {
        // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:55 LoadAspectsKey.<init>
        check_argument!(!top_level_aspects_classes.is_empty(), "No aspects");
        LoadAspectsKey(Arc::new(LoadAspectsKeyInner {
            top_level_aspects_classes,
            top_level_aspects_parameters,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:70 LoadAspectsKey.getTopLevelAspectsClasses
    pub fn get_top_level_aspects_classes(&self) -> &[AspectClass] {
        &self.0.top_level_aspects_classes
    }

    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:74 LoadAspectsKey.getTopLevelAspectsParameters
    pub fn get_top_level_aspects_parameters(&self) -> &IndexMap<String, String> {
        &self.0.top_level_aspects_parameters
    }
}

impl PartialEq for LoadAspectsKey {
    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:84 LoadAspectsKey.equals
    fn eq(&self, other: &LoadAspectsKey) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.top_level_aspects_classes == other.0.top_level_aspects_classes
            && self.0.top_level_aspects_parameters == other.0.top_level_aspects_parameters
    }
}

impl Eq for LoadAspectsKey {}

impl Hash for LoadAspectsKey {
    // Java: Objects.hashCode(topLevelAspectsClasses, topLevelAspectsParameters).
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.top_level_aspects_classes.hash(state);
        hash_unordered_map(&self.0.top_level_aspects_parameters, state);
    }
}

impl fmt::Debug for LoadAspectsKey {
    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsKey.java:97 LoadAspectsKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LoadAspectsKey{{topLevelAspectsClasses={}, topLevelAspectsParameters={}}}",
            list_to_string(&self.0.top_level_aspects_classes, |c| c.to_string()),
            map_to_string(&self.0.top_level_aspects_parameters)
        )
    }
}

impl fmt::Display for LoadAspectsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Key for LoadAspectsKey {
    type Value = LoadAspectsValue;
    const FUNCTION: SkyFunctionName = LOAD_ASPECTS;
}

/// SkyValue for `LoadAspectsKey`: the `Aspect`s of the top level aspects.
// Java: com/google/devtools/build/lib/skyframe/LoadAspectsValue.java:25 LoadAspectsValue
#[derive(Clone)]
pub struct LoadAspectsValue {
    aspects: Vec<Aspect>,
}

impl LoadAspectsValue {
    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsValue.java:28 LoadAspectsValue.<init>
    pub fn new(aspects: Vec<Aspect>) -> Self {
        LoadAspectsValue { aspects }
    }

    // Java: com/google/devtools/build/lib/skyframe/LoadAspectsValue.java:32 LoadAspectsValue.getAspects
    pub fn get_aspects(&self) -> &[Aspect] {
        &self.aspects
    }
}

impl fmt::Debug for LoadAspectsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadAspectsValue")
            .field("aspects", &self.aspects)
            .finish()
    }
}
