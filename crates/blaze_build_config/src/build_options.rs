//! Port of `analysis/config/BuildOptions.java`, `Scope.ScopeType` and `BuildOptionsView`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use blaze_cmdline::Label;
use blaze_options::definition::{OptionsBase, map_to_cache_key};
use blaze_options::value::{OptionScalar, java_list_to_string};
use blaze_options::{OptionsClass, OptionsParser, OptionsParsingException, Value};
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};

use crate::fragment_options_class::{FragmentOptionsClass, fragment_options_class};

// Java: com/google/devtools/build/lib/analysis/config/Scope.java:31 Scope.ScopeType
/// Type of supported scopes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum ScopeType {
    /// The flag's value never changes except explicitly by a configuration transition.
    UNIVERSAL,
    /// The flag's value resets on exec transitions.
    TARGET,
    /// The flag resets on targets outside the flag's project. See PROJECT.scl.
    PROJECT,
    /// Placeholder for flags that don't explicitly specify scope. Shouldn't be set directly.
    DEFAULT,
}

impl ScopeType {
    pub fn name(&self) -> &'static str {
        match self {
            ScopeType::UNIVERSAL => "UNIVERSAL",
            ScopeType::TARGET => "TARGET",
            ScopeType::PROJECT => "PROJECT",
            ScopeType::DEFAULT => "DEFAULT",
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:49 Scope.ScopeType.valueOfIgnoreCase
    /// Returns the enum of a `scope = "<string>"` value.
    pub fn value_of_ignore_case(scope_type: &str) -> Option<ScopeType> {
        match scope_type.to_uppercase().as_str() {
            "UNIVERSAL" => Some(ScopeType::UNIVERSAL),
            "TARGET" => Some(ScopeType::TARGET),
            "PROJECT" => Some(ScopeType::PROJECT),
            "DEFAULT" => Some(ScopeType::DEFAULT),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:54 Scope.ScopeType.allowedAttributeValues
    /// Which values can a rule's `scope` attribute have?
    pub fn allowed_attribute_values() -> Vec<String> {
        [
            ScopeType::UNIVERSAL,
            ScopeType::TARGET,
            ScopeType::PROJECT,
            ScopeType::DEFAULT,
        ]
        .iter()
        .map(|e| e.name().to_lowercase())
        // "default" is an internal value for unset attributes.
        .filter(|e| e != "default")
        .collect()
    }
}

// Java: com/google/devtools/build/lib/analysis/config/Scope.java:67 Scope.ScopeDefinition
/// Definition of a scope. Users can define this in their PROJECT.scl file that is in the same
/// directory as the BUILD file where the scoped flags are defined or in a parent directory. This
/// is only relevant if the scope type is PROJECT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeDefinition {
    owned_code_paths: IndexSet<String>,
}

impl ScopeDefinition {
    pub fn new(owned_code_paths: IndexSet<String>) -> Self {
        ScopeDefinition { owned_code_paths }
    }

    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:74 Scope.ScopeDefinition.getOwnedCodePaths
    pub fn get_owned_code_paths(&self) -> &IndexSet<String> {
        &self.owned_code_paths
    }
}

// Java: com/google/devtools/build/lib/analysis/config/Scope.java:29 Scope
/// Scope of a `BuildOptions` is defined by the `Scope.ScopeType` and `Scope.ScopeDefinition`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    scope_type: ScopeType,
    scope_definition: Option<ScopeDefinition>,
}

impl Scope {
    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:87 Scope.Scope
    pub fn new(scope_type: ScopeType, scope_definition: Option<ScopeDefinition>) -> Self {
        Scope {
            scope_type,
            scope_definition,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:92 Scope.getScopeType
    pub fn get_scope_type(&self) -> ScopeType {
        self.scope_type
    }

    // Java: com/google/devtools/build/lib/analysis/config/Scope.java:97 Scope.getScopeDefinition
    pub fn get_scope_definition(&self) -> Option<&ScopeDefinition> {
        self.scope_definition.as_ref()
    }
}

impl OptionScalar for ScopeType {
    /// Java `Enum.toString()` is `name()`.
    fn java_to_string(&self) -> String {
        self.name().to_string()
    }
}

/// The Java class of a collection-valued Starlark option. The class decides `toString()` (which
/// feeds `BuildOptions.checksum` and the ST hash) and `equals()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JavaCollectionClass {
    /// A `java.util.List` that is not a Starlark value (`ImmutableList` from an option converter
    /// or an attribute value, `ArrayList` of an `allow_multiple`/`repeatable` flag).
    /// `toString()` is `[a, b]`; `equals` is element-wise with any `java.util.List`.
    List,
    /// `net.starlark.java.eval.StarlarkList` (a list a transition returned). `toString()` is the
    /// Starlark `repr` (`["a", "b"]`); `equals` only with another `StarlarkList`.
    StarlarkList,
    /// `net.starlark.java.eval.Tuple` (a tuple a transition returned). `toString()` is the
    /// Starlark `repr` (`("a", "b")`); `equals` only with another `Tuple`.
    Tuple,
    /// A `java.util.Set` (`ImmutableSet` of a `string_set` setting, `LinkedHashSet` of a
    /// repeatable one). `toString()` is `[a, b]`; `equals` only with another `Set`.
    Set,
}

/// A collection-valued Starlark option: its Java class and elements (in iteration order).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StarlarkOptionList {
    class: JavaCollectionClass,
    items: Vec<StarlarkOptionValue>,
}

impl StarlarkOptionList {
    pub fn new(class: JavaCollectionClass, items: Vec<StarlarkOptionValue>) -> Self {
        StarlarkOptionList { class, items }
    }

    /// The Java class of the collection.
    pub fn class(&self) -> JavaCollectionClass {
        self.class
    }

    pub fn items(&self) -> &[StarlarkOptionValue] {
        &self.items
    }

    pub fn into_items(self) -> Vec<StarlarkOptionValue> {
        self.items
    }

    /// `value instanceof java.util.List` (`StarlarkList` and `Tuple` implement `List`).
    pub fn is_java_list(&self) -> bool {
        self.class != JavaCollectionClass::Set
    }
}

impl std::ops::Deref for StarlarkOptionList {
    type Target = [StarlarkOptionValue];
    fn deref(&self) -> &[StarlarkOptionValue] {
        &self.items
    }
}

/// A plain `java.util.List` ([`JavaCollectionClass::List`]).
impl From<Vec<StarlarkOptionValue>> for StarlarkOptionList {
    fn from(items: Vec<StarlarkOptionValue>) -> Self {
        StarlarkOptionList::new(JavaCollectionClass::List, items)
    }
}

/// A plain `java.util.List` ([`JavaCollectionClass::List`]).
impl FromIterator<StarlarkOptionValue> for StarlarkOptionList {
    fn from_iter<I: IntoIterator<Item = StarlarkOptionValue>>(iter: I) -> Self {
        StarlarkOptionList::new(JavaCollectionClass::List, iter.into_iter().collect())
    }
}

/// The value of a Starlark build setting stored in `BuildOptions` (Java `Object`).
///
/// PORT: Java stores whatever object the producer made: the build setting converter's result on
/// the command line (`Boolean`, `StarlarkInt`, `String`, `Label`, `ImmutableList`, `ImmutableSet`,
/// `ArrayList`/`LinkedHashSet` for flags that allow multiple values), the attribute value for a
/// default, or the raw Starlark value a transition returned (`StarlarkList`, `Tuple`). Each
/// variant keeps that Java class's `toString()` and `equals()`.
///
/// `==` is structural (same Java class, same elements). Where Java calls `a.equals(b)` use
/// [`StarlarkOptionValue::java_equals`], which is not symmetric across collection classes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StarlarkOptionValue {
    /// `java.lang.Boolean`.
    Bool(bool),
    /// `StarlarkInt`.
    ///
    /// PORT: Java's `StarlarkInt` is unbounded; values beyond 64 bits are not representable here
    /// and their producers fail loudly (`unported!("StarlarkInt.parse")`).
    Int(i64),
    /// `java.lang.String`.
    Str(String),
    /// `Label` (`toString()` is the canonical form).
    Label(Label),
    /// A Java collection; see [`JavaCollectionClass`].
    List(StarlarkOptionList),
}

impl StarlarkOptionValue {
    /// A `java.util.List` that is not a Starlark value (`ImmutableList`, `ArrayList`).
    pub fn list(items: Vec<StarlarkOptionValue>) -> StarlarkOptionValue {
        StarlarkOptionValue::List(StarlarkOptionList::new(JavaCollectionClass::List, items))
    }

    /// A `StarlarkList`.
    pub fn starlark_list(items: Vec<StarlarkOptionValue>) -> StarlarkOptionValue {
        StarlarkOptionValue::List(StarlarkOptionList::new(
            JavaCollectionClass::StarlarkList,
            items,
        ))
    }

    /// A Starlark `Tuple`.
    pub fn tuple(items: Vec<StarlarkOptionValue>) -> StarlarkOptionValue {
        StarlarkOptionValue::List(StarlarkOptionList::new(JavaCollectionClass::Tuple, items))
    }

    /// A `java.util.Set` (`ImmutableSet`, `LinkedHashSet`): `items` without duplicates, in
    /// insertion order.
    pub fn set(items: Vec<StarlarkOptionValue>) -> StarlarkOptionValue {
        let mut distinct: Vec<StarlarkOptionValue> = Vec::with_capacity(items.len());
        for item in items {
            if !distinct.contains(&item) {
                distinct.push(item);
            }
        }
        StarlarkOptionValue::List(StarlarkOptionList::new(JavaCollectionClass::Set, distinct))
    }

    /// Java `String.valueOf(value)` (`value.toString()`).
    pub fn java_to_string(&self) -> String {
        match self {
            StarlarkOptionValue::Bool(b) => b.to_string(),
            StarlarkOptionValue::Int(i) => i.to_string(),
            StarlarkOptionValue::Str(s) => s.clone(),
            StarlarkOptionValue::Label(l) => l.to_string(),
            StarlarkOptionValue::List(l) => match l.class() {
                // Java: java/util/AbstractCollection.java AbstractCollection.toString
                JavaCollectionClass::List | JavaCollectionClass::Set => {
                    java_list_to_string(l.iter().map(StarlarkOptionValue::java_to_string))
                }
                // Java: net/starlark/java/eval/StarlarkList.java:272 StarlarkList.toString
                // Java: net/starlark/java/eval/Tuple.java:139 Tuple.toString
                JavaCollectionClass::StarlarkList | JavaCollectionClass::Tuple => {
                    self.starlark_repr()
                }
            },
        }
    }

    /// Java `Starlark.repr(value)`.
    // Java: net/starlark/java/eval/Printer.java:160 Printer.repr
    pub fn starlark_repr(&self) -> String {
        match self {
            StarlarkOptionValue::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            StarlarkOptionValue::Int(i) => i.to_string(),
            StarlarkOptionValue::Str(s) => blaze_starlark::printer::quote(s),
            // Java: com/google/devtools/build/lib/cmdline/Label.java:718 Label.repr
            StarlarkOptionValue::Label(l) => format!(
                "Label({})",
                blaze_starlark::printer::quote(&l.get_canonical_form())
            ),
            StarlarkOptionValue::List(l) => {
                let elems: Vec<String> = l.iter().map(StarlarkOptionValue::starlark_repr).collect();
                match l.class() {
                    // Java: net/starlark/java/eval/StarlarkList.java:266 StarlarkList.repr; a
                    // non-Starlark `List` prints the same way (Printer.java:203).
                    JavaCollectionClass::StarlarkList | JavaCollectionClass::List => {
                        format!("[{}]", elems.join(", "))
                    }
                    // Java: net/starlark/java/eval/RegularTuple.java:108 RegularTuple.repr and
                    // SingletonTuple.repr (`(x,)`).
                    JavaCollectionClass::Tuple => {
                        if elems.len() == 1 {
                            format!("({},)", elems[0])
                        } else {
                            format!("({})", elems.join(", "))
                        }
                    }
                    // Java: net/starlark/java/eval/Printer.java:216 Printer.repr (other
                    // non-Starlark values print their `toString()`).
                    JavaCollectionClass::Set => self.java_to_string(),
                }
            }
        }
    }

    /// Java `this.equals(other)`.
    ///
    /// Not symmetric: an `ImmutableList` equals a `StarlarkList` with the same elements
    /// (`ImmutableList.equals` accepts any `java.util.List`), but a `StarlarkList` only equals
    /// another `StarlarkList` (`net/starlark/java/eval/StarlarkList.java:244`), and a `Tuple`
    /// only another `Tuple` (`net/starlark/java/eval/Tuple.java:129`).
    pub fn java_equals(&self, other: &StarlarkOptionValue) -> bool {
        match (self, other) {
            (StarlarkOptionValue::Bool(a), StarlarkOptionValue::Bool(b)) => a == b,
            (StarlarkOptionValue::Int(a), StarlarkOptionValue::Int(b)) => a == b,
            (StarlarkOptionValue::Str(a), StarlarkOptionValue::Str(b)) => a == b,
            (StarlarkOptionValue::Label(a), StarlarkOptionValue::Label(b)) => a == b,
            (StarlarkOptionValue::List(a), StarlarkOptionValue::List(b)) => {
                let same_elems =
                    || a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.java_equals(y));
                match a.class() {
                    JavaCollectionClass::List => b.is_java_list() && same_elems(),
                    JavaCollectionClass::StarlarkList => {
                        b.class() == JavaCollectionClass::StarlarkList && same_elems()
                    }
                    JavaCollectionClass::Tuple => {
                        b.class() == JavaCollectionClass::Tuple && same_elems()
                    }
                    // Java: java/util/AbstractSet.java AbstractSet.equals
                    JavaCollectionClass::Set => {
                        b.class() == JavaCollectionClass::Set
                            && a.len() == b.len()
                            && b.iter().all(|y| a.iter().any(|x| y.java_equals(x)))
                    }
                }
            }
            _ => false,
        }
    }

    /// Java `Objects.equals(a, b)` for nullable values.
    pub fn java_objects_equals(
        a: Option<&StarlarkOptionValue>,
        b: Option<&StarlarkOptionValue>,
    ) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.java_equals(b),
            _ => false,
        }
    }

    /// `value instanceof List<?> && ((List<?>) value).isEmpty()`.
    fn is_empty_java_list(&self) -> bool {
        matches!(self, StarlarkOptionValue::List(l) if l.is_java_list() && l.is_empty())
    }

    /// The `BuildOptions` form of a value an `OptionsProvider` reports as a Starlark option
    /// (Java: the same `Object`).
    ///
    /// PORT: `OptionsParser.setStarlarkOptions` takes type-erased option values. The Starlark
    /// options parser stores [`StarlarkOptionValue`]s; plain option values (tests of the options
    /// framework) are classified by their Java class.
    pub fn from_option_value(value: &Value) -> StarlarkOptionValue {
        if let Some(v) = value.downcast_ref::<StarlarkOptionValue>() {
            return v.clone();
        }
        if let Some(b) = value.downcast_ref::<bool>() {
            return StarlarkOptionValue::Bool(*b);
        }
        if let Some(s) = value.downcast_ref::<String>() {
            return StarlarkOptionValue::Str(s.clone());
        }
        if let Some(i) = value.downcast_ref::<i32>() {
            return StarlarkOptionValue::Int(i64::from(*i));
        }
        if let Some(i) = value.downcast_ref::<i64>() {
            return StarlarkOptionValue::Int(*i);
        }
        if let Some(l) = value.downcast_ref::<blaze_options::Foreign<Label>>() {
            return StarlarkOptionValue::Label(l.0.clone());
        }
        if let Some(list) = value.as_list() {
            return StarlarkOptionValue::list(
                list.iter()
                    .map(StarlarkOptionValue::from_option_value)
                    .collect(),
            );
        }
        blaze_util::unported!(
            "BuildOptions.labelizeStarlarkOptions",
            "Starlark option value {:?}",
            value
        )
    }
}

impl OptionScalar for StarlarkOptionValue {
    fn java_to_string(&self) -> String {
        StarlarkOptionValue::java_to_string(self)
    }
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:70 BuildOptions
/// Stores the command-line options from a set of configuration fragments.
///
/// PORT: Java's `FragmentOptions` are mutable and `BuildOptions` hands them out; callers clone
/// before mutating ("BuildOptions instances may be mutated after construction but before
/// consumption"). The Rust port owns each fragment and exposes mutation through
/// [`BuildOptions::get_mut`], which, like Java, does not reset an already computed checksum.
pub struct BuildOptions {
    /// Maps options class definitions to FragmentOptions objects, sorted by Java class name.
    fragment_options_map: IndexMap<FragmentOptionsClass, Box<dyn OptionsBase>>,
    /// Maps Starlark options names to Starlark options values. This should never contain an
    /// entry for a Starlark option and the default value: if a Starlark option is explicitly or
    /// implicitly set to the default it should be removed from this map so that configurations
    /// are not duplicated needlessly.
    starlark_options_map: IndexMap<Label, StarlarkOptionValue>,
    /// Maps Starlark options names to `Scope` information.
    scopes: IndexMap<Label, ScopeType>,
    /// Lazily initialized; see `checksum()`.
    checksum: OnceLock<String>,
}

impl BuildOptions {
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:97 BuildOptions.getDefaultBuildOptionsForFragments
    pub fn get_default_build_options_for_fragments(
        fragment_classes: &[FragmentOptionsClass],
    ) -> BuildOptions {
        match BuildOptions::of_args(fragment_classes, &[]) {
            Ok(o) => o,
            Err(e) => panic!(
                "IllegalArgumentException: Failed to parse empty options: {}",
                e.get_message()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:110 BuildOptions.of
    /// Creates a BuildOptions class by taking the option values from an options provider (eg. an
    /// OptionsParser).
    pub fn of(options_list: &[FragmentOptionsClass], provider: &OptionsParser) -> BuildOptions {
        let mut builder = BuildOptions::builder();
        for options_class in options_list {
            let options = provider
                .get_options_dyn(options_class.options_class())
                .unwrap_or_else(|| panic!("NullPointerException: no {:?}", options_class));
            builder.add_fragment_options_dyn(options_class, options.as_ref());
        }
        builder
            .add_starlark_options(BuildOptions::labelize_starlark_options(
                provider.get_starlark_options(),
            ))
            .add_scope_type_map(BuildOptions::convert_scopes_attributes(
                provider.get_scopes_attributes(),
                provider.get_starlark_options(),
            ))
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:76 BuildOptions.labelizeStarlarkOptions
    pub fn labelize_starlark_options(
        starlark_options: &BTreeMap<String, Value>,
    ) -> IndexMap<Label, StarlarkOptionValue> {
        let mut result: IndexMap<Label, StarlarkOptionValue> = IndexMap::new();
        for (key, value) in starlark_options {
            let label = Label::parse_canonical_unchecked(key);
            // Collectors.toMap throws on a duplicate key.
            if result
                .insert(label, StarlarkOptionValue::from_option_value(value))
                .is_some()
            {
                panic!("IllegalStateException: Duplicate key {}", key);
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:87 BuildOptions.convertScopesAttributes
    /// Converts the map containing String representation of scopes attributes to a map of
    /// `Label` of Starlark options to their corresponding `Scope.ScopeType`, keeping only
    /// entries for flags present in `starlark_options`.
    pub fn convert_scopes_attributes(
        scopes_attributes: &BTreeMap<String, String>,
        starlark_options: &BTreeMap<String, Value>,
    ) -> IndexMap<Label, ScopeType> {
        let mut result: IndexMap<Label, ScopeType> = IndexMap::new();
        for (key, value) in scopes_attributes {
            if !starlark_options.contains_key(key) {
                continue;
            }
            let scope_type = ScopeType::value_of_ignore_case(value).unwrap_or_else(|| {
                panic!(
                    "IllegalArgumentException: No enum constant com.google.devtools.build.lib.analysis.config.Scope.ScopeType.{}",
                    value.to_uppercase()
                )
            });
            // toImmutableMap throws on a duplicate key.
            if result
                .insert(Label::parse_canonical_unchecked(key), scope_type)
                .is_some()
            {
                panic!("IllegalArgumentException: Multiple entries with same key: {key}");
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:110 BuildOptions.of
    /// Creates a BuildOptions class by taking the option values from command-line arguments.
    /// Returns a BuildOptions class that only has native options.
    pub fn of_args(
        options_list: &[FragmentOptionsClass],
        args: &[&str],
    ) -> Result<BuildOptions, OptionsParsingException> {
        let mut builder = BuildOptions::builder();
        let classes: Vec<_> = options_list.iter().map(|c| c.options_class()).collect();
        let mut parser = OptionsParser::builder()
            .options_classes(&classes)
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        parser.parse(args)?;
        for options_class in options_list {
            let options = parser
                .get_options_dyn(options_class.options_class())
                .expect("parsed options");
            builder.add_fragment_options_dyn(options_class, options.as_ref());
        }
        Ok(builder.build())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:110 BuildOptions.of
    /// Returns a BuildOptions class that only has Starlark options.
    pub fn of_starlark(starlark_options: IndexMap<Label, StarlarkOptionValue>) -> BuildOptions {
        BuildOptions::builder()
            .add_starlark_options(starlark_options)
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:153 BuildOptions.get
    /// Returns the actual instance of a `FragmentOptions` class, or `None` if the options class
    /// is not present.
    pub fn get<T: OptionsClass>(&self) -> Option<&T> {
        self.get_dyn(fragment_options_class::<T>())
            .map(|o| o.as_any().downcast_ref::<T>().expect("class matches"))
    }

    /// Mutable access to a fragment (Java mutates the object `get` returns). Like Java, an
    /// already computed checksum is not recomputed: clone first.
    pub fn get_mut<T: OptionsClass>(&mut self) -> Option<&mut T> {
        self.fragment_options_map
            .get_mut(fragment_options_class::<T>())
            .map(|o| o.as_any_mut().downcast_mut::<T>().expect("class matches"))
    }

    /// `get(Class)` with a class handle.
    pub fn get_dyn(&self, options_class: FragmentOptionsClass) -> Option<&dyn OptionsBase> {
        self.fragment_options_map
            .get(options_class)
            .map(|o| o.as_ref())
    }

    /// Mutable `get(Class)` with a class handle.
    pub fn get_dyn_mut(
        &mut self,
        options_class: FragmentOptionsClass,
    ) -> Option<&mut (dyn OptionsBase + 'static)> {
        self.fragment_options_map
            .get_mut(options_class)
            .map(|o| o.as_mut())
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:159 BuildOptions.contains
    /// Returns true if these options contain the given `FragmentOptions`.
    pub fn contains(&self, options_class: FragmentOptionsClass) -> bool {
        self.fragment_options_map.contains_key(options_class)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:168 BuildOptions.hasNoConfig
    /// Are these options "empty", meaning they contain no meaningful configuration information?
    pub fn has_no_config(&self) -> bool {
        // Ideally the implementation is fragmentOptionsMap.isEmpty() && starlarkOptionsMap.isEmpty().
        // See NoConfigTransition for why CoreOptions stays included.
        self.fragment_options_map.len() == 1
            && self
                .fragment_options_map
                .keys()
                .next()
                .unwrap()
                .get_simple_name()
                == "CoreOptions"
            && self.starlark_options_map.is_empty()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:180 BuildOptions.checksum
    /// Returns a hex digest string uniquely identifying the build options.
    pub fn checksum(&self) -> &str {
        self.checksum.get_or_init(|| {
            if self.fragment_options_map.is_empty() && self.starlark_options_map.is_empty() {
                "0".repeat(64) // Make empty build options easy to distinguish.
            } else {
                let mut fingerprint = Fingerprint::new();
                for options in self.fragment_options_map.values() {
                    fingerprint.add_string(&self.cache_key_of(options.as_ref()));
                }
                fingerprint.add_string(&starlark_map_to_cache_key(&self.starlark_options_map));
                fingerprint.add_string(&map_to_cache_key(&scopes_as_values(&self.scopes)));
                fingerprint.hex_digest_and_reset()
            }
        })
    }

    /// `options.cacheKey()` with the Java class name (Rust options classes only know their Rust
    /// struct name).
    // Java: com/google/devtools/common/options/OptionsBase.java:90 OptionsBase.cacheKey
    fn cache_key_of(&self, options: &dyn OptionsBase) -> String {
        let class = self
            .fragment_options_map
            .keys()
            .find(|c| c.options_class() == options.options_class())
            .expect("registered class");
        format!(
            "{}{{{}}}",
            class.get_name(),
            map_to_cache_key(&options.as_map())
        )
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:206 BuildOptions.shortId
    /// Returns a user-friendly configuration identifier as a prefix of `fullId`.
    pub fn short_id(&self) -> String {
        self.checksum()[..7].to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:225 BuildOptions.getNativeOptions
    /// Returns the options contained in this collection, sorted by `FragmentOptions` name.
    pub fn get_native_options(
        &self,
    ) -> impl Iterator<Item = (FragmentOptionsClass, &dyn OptionsBase)> {
        self.fragment_options_map
            .iter()
            .map(|(k, v)| (*k, v.as_ref()))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:233 BuildOptions.getFragmentClasses
    /// Returns the set of fragment classes contained in these options, sorted by
    /// `FragmentOptions` name.
    pub fn get_fragment_classes(&self) -> Vec<FragmentOptionsClass> {
        self.fragment_options_map.keys().copied().collect()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:238 BuildOptions.getStarlarkOptions
    /// Starlark options, sorted lexicographically by name.
    pub fn get_starlark_options(&self) -> &IndexMap<Label, StarlarkOptionValue> {
        &self.starlark_options_map
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:249 BuildOptions.getScopeTypeMap
    pub fn get_scope_type_map(&self) -> &IndexMap<Label, ScopeType> {
        &self.scopes
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:112 BuildOptions.builder
    /// Creates a builder object for BuildOptions.
    pub fn builder() -> BuildOptionsBuilder {
        BuildOptionsBuilder::default()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:378 BuildOptions.toBuilder
    /// Creates a builder operating on a clone of this BuildOptions.
    pub fn to_builder(&self) -> BuildOptionsBuilder {
        let mut b = BuildOptions::builder();
        b.merge(&self.clone());
        b
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:332 BuildOptions.matches
    /// Returns true if the passed parsing result's options have the same value as these options.
    pub fn matches(&self, parsing_result: &OptionsParser) -> Result<bool, OptionsParsingException> {
        let explicit = parsing_result.as_list_of_explicit_options();
        // Set<OptionDefinition> ignoredDefinitions (an option name identifies its definition).
        let mut ignored_definitions: IndexSet<&'static str> = IndexSet::new();
        for parsed_option in &explicit {
            let option_definition = parsed_option.get_option_definition();
            // All options obtained from an options parser are guaranteed to have been defined in
            // an FragmentOptions class.
            let original_fragment = self
                .fragment_options_map
                .iter()
                .find(|(c, _)| {
                    c.options_class().type_id() == option_definition.declaring_class_id()
                })
                .map(|(_, v)| v);
            let Some(original_fragment) = original_fragment else {
                // Ignore flags set in trimmed fragments.
                ignored_definitions.insert(option_definition.get_option_name());
                continue;
            };
            let original_value = original_fragment
                .as_map()
                .get(option_definition.get_option_name())
                .cloned()
                .flatten();
            if original_value != parsed_option.get_converted_value()? {
                return Ok(false);
            }
        }
        let starlark_options =
            BuildOptions::labelize_starlark_options(parsing_result.get_starlark_options());
        // Maps.difference(starlarkOptionsMap, starlarkOptions).entriesInCommon(): the keys of
        // both maps whose values are equal (`Objects.equals(leftValue, rightValue)`).
        let entries_in_common = self
            .starlark_options_map
            .iter()
            .filter(|(key, left_value)| {
                starlark_options
                    .get(*key)
                    .is_some_and(|right_value| left_value.java_equals(right_value))
            })
            .count();
        if entries_in_common < starlark_options.len() {
            return Ok(false);
        }

        if ignored_definitions.len() == explicit.len() && starlark_options.is_empty() {
            // Zero options were compared, either because none were passed or because all of them
            // were trimmed.
            return Ok(false);
        }
        Ok(true)
    }
}

/// `OptionsBase.mapToCacheKey(starlarkOptionsMap)`: keys by their `toString()`, values by their
/// Java class's `toString()`.
// Java: com/google/devtools/common/options/OptionsBase.java:96 OptionsBase.mapToCacheKey
fn starlark_map_to_cache_key(map: &IndexMap<Label, StarlarkOptionValue>) -> String {
    let mut result = String::new();
    for (key, value) in map {
        result.push_str(&key.to_string());
        result.push('=');
        // This special case is needed because List.toString() prints the same ("[]") for an
        // empty list and for a list with a single empty string.
        if value.is_empty_java_list() {
            result.push_str("EMPTY");
        } else {
            result.push('"');
            result.push_str(
                &value
                    .java_to_string()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\""),
            );
            result.push('"');
        }
        result.push_str(", ");
    }
    result
}

fn scopes_as_values(map: &IndexMap<Label, ScopeType>) -> IndexMap<String, Option<Value>> {
    map.iter()
        .map(|(k, v)| (k.to_string(), Some(Value::new(*v))))
        .collect()
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:258 BuildOptions.clone
/// Creates a copy of the BuildOptions object that contains copies of the FragmentOptions and
/// Starlark options. The copy has a fresh (not yet computed) checksum.
impl Clone for BuildOptions {
    fn clone(&self) -> Self {
        BuildOptions {
            fragment_options_map: self
                .fragment_options_map
                .iter()
                // Explicitly clone native options because FragmentOptions is mutable.
                .map(|(k, v)| (*k, v.clone_box()))
                .collect(),
            // Note that this assumes that starlark option values are immutable.
            starlark_options_map: self.starlark_options_map.clone(),
            scopes: self.scopes.clone(),
            checksum: OnceLock::new(),
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:273 BuildOptions.equals
impl PartialEq for BuildOptions {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other) || self.checksum() == other.checksum()
    }
}
impl Eq for BuildOptions {}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:284 BuildOptions.hashCode
impl std::hash::Hash for BuildOptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.checksum().hash(state)
    }
}

impl fmt::Debug for BuildOptions {
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:215 BuildOptions.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fragments: Vec<String> = self
            .fragment_options_map
            .values()
            .map(|o| o.java_to_string())
            .collect();
        let starlark: Vec<String> = self
            .starlark_options_map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v.java_to_string()))
            .collect();
        let scopes: Vec<String> = self
            .scopes
            .iter()
            .map(|(k, v)| format!("{}={}", k, v.name()))
            .collect();
        write!(
            f,
            "BuildOptions{{checksum={}, fragmentOptions=[{}], starlarkOptions={{{}}}, scopes={{{}}}}}",
            self.checksum(),
            fragments.join(", "),
            starlark.join(", "),
            scopes.join(", ")
        )
    }
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:383 BuildOptions.Builder
/// Builder class for BuildOptions.
#[derive(Default)]
pub struct BuildOptionsBuilder {
    fragment_options: Vec<(FragmentOptionsClass, Box<dyn OptionsBase>)>,
    starlark_options: IndexMap<Label, StarlarkOptionValue>,
    scopes: IndexMap<Label, ScopeType>,
}

impl BuildOptionsBuilder {
    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:389 BuildOptions.Builder.merge
    /// Merges the given BuildOptions into this builder, overriding any previous instances of
    /// Starlark options or FragmentOptions subclasses found in the new BuildOptions.
    pub fn merge(&mut self, options: &BuildOptions) -> &mut Self {
        for (class, fragment) in options.get_native_options() {
            self.add_fragment_options_dyn(class, fragment);
        }
        self.add_starlark_options(options.get_starlark_options().clone());
        self.add_scope_type_map(options.get_scope_type_map().clone());
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:406 BuildOptions.Builder.addFragmentOptions
    /// Adds a new `FragmentOptions` instance to the builder. Overrides previous instances of the
    /// exact same subclass of `FragmentOptions`. The options get preprocessed with
    /// `FragmentOptions.getNormalized`.
    pub fn add_fragment_options<T: OptionsClass>(&mut self, options: &T) -> &mut Self {
        self.add_fragment_options_dyn(fragment_options_class::<T>(), options)
    }

    /// `addFragmentOptions` with a class handle.
    pub fn add_fragment_options_dyn(
        &mut self,
        class: FragmentOptionsClass,
        options: &dyn OptionsBase,
    ) -> &mut Self {
        let normalized = class.get_normalized(options);
        match self.fragment_options.iter_mut().find(|(c, _)| *c == class) {
            Some(entry) => entry.1 = normalized,
            None => self.fragment_options.push((class, normalized)),
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:417 BuildOptions.Builder.getFragmentOptions
    pub fn get_fragment_options<T: OptionsClass>(&mut self) -> Option<&mut T> {
        let class = fragment_options_class::<T>();
        self.fragment_options
            .iter_mut()
            .find(|(c, _)| *c == class)
            .map(|(_, o)| o.as_any_mut().downcast_mut::<T>().expect("class matches"))
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:423 BuildOptions.Builder.removeFragmentOptions
    /// Removes the value for the `FragmentOptions` with the given FragmentOptions class.
    pub fn remove_fragment_options(&mut self, key: FragmentOptionsClass) -> &mut Self {
        self.fragment_options.retain(|(c, _)| *c != key);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:432 BuildOptions.Builder.addStarlarkOptions
    /// Adds multiple Starlark options to the builder. Overrides previous instances of the same
    /// key.
    pub fn add_starlark_options(
        &mut self,
        options: IndexMap<Label, StarlarkOptionValue>,
    ) -> &mut Self {
        for (k, v) in options {
            self.starlark_options.insert(k, v);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:439 BuildOptions.Builder.addStarlarkOption
    pub fn add_starlark_option(&mut self, key: Label, value: StarlarkOptionValue) -> &mut Self {
        self.starlark_options.insert(key, value);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:449 BuildOptions.Builder.addScopeType
    pub fn add_scope_type(&mut self, key: Label, value: ScopeType) -> &mut Self {
        self.scopes.insert(key, value);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:455 BuildOptions.Builder.addScopeTypeMap
    pub fn add_scope_type_map(&mut self, scopes: IndexMap<Label, ScopeType>) -> &mut Self {
        for (k, v) in scopes {
            self.scopes.insert(k, v);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:463 BuildOptions.Builder.removeScope
    pub fn remove_scope(&mut self, key: &Label) -> &mut Self {
        self.scopes.shift_remove(key);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:470 BuildOptions.Builder.removeStarlarkOption
    /// Removes the value and associated ScopeType for the Starlark option with the given key.
    pub fn remove_starlark_option(&mut self, key: &Label) -> &mut Self {
        self.starlark_options.shift_remove(key);
        self.remove_scope(key);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:476 BuildOptions.Builder.build
    pub fn build(&mut self) -> BuildOptions {
        let mut fragments = std::mem::take(&mut self.fragment_options);
        // LEXICAL_FRAGMENT_OPTIONS_COMPARATOR: Comparator.comparing(Class::getName).
        fragments.sort_by(|a, b| java_compare(a.0.get_name(), b.0.get_name()));
        let mut starlark: Vec<(Label, StarlarkOptionValue)> =
            std::mem::take(&mut self.starlark_options)
                .into_iter()
                .collect();
        starlark.sort_by(|a, b| a.0.cmp(&b.0));
        let mut scopes: Vec<(Label, ScopeType)> =
            std::mem::take(&mut self.scopes).into_iter().collect();
        scopes.sort_by(|a, b| a.0.cmp(&b.0));
        BuildOptions {
            fragment_options_map: fragments.into_iter().collect(),
            starlark_options_map: starlark.into_iter().collect(),
            scopes: scopes.into_iter().collect(),
            checksum: OnceLock::new(),
        }
    }
}

/// Java `String.compareTo`.
pub(crate) fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

// Java: com/google/devtools/build/lib/analysis/config/BuildOptionsView.java:28 BuildOptionsView
/// Wrapper for `BuildOptions` that only permits `BuildOptions.get` calls to `FragmentOptions`
/// from a pre-declared set.
pub struct BuildOptionsView<'a> {
    options: &'a BuildOptions,
    allowed_fragments: Vec<FragmentOptionsClass>,
}

impl<'a> BuildOptionsView<'a> {
    pub fn new(options: &'a BuildOptions, allowed_fragments: Vec<FragmentOptionsClass>) -> Self {
        BuildOptionsView {
            options,
            allowed_fragments,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionsView.java:44 BuildOptionsView.get
    pub fn get<T: OptionsClass>(&self) -> Option<&T> {
        self.check_fragment(fragment_options_class::<T>());
        self.options.get::<T>()
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionsView.java:52 BuildOptionsView.contains
    pub fn contains(&self, options_class: FragmentOptionsClass) -> bool {
        self.check_fragment(options_class);
        self.options.contains(options_class)
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionsView.java:73 BuildOptionsView.underlying
    pub fn underlying(&self) -> &'a BuildOptions {
        self.options
    }

    pub fn allowed_fragments(&self) -> &[FragmentOptionsClass] {
        &self.allowed_fragments
    }

    // Java: com/google/devtools/build/lib/analysis/config/BuildOptionsView.java:77 BuildOptionsView.checkFragment
    fn check_fragment(&self, options_class: FragmentOptionsClass) {
        blaze_util::check_argument!(
            self.allowed_fragments.contains(&options_class),
            "Can't access {:?} in allowed fragments {:?}",
            options_class,
            self.allowed_fragments
        );
    }
}

/// Starlark options parsed from text, keyed by label string (Java
/// `OptionsProvider.getStarlarkOptions`).
pub type StarlarkOptionsByName = BTreeMap<String, StarlarkOptionValue>;
