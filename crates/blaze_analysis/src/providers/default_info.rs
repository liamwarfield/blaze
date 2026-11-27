//! Port of `analysis/DefaultInfo.java` (with `DefaultDefaultInfo`, `DelegatingDefaultInfo` and
//! the `DefaultInfoProvider` constructor).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::analysis::starlark::starlark_file::{StarlarkFile, artifact_from_value};
use blaze_collect::NestedSet;
use blaze_packages::starlark_defs::builtin_providers::{BuiltinProviderId, DEFAULT_INFO};
use blaze_starlark::Location;
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_complex_value;
use starlark::values::{
    Demand, Freeze, FreezeResult, Freezer, FrozenValue, Heap, NoSerialize, OwnedFrozenValue,
    StarlarkValue, Trace, Value, ValueLifetimeless, ValueLike, starlark_value,
};

use crate::info::{InfoProviderKey, freeze_native_info};
use crate::providers::file_provider::{FilesToRunProvider, RunfilesProvider};
use crate::providers::starlark_runfiles::StarlarkRunfiles;
use crate::rule::depsets::{artifacts_to_depset, depset_value};

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// The Rust data of a `DefaultInfo` (Java's two subclasses).
#[derive(Clone, Debug)]
pub enum DefaultInfoData {
    /// Java `DefaultDefaultInfo` (created by the Starlark constructor); `files` is the Starlark
    /// field of `DefaultInfoGen`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:78 DefaultInfo.DefaultDefaultInfo
    Default {
        location: Location,
        runfiles: Option<Runfiles>,
        data_runfiles: Option<Runfiles>,
        default_runfiles: Option<Runfiles>,
        executable: Option<Artifact>,
        files_to_run_provider: Option<Arc<FilesToRunProvider>>,
    },
    /// Java `DelegatingDefaultInfo`: reads a configured target's native providers.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:138 DefaultInfo.DelegatingDefaultInfo
    Delegating {
        files_to_build: NestedSet<Artifact>,
        files_to_run: Option<Arc<FilesToRunProvider>>,
        runfiles_provider: Option<RunfilesProvider>,
    },
}

/// A provider that gives general information about a target's direct and transitive files.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:32 DefaultInfo
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct DefaultInfoGen<V: ValueLifetimeless> {
    /// Java `DefaultDefaultInfo.files`: a depset, or None (Java null).
    files: V,
    #[allocative(skip)]
    #[trace(unsafe_ignore)]
    data: DefaultInfoData,
}

starlark_complex_value!(pub DefaultInfo);

// SAFETY: `repr(C)`; FrozenValue and Value have the same layout and `data` holds no values.
unsafe impl<'v> Coerce<DefaultInfoGen<Value<'v>>> for DefaultInfoGen<FrozenValue> {}

impl<'v> Freeze for DefaultInfo<'v> {
    type Frozen = FrozenDefaultInfo;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenDefaultInfo> {
        Ok(DefaultInfoGen {
            files: freezer.freeze(self.files)?,
            data: self.data,
        })
    }
}

/// Java `DefaultInfo.build(target)`: a frozen `DelegatingDefaultInfo` over a configured
/// target's providers.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:72 DefaultInfo.build
pub fn delegating(
    files_to_build: NestedSet<Artifact>,
    files_to_run: Option<Arc<FilesToRunProvider>>,
    runfiles_provider: Option<RunfilesProvider>,
) -> OwnedFrozenValue {
    freeze_native_info(DefaultInfoGen::<FrozenValue> {
        files: FrozenValue::new_none(),
        data: DefaultInfoData::Delegating {
            files_to_build,
            files_to_run,
            runfiles_provider,
        },
    })
}

/// Java `DefaultInfo.createEmpty(location)`.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:38 DefaultInfo.createEmpty
pub fn create_empty(location: Location) -> OwnedFrozenValue {
    freeze_native_info(DefaultInfoGen::<FrozenValue> {
        files: FrozenValue::new_none(),
        data: DefaultInfoData::Default {
            location,
            runfiles: None,
            data_runfiles: None,
            default_runfiles: None,
            executable: None,
            files_to_run_provider: None,
        },
    })
}

impl<'v, V: ValueLike<'v>> DefaultInfoGen<V> {
    /// The Rust data of this info.
    pub fn data(&self) -> &DefaultInfoData {
        &self.data
    }

    /// Java `getCreationLocation()`: the constructor call's location; `Location.BUILTIN` for the
    /// delegating info of a native target.
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:53 NativeInfo.getCreationLocation
    pub fn get_creation_location(&self) -> Location {
        match &self.data {
            DefaultInfoData::Default { location, .. } => location.clone(),
            DefaultInfoData::Delegating { .. } => Location::builtin(),
        }
    }

    /// Java `getFiles()`: the `files` depset, or `None`.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/DefaultInfoApi.java:53 DefaultInfoApi.getFiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:108 DefaultInfo.DefaultDefaultInfo.getFiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:153 DefaultInfo.DelegatingDefaultInfo.getFiles
    pub fn get_files(&self) -> Option<Value<'v>> {
        match &self.data {
            DefaultInfoData::Default { .. } => {
                let v = self.files.to_value();
                if v.is_none() { None } else { Some(v) }
            }
            DefaultInfoData::Delegating { files_to_build, .. } => {
                // Java: Depset.of(Artifact.class, target.getProvider(FileProvider.class).getFilesToBuild())
                Some(depset_value(&artifacts_to_depset(files_to_build)))
            }
        }
    }

    /// Java `getFilesToRun()`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:59 DefaultInfo.getFilesToRun
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:113 DefaultInfo.DefaultDefaultInfo.getFilesToRun
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:159 DefaultInfo.DelegatingDefaultInfo.getFilesToRun
    pub fn get_files_to_run(&self) -> Option<Arc<FilesToRunProvider>> {
        match &self.data {
            DefaultInfoData::Default {
                files_to_run_provider,
                ..
            } => files_to_run_provider.clone(),
            DefaultInfoData::Delegating { files_to_run, .. } => files_to_run.clone(),
        }
    }

    /// Java `getStatelessRunfiles()`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:49 DefaultInfo.getStatelessRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:118 DefaultInfo.DefaultDefaultInfo.getStatelessRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:179 DefaultInfo.DelegatingDefaultInfo.getStatelessRunfiles
    pub fn get_stateless_runfiles(&self) -> Option<Runfiles> {
        match &self.data {
            DefaultInfoData::Default { runfiles, .. } => runfiles.clone(),
            DefaultInfoData::Delegating { .. } => None,
        }
    }

    /// Java `getDataRunfiles()`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:52 DefaultInfo.getDataRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:123 DefaultInfo.DefaultDefaultInfo.getDataRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:165 DefaultInfo.DelegatingDefaultInfo.getDataRunfiles
    pub fn get_data_runfiles(&self) -> Option<Runfiles> {
        match &self.data {
            DefaultInfoData::Default { data_runfiles, .. } => data_runfiles.clone(),
            DefaultInfoData::Delegating {
                runfiles_provider, ..
            } => Some(match runfiles_provider {
                None => Runfiles::empty(),
                Some(p) => p.get_data_runfiles().clone(),
            }),
        }
    }

    /// Java `getDefaultRunfiles()`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:55 DefaultInfo.getDefaultRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:128 DefaultInfo.DefaultDefaultInfo.getDefaultRunfiles
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:172 DefaultInfo.DelegatingDefaultInfo.getDefaultRunfiles
    pub fn get_default_runfiles(&self) -> Option<Runfiles> {
        match &self.data {
            DefaultInfoData::Default {
                runfiles,
                data_runfiles,
                default_runfiles,
                ..
            } => {
                if data_runfiles.is_none() && default_runfiles.is_none() {
                    runfiles.clone()
                } else {
                    default_runfiles.clone()
                }
            }
            DefaultInfoData::Delegating {
                runfiles_provider, ..
            } => Some(match runfiles_provider {
                None => Runfiles::empty(),
                Some(p) => p.get_default_runfiles().clone(),
            }),
        }
    }

    /// Java `getExecutable()`.
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:57 DefaultInfo.getExecutable
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:139 DefaultInfo.DefaultDefaultInfo.getExecutable
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:184 DefaultInfo.DelegatingDefaultInfo.getExecutable
    pub fn get_executable(&self) -> Option<Artifact> {
        match &self.data {
            DefaultInfoData::Default { executable, .. } => executable.clone(),
            DefaultInfoData::Delegating { files_to_run, .. } => files_to_run
                .as_ref()
                .expect("NullPointerException: FilesToRunProvider")
                .get_executable()
                .cloned(),
        }
    }

    fn runfiles_value<'a>(r: Option<Runfiles>, heap: Heap<'a>) -> Value<'a> {
        match r {
            Some(r) => heap.alloc(StarlarkRunfiles::new(r)),
            None => Value::new_none(),
        }
    }

    /// The Starlark `default_runfiles` field.
    pub fn default_runfiles_value<'a>(&self, heap: Heap<'a>) -> Value<'a> {
        Self::runfiles_value(self.get_default_runfiles(), heap)
    }

    /// The Starlark `data_runfiles` field.
    pub fn data_runfiles_value<'a>(&self, heap: Heap<'a>) -> Value<'a> {
        Self::runfiles_value(self.get_data_runfiles(), heap)
    }

    /// The Starlark `files_to_run` field.
    pub fn files_to_run_value<'a>(&self, heap: Heap<'a>) -> Value<'a> {
        match self.get_files_to_run() {
            Some(p) => heap.alloc(p.as_ref().clone()),
            None => Value::new_none(),
        }
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for DefaultInfoGen<V> {
    // Java: StructImpl.repr: the provider name and the fields in sorted order.
    // Java: com/google/devtools/build/lib/packages/StructImpl.java:269 StructImpl.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let runfiles_repr = |r: Option<Runfiles>| match r {
            Some(_) => {
                "<unknown object com.google.devtools.build.lib.analysis.Runfiles>".to_owned()
            }
            None => "None".to_owned(),
        };
        let files = match self.get_files() {
            Some(v) => v.to_repr(),
            None => "None".to_owned(),
        };
        let files_to_run = match self.get_files_to_run() {
            Some(p) => p.to_string(),
            None => "None".to_owned(),
        };
        write!(
            f,
            "DefaultInfo(data_runfiles = {}, default_runfiles = {}, files = {}, files_to_run = {})",
            runfiles_repr(self.get_data_runfiles()),
            runfiles_repr(self.get_default_runfiles()),
            files,
            files_to_run
        )
    }
}

/// The `structField` methods of `DefaultInfoApi`, in declaration order.
const DEFAULT_INFO_FIELDS: [&str; 4] =
    ["files", "files_to_run", "data_runfiles", "default_runfiles"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/DefaultInfoApi.java:32 DefaultInfoApi
#[starlark_value(type = "DefaultInfo")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for DefaultInfoGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "files" => Some(self.get_files().unwrap_or_else(Value::new_none)),
            "files_to_run" => Some(self.files_to_run_value(heap)),
            "data_runfiles" => Some(self.data_runfiles_value(heap)),
            "default_runfiles" => Some(self.default_runfiles_value(heap)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        DEFAULT_INFO_FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut v: Vec<String> = DEFAULT_INFO_FIELDS.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:44 DefaultInfo.getProvider
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(DEFAULT_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// Unpacks a keyword-only parameter list for a builtin provider constructor (Java
/// `BuiltinFunction` argument processing): rejects positional arguments and unknown names.
pub(crate) fn named_args<'v, const N: usize>(
    function_name: &str,
    names: [&str; N],
    num_positional: usize,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<[Option<Value<'v>>; N]> {
    let mut out: [Option<Value<'v>>; N] = [None; N];
    let positional: Vec<Value<'v>> = args.positions(eval.heap())?.collect();
    if positional.len() > num_positional {
        return Err(errorf(format!(
            "{function_name}() accepts no more than {num_positional} positional argument{} but got {}",
            if num_positional == 1 { "" } else { "s" },
            positional.len()
        )));
    }
    for (i, v) in positional.into_iter().enumerate() {
        out[i] = Some(v);
    }
    for (name, value) in args.names_map()? {
        let Some(i) = names.iter().position(|n| *n == name.as_str()) else {
            return Err(errorf(format!(
                "{function_name}() got unexpected keyword argument '{}'",
                name.as_str()
            )));
        };
        if out[i].is_some() {
            return Err(errorf(format!(
                "{function_name}() got multiple values for argument '{}'",
                name.as_str()
            )));
        }
        out[i] = Some(value);
    }
    Ok(out)
}

/// The `DefaultInfo(...)` constructor.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:206 DefaultInfo.DefaultInfoProvider.constructor
pub fn default_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let [files, runfiles, data_runfiles, default_runfiles, executable] = named_args(
        "DefaultInfo",
        [
            "files",
            "runfiles",
            "data_runfiles",
            "default_runfiles",
            "executable",
        ],
        0,
        args,
        eval,
    )?;
    // Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:230 DefaultInfo.castNoneToNull
    let none_or = |v: Option<Value<'v>>| v.filter(|v| !v.is_none());
    let check_type = |param: &str,
                      v: Option<Value<'v>>,
                      ok: bool,
                      want: &str|
     -> starlark::Result<()> {
        match v {
            Some(v) if !ok => Err(errorf(format!(
                "in call to DefaultInfo(), parameter '{param}' got value of type '{}', want '{want}'",
                v.get_type()
            ))),
            _ => Ok(()),
        }
    };
    let files = none_or(files);
    check_type(
        "files",
        files,
        files.is_some_and(|v| Depset::from_value(v).is_some()),
        "depset or NoneType",
    )?;
    let runfiles_of = |param: &str, v: Option<Value<'v>>| -> starlark::Result<Option<Runfiles>> {
        match none_or(v) {
            None => Ok(None),
            Some(v) => match StarlarkRunfiles::runfiles_of(v) {
                Some(r) => Ok(Some(r)),
                None => Err(errorf(format!(
                    "in call to DefaultInfo(), parameter '{param}' got value of type '{}', want 'runfiles or NoneType'",
                    v.get_type()
                ))),
            },
        }
    };
    let stateless_runfiles = runfiles_of("runfiles", runfiles)?;
    let data_runfiles = runfiles_of("data_runfiles", data_runfiles)?;
    let default_runfiles = runfiles_of("default_runfiles", default_runfiles)?;
    let executable = match none_or(executable) {
        None => None,
        Some(v) => match artifact_from_value(v) {
            Some(a) => Some(a),
            None => {
                return Err(errorf(format!(
                    "in call to DefaultInfo(), parameter 'executable' got value of type '{}', want 'File or NoneType'",
                    v.get_type()
                )));
            }
        },
    };

    if stateless_runfiles.is_some() && (data_runfiles.is_some() || default_runfiles.is_some()) {
        return Err(errorf(
            "Cannot specify the provider 'runfiles' together with 'data_runfiles' or 'default_runfiles'"
                .to_owned(),
        ));
    }

    // Java: thread.getCallerLocation().
    let location = eval
        .call_stack_top_location()
        .map(|l| Location {
            file: l.filename().to_owned(),
            line: l.resolve_span().begin.line + 1,
            column: l.resolve_span().begin.column + 1,
        })
        .unwrap_or_else(Location::builtin);
    Ok(eval.heap().alloc(DefaultInfoGen::<Value<'v>> {
        files: files.unwrap_or_else(Value::new_none),
        data: DefaultInfoData::Default {
            location,
            runfiles: stateless_runfiles,
            data_runfiles,
            default_runfiles,
            executable,
            files_to_run_provider: None,
        },
    }))
}

/// Keeps `StarlarkFile` referenced for the `executable` field's element type.
pub type ExecutableFile = StarlarkFile;
