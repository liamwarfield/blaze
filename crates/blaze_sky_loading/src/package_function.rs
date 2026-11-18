//! PACKAGE: loading a package from its BUILD file.
//!
//! Java: com/google/devtools/build/lib/skyframe/{PackageFunction,
//! PackageFunctionWithSingleGlobsDep,PackageValue}.java (Bazel's default globbing strategy,
//! `SINGLE_GLOBS_HYBRID`), and the `CachingPackageLocator` part of `SkyframePackageManager`.
//!
//! PORT: lazy macro expansion (package pieces), BzlLoadFunction inlining, the package progress
//! receiver, metrics and the cpu-bound semaphore are not ported. Blaze's evaluator is blocking,
//! so there are no Skyframe restarts and no `SkyKeyComputeState`.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::label_constants::EXTERNAL_PACKAGE_IDENTIFIER;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, LabelRepoContextImpl, PackageIdentifier, RepositoryMapping};
use blaze_events::{Event, EventHandler, ExtendedEventHandler, StoredEventHandler};
use blaze_packages::build::glob_cache::{CachingPackageLocator, IgnoredSubdirectoriesMatcher};
use blaze_packages::build::glob_cache::{NonSkyframeGlobber, bad_glob_exception_all_excluded};
use blaze_packages::build::package_factory::{DebugPrintHandler, PackageFactory};
use blaze_packages::builtins::BazelStarlarkEnvironment;
use blaze_packages::builtins::env::{EnvEntry, StarlarkEnv, build_globals};
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_packages::exceptions::{
    DetailedExitCode, NoSuchPackageException, NoSuchPackageKind, PackageLoadingCode,
};
use blaze_packages::package::{
    BadGlobException, ConfigSettingVisibilityPolicy, Globber, GlobberError, GlobberOperation,
    GlobberToken, Package, PackageArgs, RuleVisibility, TargetDefinitionContext,
};
use blaze_skyframe::{
    ComputeError, Environment, Interrupted, Key, SkyFunctionException, SkyFunctionName, SkyKey,
    Transience,
};
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{CompiledFile, EvalOptions, compile};
use blaze_starlark::loader::{LoadMap, LoadRequest};
use blaze_starlark::{FileKind, Location, Phase, StarlarkSemantics, SyntaxError, ThreadContext};
use blaze_util::Fingerprint;
use blaze_vfs::{Path, PathFragment, Root, RootedPath};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, RwLock};
use starlark::environment::{FrozenModule, Globals};

use crate::bzl_load::{
    BzlLoadFailedException, BzlLoadKey, BzlLoadValue, check_load_visibilities, get_load_labels,
    is_package_under_experimental,
};
use crate::file::FileKey;
use crate::globs::{GlobRequest, GlobsKey};
use crate::ignored_subdirectories::{
    IgnoredSubdirectories, IgnoredSubdirectoriesKey, InvalidIgnorePathException,
};
use crate::io::IOException;
use crate::package_lookup::{
    ErrorReason, PackageLookupFunction, PackageLookupKey, PackageLookupValue,
};
use crate::pkgcache::PathPackageLocator;
use crate::precomputed::{
    AUTOLOAD_SYMBOLS, CONFIG_SETTING_VISIBILITY_POLICY, DEFAULT_VISIBILITY,
    LAZY_MACRO_EXPANSION_PACKAGES,
};
use crate::repo_file::{BadRepoFileException, RepoPackageArgsKey, RepoPackageArgsValue};
use crate::repos::RepositoryMappingKey;
use crate::sky_functions;
use crate::starlark_builtins::{
    BuiltinsFailedException, StarlarkBuiltinsKey, StarlarkBuiltinsValue,
};
use crate::starlark_util;

/// The key of PACKAGE: the package identifier (Java uses `PackageIdentifier` itself as the key;
/// `Debug` prints its `toString()`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageKey(pub PackageIdentifier);

impl PackageKey {
    pub fn key(package_identifier: PackageIdentifier) -> PackageKey {
        PackageKey(package_identifier)
    }

    pub fn argument(&self) -> &PackageIdentifier {
        &self.0
    }
}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:267 PackageIdentifier.toString
impl fmt::Debug for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Key for PackageKey {
    type Value = PackageValue;
    const FUNCTION: SkyFunctionName = sky_functions::PACKAGE;
}

/// A Skyframe value representing a package.
// Java: com/google/devtools/build/lib/skyframe/PackageValue.java:32 PackageValue
pub struct PackageValue {
    pkg: Arc<Package>,
}

impl PackageValue {
    pub fn new(pkg: Arc<Package>) -> PackageValue {
        PackageValue { pkg }
    }

    /// Returns the package. This package may contain errors, in which case the caller should
    /// throw a `BuildFileContainsErrorsException` if an error-free package is needed.
    // Java: com/google/devtools/build/lib/skyframe/PackageValue.java:46 PackageValue.getPackage
    pub fn get_package(&self) -> &Arc<Package> {
        &self.pkg
    }
}

// Java: com/google/devtools/build/lib/skyframe/PackageValue.java:56 PackageValue.toString
impl fmt::Debug for PackageValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<PackageValue name={}>", self.pkg.get_name())
    }
}

/// Finds BUILD files through the package locator (Java `SkyframePackageManager` as a
/// `CachingPackageLocator`).
// Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:42 SkyframePackageManager
pub struct SkyframePackageLocator {
    pub pkg_locator: Arc<RwLock<PathPackageLocator>>,
    pub deleted_packages: Arc<RwLock<HashSet<PackageIdentifier>>>,
}

impl CachingPackageLocator for SkyframePackageLocator {
    // Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java:103 SkyframePackageManager.getBuildFileForPackage
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path> {
        // Note that this method needs to be thread-safe, as it is currently used concurrently by
        // legacy blaze code.
        if self.deleted_packages.read().contains(package_name) {
            return None;
        }
        self.pkg_locator
            .read()
            .get_package_build_file_nullable(package_name)
    }
}

/// [`IgnoredSubdirectories`] as the matcher the non-Skyframe globber uses.
struct IgnoredSubdirectoriesAdapter(IgnoredSubdirectories);

impl IgnoredSubdirectoriesMatcher for IgnoredSubdirectoriesAdapter {
    fn matching_entry(&self, directory: &PathFragment) -> Option<String> {
        self.0.matching_entry(directory)
    }
}

/// Performs non-Skyframe globbing operations and prepares the [`GlobRequest`]s set for
/// subsequent Skyframe-based globbing.
// Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:96 PackageFunctionWithSingleGlobsDep.GlobsGlobber
pub struct GlobsGlobber {
    non_skyframe_globber: NonSkyframeGlobber,
    glob_requests: Mutex<IndexSet<GlobRequest>>,
    /// Java `GlobsToken`s by token id: (operation, allow_empty).
    tokens: Mutex<IndexMap<u64, (GlobberOperation, bool)>>,
}

impl GlobsGlobber {
    fn new(non_skyframe_globber: NonSkyframeGlobber) -> GlobsGlobber {
        GlobsGlobber {
            non_skyframe_globber,
            glob_requests: Mutex::new(IndexSet::new()),
            tokens: Mutex::new(IndexMap::new()),
        }
    }

    /// Returns all package's globs, which will be used to construct the GLOBS key to be
    /// requested in Skyframe downstream. Empty if there is no glob in the package's BUILD file;
    /// requesting GLOBS is then skipped.
    // Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:150 PackageFunctionWithSingleGlobsDep.GlobsGlobber.getGlobRequests
    pub fn get_glob_requests(&self) -> Vec<GlobRequest> {
        self.glob_requests.lock().iter().cloned().collect()
    }
}

impl Globber for GlobsGlobber {
    // Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:105 PackageFunctionWithSingleGlobsDep.GlobsGlobber.runAsync
    fn run_async(
        &self,
        includes: &[String],
        excludes: &[String],
        operation: GlobberOperation,
        allow_empty: bool,
    ) -> Result<GlobberToken, GlobberError> {
        for pattern in includes {
            match GlobRequest::create(pattern, operation) {
                Ok(r) => {
                    self.glob_requests.lock().insert(r);
                }
                Err(e) => return Err(BadGlobException(e.to_string()).into()),
            }
        }

        let non_skyframe_glob_token =
            self.non_skyframe_globber
                .run_async(includes, excludes, operation, allow_empty)?;
        self.tokens
            .lock()
            .insert(non_skyframe_glob_token.0, (operation, allow_empty));
        Ok(non_skyframe_glob_token)
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:123 PackageFunctionWithSingleGlobsDep.GlobsGlobber.fetchUnsorted
    fn fetch_unsorted(&self, token: GlobberToken) -> Result<Vec<String>, GlobberError> {
        let (globber_operation, allow_empty) = *self
            .tokens
            .lock()
            .get(&token.0)
            .expect("ClassCastException: not a GlobsToken");
        // PORT: java-hash-order. Java copies the matches into a HashSet and returns them in its
        // iteration order; callers sort them. Here duplicates are dropped keeping first
        // occurrence.
        let mut matches: IndexSet<String> = IndexSet::new();
        matches.extend(self.non_skyframe_globber.fetch_unsorted(token)?);

        let result: Vec<String> = matches.into_iter().collect();
        if !allow_empty && result.is_empty() {
            return Err(bad_glob_exception_all_excluded(globber_operation).into());
        }
        Ok(result)
    }

    fn on_interrupt(&self) {
        self.non_skyframe_globber.on_interrupt();
    }

    fn on_completion(&self) {
        self.non_skyframe_globber.on_completion();
    }
}

/// Java `PackageFunctionException.Type`: which `NoSuchPackageException` subclass to create.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1733 PackageFunction.PackageFunctionException.Type
#[derive(Clone, Copy, Debug)]
enum ExceptionType {
    BuildFileContainsErrors,
    BuildFileNotFound,
    InvalidPackageName,
    NoSuchPackage,
}

/// The builder of a `PackageFunctionException` and its `NoSuchPackageException` cause.
///
/// PORT: Java's `setFilesystemCode` is not ported: Bazel's
/// `ActionOnFilesystemErrorCodeLoadingBzlFile` is `ALWAYS_USE_PACKAGE_LOADING_CODE` and
/// `ActionOnIOExceptionReadingBuildFile` is `UseOriginalIOException`, so only PackageLoading codes
/// occur.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1776 PackageFunction.PackageFunctionException.Builder
struct PackageFunctionExceptionBuilder {
    exception_type: ExceptionType,
    package_identifier: PackageIdentifier,
    transience: Transience,
    /// The message of Java's `exception` (the cause), if any.
    exception: Option<String>,
    message: String,
    package_loading_code: PackageLoadingCode,
}

impl PackageFunctionExceptionBuilder {
    fn new(
        exception_type: ExceptionType,
        package_identifier: &PackageIdentifier,
        transience: Transience,
        message: impl Into<String>,
        package_loading_code: PackageLoadingCode,
    ) -> Self {
        PackageFunctionExceptionBuilder {
            exception_type,
            package_identifier: package_identifier.clone(),
            transience,
            exception: None,
            message: message.into(),
            package_loading_code,
        }
    }

    fn set_exception(mut self, exception: impl Into<String>) -> Self {
        self.exception = Some(exception.into());
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1846 PackageFunction.PackageFunctionException.Builder.buildCause
    fn build_cause(self) -> NoSuchPackageException {
        let detailed_exit_code =
            DetailedExitCode::package_loading(self.message.clone(), self.package_loading_code);
        let (kind, cause) = match self.exception_type {
            // Java: BuildFileContainsErrorsException keeps an IOException cause only.
            ExceptionType::BuildFileContainsErrors => {
                (NoSuchPackageKind::BuildFileContainsErrors, self.exception)
            }
            ExceptionType::BuildFileNotFound => (NoSuchPackageKind::BuildFileNotFound, None),
            ExceptionType::InvalidPackageName => (NoSuchPackageKind::InvalidPackageName, None),
            ExceptionType::NoSuchPackage => (NoSuchPackageKind::NoSuchPackage, self.exception),
        };
        NoSuchPackageException::of_kind(
            kind,
            self.package_identifier,
            self.message,
            cause,
            Some(detailed_exit_code),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1864 PackageFunction.PackageFunctionException.Builder.build
    fn build(self) -> ComputeError {
        let transience = self.transience;
        package_function_exception(self.build_cause(), transience)
    }
}

/// Java `new PackageFunctionException(e, transience)`.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1717 PackageFunction.PackageFunctionException.<init>
fn package_function_exception(e: NoSuchPackageException, transience: Transience) -> ComputeError {
    SkyFunctionException::new(e, transience).into()
}

/// Java `PackageFunction.InternalInconsistentFilesystemException`.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1660 PackageFunction.InternalInconsistentFilesystemException
struct InternalInconsistentFilesystemException {
    is_transient: bool,
    package_identifier: PackageIdentifier,
    message: String,
}

impl InternalInconsistentFilesystemException {
    /// Used to represent a filesystem inconsistency discovered outside the PackageFunction.
    fn of(package_identifier: &PackageIdentifier, e: &IOException) -> Self {
        // This is not a transient error from the perspective of the PackageFunction.
        InternalInconsistentFilesystemException {
            is_transient: false,
            package_identifier: package_identifier.clone(),
            message: e.get_message().to_owned(),
        }
    }

    /// Used to represent a filesystem inconsistency discovered by the PackageFunction.
    fn discovered(package_identifier: &PackageIdentifier, inconsistency_message: String) -> Self {
        InternalInconsistentFilesystemException {
            is_transient: true,
            package_identifier: package_identifier.clone(),
            message: IOException::inconsistent_filesystem(inconsistency_message)
                .get_message()
                .to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1690 PackageFunction.InternalInconsistentFilesystemException.throwPackageFunctionException
    fn throw_package_function_exception(self) -> ComputeError {
        PackageFunctionExceptionBuilder::new(
            ExceptionType::NoSuchPackage,
            &self.package_identifier,
            if self.is_transient {
                Transience::Transient
            } else {
                Transience::Persistent
            },
            self.message.clone(),
            if self.is_transient {
                PackageLoadingCode::TransientInconsistentFilesystemError
            } else {
                PackageLoadingCode::PersistentInconsistentFilesystemError
            },
        )
        .set_exception(self.message)
        .build()
    }
}

// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:331 PackageFunction.maybeThrowFilesystemInconsistency
fn maybe_throw_filesystem_inconsistency(
    pkg_id: &PackageIdentifier,
    skyframe_exception_message: &str,
    package_was_in_error: bool,
) -> Result<(), InternalInconsistentFilesystemException> {
    if !package_was_in_error {
        return Err(InternalInconsistentFilesystemException::discovered(
            pkg_id,
            format!(
                "Encountered error '{}' but didn't encounter it when doing the same thing earlier \
                 in the build",
                skyframe_exception_message
            ),
        ));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:982 PackageFunction.badRepoFileException
fn bad_repo_file_exception(cause: &str, package_id: &PackageIdentifier) -> ComputeError {
    PackageFunctionExceptionBuilder::new(
        ExceptionType::BuildFileContainsErrors,
        package_id,
        Transience::Persistent,
        "bad REPO.bazel file",
        PackageLoadingCode::BadRepoFile,
    )
    .set_exception(cause)
    .build()
}

/// The result of compiling a BUILD file (Java `PackageFunction.CompiledBuildFile`): either the
/// static errors or the program and what Java extracted from the syntax tree.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:147 PackageFunction.CompiledBuildFile
enum CompiledBuildFile {
    Errors(Vec<SyntaxError>),
    Ok {
        prog: CompiledFile,
        generator_map: IndexMap<Location, String>,
        /// The predeclared environment the program was compiled against.
        predeclared: Arc<Globals>,
        /// The predeclared names left out of `predeclared` because their `GuardedValue` is not
        /// accessible, each with Java's error for a use of it (`Module.resolve`).
        guarded_symbol_errors: IndexMap<String, String>,
    },
}

/// What PACKAGE is constructed with (Java `PackageFunction.Builder` settings).
pub struct PackageFunctionConfig {
    pub package_factory: Arc<PackageFactory>,
    /// Java `packageLocator` (`SkyframePackageManager`).
    pub package_locator: Arc<dyn CachingPackageLocator>,
    pub bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
    /// Java `showLoadingProgress` (`--show_loading_progress`).
    pub show_loading_progress: bool,
    /// Java `shouldUseRepoDotBazel` (true in Bazel).
    pub should_use_repo_dot_bazel: bool,
}

/// A SkyFunction which computes a [`PackageValue`] given a [`PackageKey`] (Java
/// `PackageFunctionWithSingleGlobsDep`).
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:120 PackageFunction
pub struct PackageFunction {
    config: Arc<PackageFunctionConfig>,
}

/// A loaded package and its globber (Java `LoadedPackageWithGlobRequests`).
// Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:83 PackageFunctionWithSingleGlobsDep.LoadedPackageWithGlobRequests
struct LoadedPackage {
    builder: TargetDefinitionContext,
    glob_requests: Vec<GlobRequest>,
}

impl PackageFunction {
    pub fn new(config: Arc<PackageFunctionConfig>) -> Self {
        PackageFunction { config }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:386 PackageFunction.compute
    pub fn compute(
        &self,
        key: &PackageKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackageValue>, ComputeError> {
        let package_id = key.argument();
        if *package_id == *EXTERNAL_PACKAGE_IDENTIFIER {
            return Err(PackageFunctionExceptionBuilder::new(
                ExceptionType::NoSuchPackage,
                &EXTERNAL_PACKAGE_IDENTIFIER,
                Transience::Persistent,
                "//external package is not available since the WORKSPACE file is deprecated, \
                 please migrate to Bzlmod. See \
                 https://bazel.build/external/migration#bind-targets.",
                PackageLoadingCode::WorkspaceFileError,
            )
            .build());
        }

        let Some(lazy_macro_expansion_packages) = LAZY_MACRO_EXPANSION_PACKAGES.get(env)? else {
            return Ok(None);
        };
        if lazy_macro_expansion_packages.contains(package_id) {
            blaze_util::unported!("PackageFunction.computePackageFromPackagePieces");
        }

        let Some(package_lookup_value) = self.get_package_lookup_value(package_id, env)? else {
            return Ok(None);
        };

        if !package_lookup_value.package_exists() {
            let exception = match package_lookup_value.get_error_reason() {
                ErrorReason::NoBuildFile => {
                    let message =
                        PackageLookupFunction::explain_no_build_file_value(package_id, env)?;
                    PackageFunctionExceptionBuilder::new(
                        ExceptionType::BuildFileNotFound,
                        package_id,
                        Transience::Persistent,
                        message,
                        PackageLoadingCode::BuildFileMissing,
                    )
                }
                ErrorReason::DeletedPackage | ErrorReason::RepositoryNotFound => {
                    PackageFunctionExceptionBuilder::new(
                        ExceptionType::BuildFileNotFound,
                        package_id,
                        Transience::Persistent,
                        package_lookup_value.get_error_msg(),
                        PackageLoadingCode::RepositoryMissing,
                    )
                }
                ErrorReason::InvalidPackageName => PackageFunctionExceptionBuilder::new(
                    ExceptionType::InvalidPackageName,
                    package_id,
                    Transience::Persistent,
                    package_lookup_value.get_error_msg(),
                    PackageLoadingCode::InvalidName,
                ),
            };
            return Err(exception.build());
        }

        let Some(autoload_symbols) = AUTOLOAD_SYMBOLS.get(env)? else {
            return Ok(None);
        };
        let requires_autoloads =
            !autoload_symbols.autoloads_disabled_in_build_for_repo(package_id.get_repository());
        let starlark_builtins_value = match env
            .get_value_or_throw::<StarlarkBuiltinsKey, BuiltinsFailedException>(
                StarlarkBuiltinsKey::key(/* with_autoloads= */ requires_autoloads),
            )? {
            Err(e) => {
                return Err(PackageFunctionExceptionBuilder::new(
                    ExceptionType::BuildFileContainsErrors,
                    package_id,
                    Transience::Persistent,
                    format!(
                        "Internal error while loading Starlark builtins: {}",
                        e.get_message()
                    ),
                    PackageLoadingCode::BuiltinsInjectionFailure,
                )
                .build());
            }
            Ok(v) => v,
        };

        if env.values_missing() {
            return Ok(None);
        }
        let Some(starlark_builtins_value) = starlark_builtins_value else {
            return Ok(None);
        };

        let Some(mut loaded_package) = self.load_package(
            &package_lookup_value,
            package_id,
            &starlark_builtins_value,
            package_lookup_value.get_root(),
            env,
        )?
        else {
            return Ok(None);
        };

        let mut pfe_from_non_skyframe_globbing = None;
        let pkg_builder = &mut loaded_package.builder;
        match pkg_builder.build_partial() {
            Ok(()) => {
                // Note that handleLabelsCrossingSubpackagesAndPropagateInconsistentFilesystemExceptions
                // expects to mutate pkgBuilder.getTargets(), and thus can only be safely called if
                // pkgBuilder.buildPartial() didn't throw.
                if let Err(e) =
                    handle_labels_crossing_subpackages_and_propagate_inconsistent_filesystem_exceptions(
                        package_lookup_value.get_root(),
                        package_id,
                        pkg_builder,
                        env,
                    )?
                {
                    return Err(e.throw_package_function_exception());
                }
            }
            Err(e) => {
                // If non-Skyframe globbing encounters an IOException, #buildPartial will throw a
                // NoSuchPackageException. If that happens, we prefer throwing an exception derived
                // from Skyframe globbing. See the comments in
                // #handleGlobDepsAndPropagateFilesystemExceptions. Therefore we store the
                // exception encountered here and maybe use it later.
                // PORT: Java picks the transience from the cause's class (DetailedIOException,
                // SkyframeGlobbingIOException); the ported exception keeps only the cause's
                // message, so this is Java's default branch.
                pfe_from_non_skyframe_globbing =
                    Some(package_function_exception(e, Transience::Transient));
            }
        }

        let contains_errors = pkg_builder.contains_errors();
        self.handle_glob_deps_and_propagate_filesystem_exceptions(
            package_id,
            package_lookup_value.get_root(),
            &loaded_package.glob_requests,
            env,
            contains_errors,
        )??;

        if let Some(pfe) = pfe_from_non_skyframe_globbing {
            // Throw before checking for missing values, since this may be our last chance to throw
            // if in nokeep-going error bubbling.
            return Err(pfe);
        }

        if env.values_missing() {
            return Ok(None);
        }

        let pkg_builder = &mut loaded_package.builder;
        let pkg = pkg_builder.finish_build();

        pkg_builder
            .get_local_event_handler()
            .replay_on(env.listener());

        if let Err(e) = self.config.package_factory.after_done_loading_package(
            &pkg,
            &starlark_builtins_value.starlark_semantics,
            env.listener(),
        ) {
            return Err(package_function_exception(e, Transience::Persistent));
        }

        Ok(Some(PackageValue::new(pkg)))
    }

    /// Java `env.getValueOrThrow(packageLookupKey, BadRepoFileException.class,
    /// InvalidIgnorePathException.class, BuildFileNotFoundException.class,
    /// InconsistentFilesystemException.class)` and its catch blocks.
    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:435 PackageFunction.compute
    fn get_package_lookup_value(
        &self,
        package_id: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<PackageLookupValue>>, ComputeError> {
        enum Caught {
            InvalidIgnorePath(InvalidIgnorePathException),
            BadRepoFile(BadRepoFileException),
            BuildFileNotFound(NoSuchPackageException),
            InconsistentFilesystem(IOException),
        }
        let package_lookup_key = PackageLookupKey::key(package_id.clone());
        let caught = env.get_value_or_throw_with(package_lookup_key, |e| {
            if let Some(e) = e.downcast_ref::<InvalidIgnorePathException>() {
                Some(Caught::InvalidIgnorePath(e.clone()))
            } else if let Some(e) = e.downcast_ref::<BadRepoFileException>() {
                Some(Caught::BadRepoFile(e.clone()))
            } else if let Some(e) = e
                .downcast_ref::<NoSuchPackageException>()
                .filter(|e| e.kind() == NoSuchPackageKind::BuildFileNotFound)
            {
                Some(Caught::BuildFileNotFound(e.clone()))
            } else {
                e.downcast_ref::<IOException>()
                    .filter(|e| e.is_inconsistent_filesystem())
                    .map(|e| Caught::InconsistentFilesystem(e.clone()))
            }
        })?;
        match caught {
            Ok(v) => Ok(v),
            Err(Caught::InvalidIgnorePath(e)) => Err(PackageFunctionExceptionBuilder::new(
                ExceptionType::NoSuchPackage,
                package_id,
                Transience::Persistent,
                e.to_string(),
                PackageLoadingCode::BadIgnoredDirectories,
            )
            .set_exception(e.to_string())
            .build()),
            Err(Caught::BadRepoFile(e)) => {
                Err(bad_repo_file_exception(e.get_message(), package_id))
            }
            Err(Caught::BuildFileNotFound(e)) => {
                Err(package_function_exception(e, Transience::Persistent))
            }
            // This error is not transient from the perspective of the PackageFunction.
            Err(Caught::InconsistentFilesystem(e)) => Err(PackageFunctionExceptionBuilder::new(
                ExceptionType::NoSuchPackage,
                package_id,
                Transience::Persistent,
                e.get_message(),
                PackageLoadingCode::PersistentInconsistentFilesystemError,
            )
            .set_exception(e.get_message())
            .build()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageFunctionWithSingleGlobsDep.java:174 PackageFunctionWithSingleGlobsDep.handleGlobDepsAndPropagateFilesystemExceptions
    fn handle_glob_deps_and_propagate_filesystem_exceptions(
        &self,
        package_identifier: &PackageIdentifier,
        package_root: &Root,
        glob_requests: &[GlobRequest],
        env: &mut Environment<'_>,
        package_was_in_error: bool,
    ) -> Result<Result<(), ComputeError>, Interrupted> {
        if glob_requests.is_empty() {
            return Ok(Ok(()));
        }

        let globs_key = GlobsKey::key(
            package_identifier.clone(),
            package_root.clone(),
            glob_requests.iter().cloned(),
        );
        enum Caught {
            Io(IOException),
            BuildFileNotFound(NoSuchPackageException),
        }
        let caught = env.get_value_or_throw_with(globs_key, |e| {
            if let Some(e) = e.downcast_ref::<IOException>() {
                Some(Caught::Io(e.clone()))
            } else {
                e.downcast_ref::<NoSuchPackageException>()
                    .filter(|e| e.kind() == NoSuchPackageKind::BuildFileNotFound)
                    .map(|e| Caught::BuildFileNotFound(e.clone()))
            }
        })?;
        let message = match caught {
            Ok(_) => return Ok(Ok(())),
            Err(Caught::BuildFileNotFound(e)) => e.get_message(),
            Err(Caught::Io(e)) if e.is_inconsistent_filesystem() => {
                return Ok(Err(InternalInconsistentFilesystemException::of(
                    package_identifier,
                    &e,
                )
                .throw_package_function_exception()));
            }
            Err(Caught::Io(e)) if e.is_file_symlink_exception() => {
                // Please note that GlobsFunction or its deps FileFunction throws the first
                // `FileSymlinkException` discovered.
                let message = format!(
                    "Symlink issue while evaluating globs: {}",
                    e.get_user_friendly_message()
                        .expect("a FileSymlinkException has a user-friendly message")
                );
                return Ok(Err(PackageFunctionExceptionBuilder::new(
                    ExceptionType::NoSuchPackage,
                    package_identifier,
                    Transience::Persistent,
                    message,
                    PackageLoadingCode::EvalGlobsSymlinkError,
                )
                .build()));
            }
            Err(Caught::Io(e)) => e.get_message().to_owned(),
        };
        Ok(
            match maybe_throw_filesystem_inconsistency(
                package_identifier,
                &message,
                package_was_in_error,
            ) {
                Ok(()) => Ok(()),
                Err(e) => Err(e.throw_package_function_exception()),
            },
        )
    }

    /// Constructs a [`Package`] builder for the given package and executes its BUILD file. Note
    /// that the returned package may be in error.
    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1001 PackageFunction.loadPackage
    fn load_package(
        &self,
        package_lookup_value: &PackageLookupValue,
        package_id: &PackageIdentifier,
        starlark_builtins_value: &StarlarkBuiltinsValue,
        _package_root: &Root,
        env: &mut Environment<'_>,
    ) -> Result<Option<LoadedPackage>, ComputeError> {
        let repository_mapping_value = env.get_value(RepositoryMappingKey::key(
            package_id.get_repository().clone(),
        ))?;
        let main_repository_mapping_value =
            env.get_value(RepositoryMappingKey::key(MAIN.clone()))?;
        let build_file_rooted_path = package_lookup_value.get_rooted_path(package_id);
        let build_file_value = get_build_file_value(env, &build_file_rooted_path)?;
        let default_visibility = DEFAULT_VISIBILITY.get(env)?;
        let config_setting_visibility_policy = CONFIG_SETTING_VISIBILITY_POLICY.get(env)?;
        let repository_ignored_subdirectories = env.get_value(IgnoredSubdirectoriesKey::key(
            package_id.get_repository().clone(),
        ))?;
        let repo_package_args_value = if self.config.should_use_repo_dot_bazel {
            let repo_key = RepoPackageArgsKey::key(package_id.get_repository().clone());
            match env.get_value_or_throw_with(repo_key, |e| {
                if let Some(e) = e.downcast_ref::<IOException>() {
                    Some(e.get_message().to_owned())
                } else {
                    e.downcast_ref::<BadRepoFileException>()
                        .map(|e| e.get_message().to_owned())
                }
            })? {
                Err(message) => return Err(bad_repo_file_exception(&message, package_id)),
                Ok(v) => v,
            }
        } else {
            Some(Arc::new(RepoPackageArgsValue::empty()))
        };

        if env.values_missing() {
            return Ok(None);
        }
        let (
            Some(repository_mapping_value),
            Some(main_repository_mapping_value),
            Some(build_file_value),
            Some(default_visibility),
            Some(config_setting_visibility_policy),
            Some(repository_ignored_subdirectories),
            Some(repo_package_args_value),
        ) = (
            repository_mapping_value,
            main_repository_mapping_value,
            build_file_value,
            default_visibility,
            config_setting_visibility_policy,
            repository_ignored_subdirectories,
            repo_package_args_value,
        )
        else {
            return Ok(None);
        };

        let repository_mapping = repository_mapping_value
            .repository_mapping()
            .cloned()
            .unwrap_or_else(|| {
                blaze_util::unported!("PackageFunction.loadPackage without a repository mapping")
            });
        let main_repository_mapping = main_repository_mapping_value
            .repository_mapping()
            .cloned()
            .expect("the main repository has a mapping");
        let semantics = starlark_builtins_value.starlark_semantics.clone();

        // Load (optional) prelude, which determines environment.
        let package_factory = &self.config.package_factory;
        let mut prelude_bindings: Option<FrozenModule> = None;
        // Load the prelude from the same repository as the package being loaded.
        if let Some(raw_prelude_label) = package_factory
            .get_rule_class_provider()
            .get_prelude_label()
        {
            let prelude_package = PackageIdentifier::create(
                package_id.get_repository().clone(),
                raw_prelude_label.get_package_fragment().clone(),
            );
            let prelude_label =
                Label::create_unvalidated(prelude_package, raw_prelude_label.get_name());
            match load_prelude(env, package_id, prelude_label)? {
                Err(e) => return Err(package_function_exception(e, Transience::Persistent)),
                Ok(None) => return Ok(None), // skyframe restart
                Ok(Some(prelude)) => prelude_bindings = Some(prelude),
            }
        }

        // vv ---- begin PackageCompileFunction ---- vv

        if self.config.show_loading_progress {
            env.listener()
                .handle(&Event::progress(format!("Loading package: {}", package_id)));
        }
        let compiled = self.compile_build_file(
            package_id,
            &build_file_rooted_path,
            starlark_builtins_value,
            prelude_bindings.as_ref(),
            env.listener(),
        )?;

        // ^^ ---- end PackageCompileFunction ---- ^^

        let mut loaded_modules: Option<(LoadMap, Vec<Arc<BazelModuleContext>>)> = None;
        if let CompiledBuildFile::Ok { prog, .. } = &compiled {
            // Parse the labels in the file's load statements.
            let program_loads = &prog.loads;
            let Some(load_labels) = get_load_labels(
                env.listener(),
                program_loads,
                package_id,
                &is_package_under_experimental,
                semantics.allow_experimental_loads,
                &repository_mapping,
                /* within_scl_dialect= */ false,
                semantics.experimental_enable_scl_dialect,
                None,
            ) else {
                return Err(PackageFunctionExceptionBuilder::new(
                    ExceptionType::BuildFileContainsErrors,
                    package_id,
                    Transience::Persistent,
                    "malformed load statements",
                    PackageLoadingCode::ImportStarlarkFileError,
                )
                .build());
            };

            // Compute key for each label in loads.
            let keys: Vec<BzlLoadKey> = load_labels
                .into_iter()
                .map(BzlLoadKey::key_for_build)
                .collect();

            // Load .bzl modules in parallel.
            let build_file_label = Label::create_in_package(
                package_id,
                package_lookup_value
                    .get_build_file_name()
                    .get_filename_fragment()
                    .get_path_string(),
            )
            .expect("Failed to construct label representing BUILD file");
            match load_bzl_modules(
                env,
                package_id,
                &format!("file {}", build_file_label.get_canonical_form()),
                program_loads,
                &keys,
                &semantics,
                /* check_visibility= */ true,
            )? {
                Err(e) => return Err(package_function_exception(e, Transience::Persistent)),
                Ok(None) => return Ok(None), // skyframe restart
                Ok(Some(m)) => loaded_modules = Some(m),
            }
        }

        // From this point on, no matter whether the function returns successfully or throws an
        // exception, there will be no more Skyframe restarts.
        // PORT: Java notes `packageProgress.doneReadPackage(packageId)` in the finally block once
        // committed; counted here (the count is the same, no restart follows).
        crate::package_progress_receiver::PACKAGE_PROGRESS.done_read_package(package_id);

        let package_directory = build_file_rooted_path
            .as_path()
            .get_parent_directory()
            .expect("a BUILD file has a parent directory");
        let non_skyframe_globber = package_factory.create_non_skyframe_globber(
            package_directory,
            package_id.clone(),
            Arc::new(IgnoredSubdirectoriesAdapter(
                repository_ignored_subdirectories
                    .as_ignored_subdirectories()
                    .clone(),
            )),
            self.config.package_locator.clone(),
        );
        let globber = Arc::new(GlobsGlobber::new(non_skyframe_globber));

        let generator_map = match &compiled {
            CompiledBuildFile::Ok { generator_map, .. } => Some(generator_map.clone()),
            CompiledBuildFile::Errors(_) => None,
        };
        // Create the package, even if it will be empty because we cannot attempt execution.
        let mut pkg_builder = package_factory.new_package_builder(
            package_id.clone(),
            build_file_rooted_path.clone(),
            repository_mapping_value
                .associated_module_name()
                .map(str::to_owned),
            repository_mapping_value
                .associated_module_version()
                .map(str::to_owned),
            &semantics,
            repository_mapping.clone(),
            main_repository_mapping.clone(),
            generator_map,
            Some(*config_setting_visibility_policy),
            Some(globber.clone() as Arc<dyn Globber>),
        );

        pkg_builder.merge_package_args_from(
            &PackageArgs::builder()
                .set_default_visibility((*default_visibility).clone())
                .build(),
        );
        pkg_builder.merge_package_args_from(repo_package_args_value.get_package_args());

        match compiled {
            CompiledBuildFile::Ok {
                prog,
                predeclared,
                guarded_symbol_errors,
                ..
            } => {
                let (load_map, direct_loads) =
                    loaded_modules.expect("loads are computed for a compiled BUILD file");
                pkg_builder = self.execute_build_file(
                    pkg_builder,
                    prog,
                    &predeclared,
                    &load_map,
                    &direct_loads,
                    &semantics,
                    &repository_mapping,
                    &main_repository_mapping,
                )?;
                // Java resolves the BUILD file's names in compileBuildFile: a use of a
                // predeclared symbol whose GuardedValue is not accessible is a static error with
                // the guard's message (Module.resolve), reported as a SYNTAX_ERROR event at the
                // identifier, and the file is not executed.
                // PORT: starlark-rust resolves names when execution starts and PackageFactory
                // (blaze_packages `handle_eval_error`) reports its "Variable `X` not found" as
                // an evaluation error. Bazel 9.3.2 registers no guarded BUILD toplevel, so this
                // cannot happen with Bazel's rule class provider; with one that does, fail
                // loudly instead of showing the wrong error.
                // Java: net/starlark/java/eval/Module.java:237 Module.resolve
                // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1276 PackageFunction.compileBuildFile
                if !guarded_symbol_errors.is_empty() {
                    for event in pkg_builder.get_local_event_handler().get_events() {
                        let message = event.get_message();
                        let Some(error) = message
                            .lines()
                            .last()
                            .and_then(|line| line.strip_prefix("Error: "))
                        else {
                            continue;
                        };
                        let Some(name) =
                            blaze_packages::builtins::env::undefined_variable_name(error)
                        else {
                            continue;
                        };
                        if let Some(java_message) = guarded_symbol_errors.get(name) {
                            blaze_util::unported!(
                                "Module.resolve",
                                "BUILD file of package '{}' uses the guarded symbol '{}'; Java \
                                 reports the syntax error: {}",
                                package_id,
                                name,
                                java_message
                            );
                        }
                    }
                }
                // TODO: b/155396641 - Validate that transitive visibility groups are correctly
                // declared and that this package is a member of all transitive visibility groups
                // it declares, but probably not in this part of the code.
                if let Err(ex) = pkg_builder.expand_all_remaining_macros(&semantics) {
                    // PORT: Java reports `getMessageWithStack()`; the ported EvalException has
                    // no stack.
                    pkg_builder
                        .get_local_event_handler()
                        .handle(&Package::error(
                            None,
                            ex.get_message(),
                            PackageLoadingCode::StarlarkEvalError,
                        ));
                    pkg_builder.set_contains_errors();
                }
            }
            CompiledBuildFile::Errors(errors) => {
                // Execution not attempted due to static errors.
                for err in &errors {
                    pkg_builder
                        .get_local_event_handler()
                        .handle(&Package::error(
                            Some(&err.location),
                            &err.message,
                            PackageLoadingCode::SyntaxError,
                        ));
                }
                pkg_builder.set_contains_errors();
            }
        }

        let _ = build_file_value;
        Ok(Some(LoadedPackage {
            builder: pkg_builder,
            glob_requests: globber.get_glob_requests(),
        }))
    }

    /// Executes the compiled BUILD file into `pkg_builder` (Java
    /// `packageFactory.executeBuildFile(...)` with the thread set up as Java's
    /// `executeBuildFileImpl` does).
    // Java: com/google/devtools/build/lib/packages/PackageFactory.java:518 PackageFactory.executeBuildFileImpl
    fn execute_build_file(
        &self,
        pkg_builder: TargetDefinitionContext,
        prog: CompiledFile,
        predeclared: &Globals,
        load_map: &LoadMap,
        direct_loads: &[Arc<BazelModuleContext>],
        semantics: &Arc<StarlarkSemantics>,
        repository_mapping: &RepositoryMapping,
        main_repository_mapping: &RepositoryMapping,
    ) -> Result<TargetDefinitionContext, ComputeError> {
        // Java Package.AbstractBuilder.setLoads(loadedModules.values()).
        let transitive_loads = compute_transitive_loads(direct_loads);
        let mut fp = Fingerprint::new();
        for module in direct_loads {
            fp.add_bytes(&module.bzl_transitive_digest);
        }
        let transitive_bzl_digest = fp.digest_and_reset();

        let build_file_label = pkg_builder.get_metadata().build_file_label.clone();
        // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(pkgBuilder.getLocalEventHandler())).
        let events = Arc::new(DebugPrintHandler::new(
            pkg_builder.get_local_event_handler_arc().clone(),
        ));
        let thread_context = ThreadContext::new(Phase::Loading, semantics.clone(), events.clone())
            .with_file_label(build_file_label.clone())
            .with_label_repo_context(LabelRepoContextImpl::of(
                build_file_label.get_repository().clone(),
                repository_mapping.clone(),
            ))
            .with_main_repo_mapping(main_repository_mapping.clone());
        // `Label()` called from functions of loaded modules (the BUILD file's loads, the
        // prelude's loads, @_builtins modules) resolves against the function's own module, read
        // from its extra value (Java BazelModuleContext.ofInnermostBzlOrFail).
        let thread_context = thread_context.with_payload(pkg_builder);

        let result = self.config.package_factory.execute_build_file(
            &thread_context,
            prog,
            predeclared,
            load_map,
            transitive_loads,
            transitive_bzl_digest,
            &EvalOptions::default(),
        );
        let pkg_builder = thread_context
            .take_payload::<TargetDefinitionContext>()
            .expect("the BUILD thread context holds the package builder");
        match result {
            Ok(()) => Ok(pkg_builder),
            Err(_) => Err(Interrupted.into()),
        }
    }

    /// Reads, parses, resolves, and compiles a BUILD file. A read error is reported as a
    /// PackageFunctionException; a syntax error by returning [`CompiledBuildFile::Errors`].
    // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:1276 PackageFunction.compileBuildFile
    fn compile_build_file(
        &self,
        package_id: &PackageIdentifier,
        build_file_path: &RootedPath,
        starlark_builtins_value: &StarlarkBuiltinsValue,
        prelude_bindings: Option<&FrozenModule>,
        reporter: &dyn ExtendedEventHandler,
    ) -> Result<CompiledBuildFile, ComputeError> {
        let semantics = &starlark_builtins_value.starlark_semantics;

        // read BUILD file
        let input_file = build_file_path.as_path();
        let read = (|| -> std::io::Result<Vec<u8>> {
            let mut input = input_file.get_input_stream()?;
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut input, &mut contents)?;
            Ok(contents)
        })();
        let build_file_bytes = match read {
            Ok(b) => b,
            Err(e) => {
                // Note that we did the work that led to this IOException, so we should
                // conservatively report this error as transient.
                let message = IOException::from_io(&e).get_message().to_owned();
                return Err(PackageFunctionExceptionBuilder::new(
                    ExceptionType::BuildFileContainsErrors,
                    package_id,
                    Transience::Transient,
                    message.clone(),
                    PackageLoadingCode::BuildFileMissing,
                )
                .set_exception(message)
                .build());
            }
        };
        let handler = StoredEventHandler::new();
        let input = match starlark_util::create_parser_input(
            build_file_bytes,
            input_file.get_path_string(),
            semantics.incompatible_enforce_starlark_utf8,
            &handler,
        ) {
            Ok(input) => input,
            Err(e) => {
                handler.replay_on(reporter);
                return Err(PackageFunctionExceptionBuilder::new(
                    ExceptionType::BuildFileContainsErrors,
                    package_id,
                    Transience::Persistent,
                    format!("error reading {}", input_file),
                    PackageLoadingCode::StarlarkEvalError,
                )
                .set_exception(e.to_string())
                .build());
            }
        };
        handler.replay_on(reporter);

        // Parse, check BUILD syntax (PackageFactory.checkBuildSyntax: forbidden syntax, literal
        // glob patterns, generator names) and resolve: blaze_starlark::eval::compile with the
        // BUILD file options (loads bind globally, top-level rebinding allowed).
        let prog = match compile(FileKind::Build, input_file.get_path_string(), input) {
            Ok(prog) => prog,
            Err(e) => {
                let EvalErrorKind::Syntax(errors) = e.kind else {
                    unreachable!("compile only fails with syntax errors")
                };
                return Ok(CompiledBuildFile::Errors(errors));
            }
        };
        let mut generator_map = IndexMap::new();
        for (location, name) in &prog.build_facts.generator_name_by_location {
            generator_map.insert(location.clone(), name.clone());
        }

        // Construct static environment for resolution/compilation.
        // TODO(#11437): Remove conditional once disabling injection is no longer allowed.
        let predeclared_env: &StarlarkEnv = if semantics.experimental_builtins_bzl_path.is_empty() {
            self.config
                .bazel_starlark_environment
                .get_uninjected_build_env()
        } else {
            &starlark_builtins_value.predeclared_for_build
        };
        let guarded_symbol_errors;
        let predeclared = match prelude_bindings {
            None => {
                guarded_symbol_errors = blaze_packages::builtins::env::guarded_symbol_errors(
                    predeclared_env,
                    semantics,
                    None,
                );
                starlark_builtins_value.globals(predeclared_env, None, None)
            }
            Some(prelude) => {
                let mut env = predeclared_env.clone();
                // Java: PackageFunction.java:1068 `preludeBindings = prelude.getGlobals()` and
                // :1400 `predeclared.putAll(preludeBindings)`. Module.getGlobals() has every
                // global binding: names starting with `_` and, because the prelude is compiled
                // with loadBindsGlobally (BzlCompileFunction.java:204), names bound by load().
                // starlark-rust keeps both as private module bindings (patch 0021 lists them).
                // A name declared but never assigned has no value and no entry, as in Java.
                for name in prelude.names_any_visibility() {
                    let name = name.as_str().to_owned();
                    if let Ok((value, _visibility)) = prelude.get_any_visibility(&name) {
                        env.insert(name, EnvEntry::plain(value));
                    }
                }
                guarded_symbol_errors =
                    blaze_packages::builtins::env::guarded_symbol_errors(&env, semantics, None);
                Arc::new(build_globals(&env, None, semantics, None))
            }
        };

        // success
        Ok(CompiledBuildFile::Ok {
            prog,
            generator_map,
            predeclared,
            guarded_symbol_errors,
        })
    }
}

/// Java `getBuildFileValue`.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:645 PackageFunction.getBuildFileValue
fn get_build_file_value(
    env: &mut Environment<'_>,
    build_file_rooted_path: &RootedPath,
) -> Result<Option<Arc<crate::actions::FileValue>>, Interrupted> {
    let build_file_value = match env
        .get_value_or_throw::<FileKey, IOException>(FileKey::key(build_file_rooted_path.clone()))?
    {
        Err(e) => panic!(
            "IllegalStateException: Package lookup succeeded but encountered error when \
                 getting FileValue for BUILD file directly.: {}",
            e
        ),
        Ok(v) => v,
    };
    let Some(build_file_value) = build_file_value else {
        return Ok(None);
    };
    assert!(
        build_file_value.exists(),
        "Package lookup succeeded but BUILD file doesn't exist"
    );
    Ok(Some(build_file_value))
}

/// Java `BazelModuleContext.visitLoadGraphRecursively`: the visitor decides whether to recurse.
// Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:88 BazelModuleContext.visitLoadGraphRecursively
fn visit_load_graph(
    loads: &[Arc<BazelModuleContext>],
    visitor: &mut dyn FnMut(&Arc<BazelModuleContext>) -> bool,
) {
    for module in loads {
        if visitor(module) {
            visit_load_graph(&module.loads, visitor);
        }
    }
}

// Java: com/google/devtools/build/lib/packages/Package.java:736 Package.AbstractBuilder.computeTransitiveLoads
fn compute_transitive_loads(direct_loads: &[Arc<BazelModuleContext>]) -> Vec<Label> {
    let mut loads: IndexSet<Label> = IndexSet::new();
    visit_load_graph(direct_loads, &mut |m| loads.insert(m.label.clone()));
    loads.into_iter().collect()
}

/// Java `BzlLoadFailedException` → the `NoSuchPackageException` PackageFunction throws for it.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:700 PackageFunction.loadBzlModules
fn bzl_load_failed(
    package_id: &PackageIdentifier,
    e: &BzlLoadFailedException,
) -> NoSuchPackageException {
    // PORT: Java attaches the root cause when it is an IOException; the ported exception keeps
    // no cause chain.
    PackageFunctionExceptionBuilder::new(
        ExceptionType::BuildFileContainsErrors,
        package_id,
        e.get_transience(),
        e.get_message(),
        PackageLoadingCode::ImportStarlarkFileError,
    )
    .build_cause()
}

/// Compute the BzlLoadValue for all given keys using vanilla Skyframe evaluation, returning
/// `None` if Skyframe deps were missing.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:768 PackageFunction.computeBzlLoadsNoInlining
fn compute_bzl_loads_no_inlining(
    env: &mut Environment<'_>,
    keys: &[BzlLoadKey],
) -> Result<Result<Option<Vec<Arc<BzlLoadValue>>>, BzlLoadFailedException>, Interrupted> {
    let sky_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
    let starlark_lookup_results = env.get_values_and_exceptions(&sky_keys)?;
    let mut bzl_loads = Vec::with_capacity(keys.len());
    for key in keys {
        // TODO(adonovan): if get fails, report the source location in the corresponding
        // programLoads[i] (see caller).
        match starlark_lookup_results.get_or_throw_typed::<BzlLoadKey, BzlLoadFailedException>(key)
        {
            Err(e) => return Ok(Err(e)),
            Ok(Some(v)) => bzl_loads.push(v),
            Ok(None) => {}
        }
    }
    Ok(Ok(if env.values_missing() {
        None
    } else {
        Some(bzl_loads)
    }))
}

/// Loads the .bzl modules whose names and load-locations are `program_loads`, and whose
/// corresponding Skyframe keys are `keys`; validates load visibility. Returns the load map
/// (load string → module) and the direct loads in the map's order, or `None` for a Skyframe
/// restart.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:700 PackageFunction.loadBzlModules
fn load_bzl_modules(
    env: &mut Environment<'_>,
    package_id: &PackageIdentifier,
    requesting_file_description: &str,
    program_loads: &[LoadRequest],
    keys: &[BzlLoadKey],
    semantics: &StarlarkSemantics,
    check_visibility: bool,
) -> Result<
    Result<Option<(LoadMap, Vec<Arc<BazelModuleContext>>)>, NoSuchPackageException>,
    Interrupted,
> {
    let bzl_loads = match compute_bzl_loads_no_inlining(env, keys)? {
        Err(e) => return Ok(Err(bzl_load_failed(package_id, &e))),
        Ok(None) => return Ok(Ok(None)), // Skyframe deps unavailable
        Ok(Some(v)) => v,
    };
    // Validate that the current BUILD file satisfies each loaded dependency's load visibility.
    if check_visibility
        && let Err(e) = check_load_visibilities(
            package_id,
            requesting_file_description,
            &bzl_loads,
            keys,
            program_loads,
            /* demote_errors_to_warnings= */ !semantics.check_bzl_visibility,
            env.listener(),
        )
    {
        return Ok(Err(bzl_load_failed(package_id, &e)));
    }

    // Build map of loaded modules. Java's LinkedHashMap keeps the first position of each load
    // string and the last module put for it (dups ok).
    let mut load_map = LoadMap::new();
    let mut order: IndexMap<String, Arc<BazelModuleContext>> = IndexMap::new();
    for (load, value) in program_loads.iter().zip(bzl_loads.iter()) {
        load_map.insert(load.module.clone(), value.get_module().clone());
        order.insert(load.module.clone(), value.get_module_context().clone());
    }
    Ok(Ok(Some((load_map, order.into_values().collect()))))
}

/// Loads the prelude identified by the label. Returns `None` for a Skyframe restart.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:742 PackageFunction.loadPrelude
fn load_prelude(
    env: &mut Environment<'_>,
    package_id: &PackageIdentifier,
    label: Label,
) -> Result<Result<Option<FrozenModule>, NoSuchPackageException>, Interrupted> {
    let keys = [BzlLoadKey::key_for_build_prelude(label)];
    match compute_bzl_loads_no_inlining(env, &keys)? {
        Ok(None) => Ok(Ok(None)), // skyframe restart
        // No need to validate visibility since we're processing an internal load on behalf of
        // Bazel.
        Ok(Some(loads)) => Ok(Ok(Some(loads[0].get_module().clone()))),
        Err(e) => Ok(Err(PackageFunctionExceptionBuilder::new(
            ExceptionType::BuildFileContainsErrors,
            package_id,
            Transience::Persistent,
            e.get_message(),
            PackageLoadingCode::ImportStarlarkFileError,
        )
        .build_cause())),
    }
}

/// For each of a package builder's targets, propagate the target's corresponding
/// inconsistent-filesystem exception (if any) and verify that the target's label does not cross
/// subpackage boundaries.
// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:883 PackageFunction.handleLabelsCrossingSubpackagesAndPropagateInconsistentFilesystemExceptions
fn handle_labels_crossing_subpackages_and_propagate_inconsistent_filesystem_exceptions(
    pkg_root: &Root,
    pkg_id: &PackageIdentifier,
    pkg_builder: &mut TargetDefinitionContext,
    env: &mut Environment<'_>,
) -> Result<Result<(), InternalInconsistentFilesystemException>, Interrupted> {
    let pkg_dir = pkg_id.get_package_fragment();
    // Contains a key for each package whose label that might have a presence of a subpackage.
    // Values are all potential subpackages of the label.
    let mut targets_and_subpackage_package_lookup_keys: Vec<(
        String,
        Label,
        Location,
        Vec<PackageLookupKey>,
    )> = Vec::new();
    let mut all_package_lookup_keys: IndexSet<PackageLookupKey> = IndexSet::new();
    for target in pkg_builder.get_targets() {
        let label = target.get_label();
        let dir = Label::get_containing_directory(label);
        if dir == *pkg_dir {
            continue;
        }
        let mut subpackage_package_lookup_keys = Vec::new();
        let label_name = label.get_name();
        let label_as_relative_path = PathFragment::create(label_name)
            .get_parent_directory()
            .unwrap_or_else(PathFragment::empty_fragment);
        let mut subpackage_path = pkg_dir.clone();
        for segment in label_as_relative_path.segments() {
            // Please note that the order from the shallowest path to the deepest is preserved.
            subpackage_path = subpackage_path.get_relative_str(segment);
            let current_package_lookup_key = PackageLookupKey::key(PackageIdentifier::create(
                pkg_id.get_repository().clone(),
                subpackage_path.clone(),
            ));
            subpackage_package_lookup_keys.push(current_package_lookup_key.clone());
            all_package_lookup_keys.insert(current_package_lookup_key);
        }
        targets_and_subpackage_package_lookup_keys.push((
            label_name.to_owned(),
            label.clone(),
            target.get_location(),
            subpackage_package_lookup_keys,
        ));
    }

    if targets_and_subpackage_package_lookup_keys.is_empty() {
        return Ok(Ok(()));
    }

    let sky_keys: Vec<SkyKey> = all_package_lookup_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let package_lookup_results = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(Ok(()));
    }

    for (target_name, target_label, target_location, target_package_lookup_keys) in
        targets_and_subpackage_package_lookup_keys
    {
        // Iterate from the deepest potential subpackage to the shallowest in that we only want to
        // display the deepest subpackage in the error message for each target.
        for package_lookup_key in target_package_lookup_keys.iter().rev() {
            enum Caught {
                BuildFileNotFound(NoSuchPackageException),
                InconsistentFilesystem(IOException),
            }
            let package_lookup_value =
                match package_lookup_results.get_or_throw_with(package_lookup_key, |e| {
                    if let Some(e) = e
                        .downcast_ref::<NoSuchPackageException>()
                        .filter(|e| e.kind() == NoSuchPackageKind::BuildFileNotFound)
                    {
                        Some(Caught::BuildFileNotFound(e.clone()))
                    } else {
                        e.downcast_ref::<IOException>()
                            .filter(|e| e.is_inconsistent_filesystem())
                            .map(|e| Caught::InconsistentFilesystem(e.clone()))
                    }
                }) {
                    Err(Caught::BuildFileNotFound(e)) => {
                        env.listener().handle(&Event::error(e.get_message()));
                        None
                    }
                    Err(Caught::InconsistentFilesystem(e)) => {
                        return Ok(Err(InternalInconsistentFilesystemException::of(pkg_id, &e)));
                    }
                    Ok(v) => v,
                };

            if maybe_add_event_about_label_crossing_subpackage(
                pkg_builder,
                pkg_root,
                &target_label,
                &target_location,
                &package_lookup_key.0,
                package_lookup_value.as_deref(),
            ) {
                pkg_builder.remove_target(&target_name);
                pkg_builder.set_contains_errors();
                break;
            }
        }
    }
    Ok(Ok(()))
}

// Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:962 PackageFunction.maybeAddEventAboutLabelCrossingSubpackage
fn maybe_add_event_about_label_crossing_subpackage(
    pkg_builder: &TargetDefinitionContext,
    pkg_root: &Root,
    target_label: &Label,
    target_location: &Location,
    subpackage_identifier: &PackageIdentifier,
    package_lookup_value: Option<&PackageLookupValue>,
) -> bool {
    let Some(package_lookup_value) = package_lookup_value else {
        return true;
    };
    match PackageLookupValue::get_error_message_for_label_crossing_package_boundary(
        pkg_root,
        target_label,
        subpackage_identifier,
        package_lookup_value,
    ) {
        Some(err_msg) => {
            let error = Package::error(
                Some(target_location),
                &err_msg,
                PackageLoadingCode::LabelCrossesPackageBoundary,
            );
            pkg_builder.get_local_event_handler().handle(&error);
            true
        }
        None => false,
    }
}

/// Bazel's `--default_visibility` default (`private`).
// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:107 PackageOptions.defaultVisibility
pub fn default_visibility() -> RuleVisibility {
    RuleVisibility::Private
}

/// Java `SkyframeExecutor.preparePackageLoading`'s policy for the defaults of
/// `--incompatible_enforce_config_setting_visibility` (true) and
/// `--incompatible_config_setting_private_default_visibility` (false).
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1713 SkyframeExecutor.preparePackageLoading
pub fn config_setting_visibility_policy(
    enforce_config_setting_visibility: bool,
    config_setting_private_default_visibility: bool,
) -> ConfigSettingVisibilityPolicy {
    if !enforce_config_setting_visibility {
        ConfigSettingVisibilityPolicy::LegacyOff
    } else if config_setting_private_default_visibility {
        ConfigSettingVisibilityPolicy::DefaultStandard
    } else {
        ConfigSettingVisibilityPolicy::DefaultPublic
    }
}
