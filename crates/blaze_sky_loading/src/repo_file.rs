//! REPO_FILE and REPO_PACKAGE_ARGS: evaluating the REPO.bazel file at the root of a repo.
//!
//! Java: com/google/devtools/build/lib/skyframe/{RepoFileFunction,RepoFileValue,
//! RepoPackageArgsFunction}.java. The `repo()` / `ignore_directories()` globals are in
//! `blaze_packages::repo_file`.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::label_constants::REPO_FILE_NAME;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{PackageIdentifier, RepositoryMapping, RepositoryName};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::package::PackageArgs;
use blaze_packages::repo_file::RepoThreadContext;
use blaze_packages::types::label_converter::LabelConverter;
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, Transience,
};
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::eval::{EvalOptions, compile, exec_module};
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::{FileKind, Phase, StarlarkSemantics, ThreadContext};
use blaze_vfs::{Path, PathFragment, Root, RootedPath};
use starlark::values::OwnedFrozenValue;
use starlark::values::dict::DictRef;

use crate::file::FileKey;
use crate::io::IOException;
use crate::precomputed::STARLARK_SEMANTICS;
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue, RepositoryMappingKey};
use crate::sky_functions;
use crate::starlark_util;

/// Contains information about the REPO.bazel file at the root of a repo.
///
/// `package_args_map` is Java's `ImmutableMap<String, Object>` of `repo()` keyword arguments: a
/// frozen Starlark dict (in call order), or `None` when `repo()` was not called (Java: an empty
/// map).
// Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:29 RepoFileValue
#[derive(Clone, Debug)]
pub struct RepoFileValue {
    package_args_map: Option<OwnedFrozenValue>,
    ignored_directories: Vec<String>,
}

impl RepoFileValue {
    // Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:38 RepoFileValue.EMPTY
    pub fn empty() -> RepoFileValue {
        RepoFileValue::of(None, Vec::new())
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:40 RepoFileValue.of
    pub fn of(
        package_args_map: Option<OwnedFrozenValue>,
        ignored_directories: Vec<String>,
    ) -> RepoFileValue {
        RepoFileValue {
            package_args_map,
            ignored_directories,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:29 RepoFileValue.packageArgsMap
    pub fn package_args_map(&self) -> Option<&OwnedFrozenValue> {
        self.package_args_map.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:29 RepoFileValue.ignoredDirectories
    pub fn ignored_directories(&self) -> &[String] {
        &self.ignored_directories
    }
}

/// Key type for [`RepoFileValue`]. `Debug` prints `REPO_FILE:@@name`.
// Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:51 RepoFileValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RepoFileKey(pub RepositoryName);

impl RepoFileKey {
    // Java: com/google/devtools/build/lib/skyframe/RepoFileValue.java:46 RepoFileValue.key
    pub fn key(repo_name: RepositoryName) -> Self {
        RepoFileKey(repo_name)
    }
}

impl fmt::Debug for RepoFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for RepoFileKey {
    type Value = RepoFileValue;
    const FUNCTION: SkyFunctionName = sky_functions::REPO_FILE;
}

/// Thrown when something is wrong with the contents of the REPO.bazel file of a certain repo.
// Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:181 RepoFileFunction.BadRepoFileException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BadRepoFileException {
    message: String,
}

impl BadRepoFileException {
    pub fn new(message: impl Into<String>) -> Self {
        BadRepoFileException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

// Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:192 RepoFileFunction.RepoFileFunctionException
fn bad_repo_file(e: BadRepoFileException) -> ComputeError {
    SkyFunctionException::persistent(e).into()
}

// Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:140 RepoFileFunction.getDisplayNameForRepo
pub fn get_display_name_for_repo(
    repo_name: &RepositoryName,
    main_repo_mapping: Option<&RepositoryMapping>,
) -> String {
    let display_name = repo_name.get_display_form(main_repo_mapping);
    if display_name.is_empty() {
        return "the main repo".to_owned();
    }
    display_name
}

/// The function to evaluate the REPO.bazel file at the root of a repo.
// Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:50 RepoFileFunction
pub struct RepoFileFunction {
    /// Java `BazelStarlarkEnvironment.getRepoBazelEnv()`, as natives registered for
    /// `FileKind::RepoFile`.
    starlark_env: Arc<NativeRegistry>,
    workspace_root: Root,
}

impl RepoFileFunction {
    // Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:54 RepoFileFunction.<init>
    pub fn new(starlark_env: Arc<NativeRegistry>, workspace_root: Root) -> Self {
        RepoFileFunction {
            starlark_env,
            workspace_root,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:59 RepoFileFunction.compute
    pub fn compute(
        &self,
        sky_key: &RepoFileKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<RepoFileValue>, ComputeError> {
        let repo_name = &sky_key.0;
        // First we need to find the REPO.bazel file. How we do this depends on whether this is
        // for the main repo or an external repo.
        let repo_root = if repo_name.is_main() {
            self.workspace_root.clone()
        } else {
            let Some(repo_dir_value) =
                env.get_value(RepositoryDirectoryKey::key(repo_name.clone()))?
            else {
                return Ok(None);
            };
            match &*repo_dir_value {
                RepositoryDirectoryValue::Success { root, .. } => root.clone(),
                RepositoryDirectoryValue::Failure { error_msg } => {
                    return Err(SkyFunctionException::new(
                        IOException::new(error_msg.clone()),
                        Transience::Persistent,
                    )
                    .into());
                }
            }
        };
        let repo_file_path = RootedPath::to_rooted_path(repo_root, &REPO_FILE_NAME);
        let Some(repo_file_value) = env.get_value(FileKey::key(repo_file_path.clone()))? else {
            return Ok(None);
        };
        if !repo_file_value.exists() {
            // It's okay to not have a REPO.bazel file.
            return Ok(Some(RepoFileValue::of(None, Vec::new())));
        }

        // Now we can actually evaluate the file.
        let starlark_semantics = STARLARK_SEMANTICS.get(env)?;
        if env.values_missing() {
            return Ok(None);
        }
        let starlark_semantics = starlark_semantics.expect("not missing");
        let path = repo_file_path.as_path();
        let source = Self::read_repo_file(&path, env.listener(), &starlark_semantics)?;
        self.eval_repo_file(&path, source, repo_name, starlark_semantics, env.listener())
            .map(Some)
    }

    /// Reads the file and checks its encoding. PORT: Java parses here too
    /// (`readAndParseRepoFile`); Blaze parses in [`RepoFileFunction::eval_repo_file`] and tells a
    /// parse failure from a static-check failure there, keeping Java's two messages.
    // Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:101 RepoFileFunction.readAndParseRepoFile
    fn read_repo_file(
        path: &Path,
        listener: &dyn ExtendedEventHandler,
        starlark_semantics: &StarlarkSemantics,
    ) -> Result<String, ComputeError> {
        let read = (|| -> std::io::Result<Vec<u8>> {
            let mut input = path.get_input_stream()?;
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut input, &mut contents)?;
            Ok(contents)
        })();
        let contents = match read {
            Ok(c) => c,
            Err(_e) => {
                // Java: new IOException("error reading REPO.bazel file at " + path, e)
                return Err(SkyFunctionException::transient(IOException::new(format!(
                    "error reading REPO.bazel file at {}",
                    path
                )))
                .into());
            }
        };
        match starlark_util::create_parser_input(
            contents,
            path.get_path_string(),
            starlark_semantics.incompatible_enforce_starlark_utf8,
            listener,
        ) {
            Ok(s) => Ok(s),
            // createParserInput() reports its own error message
            Err(_) => Err(bad_repo_file(BadRepoFileException::new(format!(
                "error reading REPO.bazel file at {}",
                path
            )))),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoFileFunction.java:149 RepoFileFunction.evalRepoFile
    fn eval_repo_file(
        &self,
        path: &Path,
        source: String,
        repo_name: &RepositoryName,
        starlark_semantics: Arc<StarlarkSemantics>,
        handler: &dyn ExtendedEventHandler,
    ) -> Result<RepoFileValue, ComputeError> {
        let file_name = path.get_path_string();
        let repo_display_name = get_display_name_for_repo(repo_name, None);
        // Java: StarlarkFile.parse, then DotBazelFileSyntaxChecker("REPO.bazel files",
        // canLoadBzl=false).check and Program.compileFile; blaze_starlark::eval::compile does
        // all three.
        let compiled = match compile(FileKind::RepoFile, file_name, source.clone()) {
            Ok(c) => c,
            Err(e) => {
                let EvalErrorKind::Syntax(errors) = &e.kind else {
                    unreachable!("compile only fails with syntax errors")
                };
                starlark_util::replay_syntax_errors_on(handler, errors);
                let parse_failed = starlark::syntax::AstModule::parse(
                    file_name,
                    source,
                    &FileKind::RepoFile.dialect(),
                )
                .is_err();
                return Err(bad_repo_file(BadRepoFileException::new(if parse_failed {
                    // Java: readAndParseRepoFile, !starlarkFile.ok()
                    format!("error parsing REPO.bazel file at {}", path)
                } else {
                    format!("error parsing REPO.bazel file for {}", repo_display_name)
                })));
            }
        };
        let globals = self
            .starlark_env
            .build(FileKind::RepoFile, &starlark_semantics);
        let events = Arc::new(CollectingEvents::default());
        let context = ThreadContext::new(Phase::RepoFile, starlark_semantics, events.clone())
            .with_payload(RepoThreadContext::new());
        let result = exec_module(
            compiled,
            &globals,
            &LoadMap::new(),
            &context,
            &EvalOptions::default(),
        );
        starlark_util::replay_starlark_events_on(handler, events.take());
        let module = match result {
            Ok(m) => m,
            Err(e) => {
                if let EvalErrorKind::Syntax(errors) = &e.kind {
                    starlark_util::replay_syntax_errors_on(handler, errors);
                    return Err(bad_repo_file(BadRepoFileException::new(format!(
                        "error parsing REPO.bazel file for {}",
                        repo_display_name
                    ))));
                }
                handler.handle(&Event::error(e.message_with_stack()));
                return Err(bad_repo_file(BadRepoFileException::new(format!(
                    "error evaluating REPO.bazel file for {}",
                    repo_display_name
                ))));
            }
        };
        let thread_context = context
            .take_payload::<RepoThreadContext>()
            .expect("the REPO.bazel thread context");
        Ok(RepoFileValue::of(
            module.owned_extra_value(),
            thread_context.get_ignored_directories().to_vec(),
        ))
    }
}

/// SkyValue wrapping the package args of a repository.
// Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:37 RepoPackageArgsFunction.RepoPackageArgsValue
#[derive(Clone, Debug)]
pub struct RepoPackageArgsValue {
    package_args: PackageArgs,
}

impl RepoPackageArgsValue {
    // Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:43 RepoPackageArgsValue.EMPTY
    pub fn empty() -> Self {
        RepoPackageArgsValue::new(PackageArgs::empty())
    }

    pub fn new(package_args: PackageArgs) -> Self {
        RepoPackageArgsValue { package_args }
    }

    // Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:46 RepoPackageArgsValue.getPackageArgs
    pub fn get_package_args(&self) -> &PackageArgs {
        &self.package_args
    }
}

/// The error of REPO_PACKAGE_ARGS (Java `RepoPackageArgsFunction.BadPackageArgsException`): a
/// `repo()` argument failed to convert.
// Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:71 RepoPackageArgsFunction.BadPackageArgsException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BadPackageArgsException {
    message: String,
}

impl BadPackageArgsException {
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Key type for [`RepoPackageArgsValue`]. `Debug` prints `REPO_PACKAGE_ARGS:@@name`.
// Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:87 RepoPackageArgsFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RepoPackageArgsKey(pub RepositoryName);

impl RepoPackageArgsKey {
    // Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:66 RepoPackageArgsFunction.key
    pub fn key(repo_name: RepositoryName) -> Self {
        RepoPackageArgsKey(repo_name)
    }
}

impl fmt::Debug for RepoPackageArgsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for RepoPackageArgsKey {
    type Value = RepoPackageArgsValue;
    const FUNCTION: SkyFunctionName = sky_functions::REPO_PACKAGE_ARGS;
}

/// A SkyFunction that returns the package args for a given repository.
// Java: com/google/devtools/build/lib/skyframe/RepoPackageArgsFunction.java:102 RepoPackageArgsFunction.compute
pub fn repo_package_args_function(
    sky_key: &RepoPackageArgsKey,
    env: &mut Environment<'_>,
) -> Result<Option<RepoPackageArgsValue>, ComputeError> {
    let repository_name = &sky_key.0;
    let repo_file_value = env.get_value(RepoFileKey::key(repository_name.clone()))?;
    let repository_mapping_value =
        env.get_value(RepositoryMappingKey::key(repository_name.clone()))?;
    let main_repo_mapping = env.get_value(RepositoryMappingKey::key(MAIN.clone()))?;
    if env.values_missing() {
        return Ok(None);
    }
    let (Some(repo_file_value), Some(repository_mapping_value), Some(main_repo_mapping)) =
        (repo_file_value, repository_mapping_value, main_repo_mapping)
    else {
        return Ok(None);
    };
    let repo_display_name =
        get_display_name_for_repo(repository_name, main_repo_mapping.repository_mapping());

    let mut pkg_args_builder = PackageArgs::builder();
    let result = match repo_file_value.package_args_map() {
        None => Ok(()),
        Some(args) => {
            let Some(repository_mapping) = repository_mapping_value.repository_mapping().cloned()
            else {
                // Java passes a null mapping to the LabelConverter.
                blaze_util::unported!(
                    "RepoPackageArgsFunction.compute without a repository mapping"
                )
            };
            let label_converter = LabelConverter::new(
                &PackageIdentifier::create(repository_name.clone(), PathFragment::empty_fragment()),
                repository_mapping,
            );
            let dict = DictRef::from_value(args.value()).expect("repo() kwargs are a dict");
            dict.iter().try_for_each(|(k, v)| {
                let key = k.unpack_str().expect("repo() kwarg names are strings");
                PackageArgs::process_param(
                    key,
                    v,
                    &format!("repo() argument '{}'", key),
                    &label_converter,
                    &mut pkg_args_builder,
                )
            })
        }
    };
    if let Err(e) = result {
        // PORT: Java prints `e.getMessageWithStack()`; conversion errors carry no stack.
        env.listener().handle(&Event::error(e.get_message()));
        return Err(SkyFunctionException::persistent(BadPackageArgsException {
            message: format!("error evaluating REPO.bazel file for {}", repo_display_name),
        })
        .into());
    }

    Ok(Some(RepoPackageArgsValue::new(pkg_args_builder.build())))
}
