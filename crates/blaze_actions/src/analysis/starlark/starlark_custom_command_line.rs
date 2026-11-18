//! Port of `lib/analysis/starlark/StarlarkCustomCommandLine.java`: the command line built from
//! `ctx.actions.args()`.
//!
//! To be as memory-friendly as possible, expansion happens in three stages. First, when a
//! Starlark rule is analyzed, its `Args` are built into a `StarlarkCustomCommandLine`. At this
//! point, the representation (`arguments`) is just a "recipe" to compute the full command line
//! later on. Additionally, `add_to_fingerprint` supports computing a fingerprint without actually
//! constructing the expanded command line.
//!
//! Second, right before an action executes, `expand` is called to "preprocess" the recipe into a
//! `PreprocessedCommandLine`. This step includes flattening nested sets and applying any
//! operations that can throw an exception, such as expanding directories and invoking `map_each`
//! functions. String formatting (`format`, `format_each`, `before_each`, `join_with`,
//! `format_joined`, `flag_per_line`) is not yet applied; if `map_each` is not used, path mapping
//! is also not applied yet.
//!
//! Finally, string formatting and path mapping are applied during iteration over a
//! `PreprocessedCommandLine`.
//!
//! Representation. Java keeps an `Object[]` of heterogeneous objects: `VectorArg` headers,
//! `StarlarkCallable`s, `Location`s, `NestedSet`s, `Integer` counts, values, format strings, the
//! `SINGLE_FORMATTED_ARG_MARKER` and a trailing `RepositoryMapping`. Here that array is a
//! `Vec<Entry>` with one variant per kind of object, in the same layout. Command line values
//! (Java `Object`: `String`, `Artifact`, `Label`, `ArtifactRoot`, other Starlark values) are
//! [`ArgValue`]s. Starlark values that must outlive the analysis (the `map_each` callables and
//! values other than strings, files, labels and roots) are kept as `OwnedFrozenValue`s, which keep
//! their frozen heap alive; `map_each` is called later in a fresh evaluation through
//! `blaze_starlark::eval::with_evaluator`.

use std::cell::Cell;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use allocative::Allocative;
use blaze_cmdline::{Label, RepositoryMapping};
use blaze_collect::NestedSet;
use blaze_collect::command_line_item::{
    CommandLineExpansionException, CommandLineItem, InterruptedException, MapFn, MapFnError,
    ParametrizedMapFn,
};
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{EvalOptions, frozen_into, with_evaluator};
use blaze_starlark::label::LabelValue;
use blaze_starlark::thread::{Phase, StarlarkEvents, ThreadContext};
use blaze_starlark::{EvalError, Location, StarlarkSemantics};
use blaze_util::{Fingerprint, check_state, unported};
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic, Module};
use starlark::starlark_module;
use starlark::values::list::AllocList;
use starlark::values::{
    Heap, NoSerialize, OwnedFrozenValue, StarlarkValue, Value, ValueLike, starlark_value,
};

use crate::actions::action::{CommandLineError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::actions::artifact_root::ArtifactRoot;
use crate::actions::command_line::{self, ArgChunk, CommandLine, CommandLineRef};
use crate::actions::path_mapper::{FileRoot, NOOP, OutputPathsMode, PathMapper, for_action_key};
use crate::actions::single_string_arg_formatter;

use super::starlark_file::{StarlarkFile, StarlarkFileRoot};

// ---------------------------------------------------------------------------------------------
// Values

/// A value on a Starlark command line (Java: the `Object`s stored in the arguments array, as
/// produced by `Args.add`/`add_all`/`add_joined`).
#[derive(Clone, Debug)]
pub enum ArgValue {
    String(String),
    /// A `File`.
    Artifact(Artifact),
    Label(Label),
    /// A `root` (`ArtifactRoot` or `MappedArtifactRoot`).
    FileRoot(FileRoot),
    /// Any other Starlark value, kept frozen.
    Starlark(StarlarkObject),
}

/// A frozen Starlark value used as a command line element.
#[derive(Clone, Debug)]
pub struct StarlarkObject {
    value: OwnedFrozenValue,
    /// The Starlark hash of the value, or 0 if it is unhashable (only depset elements, which are
    /// always hashable, are ever hashed).
    hash: u32,
}

impl StarlarkObject {
    pub fn new(value: OwnedFrozenValue) -> StarlarkObject {
        let hash = value
            .value()
            .get_hashed()
            .map(|h| h.hash().get())
            .unwrap_or(0);
        StarlarkObject { value, hash }
    }

    pub fn value(&self) -> &OwnedFrozenValue {
        &self.value
    }
}

fn values_equal<'v>(a: Value<'v>, b: Value<'v>) -> bool {
    a.equals(b).unwrap_or(false)
}

impl PartialEq for StarlarkObject {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self.value.value(), other.value.value())
    }
}

impl Eq for StarlarkObject {}

impl Hash for StarlarkObject {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl PartialEq for ArgValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ArgValue::String(a), ArgValue::String(b)) => a == b,
            (ArgValue::Artifact(a), ArgValue::Artifact(b)) => a == b,
            (ArgValue::Label(a), ArgValue::Label(b)) => a == b,
            (ArgValue::FileRoot(a), ArgValue::FileRoot(b)) => a == b,
            (ArgValue::Starlark(a), ArgValue::Starlark(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ArgValue {}

impl Hash for ArgValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            ArgValue::String(s) => s.hash(state),
            ArgValue::Artifact(a) => a.hash(state),
            ArgValue::Label(l) => l.hash(state),
            ArgValue::FileRoot(r) => r.hash(state),
            ArgValue::Starlark(o) => o.hash(state),
        }
    }
}

impl From<&str> for ArgValue {
    fn from(s: &str) -> ArgValue {
        ArgValue::String(s.to_owned())
    }
}

impl From<String> for ArgValue {
    fn from(s: String) -> ArgValue {
        ArgValue::String(s)
    }
}

impl From<Artifact> for ArgValue {
    fn from(a: Artifact) -> ArgValue {
        ArgValue::Artifact(a)
    }
}

impl From<&Artifact> for ArgValue {
    fn from(a: &Artifact) -> ArgValue {
        ArgValue::Artifact(a.clone())
    }
}

impl From<Label> for ArgValue {
    fn from(l: Label) -> ArgValue {
        ArgValue::Label(l)
    }
}

impl From<ArtifactRoot> for ArgValue {
    fn from(r: ArtifactRoot) -> ArgValue {
        ArgValue::FileRoot(FileRoot::Artifact(r))
    }
}

impl From<&ArtifactRoot> for ArgValue {
    fn from(r: &ArtifactRoot) -> ArgValue {
        ArgValue::FileRoot(FileRoot::Artifact(r.clone()))
    }
}

impl From<FileRoot> for ArgValue {
    fn from(r: FileRoot) -> ArgValue {
        ArgValue::FileRoot(r)
    }
}

impl ArgValue {
    /// Converts a Starlark value of a frozen `Args` (or any value reachable from `owner`'s
    /// heap) into a command line value. `v` must be frozen unless it is a string, `File`, `Label`
    /// or `root`.
    pub fn from_frozen_value(owner: &OwnedFrozenValue, v: Value) -> ArgValue {
        if let Some(s) = v.unpack_str() {
            return ArgValue::String(s.to_owned());
        }
        if let Some(f) = v.downcast_ref::<StarlarkFile>() {
            return ArgValue::Artifact(f.0.clone());
        }
        if let Some(l) = v.downcast_ref::<LabelValue>() {
            return ArgValue::Label(l.0.clone());
        }
        if let Some(r) = v.downcast_ref::<StarlarkFileRoot>() {
            return ArgValue::FileRoot(r.0.clone());
        }
        let fv = v
            .unpack_frozen()
            .expect("command line values must be frozen before the command line is built");
        ArgValue::Starlark(StarlarkObject::new(owner.map(|_| fv)))
    }

    /// The value as a Starlark value on an evaluation's heap (for `map_each`).
    fn to_starlark<'v>(&self, module: &Module<'v>, heap: Heap<'v>) -> Value<'v> {
        match self {
            ArgValue::String(s) => heap.alloc_str(s).to_value(),
            ArgValue::Artifact(a) => heap.alloc(StarlarkFile(a.clone())),
            ArgValue::Label(l) => heap.alloc(LabelValue(l.clone())),
            ArgValue::FileRoot(r) => heap.alloc(StarlarkFileRoot(r.clone())),
            ArgValue::Starlark(o) => frozen_into(module, &o.value),
        }
    }

    /// Java `object instanceof FileApi`.
    fn is_file(&self) -> bool {
        matches!(self, ArgValue::Artifact(_))
    }
}

/// Java `CommandLineItem.expandToCommandLine(Object)`: the item's own expansion for
/// `CommandLineItem`s, `toString()` otherwise.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:75 CommandLineItem.expandToCommandLine
impl CommandLineItem for ArgValue {
    fn expand_to_command_line(&self) -> String {
        match self {
            ArgValue::String(s) => s.clone(),
            // Java: com/google/devtools/build/lib/actions/Artifact.java:431 Artifact.expandToCommandLine
            ArgValue::Artifact(a) => a.expand_to_command_line(),
            // Java: com/google/devtools/build/lib/cmdline/Label.java:757 Label.expandToCommandLine
            ArgValue::Label(l) => l.get_canonical_form(),
            // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:290 ArtifactRoot.expandToCommandLine
            ArgValue::FileRoot(FileRoot::Artifact(r)) => r.expand_to_command_line(),
            // MappedArtifactRoot is not a CommandLineItem: toString().
            // Java: com/google/devtools/build/lib/actions/PathMapper.java:264 MappedArtifactRoot.toString
            ArgValue::FileRoot(FileRoot::Mapped(m)) => m.to_string(),
            ArgValue::Starlark(o) => java_to_string(o.value.value()),
        }
    }
}

/// Java `Object.toString()` of a Starlark value.
///
/// PORT: Java's `toString` is `Starlark.repr` for lists, tuples, dicts, structs and providers,
/// the decimal form for ints, `"None"` for None, and Java's `"true"`/`"false"` for bools. Strings
/// are themselves, `File`s are `Artifact.toString` (`File:[...]...`), labels their canonical form.
/// Floats use Java's `%g`-style `StarlarkFloat.toString`, which is not ported.
pub(crate) fn java_to_string(v: Value) -> String {
    if let Some(s) = v.unpack_str() {
        return s.to_owned();
    }
    if let Some(b) = v.unpack_bool() {
        return if b { "true" } else { "false" }.to_owned();
    }
    if let Some(f) = v.downcast_ref::<StarlarkFile>() {
        return f.0.to_string();
    }
    if let Some(l) = v.downcast_ref::<LabelValue>() {
        return l.0.to_string();
    }
    if v.get_type() == "float" {
        unported!("StarlarkFloat.toString");
    }
    v.to_repr()
}

// ---------------------------------------------------------------------------------------------
// Constants

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:133 LINE_JOINER
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:134 FIELD_JOINER

/// Used to distinguish command line arguments that are potentially subject to special default
/// stringification (such as Artifacts when path mapped or Labels when not main repo labels) from
/// strings that happen to be identical to their string representations.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:139 StringificationType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StringificationType {
    DEFAULT,
    FILE,
    LABEL,
}

impl StringificationType {
    fn ordinal(self) -> i32 {
        match self {
            StringificationType::DEFAULT => 0,
            StringificationType::FILE => 1,
            StringificationType::LABEL => 2,
        }
    }
}

/// Java `UUID.fromString(s)`, as (least significant bits, most significant bits).
fn parse_uuid(s: &str) -> (i64, i64) {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    let most = u64::from_str_radix(&hex[..16], 16).expect("uuid") as i64;
    let least = u64::from_str_radix(&hex[16..], 16).expect("uuid") as i64;
    (least, most)
}

/// Java `Fingerprint.addUUID(UUID.fromString(s))`.
fn add_uuid(fingerprint: &mut Fingerprint, s: &str) {
    let (least, most) = parse_uuid(s);
    fingerprint.add_uuid(least, most);
}

const HAS_MAP_EACH: i32 = 1;
const IS_NESTED_SET: i32 = 1 << 1;
const EXPAND_DIRECTORIES: i32 = 1 << 2;
const UNIQUIFY: i32 = 1 << 3;
const OMIT_IF_EMPTY: i32 = 1 << 4;
const HAS_ARG_NAME: i32 = 1 << 5;
const HAS_FORMAT_EACH: i32 = 1 << 6;
const HAS_BEFORE_EACH: i32 = 1 << 7;
const HAS_JOIN_WITH: i32 = 1 << 8;
const HAS_FORMAT_JOINED: i32 = 1 << 9;
const HAS_TERMINATE_WITH: i32 = 1 << 10;
const HAS_SINGLE_ARG: i32 = 1 << 11;

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:171 VectorArg UUIDs
const EXPAND_DIRECTORIES_UUID: &str = "9d7520d2-a187-11e8-98d0-529269fb1459";
const UNIQUIFY_UUID: &str = "7f494c3e-faea-4498-a521-5d3bc6ee19eb";
const OMIT_IF_EMPTY_UUID: &str = "923206f1-6474-4a8f-b30f-4dd3143622e6";
const ARG_NAME_UUID: &str = "2bc00382-7199-46ec-ad52-1556577cde1a";
const FORMAT_EACH_UUID: &str = "8e974aec-df07-4a51-9418-f4c1172b4045";
const BEFORE_EACH_UUID: &str = "f7e101bc-644d-4277-8562-6515ad55a988";
const JOIN_WITH_UUID: &str = "c227dbd3-edad-454e-bc8a-c9b5ba1c38a3";
const FORMAT_JOINED_UUID: &str = "528af376-4233-4c27-be4d-b0ff24ed68db";
const TERMINATE_WITH_UUID: &str = "a4e5e090-0dbd-4d41-899a-77cfbba58655";

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:791 SingleFormattedArg.SINGLE_FORMATTED_ARG_UUID
const SINGLE_FORMATTED_ARG_UUID: &str = "8cb96642-a235-4fe0-b3ed-ebfdae8a0bd9";

// ---------------------------------------------------------------------------------------------
// The arguments array

/// One element of Java's `Object[] arguments`.
#[derive(Clone)]
enum Entry {
    VectorArg(VectorArg),
    /// A `map_each` callable.
    MapEach(OwnedFrozenValue),
    Location(Location),
    NestedSet(NestedSet<ArgValue>),
    /// An element count (Java `Integer`).
    Count(i32),
    /// A value, an arg name, or a format/join string.
    Object(ArgValue),
    /// Java `SINGLE_FORMATTED_ARG_MARKER`.
    SingleFormattedArgMarker,
    RepositoryMapping(RepositoryMapping),
}

impl Entry {
    /// Java `(String) arguments.get(i)`.
    fn as_string(&self) -> &str {
        match self {
            Entry::Object(ArgValue::String(s)) => s,
            _ => panic!("ClassCastException: expected a String in the arguments array"),
        }
    }

    fn as_object(&self) -> &ArgValue {
        match self {
            Entry::Object(v) => v,
            _ => panic!("ClassCastException: expected a value in the arguments array"),
        }
    }

    fn as_location(&self) -> &Location {
        match self {
            Entry::Location(l) => l,
            _ => panic!("ClassCastException: expected a Location in the arguments array"),
        }
    }

    fn as_map_each(&self) -> &OwnedFrozenValue {
        match self {
            Entry::MapEach(f) => f,
            _ => panic!("ClassCastException: expected a StarlarkCallable in the arguments array"),
        }
    }

    fn as_count(&self) -> i32 {
        match self {
            Entry::Count(c) => *c,
            _ => panic!("ClassCastException: expected an Integer in the arguments array"),
        }
    }

    fn as_nested_set(&self) -> &NestedSet<ArgValue> {
        match self {
            Entry::NestedSet(s) => s,
            _ => panic!("ClassCastException: expected a NestedSet in the arguments array"),
        }
    }
}

/// Representation of a sequence of arguments originating from `Args.add_all` or
/// `Args.add_joined`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:150 VectorArg
//
// PORT: Java interns VectorArg instances and stores a global `map_each` function inside the
// interned VectorArg (`mapEachGlobalFunction`) to save one array slot. Both are memory
// optimizations with no observable effect (the fingerprint does not depend on them); here the
// `map_each` callable is always stored in the arguments array after the VectorArg.
#[derive(Clone, Debug)]
struct VectorArg {
    features: i32,
    stringification_type: StringificationType,
    // None unless map_each is present.
    starlark_semantics: Option<Arc<StarlarkSemantics>>,
}

/// Java `StarlarkCustomCommandLine.VectorArg.Builder`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:650 VectorArg.Builder
#[derive(Clone)]
pub struct VectorArgBuilder {
    list: Option<Vec<ArgValue>>,
    nested_set: Option<NestedSet<ArgValue>>,
    nested_set_stringification_type: StringificationType,
    location: Option<Location>,
    arg_name: Option<String>,
    expand_directories: bool,
    map_each: Option<OwnedFrozenValue>,
    format_each: Option<String>,
    before_each: Option<String>,
    join_with: Option<String>,
    format_joined: Option<String>,
    omit_if_empty: bool,
    uniquify: bool,
    terminate_with: Option<String>,
}

impl VectorArgBuilder {
    fn new(
        list: Option<Vec<ArgValue>>,
        nested_set: Option<NestedSet<ArgValue>>,
        nested_set_stringification_type: StringificationType,
    ) -> VectorArgBuilder {
        VectorArgBuilder {
            list,
            nested_set,
            nested_set_stringification_type,
            location: None,
            arg_name: None,
            expand_directories: false,
            map_each: None,
            format_each: None,
            before_each: None,
            join_with: None,
            format_joined: None,
            omit_if_empty: false,
            uniquify: false,
            terminate_with: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:666 VectorArg.Builder.<init>(Sequence)
    pub fn from_list(list: Vec<ArgValue>) -> VectorArgBuilder {
        VectorArgBuilder::new(Some(list), None, StringificationType::DEFAULT)
    }

    /// `element_type` is the depset's element type name (Java `Depset.getElementClass()`):
    /// `"File"` (FileApi) and `"Label"` select the FILE and LABEL stringification types.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:672 VectorArg.Builder.<init>(NestedSet, Class)
    pub fn from_nested_set(
        nested_set: NestedSet<ArgValue>,
        element_type: &str,
    ) -> VectorArgBuilder {
        let t = match element_type {
            "File" => StringificationType::FILE,
            "Label" => StringificationType::LABEL,
            _ => StringificationType::DEFAULT,
        };
        VectorArgBuilder::new(None, Some(nested_set), t)
    }

    pub fn set_location(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    pub fn set_arg_name(mut self, arg_name: impl Into<String>) -> Self {
        self.arg_name = Some(arg_name.into());
        self
    }

    pub fn set_expand_directories(mut self, expand_directories: bool) -> Self {
        self.expand_directories = expand_directories;
        self
    }

    pub fn set_map_each(mut self, map_each: OwnedFrozenValue) -> Self {
        self.map_each = Some(map_each);
        self
    }

    pub fn set_format_each(mut self, format: impl Into<String>) -> Self {
        self.format_each = Some(format.into());
        self
    }

    pub fn set_before_each(mut self, before_each: impl Into<String>) -> Self {
        self.before_each = Some(before_each.into());
        self
    }

    pub fn set_join_with(mut self, join_with: impl Into<String>) -> Self {
        self.join_with = Some(join_with.into());
        self
    }

    pub fn set_format_joined(mut self, format_joined: impl Into<String>) -> Self {
        self.format_joined = Some(format_joined.into());
        self
    }

    pub fn omit_if_empty(mut self, omit_if_empty: bool) -> Self {
        self.omit_if_empty = omit_if_empty;
        self
    }

    pub fn uniquify(mut self, uniquify: bool) -> Self {
        self.uniquify = uniquify;
        self
    }

    pub fn set_terminate_with(mut self, terminate_with: impl Into<String>) -> Self {
        self.terminate_with = Some(terminate_with.into());
        self
    }

    // Optional setters for Option-valued fields (the Java setters accept null).
    pub(crate) fn set_arg_name_opt(mut self, v: Option<String>) -> Self {
        self.arg_name = v;
        self
    }

    pub(crate) fn set_map_each_opt(mut self, v: Option<OwnedFrozenValue>) -> Self {
        self.map_each = v;
        self
    }

    pub(crate) fn set_format_each_opt(mut self, v: Option<String>) -> Self {
        self.format_each = v;
        self
    }

    pub(crate) fn set_before_each_opt(mut self, v: Option<String>) -> Self {
        self.before_each = v;
        self
    }

    pub(crate) fn set_join_with_opt(mut self, v: Option<String>) -> Self {
        self.join_with = v;
        self
    }

    pub(crate) fn set_format_joined_opt(mut self, v: Option<String>) -> Self {
        self.format_joined = v;
        self
    }

    pub(crate) fn set_terminate_with_opt(mut self, v: Option<String>) -> Self {
        self.terminate_with = v;
        self
    }
}

impl VectorArg {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:224 VectorArg.push
    fn push(
        arguments: &mut Vec<Entry>,
        arg: VectorArgBuilder,
        starlark_semantics: &Arc<StarlarkSemantics>,
    ) {
        // The location is really only needed if map_each is present, but it's easy enough to
        // require it unconditionally.
        let location = arg
            .location
            .clone()
            .expect("NullPointerException: VectorArg location");

        let mut features = 0;
        features |= if arg.map_each.is_some() {
            HAS_MAP_EACH
        } else {
            0
        };
        features |= if arg.nested_set.is_some() {
            IS_NESTED_SET
        } else {
            0
        };
        features |= if arg.expand_directories {
            EXPAND_DIRECTORIES
        } else {
            0
        };
        features |= if arg.uniquify { UNIQUIFY } else { 0 };
        features |= if arg.omit_if_empty { OMIT_IF_EMPTY } else { 0 };
        features |= if arg.arg_name.is_some() {
            HAS_ARG_NAME
        } else {
            0
        };
        features |= if arg.format_each.is_some() {
            HAS_FORMAT_EACH
        } else {
            0
        };
        features |= if arg.before_each.is_some() {
            HAS_BEFORE_EACH
        } else {
            0
        };
        features |= if arg.join_with.is_some() {
            HAS_JOIN_WITH
        } else {
            0
        };
        features |= if arg.format_joined.is_some() {
            HAS_FORMAT_JOINED
        } else {
            0
        };
        features |= if arg.terminate_with.is_some() {
            HAS_TERMINATE_WITH
        } else {
            0
        };
        features |= if arg.nested_set.is_none() && arg.list.as_ref().map_or(0, |l| l.len()) == 1 {
            HAS_SINGLE_ARG
        } else {
            0
        };
        arguments.push(Entry::VectorArg(VectorArg {
            features,
            stringification_type: arg.nested_set_stringification_type,
            starlark_semantics: if arg.map_each.is_some() {
                Some(starlark_semantics.clone())
            } else {
                None
            },
        }));
        if let Some(map_each) = &arg.map_each {
            arguments.push(Entry::MapEach(map_each.clone()));
            arguments.push(Entry::Location(location));
        }
        if let Some(nested_set) = &arg.nested_set {
            arguments.push(Entry::NestedSet(nested_set.clone()));
        } else {
            let list = arg.list.as_ref().expect("list");
            let count = list.len();
            if count != 1 {
                // A count of 1 is encoded via the HAS_SINGLE_ARG feature.
                arguments.push(Entry::Count(count as i32));
            }
            for v in list {
                arguments.push(Entry::Object(v.clone()));
            }
        }
        if let Some(arg_name) = &arg.arg_name {
            arguments.push(Entry::Object(ArgValue::String(arg_name.clone())));
        }
        if let Some(format_each) = &arg.format_each {
            arguments.push(Entry::Object(ArgValue::String(format_each.clone())));
        }
        if let Some(before_each) = &arg.before_each {
            check_state!(
                arg.join_with.is_none(),
                "before_each and join_with are mutually exclusive"
            );
            check_state!(
                arg.format_joined.is_none(),
                "before_each and format_joined are mutually exclusive"
            );
            arguments.push(Entry::Object(ArgValue::String(before_each.clone())));
        }
        if let Some(join_with) = &arg.join_with {
            arguments.push(Entry::Object(ArgValue::String(join_with.clone())));
        }
        if let Some(format_joined) = &arg.format_joined {
            if arg.join_with.is_none() {
                panic!("NullPointerException: format_joined requires join_with");
            }
            arguments.push(Entry::Object(ArgValue::String(format_joined.clone())));
        }
        if let Some(terminate_with) = &arg.terminate_with {
            arguments.push(Entry::Object(ArgValue::String(terminate_with.clone())));
        }
    }

    /// Adds this VectorArg to the given builder. `argi` is the index in `arguments` at which
    /// this VectorArg's data begins (directly after the VectorArg itself); returns the index where
    /// the next arg begins.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:309 VectorArg.preprocess
    fn preprocess(
        &self,
        arguments: &[Entry],
        mut argi: usize,
        builder: &mut PreprocessedCommandLineBuilder,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        main_repo_mapping: Option<&RepositoryMapping>,
    ) -> Result<usize, CommandLineError> {
        let mut map_each: Option<&OwnedFrozenValue> = None;
        let mut location: Option<&Location> = None;
        if (self.features & HAS_MAP_EACH) != 0 {
            map_each = Some(arguments[argi].as_map_each());
            argi += 1;
            location = Some(arguments[argi].as_location());
            argi += 1;
        }

        let original_values: Vec<ArgValue> = if (self.features & IS_NESTED_SET) != 0 {
            let nested_set = arguments[argi].as_nested_set();
            argi += 1;
            nested_set.to_list().as_ref().clone()
        } else {
            let count = if (self.features & HAS_SINGLE_ARG) != 0 {
                1
            } else {
                let c = arguments[argi].as_count() as usize;
                argi += 1;
                c
            };
            let values = arguments[argi..argi + count]
                .iter()
                .map(|e| e.as_object().clone())
                .collect();
            argi += count;
            values
        };
        let expanded_values =
            self.maybe_expand_directories(input_metadata_provider, original_values, path_mapper)?;
        let mut values: Vec<Expanded> = Vec::with_capacity(expanded_values.len());
        if let Some(map_each) = map_each {
            apply_map_each(
                map_each,
                &expanded_values,
                &mut |s| values.push(Expanded::String(s)),
                location.expect("location"),
                input_metadata_provider,
                path_mapper,
                self.starlark_semantics
                    .as_ref()
                    .expect("starlark semantics"),
            )?;
        } else {
            for v in &expanded_values {
                values.push(expand_to_command_line(v, main_repo_mapping));
            }
        }
        // It's safe to uniquify at this stage, any transformations after this
        // will ensure continued uniqueness of the values
        if (self.features & UNIQUIFY) != 0 {
            let mut seen: HashSet<String> = HashSet::with_capacity(values.len());
            let mut unique = Vec::with_capacity(values.len());
            for val in values {
                // If the path mapper is a no-op, an artifact behaves just like its (trivially
                // mapped) exec path string. If the path mapper is not a no-op, mapped paths are
                // always distinct from unmapped paths. We can thus uniquify based on the mapped
                // exec path string in each case.
                if seen.insert(maybe_path_map(&val, path_mapper)) {
                    unique.push(val);
                }
            }
            values = unique;
        }
        let omit_if_empty = (self.features & OMIT_IF_EMPTY) != 0;
        let is_empty_and_should_omit = omit_if_empty && values.is_empty();
        if (self.features & HAS_ARG_NAME) != 0 {
            let arg_name = arguments[argi].as_string();
            argi += 1;
            if !is_empty_and_should_omit {
                builder.add_string(arg_name.to_owned());
            }
        }

        let mut format_each: Option<String> = None;
        let mut before_each: Option<String> = None;
        let mut join_with: Option<String> = None;
        let mut format_joined: Option<String> = None;
        if (self.features & HAS_FORMAT_EACH) != 0 {
            format_each = Some(arguments[argi].as_string().to_owned());
            argi += 1;
        }
        if (self.features & HAS_BEFORE_EACH) != 0 {
            before_each = Some(arguments[argi].as_string().to_owned());
            argi += 1;
        } else if (self.features & HAS_JOIN_WITH) != 0 {
            join_with = Some(arguments[argi].as_string().to_owned());
            argi += 1;
            if (self.features & HAS_FORMAT_JOINED) != 0 {
                format_joined = Some(arguments[argi].as_string().to_owned());
                argi += 1;
            }
        }

        // If !omitIfEmpty, joining yields a single argument even if values is empty. Note that
        // the argument may still be non-empty if format_joined is used.
        if !values.is_empty() || (!omit_if_empty && join_with.is_some()) {
            let arg = match join_with {
                Some(join_with) => PreprocessedArg::JoinedVector {
                    values,
                    format_each,
                    join_with,
                    format_joined,
                },
                None => PreprocessedArg::UnjoinedVector {
                    values,
                    format_each,
                    before_each,
                },
            };
            builder.add_preprocessed_arg(arg);
        }

        if (self.features & HAS_TERMINATE_WITH) != 0 {
            let terminate_with = arguments[argi].as_string();
            argi += 1;
            if !is_empty_and_should_omit {
                builder.add_string(terminate_with.to_owned());
            }
        }
        Ok(argi)
    }

    /// Expands the directories if `expand_directories` feature is enabled and an
    /// InputMetadataProvider is available.
    ///
    /// Technically, we should always expand the directories if the feature is requested, however
    /// we cannot do that in the absence of the InputMetadataProvider.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:427 VectorArg.maybeExpandDirectories
    fn maybe_expand_directories(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        original_values: Vec<ArgValue>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<ArgValue>, CommandLineExpansionException> {
        let Some(input_metadata_provider) = input_metadata_provider else {
            return Ok(original_values);
        };
        if (self.features & EXPAND_DIRECTORIES) == 0 || !has_directory(&original_values) {
            return Ok(original_values);
        }
        expand_directories(input_metadata_provider, &original_values, path_mapper)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:509 VectorArg.addToFingerprint
    fn add_to_fingerprint(
        &self,
        arguments: &[Entry],
        mut argi: usize,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        output_paths_mode: OutputPathsMode,
        main_repo_mapping: Option<&RepositoryMapping>,
    ) -> Result<usize, CommandLineError> {
        let mut map_each: Option<&OwnedFrozenValue> = None;
        let mut location: Option<&Location> = None;
        if (self.features & HAS_MAP_EACH) != 0 {
            map_each = Some(arguments[argi].as_map_each());
            argi += 1;
            location = Some(arguments[argi].as_location());
            argi += 1;
        }

        // NestedSets and lists never result in the same fingerprint as the
        // ActionKeyContext#addNestedSetToFingerprint call below always adds the order of the
        // NestedSet to the fingerprint.
        //
        // Path mapping may affect the default stringification of Artifact instances at execution
        // time, but the effect of path mapping on an individual command line element is a pure
        // function of:
        // * whether the element is of type Artifact (FileApi in Starlark), which is fingerprinted
        //   via the elementType UUID below;
        // * the path of the artifact, which is fingerprinted via its default string
        //   representation below;
        // * the paths and possibly the digests of all input artifacts as well as the path mapping
        //   mode, which are fingerprinted by SpawnAction.
        // It is thus safe to ignore pathMapper below for anything that relies on the default
        // stringification behavior (which excludes custom mapEach functions).
        if (self.features & IS_NESTED_SET) != 0 {
            let values = arguments[argi].as_nested_set();
            argi += 1;
            if let Some(map_each) = map_each {
                // mapEach functions do not rely on default stringification behavior, so we can
                // omit fingerprinting stringificationType here.
                let provider = if (self.features & EXPAND_DIRECTORIES) != 0
                    || wants_directory_expander(map_each)
                {
                    input_metadata_provider
                } else {
                    None
                };
                let command_line_item_map_fn = CommandLineItemMapEachAdaptor::intern(
                    map_each,
                    location.expect("location"),
                    self.starlark_semantics
                        .as_ref()
                        .expect("starlark semantics"),
                    provider.is_some(),
                    output_paths_mode,
                );
                // The cache holds an entry for a NestedSet for every (map_fn,
                // hasInputMetadataProvider bit, pathMapperCacheKey). The input metadata provider
                // itself is only reachable while this call runs (Java clears it in a finally
                // block, `clearInputMetadataProvider`).
                let _guard = ProviderScope::enter(provider);
                action_key_context.add_nested_set_to_fingerprint_with_map_fn(
                    command_line_item_map_fn,
                    fingerprint,
                    values,
                )?;
            } else {
                fingerprint.add_int(self.stringification_type.ordinal());
                if self.stringification_type == StringificationType::LABEL {
                    let mapping = main_repo_mapping
                        .expect("NullPointerException: main repo mapping for a depset of labels");
                    fingerprint.add_string_map(
                        mapping
                            .entries()
                            .iter()
                            .map(|(k, v)| (k.as_str(), v.get_name())),
                    );
                }
                action_key_context.add_nested_set_to_fingerprint(fingerprint, values)?;
            }
        } else {
            let count = if (self.features & HAS_SINGLE_ARG) != 0 {
                1
            } else {
                let c = arguments[argi].as_count() as usize;
                argi += 1;
                c
            };
            let values: Vec<ArgValue> = arguments[argi..argi + count]
                .iter()
                .map(|e| e.as_object().clone())
                .collect();
            let maybe_expanded_values = self.maybe_expand_directories(
                input_metadata_provider,
                values,
                for_action_key(output_paths_mode),
            )?;
            argi += count;
            if let Some(map_each) = map_each {
                // TODO(b/160181927): If inputMetadataProvider == null (happens in the analysis
                // phase) but expandDirectories is true, we run the map_each function on directory
                // values without actually expanding them. This differs from the real evaluation
                // behavior. This means that we can erroneously produce the same digest for two
                // command lines that differ only in their directory expansion. Fortunately, this
                // is only a problem for shared action conflict checking/aquery result, since at
                // execution time we have an input metadata provider.
                apply_map_each(
                    map_each,
                    &maybe_expanded_values,
                    &mut |s| {
                        fingerprint.add_string(&s);
                    },
                    location.expect("location"),
                    input_metadata_provider,
                    for_action_key(output_paths_mode),
                    self.starlark_semantics
                        .as_ref()
                        .expect("starlark semantics"),
                )?;
            } else {
                for value in &maybe_expanded_values {
                    add_single_object_to_fingerprint(fingerprint, value, main_repo_mapping);
                }
            }
        }
        if (self.features & EXPAND_DIRECTORIES) != 0 {
            add_uuid(fingerprint, EXPAND_DIRECTORIES_UUID);
        }
        if (self.features & UNIQUIFY) != 0 {
            add_uuid(fingerprint, UNIQUIFY_UUID);
        }
        if (self.features & OMIT_IF_EMPTY) != 0 {
            add_uuid(fingerprint, OMIT_IF_EMPTY_UUID);
        }
        if (self.features & HAS_ARG_NAME) != 0 {
            let arg_name = arguments[argi].as_string();
            argi += 1;
            add_uuid(fingerprint, ARG_NAME_UUID);
            fingerprint.add_string(arg_name);
        }
        if (self.features & HAS_FORMAT_EACH) != 0 {
            let format_str = arguments[argi].as_string();
            argi += 1;
            add_uuid(fingerprint, FORMAT_EACH_UUID);
            fingerprint.add_string(format_str);
        }
        if (self.features & HAS_BEFORE_EACH) != 0 {
            let before_each = arguments[argi].as_string();
            argi += 1;
            add_uuid(fingerprint, BEFORE_EACH_UUID);
            fingerprint.add_string(before_each);
        } else if (self.features & HAS_JOIN_WITH) != 0 {
            let join_with = arguments[argi].as_string();
            argi += 1;
            add_uuid(fingerprint, JOIN_WITH_UUID);
            fingerprint.add_string(join_with);
            if (self.features & HAS_FORMAT_JOINED) != 0 {
                let format_joined = arguments[argi].as_string();
                argi += 1;
                add_uuid(fingerprint, FORMAT_JOINED_UUID);
                fingerprint.add_string(format_joined);
            }
        }
        if (self.features & HAS_TERMINATE_WITH) != 0 {
            let terminate_with = arguments[argi].as_string();
            argi += 1;
            add_uuid(fingerprint, TERMINATE_WITH_UUID);
            fingerprint.add_string(terminate_with);
        }
        Ok(argi)
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:441 VectorArg.hasDirectory
fn has_directory(original_values: &[ArgValue]) -> bool {
    original_values.iter().any(is_directory)
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:452 VectorArg.isDirectory
fn is_directory(object: &ArgValue) -> bool {
    matches!(object, ArgValue::Artifact(a) if a.is_directory())
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:456 VectorArg.expandDirectories
fn expand_directories(
    input_metadata_provider: &dyn InputMetadataProvider,
    original_values: &[ArgValue],
    _path_mapper: &dyn PathMapper,
) -> Result<Vec<ArgValue>, CommandLineExpansionException> {
    let mut expanded_values = Vec::with_capacity(original_values.len());
    for object in original_values {
        if is_directory(object) {
            let ArgValue::Artifact(artifact) = object else {
                unreachable!()
            };
            if artifact.is_tree_artifact() {
                let Some(children) = input_metadata_provider.get_tree_children(artifact) else {
                    return Err(CommandLineExpansionException::new(format!(
                        "Failed to expand directory {}. Either add the directory as an input of the \
                         action or set 'expand_directories = False' in the 'add_all' or \
                         'add_joined' call to have the path of the directory added to the command \
                         line instead of its contents.",
                        artifact.repr()
                    )));
                };
                expanded_values.extend(children.into_iter().map(ArgValue::Artifact));
            } else if artifact.is_fileset() {
                // Java: VectorArg.expandFileset needs InputMetadataProvider.getFileset, a
                // FilesetOutputTree and StarlarkCustomCommandLine.FilesetSymlinkFile.
                unported!("StarlarkCustomCommandLine.VectorArg.expandFileset");
            } else {
                panic!("AssertionError: Unknown artifact type.");
            }
        } else {
            expanded_values.push(object.clone());
        }
    }
    Ok(expanded_values)
}

// ---------------------------------------------------------------------------------------------
// Builder

/// Java `StarlarkCustomCommandLine.Builder`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:844 Builder
pub struct Builder {
    starlark_semantics: Arc<StarlarkSemantics>,
    arguments: Vec<Entry>,
    // Indexes in arguments list where individual args begin
    arg_start_indexes: Vec<usize>,
}

impl Builder {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:850 Builder.<init>
    pub fn new(starlark_semantics: Arc<StarlarkSemantics>) -> Builder {
        Builder {
            starlark_semantics,
            arguments: Vec::new(),
            arg_start_indexes: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:855 Builder.recordArgStart
    pub fn record_arg_start(&mut self) -> &mut Self {
        if !self.arguments.is_empty() {
            self.arg_start_indexes.push(self.arguments.len());
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:863 Builder.add(Object)
    pub fn add(&mut self, object: impl Into<ArgValue>) -> &mut Self {
        self.arguments.push(Entry::Object(object.into()));
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:869 Builder.add(VectorArg.Builder)
    pub fn add_vector_arg(&mut self, vector_arg: VectorArgBuilder) -> &mut Self {
        VectorArg::push(&mut self.arguments, vector_arg, &self.starlark_semantics);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:875 Builder.addFormatted
    pub fn add_formatted(
        &mut self,
        object: impl Into<ArgValue>,
        format: impl Into<String>,
    ) -> &mut Self {
        // Java: SingleFormattedArg.push
        self.arguments.push(Entry::SingleFormattedArgMarker);
        self.arguments.push(Entry::Object(object.into()));
        self.arguments
            .push(Entry::Object(ArgValue::String(format.into())));
        self
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:882 Builder.build
    pub fn build(
        &self,
        flag_per_line: bool,
        main_repo_mapping: Option<RepositoryMapping>,
    ) -> CommandLineRef {
        if self.arguments.is_empty() {
            return command_line::empty();
        }
        let mut args = self.arguments.clone();
        if let Some(mapping) = main_repo_mapping {
            args.push(Entry::RepositoryMapping(mapping));
        }
        Arc::new(StarlarkCustomCommandLine {
            arguments: args,
            arg_start_indexes: if flag_per_line {
                Some(self.arg_start_indexes.clone())
            } else {
                None
            },
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The command line

/// Supports `ctx.actions.args()` from Starlark.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:131 StarlarkCustomCommandLine
//
// PORT: Java's subclass `StarlarkCustomCommandLineWithIndexes` (used for `flag_per_line`) is the
// `Some(arg_start_indexes)` case of this struct.
pub struct StarlarkCustomCommandLine {
    /// Java `Object[] arguments`, possibly ending with the main `RepositoryMapping`.
    arguments: Vec<Entry>,
    /// Java `StarlarkCustomCommandLineWithIndexes.argStartIndexes`: an extra level of grouping
    /// on top of the 'arguments' list. Each element is the start of a group of args, with index 0
    /// omitted. The expanded version of these arguments will be concatenated together to support
    /// `flag_per_line` format.
    arg_start_indexes: Option<Vec<usize>>,
}

impl fmt::Debug for StarlarkCustomCommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StarlarkCustomCommandLine")
            .field("arguments", &self.arguments.len())
            .field("arg_start_indexes", &self.arg_start_indexes)
            .finish()
    }
}

impl StarlarkCustomCommandLine {
    /// The main repo mapping added in `Builder.build` if any labels in the command line require
    /// it, and the number of real arguments.
    fn split_mapping(&self) -> (Option<&RepositoryMapping>, usize) {
        match self.arguments.last() {
            Some(Entry::RepositoryMapping(m)) => (Some(m), self.arguments.len() - 1),
            _ => (None, self.arguments.len()),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:920 StarlarkCustomCommandLine.expand
    fn expand_plain(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<PreprocessedCommandLine, CommandLineError> {
        let mut builder = PreprocessedCommandLineBuilder::new();
        let arguments = &self.arguments;
        // Added in Builder.build() if any labels in the command line require this to be
        // formatted with an apparent repository name.
        let (main_repo_mapping, size) = self.split_mapping();
        let mut argi = 0;
        while argi < size {
            let arg = &arguments[argi];
            argi += 1;
            match arg {
                Entry::VectorArg(v) => {
                    argi = v.preprocess(
                        arguments,
                        argi,
                        &mut builder,
                        input_metadata_provider,
                        path_mapper,
                        main_repo_mapping,
                    )?;
                }
                Entry::SingleFormattedArgMarker => {
                    argi = single_formatted_arg_preprocess(
                        arguments,
                        argi,
                        &mut builder,
                        main_repo_mapping,
                    );
                }
                other => {
                    builder.add_arg(expand_to_command_line(other.as_object(), main_repo_mapping));
                }
            }
        }
        Ok(builder.build())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1009 StarlarkCustomCommandLineWithIndexes.expand
    fn expand_with_indexes(
        &self,
        arg_start_indexes: &[usize],
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<PreprocessedCommandLine, CommandLineError> {
        let mut builder = PreprocessedCommandLineBuilder::new();
        let arguments = &self.arguments;
        let mut start_index_iterator = arg_start_indexes.iter();
        let (main_repo_mapping, size) = self.split_mapping();

        let mut argi = 0;
        while argi < size {
            let next_start_index = start_index_iterator.next().copied().unwrap_or(size);
            let mut line = PreprocessedCommandLineBuilder::new();

            while argi < next_start_index {
                let arg = &arguments[argi];
                argi += 1;
                match arg {
                    Entry::VectorArg(v) => {
                        argi = v.preprocess(
                            arguments,
                            argi,
                            &mut line,
                            input_metadata_provider,
                            path_mapper,
                            main_repo_mapping,
                        )?;
                    }
                    Entry::SingleFormattedArgMarker => {
                        argi = single_formatted_arg_preprocess(
                            arguments,
                            argi,
                            &mut line,
                            main_repo_mapping,
                        );
                    }
                    other => {
                        line.add_arg(expand_to_command_line(other.as_object(), main_repo_mapping));
                    }
                }
            }

            builder.add_line_for_flag_per_line(line);
        }

        Ok(builder.build())
    }
}

impl CommandLine for StarlarkCustomCommandLine {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:920 StarlarkCustomCommandLine.expand
    fn expand(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Box<dyn ArgChunk + '_>, CommandLineError> {
        let chunk = match &self.arg_start_indexes {
            None => self.expand_plain(input_metadata_provider, path_mapper)?,
            Some(indexes) => {
                self.expand_with_indexes(indexes, input_metadata_provider, path_mapper)?
            }
        };
        let chunk: Box<dyn ArgChunk + '_> = Box::new(chunk);
        Ok(path_mapper.map_custom_starlark_args(chunk))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:955 StarlarkCustomCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        Ok(self.expand(None, &NOOP)?.arguments(&NOOP))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:961 StarlarkCustomCommandLine.arguments
    fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        Ok(self
            .expand(input_metadata_provider, path_mapper)?
            .arguments(path_mapper))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1052 StarlarkCustomCommandLine.addToFingerprint
    fn add_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        effective_output_paths_mode: OutputPathsMode,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        let arguments = &self.arguments;
        let (main_repo_mapping, size) = self.split_mapping();
        let mut argi = 0;
        while argi < size {
            let arg = &arguments[argi];
            argi += 1;
            match arg {
                Entry::VectorArg(v) => {
                    argi = v.add_to_fingerprint(
                        arguments,
                        argi,
                        action_key_context,
                        fingerprint,
                        input_metadata_provider,
                        effective_output_paths_mode,
                        main_repo_mapping,
                    )?;
                }
                Entry::SingleFormattedArgMarker => {
                    argi = single_formatted_arg_add_to_fingerprint(
                        arguments,
                        argi,
                        fingerprint,
                        main_repo_mapping,
                    );
                }
                other => {
                    add_single_object_to_fingerprint(
                        fingerprint,
                        other.as_object(),
                        main_repo_mapping,
                    );
                }
            }
        }
        Ok(())
    }
}

/// `String | DerivedArtifact`: a value after `expandToCommandLine`, before path mapping.
#[derive(Clone, Debug)]
enum Expanded {
    String(String),
    DerivedArtifact(Artifact),
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:967 StarlarkCustomCommandLine.expandToCommandLine
fn expand_to_command_line(
    object: &ArgValue,
    main_repo_mapping: Option<&RepositoryMapping>,
) -> Expanded {
    // Label arguments are rare, so we don't bother rendering them lazily.
    if let ArgValue::Label(label) = object {
        return Expanded::String(label.get_display_form(main_repo_mapping));
    }

    // DerivedArtifacts are path mapped lazily.
    match object {
        ArgValue::Artifact(a) if a.is_derived_artifact_instance() => {
            Expanded::DerivedArtifact(a.clone())
        }
        other => Expanded::String(other.expand_to_command_line()),
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:980 StarlarkCustomCommandLine.addSingleObjectToFingerprint
fn add_single_object_to_fingerprint(
    fingerprint: &mut Fingerprint,
    object: &ArgValue,
    main_repo_mapping: Option<&RepositoryMapping>,
) {
    if let ArgValue::Label(label) = object {
        fingerprint.add_string(&label.get_display_form(main_repo_mapping));
        return;
    }
    let stringification_type = if object.is_file() {
        StringificationType::FILE
    } else {
        StringificationType::DEFAULT
    };
    fingerprint.add_int(stringification_type.ordinal());
    fingerprint.add_string(&object.expand_to_command_line());
}

/// Adds a single formatted arg (from `Args.add(value, format = ...)`); `argi` points just after
/// the `SINGLE_FORMATTED_ARG_MARKER`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:812 SingleFormattedArg.preprocess
fn single_formatted_arg_preprocess(
    arguments: &[Entry],
    mut argi: usize,
    builder: &mut PreprocessedCommandLineBuilder,
    main_repo_mapping: Option<&RepositoryMapping>,
) -> usize {
    let object = arguments[argi].as_object();
    argi += 1;
    let format_str = arguments[argi].as_string().to_owned();
    argi += 1;
    match expand_to_command_line(object, main_repo_mapping) {
        Expanded::DerivedArtifact(artifact) => {
            builder.add_preprocessed_arg(PreprocessedArg::SingleFormattedArtifact {
                format: format_str,
                artifact,
            })
        }
        Expanded::String(string_value) => {
            builder.add_preprocessed_arg(PreprocessedArg::SingleFormatted {
                format: format_str,
                string_value,
            })
        }
    }
    argi
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:830 SingleFormattedArg.addToFingerprint
fn single_formatted_arg_add_to_fingerprint(
    arguments: &[Entry],
    mut argi: usize,
    fingerprint: &mut Fingerprint,
    main_repo_mapping: Option<&RepositoryMapping>,
) -> usize {
    let object = arguments[argi].as_object();
    argi += 1;
    add_single_object_to_fingerprint(fingerprint, object, main_repo_mapping);
    let format_str = arguments[argi].as_string();
    argi += 1;
    fingerprint.add_string(format_str);
    add_uuid(fingerprint, SINGLE_FORMATTED_ARG_UUID);
    argi
}

// ---------------------------------------------------------------------------------------------
// map_each

/// Discards `print()` output of `map_each` callbacks (Java: `thread.setPrintHandler((th, msg) ->
/// {})`).
struct NoopEvents;

impl StarlarkEvents for NoopEvents {
    fn debug(&self, _location: Option<&Location>, _message: &str) {}

    fn warn(&self, _location: Option<&Location>, _message: &str) {}
}

/// Extends the lifetime of an input metadata provider so that a Starlark value
/// (`DirectoryExpander`) or the map-each thread-local can refer to it.
///
/// # Safety
///
/// The caller must make sure no reference obtained from the result is used after `provider`
/// goes out of scope: the expander lives on the temporary heap of one `with_evaluator` call
/// that ends before the provider's borrow does, and the thread-local is reset by
/// [`ProviderScope`] before the borrow ends.
unsafe fn extend_provider_lifetime<'a>(
    provider: &'a dyn InputMetadataProvider,
) -> &'static dyn InputMetadataProvider {
    // SAFETY: see the function contract; only the lifetime changes.
    unsafe {
        std::mem::transmute::<&'a dyn InputMetadataProvider, &'static dyn InputMetadataProvider>(
            provider,
        )
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1125 StarlarkCustomCommandLine.applyMapEach
fn apply_map_each(
    map_fn: &OwnedFrozenValue,
    original_values: &[ArgValue],
    consumer: &mut dyn FnMut(String),
    loc: &Location,
    input_metadata_provider: Option<&dyn InputMetadataProvider>,
    path_mapper: &dyn PathMapper,
    starlark_semantics: &Arc<StarlarkSemantics>,
) -> Result<(), CommandLineError> {
    if !path_mapper.is_noop() {
        // Java stores the path mapper in the thread's semantics (`pathMapper.storeIn`) so that
        // File.path etc. return mapped paths inside the callback.
        unported!("PathMapper.storeIn (path mapping inside map_each)");
    }
    // This computation produces only a String list, which doesn't require reference semantics,
    // so a transient thread is safe.
    let ctx = ThreadContext::new(
        Phase::Other,
        starlark_semantics.clone(),
        Arc::new(NoopEvents),
    );
    let wants_expander = wants_directory_expander(map_fn);
    // SAFETY: the expander holding this reference is allocated on the temporary heap of the
    // `with_evaluator` call below, which is dropped before this function returns.
    let provider: Option<&'static dyn InputMetadataProvider> =
        input_metadata_provider.map(|p| unsafe { extend_provider_lifetime(p) });
    let outcome = with_evaluator(&ctx, None, &EvalOptions::default(), |module, eval| {
        let f = frozen_into(module, map_fn);
        // We create a list that we reuse for the args to map_each
        let mut args: Vec<Value> = Vec::with_capacity(2);
        args.push(Value::new_none()); // This will be overwritten each iteration.
        // map_each can accept either each object, or each object + a directory expander.
        if wants_expander {
            let expander = DirectoryExpander { provider };
            args.push(eval.heap().alloc(expander)); // This will remain constant each iteration
        }
        for value in original_values {
            args[0] = value.to_starlark(module, eval.heap());
            let ret = eval
                .eval_function(f, &args, &[])
                .map_err(|e| EvalError::from_starlark(&e))?;
            if let Some(string) = ret.unpack_str() {
                consumer(string.to_owned());
            } else if is_sequence(ret) {
                let items = ret
                    .iterate(eval.heap())
                    .map_err(|e| EvalError::from_starlark(&e))?;
                for val in items {
                    match val.unpack_str() {
                        Some(s) => consumer(s.to_owned()),
                        None => {
                            return Ok(Err(CommandLineExpansionException::new(format!(
                                "Expected map_each to return string, None, or list of strings, \
                                 found list containing {}",
                                val.get_type()
                            ))));
                        }
                    }
                }
            } else if !ret.is_none() {
                return Ok(Err(CommandLineExpansionException::new(format!(
                    "Expected map_each to return string, None, or list of strings, found {}",
                    ret.get_type()
                ))));
            }
        }
        Ok(Ok(()))
    });
    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(MapFnError::CommandLineExpansion(e)),
        Err(e) if e.kind == EvalErrorKind::Cancelled => {
            Err(MapFnError::Interrupted(InterruptedException))
        }
        // TODO(adonovan): consider calling a wrapper function to interpose a fake stack
        // frame that establishes the args.add_all call at loc. Or manipulating the stack
        // before printing it.
        Err(e) => Err(MapFnError::CommandLineExpansion(
            CommandLineExpansionException::new(error_message(
                &e.message_with_stack(),
                Some(loc),
                None,
            )),
        )),
    }
}

/// Java `ret instanceof Sequence`: list, tuple or range.
fn is_sequence(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range")
}

/// Java `mapFn instanceof StarlarkFunction && getParameterNames().size() >= 2`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1185 StarlarkCustomCommandLine.wantsDirectoryExpander
fn wants_directory_expander(map_fn: &OwnedFrozenValue) -> bool {
    map_fn
        .value()
        .parameters_spec()
        .is_some_and(|p| p.len() >= 2)
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1293 StarlarkCustomCommandLine.errorMessage
fn error_message(message: &str, location: Option<&Location>, cause: Option<&str>) -> String {
    // Java: LINE_JOINER.join("\n", FIELD_JOINER.join(location, message), causeMessage) with
    // skipNulls: the literal "\n" is the first joined element.
    let mut field = String::new();
    if let Some(location) = location {
        field.push_str(&location.to_string());
        field.push_str(": ");
    }
    field.push_str(message);
    let mut parts = vec!["\n".to_owned(), field];
    if let Some(cause) = get_cause_message(cause, Some(message)) {
        parts.push(cause);
    }
    parts.join("\n")
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1300 StarlarkCustomCommandLine.getCauseMessage
fn get_cause_message(cause_message: Option<&str>, message: Option<&str>) -> Option<String> {
    let cause_message = cause_message?;
    let Some(message) = message else {
        return Some(cause_message.to_owned());
    };
    // Skip the cause if it is redundant with the message so far.
    if message.contains(cause_message) {
        return None;
    }
    Some(cause_message.to_owned())
}

/// The `DirectoryExpander` passed as second argument to two-parameter `map_each` functions.
/// `provider` is `None` for Java's `NoopExpander` (used during action key evaluation without an
/// input metadata provider) and `Some` for `FullExpander`.
// Java: com/google/devtools/build/lib/starlarkbuildapi/DirectoryExpander.java:34 DirectoryExpander
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1090 NoopExpander
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1099 FullExpander
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
struct DirectoryExpander {
    #[allocative(skip)]
    provider: Option<&'static dyn InputMetadataProvider>,
}

starlark::starlark_simple_value!(DirectoryExpander);

impl fmt::Debug for DirectoryExpander {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.class_name())
    }
}

impl DirectoryExpander {
    fn class_name(&self) -> &'static str {
        if self.provider.is_some() {
            "com.google.devtools.build.lib.analysis.starlark.StarlarkCustomCommandLine$FullExpander"
        } else {
            "com.google.devtools.build.lib.analysis.starlark.StarlarkCustomCommandLine$NoopExpander"
        }
    }
}

impl fmt::Display for DirectoryExpander {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<unknown object {}>", self.class_name())
    }
}

#[starlark_value(type = "DirectoryExpander")]
impl<'v> StarlarkValue<'v> for DirectoryExpander {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("DirectoryExpander", directory_expander_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn directory_expander_methods(builder: &mut MethodsBuilder) {
    /// Java: DirectoryExpander.list (NoopExpander.list, FullExpander.list)
    fn expand<'v>(
        this: &DirectoryExpander,
        #[starlark(require = pos)] file: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        let Some(provider) = this.provider else {
            // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1092 NoopExpander.list
            return Ok(heap.alloc(AllocList([file])));
        };
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1107 FullExpander.list
        let artifact = match file.downcast_ref::<StarlarkFile>() {
            Some(f) => f.0.clone(),
            None => panic!(
                "ClassCastException: class {} cannot be cast to class Artifact",
                file.get_type()
            ),
        };
        if artifact.is_tree_artifact() {
            let Some(children) = provider.get_tree_children(&artifact) else {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Failed to expand directory {}. Only directories that are action inputs can be \
                     expanded.",
                    artifact.repr()
                )));
            };
            let values: Vec<Value<'v>> = children
                .into_iter()
                .map(|c| heap.alloc(StarlarkFile(c)))
                .collect();
            Ok(heap.alloc(AllocList(values)))
        } else {
            Ok(heap.alloc(AllocList([file])))
        }
    }
}

thread_local! {
    /// The input metadata provider of the `CommandLineItemMapEachAdaptor`s used by the running
    /// nested set fingerprint computation (Java: the adaptor's `inputMetadataProvider` field,
    /// cleared after the computation).
    static MAP_EACH_PROVIDER: Cell<Option<&'static dyn InputMetadataProvider>> =
        const { Cell::new(None) };
}

/// Makes a provider visible to the adaptors for the duration of one fingerprint computation.
struct ProviderScope {
    previous: Option<&'static dyn InputMetadataProvider>,
}

impl ProviderScope {
    fn enter(provider: Option<&dyn InputMetadataProvider>) -> ProviderScope {
        // SAFETY: the thread-local is restored when the returned scope is dropped, which happens
        // before `provider`'s borrow ends (the scope is a local of the borrowing caller).
        let provider = provider.map(|p| unsafe { extend_provider_lifetime(p) });
        let previous = MAP_EACH_PROVIDER.with(|c| c.replace(provider));
        ProviderScope { previous }
    }
}

impl Drop for ProviderScope {
    fn drop(&mut self) {
        MAP_EACH_PROVIDER.with(|c| c.set(self.previous));
    }
}

/// Adapts a `map_each` function to `CommandLineItem.MapFn` for nested set fingerprinting.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1190 CommandLineItemMapEachAdaptor
//
// PORT: Java creates one adaptor per fingerprint computation; the fingerprint cache keys its
// memo tables by adaptor equality (same callable instance, same `hasInputMetadataProvider`, same
// output paths mode). The Rust cache needs `&'static` map fns, so equal adaptors are interned
// (and leaked, with the callable's frozen heap) in a process-wide table; the first adaptor's
// location is used for error messages, as Java's cache would also reuse the first entry. The
// input metadata provider is passed through a thread-local scoped to the computation.
struct CommandLineItemMapEachAdaptor {
    map_fn: OwnedFrozenValue,
    /// Stands in for `System.identityHashCode(mapFn)`.
    identity: i32,
    location: Location,
    starlark_semantics: Arc<StarlarkSemantics>,
    /// Indicates whether an input metadata provider was provided on construction.
    has_input_metadata_provider: bool,
    output_paths_mode: OutputPathsMode,
}

static MAP_EACH_ADAPTORS: LazyLock<Mutex<Vec<&'static CommandLineItemMapEachAdaptor>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

impl CommandLineItemMapEachAdaptor {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1207 CommandLineItemMapEachAdaptor.<init>
    fn intern(
        map_fn: &OwnedFrozenValue,
        location: &Location,
        starlark_semantics: &Arc<StarlarkSemantics>,
        has_input_metadata_provider: bool,
        output_paths_mode: OutputPathsMode,
    ) -> &'static CommandLineItemMapEachAdaptor {
        let mut table = MAP_EACH_ADAPTORS.lock();
        let mut identity: Option<i32> = None;
        for a in table.iter() {
            if a.map_fn.value().ptr_eq(map_fn.value()) {
                if a.has_input_metadata_provider == has_input_metadata_provider
                    && a.output_paths_mode == output_paths_mode
                {
                    return a;
                }
                identity = Some(a.identity);
            }
        }
        let identity = identity.unwrap_or(table.len() as i32);
        let adaptor: &'static CommandLineItemMapEachAdaptor =
            Box::leak(Box::new(CommandLineItemMapEachAdaptor {
                map_fn: map_fn.clone(),
                identity,
                location: location.clone(),
                starlark_semantics: starlark_semantics.clone(),
                has_input_metadata_provider,
                output_paths_mode,
            }));
        table.push(adaptor);
        adaptor
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1235 CommandLineItemMapEachAdaptor.maybeExpandDirectory
    fn maybe_expand_directory(
        &self,
        object: &ArgValue,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> Result<Vec<ArgValue>, CommandLineExpansionException> {
        let Some(input_metadata_provider) = input_metadata_provider else {
            return Ok(vec![object.clone()]);
        };
        if !is_directory(object) {
            return Ok(vec![object.clone()]);
        }
        expand_directories(
            input_metadata_provider,
            std::slice::from_ref(object),
            for_action_key(self.output_paths_mode),
        )
    }
}

impl MapFn<ArgValue> for CommandLineItemMapEachAdaptor {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1222 CommandLineItemMapEachAdaptor.expandToCommandLine
    fn expand_to_command_line(
        &self,
        object: &ArgValue,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        let input_metadata_provider = if self.has_input_metadata_provider {
            MAP_EACH_PROVIDER.with(|c| c.get())
        } else {
            None
        };
        check_state!(input_metadata_provider.is_some() || !self.has_input_metadata_provider);
        let values = self.maybe_expand_directory(object, input_metadata_provider)?;
        apply_map_each(
            &self.map_fn,
            &values,
            &mut |s| args(&s),
            &self.location,
            input_metadata_provider,
            for_action_key(self.output_paths_mode),
            &self.starlark_semantics,
        )
    }

    fn as_parametrized_map_fn(&self) -> Option<&dyn ParametrizedMapFn> {
        Some(self)
    }
}

impl ParametrizedMapFn for CommandLineItemMapEachAdaptor {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1247 CommandLineItemMapEachAdaptor.equals
    fn equals(&self, other: &dyn std::any::Any) -> bool {
        let Some(other) = other.downcast_ref::<CommandLineItemMapEachAdaptor>() else {
            return false;
        };
        // Instance compare intentional
        // The normal implementation uses location + name of function,
        // which can conceivably conflict in tests
        // We only compare presence of inputMetadataProvider vs absence of it since the nested set
        // fingerprint cache is emptied after every build, therefore if the artifact expander is
        // provided, it will be the same.
        self.map_fn.value().ptr_eq(other.map_fn.value())
            && self.has_input_metadata_provider == other.has_input_metadata_provider
            && self.output_paths_mode == other.output_paths_mode
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1263 CommandLineItemMapEachAdaptor.hashCode
    fn hash_code(&self) -> i32 {
        // PORT: Java's enum hashCode and identityHashCode are arbitrary; the ordinal and the
        // intern identity stand in for them.
        let mode_hash: i32 = match self.output_paths_mode {
            OutputPathsMode::OFF => 0,
            OutputPathsMode::STRIP => 1,
        };
        let bool_hash: i32 = if self.has_input_metadata_provider {
            1231
        } else {
            1237
        };
        mode_hash.wrapping_add(31i32.wrapping_mul(
            bool_hash.wrapping_add(31i32.wrapping_mul(self.identity.wrapping_add(1))),
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1274 CommandLineItemMapEachAdaptor.maxInstancesAllowed
    fn max_instances_allowed(&self) -> i32 {
        // No limit to these, as this is just a wrapper for Starlark functions, which are
        // always static
        i32::MAX
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1322 FilesetSymlinkFile
// PORT: not ported; it is only created by fileset expansion (`VectorArg.expandFileset`), which
// calls `unported!`.

// ---------------------------------------------------------------------------------------------
// Preprocessed command lines

/// An element in a `PreprocessedCommandLine`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1407 PreprocessedArg
enum PreprocessedArg {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1486 PreprocessedStringArg
    String(String),
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1509 PreprocessedArtifactArg
    Artifact(Artifact),
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1535 PreprocessedSingleFormattedArg
    SingleFormatted {
        format: String,
        string_value: String,
    },
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1561 PreprocessedSingleFormattedArtifactArg
    SingleFormattedArtifact {
        format: String,
        artifact: Artifact,
    },
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1591 UnjoinedPreprocessedVectorArg
    UnjoinedVector {
        values: Vec<Expanded>,
        format_each: Option<String>,
        before_each: Option<String>,
    },
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1641 JoinedPreprocessedVectorArg
    JoinedVector {
        values: Vec<Expanded>,
        format_each: Option<String>,
        join_with: String,
        format_joined: Option<String>,
    },
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1696 GroupedPreprocessedArgs
    Grouped(Vec<PreprocessedArg>),
}

/// Java `String.length()` for command line strings.
// PORT: as in `SimpleArgChunk.totalArgLength`, the length of Bazel's internal (Latin-1) string
// form is the UTF-8 byte count.
fn len(s: &str) -> i32 {
    s.len() as i32
}

impl PreprocessedArg {
    fn to_iterable(&self, path_mapper: &dyn PathMapper) -> Vec<String> {
        match self {
            PreprocessedArg::String(arg) => vec![arg.clone()],
            PreprocessedArg::Artifact(artifact) => {
                vec![path_mapper.get_mapped_exec_path_string(artifact)]
            }
            PreprocessedArg::SingleFormatted {
                format,
                string_value,
            } => vec![single_string_arg_formatter::format(format, string_value)],
            PreprocessedArg::SingleFormattedArtifact { format, artifact } => {
                vec![single_string_arg_formatter::format(
                    format,
                    &path_mapper.get_mapped_exec_path_string(artifact),
                )]
            }
            PreprocessedArg::UnjoinedVector {
                values,
                format_each,
                before_each,
            } => {
                let mut list: Vec<String> = values
                    .iter()
                    .map(|v| maybe_path_map(v, path_mapper))
                    .collect();
                if let Some(format_each) = format_each {
                    list = list
                        .iter()
                        .map(|s| single_string_arg_formatter::format(format_each, s))
                        .collect();
                }
                match before_each {
                    None => list,
                    // Java: BeforeEachIterator
                    Some(before_each) => {
                        let mut out = Vec::with_capacity(list.len() * 2);
                        for s in list {
                            out.push(before_each.clone());
                            out.push(s);
                        }
                        out
                    }
                }
            }
            PreprocessedArg::JoinedVector {
                values,
                format_each,
                join_with,
                format_joined,
            } => {
                let mut it: Vec<String> = values
                    .iter()
                    .map(|v| maybe_path_map(v, path_mapper))
                    .collect();
                if let Some(format_each) = format_each {
                    it = it
                        .iter()
                        .map(|s| single_string_arg_formatter::format(format_each, s))
                        .collect();
                }
                let mut result = it.join(join_with);
                if let Some(format_joined) = format_joined {
                    result = single_string_arg_formatter::format(format_joined, &result);
                }
                vec![result]
            }
            PreprocessedArg::Grouped(args) => {
                let all: Vec<String> = args
                    .iter()
                    .flat_map(|a| a.to_iterable(path_mapper))
                    .collect();
                let mut it = all.into_iter();
                let first = it.next().expect("NoSuchElementException");
                let rest = it.collect::<Vec<_>>().join(" ");
                let line = if first.is_empty() {
                    rest
                } else {
                    format!("{first}={rest}")
                };
                vec![line]
            }
        }
    }

    fn num_args(&self) -> i32 {
        match self {
            PreprocessedArg::UnjoinedVector {
                values,
                before_each,
                ..
            } => (if before_each.is_some() { 2 } else { 1 }) * values.len() as i32,
            _ => 1,
        }
    }

    fn total_arg_length(&self, path_mapper: &dyn PathMapper) -> i32 {
        match self {
            PreprocessedArg::String(arg) => len(arg) + 1,
            PreprocessedArg::Artifact(artifact) => {
                len(artifact.get_exec_path_string())
                    - path_mapper.compute_exec_path_length_diff(artifact)
                    + 1
            }
            PreprocessedArg::SingleFormatted {
                format,
                string_value,
            } => single_string_arg_formatter::formatted_length(format) + len(string_value) + 1,
            PreprocessedArg::SingleFormattedArtifact { format, artifact } => {
                single_string_arg_formatter::formatted_length(format)
                    + len(artifact.get_exec_path_string())
                    - path_mapper.compute_exec_path_length_diff(artifact)
                    + 1
            }
            PreprocessedArg::UnjoinedVector {
                values,
                format_each,
                before_each,
            } => {
                let mut total: i32 = 0;
                for arg in values {
                    total = total.wrapping_add(arg_length(arg, path_mapper));
                }
                if let Some(format_each) = format_each {
                    total = total.wrapping_add(
                        single_string_arg_formatter::formatted_length(format_each)
                            .wrapping_mul(values.len() as i32),
                    );
                }
                if let Some(before_each) = before_each {
                    total = total.wrapping_add(len(before_each).wrapping_mul(values.len() as i32));
                }
                total.wrapping_add(self.num_args())
            }
            PreprocessedArg::JoinedVector {
                values,
                format_each,
                join_with,
                format_joined,
            } => {
                let mut total: i32 = 0;
                for arg in values {
                    total = total.wrapping_add(arg_length(arg, path_mapper));
                }
                if let Some(format_each) = format_each {
                    total = total.wrapping_add(
                        single_string_arg_formatter::formatted_length(format_each)
                            .wrapping_mul(values.len() as i32),
                    );
                }
                if values.len() > 1 {
                    total =
                        total.wrapping_add(len(join_with).wrapping_mul(values.len() as i32 - 1));
                }
                if let Some(format_joined) = format_joined {
                    total = total
                        .wrapping_add(single_string_arg_formatter::formatted_length(format_joined));
                }
                total.wrapping_add(1)
            }
            PreprocessedArg::Grouped(args) => {
                let mut total: i32 = 0;
                for arg in args {
                    total = total.wrapping_add(arg.total_arg_length(path_mapper));
                }
                let first = args
                    .iter()
                    .flat_map(|a| a.to_iterable(path_mapper))
                    .next()
                    .expect("NoSuchElementException");
                if first.is_empty() {
                    total -= 1;
                }
                total
            }
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1737 StarlarkCustomCommandLine.maybePathMap
fn maybe_path_map(arg: &Expanded, path_mapper: &dyn PathMapper) -> String {
    match arg {
        Expanded::String(s) => s.clone(),
        Expanded::DerivedArtifact(artifact) => path_mapper.get_mapped_exec_path_string(artifact),
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1746 StarlarkCustomCommandLine.argLength
fn arg_length(arg: &Expanded, path_mapper: &dyn PathMapper) -> i32 {
    match arg {
        Expanded::String(s) => len(s),
        Expanded::DerivedArtifact(artifact) => {
            len(artifact.get_exec_path_string())
                - path_mapper.compute_exec_path_length_diff(artifact)
        }
    }
}

/// Intermediate command line representation with directory expansion and `map_each` already
/// applied, but with string formatting and path mapping not yet applied.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1424 PreprocessedCommandLine
struct PreprocessedCommandLine {
    preprocessed_args: Vec<PreprocessedArg>,
}

impl ArgChunk for PreprocessedCommandLine {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1432 PreprocessedCommandLine.arguments
    fn arguments(&self, path_mapper: &dyn PathMapper) -> Vec<String> {
        self.preprocessed_args
            .iter()
            .flat_map(|arg| arg.to_iterable(path_mapper))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1437 PreprocessedCommandLine.totalArgLength
    fn total_arg_length(&self, path_mapper: &dyn PathMapper) -> i32 {
        let mut total: i32 = 0;
        for arg in &self.preprocessed_args {
            total = total.wrapping_add(arg.total_arg_length(path_mapper));
        }
        total
    }
}

// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkCustomCommandLine.java:1445 PreprocessedCommandLine.Builder
struct PreprocessedCommandLineBuilder {
    preprocessed_args: Vec<PreprocessedArg>,
    num_args: i32,
}

impl PreprocessedCommandLineBuilder {
    fn new() -> PreprocessedCommandLineBuilder {
        PreprocessedCommandLineBuilder {
            preprocessed_args: Vec::new(),
            num_args: 0,
        }
    }

    fn add_preprocessed_arg(&mut self, arg: PreprocessedArg) {
        self.num_args += arg.num_args();
        self.preprocessed_args.push(arg);
    }

    fn add_string(&mut self, string: String) {
        self.add_preprocessed_arg(PreprocessedArg::String(string));
    }

    fn add_arg(&mut self, arg: Expanded) {
        match arg {
            Expanded::String(string) => self.add_preprocessed_arg(PreprocessedArg::String(string)),
            Expanded::DerivedArtifact(artifact) => {
                self.add_preprocessed_arg(PreprocessedArg::Artifact(artifact))
            }
        }
    }

    fn add_line_for_flag_per_line(&mut self, line: PreprocessedCommandLineBuilder) {
        let group = line.preprocessed_args;
        if line.num_args < 2 {
            for arg in group {
                self.add_preprocessed_arg(arg);
            }
        } else {
            self.add_preprocessed_arg(PreprocessedArg::Grouped(group));
        }
    }

    fn build(self) -> PreprocessedCommandLine {
        PreprocessedCommandLine {
            preprocessed_args: self.preprocessed_args,
        }
    }
}
