//! VENDOR_FILE: evaluating the VENDOR.bazel file under the vendor directory (`--vendor_dir`).
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/{VendorFileFunction,VendorFileValue}.java.
//!
//! PORT: Java's package is `bazel.bzlmod` (`blaze_bzlmod`); the function reads a `FileValue`
//! (`blaze_sky_loading`), which that crate does not depend on, so it is here.

use std::fmt;
use std::sync::Arc;

use blaze_bzlmod::skyframe::precomputed::VENDOR_DIRECTORY;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::label_constants::VENDOR_FILE_NAME;
use blaze_events::ExtendedEventHandler;
use blaze_packages::vendor_file::VendorThreadContext;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_sky_loading::starlark_util;
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
};
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{EvalOptions, compile, exec_module};
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::{FileKind, Phase, StarlarkSemantics, ThreadContext};
use blaze_util::string_encoding::{internal_to_platform, platform_to_internal};
use blaze_vfs::{Path, PathFragment, Root, RootedPath};

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:182 SkyFunctions.VENDOR_FILE
pub const VENDOR_FILE: SkyFunctionName = SkyFunctionName("VENDOR_FILE");

// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:56 VendorFileFunction.VENDOR_FILE_HEADER
const VENDOR_FILE_HEADER: &str = "\
###############################################################################
# This file is used to configure how external repositories are handled in vendor mode.
# ONLY the two following functions can be used:
#
# ignore('@@<canonical repo name>', ...) is used to completely ignore this repo from vendoring.
# Bazel will use the normal external cache and fetch process for this repo.
#
# pin('@@<canonical repo name>', ...) is used to pin the contents of this repo under the vendor
# directory as if there is a --override_repository flag for this repo.
# Note that Bazel will NOT update the vendored source for this repo while running vendor command
# unless it's unpinned. The user can modify and maintain the vendored source for this repo manually.
###############################################################################
";

/// Represent the parsed VENDOR.bazel file.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileValue.java:27 VendorFileValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorFileValue {
    ignored_repos: Vec<RepositoryName>,
    pinned_repos: Vec<RepositoryName>,
}

impl VendorFileValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileValue.java:37 VendorFileValue.create
    pub fn create(ignored_repos: Vec<RepositoryName>, pinned_repos: Vec<RepositoryName>) -> Self {
        VendorFileValue {
            ignored_repos,
            pinned_repos,
        }
    }

    pub fn ignored_repos(&self) -> &[RepositoryName] {
        &self.ignored_repos
    }

    pub fn pinned_repos(&self) -> &[RepositoryName] {
        &self.pinned_repos
    }
}

/// `VendorFileValue.KEY`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileValue.java:35 VendorFileValue.KEY
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VendorFileKey;

impl fmt::Debug for VendorFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", VENDOR_FILE)
    }
}

impl Key for VendorFileKey {
    type Value = VendorFileValue;
    const FUNCTION: SkyFunctionName = VENDOR_FILE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:182 SkyFunctions.VENDOR_FILE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record of two lists of repository names.
    fn values_equal(old: &VendorFileValue, new: &VendorFileValue) -> bool {
        old == new
    }
}

/// Thrown when something is wrong with the contents of the VENDOR.bazel file.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:183 VendorFileFunction.BadVendorFileException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BadVendorFileException {
    message: String,
}

impl BadVendorFileException {
    pub fn new(message: impl Into<String>) -> Self {
        BadVendorFileException {
            message: message.into(),
        }
    }
}

/// The `IllegalStateException` VENDOR_FILE fails with when vendor mode is off.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct IllegalStateException(pub String);

/// The function to evaluate the VENDOR.bazel file under the vendor directory specified by the
/// flag: `--vendor_dir`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:52 VendorFileFunction
pub struct VendorFileFunction {
    /// Java `starlarkEnv.getStarlarkGlobals().getVendorToplevels()`.
    starlark_env: Arc<NativeRegistry>,
    /// The file system the vendor directory is on.
    file_system: blaze_vfs::FileSystemRef,
}

impl VendorFileFunction {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:73 VendorFileFunction.<init>
    pub fn new(file_system: blaze_vfs::FileSystemRef) -> Self {
        let mut registry = NativeRegistry::with_blaze_starlark_builtins();
        registry.register(
            &[FileKind::VendorFile],
            blaze_packages::vendor_file::register_vendor_file_globals,
        );
        VendorFileFunction {
            starlark_env: Arc::new(registry),
            file_system,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:77 VendorFileFunction.compute
    pub fn compute(
        &self,
        _key: &VendorFileKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<VendorFileValue>, ComputeError> {
        let vendor_directory = VENDOR_DIRECTORY.get_required(env)?;
        let Some(vendor_directory) = vendor_directory.as_ref() else {
            return Err(SkyFunctionException::persistent(IllegalStateException(
                "VENDOR.bazel file is not accessible with vendor mode off (without --vendor_dir \
                 flag)"
                    .to_string(),
            ))
            .into());
        };

        let vendor_path = Path::create(
            PathFragment::create(&platform_to_internal(vendor_directory.as_os_str())),
            &self.file_system,
        );
        let vendor_file_path =
            RootedPath::to_rooted_path(Root::from_path(vendor_path.clone()), &VENDOR_FILE_NAME);

        let Some(vendor_file_value) = env.get_value(FileKey::key(vendor_file_path.clone()))?
        else {
            return Ok(None);
        };
        if !vendor_file_value.exists() {
            Self::create_vendor_file(&vendor_path, &vendor_file_path.as_path())?;
            return Ok(Some(VendorFileValue::create(Vec::new(), Vec::new())));
        }

        let starlark_semantics = STARLARK_SEMANTICS.get(env)?;
        let Some(starlark_semantics) = starlark_semantics else {
            return Ok(None);
        };
        let context = self.get_vendor_file_context(
            env.listener(),
            &vendor_file_path.as_path(),
            starlark_semantics,
        )?;
        Ok(Some(VendorFileValue::create(
            context.get_ignored_repos(),
            context.get_pinned_repos(),
        )))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:108 VendorFileFunction.getVendorFileContext
    fn get_vendor_file_context(
        &self,
        handler: &dyn ExtendedEventHandler,
        vendor_file_path: &Path,
        starlark_semantics: Arc<StarlarkSemantics>,
    ) -> Result<VendorThreadContext, ComputeError> {
        let source = Self::read_vendor_file(vendor_file_path, handler, &starlark_semantics)?;
        let file_name = vendor_file_path.get_path_string();
        // StarlarkFile.parse (readAndParseVendorFile), then
        // DotBazelFileSyntaxChecker("VENDOR.bazel files", canLoadBzl=false).check and
        // Program.compileFile: blaze_starlark::eval::compile does all three.
        let compiled = match compile(FileKind::VendorFile, file_name, source.clone()) {
            Ok(c) => c,
            Err(e) => {
                let EvalErrorKind::Syntax(errors) = &e.kind else {
                    unreachable!("compile only fails with syntax errors")
                };
                let parse_failed = starlark::syntax::AstModule::parse(
                    file_name,
                    source,
                    &FileKind::VendorFile.dialect(),
                )
                .is_err();
                if parse_failed {
                    // Java: readAndParseVendorFile, !starlarkFile.ok(): the errors are replayed.
                    starlark_util::replay_syntax_errors_on(handler, errors);
                    return Err(SkyFunctionException::persistent(BadVendorFileException::new(
                        "error parsing VENDOR.bazel file",
                    ))
                    .into());
                }
                // catch (SyntaxError.Exception e): its message is the first error's, with the
                // count of the others.
                // Java: net/starlark/java/syntax/SyntaxError.java:89 SyntaxError.Exception.getMessage
                let first = errors
                    .first()
                    .map(|e| e.message.clone())
                    .unwrap_or_default();
                let message = if errors.len() > 1 {
                    format!("{} (+ {} more)", first, errors.len() - 1)
                } else {
                    first
                };
                return Err(SkyFunctionException::persistent(BadVendorFileException::new(
                    format!("error parsing VENDOR.bazel file: {}", message),
                ))
                .into());
            }
        };
        let globals = self
            .starlark_env
            .build(FileKind::VendorFile, &starlark_semantics);
        let events = Arc::new(CollectingEvents::default());
        let context = ThreadContext::new(Phase::RepoFile, starlark_semantics, events.clone())
            .with_payload(VendorThreadContext::new());
        let result = exec_module(
            compiled,
            &globals,
            &LoadMap::new(),
            &context,
            &EvalOptions::default(),
        );
        starlark_util::replay_starlark_events_on(handler, events.take());
        if let Err(e) = result {
            // catch (SyntaxError.Exception | EvalException e)
            return Err(SkyFunctionException::persistent(BadVendorFileException::new(format!(
                "error parsing VENDOR.bazel file: {}",
                e.message
            )))
            .into());
        }
        Ok(context
            .take_payload::<VendorThreadContext>()
            .expect("the VENDOR.bazel thread context"))
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:136 VendorFileFunction.createVendorFile
    fn create_vendor_file(vendor_path: &Path, vendor_file_path: &Path) -> Result<(), ComputeError> {
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&*internal_to_platform(vendor_path.get_path_string()))?;
            // FileSystemUtils.writeContent(vendorFilePath, ISO_8859_1, VENDOR_FILE_HEADER)
            std::fs::write(
                &*internal_to_platform(vendor_file_path.get_path_string()),
                blaze_util::string_encoding::latin1_encode_lossy(VENDOR_FILE_HEADER).as_ref(),
            )
        };
        write().map_err(|_| {
            // new IOException("error creating VENDOR.bazel file", e)
            SkyFunctionException::transient(IOException::new("error creating VENDOR.bazel file"))
                .into()
        })
    }

    /// Reads the file and checks its encoding.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorFileFunction.java:147 VendorFileFunction.readAndParseVendorFile
    fn read_vendor_file(
        path: &Path,
        handler: &dyn ExtendedEventHandler,
        starlark_semantics: &StarlarkSemantics,
    ) -> Result<String, ComputeError> {
        let contents = std::fs::read(&*internal_to_platform(path.get_path_string())).map_err(
            |_| -> ComputeError {
                SkyFunctionException::transient(IOException::new(
                    "error reading VENDOR.bazel file",
                ))
                .into()
            },
        )?;
        starlark_util::create_parser_input(
            contents,
            path.get_path_string(),
            starlark_semantics.incompatible_enforce_starlark_utf8,
            handler,
        )
        .map_err(|_| {
            // createParserInput() reports its own error message
            SkyFunctionException::persistent(BadVendorFileException::new(
                "error reading VENDOR.bazel file",
            ))
            .into()
        })
    }
}
