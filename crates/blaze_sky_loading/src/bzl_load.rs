//! BZL_LOAD: loading (executing) a .bzl or .scl module.
//!
//! Java: com/google/devtools/build/lib/skyframe/{BzlLoadFunction,BzlLoadValue,
//! BzlLoadFailedException}.java.
//!
//! PORT: Java's bzl inlining mode (`computeInline`, `InliningState`, `CachedBzlLoadData`) is used
//! only when PACKAGE inlines its loads, which Bazel does not configure by default; the ordinary
//! Skyframe path is ported. As in Bazel's default configuration, BzlCompileFunction is inlined
//! (no BZL_COMPILE nodes; `InliningAndCachingGetter`).

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::label_constants::EXTERNAL_PACKAGE_IDENTIFIER;
use blaze_cmdline::repository_name::{BAZEL_TOOLS, MAIN};
use blaze_cmdline::{
    Label, LabelPackageContext, LabelRepoContextImpl, LabelRepoMappingRecorder,
    LabelSimpleRepoMappingRecorder, LabelSyntaxException, PackageIdentifier, RepositoryMapping,
    RepositoryName,
};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::builtins::BazelStarlarkEnvironment;
use blaze_packages::bzl_context::{BazelModuleContext, BzlInitThreadContext, BzlVisibility};
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, SkyKey, Transience,
};
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{CompiledFile, EvalOptions};
use blaze_starlark::loader::{LoadMap, LoadRequest};
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::{EvalError, Location, Phase, StarlarkSemantics, ThreadContext};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, PathFragment, Root};
use parking_lot::Mutex;
use starlark::environment::{FrozenModule, Globals};

use crate::bzl_compile::{BzlCompileFunction, BzlCompileKey, BzlCompileValue, FailedIOException};
use crate::containing_package_lookup::{ContainingPackageLookupKey, ContainingPackageLookupValue};
use crate::directories::LoadingDirectories;
use crate::exceptions::{DetailedExitCode, FailureCode, NoSuchPackageException, NoSuchPackageKind};
use crate::io::IOException;
use crate::precomputed::{AUTOLOAD_SYMBOLS, STARLARK_SEMANTICS};
use crate::repos::{RepositoryMappingKey, is_builtins_repo};
use crate::sky_functions;
use crate::starlark_builtins::{
    BuiltinsFailedException, StarlarkBuiltinsKey, StarlarkBuiltinsValue,
};
use crate::starlark_util;

// ---------------------------------------------------------------------------------------------
// Keys and values

/// SkyKey for a Starlark load. The label is absolute and should not reference the special
/// `external` package. `Debug` prints Java's `toString`, e.g.
/// `KeyForBuild{label=//pkg:foo.bzl, isBuildPrelude=false}`.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:107 BzlLoadValue.Key
#[derive(Clone, PartialEq, Eq)]
pub enum BzlLoadKey {
    /// A key for loading a .bzl during package loading (BUILD evaluation).
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:206 BzlLoadValue.KeyForBuild
    ForBuild {
        label: Label,
        is_build_prelude: bool,
    },
    /// A key for loading a .bzl during `@_builtins` evaluation. The label's repository is
    /// `@_builtins`.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:258 BzlLoadValue.KeyForBuiltins
    ForBuiltins(Label),
    /// A key for loading a .bzl to get the repo rule required by Bzlmod generated repositories.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:290 BzlLoadValue.KeyForBzlmod
    ForBzlmod(Label),
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:316 BzlLoadValue.KeyForBzlmodBootstrap
    ForBzlmodBootstrap(Label),
}

impl BzlLoadKey {
    /// Constructs a key for loading a regular .bzl file from BUILD files.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:330 BzlLoadValue.keyForBuild
    pub fn key_for_build(label: Label) -> Self {
        BzlLoadKey::ForBuild {
            label,
            is_build_prelude: false,
        }
    }

    /// Constructs a key for loading a .bzl file within the `@_builtins` pseudo-repository.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:335 BzlLoadValue.keyForBuiltins
    pub fn key_for_builtins(label: Label) -> Self {
        // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:262 BzlLoadValue.KeyForBuiltins.<init>
        blaze_util::check_argument!(
            is_builtins_repo(label.get_repository()),
            "repository name for builtins key must be '@_builtins'"
        );
        BzlLoadKey::ForBuiltins(label)
    }

    /// Constructs a key for loading the special prelude .bzl.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:340 BzlLoadValue.keyForBuildPrelude
    pub fn key_for_build_prelude(label: Label) -> Self {
        BzlLoadKey::ForBuild {
            label,
            is_build_prelude: true,
        }
    }

    /// Constructs a key for loading a .bzl for Bzlmod repos.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:345 BzlLoadValue.keyForBzlmod
    pub fn key_for_bzlmod(label: Label) -> Self {
        BzlLoadKey::ForBzlmod(label)
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:349 BzlLoadValue.keyForBzlmodBootstrap
    pub fn key_for_bzlmod_bootstrap(label: Label) -> Self {
        blaze_util::check_argument!(
            *label.get_repository() == *BAZEL_TOOLS,
            "keyForBzlmodBootstrap must be called with a label in the bazel_tools repository"
        );
        BzlLoadKey::ForBzlmodBootstrap(label)
    }

    /// Returns the absolute label of the .bzl file to be loaded.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:122 BzlLoadValue.Key.getLabel
    pub fn get_label(&self) -> &Label {
        match self {
            BzlLoadKey::ForBuild { label, .. }
            | BzlLoadKey::ForBuiltins(label)
            | BzlLoadKey::ForBzlmod(label)
            | BzlLoadKey::ForBzlmodBootstrap(label) => label,
        }
    }

    /// Returns true if this is a request for the special BUILD prelude file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:125 BzlLoadValue.Key.isBuildPrelude
    pub fn is_build_prelude(&self) -> bool {
        matches!(
            self,
            BzlLoadKey::ForBuild {
                is_build_prelude: true,
                ..
            }
        )
    }

    /// Returns true if this is a request for a builtins bzl file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:130 BzlLoadValue.Key.isBuiltins
    pub fn is_builtins(&self) -> bool {
        matches!(self, BzlLoadKey::ForBuiltins(_))
    }

    /// Returns true if the requested file follows the .scl dialect.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:146 BzlLoadValue.Key.isSclDialect
    pub fn is_scl_dialect(&self) -> bool {
        self.get_label().get_name().ends_with(".scl")
    }

    /// Constructs a new key suitable for evaluating a `load()` dependency of this key's .bzl
    /// file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:158 BzlLoadValue.Key.getKeyForLoad
    pub fn get_key_for_load(&self, load_label: Label) -> BzlLoadKey {
        match self {
            // Note that the returned key always has !isBuildPrelude. I.e., if the prelude file
            // loads another .bzl, the loaded .bzl is processed as normal with no special prelude
            // magic.
            BzlLoadKey::ForBuild { .. } => Self::key_for_build(load_label),
            BzlLoadKey::ForBuiltins(_) => Self::key_for_builtins(load_label),
            BzlLoadKey::ForBzlmod(_) => Self::key_for_bzlmod(load_label),
            BzlLoadKey::ForBzlmodBootstrap(_) => Self::key_for_bzlmod_bootstrap(load_label),
        }
    }

    /// Constructs a BzlCompileValue key suitable for retrieving the Starlark code for this .bzl,
    /// given the Root in which to find its file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:164 BzlLoadValue.Key.getCompileKey
    pub fn get_compile_key(&self, root: Root) -> BzlCompileKey {
        match self {
            BzlLoadKey::ForBuild {
                label,
                is_build_prelude,
            } => {
                if *is_build_prelude {
                    BzlCompileKey::key_for_build_prelude(root, label.clone())
                } else {
                    BzlCompileKey::key(root, label.clone())
                }
            }
            BzlLoadKey::ForBuiltins(label) => BzlCompileKey::key_for_builtins(root, label.clone()),
            BzlLoadKey::ForBzlmod(label) | BzlLoadKey::ForBzlmodBootstrap(label) => {
                BzlCompileKey::key(root, label.clone())
            }
        }
    }

    fn class_name(&self) -> &'static str {
        match self {
            BzlLoadKey::ForBuild { .. } => "KeyForBuild",
            BzlLoadKey::ForBuiltins(_) => "KeyForBuiltins",
            BzlLoadKey::ForBzlmod(_) => "KeyForBzlmod",
            BzlLoadKey::ForBzlmodBootstrap(_) => "KeyForBzlmodBootstrap",
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:191 BzlLoadValue.Key.hashCode
impl Hash for BzlLoadKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.class_name().hash(state);
        self.get_label().hash(state);
        self.is_build_prelude().hash(state);
        self.is_builtins().hash(state);
    }
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:198 BzlLoadValue.Key.toString
impl fmt::Debug for BzlLoadKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{{label={}, isBuildPrelude={}}}",
            self.class_name(),
            self.get_label(),
            self.is_build_prelude()
        )
    }
}

impl Key for BzlLoadKey {
    type Value = BzlLoadValue;
    const FUNCTION: SkyFunctionName = sky_functions::BZL_LOAD;
}

/// A value that represents the .bzl (or .scl) module loaded by a Starlark `load()` statement.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:53 BzlLoadValue
#[derive(Clone)]
pub struct BzlLoadValue {
    /// .bzl module (and indirectly, the entire load DAG).
    module: FrozenModule,
    /// Of .bzl file and load dependencies.
    transitive_digest: Vec<u8>,
    bzl_visibility: BzlVisibility,
    recorded_repo_mappings: Vec<(RepositoryName, String, RepositoryName)>,
    /// The module's context (Java: the Module's client data).
    module_context: Arc<BazelModuleContext>,
}

impl fmt::Debug for BzlLoadValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BzlLoadValue{{label={}, transitiveDigest={}}}",
            self.module_context.label,
            hex::encode(&self.transitive_digest)
        )
    }
}

impl BzlLoadValue {
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:61 BzlLoadValue.<init>
    pub fn new(
        module: FrozenModule,
        transitive_digest: Vec<u8>,
        bzl_visibility: BzlVisibility,
        recorded_repo_mappings: Vec<(RepositoryName, String, RepositoryName)>,
        module_context: Arc<BazelModuleContext>,
    ) -> Self {
        BzlLoadValue {
            module,
            transitive_digest,
            bzl_visibility,
            recorded_repo_mappings,
            module_context,
        }
    }

    /// Returns the .bzl module.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:73 BzlLoadValue.getModule
    pub fn get_module(&self) -> &FrozenModule {
        &self.module
    }

    /// Returns the digest of the .bzl module and its transitive load dependencies.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:78 BzlLoadValue.getTransitiveDigest
    pub fn get_transitive_digest(&self) -> &[u8] {
        &self.transitive_digest
    }

    /// Returns the visibility of this module for the purpose of `load()` statements.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:83 BzlLoadValue.getBzlVisibility
    pub fn get_bzl_visibility(&self) -> &BzlVisibility {
        &self.bzl_visibility
    }

    /// Returns the repo mapping entries used to load this bzl file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:91 BzlLoadValue.getRecordedRepoMappings
    pub fn get_recorded_repo_mappings(&self) -> &[(RepositoryName, String, RepositoryName)] {
        &self.recorded_repo_mappings
    }

    /// The module's `BazelModuleContext` (Java `BazelModuleContext.of(getModule())`).
    pub fn get_module_context(&self) -> &Arc<BazelModuleContext> {
        &self.module_context
    }
}

/// The `BazelModuleContext` as a Starlark value (moved to blaze_packages `bzl_context`, where
/// the natives read it back from the calling function's module).
pub use blaze_packages::bzl_context::BazelModuleContextValue;

// ---------------------------------------------------------------------------------------------
// Exceptions

/// `FailureDetails.StarlarkLoading.Code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarlarkLoadingCode {
    Cycle,
    CompileError,
    ParseError,
    EvalError,
    ContainingPackageNotFound,
    PackageNotFound,
    IoError,
    LabelCrossesPackageBoundary,
    BuiltinsError,
    VisibilityError,
}

impl StarlarkLoadingCode {
    fn name(self) -> &'static str {
        match self {
            StarlarkLoadingCode::Cycle => "CYCLE",
            StarlarkLoadingCode::CompileError => "COMPILE_ERROR",
            StarlarkLoadingCode::ParseError => "PARSE_ERROR",
            StarlarkLoadingCode::EvalError => "EVAL_ERROR",
            StarlarkLoadingCode::ContainingPackageNotFound => "CONTAINING_PACKAGE_NOT_FOUND",
            StarlarkLoadingCode::PackageNotFound => "PACKAGE_NOT_FOUND",
            StarlarkLoadingCode::IoError => "IO_ERROR",
            StarlarkLoadingCode::LabelCrossesPackageBoundary => "LABEL_CROSSES_PACKAGE_BOUNDARY",
            StarlarkLoadingCode::BuiltinsError => "BUILTINS_ERROR",
            StarlarkLoadingCode::VisibilityError => "VISIBILITY_ERROR",
        }
    }
}

/// Failure of BZL_LOAD.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:23 BzlLoadFailedException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BzlLoadFailedException {
    message: String,
    detailed_exit_code: DetailedExitCode,
    transience: Transience,
}

impl BzlLoadFailedException {
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:48 BzlLoadFailedException.<init>
    pub fn new(message: impl Into<String>, code: StarlarkLoadingCode) -> Self {
        let message = message.into();
        BzlLoadFailedException {
            detailed_exit_code: Self::create_detailed_exit_code(&message, code),
            message,
            transience: Transience::Persistent,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:38 BzlLoadFailedException.<init>
    pub fn with_detailed_exit_code(
        message: impl Into<String>,
        detailed_exit_code: DetailedExitCode,
        transience: Transience,
    ) -> Self {
        BzlLoadFailedException {
            message: message.into(),
            detailed_exit_code,
            transience,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:56 BzlLoadFailedException.getTransience
    pub fn get_transience(&self) -> Transience {
        self.transience
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:61 BzlLoadFailedException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFailedException.java:65 BzlLoadFailedException.createDetailedExitCode
    pub fn create_detailed_exit_code(message: &str, code: StarlarkLoadingCode) -> DetailedExitCode {
        DetailedExitCode {
            message: message.to_owned(),
            code: FailureCode::Other(format!("StarlarkLoading.{}", code.name())),
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1564 BzlLoadFunction.whileLoadingDep
fn while_loading_dep(loc: &Location, cause: &BzlLoadFailedException) -> BzlLoadFailedException {
    // Don't chain exception cause, just incorporate the message with a prefix.
    BzlLoadFailedException::with_detailed_exit_code(
        format!("at {}: {}", loc, cause.get_message()),
        cause.get_detailed_exit_code().clone(),
        Transience::Persistent,
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1573 BzlLoadFunction.executionFailed
pub fn execution_failed(label: &Label) -> BzlLoadFailedException {
    BzlLoadFailedException::new(
        format!(
            "initialization of module '{}'{} failed",
            // TODO(brandjon): This error message drops the repo part of the label.
            label.to_path_fragment(),
            if is_builtins_repo(label.get_repository()) {
                " (internal)"
            } else {
                ""
            }
        ),
        StarlarkLoadingCode::EvalError,
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1583 BzlLoadFunction.errorFindingContainingPackage
fn error_finding_containing_package(
    file: &PathFragment,
    cause_message: &str,
    cause_detailed_exit_code: Option<DetailedExitCode>,
) -> BzlLoadFailedException {
    let error_message = format!(
        "Encountered error while reading extension file '{}': {}",
        file, cause_message
    );
    let detailed_exit_code = cause_detailed_exit_code.unwrap_or_else(|| {
        BzlLoadFailedException::create_detailed_exit_code(
            &error_message,
            StarlarkLoadingCode::ContainingPackageNotFound,
        )
    });
    BzlLoadFailedException::with_detailed_exit_code(
        error_message,
        detailed_exit_code,
        Transience::Persistent,
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1595 BzlLoadFunction.errorReadingBzl
fn error_reading_bzl(file: &PathFragment, cause: &FailedIOException) -> BzlLoadFailedException {
    let error_message = format!(
        "Encountered error while reading extension file '{}': {}",
        file, cause
    );
    // PORT: DetailedIOException causes are never produced by Blaze's file systems.
    BzlLoadFailedException::with_detailed_exit_code(
        error_message.clone(),
        BzlLoadFailedException::create_detailed_exit_code(
            &error_message,
            StarlarkLoadingCode::IoError,
        ),
        cause.get_transience(),
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1611 BzlLoadFunction.noBuildFile
fn no_build_file(file: &Label, reason: Option<&str>) -> BzlLoadFailedException {
    if let Some(reason) = reason {
        return BzlLoadFailedException::new(
            format!("Unable to find package for {}: {}.", file, reason),
            StarlarkLoadingCode::PackageNotFound,
        );
    }
    BzlLoadFailedException::new(
        format!(
            "Every .bzl file must have a corresponding package, but '{}' does not have one. \
             Please create a BUILD file in the same or any parent directory. Note that this \
             BUILD file does not need to do anything except exist.",
            file
        ),
        StarlarkLoadingCode::PackageNotFound,
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1627 BzlLoadFunction.labelCrossesPackageBoundary
fn label_crosses_package_boundary(
    label: &Label,
    containing_package_lookup_value: &ContainingPackageLookupValue,
) -> BzlLoadFailedException {
    BzlLoadFailedException::new(
        ContainingPackageLookupValue::get_error_message_for_label_crossing_package_boundary(
            // We don't actually know the proper Root to pass in here (since we don't e.g. know
            // the root of the bzl/BUILD file that is trying to load 'label'). Therefore we just
            // pass in the Root of the containing package in order to still get a useful error
            // message for the user.
            containing_package_lookup_value.get_containing_package_root(),
            label,
            containing_package_lookup_value,
        ),
        StarlarkLoadingCode::LabelCrossesPackageBoundary,
    )
}

// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1642 BzlLoadFunction.builtinsFailed
fn builtins_failed(file: &Label, cause: &BuiltinsFailedException) -> BzlLoadFailedException {
    let message = format!(
        "Internal error while loading Starlark builtins for {}: {}",
        file,
        cause.get_message()
    );
    BzlLoadFailedException::with_detailed_exit_code(
        message.clone(),
        BzlLoadFailedException::create_detailed_exit_code(
            &message,
            StarlarkLoadingCode::BuiltinsError,
        ),
        cause.get_transience(),
    )
}

/// Returns an exception for load visibility violations. `file_description` is a string like
/// `"module //pkg:foo.bzl"` or `"file //pkg:BUILD"`.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1658 BzlLoadFunction.visibilityViolation
pub fn visibility_violation(file_description: &str) -> BzlLoadFailedException {
    BzlLoadFailedException::new(
        format!(
            "{} contains .bzl load visibility violations",
            file_description
        ),
        StarlarkLoadingCode::VisibilityError,
    )
}

// ---------------------------------------------------------------------------------------------
// Load label validation (also used by PACKAGE)

/// Validates a label appearing in a `load()` statement.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1001 BzlLoadFunction.checkValidLoadLabel
fn check_valid_load_label_internal(
    label: &Label,
    from_builtins_repo: bool,
    within_scl_dialect: bool,
    mention_scl_in_error_message: bool,
) -> Result<(), LabelSyntaxException> {
    // Check file extension.
    let base_name = label.get_name();
    if within_scl_dialect {
        if !base_name.ends_with(".scl") {
            let mut msg = "The label must reference a file with extension \".scl\"".to_owned();
            if base_name.ends_with(".bzl") {
                msg.push_str(" (.scl files cannot load .bzl files)");
            }
            return Err(LabelSyntaxException::new(msg));
        }
    } else if !(base_name.ends_with(".scl") || base_name.ends_with(".bzl")) {
        let mut msg = "The label must reference a file with extension \".bzl\"".to_owned();
        if mention_scl_in_error_message {
            msg.push_str(" or \".scl\"");
        }
        return Err(LabelSyntaxException::new(msg));
    }

    if *label.get_package_identifier() == *EXTERNAL_PACKAGE_IDENTIFIER {
        return Err(LabelSyntaxException::new(
            "Starlark files may not be loaded from the //external package",
        ));
    }
    if from_builtins_repo && !is_builtins_repo(label.get_repository()) {
        return Err(LabelSyntaxException::new(
            ".bzl files in @_builtins cannot load from outside of @_builtins",
        ));
    }
    Ok(())
}

/// Validates a label appearing in a `load()` statement (does not enforce restrictions on
/// loading experimental .bzls).
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1043 BzlLoadFunction.checkValidLoadLabel
pub fn check_valid_load_label(
    label: &Label,
    starlark_semantics: &StarlarkSemantics,
) -> Result<(), LabelSyntaxException> {
    check_valid_load_label_internal(
        label,
        false,
        false,
        starlark_semantics.experimental_enable_scl_dialect,
    )
}

/// Given a list of `load("module")` strings and their locations, in source order, returns a
/// corresponding list of Labels they each resolve to. Labels are resolved relative to `base`,
/// the file's package. If any label is malformed, the function reports one or more errors to the
/// handler and returns `None`.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1073 BzlLoadFunction.getLoadLabels
pub fn get_load_labels(
    handler: &dyn ExtendedEventHandler,
    loads: &[LoadRequest],
    base: &PackageIdentifier,
    is_under_experimental: &dyn Fn(&PackageIdentifier) -> bool,
    allow_experimental_loads: bool,
    repo_mapping: &RepositoryMapping,
    within_scl_dialect: bool,
    is_scl_flag_enabled: bool,
    mut repo_mapping_recorder: Option<&mut dyn LabelRepoMappingRecorder>,
) -> Option<Vec<Label>> {
    let mut ok = true;
    let base_within_experimental = is_under_experimental(base);
    let mut load_labels = Vec::with_capacity(loads.len());
    for load in loads {
        // Parse the load statement's module string as a label. Validate the unparsed string for
        // syntax and the parsed label for structure.
        let result: Result<Label, LabelSyntaxException> = 'parse: {
            let mut unparsed_label = load.module.as_str();
            if within_scl_dialect {
                if !unparsed_label.starts_with("//") {
                    break 'parse Err(LabelSyntaxException::new(
                        "in .scl files, load labels must begin with \"//\"",
                    ));
                }
                // Map the magic label "//:project_proto.scl" to the corresponding label in
                // bazel_tools, since .scl doesn't support @repo syntax.
                if unparsed_label == "//:project_proto.scl" {
                    unparsed_label = "@bazel_tools//src/main/protobuf/project:project_proto.scl";
                }
            }
            let recorder: Option<&mut dyn LabelRepoMappingRecorder> =
                match &mut repo_mapping_recorder {
                    Some(r) => Some(&mut **r),
                    None => None,
                };
            let label = match Label::parse_with_package_context_recording(
                unparsed_label,
                &LabelPackageContext::of(base, repo_mapping.clone()),
                recorder,
            ) {
                Ok(l) => l,
                Err(e) => break 'parse Err(e),
            };
            if let Err(e) = check_valid_load_label_internal(
                &label,
                is_builtins_repo(base.get_repository()),
                within_scl_dialect,
                is_scl_flag_enabled,
            ) {
                break 'parse Err(e);
            }
            if !allow_experimental_loads
                && !base_within_experimental
                && is_under_experimental(label.get_package_identifier())
            {
                break 'parse Err(LabelSyntaxException::new(
                    "Cannot load an experimental Starlark file from a non-experimental package.\n\
                     Consider moving the loaded file to a non-experimental package.\n\
                     To temporarily bypass this error, use --allow_experimental_loads.\n",
                ));
            }
            Ok(label)
        };
        match result {
            Ok(label) => load_labels.push(label),
            Err(ex) => {
                handler.handle(&Event::error_at(
                    Some(starlark_util::event_location(&load.location)),
                    format!("in load statement: {}", ex.get_message()),
                ));
                ok = false;
            }
        }
    }
    if ok { Some(load_labels) } else { None }
}

/// Java `ConfiguredRuleClassProvider.isPackageUnderExperimental` for Bazel
/// (`BazelPrerequisiteValidator.packageUnderExperimental`): never.
// Java: com/google/devtools/build/lib/bazel/rules/BazelPrerequisiteValidator.java:30 BazelPrerequisiteValidator.packageUnderExperimental
pub fn is_package_under_experimental(_package_identifier: &PackageIdentifier) -> bool {
    false
}

/// Checks that all (directly) requested loads are visible to the requesting file's package.
/// Each load that is not visible is reported as an error on the event handler. If there is at
/// least one error, [`BzlLoadFailedException`] is returned.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1252 BzlLoadFunction.checkLoadVisibilities
pub fn check_load_visibilities(
    requesting_package: &PackageIdentifier,
    requesting_file_description: &str,
    load_values: &[Arc<BzlLoadValue>],
    load_keys: &[BzlLoadKey],
    program_loads: &[LoadRequest],
    demote_errors_to_warnings: bool,
    handler: &dyn ExtendedEventHandler,
) -> Result<(), BzlLoadFailedException> {
    let mut found_violation = false;
    for i in 0..load_values.len() {
        let load_visibility = load_values[i].get_bzl_visibility();
        let load_label = load_keys[i].get_label();
        let load_package = load_label.get_package_identifier();
        if !(requesting_package == load_package
            || load_visibility.allows_package(requesting_package))
        {
            let loc = starlark_util::event_location(&program_loads[i].location);
            let mut msg = format!(
                "Starlark file {} is not visible for loading from package {}. Check the file's \
                 `visibility()` declaration.",
                load_label,
                requesting_package.get_canonical_form()
            );
            if demote_errors_to_warnings {
                msg.push_str(" Continuing because --nocheck_bzl_visibility is active");
                handler.handle(&Event::warn_at(Some(loc), msg));
            } else {
                handler.handle(&Event::error_at(Some(loc), msg));
            }
            found_violation = true;
        }
    }
    if found_violation && !demote_errors_to_warnings {
        return Err(visibility_violation(requesting_file_description));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Execution

/// Keeps every frozen .bzl heap alive for the life of the process: rule classes and other
/// values hold `FrozenValue` handles into them (DESIGN.md). PORT: keep-alive.
static KEEP_ALIVE: Mutex<Vec<FrozenModule>> = Mutex::new(Vec::new());

/// `blaze_starlark::eval::exec_module`, plus the module's extra value (its
/// `BazelModuleContext`), which must be set before freezing.
fn exec_module_with_context(
    compiled: CompiledFile,
    globals: &Globals,
    loads: &LoadMap,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    module_context: Arc<BazelModuleContext>,
) -> Result<FrozenModule, EvalError> {
    blaze_starlark::eval::exec_module_with_extra_value(
        compiled,
        globals,
        loads,
        ctx,
        opts,
        move |module| {
            module
                .heap()
                .alloc(BazelModuleContextValue(module_context.clone()))
        },
    )
}

/// What BZL_LOAD needs from the rule class provider and the server.
pub struct BzlLoadConfig {
    pub bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
    pub directories: LoadingDirectories,
    /// Java `ruleClassProvider.getToolsRepository()`.
    pub tools_repository: RepositoryName,
    /// Java `ruleClassProvider.getConfigurationFragmentMap().keySet()`.
    pub fragment_names: Vec<String>,
    /// The file system's digest function (for the inlined BzlCompileFunction).
    pub hash_function: DigestHashFunction,
}

/// A Skyframe function to look up and load a single .bzl (or .scl) module.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:109 BzlLoadFunction
pub struct BzlLoadFunction {
    config: Arc<BzlLoadConfig>,
    /// Java `InliningAndCachingGetter.bzlCompileCache`.
    bzl_compile_cache: Mutex<HashMap<BzlCompileKey, BzlCompileValue>>,
}

/// Bazel's builtins package path in its source tree (Java
/// `ruleClassProvider.getBuiltinsBzlPackagePathInSource()`).
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:216 BazelRuleClassProvider
pub const BUILTINS_BZL_PACKAGE_PATH_IN_SOURCE: &str = "src/main/starlark/builtins_bzl";

impl BzlLoadFunction {
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:140 BzlLoadFunction.create
    pub fn create(config: Arc<BzlLoadConfig>) -> Self {
        BzlLoadFunction {
            config,
            bzl_compile_cache: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:203 BzlLoadFunction.compute
    pub fn compute(
        &self,
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<BzlLoadValue>, ComputeError> {
        match self.compute_internal(key, env)? {
            Ok(v) => Ok(v),
            Err(e) => {
                let t = e.get_transience();
                Err(SkyFunctionException::new(e, t).into())
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:549 BzlLoadFunction.computeInternal
    fn compute_internal(
        &self,
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
    ) -> Result<Result<Option<BzlLoadValue>, BzlLoadFailedException>, ComputeError> {
        let label = key.get_label();
        let file_path = label.to_path_fragment();

        let builtins = match self.get_builtins(key, env)? {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };

        let compile_key = match self.validate_package_and_get_compile_key(
            key,
            env,
            &builtins.starlark_semantics.experimental_builtins_bzl_path,
        )? {
            Ok(Some(k)) => k,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };
        let compile_value = match self.get_bzl_compile_value(&compile_key, env)? {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(error_reading_bzl(&file_path, &e))),
        };

        // Release the compiled bzl iff the value gets completely evaluated (to either error or
        // non-null result).
        let result = self.compute_internal_with_compiled_bzl(key, &compile_value, &builtins, env);
        let completed = !matches!(result, Ok(Ok(None)));
        if completed {
            // Java: InliningAndCachingGetter.doneWithBzlCompileValue
            self.bzl_compile_cache.lock().remove(&compile_key);
        }
        result
    }

    /// Obtain a suitable StarlarkBuiltinsValue: a real builtins value for BUILD-loaded and most
    /// bzlmod-loaded .bzl files, else an empty value that just wraps the semantics.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:600 BzlLoadFunction.getBuiltins
    fn get_builtins(
        &self,
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
    ) -> Result<Result<Option<Arc<StarlarkBuiltinsValue>>, BzlLoadFailedException>, ComputeError>
    {
        if !Self::requires_builtins_injection(key) {
            let Some(starlark_semantics) = STARLARK_SEMANTICS.get(env)? else {
                return Ok(Ok(None));
            };
            return Ok(Ok(Some(Arc::new(StarlarkBuiltinsValue::create_empty(
                starlark_semantics,
            )))));
        }
        let Some(autoload_symbols) = AUTOLOAD_SYMBOLS.get(env)? else {
            return Ok(Ok(None));
        };
        let with_autoloads = Self::requires_autoloads(key, &autoload_symbols);
        match env.get_value_or_throw::<StarlarkBuiltinsKey, BuiltinsFailedException>(
            StarlarkBuiltinsKey::key(with_autoloads),
        )? {
            Ok(v) => Ok(Ok(v)),
            Err(e) => Ok(Err(builtins_failed(key.get_label(), &e))),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:637 BzlLoadFunction.requiresBuiltinsInjection
    fn requires_builtins_injection(key: &BzlLoadKey) -> bool {
        // https://github.com/bazelbuild/bazel/issues/17713
        // `@_builtins` depends on `@bazel_tools` for repo mapping, so we ignore some bzl files
        // to avoid a cyclic dependency
        matches!(key, BzlLoadKey::ForBuild { .. } | BzlLoadKey::ForBzlmod(_))
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:646 BzlLoadFunction.requiresAutoloads
    fn requires_autoloads(
        key: &BzlLoadKey,
        autoload_symbols: &blaze_packages::builtins::AutoloadSymbols,
    ) -> bool {
        // We do autoloads for all BUILD files and BUILD-loaded .bzl files, except for files in
        // certain rule repos (see AutoloadSymbols#reposDisallowingAutoloads).
        //
        // We don't do autoloads for the WORKSPACE file, Bzlmod files, or .bzls loaded by them,
        // because in general the rules repositories that we would load are not yet available.
        //
        // We never do autoloads for builtins bzls.
        //
        // We don't do autoloads for the prelude file, but that's a single file so users can
        // migrate it easily. (We do autoloads in .bzl files that are loaded by the prelude
        // file.)
        autoload_symbols.is_enabled()
            && matches!(key, BzlLoadKey::ForBuild { .. })
            && !key.is_build_prelude()
            && !autoload_symbols
                .autoloads_disabled_in_bzl_for_repo(Some(key.get_label().get_repository()))
    }

    /// Given a bzl key, validates that the corresponding package exists (if required) and
    /// returns the associated compile key based on the package's root. Returns `None` for a
    /// missing Skyframe dep or unspecified exception.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:681 BzlLoadFunction.validatePackageAndGetCompileKey
    fn validate_package_and_get_compile_key(
        &self,
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
        builtins_bzl_path: &str,
    ) -> Result<Result<Option<BzlCompileKey>, BzlLoadFailedException>, ComputeError> {
        let label = key.get_label();

        // Bypass package lookup entirely if builtins.
        if key.is_builtins() {
            if !label.get_package_name().is_empty() {
                return Ok(Err(no_build_file(
                    label,
                    Some("@_builtins cannot have subpackages"),
                )));
            }
            return Ok(Ok(Some(
                key.get_compile_key(self.get_builtins_root(builtins_bzl_path)),
            )));
        }

        // Do package lookup.
        let dir = Label::get_containing_directory(label);
        let dir_id = PackageIdentifier::create(label.get_repository().clone(), dir);
        let lookup_key = ContainingPackageLookupKey::key(dir_id);
        // Java: getValueOrThrow(key, BuildFileNotFoundException.class,
        // InconsistentFilesystemException.class)
        let package_lookup = match env.get_value_or_throw_with(lookup_key, |e| {
            if let Some(e) = e
                .downcast_ref::<NoSuchPackageException>()
                .filter(|e| e.kind() == NoSuchPackageKind::BuildFileNotFound)
            {
                Some(error_finding_containing_package(
                    &label.to_path_fragment(),
                    &e.get_message(),
                    Some(e.get_detailed_exit_code()),
                ))
            } else {
                // PORT: InconsistentFilesystemException is not a DetailedException.
                e.downcast_ref::<IOException>()
                    .filter(|e| e.is_inconsistent_filesystem())
                    .map(|e| {
                        error_finding_containing_package(
                            &label.to_path_fragment(),
                            e.get_message(),
                            None,
                        )
                    })
            }
        })? {
            Err(e) => return Ok(Err(e)),
            Ok(v) => v,
        };
        let Some(package_lookup) = package_lookup else {
            return Ok(Ok(None));
        };

        // Resolve to compile key or error.
        let package_ok = package_lookup.has_containing_package()
            && package_lookup.get_containing_package_name() == label.get_package_identifier();
        let compile_key = if key.is_build_prelude() && !package_ok {
            // Ignore the prelude, its package doesn't exist.
            BzlCompileKey::empty_prelude_key()
        } else if package_ok {
            key.get_compile_key(package_lookup.get_containing_package_root().clone())
        } else if !package_lookup.has_containing_package() {
            return Ok(Err(no_build_file(
                label,
                package_lookup.get_reason_for_no_containing_package(),
            )));
        } else {
            return Ok(Err(label_crosses_package_boundary(label, &package_lookup)));
        };
        Ok(Ok(Some(compile_key)))
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:733 BzlLoadFunction.getBuiltinsRoot
    fn get_builtins_root(&self, builtins_bzl_path: &str) -> Root {
        // TODO(#11437): Remove once injection can't be disabled.
        if builtins_bzl_path.is_empty() {
            panic!("IllegalStateException: Requested builtins root, but injection is disabled");
        }
        if builtins_bzl_path == "%bundled%" {
            crate::bundled_fs::bundled_builtins_root()
        } else if builtins_bzl_path == "%workspace%" {
            Root::from_path(
                self.config
                    .directories
                    .get_workspace()
                    .get_relative_str(BUILTINS_BZL_PACKAGE_PATH_IN_SOURCE),
            )
        } else {
            Root::from_path(
                self.config
                    .directories
                    .get_workspace()
                    .get_relative_str(builtins_bzl_path),
            )
        }
    }

    /// Java `InliningAndCachingGetter.getBzlCompileValue`.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1491 BzlLoadFunction.InliningAndCachingGetter.getBzlCompileValue
    fn get_bzl_compile_value(
        &self,
        key: &BzlCompileKey,
        env: &mut Environment<'_>,
    ) -> Result<Result<Option<BzlCompileValue>, FailedIOException>, ComputeError> {
        let cached = self.bzl_compile_cache.lock().get(key).cloned();
        match cached {
            None => {
                let value = match BzlCompileFunction::compute_inline(
                    key,
                    env,
                    self.config.hash_function,
                )? {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                };
                if let Some(v) = &value {
                    self.bzl_compile_cache.lock().insert(key.clone(), v.clone());
                }
                Ok(Ok(value))
            }
            Some(value) => {
                // The cache hit may have been populated on behalf of a different BzlLoadValue
                // node with the same compile key; make sure this node depends on the .bzl file
                // too.
                if let Some(bzl_file_key) = key.get_bzl_file_key() {
                    match env
                        .get_value_or_throw::<crate::file::FileKey, IOException>(bzl_file_key)?
                    {
                        Err(e) => {
                            return Ok(Err(FailedIOException::new(e, Transience::Persistent)));
                        }
                        Ok(None) => return Ok(Ok(None)),
                        Ok(Some(_)) => {}
                    }
                }
                Ok(Ok(Some(value)))
            }
        }
    }

    /// Compute logic for once the compiled .bzl has been fetched and confirmed to exist (though
    /// it may have Starlark errors).
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:770 BzlLoadFunction.computeInternalWithCompiledBzl
    fn compute_internal_with_compiled_bzl(
        &self,
        key: &BzlLoadKey,
        compile_value: &BzlCompileValue,
        builtins: &StarlarkBuiltinsValue,
        env: &mut Environment<'_>,
    ) -> Result<Result<Option<BzlLoadValue>, BzlLoadFailedException>, ComputeError> {
        // Ensure the .bzl exists and passes static checks (parsing, resolving).
        // (A missing prelude file still returns a valid but empty BzlCompileValue.)
        if !compile_value.lookup_successful() {
            // Java: BzlCompileFunction.java:234 Program.compileFile and :240
            // addSyntaxErrorsToListener: every static error of the file, with the predeclared
            // names (see `BzlCompileValue::Failure`'s `deferred`).
            if let Some(d) = compile_value.deferred() {
                let globals = self.get_and_digest_predeclared_environment(
                    key,
                    builtins,
                    &mut Fingerprint::new(),
                );
                let compile_env = self.compile_environment(key);
                let semantics = &builtins.starlark_semantics;
                let label = key.get_label();
                let names = globals.names().map(|n| n.as_str().to_owned()).collect();
                let guarded = |name: &str| {
                    blaze_packages::builtins::env::guarded_symbol_error(
                        compile_env,
                        name,
                        semantics,
                        Some(label),
                    )
                };
                if let Err(e) = blaze_starlark::eval::compile_and_resolve(
                    d.kind,
                    d.options,
                    &d.input_name,
                    d.source.clone(),
                    names,
                    &guarded,
                ) && let blaze_starlark::error::EvalErrorKind::Syntax(errors) = &e.kind
                {
                    BzlCompileFunction::add_syntax_errors_to_listener(
                        env.listener(),
                        errors,
                        &d.key,
                    );
                }
            }
            return Ok(Err(BzlLoadFailedException::new(
                compile_value.get_error(),
                StarlarkLoadingCode::CompileError,
            )));
        }
        let prog = compile_value.get_program();
        let label = key.get_label();
        let pkg = label.get_package_identifier();
        let semantics = &builtins.starlark_semantics;

        // Java resolves the file's names in BzlCompileFunction (`Program.compileFile`), before
        // any load is requested: every undefined name, and every use of a predeclared symbol
        // whose GuardedValue is not accessible (with the guard's message), is a syntax error
        // replayed on the listener, and the load fails with the compile error.
        // PORT: Blaze's BzlCompileFunction parses only (the predeclared `Globals` are built
        // here); the names, and so the errors, are the same ("the names are all we need to do
        // symbol resolution", BzlCompileFunction.java:157).
        // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:234 BzlCompileFunction.computeInline
        {
            let globals =
                self.get_and_digest_predeclared_environment(key, builtins, &mut Fingerprint::new());
            let compile_env = self.compile_environment(key);
            let errors = starlark_util::resolve_errors(prog, &globals, |name| {
                blaze_packages::builtins::env::guarded_symbol_error(
                    compile_env,
                    name,
                    semantics,
                    Some(label),
                )
            });
            if !errors.is_empty() {
                starlark_util::replay_syntax_errors_on(env.listener(), &errors);
                return Ok(Err(BzlLoadFailedException::new(
                    format!(
                        "compilation of module '{}'{} failed",
                        label.to_path_fragment(),
                        if is_builtins_repo(label.get_repository()) {
                            " (internal)"
                        } else {
                            ""
                        }
                    ),
                    StarlarkLoadingCode::CompileError,
                )));
            }
        }

        let is_scl_flag_enabled = semantics.experimental_enable_scl_dialect;
        if key.is_scl_dialect() && !is_scl_flag_enabled {
            return Ok(Err(BzlLoadFailedException::new(
                "loading .scl files requires setting --experimental_enable_scl_dialect",
                StarlarkLoadingCode::ParseError,
            )));
        }

        // Determine dependency BzlLoadValue keys for the load statements in this bzl.
        // Labels are resolved relative to the current repo mapping.
        let Some(repo_mapping) = Self::get_repository_mapping(key, env)? else {
            return Ok(Ok(None));
        };
        let Some(main_repo_mapping) = Self::get_main_repository_mapping(key, env)? else {
            return Ok(Ok(None));
        };
        let mut repo_mapping_recorder = LabelSimpleRepoMappingRecorder::new();
        let program_loads = &prog.loads;
        let Some(load_labels) = get_load_labels(
            env.listener(),
            program_loads,
            pkg,
            &is_package_under_experimental,
            semantics.allow_experimental_loads,
            &repo_mapping,
            key.is_scl_dialect(),
            is_scl_flag_enabled,
            Some(&mut repo_mapping_recorder),
        ) else {
            return Ok(Err(BzlLoadFailedException::new(
                format!(
                    "module '{}'{} has invalid load statements",
                    label.to_path_fragment(),
                    if is_builtins_repo(label.get_repository()) {
                        " (internal)"
                    } else {
                        ""
                    }
                ),
                StarlarkLoadingCode::ParseError,
            )));
        };
        let load_keys: Vec<BzlLoadKey> = load_labels
            .into_iter()
            .map(|l| key.get_key_for_load(l))
            .collect();

        // Load .bzl modules. When not using bzl inlining, this is done in parallel for all
        // loads.
        let load_values =
            match Self::compute_bzl_loads_with_skyframe(env, &load_keys, program_loads)? {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(Ok(None)), // Skyframe deps unavailable
                Err(e) => return Ok(Err(e)),
            };

        // Validate that the current .bzl file satisfies each loaded dependency's load visibility.
        // Violations are reported as error events (since there can be more than one in a single
        // file) and also trigger a BzlLoadFailedException.
        if let Err(e) = check_load_visibilities(
            pkg,
            &format!("module {}", label.get_canonical_form()),
            &load_values,
            &load_keys,
            program_loads,
            !semantics.check_bzl_visibility,
            env.listener(),
        ) {
            return Ok(Err(e));
        }

        // Accumulate a transitive digest of the bzl file, the digests of its direct loads, and
        // the digest of the @_builtins pseudo-repository (if applicable).
        let mut fp = Fingerprint::new();
        fp.add_bytes(compile_value.get_digest());

        // Populate the load map and add transitive digests to the fingerprint.
        let mut load_map = LoadMap::new();
        for (load, v) in program_loads.iter().zip(load_values.iter()) {
            load_map.insert(load.module.clone(), v.get_module().clone()); // dups ok
            fp.add_bytes(v.get_transitive_digest());
            repo_mapping_recorder.record_all(v.get_recorded_repo_mappings());
        }
        // Java's loadMap is a LinkedHashMap keyed by load string: its values are in order of the
        // first occurrence of each string.
        let mut seen_strings: Vec<&str> = Vec::new();
        let mut loads_in_map_order: Vec<Arc<BazelModuleContext>> = Vec::new();
        for (load, v) in program_loads.iter().zip(load_values.iter()) {
            if let Some(pos) = seen_strings.iter().position(|s| *s == load.module) {
                // dups ok: the later value replaces the earlier in place.
                loads_in_map_order[pos] = v.get_module_context().clone();
            } else {
                seen_strings.push(&load.module);
                loads_in_map_order.push(v.get_module_context().clone());
            }
        }

        // Retrieve predeclared symbols and complete the digest computation.
        let globals = self.get_and_digest_predeclared_environment(key, builtins, &mut fp);
        let transitive_digest = fp.digest_and_reset();

        // The BazelModuleContext holds additional contextual info to be associated with the
        // Module, including the label and a reified copy of the load DAG.
        let bazel_module_context = Arc::new(BazelModuleContext {
            label: label.clone(),
            is_builtins: key.is_builtins(),
            repo_mapping: repo_mapping.clone(),
            filename: compile_value.get_filename().to_owned(),
            loads: loads_in_map_order,
            bzl_transitive_digest: transitive_digest.clone(),
        });

        // The BzlInitThreadContext holds Starlark thread-local state to be read and updated
        // during evaluation.
        let context = BzlInitThreadContext {
            bzl_file: label.clone(),
            transitive_digest: transitive_digest.clone(),
            bzl_visibility: None,
            tools_repository: self.config.tools_repository.clone(),
            fragment_names: self.config.fragment_names.clone(),
            main_repo_mapping: main_repo_mapping.clone(),
        };

        // executeBzlFile may post events to the Environment's handler, but events do not matter
        // when caching BzlLoadValues. Note that executing the code mutates the Module and
        // BzlInitThreadContext.
        let (module, context, repo_mapping_recorder) = match Self::execute_bzl_file(
            crate::bzl_compile::clone_compiled_file(prog),
            key,
            &globals,
            &load_map,
            context,
            bazel_module_context.clone(),
            semantics.clone(),
            &main_repo_mapping,
            env.listener(),
            repo_mapping_recorder,
        ) {
            Ok(r) => r,
            Err(e) => return Ok(Err(e)),
        };

        let bzl_visibility = context.bzl_visibility.unwrap_or(BzlVisibility::Public);
        // We save load visibility in the BzlLoadValue rather than the BazelModuleContext because
        // visibility doesn't need to be introspected by any Starlark builtin methods, and because
        // the alternative would mean mutating or overwriting the BazelModuleContext after
        // evaluation.
        Ok(Ok(Some(BzlLoadValue::new(
            module,
            transitive_digest,
            bzl_visibility,
            repo_mapping_recorder.recorded_entries(),
            bazel_module_context,
        ))))
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:907 BzlLoadFunction.getRepositoryMapping
    fn get_repository_mapping(
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMapping>, blaze_skyframe::Interrupted> {
        let repo_name = key.get_label().get_repository();

        if let BzlLoadKey::ForBzlmodBootstrap(_) = key {
            // Special case: we're only here to get one of the rules in the @bazel_tools repo
            // that load Bazel modules. At this point we can't load from any other modules and
            // thus use a repository mapping that contains only @bazel_tools itself.
            return Ok(Some(RepositoryMapping::create(
                [("bazel_tools", BAZEL_TOOLS.clone())],
                BAZEL_TOOLS.clone(),
            )));
        }

        // This is either a .bzl loaded from BUILD files, or a .bzl loaded for bzlmod, so we can
        // just use the full repo mapping from RepositoryMappingFunction.
        let Some(repository_mapping_value) =
            env.get_value(RepositoryMappingKey::key(repo_name.clone()))?
        else {
            return Ok(None);
        };
        // Java returns the (possibly null) mapping of a NOT_FOUND value as is.
        Ok(Some(
            repository_mapping_value
                .repository_mapping()
                .cloned()
                .unwrap_or_else(|| {
                    blaze_util::unported!(
                        "BzlLoadFunction.getRepositoryMapping (null mapping of a repository \
                         that was not found)"
                    )
                }),
        ))
    }

    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:930 BzlLoadFunction.getMainRepositoryMapping
    fn get_main_repository_mapping(
        key: &BzlLoadKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepositoryMapping>, blaze_skyframe::Interrupted> {
        if matches!(
            key,
            BzlLoadKey::ForBuiltins(_) | BzlLoadKey::ForBzlmodBootstrap(_)
        ) {
            // For builtins and @bazel_tools, the key's local repo mapping can be used as the main
            // repo mapping.
            return Self::get_repository_mapping(key, env);
        }
        let Some(main_repository_mapping_value) =
            env.get_value(RepositoryMappingKey::key(MAIN.clone()))?
        else {
            return Ok(None);
        };
        Ok(Some(
            main_repository_mapping_value
                .repository_mapping()
                .cloned()
                .expect("the main repository has a mapping"),
        ))
    }

    /// Computes the BzlLoadValue for all given .bzl load keys using ordinary Skyframe
    /// evaluation. `program_loads` provides the locations of the load statements in source
    /// order, for error reporting.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1166 BzlLoadFunction.computeBzlLoadsWithSkyframe
    fn compute_bzl_loads_with_skyframe(
        env: &mut Environment<'_>,
        keys: &[BzlLoadKey],
        program_loads: &[LoadRequest],
    ) -> Result<Result<Option<Vec<Arc<BzlLoadValue>>>, BzlLoadFailedException>, ComputeError> {
        let sky_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
        let values = env.get_values_and_exceptions(&sky_keys)?;
        let mut bzl_loads = Vec::with_capacity(keys.len());
        // Process loads (and report first error) in source order.
        for (i, key) in keys.iter().enumerate() {
            match values.get_or_throw_typed::<BzlLoadKey, BzlLoadFailedException>(key) {
                Ok(Some(v)) => bzl_loads.push(v),
                Ok(None) => {}
                Err(ex) => {
                    return Ok(Err(while_loading_dep(&program_loads[i].location, &ex)));
                }
            }
        }
        if env.values_missing() {
            return Ok(Ok(None));
        }
        Ok(Ok(Some(bzl_loads)))
    }

    /// Obtains the predeclared environment for a .bzl (or .scl) file, based on the type of .bzl
    /// and (if applicable) the injected builtins, as `Globals`. In the case that injected
    /// builtins are used, updates the given fingerprint with the digest of the `@_builtins`
    /// pseudo-repository.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1308 BzlLoadFunction.getAndDigestPredeclaredEnvironment
    fn get_and_digest_predeclared_environment(
        &self,
        key: &BzlLoadKey,
        builtins: &StarlarkBuiltinsValue,
        fp: &mut Fingerprint,
    ) -> Arc<Globals> {
        let starlark_env = &self.config.bazel_starlark_environment;
        let label = Some(key.get_label());
        // Name resolution (Java BzlCompileFunction) uses the uninjected BUILD-loaded .bzl
        // environment's guards for every non-builtins, non-.scl file.
        let compile_guards = starlark_env.get_uninjected_build_bzl_env();
        if key.is_scl_dialect() {
            // .scl doesn't use injection and doesn't care what kind of key it is.
            return builtins.globals(starlark_env.get_scl_toplevels(), None, label);
        }
        // TODO(#11437): Remove ability to disable injection by setting flag to empty string.
        let injection_disabled = builtins
            .starlark_semantics
            .experimental_builtins_bzl_path
            .is_empty();
        match key {
            BzlLoadKey::ForBuild { .. } => {
                if injection_disabled {
                    return builtins.globals(compile_guards, None, label);
                }
                fp.add_bytes(&builtins.transitive_digest);
                builtins.globals(
                    &builtins.predeclared_for_build_bzl,
                    Some(compile_guards),
                    label,
                )
            }
            BzlLoadKey::ForBzlmod(_) | BzlLoadKey::ForBzlmodBootstrap(_) => {
                // TODO(#11954): We should converge all .bzl dialects regardless of whether
                // they're loaded by BUILD or MODULE.
                if injection_disabled || matches!(key, BzlLoadKey::ForBzlmodBootstrap(_)) {
                    return builtins.globals(
                        starlark_env.get_uninjected_module_bzl_env(),
                        Some(compile_guards),
                        label,
                    );
                }
                // Note that we don't actually fingerprint the injected builtins here (see Java).
                builtins.globals(
                    &builtins.predeclared_for_module_bzl,
                    Some(compile_guards),
                    label,
                )
            }
            BzlLoadKey::ForBuiltins(_) => {
                builtins.globals(starlark_env.get_builtins_bzl_env(), None, label)
            }
        }
    }

    /// The predeclared environment whose names (and `GuardedValue`s) Java's BzlCompileFunction
    /// resolves a .bzl file against: the .scl toplevels, the `@_builtins` environment, or the
    /// uninjected environment of BUILD-loaded .bzl files ("the names of the symbols are the
    /// same, and the names are all we need to do symbol resolution").
    ///
    /// PORT: Java takes the last one from `AutoloadSymbols.getUninjectedBuildBzlEnv(repo)`; like
    /// `get_and_digest_predeclared_environment` this uses the BazelStarlarkEnvironment's.
    // Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java:150 BzlCompileFunction.computeInline
    fn compile_environment(&self, key: &BzlLoadKey) -> &blaze_packages::builtins::env::StarlarkEnv {
        let starlark_env = &self.config.bazel_starlark_environment;
        if key.is_scl_dialect() {
            starlark_env.get_scl_toplevels()
        } else if key.is_builtins() {
            starlark_env.get_builtins_bzl_env()
        } else {
            starlark_env.get_uninjected_build_bzl_env()
        }
    }

    /// Executes the compiled .bzl file defining the module to be loaded.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java:1378 BzlLoadFunction.executeBzlFile
    fn execute_bzl_file(
        prog: CompiledFile,
        key: &BzlLoadKey,
        globals: &Globals,
        loaded_modules: &LoadMap,
        context: BzlInitThreadContext,
        module_context: Arc<BazelModuleContext>,
        starlark_semantics: Arc<StarlarkSemantics>,
        main_repo_mapping: &RepositoryMapping,
        skyframe_event_handler: &dyn ExtendedEventHandler,
        repo_mapping_recorder: LabelSimpleRepoMappingRecorder,
    ) -> Result<
        (
            FrozenModule,
            BzlInitThreadContext,
            LabelSimpleRepoMappingRecorder,
        ),
        BzlLoadFailedException,
    > {
        let label = key.get_label();
        // PORT: natives find the module of the innermost Starlark function through the module's
        // extra value (`exec_module_with_context`; Java Module client data). The table keyed by
        // file name is still filled for callers in other crates (it is ambiguous when a file is
        // executed under several keys; see blaze_packages `bzl_context`).
        BazelModuleContext::register(module_context.clone());

        let events = Arc::new(CollectingEvents::default());
        let thread_context = ThreadContext::new(Phase::BzlLoad, starlark_semantics, events.clone())
            .with_file_label(label.clone())
            .with_label_repo_context(LabelRepoContextImpl::of(
                label.get_repository().clone(),
                module_context.repo_mapping.clone(),
            ))
            .with_main_repo_mapping(main_repo_mapping.clone());
        // `Label()` resolves against the module of the innermost Starlark function (this module
        // or a loaded one), read from that module's extra value.
        // This is needed so that any calls to `Label()` will have its used repo mapping entries
        // recorded. See #20721 for more details.
        let thread_context = thread_context
            .with_simple_repo_mapping_recorder(repo_mapping_recorder)
            .with_payload(context);

        let result = exec_module_with_context(
            prog,
            globals,
            loaded_modules,
            &thread_context,
            &EvalOptions::default(),
            module_context,
        );
        // Wrap the skyframe event handler to listen for starlark errors.
        let mut saw_starlark_error = false;
        starlark_util::replay_starlark_events_on(skyframe_event_handler, events.take());
        let module = match result {
            Ok(m) => Some(m),
            Err(ex) => {
                if let EvalErrorKind::Syntax(errors) = &ex.kind {
                    // Static errors are found before execution (above, by resolve); a syntax
                    // error from execution is replayed as such.
                    starlark_util::replay_syntax_errors_on(skyframe_event_handler, errors);
                } else {
                    skyframe_event_handler.handle(&Event::error(ex.message_with_stack()));
                }
                saw_starlark_error = true;
                None
            }
        };
        if saw_starlark_error {
            return Err(execution_failed(label));
        }
        let module = module.expect("no error");
        KEEP_ALIVE.lock().push(module.clone());
        let context = thread_context
            .take_payload::<BzlInitThreadContext>()
            .expect("the .bzl thread context");
        let repo_mapping_recorder = thread_context
            .take_simple_repo_mapping_recorder()
            .expect("the .bzl thread's repo mapping recorder");
        Ok((module, context, repo_mapping_recorder))
    }
}
