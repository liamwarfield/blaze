//! Port of `lib/analysis/actions/CustomCommandLine.java`.
//!
//! PORT: Java stores the command line as a flat `Object[]` of raw arguments interleaved with
//! `ArgvFragment` markers that consume the following slots. The port keeps that flat layout
//! ([`Obj`]) with one deviation: a vector arg's map function and values (Java: `[mapFn]`, then
//! either the nested set or `count` followed by the elements) occupy one typed slot
//! ([`Obj::VectorValues`]), because Rust needs the element type to apply the map function. The
//! only Java code that looks at neighbouring slots (`VectorArgFragment.eval` reading
//! `arguments.get(argi - 1)`) always sees the fragment itself, so the outputs are unchanged.
//!
//! Strings: command line strings are internal strings (`blaze_util::string_encoding`), as in
//! Java, so fingerprints add them with `Fingerprint::add_string` where Java calls `addString`
//! (see `actions::fingerprint_util`).

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_collect::NestedSet;
use blaze_collect::command_line_item::{DefaultMapFn, ExceptionlessMapFn, MapFn, MapFnError};
use blaze_util::{Fingerprint, check_argument, unported};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::action::{CommandLineError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::actions::command_line::CommandLine;
use crate::actions::path_mapper::{NOOP, OutputPathsMode, PathMapper};
use crate::actions::single_string_arg_formatter;

// UUIDs as (least significant bits, most significant bits), the order `Fingerprint.addUUID`
// writes them.

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:354 VectorArgFragment.FORMAT_EACH_UUID
// "f830781f-2e0d-4e3b-9b99-ece7f249e0f3"
const FORMAT_EACH_UUID: (i64, i64) = (
    0x9b99_ece7_f249_e0f3_u64 as i64,
    0xf830_781f_2e0d_4e3b_u64 as i64,
);
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:356 VectorArgFragment.BEFORE_EACH_UUID
// "07d22a0d-2691-4f1c-9f47-5294de1f94e4"
const BEFORE_EACH_UUID: (i64, i64) = (
    0x9f47_5294_de1f_94e4_u64 as i64,
    0x07d2_2a0d_2691_4f1c_u64 as i64,
);
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:358 VectorArgFragment.JOIN_WITH_UUID
// "c96ed6f0-9220-40f6-9e0c-1c0c5e0b47e4"
const JOIN_WITH_UUID: (i64, i64) = (
    0x9e0c_1c0c_5e0b_47e4_u64 as i64,
    0xc96e_d6f0_9220_40f6_u64 as i64,
);
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:536 FormatArg.FORMAT_UUID
// "377cee34-e947-49e0-94a2-6ab95b396ec4"
const FORMAT_UUID: (i64, i64) = (
    0x94a2_6ab9_5b39_6ec4_u64 as i64,
    0x377c_ee34_e947_49e0_u64 as i64,
);
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:582 PrefixArg.PREFIX_UUID
// "a95eccdf-4f54-46fc-b925-c8c7e1f50c95"
const PREFIX_UUID: (i64, i64) = (
    0xb925_c8c7_e1f5_0c95_u64 as i64,
    0xa95e_ccdf_4f54_46fc_u64 as i64,
);
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:686 ExpandedTreeArtifactArg.TREE_UUID
// "13b7626b-c77d-4a30-ad56-ff08c06b1cee"
const TREE_UUID: (i64, i64) = (
    0xad56_ff08_c06b_1cee_u64 as i64,
    0x13b7_626b_c77d_4a30_u64 as i64,
);

fn add_uuid(fingerprint: &mut Fingerprint, uuid: (i64, i64)) {
    fingerprint.add_uuid(uuid.0, uuid.1);
}

/// A `CharSequence` that delays string construction (mostly till the execution phase); its
/// `toString()` is the `Display` output.
// Java: com/google/devtools/build/lib/util/OnDemandString.java:21 OnDemandString
// PORT: lib/util's OnDemandString is not ported elsewhere; any `Display` type qualifies.
pub trait OnDemandString: fmt::Display + fmt::Debug + Send + Sync {}

impl<T: fmt::Display + fmt::Debug + Send + Sync + ?Sized> OnDemandString for T {}

/// An object added to a `CustomCommandLine` whose string form is computed at expansion time
/// (Java: the `Object` parameters of `addObject` and `addFormatted`).
///
/// PORT: Java accepts any `Object` (expanded with `CommandLineItem.expandToCommandLine`); these
/// are the kinds the builder methods admit.
#[derive(Clone, Debug)]
pub enum CommandLineArg {
    String(String),
    /// An `ActionInput`: expanded to its (possibly path-mapped) exec path.
    Artifact(Artifact),
    /// Expanded to its path string (path-mapped at top level).
    PathFragment(PathFragment),
    /// Expanded to its canonical form.
    Label(Label),
    /// Expanded with `toString()`.
    LazyString(Arc<dyn OnDemandString>),
}

impl CommandLineArg {
    /// `CommandLineItem.expandToCommandLine(object)`.
    // Java: com/google/devtools/build/lib/actions/CommandLineItem.java:75 CommandLineItem.expandToCommandLine
    pub fn expand_to_command_line(&self) -> String {
        match self {
            CommandLineArg::String(s) => s.clone(),
            CommandLineArg::Artifact(a) => a.get_exec_path_string().to_string(),
            // Java: PathStrippable.expandToCommandLine → expand(identity) → normalizedPath.
            CommandLineArg::PathFragment(p) => p.get_path_string().to_string(),
            // Java: com/google/devtools/build/lib/cmdline/Label.java:757 Label.expandToCommandLine
            CommandLineArg::Label(l) => l.get_canonical_form(),
            CommandLineArg::LazyString(s) => s.to_string(),
        }
    }
}

impl From<String> for CommandLineArg {
    fn from(s: String) -> Self {
        CommandLineArg::String(s)
    }
}

impl From<&str> for CommandLineArg {
    fn from(s: &str) -> Self {
        CommandLineArg::String(s.to_string())
    }
}

impl From<Artifact> for CommandLineArg {
    fn from(a: Artifact) -> Self {
        CommandLineArg::Artifact(a)
    }
}

impl From<PathFragment> for CommandLineArg {
    fn from(p: PathFragment) -> Self {
        CommandLineArg::PathFragment(p)
    }
}

impl From<Label> for CommandLineArg {
    fn from(l: Label) -> Self {
        CommandLineArg::Label(l)
    }
}

/// An element type of a vector arg (Java: the `T` of `VectorArg<T>`).
///
/// PORT: Java's vector args hold arbitrary objects; Rust needs to know how an element expands
/// (`CommandLineItem.expandToCommandLine`), whether it is an `ActionInput`, and how a nested set
/// of it is fingerprinted with `MapFn.DEFAULT`.
pub trait VectorArgElement: Clone + Eq + Hash + Send + Sync + fmt::Debug + 'static {
    /// `CommandLineItem.expandToCommandLine(object)`.
    fn expand_item_to_command_line(&self) -> String;

    /// `object instanceof ActionInput`.
    fn as_action_input(&self) -> Option<&Artifact> {
        None
    }

    /// `actionKeyContext.addNestedSetToFingerprint(fingerprint, nestedSet)` (with
    /// `CommandLineItem.MapFn.DEFAULT`).
    fn add_nested_set_to_fingerprint(
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<Self>,
    ) -> Result<(), MapFnError> {
        // PORT: MapFn.DEFAULT needs `T: CommandLineItem`, which foreign types (PathFragment,
        // Label) cannot implement here. A per-type map fn that expands with
        // `expand_item_to_command_line` writes the same strings, hence the same bytes.
        let map_fn: &'static ExpandItemMapFn<Self> =
            Box::leak(Box::new(ExpandItemMapFn(PhantomData)));
        action_key_context.add_nested_set_to_fingerprint_with_map_fn(
            map_fn,
            fingerprint,
            nested_set,
        )
    }
}

/// `MapFn.DEFAULT` for a [`VectorArgElement`] type (a zero-sized singleton per type).
pub struct ExpandItemMapFn<T>(PhantomData<fn(&T)>);

impl<T: VectorArgElement> MapFn<T> for ExpandItemMapFn<T> {
    fn expand_to_command_line(
        &self,
        object: &T,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&object.expand_item_to_command_line());
        Ok(())
    }
}

impl<T: VectorArgElement> ExceptionlessMapFn<T> for ExpandItemMapFn<T> {}

impl VectorArgElement for String {
    fn expand_item_to_command_line(&self) -> String {
        self.clone()
    }

    fn add_nested_set_to_fingerprint(
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<Self>,
    ) -> Result<(), MapFnError> {
        action_key_context.add_nested_set_to_fingerprint(fingerprint, nested_set)
    }
}

impl VectorArgElement for Artifact {
    fn expand_item_to_command_line(&self) -> String {
        self.get_exec_path_string().to_string()
    }

    fn as_action_input(&self) -> Option<&Artifact> {
        Some(self)
    }

    fn add_nested_set_to_fingerprint(
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<Self>,
    ) -> Result<(), MapFnError> {
        action_key_context.add_nested_set_to_fingerprint(fingerprint, nested_set)
    }
}

impl VectorArgElement for PathFragment {
    fn expand_item_to_command_line(&self) -> String {
        self.get_path_string().to_string()
    }
}

impl VectorArgElement for Label {
    fn expand_item_to_command_line(&self) -> String {
        self.get_canonical_form()
    }
}

/// Java `pathMapper.getMapFn(previousFlag)` applied to an arbitrary value.
///
/// PORT: the Rust `PathMapper::get_map_fn` is typed over artifacts. `MapFn.DEFAULT` (the only map
/// fn of the ported path mappers) expands any value; a custom path mapper map fn applied to a
/// non-artifact value needs `StrippingPathMapper`, which is not ported.
fn apply_path_mapper_map_fn<T: VectorArgElement>(
    map_fn: &dyn ExceptionlessMapFn<Artifact>,
    object: &T,
    out: &mut Vec<String>,
) {
    if map_fn.class_name() == std::any::type_name::<DefaultMapFn>() {
        out.push(object.expand_item_to_command_line());
        return;
    }
    match object.as_action_input() {
        Some(artifact) => {
            let result = map_fn.expand_to_command_line(artifact, &mut |s: &str| {
                out.push(s.to_string());
            });
            if let Err(e) = result {
                panic!("IllegalStateException: ExceptionlessMapFn threw: {e}");
            }
        }
        None => unported!("StrippingPathMapper.getMapFn"),
    }
}

/// The values of a vector arg or of a top-level nested set: a collection or a nested set.
#[derive(Clone)]
enum VectorValues<T> {
    Collection(Vec<T>),
    NestedSet(NestedSet<T>),
}

/// A vector arg's values and optional map function, with the element type erased.
trait ErasedVectorValues: Send + Sync + fmt::Debug {
    /// Java `VectorArgFragment.eval` up to (and including) the computation of `mutatedValues`.
    fn expand_values(&self, path_mapper: &dyn PathMapper) -> Result<Vec<String>, CommandLineError>;

    /// Java `VectorArgFragment.addToFingerprint` for the map fn and values slots.
    fn add_values_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError>;

    /// Java `CustomCommandLine.evalSimpleVectorArg(nestedSet.toList(), ...)`.
    fn eval_simple_vector_arg(
        &self,
        builder: &mut Vec<String>,
        path_mapper: &dyn PathMapper,
        previous_flag: Option<&str>,
    );

    /// Java `actionKeyContext.addNestedSetToFingerprint(fingerprint, nestedSet)` for a
    /// top-level nested set.
    fn add_simple_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError>;
}

struct TypedVectorValues<T: VectorArgElement> {
    values: VectorValues<T>,
    map_fn: Option<&'static dyn MapFn<T>>,
}

impl<T: VectorArgElement> fmt::Debug for TypedVectorValues<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("VectorValues");
        match &self.values {
            VectorValues::Collection(v) => d.field("collection", v),
            VectorValues::NestedSet(s) => d.field("nested_set", s),
        };
        d.field(
            "map_fn",
            &self.map_fn.map(|m| m.class_name()).unwrap_or("null"),
        );
        d.finish()
    }
}

impl<T: VectorArgElement> TypedVectorValues<T> {
    /// Java `VectorArgFragment.expandToCommandLine(object, pathMapper)`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:380 VectorArgFragment.expandToCommandLine
    fn expand_with_path_mapper(object: &T, path_mapper: &dyn PathMapper) -> String {
        // It'd be nice to build this into ActionInput's CommandLine interface so we don't have
        // to explicitly check if an object is a ActionInput. Unfortunately that would require
        // a lot more dependencies on the Java library ActionInput is built into.
        match object.as_action_input() {
            Some(action_input) => path_mapper.get_mapped_exec_path_string(action_input),
            None => object.expand_item_to_command_line(),
        }
    }

    fn expand_list(
        &self,
        list: &[T],
        has_map_each: bool,
        is_nested_set: bool,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let mut mutated_values: Vec<String> = Vec::with_capacity(list.len());
        if has_map_each {
            let map_fn = self.map_fn.expect("mapFn");
            for object in list {
                map_fn.expand_to_command_line(object, &mut |s: &str| {
                    mutated_values.push(s.to_string());
                })?;
            }
        } else if !path_mapper.is_noop() && !is_nested_set {
            // Allow the PathMapper to apply a map function to string arguments depending on the
            // previous argument (e.g. to modify exec paths obtained in string form from location
            // expansion).
            // PORT: Java reads `arguments.get(argi - 1)`, which is always this VectorArgFragment
            // (never a String), so the previous argument is always null.
            let map_fn = path_mapper.get_map_fn(None);
            for object in list {
                apply_path_mapper_map_fn(map_fn, object, &mut mutated_values);
            }
        } else {
            for object in list {
                mutated_values.push(Self::expand_with_path_mapper(object, path_mapper));
            }
        }
        Ok(mutated_values)
    }
}

impl<T: VectorArgElement> ErasedVectorValues for TypedVectorValues<T> {
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:391 VectorArgFragment.eval
    fn expand_values(&self, path_mapper: &dyn PathMapper) -> Result<Vec<String>, CommandLineError> {
        let has_map_each = self.map_fn.is_some();
        match &self.values {
            VectorValues::NestedSet(values) => {
                let list = values.to_list();
                self.expand_list(&list, has_map_each, true, path_mapper)
            }
            VectorValues::Collection(values) => {
                self.expand_list(values, has_map_each, false, path_mapper)
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:467 VectorArgFragment.addToFingerprint
    fn add_values_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        match &self.values {
            VectorValues::NestedSet(values) => match self.map_fn {
                Some(map_fn) => action_key_context.add_nested_set_to_fingerprint_with_map_fn(
                    map_fn,
                    fingerprint,
                    values,
                )?,
                None => T::add_nested_set_to_fingerprint(action_key_context, fingerprint, values)?,
            },
            VectorValues::Collection(values) => match self.map_fn {
                Some(map_fn) => {
                    for object in values {
                        map_fn.expand_to_command_line(object, &mut |s: &str| {
                            fingerprint.add_string(s);
                        })?;
                    }
                }
                None => {
                    for object in values {
                        fingerprint.add_string(&object.expand_item_to_command_line());
                    }
                }
            },
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1340 CustomCommandLine.evalSimpleVectorArg
    fn eval_simple_vector_arg(
        &self,
        builder: &mut Vec<String>,
        path_mapper: &dyn PathMapper,
        previous_flag: Option<&str>,
    ) {
        let map_fn = path_mapper.get_map_fn(previous_flag);
        let eval = |value: &T, builder: &mut Vec<String>| match value.as_action_input() {
            Some(action_input) => {
                builder.push(path_mapper.get_mapped_exec_path_string(action_input))
            }
            None => apply_path_mapper_map_fn(map_fn, value, builder),
        };
        match &self.values {
            VectorValues::NestedSet(values) => {
                for value in values.to_list().iter() {
                    eval(value, builder);
                }
            }
            VectorValues::Collection(values) => {
                for value in values {
                    eval(value, builder);
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1379 CustomCommandLine.addToFingerprint
    fn add_simple_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        match &self.values {
            VectorValues::NestedSet(values) => {
                T::add_nested_set_to_fingerprint(action_key_context, fingerprint, values)
            }
            // Java: `arg instanceof Iterable`.
            VectorValues::Collection(values) => {
                for value in values {
                    fingerprint.add_string(&value.expand_item_to_command_line());
                }
                Ok(())
            }
        }
    }
}

/// Builder for [`VectorArg`] (Java `VectorArg.Builder`).
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:269 VectorArg.Builder
#[derive(Clone, Debug, Default)]
pub struct VectorArgBuilder {
    format_each: Option<String>,
    before_each: Option<String>,
    join_with: Option<String>,
}

impl VectorArgBuilder {
    pub fn new() -> VectorArgBuilder {
        VectorArgBuilder::default()
    }

    /// Each argument is formatted via `SingleStringArgFormatter.format`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:276 VectorArg.Builder.format
    pub fn format(mut self, format_each: impl Into<String>) -> VectorArgBuilder {
        self.format_each = Some(format_each.into());
        self
    }

    /// Each argument is prepended by the beforeEach param.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:284 VectorArg.Builder.addBefore
    pub fn add_before(mut self, before_each: impl Into<String>) -> VectorArgBuilder {
        self.before_each = Some(before_each.into());
        self
    }

    /// Once all arguments have been evaluated, they are joined with this delimiter.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:292 VectorArg.Builder.join
    pub fn join(mut self, delimiter: impl Into<String>) -> VectorArgBuilder {
        self.join_with = Some(delimiter.into());
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:298 VectorArg.Builder.each
    pub fn each<T: VectorArgElement>(&self, values: Vec<T>) -> SimpleVectorArg<T> {
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:188 SimpleVectorArg.<init>
        SimpleVectorArg {
            is_nested_set: false,
            is_empty: values.is_empty(),
            count: values.len() as i32,
            format_each: self.format_each.clone(),
            before_each: self.before_each.clone(),
            join_with: self.join_with.clone(),
            values: VectorValues::Collection(values),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:302 VectorArg.Builder.each
    pub fn each_nested_set<T: VectorArgElement>(&self, values: NestedSet<T>) -> SimpleVectorArg<T> {
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:199 SimpleVectorArg.<init>
        SimpleVectorArg {
            is_nested_set: true,
            is_empty: values.is_empty(),
            count: -1,
            format_each: self.format_each.clone(),
            before_each: self.before_each.clone(),
            join_with: self.join_with.clone(),
            values: VectorValues::NestedSet(values),
        }
    }
}

/// The static factory methods of `VectorArg` (`VectorArg.of`, `VectorArg.format`, ...).
///
/// Vector args support formatting, interspersing args (adding strings before each value),
/// joining, and mapping custom types. Please use this whenever you need to transform lists or
/// nested sets instead of doing it manually, as use of this class is more memory efficient.
///
/// The order of evaluation is:
///
/// * Map the type T to a string using a custom map function, if any, or
/// * Map any non-string type {PathFragment, Artifact} to their path/exec path
/// * Format the string using the supplied format string, if any
/// * Add the arguments each prepended by the before string, if any, or
/// * Join the arguments with the join string, if any, or
/// * Simply add all arguments
pub mod vector_arg {
    use super::{NestedSet, SimpleVectorArg, VectorArgBuilder, VectorArgElement};

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:246 VectorArg.of
    pub fn of<T: VectorArgElement>(values: Vec<T>) -> SimpleVectorArg<T> {
        VectorArgBuilder::new().each(values)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:250 VectorArg.of
    pub fn of_nested_set<T: VectorArgElement>(values: NestedSet<T>) -> SimpleVectorArg<T> {
        VectorArgBuilder::new().each_nested_set(values)
    }

    /// Each argument is formatted via `SingleStringArgFormatter.format`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:255 VectorArg.format
    pub fn format(format_each: impl Into<String>) -> VectorArgBuilder {
        VectorArgBuilder::new().format(format_each)
    }

    /// Each argument is prepended by the beforeEach param.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:259 VectorArg.addBefore
    pub fn add_before(before_each: impl Into<String>) -> VectorArgBuilder {
        VectorArgBuilder::new().add_before(before_each)
    }

    /// Once all arguments have been evaluated, they are joined with this delimiter.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:264 VectorArg.join
    pub fn join(delimiter: impl Into<String>) -> VectorArgBuilder {
        VectorArgBuilder::new().join(delimiter)
    }
}

/// A vector arg that doesn't map its parameters.
///
/// Call [`SimpleVectorArg::mapped`] to produce a vector arg that maps from a given type to a
/// string.
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:185 VectorArg.SimpleVectorArg
#[derive(Clone)]
pub struct SimpleVectorArg<T> {
    is_nested_set: bool,
    is_empty: bool,
    count: i32,
    format_each: Option<String>,
    before_each: Option<String>,
    join_with: Option<String>,
    values: VectorValues<T>,
}

impl<T: VectorArgElement> SimpleVectorArg<T> {
    /// `SimpleVectorArg.of(collection)` (the inherited `VectorArg.of`).
    pub fn of(values: Vec<T>) -> SimpleVectorArg<T> {
        vector_arg::of(values)
    }

    /// `SimpleVectorArg.of(nestedSet)` (the inherited `VectorArg.of`).
    pub fn of_nested_set(values: NestedSet<T>) -> SimpleVectorArg<T> {
        vector_arg::of_nested_set(values)
    }

    /// Each argument is mapped using the supplied map function.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:223 SimpleVectorArg.mapped
    pub fn mapped(self, map_fn: &'static dyn MapFn<T>) -> VectorArg<String> {
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:233 MappedVectorArg.<init>
        VectorArg {
            is_nested_set: self.is_nested_set,
            is_empty: self.is_empty,
            count: self.count,
            format_each: self.format_each,
            before_each: self.before_each,
            join_with: self.join_with,
            values: Arc::new(TypedVectorValues {
                values: self.values,
                map_fn: Some(map_fn),
            }),
            has_map_each: true,
            _marker: PhantomData,
        }
    }
}

/// A vector arg (Java `VectorArg<T>`): a [`SimpleVectorArg`] or a mapped one
/// (`MappedVectorArg<T> extends VectorArg<String>`).
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:156 VectorArg
#[derive(Clone)]
pub struct VectorArg<T> {
    is_nested_set: bool,
    is_empty: bool,
    count: i32,
    format_each: Option<String>,
    before_each: Option<String>,
    join_with: Option<String>,
    values: Arc<dyn ErasedVectorValues>,
    has_map_each: bool,
    _marker: PhantomData<fn() -> T>,
}

impl<T: VectorArgElement> From<SimpleVectorArg<T>> for VectorArg<T> {
    fn from(arg: SimpleVectorArg<T>) -> VectorArg<T> {
        VectorArg {
            is_nested_set: arg.is_nested_set,
            is_empty: arg.is_empty,
            count: arg.count,
            format_each: arg.format_each,
            before_each: arg.before_each,
            join_with: arg.join_with,
            values: Arc::new(TypedVectorValues {
                values: arg.values,
                map_fn: None,
            }),
            has_map_each: false,
            _marker: PhantomData,
        }
    }
}

impl<T> VectorArg<T> {
    /// The number of collection values, or -1 for a nested set (Java's `count` field).
    pub fn count(&self) -> i32 {
        self.count
    }
}

/// `VectorArg.VectorArgFragment`: the interned marker preceding a vector arg's slots.
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:351 VectorArg.VectorArgFragment
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VectorArgFragment {
    // PORT: `isNestedSet` and `hasMapEach` select how Java reads the following slots; here the
    // typed values slot carries that information, so these only take part in equality.
    #[allow(dead_code)]
    is_nested_set: bool,
    #[allow(dead_code)]
    has_map_each: bool,
    has_format_each: bool,
    has_before_each: bool,
    has_join_with: bool,
}

/// One slot of the flat argument array (Java `Object[] arguments`).
#[derive(Clone)]
enum Obj {
    /// A raw object whose string form is its `CommandLineItem` expansion.
    Arg(CommandLineArg),
    /// `Integer` (FormatArg's argument count).
    Int(i32),
    /// A top-level `NestedSet` (Java `addAll(NestedSet)` and friends).
    NestedSet(Arc<dyn ErasedVectorValues>),
    /// `VectorArgFragment` marker.
    VectorArgFragment(VectorArgFragment),
    /// A vector arg's map fn and values (see the module comment).
    VectorValues(Arc<dyn ErasedVectorValues>),
    /// `FormatArg.INSTANCE` marker.
    FormatArg,
    /// `PrefixArg.INSTANCE` marker.
    PrefixArg,
    /// The main repo mapping slot of a `PrefixArg`.
    RepositoryMapping(RepositoryMapping),
    /// `ExpandedTreeArtifactArg(treeArtifact)`.
    ExpandedTreeArtifactArg(Artifact),
    /// `TreeFileArtifactExecPathArg(placeHolderTreeArtifact)`.
    TreeFileArtifactExecPathArg(Artifact),
}

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Obj::Arg(a) => write!(f, "{:?}", a),
            Obj::Int(i) => write!(f, "{}", i),
            Obj::NestedSet(v) => write!(f, "{:?}", v),
            Obj::VectorArgFragment(v) => write!(f, "{:?}", v),
            Obj::VectorValues(v) => write!(f, "{:?}", v),
            Obj::FormatArg => f.write_str("FormatArg"),
            Obj::PrefixArg => f.write_str("PrefixArg"),
            Obj::RepositoryMapping(m) => write!(f, "{:?}", m),
            Obj::ExpandedTreeArtifactArg(a) => write!(
                f,
                "ExpandedTreeArtifactArg{{ treeArtifact: {}}}",
                a.get_exec_path_string()
            ),
            Obj::TreeFileArtifactExecPathArg(a) => {
                write!(
                    f,
                    "TreeFileArtifactExecPathArg({})",
                    a.get_exec_path_string()
                )
            }
        }
    }
}

impl Obj {
    fn string(s: &str) -> Obj {
        Obj::Arg(CommandLineArg::String(s.to_string()))
    }

    /// Java `instanceof ArgvFragment`.
    fn is_argv_fragment(&self) -> bool {
        matches!(
            self,
            Obj::VectorArgFragment(_)
                | Obj::FormatArg
                | Obj::PrefixArg
                | Obj::ExpandedTreeArtifactArg(_)
        )
    }

    /// `CommandLineItem.expandToCommandLine(object)` for a raw slot.
    fn expand_to_command_line(&self) -> String {
        match self {
            Obj::Arg(a) => a.expand_to_command_line(),
            // Java: Integer.toString.
            Obj::Int(i) => i.to_string(),
            other => panic!(
                "IllegalStateException: unexpected argument slot {:?}",
                other
            ),
        }
    }
}

/// `(String) arguments.get(argi)`.
fn string_at(arguments: &[Obj], argi: usize) -> &str {
    match &arguments[argi] {
        Obj::Arg(CommandLineArg::String(s)) => s,
        other => panic!("ClassCastException: {:?} is not a String", other),
    }
}

/// `(Integer) arguments.get(argi)`.
fn int_at(arguments: &[Obj], argi: usize) -> i32 {
    match &arguments[argi] {
        Obj::Int(i) => *i,
        other => panic!("ClassCastException: {:?} is not an Integer", other),
    }
}

fn vector_values_at(arguments: &[Obj], argi: usize) -> &Arc<dyn ErasedVectorValues> {
    match &arguments[argi] {
        Obj::VectorValues(v) => v,
        other => panic!("ClassCastException: {:?} is not a vector arg value", other),
    }
}

impl VectorArgFragment {
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:391 VectorArgFragment.eval
    fn eval(
        &self,
        arguments: &[Obj],
        mut argi: usize,
        builder: &mut Vec<String>,
        path_mapper: &dyn PathMapper,
    ) -> Result<usize, CommandLineError> {
        let values = vector_values_at(arguments, argi);
        argi += 1;
        let mut mutated_values = values.expand_values(path_mapper)?;
        let count = mutated_values.len();
        if self.has_format_each {
            let format_str = string_at(arguments, argi);
            argi += 1;
            for value in mutated_values.iter_mut().take(count) {
                *value = single_string_arg_formatter::format(format_str, value);
            }
        }
        if self.has_before_each {
            let before_each = string_at(arguments, argi);
            argi += 1;
            for value in mutated_values {
                builder.push(before_each.to_string());
                builder.push(value);
            }
        } else if self.has_join_with {
            let join_with = string_at(arguments, argi);
            argi += 1;
            builder.push(mutated_values.join(join_with));
        } else {
            builder.extend(mutated_values);
        }
        Ok(argi)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:467 VectorArgFragment.addToFingerprint
    fn add_to_fingerprint(
        &self,
        arguments: &[Obj],
        mut argi: usize,
        action_key_context: &ActionKeyContext,
        fingerprint: &mut Fingerprint,
    ) -> Result<usize, CommandLineError> {
        let values = vector_values_at(arguments, argi);
        argi += 1;
        values.add_values_to_fingerprint(action_key_context, fingerprint)?;
        if self.has_format_each {
            add_uuid(fingerprint, FORMAT_EACH_UUID);
            fingerprint.add_string(string_at(arguments, argi));
            argi += 1;
        }
        if self.has_before_each {
            add_uuid(fingerprint, BEFORE_EACH_UUID);
            fingerprint.add_string(string_at(arguments, argi));
            argi += 1;
        } else if self.has_join_with {
            add_uuid(fingerprint, JOIN_WITH_UUID);
            fingerprint.add_string(string_at(arguments, argi));
            argi += 1;
        }
        Ok(argi)
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:307 VectorArg.push
fn push_vector_arg<T>(arguments: &mut Vec<Obj>, vector_arg: &VectorArg<T>) {
    let vector_arg_fragment = VectorArgFragment {
        is_nested_set: vector_arg.is_nested_set,
        has_map_each: vector_arg.has_map_each,
        has_format_each: vector_arg.format_each.is_some(),
        has_before_each: vector_arg.before_each.is_some(),
        has_join_with: vector_arg.join_with.is_some(),
    };
    if vector_arg_fragment.has_before_each && vector_arg_fragment.has_join_with {
        panic!("IllegalArgumentException: Cannot use both 'before' and 'join' in vector arg.");
    }
    arguments.push(Obj::VectorArgFragment(vector_arg_fragment));
    arguments.push(Obj::VectorValues(Arc::clone(&vector_arg.values)));
    if let Some(format_each) = &vector_arg.format_each {
        arguments.push(Obj::string(format_each));
    }
    if let Some(before_each) = &vector_arg.before_each {
        arguments.push(Obj::string(before_each));
    }
    if let Some(join_with) = &vector_arg.join_with {
        arguments.push(Obj::string(join_with));
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:546 FormatArg.eval
fn format_arg_eval(arguments: &[Obj], mut argi: usize, builder: &mut Vec<String>) -> usize {
    let arg_count = int_at(arguments, argi);
    argi += 1;
    let format_str = string_at(arguments, argi);
    argi += 1;
    let mut args: Vec<String> = Vec::with_capacity(arg_count.max(0) as usize);
    for _ in 0..arg_count {
        args.push(arguments[argi].expand_to_command_line());
        argi += 1;
    }
    builder.push(java_string_format(format_str, &args));
    argi
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:562 FormatArg.addToFingerprint
fn format_arg_add_to_fingerprint(
    arguments: &[Obj],
    mut argi: usize,
    fingerprint: &mut Fingerprint,
) -> usize {
    let arg_count = int_at(arguments, argi);
    argi += 1;
    add_uuid(fingerprint, FORMAT_UUID);
    fingerprint.add_string(string_at(arguments, argi));
    argi += 1;
    for _ in 0..arg_count {
        fingerprint.add_string(&arguments[argi].expand_to_command_line());
        argi += 1;
    }
    argi
}

/// Java `String.format(formatStr, args)` where every argument is a `String`.
///
/// PORT: supports the conversions that are meaningful for string arguments without flags, width
/// or precision: `%s`, `%S`, `%%`, `%n` and explicit indices (`%2$s`). Anything else is
/// `unported!`.
fn java_string_format(format_str: &str, args: &[String]) -> String {
    let bytes = format_str.as_bytes();
    let mut out = String::with_capacity(format_str.len());
    let mut ordinary_index = 0usize;
    let mut i = 0usize;
    let mut literal_start = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        out.push_str(&format_str[literal_start..i]);
        let spec_start = i;
        i += 1;
        // Optional explicit argument index: digits followed by '$'.
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let mut explicit_index: Option<usize> = None;
        if i > digits_start && i < bytes.len() && bytes[i] == b'$' {
            explicit_index = Some(format_str[digits_start..i].parse::<usize>().unwrap_or(0));
            i += 1;
        } else if i > digits_start {
            unported!(
                "String.format",
                "width in format specifier {:?}",
                &format_str[spec_start..]
            );
        }
        if i >= bytes.len() {
            panic!("UnknownFormatConversionException: Conversion = '%'");
        }
        let conversion = bytes[i];
        i += 1;
        match conversion {
            b'%' if explicit_index.is_none() => out.push('%'),
            b'n' if explicit_index.is_none() => out.push('\n'),
            b's' | b'S' => {
                let index = match explicit_index {
                    Some(0) => panic!(
                        "IllegalFormatArgumentIndexException: Illegal format argument index = 0"
                    ),
                    Some(n) => n - 1,
                    None => {
                        let n = ordinary_index;
                        ordinary_index += 1;
                        n
                    }
                };
                let arg = match args.get(index) {
                    Some(a) => a,
                    None => panic!(
                        "MissingFormatArgumentException: Format specifier '{}'",
                        &format_str[spec_start..i]
                    ),
                };
                if conversion == b'S' {
                    out.push_str(&arg.to_uppercase());
                } else {
                    out.push_str(arg);
                }
            }
            _ => unported!(
                "String.format",
                "format specifier {:?}",
                &format_str[spec_start..i.min(format_str.len())]
            ),
        }
        literal_start = i;
    }
    out.push_str(&format_str[literal_start..]);
    out
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:584 PrefixArg.push
fn push_prefix_arg(
    arguments: &mut Vec<Obj>,
    before: &str,
    arg: CommandLineArg,
    main_repo_mapping: Option<RepositoryMapping>,
) {
    arguments.push(Obj::PrefixArg);
    arguments.push(Obj::string(before));
    if let Some(m) = main_repo_mapping {
        arguments.push(Obj::RepositoryMapping(m));
    }
    arguments.push(Obj::Arg(arg));
}

/// Shared by `PrefixArg.eval` and `PrefixArg.addToFingerprint`: reads the argument slot(s) and
/// returns the expanded argument.
fn prefix_arg_value(arguments: &[Obj], argi: &mut usize) -> String {
    let arg = &arguments[*argi];
    *argi += 1;
    if let Obj::RepositoryMapping(main_repo_mapping) = arg {
        let label = match &arguments[*argi] {
            Obj::Arg(CommandLineArg::Label(l)) => l,
            other => panic!("ClassCastException: {:?} is not a Label", other),
        };
        *argi += 1;
        return label.get_display_form(Some(main_repo_mapping));
    }
    arg.expand_to_command_line()
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:598 PrefixArg.eval
fn prefix_arg_eval(arguments: &[Obj], mut argi: usize, builder: &mut Vec<String>) -> usize {
    let before = string_at(arguments, argi);
    argi += 1;
    let value = prefix_arg_value(arguments, &mut argi);
    builder.push(format!("{}{}", before, value));
    argi
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:613 PrefixArg.addToFingerprint
fn prefix_arg_add_to_fingerprint(
    arguments: &[Obj],
    mut argi: usize,
    fingerprint: &mut Fingerprint,
) -> usize {
    add_uuid(fingerprint, PREFIX_UUID);
    fingerprint.add_string(string_at(arguments, argi));
    argi += 1;
    let value = prefix_arg_value(arguments, &mut argi);
    fingerprint.add_string(&value);
    argi
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:707 ExpandedTreeArtifactArg.describe
fn expanded_tree_artifact_arg_describe(tree_artifact: &Artifact) -> String {
    format!(
        "ExpandedTreeArtifactArg{{ treeArtifact: {}}}",
        tree_artifact.get_exec_path_string()
    )
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:696 ExpandedTreeArtifactArg.eval
fn expanded_tree_artifact_arg_eval_with(
    tree_artifact: &Artifact,
    builder: &mut Vec<String>,
    input_metadata_provider: &dyn InputMetadataProvider,
) {
    let children = match input_metadata_provider.get_tree_children(tree_artifact) {
        None => return,
        Some(c) => c,
    };
    for child in children {
        builder.push(child.get_exec_path_string().to_string());
    }
}

/// Dispatches `ArgvFragment.eval`.
fn argv_fragment_eval(
    fragment: &Obj,
    arguments: &[Obj],
    argi: usize,
    builder: &mut Vec<String>,
    path_mapper: &dyn PathMapper,
) -> Result<usize, CommandLineError> {
    match fragment {
        Obj::VectorArgFragment(f) => f.eval(arguments, argi, builder, path_mapper),
        Obj::FormatArg => Ok(format_arg_eval(arguments, argi, builder)),
        Obj::PrefixArg => Ok(prefix_arg_eval(arguments, argi, builder)),
        // Java: StandardArgvFragment.eval → TreeArtifactExpansionArgvFragment.eval(builder),
        // which adds `describe()`.
        Obj::ExpandedTreeArtifactArg(tree_artifact) => {
            builder.push(expanded_tree_artifact_arg_describe(tree_artifact));
            Ok(argi)
        }
        other => panic!("IllegalStateException: not an ArgvFragment: {:?}", other),
    }
}

/// Dispatches `ArgvFragment.addToFingerprint`.
fn argv_fragment_add_to_fingerprint(
    fragment: &Obj,
    arguments: &[Obj],
    argi: usize,
    action_key_context: &ActionKeyContext,
    fingerprint: &mut Fingerprint,
) -> Result<usize, CommandLineError> {
    match fragment {
        Obj::VectorArgFragment(f) => {
            f.add_to_fingerprint(arguments, argi, action_key_context, fingerprint)
        }
        Obj::FormatArg => Ok(format_arg_add_to_fingerprint(arguments, argi, fingerprint)),
        Obj::PrefixArg => Ok(prefix_arg_add_to_fingerprint(arguments, argi, fingerprint)),
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:713 ExpandedTreeArtifactArg.addToFingerprint
        Obj::ExpandedTreeArtifactArg(tree_artifact) => {
            add_uuid(fingerprint, TREE_UUID);
            // Java: Fingerprint.addPath(PathFragment) = addString(getPathString()).
            fingerprint.add_string(tree_artifact.get_exec_path().get_path_string());
            Ok(argi)
        }
        other => panic!("IllegalStateException: not an ArgvFragment: {:?}", other),
    }
}

/// A customizable, serializable class for building memory efficient command lines.
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:65 CustomCommandLine
// PORT: the subclass `TreeArtifactSubstitutionCustomCommandLine` is the same struct with a
// substitution map.
#[derive(Clone)]
pub struct CustomCommandLine {
    /// Stored as an array instead of a list to save memory, but is never modified.
    arguments: Arc<[Obj]>,
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1398 TreeArtifactSubstitutionCustomCommandLine.substitutionMap
    substitution_map: Option<Arc<IndexMap<Artifact, Artifact>>>,
}

impl fmt::Debug for CustomCommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomCommandLine")
            .field("arguments", &self.arguments)
            .field("substitution_map", &self.substitution_map)
            .finish()
    }
}

impl CustomCommandLine {
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1254 CustomCommandLine.builder
    pub fn builder() -> CustomCommandLineBuilder {
        CustomCommandLineBuilder {
            arguments: Vec::new(),
        }
    }

    /// Given the list of `TreeFileArtifact`s, returns another CustomCommandLine that replaces
    /// their parent TreeArtifacts with the TreeFileArtifacts in all `TreeFileArtifactArgvFragment`
    /// argument objects.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1279 CustomCommandLine.evaluateTreeFileArtifacts
    pub fn evaluate_tree_file_artifacts(
        &self,
        tree_file_artifacts: impl IntoIterator<Item = Artifact>,
    ) -> CustomCommandLine {
        // Java: Maps.uniqueIndex(treeFileArtifacts, TreeFileArtifact::getParent).
        let mut substitution_map: IndexMap<Artifact, Artifact> = IndexMap::new();
        for tree_file_artifact in tree_file_artifacts {
            let parent = match tree_file_artifact.get_parent() {
                Some(p) => p.clone(),
                None => panic!("NullPointerException: {} has no parent", tree_file_artifact),
            };
            if let Some(existing) = substitution_map.get(&parent) {
                // PORT: Guava's message, abbreviated.
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}={} and {}={}",
                    parent, existing, parent, tree_file_artifact
                );
            }
            substitution_map.insert(parent, tree_file_artifact);
        }
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1400 TreeArtifactSubstitutionCustomCommandLine.<init>
        CustomCommandLine {
            arguments: Arc::clone(&self.arguments),
            substitution_map: Some(Arc::new(substitution_map)),
        }
    }

    /// Returns another argument object that has its enclosing tree artifact substituted by a
    /// `TreeFileArtifact`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1360 CustomCommandLine.substituteTreeFileArtifactArgvFragment
    fn substitute_tree_file_artifact_argv_fragment(&self, argv_fragment: &Obj) -> Obj {
        let place_holder_tree_artifact = match argv_fragment {
            Obj::TreeFileArtifactExecPathArg(a) => a,
            other => panic!("IllegalStateException: Unexpected {:?}", other),
        };
        match &self.substitution_map {
            None => panic!("IllegalStateException: Unexpected {:?}", argv_fragment),
            // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1407 TreeArtifactSubstitutionCustomCommandLine.substituteTreeFileArtifactArgvFragment
            Some(substitution_map) => {
                // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:732 TreeFileArtifactExecPathArg.substituteTreeArtifact
                let artifact = match substitution_map.get(place_holder_tree_artifact) {
                    Some(a) => a,
                    None => panic!(
                        "NullPointerException: Artifact to substitute: {}",
                        place_holder_tree_artifact
                    ),
                };
                Obj::Arg(CommandLineArg::PathFragment(
                    artifact.get_exec_path().clone(),
                ))
            }
        }
    }

    /// Java `arguments(inputMetadataProvider, pathMapper)`.
    ///
    /// `path_mapper` rewrites the config parts of artifact paths to improve caching. This only
    /// affects `add_exec_path` and `add_path` entries. Output paths embedded in larger strings
    /// and added via `add` or other variants must be handled separately.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1297 CustomCommandLine.arguments
    fn arguments_impl(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let mut builder: Vec<String> = Vec::new();
        let arguments: &[Obj] = &self.arguments;
        let count = arguments.len();
        // Track the last scalar, non-path argument (e.g. "--javacopts") so that the PathMapper
        // can heuristically map subsequent argument collections that contain paths.
        let mut previous_flag: Option<String> = None;
        let mut i = 0usize;
        while i < count {
            let raw = &arguments[i];
            i += 1;
            let substituted: Obj;
            let arg: &Obj = if let Obj::TreeFileArtifactExecPathArg(_) = raw {
                substituted = self.substitute_tree_file_artifact_argv_fragment(raw);
                &substituted
            } else {
                raw
            };
            match arg {
                Obj::NestedSet(values) => {
                    values.eval_simple_vector_arg(
                        &mut builder,
                        path_mapper,
                        previous_flag.as_deref(),
                    );
                }
                _ if arg.is_argv_fragment() => match (input_metadata_provider, arg) {
                    (Some(imp), Obj::ExpandedTreeArtifactArg(tree_artifact)) => {
                        expanded_tree_artifact_arg_eval_with(tree_artifact, &mut builder, imp);
                    }
                    _ => {
                        i = argv_fragment_eval(arg, arguments, i, &mut builder, path_mapper)?;
                    }
                },
                Obj::Arg(CommandLineArg::Artifact(action_input)) => {
                    builder.push(path_mapper.get_mapped_exec_path_string(action_input));
                }
                Obj::Arg(CommandLineArg::PathFragment(path_fragment)) => {
                    builder.push(path_mapper.map(path_fragment).get_path_string().to_string());
                }
                _ => builder.push(arg.expand_to_command_line()),
            }
            // Track the last scalar string argument (e.g. "--javacopts") so that the PathMapper
            // can heuristically map subsequent argument collections that contain paths.
            previous_flag = match arg {
                Obj::Arg(CommandLineArg::String(string)) => Some(string.clone()),
                _ => None,
            };
        }
        Ok(builder)
    }
}

impl CommandLine for CustomCommandLine {
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1285 CustomCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.arguments_impl(None, &NOOP)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1297 CustomCommandLine.arguments
    fn arguments_with(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        self.arguments_impl(input_metadata_provider, path_mapper)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1366 CustomCommandLine.addToFingerprint
    fn add_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _effective_output_paths_mode: OutputPathsMode,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        let arguments: &[Obj] = &self.arguments;
        let count = arguments.len();
        let mut i = 0usize;
        while i < count {
            let raw = &arguments[i];
            i += 1;
            let substituted: Obj;
            let arg: &Obj = if let Obj::TreeFileArtifactExecPathArg(_) = raw {
                substituted = self.substitute_tree_file_artifact_argv_fragment(raw);
                &substituted
            } else {
                raw
            };
            match arg {
                Obj::NestedSet(values) => {
                    values.add_simple_to_fingerprint(action_key_context, fingerprint)?;
                }
                _ if arg.is_argv_fragment() => {
                    i = argv_fragment_add_to_fingerprint(
                        arg,
                        arguments,
                        i,
                        action_key_context,
                        fingerprint,
                    )?;
                }
                _ => {
                    fingerprint.add_string(&arg.expand_to_command_line());
                }
            }
        }
        Ok(())
    }
}

/// A Builder class for CustomCommandLine with the appropriate methods.
///
/// Collections passed to `add*` methods will be copied internally. If you have a `NestedSet`,
/// these should never be flattened to a collection before being passed to the command line.
///
/// Try to avoid coercing items to strings unnecessarily. Instead, use a more memory-efficient
/// form that defers the string coercion until the last moment. In particular, avoid flattening
/// lists and nested sets (see [`VectorArg`]).
///
/// Three types are given special consideration:
///
/// * Any labels added will be added using `Label.getCanonicalForm()`
/// * Path fragments will be added using `PathFragment.toString`
/// * Artifacts will be added using `Artifact.getExecPathString()`.
///
/// PORT: Java overloads become distinct names: `(String arg, X value)` overloads take the suffix
/// `_with_arg` (and an `Option` value, `None` adding nothing), `NestedSet` overloads take
/// `_nested_set` and `VectorArg` overloads take `_vector_arg`. Single-value methods whose Java
/// parameter is `@Nullable` take the value directly; a Java null is a no-op, so callers skip the
/// call instead.
// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:761 CustomCommandLine.Builder
#[derive(Clone, Debug)]
pub struct CustomCommandLineBuilder {
    // In order to avoid unnecessary wrapping, we keep raw objects here, but these objects are
    // always either ArgvFragments or objects whose desired string representations are just their
    // toString() results.
    arguments: Vec<Obj>,
}

impl CustomCommandLineBuilder {
    /// Adds a constant-value string.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:773 CustomCommandLine.Builder.add
    pub fn add(&mut self, value: impl Into<String>) -> &mut Self {
        self.add_object_internal(CommandLineArg::String(value.into()))
    }

    /// Adds a string argument to the command line.
    ///
    /// If the value is `None`, neither the arg nor the value is added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:783 CustomCommandLine.Builder.add
    pub fn add_with_arg(&mut self, arg: &str, value: Option<String>) -> &mut Self {
        self.add_object_internal_with_arg(arg, value.map(CommandLineArg::String))
    }

    /// Adds a single argument to the command line, which is lazily converted to string.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:799 CustomCommandLine.Builder.addObject
    pub fn add_object(&mut self, value: CommandLineArg) -> &mut Self {
        self.add_object_internal(value)
    }

    /// Adds a dynamically calculated string.
    ///
    /// Consider whether using another method could be more efficient. For instance, rather than
    /// calling this method with an Artifact's exec path, just add the artifact itself. It will
    /// lazily get converted to its exec path. Same with labels, path fragments, and many other
    /// objects.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:820 CustomCommandLine.Builder.addDynamicString
    pub fn add_dynamic_string(&mut self, value: impl Into<String>) -> &mut Self {
        self.add_object_internal(CommandLineArg::String(value.into()))
    }

    /// Adds a label value by calling `Label.getCanonicalForm`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:831 CustomCommandLine.Builder.addLabel
    pub fn add_label(&mut self, value: Label) -> &mut Self {
        self.add_object_internal(CommandLineArg::Label(value))
    }

    /// Adds a label value by calling `Label.getCanonicalForm`.
    ///
    /// If the value is `None`, neither the arg nor the value is added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:844 CustomCommandLine.Builder.addLabel
    pub fn add_label_with_arg(&mut self, arg: &str, value: Option<Label>) -> &mut Self {
        self.add_object_internal_with_arg(arg, value.map(CommandLineArg::Label))
    }

    /// Adds a path fragment by calling `PathFragment.getPathString`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:855 CustomCommandLine.Builder.addPath
    pub fn add_path(&mut self, value: PathFragment) -> &mut Self {
        self.add_object_internal(CommandLineArg::PathFragment(value))
    }

    /// Adds a path fragment by calling `PathFragment.getPathString`.
    ///
    /// If the value is `None`, neither the arg nor the value is added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:868 CustomCommandLine.Builder.addPath
    pub fn add_path_with_arg(&mut self, arg: &str, value: Option<PathFragment>) -> &mut Self {
        self.add_object_internal_with_arg(arg, value.map(CommandLineArg::PathFragment))
    }

    /// Adds an artifact by calling `Artifact.getExecPath`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:879 CustomCommandLine.Builder.addExecPath
    pub fn add_exec_path(&mut self, value: Artifact) -> &mut Self {
        self.add_object_internal(CommandLineArg::Artifact(value))
    }

    /// Adds an artifact by calling `Artifact.getExecPath`.
    ///
    /// If the value is `None`, neither the arg nor the value is added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:892 CustomCommandLine.Builder.addExecPath
    pub fn add_exec_path_with_arg(&mut self, arg: &str, value: Option<Artifact>) -> &mut Self {
        self.add_object_internal_with_arg(arg, value.map(CommandLineArg::Artifact))
    }

    /// Adds a lazily expanded string.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:898 CustomCommandLine.Builder.addLazyString
    pub fn add_lazy_string(&mut self, value: Arc<dyn OnDemandString>) -> &mut Self {
        self.add_object_internal(CommandLineArg::LazyString(value))
    }

    /// Adds a lazily expanded string.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:904 CustomCommandLine.Builder.addLazyString
    pub fn add_lazy_string_with_arg(
        &mut self,
        arg: &str,
        value: Option<Arc<dyn OnDemandString>>,
    ) -> &mut Self {
        self.add_object_internal_with_arg(arg, value.map(CommandLineArg::LazyString))
    }

    /// Calls `String.format` at command line expansion time.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:911 CustomCommandLine.Builder.addFormatted
    pub fn add_formatted(&mut self, format_str: &str, args: Vec<CommandLineArg>) -> &mut Self {
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:538 FormatArg.push
        self.arguments.push(Obj::FormatArg);
        self.arguments.push(Obj::Int(args.len() as i32));
        self.arguments.push(Obj::string(format_str));
        for a in args {
            self.arguments.push(Obj::Arg(a));
        }
        self
    }

    /// Concatenates the passed prefix string and the string.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:919 CustomCommandLine.Builder.addPrefixed
    pub fn add_prefixed(&mut self, prefix: &str, arg: impl Into<String>) -> &mut Self {
        self.add_prefixed_internal(prefix, CommandLineArg::String(arg.into()), None)
    }

    /// Concatenates the passed prefix string and the label using `Label.getDisplayForm`, which
    /// is identical to `Label.getCanonicalForm()` for main repo labels.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:928 CustomCommandLine.Builder.addPrefixedLabel
    pub fn add_prefixed_label(
        &mut self,
        prefix: &str,
        arg: Label,
        main_repo_mapping: Option<RepositoryMapping>,
    ) -> &mut Self {
        self.add_prefixed_internal(prefix, CommandLineArg::Label(arg), main_repo_mapping)
    }

    /// Concatenates the passed prefix string and the path.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:937 CustomCommandLine.Builder.addPrefixedPath
    pub fn add_prefixed_path(&mut self, prefix: &str, arg: PathFragment) -> &mut Self {
        self.add_prefixed_internal(prefix, CommandLineArg::PathFragment(arg), None)
    }

    /// Concatenates the passed prefix string and the artifact's exec path.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:943 CustomCommandLine.Builder.addPrefixedExecPath
    pub fn add_prefixed_exec_path(&mut self, prefix: &str, arg: Artifact) -> &mut Self {
        self.add_prefixed_internal(prefix, CommandLineArg::Artifact(arg), None)
    }

    /// Adds the passed strings to the command line.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:954 CustomCommandLine.Builder.addAll
    pub fn add_all<I, S>(&mut self, values: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let values: Vec<CommandLineArg> = values
            .into_iter()
            .map(|s| CommandLineArg::String(s.into()))
            .collect();
        self.add_collection_internal(values)
    }

    /// Adds the passed strings to the command line.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:965 CustomCommandLine.Builder.addAll
    pub fn add_all_nested_set(&mut self, values: NestedSet<String>) -> &mut Self {
        self.add_nested_set_internal(values)
    }

    /// Adds the arg followed by the passed strings.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:978 CustomCommandLine.Builder.addAll
    pub fn add_all_with_arg<I, S>(&mut self, arg: &str, values: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let values: Vec<CommandLineArg> = values
            .into_iter()
            .map(|s| CommandLineArg::String(s.into()))
            .collect();
        self.add_collection_internal_with_arg(arg, values)
    }

    /// Adds the arg followed by the passed strings.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:988 CustomCommandLine.Builder.addAll
    pub fn add_all_nested_set_with_arg(
        &mut self,
        arg: &str,
        values: NestedSet<String>,
    ) -> &mut Self {
        self.add_nested_set_internal_with_arg(arg, values)
    }

    /// Adds the passed vector arg. See [`VectorArg`].
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:994 CustomCommandLine.Builder.addAll
    pub fn add_all_vector_arg(&mut self, vector_arg: impl Into<VectorArg<String>>) -> &mut Self {
        self.add_vector_arg_internal(&vector_arg.into())
    }

    /// Adds the arg followed by the passed vector arg. See [`VectorArg`].
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1004 CustomCommandLine.Builder.addAll
    pub fn add_all_vector_arg_with_arg(
        &mut self,
        arg: &str,
        vector_arg: impl Into<VectorArg<String>>,
    ) -> &mut Self {
        self.add_vector_arg_internal_with_arg(arg, &vector_arg.into())
    }

    /// Adds the passed paths to the command line.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1010 CustomCommandLine.Builder.addPaths
    pub fn add_paths(&mut self, values: impl IntoIterator<Item = PathFragment>) -> &mut Self {
        let values: Vec<CommandLineArg> = values
            .into_iter()
            .map(CommandLineArg::PathFragment)
            .collect();
        self.add_collection_internal(values)
    }

    /// Adds the passed paths to the command line.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1016 CustomCommandLine.Builder.addPaths
    pub fn add_paths_nested_set(&mut self, values: NestedSet<PathFragment>) -> &mut Self {
        self.add_nested_set_internal(values)
    }

    /// Adds the arg followed by the path strings.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1026 CustomCommandLine.Builder.addPaths
    pub fn add_paths_with_arg(
        &mut self,
        arg: &str,
        values: impl IntoIterator<Item = PathFragment>,
    ) -> &mut Self {
        let values: Vec<CommandLineArg> = values
            .into_iter()
            .map(CommandLineArg::PathFragment)
            .collect();
        self.add_collection_internal_with_arg(arg, values)
    }

    /// Adds the arg followed by the path fragments.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1037 CustomCommandLine.Builder.addPaths
    pub fn add_paths_nested_set_with_arg(
        &mut self,
        arg: &str,
        values: NestedSet<PathFragment>,
    ) -> &mut Self {
        self.add_nested_set_internal_with_arg(arg, values)
    }

    /// Adds the passed vector arg. See [`VectorArg`].
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1044 CustomCommandLine.Builder.addPaths
    pub fn add_paths_vector_arg(
        &mut self,
        vector_arg: impl Into<VectorArg<PathFragment>>,
    ) -> &mut Self {
        self.add_vector_arg_internal(&vector_arg.into())
    }

    /// Adds the arg followed by the passed vector arg. See [`VectorArg`].
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1054 CustomCommandLine.Builder.addPaths
    pub fn add_paths_vector_arg_with_arg(
        &mut self,
        arg: &str,
        vector_arg: impl Into<VectorArg<PathFragment>>,
    ) -> &mut Self {
        self.add_vector_arg_internal_with_arg(arg, &vector_arg.into())
    }

    /// Adds the artifacts' exec paths to the command line.
    ///
    /// Do not use this method if the list is derived from a flattened nested set. Instead,
    /// figure out how to avoid flattening the set and use `add_exec_paths_nested_set`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1065 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths(&mut self, values: impl IntoIterator<Item = Artifact>) -> &mut Self {
        let values: Vec<CommandLineArg> =
            values.into_iter().map(CommandLineArg::Artifact).collect();
        self.add_collection_internal(values)
    }

    /// Adds the artifacts' exec paths to the command line.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1071 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths_nested_set(&mut self, values: NestedSet<Artifact>) -> &mut Self {
        self.add_nested_set_internal(values)
    }

    /// Adds the arg followed by the artifacts' exec paths.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1084 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths_with_arg(
        &mut self,
        arg: &str,
        values: impl IntoIterator<Item = Artifact>,
    ) -> &mut Self {
        let values: Vec<CommandLineArg> =
            values.into_iter().map(CommandLineArg::Artifact).collect();
        self.add_collection_internal_with_arg(arg, values)
    }

    /// Adds the arg followed by the artifacts' exec paths.
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1095 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths_nested_set_with_arg(
        &mut self,
        arg: &str,
        values: NestedSet<Artifact>,
    ) -> &mut Self {
        self.add_nested_set_internal_with_arg(arg, values)
    }

    /// Adds the passed vector arg. See [`VectorArg`].
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1102 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths_vector_arg(
        &mut self,
        vector_arg: impl Into<VectorArg<Artifact>>,
    ) -> &mut Self {
        self.add_vector_arg_internal(&vector_arg.into())
    }

    /// Adds the arg followed by the passed vector arg. See [`VectorArg`].
    ///
    /// If values is empty, the arg isn't added.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1112 CustomCommandLine.Builder.addExecPaths
    pub fn add_exec_paths_vector_arg_with_arg(
        &mut self,
        arg: &str,
        vector_arg: impl Into<VectorArg<Artifact>>,
    ) -> &mut Self {
        self.add_vector_arg_internal_with_arg(arg, &vector_arg.into())
    }

    /// Adds a placeholder TreeArtifact exec path. When the command line is used in an action
    /// template, the placeholder will be replaced by the exec path of a `TreeFileArtifact` inside
    /// the TreeArtifact at execution time for each expanded action.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1125 CustomCommandLine.Builder.addPlaceholderTreeArtifactExecPath
    pub fn add_placeholder_tree_artifact_exec_path(
        &mut self,
        tree_artifact: Artifact,
    ) -> &mut Self {
        self.arguments
            .push(new_tree_file_artifact_exec_path_arg(tree_artifact));
        self
    }

    /// Adds a flag with the exec path of a placeholder TreeArtifact. When the command line is
    /// used in an action template, the placeholder will be replaced by the exec path of a
    /// `TreeFileArtifact` inside the TreeArtifact at execution time for each expanded action.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1142 CustomCommandLine.Builder.addPlaceholderTreeArtifactExecPath
    pub fn add_placeholder_tree_artifact_exec_path_with_arg(
        &mut self,
        arg: &str,
        tree_artifact: Option<Artifact>,
    ) -> &mut Self {
        if let Some(tree_artifact) = tree_artifact {
            self.arguments.push(Obj::string(arg));
            self.arguments
                .push(new_tree_file_artifact_exec_path_arg(tree_artifact));
        }
        self
    }

    /// Adds the exec paths (one argument per exec path) of all `TreeFileArtifact`s under
    /// `tree_artifact`.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1158 CustomCommandLine.Builder.addExpandedTreeArtifactExecPaths
    pub fn add_expanded_tree_artifact_exec_paths(&mut self, tree_artifact: Artifact) -> &mut Self {
        // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:689 ExpandedTreeArtifactArg.<init>
        check_argument!(
            tree_artifact.is_tree_artifact(),
            "{} is not a TreeArtifact",
            tree_artifact
        );
        self.arguments
            .push(Obj::ExpandedTreeArtifactArg(tree_artifact));
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1164 CustomCommandLine.Builder.build
    pub fn build(&self) -> CustomCommandLine {
        CustomCommandLine {
            arguments: self.arguments.clone().into(),
            substitution_map: None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1169 CustomCommandLine.Builder.addObjectInternal
    fn add_object_internal(&mut self, value: CommandLineArg) -> &mut Self {
        self.arguments.push(Obj::Arg(value));
        self
    }

    /// Adds the arg and the passed value if the value is non-null.
    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1178 CustomCommandLine.Builder.addObjectInternal
    fn add_object_internal_with_arg(
        &mut self,
        arg: &str,
        value: Option<CommandLineArg>,
    ) -> &mut Self {
        if let Some(value) = value {
            self.arguments.push(Obj::string(arg));
            self.add_object_internal(value);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1188 CustomCommandLine.Builder.addPrefixedInternal
    fn add_prefixed_internal(
        &mut self,
        prefix: &str,
        arg: CommandLineArg,
        main_repo_mapping: Option<RepositoryMapping>,
    ) -> &mut Self {
        push_prefix_arg(&mut self.arguments, prefix, arg, main_repo_mapping);
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1198 CustomCommandLine.Builder.addCollectionInternal
    fn add_collection_internal(&mut self, values: Vec<CommandLineArg>) -> &mut Self {
        for v in values {
            self.arguments.push(Obj::Arg(v));
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1206 CustomCommandLine.Builder.addCollectionInternal
    fn add_collection_internal_with_arg(
        &mut self,
        arg: &str,
        values: Vec<CommandLineArg>,
    ) -> &mut Self {
        if !values.is_empty() {
            self.arguments.push(Obj::string(arg));
            self.add_collection_internal(values);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1217 CustomCommandLine.Builder.addNestedSetInternal
    fn add_nested_set_internal<T: VectorArgElement>(&mut self, values: NestedSet<T>) -> &mut Self {
        self.arguments
            .push(Obj::NestedSet(Arc::new(TypedVectorValues {
                values: VectorValues::NestedSet(values),
                map_fn: None,
            })));
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1225 CustomCommandLine.Builder.addNestedSetInternal
    fn add_nested_set_internal_with_arg<T: VectorArgElement>(
        &mut self,
        arg: &str,
        values: NestedSet<T>,
    ) -> &mut Self {
        if !values.is_empty() {
            self.arguments.push(Obj::string(arg));
            self.add_nested_set_internal(values);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1236 CustomCommandLine.Builder.addVectorArgInternal
    fn add_vector_arg_internal<T>(&mut self, vector_arg: &VectorArg<T>) -> &mut Self {
        if !vector_arg.is_empty {
            push_vector_arg(&mut self.arguments, vector_arg);
        }
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:1244 CustomCommandLine.Builder.addVectorArgInternal
    fn add_vector_arg_internal_with_arg<T>(
        &mut self,
        arg: &str,
        vector_arg: &VectorArg<T>,
    ) -> &mut Self {
        if !vector_arg.is_empty {
            self.arguments.push(Obj::string(arg));
            self.add_vector_arg_internal(vector_arg);
        }
        self
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/CustomCommandLine.java:726 TreeFileArtifactExecPathArg.<init>
fn new_tree_file_artifact_exec_path_arg(artifact: Artifact) -> Obj {
    check_argument!(
        artifact.is_tree_artifact(),
        "{} must be a TreeArtifact",
        artifact
    );
    Obj::TreeFileArtifactExecPathArg(artifact)
}
