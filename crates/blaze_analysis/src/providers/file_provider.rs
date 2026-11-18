//! Ports of `analysis/FileProvider.java`, `analysis/FilesToRunProvider.java` and
//! `analysis/RunfilesProvider.java`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles::Runfiles;
use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::structs::DeeplyImmutable;
use blaze_util::check_argument;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::provider_map::TransitiveInfoProvider;
use crate::rule::depsets::{artifacts_to_depset, depset_value};

/// A representation of the concept "this transitive info provider builds these files".
///
/// Every transitive info collection contains at least this provider.
// Java: com/google/devtools/build/lib/analysis/FileProvider.java:28 FileProvider
#[derive(Clone, Debug)]
pub struct FileProvider {
    files_to_build: NestedSet<Artifact>,
}

static EMPTY_FILE_PROVIDER: LazyLock<Arc<FileProvider>> = LazyLock::new(|| {
    Arc::new(FileProvider {
        files_to_build: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
    })
});

impl FileProvider {
    /// Java `FileProvider.EMPTY`.
    pub fn empty() -> Arc<FileProvider> {
        EMPTY_FILE_PROVIDER.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/FileProvider.java:33 FileProvider.of
    pub fn of(files_to_build: NestedSet<Artifact>) -> Arc<FileProvider> {
        if files_to_build.is_empty() {
            FileProvider::empty()
        } else {
            Arc::new(FileProvider { files_to_build })
        }
    }

    /// Returns the set of artifacts that are the "output" of this rule.
    ///
    /// The term "output" is somewhat hazily defined; it is vaguely the set of files that are
    /// passed on to dependent rules that list the rule in their `srcs` attribute and the set of
    /// files that are built when a rule is mentioned on the command line. It does **not** include
    /// the runfiles; that is the bailiwick of `FilesToRunProvider`.
    ///
    /// Note that the above definition is somewhat imprecise; in particular, when a rule is
    /// mentioned on the command line, some other files are also built
    /// `TopLevelArtifactHelper` and dependent rules are free to filter this set of artifacts
    /// e.g. based on their extension.
    ///
    /// Also, some rules may generate artifacts that are not listed here by way of defining other
    /// implicit targets, for example, deploy jars.
    // Java: com/google/devtools/build/lib/analysis/FileProvider.java:79 FileProvider.getFilesToBuild
    pub fn get_files_to_build(&self) -> &NestedSet<Artifact> {
        &self.files_to_build
    }
}

impl TransitiveInfoProvider for FileProvider {
    fn java_class_name(&self) -> &'static str {
        "FileProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Which `FilesToRunProvider` subclass an instance is.
#[derive(Clone, Debug)]
enum FilesToRunKind {
    /// Java `FilesToRunProvider` (no executable, no runfiles support).
    Plain,
    /// Java `SingleExecutableFilesToRunProvider`: the executable is the only file.
    SingleExecutable,
    /// Java `FullFilesToRunProvider`.
    Full {
        runfiles_support: Option<Arc<RunfilesSupport>>,
        executable: Option<Artifact>,
    },
}

/// Returns information about executables produced by a target and the files needed to run it.
// Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:32 FilesToRunProvider
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FilesToRunProvider {
    #[allocative(skip)]
    files_to_run: NestedSet<Artifact>,
    #[allocative(skip)]
    kind: FilesToRunKind,
}

starlark_simple_value!(FilesToRunProvider);

static EMPTY_FILES_TO_RUN: LazyLock<Arc<FilesToRunProvider>> = LazyLock::new(|| {
    Arc::new(FilesToRunProvider {
        files_to_run: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        kind: FilesToRunKind::Plain,
    })
});

impl FilesToRunProvider {
    /// The name of the field in Starlark used to access this class.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:35 FilesToRunProvider.STARLARK_NAME
    pub const STARLARK_NAME: &'static str = "files_to_run";

    /// Java `FilesToRunProvider.EMPTY`.
    pub fn empty() -> Arc<FilesToRunProvider> {
        EMPTY_FILES_TO_RUN.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:40 FilesToRunProvider.create
    pub fn create(
        files_to_run: NestedSet<Artifact>,
        runfiles_support: Option<Arc<RunfilesSupport>>,
        executable: Option<Artifact>,
    ) -> Arc<FilesToRunProvider> {
        if files_to_run.is_empty() {
            check_argument!(
                runfiles_support.is_none(),
                "No files to run with runfiles: {:?}",
                runfiles_support
            );
            check_argument!(
                executable.is_none(),
                "No files to run with executable: {:?}",
                executable
            );
            return FilesToRunProvider::empty();
        }
        if runfiles_support.is_none() && executable.is_none() {
            return Arc::new(FilesToRunProvider {
                files_to_run,
                kind: FilesToRunKind::Plain,
            });
        }
        if files_to_run.is_singleton()
            && runfiles_support.is_none()
            && Some(files_to_run.get_singleton()) == executable.as_ref()
        {
            return Arc::new(FilesToRunProvider {
                files_to_run,
                kind: FilesToRunKind::SingleExecutable,
            });
        }
        Arc::new(FilesToRunProvider {
            files_to_run,
            kind: FilesToRunKind::Full {
                runfiles_support,
                executable,
            },
        })
    }

    /// Returns artifacts needed to run the executable for this target.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:78 FilesToRunProvider.getFilesToRun
    pub fn get_files_to_run(&self) -> &NestedSet<Artifact> {
        &self.files_to_run
    }

    /// Returns the Executable or null if it does not exist.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:84 FilesToRunProvider.getExecutable
    pub fn get_executable(&self) -> Option<&Artifact> {
        match &self.kind {
            FilesToRunKind::Plain => None,
            FilesToRunKind::SingleExecutable => Some(self.files_to_run.get_singleton()),
            FilesToRunKind::Full { executable, .. } => executable.as_ref(),
        }
    }

    /// Returns the RunfilesSupport or null if it does not exist.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:93 FilesToRunProvider.getRunfilesSupport
    pub fn get_runfiles_support(&self) -> Option<&Arc<RunfilesSupport>> {
        match &self.kind {
            FilesToRunKind::Full {
                runfiles_support, ..
            } => runfiles_support.as_ref(),
            _ => None,
        }
    }

    /// Returns the runfiles manifest or null if it does not exist.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:100 FilesToRunProvider.getRunfilesManifest
    pub fn get_runfiles_manifest(&self) -> Option<&Artifact> {
        self.get_runfiles_support()
            .and_then(|r| r.get_runfiles_manifest())
    }

    /// Returns the repo mapping manifest or null if it does not exist.
    // Java: com/google/devtools/build/lib/analysis/FilesToRunProvider.java:107 FilesToRunProvider.getRepoMappingManifest
    pub fn get_repo_mapping_manifest(&self) -> Option<&Artifact> {
        self.get_runfiles_support()
            .and_then(|r| r.get_repo_mapping_manifest())
    }

    fn file_or_none<'v>(a: Option<&Artifact>, heap: Heap<'v>) -> Value<'v> {
        match a {
            Some(a) => heap.alloc(StarlarkFile::new(a.clone())),
            None => Value::new_none(),
        }
    }
}

impl TransitiveInfoProvider for FilesToRunProvider {
    fn java_class_name(&self) -> &'static str {
        "FilesToRunProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl fmt::Display for FilesToRunProvider {
    // Java: StarlarkValue.repr default for FilesToRunProvider (no repr override).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.analysis.FilesToRunProvider>"
        )
    }
}

/// The `structField` methods of `FilesToRunProviderApi`, in declaration order.
const FILES_TO_RUN_FIELDS: [&str; 3] = ["executable", "runfiles_manifest", "repo_mapping_manifest"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/FilesToRunProviderApi.java:24 FilesToRunProviderApi
#[starlark_value(type = "FilesToRunProvider")]
impl<'v> StarlarkValue<'v> for FilesToRunProvider {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/starlarkbuildapi/FilesToRunProviderApi.java:35 FilesToRunProviderApi.getExecutable
            "executable" => Some(Self::file_or_none(self.get_executable(), heap)),
            // Java: com/google/devtools/build/lib/starlarkbuildapi/FilesToRunProviderApi.java:43 FilesToRunProviderApi.getRunfilesManifest
            "runfiles_manifest" => Some(Self::file_or_none(self.get_runfiles_manifest(), heap)),
            // Java: com/google/devtools/build/lib/starlarkbuildapi/FilesToRunProviderApi.java:51 FilesToRunProviderApi.getRepoMappingManifest
            "repo_mapping_manifest" => {
                Some(Self::file_or_none(self.get_repo_mapping_manifest(), heap))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        FILES_TO_RUN_FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut v: Vec<String> = FILES_TO_RUN_FIELDS.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    // Java: FilesToRunProvider.isImmutable (immutable and Starlark-hashable, by identity).
    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::Hash;
        (self as *const Self as usize).hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// `FileProvider` as Starlark sees it (Java `FileProviderApi`, type `file_provider`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/FileProviderApi.java:24 FileProviderApi
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkFileProvider(#[allocative(skip)] pub Arc<FileProvider>);

starlark_simple_value!(StarlarkFileProvider);

impl fmt::Display for StarlarkFileProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.analysis.FileProvider>"
        )
    }
}

#[starlark_value(type = "file_provider")]
impl<'v> StarlarkValue<'v> for StarlarkFileProvider {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/analysis/FileProvider.java:50 FileProvider.getFilesToBuildForStarlark
            "files_to_build" => Some(depset_value(&artifacts_to_depset(
                self.0.get_files_to_build(),
            ))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "files_to_build"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["files_to_build".to_owned()]
    }

    // Java: Object.equals (FileProvider does not override it): the provider's identity.
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(
            starlark::values::ValueLike::downcast_ref::<StarlarkFileProvider>(other)
                .is_some_and(|o| Arc::ptr_eq(&o.0, &self.0)),
        )
    }

    /// Hashable (Java `FileProvider.isImmutable()` is true: "immutable and Starlark-hashable")
    /// by identity (Java `Object.hashCode`): the address of the shared provider.
    // Java: com/google/devtools/build/lib/analysis/FileProvider.java:49 FileProvider.isImmutable
    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::Hash;
        (Arc::as_ptr(&self.0) as usize).hash(hasher);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// Runfiles a target contributes to its dependents.
// Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:27 RunfilesProvider
#[derive(Clone, Debug)]
pub struct RunfilesProvider {
    default_runfiles: Runfiles,
    data_runfiles: Runfiles,
}

/// The mapping functions `RunfilesProvider.DEFAULT_RUNFILES` / `DATA_RUNFILES` (compared by
/// identity in Java, `mapping == RunfilesProvider.DATA_RUNFILES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunfilesMapping {
    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:47 RunfilesProvider.DEFAULT_RUNFILES
    DefaultRunfiles,
    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:63 RunfilesProvider.DATA_RUNFILES
    DataRunfiles,
}

impl RunfilesProvider {
    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:80 RunfilesProvider.simple
    pub fn simple(default_runfiles: Runfiles) -> RunfilesProvider {
        RunfilesProvider {
            data_runfiles: default_runfiles.clone(),
            default_runfiles,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:84 RunfilesProvider.withData
    pub fn with_data(default_runfiles: Runfiles, data_runfiles: Runfiles) -> RunfilesProvider {
        RunfilesProvider {
            default_runfiles,
            data_runfiles,
        }
    }

    /// Java `RunfilesProvider.EMPTY`.
    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:89 RunfilesProvider.EMPTY
    pub fn empty() -> RunfilesProvider {
        RunfilesProvider::with_data(Runfiles::empty(), Runfiles::empty())
    }

    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:37 RunfilesProvider.getDefaultRunfiles
    pub fn get_default_runfiles(&self) -> &Runfiles {
        &self.default_runfiles
    }

    /// Returns something akin to the runfiles of the target when used in the `data` attribute.
    // Java: com/google/devtools/build/lib/analysis/RunfilesProvider.java:41 RunfilesProvider.getDataRunfiles
    pub fn get_data_runfiles(&self) -> &Runfiles {
        &self.data_runfiles
    }
}

impl TransitiveInfoProvider for RunfilesProvider {
    fn java_class_name(&self) -> &'static str {
        "RunfilesProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_provider_of_empty_is_empty_singleton() {
        let p = FileProvider::of(NestedSetBuilder::empty_set(Order::STABLE_ORDER));
        assert!(Arc::ptr_eq(&p, &FileProvider::empty()));
    }

    #[test]
    fn files_to_run_provider_of_empty_is_empty_singleton() {
        let p = FilesToRunProvider::create(
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            None,
            None,
        );
        assert!(Arc::ptr_eq(&p, &FilesToRunProvider::empty()));
        assert!(p.get_executable().is_none());
        assert!(p.get_runfiles_manifest().is_none());
    }
}
