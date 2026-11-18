//! Port of `lib/analysis/starlark/Args.java`: the Starlark `Args` type (`ctx.actions.args()`),
//! which, in a builder-like pattern, encapsulates the data needed to build all or part of a
//! command line. Starlark methods and parameters: `lib/starlarkbuildapi/CommandLineArgsApi.java`.
//!
//! Java has two subclasses: `MutableArgs` (created by `ctx.actions.args()`, mutable until its
//! Mutability is frozen) and `RegisteredArgs` (`Args.forRegisteredAction`, wrapping an already-built
//! command line). Here `MutableArgs<'v>` is the unfrozen value; freezing the rule
//! implementation's module turns it into [`FrozenMutableArgs`] (its values frozen), from which
//! [`build_args`] makes the `StarlarkCustomCommandLine`. [`RegisteredArgs`] is Java's RegisteredArgs.
//!
//! PORT: Java's MutableArgs pushes into a `StarlarkCustomCommandLine.Builder` on each call. The
//! command line must outlive the Starlark heap, and only frozen values can, so `MutableArgs`
//! records the same builder calls (`recordArgStart`, `add`, `addFormatted`, `add(VectorArg)`)
//! with their Starlark values, and [`build_args`] replays them into a
//! [`starlark_custom_command_line::Builder`] once the values are frozen.

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::RepositoryMapping;
use blaze_starlark::depset::Depset;
use blaze_starlark::env::caller_location;
use blaze_starlark::label::LabelValue;
use blaze_starlark::{Location, StarlarkSemantics};
use blaze_util::unported;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::list::ListRef;
use starlark::values::none::NoneOr;
use starlark::values::tuple::TupleRef;
use starlark::values::typing::StarlarkCallable;
use starlark::values::{
    AllocValue, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, OwnedFrozenValue,
    StarlarkValue, Trace, Tracer, UnpackValue, Value, ValueLike, starlark_value,
};

use crate::actions::artifact::Artifact;
use crate::actions::command_line::CommandLineRef;
use crate::actions::param_file_info::ParamFileInfo;
use crate::actions::parameter_file::ParameterFileType;
use crate::actions::single_string_arg_formatter;

use super::starlark_custom_command_line::{self, ArgValue, VectorArgBuilder};
use super::starlark_file::StarlarkFile;

fn err(msg: impl Into<String>) -> starlark::Error {
    let msg: String = msg.into();
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

// ---------------------------------------------------------------------------------------------
// Recorded builder calls

/// The values of one `add_all`/`add_joined` call.
enum VectorValues<V> {
    /// A list, tuple or range, copied at call time (Java pushes the elements immediately).
    List(Vec<V>),
    /// A depset; `element_type` is its element type name (Java `Depset.getElementClass()`).
    Depset {
        depset: V,
        element_type: &'static str,
    },
}

/// One `StarlarkCustomCommandLine.VectorArg.Builder`.
struct VectorItem<V> {
    values: VectorValues<V>,
    location: Location,
    arg_name: Option<String>,
    expand_directories: bool,
    map_each: Option<V>,
    format_each: Option<String>,
    before_each: Option<String>,
    join_with: Option<String>,
    format_joined: Option<String>,
    omit_if_empty: bool,
    uniquify: bool,
    terminate_with: Option<String>,
}

/// One call on Java's `StarlarkCustomCommandLine.Builder`.
enum ArgsItem<V> {
    RecordArgStart,
    /// `add(Object)` of a value.
    Add(V),
    /// `add(Object)` of an arg name.
    AddString(String),
    AddFormatted(V, String),
    AddVector(VectorItem<V>),
}

/// The fields of Java's `MutableArgs`.
struct ArgsState<V> {
    starlark_semantics: Arc<StarlarkSemantics>,
    /// Java `commandLine` (a `StarlarkCustomCommandLine.Builder`), as recorded calls.
    items: Vec<ArgsItem<V>>,
    /// Depsets that may contain directories (Java `potentialDirectoryArtifacts`).
    potential_directory_artifacts: Vec<V>,
    // PORT: java-hash-order. Java keeps a HashSet<Artifact> and returns
    // ImmutableSet.copyOf(it), i.e. Java hash order; insertion order is used here.
    directory_artifacts: IndexSet<Artifact>,
    /// If true, flag names and values will be grouped with '=', e.g.
    ///
    /// ```text
    ///  --a=b
    ///  --noc
    ///  --d=e
    /// ```
    ///
    /// Further, if this is true, the ParamFileInfo will be marked 'flagsOnly', so that positional
    /// parameters stay on the command line and the param file contains only flags.
    flag_per_line: bool,
    /// True if the command line needs to stringify any Labels without an explicit 'map_each'
    /// function.
    may_stringify_external_label: bool,
    // May be set explicitly once -- if unset defaults to ParameterFileType.SHELL_QUOTED.
    parameter_file_type: Option<ParameterFileType>,
    flag_format_string: Option<String>,
    always_use_param_file: bool,
}

impl<V> ArgsState<V> {
    fn new(starlark_semantics: Arc<StarlarkSemantics>) -> ArgsState<V> {
        ArgsState {
            starlark_semantics,
            items: Vec::new(),
            potential_directory_artifacts: Vec::new(),
            directory_artifacts: IndexSet::new(),
            flag_per_line: false,
            may_stringify_external_label: false,
            parameter_file_type: None,
            flag_format_string: None,
            always_use_param_file: false,
        }
    }

    /// Calls `f` on every Starlark value held.
    fn for_each_value_mut(&mut self, f: &mut dyn FnMut(&mut V)) {
        for item in &mut self.items {
            match item {
                ArgsItem::RecordArgStart | ArgsItem::AddString(_) => {}
                ArgsItem::Add(v) => f(v),
                ArgsItem::AddFormatted(v, _) => f(v),
                ArgsItem::AddVector(vector) => {
                    match &mut vector.values {
                        VectorValues::List(values) => {
                            for v in values {
                                f(v);
                            }
                        }
                        VectorValues::Depset { depset, .. } => f(depset),
                    }
                    if let Some(map_each) = &mut vector.map_each {
                        f(map_each);
                    }
                }
            }
        }
        for v in &mut self.potential_directory_artifacts {
            f(v);
        }
    }

    /// Converts every Starlark value held with `f`.
    fn try_map<W, E>(self, f: &mut dyn FnMut(V) -> Result<W, E>) -> Result<ArgsState<W>, E> {
        let mut items = Vec::with_capacity(self.items.len());
        for item in self.items {
            items.push(match item {
                ArgsItem::RecordArgStart => ArgsItem::RecordArgStart,
                ArgsItem::AddString(s) => ArgsItem::AddString(s),
                ArgsItem::Add(v) => ArgsItem::Add(f(v)?),
                ArgsItem::AddFormatted(v, format) => ArgsItem::AddFormatted(f(v)?, format),
                ArgsItem::AddVector(vector) => {
                    let values = match vector.values {
                        VectorValues::List(values) => {
                            let mut out = Vec::with_capacity(values.len());
                            for v in values {
                                out.push(f(v)?);
                            }
                            VectorValues::List(out)
                        }
                        VectorValues::Depset {
                            depset,
                            element_type,
                        } => VectorValues::Depset {
                            depset: f(depset)?,
                            element_type,
                        },
                    };
                    let map_each = match vector.map_each {
                        Some(v) => Some(f(v)?),
                        None => None,
                    };
                    ArgsItem::AddVector(VectorItem {
                        values,
                        location: vector.location,
                        arg_name: vector.arg_name,
                        expand_directories: vector.expand_directories,
                        map_each,
                        format_each: vector.format_each,
                        before_each: vector.before_each,
                        join_with: vector.join_with,
                        format_joined: vector.format_joined,
                        omit_if_empty: vector.omit_if_empty,
                        uniquify: vector.uniquify,
                        terminate_with: vector.terminate_with,
                    })
                }
            });
        }
        let mut potential_directory_artifacts =
            Vec::with_capacity(self.potential_directory_artifacts.len());
        for v in self.potential_directory_artifacts {
            potential_directory_artifacts.push(f(v)?);
        }
        Ok(ArgsState {
            starlark_semantics: self.starlark_semantics,
            items,
            potential_directory_artifacts,
            directory_artifacts: self.directory_artifacts,
            flag_per_line: self.flag_per_line,
            may_stringify_external_label: self.may_stringify_external_label,
            parameter_file_type: self.parameter_file_type,
            flag_format_string: self.flag_format_string,
            always_use_param_file: self.always_use_param_file,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:276 MutableArgs.getParameterFileType
    fn get_parameter_file_type(&self) -> ParameterFileType {
        match &self.parameter_file_type {
            Some(t) => *t,
            None => ParameterFileType::SHELL_QUOTED,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:282 MutableArgs.getParamFileInfo
    fn get_param_file_info(&self) -> Option<ParamFileInfo> {
        let flag_format_string = self.flag_format_string.as_ref()?;
        let mut builder = ParamFileInfo::builder(self.get_parameter_file_type());
        builder
            .set_flag_format_string(flag_format_string.clone())
            .set_use_always(self.always_use_param_file);
        Some(builder.set_flags_only(self.flag_per_line).build())
    }
}

impl<'v, V: ValueLike<'v>> ArgsState<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:614 MutableArgs.getDirectoryArtifacts
    // PORT: Java moves the directories found in `potentialDirectoryArtifacts` into
    // `directoryArtifacts`; the frozen state cannot be mutated, so the union is computed instead
    // (same result).
    fn get_directory_artifacts(&self) -> Vec<Artifact> {
        let mut result = self.directory_artifacts.clone();
        for collection in &self.potential_directory_artifacts {
            let Some(depset) = Depset::from_value(collection.to_value()) else {
                continue;
            };
            for object in depset.to_list().unwrap_or_default() {
                if let Some(file) = object.downcast_ref::<StarlarkFile>() {
                    if file.0.is_directory() {
                        result.insert(file.0.clone());
                    }
                }
            }
        }
        result.into_iter().collect()
    }
}

// SAFETY: `trace` visits every Starlark value the state holds.
unsafe impl<'v> Trace<'v> for ArgsState<Value<'v>> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.for_each_value_mut(&mut |v| v.trace(tracer));
    }
}

// ---------------------------------------------------------------------------------------------
// MutableArgs

/// Java `Args.MutableArgs`: the value of `ctx.actions.args()` while the rule implementation runs.
// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:243 MutableArgs
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct MutableArgs<'v> {
    #[allocative(skip)]
    state: RefCell<ArgsState<Value<'v>>>,
}

/// A frozen `MutableArgs` (its Mutability is frozen): immutable, with frozen values.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenMutableArgs {
    #[allocative(skip)]
    state: ArgsState<FrozenValue>,
}

/// Java `Args.repr`.
// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:68 Args.repr
const ARGS_REPR: &str = "context.args() object";

impl fmt::Debug for MutableArgs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MutableArgs")
    }
}

impl fmt::Display for MutableArgs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ARGS_REPR)
    }
}

impl fmt::Debug for FrozenMutableArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrozenMutableArgs")
    }
}

impl fmt::Display for FrozenMutableArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ARGS_REPR)
    }
}

// SAFETY: `trace` visits every Starlark value of the state.
unsafe impl<'v> Trace<'v> for MutableArgs<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.state.get_mut().trace(tracer);
    }
}

impl<'v> Freeze for MutableArgs<'v> {
    type Frozen = FrozenMutableArgs;

    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenMutableArgs> {
        let state = self.state.into_inner();
        Ok(FrozenMutableArgs {
            state: state.try_map(&mut |v: Value<'v>| freezer.freeze(v))?,
        })
    }
}

impl<'v> AllocValue<'v> for MutableArgs<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

// PORT: Java `Args.debugPrint` (used by `print(args)`) expands the command line; starlark-rust
// has no separate debug-print hook (`print` uses `str`), so `print(args)` shows the repr.
#[starlark_value(type = "Args")]
impl<'v> StarlarkValue<'v> for MutableArgs<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Args", args_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:68 Args.repr
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(ARGS_REPR);
    }
}

#[starlark_value(type = "Args")]
impl<'v> StarlarkValue<'v> for FrozenMutableArgs {
    type Canonical = MutableArgs<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Args", args_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(ARGS_REPR);
    }
}

/// `Args.newArgs(mutability, starlarkSemantics)`: allocates the value returned by
/// `ctx.actions.args()`.
// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:129 Args.newArgs
pub fn new_args<'v>(heap: Heap<'v>, starlark_semantics: Arc<StarlarkSemantics>) -> Value<'v> {
    heap.alloc(MutableArgs::new(starlark_semantics))
}

/// Java `x instanceof Args`.
pub fn is_args(v: Value) -> bool {
    v.downcast_ref::<MutableArgs>().is_some()
        || v.downcast_ref::<FrozenMutableArgs>().is_some()
        || v.downcast_ref::<RegisteredArgs>().is_some()
}

impl<'v> MutableArgs<'v> {
    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:596 MutableArgs.<init>
    pub fn new(starlark_semantics: Arc<StarlarkSemantics>) -> MutableArgs<'v> {
        MutableArgs {
            state: RefCell::new(ArgsState::new(starlark_semantics)),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:276 MutableArgs.getParameterFileType
    pub fn get_parameter_file_type(&self) -> ParameterFileType {
        self.state.borrow().get_parameter_file_type()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:282 MutableArgs.getParamFileInfo
    pub fn get_param_file_info(&self) -> Option<ParamFileInfo> {
        self.state.borrow().get_param_file_info()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:614 MutableArgs.getDirectoryArtifacts
    pub fn get_directory_artifacts(&self) -> Vec<Artifact> {
        self.state.borrow().get_directory_artifacts()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:296 MutableArgs.addArgument
    fn add_argument(
        &self,
        arg_name_or_value: Value<'v>,
        value: Option<Value<'v>>,
        format: Option<&str>,
    ) -> starlark::Result<()> {
        let mut state = self.state.borrow_mut();
        let (value, arg_name) = match value {
            None => (arg_name_or_value, None),
            Some(value) => {
                validate_arg_name(arg_name_or_value)?;
                (
                    value,
                    Some(arg_name_or_value.unpack_str().unwrap().to_owned()),
                )
            }
        };
        state.items.push(ArgsItem::RecordArgStart);
        if let Some(arg_name) = arg_name {
            state.items.push(ArgsItem::AddString(arg_name));
        }
        if Depset::from_value(value).is_some() || is_sequence(value) {
            return Err(err(
                "Args.add() doesn't accept vectorized arguments. Please use Args.add_all() or \
                 Args.add_joined() instead.",
            ));
        }
        if let Some(label) = value.downcast_ref::<LabelValue>() {
            if !label.0.get_repository().is_main() {
                state.may_stringify_external_label = true;
            }
        }
        add_single_arg(&mut state, value, format)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:431 MutableArgs.addVectorArg
    fn add_vector_arg(
        &self,
        value: Value<'v>,
        arg_name: Option<String>,
        map_each: Option<Value<'v>>,
        format_each: Option<String>,
        before_each: Option<String>,
        join_with: Option<String>,
        format_joined: Option<String>,
        omit_if_empty: bool,
        uniquify: bool,
        expand_directories: bool,
        terminate_with: Option<String>,
        loc: Location,
        heap: Heap<'v>,
    ) -> starlark::Result<()> {
        validate_format_string("format_each", format_each.as_deref())?;
        validate_format_string("format_joined", format_joined.as_deref())?;
        let mut state = self.state.borrow_mut();
        let values = if let Some(starlark_nested_set) = Depset::from_value(value) {
            let element_type = starlark_nested_set.element_type();
            if map_each.is_none() && element_type == "Label" {
                // We don't want to eagerly check whether all labels reference targets in the main
                // repo, so just assume they might not. Nested sets of labels should be rare.
                state.may_stringify_external_label = true;
            }
            if starlark_nested_set.is_empty() && omit_if_empty {
                return Ok(());
            }
            if expand_directories {
                state.potential_directory_artifacts.push(value);
            }
            VectorValues::Depset {
                depset: value,
                element_type,
            }
        } else {
            let starlark_list = sequence_elements(value, heap)?;
            if starlark_list.is_empty() && omit_if_empty {
                return Ok(());
            }
            for object in &starlark_list {
                if expand_directories {
                    if let Some(file) = object.downcast_ref::<StarlarkFile>() {
                        if file.0.is_directory() {
                            state.directory_artifacts.insert(file.0.clone());
                        }
                    }
                }
                // Labels referencing targets in the main repo are stringified as //pkg:name and
                // thus don't require a RepositoryMapping. If a map_each function is provided,
                // default stringification via Label#toString() is not used.
                if map_each.is_none() {
                    if let Some(label) = object.downcast_ref::<LabelValue>() {
                        if !label.0.get_repository().is_main() {
                            state.may_stringify_external_label = true;
                        }
                    }
                }
            }
            VectorValues::List(starlark_list)
        };
        // Java: commandLine.recordArgStart(); commandLine.add(vectorArg)
        state.items.push(ArgsItem::RecordArgStart);
        state.items.push(ArgsItem::AddVector(VectorItem {
            values,
            location: loc,
            arg_name,
            expand_directories,
            map_each,
            format_each,
            before_each,
            join_with,
            format_joined,
            omit_if_empty,
            uniquify,
            terminate_with,
        }));
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:550 MutableArgs.useParamsFile
    fn use_params_file(&self, param_file_arg: &str, use_always: bool) -> starlark::Result<()> {
        if !single_string_arg_formatter::is_valid(param_file_arg) {
            return Err(err(format!(
                "Invalid value for parameter \"param_file_arg\": Expected string with a single \
                 \"%s\", got \"{param_file_arg}\""
            )));
        }
        let mut state = self.state.borrow_mut();
        state.flag_format_string = Some(param_file_arg.to_owned());
        state.always_use_param_file = use_always;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:566 MutableArgs.setParamFileFormat
    fn set_param_file_format(&self, format: &str) -> starlark::Result<()> {
        let mut state = self.state.borrow_mut();
        if state.parameter_file_type.is_some() {
            return Err(err("set_param_file_format() may only be called once"));
        }
        let (parameter_file_type, flag_per_line) = match format {
            "shell" => (ParameterFileType::SHELL_QUOTED, false),
            "multiline" => (ParameterFileType::UNQUOTED, false),
            "flag_per_line" => (ParameterFileType::UNQUOTED, true),
            _ => {
                return Err(err(
                    "Invalid value for parameter \"format\": Expected one of \"shell\", \
                     \"multiline\", \"flag_per_line\"",
                ));
            }
        };
        state.parameter_file_type = Some(parameter_file_type);
        state.flag_per_line = flag_per_line;
        Ok(())
    }
}

impl FrozenMutableArgs {
    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:276 MutableArgs.getParameterFileType
    pub fn get_parameter_file_type(&self) -> ParameterFileType {
        self.state.get_parameter_file_type()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:282 MutableArgs.getParamFileInfo
    pub fn get_param_file_info(&self) -> Option<ParamFileInfo> {
        self.state.get_param_file_info()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:614 MutableArgs.getDirectoryArtifacts
    pub fn get_directory_artifacts(&self) -> Vec<Artifact> {
        self.state.get_directory_artifacts()
    }

    /// Builds the command line. `owner` is an `OwnedFrozenValue` that keeps this value's frozen
    /// heap alive (for example the frozen Args value itself, or the frozen module holding it).
    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:602 MutableArgs.build
    pub fn build(
        &self,
        owner: &OwnedFrozenValue,
        main_repo_mapping_supplier: &mut dyn FnMut() -> RepositoryMapping,
    ) -> CommandLineRef {
        let state = &self.state;
        let to_arg_value = |v: FrozenValue| ArgValue::from_frozen_value(owner, v.to_value());
        let mut builder =
            starlark_custom_command_line::Builder::new(state.starlark_semantics.clone());
        for item in &state.items {
            match item {
                ArgsItem::RecordArgStart => {
                    builder.record_arg_start();
                }
                ArgsItem::Add(v) => {
                    builder.add(to_arg_value(*v));
                }
                ArgsItem::AddString(s) => {
                    builder.add(s.clone());
                }
                ArgsItem::AddFormatted(v, format) => {
                    builder.add_formatted(to_arg_value(*v), format.clone());
                }
                ArgsItem::AddVector(vector) => {
                    let vector_arg = match &vector.values {
                        VectorValues::List(values) => VectorArgBuilder::from_list(
                            values.iter().map(|v| to_arg_value(*v)).collect(),
                        ),
                        VectorValues::Depset {
                            depset,
                            element_type,
                        } => {
                            let d = Depset::from_value(depset.to_value()).expect("depset");
                            let nested_set = d
                                .to_nested_set(&mut |v: Value| -> starlark::Result<ArgValue> {
                                    Ok(ArgValue::from_frozen_value(owner, v))
                                })
                                .expect("depset elements are hashable");
                            VectorArgBuilder::from_nested_set(nested_set, element_type)
                        }
                    };
                    let vector_arg = vector_arg
                        .set_location(vector.location.clone())
                        .set_arg_name_opt(vector.arg_name.clone())
                        .set_expand_directories(vector.expand_directories)
                        .set_format_each_opt(vector.format_each.clone())
                        .set_before_each_opt(vector.before_each.clone())
                        .set_join_with_opt(vector.join_with.clone())
                        .set_format_joined_opt(vector.format_joined.clone())
                        .omit_if_empty(vector.omit_if_empty)
                        .uniquify(vector.uniquify)
                        .set_terminate_with_opt(vector.terminate_with.clone())
                        .set_map_each_opt(vector.map_each.map(|f| owner.map(|_| f)));
                    builder.add_vector_arg(vector_arg);
                }
            }
        }
        let main_repo_mapping = if state.may_stringify_external_label {
            Some(main_repo_mapping_supplier())
        } else {
            None
        };
        builder.build(state.flag_per_line, main_repo_mapping)
    }
}

// ---------------------------------------------------------------------------------------------
// RegisteredArgs

/// A frozen (immutable) representation of `Args`, constructed from an already-built command
/// line.
// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:138 RegisteredArgs
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct RegisteredArgs {
    #[allocative(skip)]
    command_line: CommandLineRef,
    #[allocative(skip)]
    param_file_info: Option<ParamFileInfo>,
    #[allocative(skip)]
    directory_inputs: Vec<Artifact>,
}

starlark::starlark_simple_value!(RegisteredArgs);

impl fmt::Debug for RegisteredArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenArgs")
            .field("command_line", &self.command_line)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RegisteredArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ARGS_REPR)
    }
}

#[starlark_value(type = "Args")]
impl<'v> StarlarkValue<'v> for RegisteredArgs {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Args", args_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(ARGS_REPR);
    }
}

impl RegisteredArgs {
    /// Returns a frozen `Args` representation corresponding to an already-registered action.
    /// `directory_inputs` are all directory artifacts of the action.
    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:119 Args.forRegisteredAction
    pub fn for_registered_action(
        command_line: CommandLineRef,
        param_file_info: Option<ParamFileInfo>,
        directory_inputs: Vec<Artifact>,
    ) -> RegisteredArgs {
        RegisteredArgs {
            command_line,
            param_file_info,
            directory_inputs,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:159 RegisteredArgs.getDirectoryArtifacts
    pub fn get_directory_artifacts(&self) -> Vec<Artifact> {
        self.directory_inputs.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:164 RegisteredArgs.build
    pub fn build(&self) -> CommandLineRef {
        self.command_line.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:169 RegisteredArgs.getParameterFileType
    pub fn get_parameter_file_type(&self) -> ParameterFileType {
        match &self.param_file_info {
            Some(info) => info.get_file_type(),
            None => ParameterFileType::SHELL_QUOTED,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:179 RegisteredArgs.getParamFileInfo
    pub fn get_param_file_info(&self) -> Option<ParamFileInfo> {
        self.param_file_info.clone()
    }
}

// ---------------------------------------------------------------------------------------------
// Building

/// What an action needs from an `Args` object (Java `Args.build`, `getParamFileInfo`,
/// `getParameterFileType`, `getDirectoryArtifacts`).
pub struct BuiltArgs {
    pub command_line: CommandLineRef,
    pub param_file_info: Option<ParamFileInfo>,
    pub parameter_file_type: ParameterFileType,
    pub directory_artifacts: Vec<Artifact>,
}

impl fmt::Debug for BuiltArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuiltArgs")
            .field("command_line", &self.command_line)
            .field("directory_artifacts", &self.directory_artifacts)
            .finish_non_exhaustive()
    }
}

/// Builds the command line of a frozen `Args` value (`args.value()` must be a frozen
/// `ctx.actions.args()` object, or an `Args` made by [`RegisteredArgs::for_registered_action`]).
/// `main_repo_mapping_supplier` is called only if a label may need an apparent repository name.
/// Returns `Err` if the value is not a frozen Args.
// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:108 Args.build
pub fn build_args(
    args: &OwnedFrozenValue,
    main_repo_mapping_supplier: &mut dyn FnMut() -> RepositoryMapping,
) -> Result<BuiltArgs, String> {
    let v = args.value();
    if let Some(a) = v.downcast_ref::<FrozenMutableArgs>() {
        return Ok(BuiltArgs {
            command_line: a.build(args, main_repo_mapping_supplier),
            param_file_info: a.get_param_file_info(),
            parameter_file_type: a.get_parameter_file_type(),
            directory_artifacts: a.get_directory_artifacts(),
        });
    }
    if let Some(a) = v.downcast_ref::<RegisteredArgs>() {
        return Ok(BuiltArgs {
            command_line: a.build(),
            param_file_info: a.get_param_file_info(),
            parameter_file_type: a.get_parameter_file_type(),
            directory_artifacts: a.get_directory_artifacts(),
        });
    }
    if v.downcast_ref::<MutableArgs>().is_some() {
        return Err("Args value is not frozen yet".to_owned());
    }
    Err(format!("expected an Args value, got '{}'", v.get_type()))
}

// ---------------------------------------------------------------------------------------------
// Validation helpers

/// Java `value instanceof Sequence`.
fn is_sequence(v: Value) -> bool {
    matches!(v.get_type(), "list" | "tuple" | "range")
}

/// The elements of a list, tuple or range.
fn sequence_elements<'v>(v: Value<'v>, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Ok(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Ok(t.content().to_vec());
    }
    Ok(v.iterate(heap)?.collect())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:498 MutableArgs.validateArgName
fn validate_arg_name(arg_name: Value) -> starlark::Result<()> {
    if arg_name.unpack_str().is_none() {
        return Err(err(format!(
            "expected value of type 'string' for arg name, got '{}'",
            arg_name.get_type()
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:505 MutableArgs.validateValues
fn validate_values(values: Value) -> starlark::Result<()> {
    if !(is_sequence(values) || Depset::from_value(values).is_some()) {
        return Err(err(format!(
            "expected value of type 'sequence or depset' for values, got '{}'",
            values.get_type()
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:513 MutableArgs.validateFormatString
fn validate_format_string(argument_name: &str, format_str: Option<&str>) -> starlark::Result<()> {
    if let Some(format_str) = format_str {
        if !single_string_arg_formatter::is_valid(format_str) {
            return Err(err(format!(
                "Invalid value for parameter \"{argument_name}\": Expected string with a single \
                 \"%s\""
            )));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:522 MutableArgs.addSingleArg
fn add_single_arg<'v>(
    state: &mut ArgsState<Value<'v>>,
    value: Value<'v>,
    format: Option<&str>,
) -> starlark::Result<()> {
    validate_no_directory(value)?;
    validate_format_string("format", format)?;
    match format {
        None => state.items.push(ArgsItem::Add(value)),
        Some(format) => state
            .items
            .push(ArgsItem::AddFormatted(value, format.to_owned())),
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:535 MutableArgs.validateNoDirectory
fn validate_no_directory(value: Value) -> starlark::Result<()> {
    if is_directory(value) {
        return Err(err(
            "Cannot add directories to Args#add since they may expand to multiple values. Either \
             use Args#add_all (if you want expansion) or args.add(directory.path) (if you do \
             not).",
        ));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:544 MutableArgs.isDirectory
fn is_directory(object: Value) -> bool {
    object
        .downcast_ref::<StarlarkFile>()
        .is_some_and(|f| f.0.is_directory())
}

/// Java `StarlarkFunction.isGlobal()`: `module.getGlobal(getName()) == this`.
///
/// PORT: starlark-rust does not expose a function's defining module. A function value that is
/// not frozen was defined by the module being evaluated, so it is looked up there (with
/// `Module::get`, which only sees public names: an unfrozen `_private` top-level function counts
/// as not global; during analysis every unfrozen function was created inside a function call,
/// so it is not global in Java either). A frozen function comes from an already loaded (frozen)
/// module, as rule implementation helpers do; it is treated as global unless it is a lambda. This
/// misclassifies frozen nested `def`s returned by a top-level call, which Java rejects.
pub(crate) fn is_global_function<'v>(f: Value<'v>, eval: &Evaluator<'v, '_, '_>) -> bool {
    let Some(name) = f.function_name() else {
        return false;
    };
    if f.unpack_frozen().is_some() {
        return name != "lambda";
    }
    eval.module().get(name).is_some_and(|g| g.ptr_eq(f))
}

/// Java `fn instanceof StarlarkFunction`.
pub(crate) fn is_starlark_function(f: Value) -> bool {
    f.parameters_spec().is_some()
}

// Java: com/google/devtools/build/lib/analysis/starlark/Args.java:366 MutableArgs.validateMapEach
fn validate_map_each<'v>(
    f: Option<Value<'v>>,
    allow_closure: bool,
    eval: &Evaluator<'v, '_, '_>,
) -> starlark::Result<Option<Value<'v>>> {
    let Some(f) = f else {
        return Ok(None);
    };
    if is_starlark_function(f) {
        // Reject non-global functions, because arbitrary closures may cause large
        // analysis-phase data structures to remain live into the execution phase.
        // We require that the function is "global" as opposed to "not a closure"
        // because a global function may be closure if it refers to load bindings.
        // This unfortunately disallows such trivially safe non-global
        // functions as "lambda x: x".
        // See https://github.com/bazelbuild/bazel/issues/12701.
        if !(is_global_function(f, eval) || allow_closure) {
            // Java: "to avoid unintended retention of analysis data structures, the map_each
            // function (declared at %s) must be declared by a top-level def statement", with
            // the function's definition location, which starlark-rust does not expose.
            unported!(
                "StarlarkFunction.getLocation",
                "map_each function '{}' is not global; its error needs the def location",
                f.function_name().unwrap_or("?")
            );
        }
    }
    Ok(Some(f))
}

/// The `map_each` parameter: a callable or None (Java `@ParamType`s StarlarkCallable, NoneType).
fn map_each_param<'v>(
    method: &str,
    map_each: NoneOr<Value<'v>>,
) -> starlark::Result<Option<Value<'v>>> {
    match map_each {
        NoneOr::None => Ok(None),
        NoneOr::Other(f) => {
            if <StarlarkCallable<'v>>::unpack_value_opt(f).is_none() {
                return Err(err(format!(
                    "in call to {method}(), parameter 'map_each' got value of type '{}', want \
                     'callable or NoneType'",
                    f.get_type()
                )));
            }
            Ok(Some(f))
        }
    }
}

/// The `values` parameter: a sequence or depset (Java `@ParamType`s Sequence, Depset).
fn values_param(method: &str, values: Option<Value>) -> starlark::Result<()> {
    if let Some(v) = values {
        if !(is_sequence(v) || Depset::from_value(v).is_some()) {
            return Err(err(format!(
                "in call to {method}(), parameter 'values' got value of type '{}', want \
                 'sequence or depset'",
                v.get_type()
            )));
        }
    }
    Ok(())
}

/// `Starlark.checkMutable(this)`, or RegisteredArgs' "cannot modify frozen value".
fn mutable_args<'v>(this: Value<'v>) -> starlark::Result<&'v MutableArgs<'v>> {
    if let Some(args) = this.downcast_ref::<MutableArgs<'v>>() {
        return Ok(args);
    }
    if this.downcast_ref::<RegisteredArgs>().is_some() {
        // Java: com/google/devtools/build/lib/analysis/starlark/Args.java:184 RegisteredArgs.addArgument
        return Err(err("cannot modify frozen value"));
    }
    // Java: net/starlark/java/eval/Starlark.java:262 Starlark.checkMutable
    Err(err("trying to mutate a frozen Args value"))
}

fn caller(eval: &Evaluator) -> Location {
    caller_location(eval).unwrap_or_else(Location::builtin)
}

#[starlark_module]
fn args_methods(builder: &mut MethodsBuilder) {
    /// Java: CommandLineArgsApi.addArgument (Args.java:296)
    fn add<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] arg_name_or_value: Value<'v>,
        #[starlark(require = pos)] value: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] format: NoneOr<&str>,
    ) -> starlark::Result<Value<'v>> {
        let args = mutable_args(this)?;
        args.add_argument(arg_name_or_value, value, format.into_option())?;
        Ok(this)
    }

    /// Java: CommandLineArgsApi.addAll (Args.java:326)
    fn add_all<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] arg_name_or_values: Value<'v>,
        #[starlark(require = pos)] values: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] map_each: NoneOr<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] format_each: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] before_each: NoneOr<&str>,
        #[starlark(require = named, default = true)] omit_if_empty: bool,
        #[starlark(require = named, default = false)] uniquify: bool,
        #[starlark(require = named, default = true)] expand_directories: bool,
        #[starlark(require = named, default = NoneOr::None)] terminate_with: NoneOr<&str>,
        #[starlark(require = named, default = false)] allow_closure: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        values_param("add_all", values)?;
        let map_each = map_each_param("add_all", map_each)?;
        let args = mutable_args(this)?;
        let (values, arg_name) = match values {
            None => {
                validate_values(arg_name_or_values)?;
                (arg_name_or_values, None)
            }
            Some(values) => {
                validate_arg_name(arg_name_or_values)?;
                (
                    values,
                    Some(arg_name_or_values.unpack_str().unwrap().to_owned()),
                )
            }
        };
        let map_each = validate_map_each(map_each, allow_closure, eval)?;
        args.add_vector_arg(
            values,
            arg_name,
            map_each,
            format_each.into_option().map(str::to_owned),
            before_each.into_option().map(str::to_owned),
            /* join_with= */ None,
            /* format_joined= */ None,
            omit_if_empty,
            uniquify,
            expand_directories,
            terminate_with.into_option().map(str::to_owned),
            caller(eval),
            eval.heap(),
        )?;
        Ok(this)
    }

    /// Java: CommandLineArgsApi.addJoined (Args.java:392)
    fn add_joined<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] arg_name_or_values: Value<'v>,
        #[starlark(require = pos)] values: Option<Value<'v>>,
        #[starlark(require = named)] join_with: &str,
        #[starlark(require = named, default = NoneOr::None)] map_each: NoneOr<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] format_each: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] format_joined: NoneOr<&str>,
        #[starlark(require = named, default = true)] omit_if_empty: bool,
        #[starlark(require = named, default = false)] uniquify: bool,
        #[starlark(require = named, default = true)] expand_directories: bool,
        #[starlark(require = named, default = false)] allow_closure: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        values_param("add_joined", values)?;
        let map_each = map_each_param("add_joined", map_each)?;
        let args = mutable_args(this)?;
        let (values, arg_name) = match values {
            None => {
                validate_values(arg_name_or_values)?;
                (arg_name_or_values, None)
            }
            Some(values) => {
                validate_arg_name(arg_name_or_values)?;
                (
                    values,
                    Some(arg_name_or_values.unpack_str().unwrap().to_owned()),
                )
            }
        };
        let map_each = validate_map_each(map_each, allow_closure, eval)?;
        args.add_vector_arg(
            values,
            arg_name,
            map_each,
            format_each.into_option().map(str::to_owned),
            /* before_each= */ None,
            Some(join_with.to_owned()),
            format_joined.into_option().map(str::to_owned),
            omit_if_empty,
            uniquify,
            expand_directories,
            /* terminate_with= */ None,
            caller(eval),
            eval.heap(),
        )?;
        Ok(this)
    }

    /// Java: CommandLineArgsApi.useParamsFile (Args.java:550)
    fn use_param_file<'v>(
        this: Value<'v>,
        param_file_arg: &str,
        #[starlark(require = named, default = false)] use_always: bool,
    ) -> starlark::Result<Value<'v>> {
        let args = mutable_args(this)?;
        args.use_params_file(param_file_arg, use_always)?;
        Ok(this)
    }

    /// Java: CommandLineArgsApi.setParamFileFormat (Args.java:566)
    fn set_param_file_format<'v>(this: Value<'v>, format: &str) -> starlark::Result<Value<'v>> {
        let args = mutable_args(this)?;
        args.set_param_file_format(format)?;
        Ok(this)
    }
}
