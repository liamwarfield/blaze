//! `repository_ctx`: the methods only repository rules have (the base methods shared with
//! `module_ctx` are in `base_external_context`).
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java.
//!
//! PORT: the Java constructor also builds the `StarlarkBaseExternalContext` part (working
//! directory = the repo root, identifying string =
//! [`repository_fetch_context_string`], `allowWatchingPathsOutsideWorkspace = true`); here the
//! caller (`RepositoryFetchFunction`) builds both parts of the [`ExternalContext`].
//!
//! PORT: `WorkspaceRuleEvent`s (the debug log of `--experimental_workspace_rules_log_file`) are
//! not posted; Blaze has no subscriber for them. Java's `InvalidPathException` catches are not
//! ported: Blaze's VFS never throws it (it comes from `java.nio` path conversion).
//!
//! PORT: the `SyscallCache` constructor argument is not ported; `getWorkspaceFile` stats directly
//! (Java `SyscallCache.NO_CACHE` behaviour).

use std::fmt;
use std::io::Write;
use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::label_constants::WORKSPACE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::label_constants::WORKSPACE_FILE_NAME;
use blaze_repo_fetch::decompressor::patch_util;
use blaze_repo_fetch::decompressor::patch_util::PatchError;
use blaze_repo_fetch::errors::IoException;
use blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectories;
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_starlark::label::LabelValue;
use blaze_vfs::DirentType;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::Symlinks;
use indexmap::IndexMap;
use starlark::environment::MethodsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::none::NoneType;

use crate::repository::repo_definition::RepoAttrs;
use crate::repository::repo_definition::RepoDefinition;
use crate::repository::repo_recorded_input::RepoRecordedInput;
use crate::starlark::base_external_context::ShouldWatch;
use crate::starlark::base_external_context::StarlarkBaseExternalContext;
use crate::starlark::context::ExternalContext;
use crate::starlark::context::RepositoryCtx;
use crate::starlark::path::StarlarkPath;
use crate::starlark::repo_metadata::RepoMetadata;
use crate::starlark::repo_metadata::RepoMetadataValue;
use crate::starlark::repo_metadata::Reproducibility;
use crate::starlark::repo_metadata::attrs_from_starlark_dict;

/// The `repository_ctx`-specific state of a repository rule evaluation.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:73 StarlarkRepositoryContext
pub struct StarlarkRepositoryContext {
    pub repo_definition: Arc<RepoDefinition>,
    pub package_locator: Arc<PathPackageLocator>,
    pub ignored_subdirectories: IgnoredSubdirectories,
}

impl StarlarkRepositoryContext {
    /// Create a new context (repository_ctx) object for a Starlark repository rule.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:83 StarlarkRepositoryContext.<init>
    pub fn new(
        repo_definition: Arc<RepoDefinition>,
        package_locator: Arc<PathPackageLocator>,
        ignored_subdirectories: IgnoredSubdirectories,
    ) -> Self {
        StarlarkRepositoryContext {
            repo_definition,
            package_locator,
            ignored_subdirectories,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:120 StarlarkRepositoryContext.shouldDeleteWorkingDirectoryOnClose
    pub fn should_delete_working_directory_on_close(&self, successful: bool) -> bool {
        !successful
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:362 StarlarkRepositoryContext.isRemotable
    pub fn is_remotable(&self) -> bool {
        self.repo_definition.repo_rule.remotable
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:367 StarlarkRepositoryContext.getRemoteExecProperties
    pub fn get_remote_exec_properties(&self) -> anyhow::Result<IndexMap<String, String>> {
        Heap::temp(|heap| {
            let Some(value) = self.repo_definition.get_value("exec_properties", heap) else {
                // Java: Dict.cast(null, ...) fails in Preconditions.checkNotNull.
                panic!("java.lang.NullPointerException");
            };
            dict_cast_string_string(value, "exec_properties").map_err(|e| anyhow::anyhow!("{e}"))
        })
    }
}

/// Java `RepositoryFetchProgress.repositoryFetchContextString`: the identifying string of a repo
/// fetch for logging.
// PORT: move to the port of com/google/devtools/build/lib/repository/RepositoryFetchProgress.java
// Java: com/google/devtools/build/lib/repository/RepositoryFetchProgress.java:26 RepositoryFetchProgress.repositoryFetchContextString
pub fn repository_fetch_context_string(repo_name: &RepositoryName) -> String {
    format!("repository {}", repo_name)
}

/// Java `Starlark.errorf` / `new EvalException(msg)`.
fn eval_error(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// A `RepositoryFunctionException(IOException e, ...)` thrown out of a Starlark method: Starlark
/// reports `new EvalException(rfe)`, whose message is the RFE's message, `e.toString()`
/// (`<class>: <message>`).
// Java: net/starlark/java/eval/MethodDescriptor.java:294 MethodDescriptor.call
fn repository_function_exception_io(e: IoException) -> starlark::Error {
    eval_error(e.to_java_string())
}

/// A `RepositoryFunctionException(EvalException e, ...)` thrown out of a Starlark method: the
/// message is `e.toString()`, which for an `EvalException` without a call stack is its message.
fn repository_function_exception_eval(message: impl fmt::Display) -> starlark::Error {
    eval_error(message)
}

/// Java `BuiltinFunction` argument type check (`ParamDescriptor.getTypeErrorMessage`).
// Java: net/starlark/java/eval/BuiltinFunction.java:393 ArgumentProcessor.checkParamValue
fn check_param_type(
    method: &str,
    param: &str,
    value: Value<'_>,
    ok: bool,
    want: &str,
) -> starlark::Result<()> {
    if ok {
        return Ok(());
    }
    Err(eval_error(format!(
        "in call to {}(), parameter '{}' got value of type '{}', want '{}'",
        method,
        param,
        value.get_type(),
        want
    )))
}

fn is_string_label_or_path(value: Value<'_>) -> bool {
    value.unpack_str().is_some()
        || value.downcast_ref::<LabelValue>().is_some()
        || value.downcast_ref::<StarlarkPath>().is_some()
}

fn is_string_or_path(value: Value<'_>) -> bool {
    value.unpack_str().is_some() || value.downcast_ref::<StarlarkPath>().is_some()
}

fn is_int(value: Value<'_>) -> bool {
    value.unpack_i32().is_some() || value.get_type() == "int"
}

/// `@Param(allowedTypes = {String, Label, StarlarkPath})`.
fn check_string_label_or_path(method: &str, param: &str, value: Value<'_>) -> starlark::Result<()> {
    check_param_type(
        method,
        param,
        value,
        is_string_label_or_path(value),
        "string, Label, or path",
    )
}

/// `Dict.cast(x, String.class, String.class, what)`, copied into an ordered map.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
fn dict_cast_string_string(x: Value<'_>, what: &str) -> Result<IndexMap<String, String>, String> {
    let Some(dict) = DictRef::from_value(x) else {
        return Err(format!("got {} for '{}', want dict", x.get_type(), what));
    };
    let mut out = IndexMap::with_capacity(dict.len());
    for (k, v) in dict.iter() {
        match (k.unpack_str(), v.unpack_str()) {
            (Some(k), Some(v)) => {
                out.insert(k.to_owned(), v.to_owned());
            }
            _ => {
                // TODO(adonovan): change message to "found <K2, V2> entry",
                // without suggesting that the entire dict is <K2, V2>.
                return Err(format!(
                    "got dict<{}, {}> for '{}', want dict<string, string>",
                    k.get_type(),
                    v.get_type(),
                    what
                ));
            }
        }
    }
    Ok(out)
}

/// `Starlark.toInt(x, what)` for a value already checked to be an int.
// Java: net/starlark/java/eval/Starlark.java:755 Starlark.toInt
fn to_int(x: Value<'_>, what: &str) -> starlark::Result<i32> {
    if let Some(i) = x.unpack_i32() {
        return Ok(i);
    }
    if x.get_type() == "int" {
        // Java: net/starlark/java/eval/StarlarkInt.java:338 StarlarkInt.toInt
        return Err(eval_error(format!(
            "got {} for {}, want value in signed 32-bit range",
            x.to_repr(),
            what
        )));
    }
    Err(eval_error(format!(
        "got {} for {}, want int",
        x.get_type(),
        what
    )))
}

/// `StringUtilities.replaceAllLiteral` on Bazel's internal (Latin-1-of-UTF-8) strings: the same
/// replacement on the raw bytes.
// PORT: StringEncoding; Java reads the template as ISO_8859_1, so chars are bytes.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:70 StringUtilities.replaceAllLiteral
fn replace_all_literal(input: &[u8], literal: &[u8], replacement: &[u8]) -> Vec<u8> {
    let literal_length = literal.len();
    if literal_length == 0 {
        return input.to_vec();
    }
    let mut result = Vec::with_capacity(input.len() + replacement.len());
    let mut start = 0;
    while let Some(offset) = input[start..]
        .windows(literal_length)
        .position(|w| w == literal)
    {
        let index = start + offset;
        result.extend_from_slice(&input[start..index]);
        result.extend_from_slice(replacement);
        start = index + literal_length;
    }
    result.extend_from_slice(&input[start..]);
    result
}

/// Java `PathPackageLocator.getWorkspaceFile(syscallCache)`.
// PORT: move to crates/blaze_sky_loading/src/pkgcache.rs (PathPackageLocator.getWorkspaceFile)
// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:264 PathPackageLocator.getWorkspaceFile
fn get_workspace_file(package_locator: &PathPackageLocator) -> Option<Path> {
    // TODO(bazel-team): correctness in the presence of changes to the location of the WORKSPACE
    //  file.
    let workspace_file = get_file_path(package_locator, &WORKSPACE_DOT_BAZEL_FILE_NAME);
    if workspace_file.is_some() {
        return workspace_file;
    }
    get_file_path(package_locator, &WORKSPACE_FILE_NAME)
}

// PORT: move to crates/blaze_sky_loading/src/pkgcache.rs (it has a private copy)
// Java: com/google/devtools/build/lib/pkgcache/PathPackageLocator.java:275 PathPackageLocator.getFilePath
fn get_file_path(package_locator: &PathPackageLocator, suffix: &PathFragment) -> Option<Path> {
    for path_entry in package_locator.get_path_entries() {
        let build_file = path_entry.get_relative(suffix);
        let type_ = match build_file.stat_if_found(Symlinks::Follow) {
            // Treat IOException as a missing file.
            Ok(Some(stat)) => Some(stat.get_dirent_type()),
            Ok(None) | Err(_) => None,
        };
        if type_ == Some(DirentType::File) || type_ == Some(DirentType::Unknown) {
            return Some(build_file);
        }
    }
    None
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:174 StarlarkRepositoryContext.externalPath
fn external_path(
    ctx: &mut ExternalContext,
    method: &str,
    path_object: Value<'_>,
) -> starlark::Result<StarlarkPath> {
    let starlark_path = ctx.get_path(path_object)?;
    let path = starlark_path.path.clone();
    let in_working_directory = path.starts_with(&ctx.base.working_directory);
    let repo = ctx.repository();
    if repo.package_locator.get_path_entries().iter().all(|root| {
        // Java: root.asPath() is non-null for package path entries.
        !path.starts_with(
            root.as_path()
                .expect("package path entry is not an absolute root"),
        )
    }) || in_working_directory
    {
        return Ok(starlark_path);
    }
    let Some(workspace_file) = get_workspace_file(&repo.package_locator) else {
        // Java: getWorkspaceFile returns null, then getParentDirectory() fails.
        panic!("java.lang.NullPointerException");
    };
    let workspace_root = workspace_file
        .get_parent_directory()
        .expect("workspace file has a parent");
    let relative_path = path.relative_to(&workspace_root);
    if repo
        .ignored_subdirectories
        .matching_entry(&relative_path)
        .is_some()
    {
        return Ok(starlark_path);
    }
    Err(eval_error(format!(
        "{} can only be applied to external paths (that is, outside the workspace or ignored in \
         .bazelignore)",
        method
    )))
}

/// `repository_ctx` methods (registered on [`RepositoryCtx`] together with the base methods).
#[starlark_module]
pub fn repository_ctx_methods(builder: &mut MethodsBuilder) {
    /// The canonical name of the external repository created by this rule. This name is
    /// guaranteed to be unique among all external repositories, but its exact format is not
    /// specified. Use `original_name` instead to get the name that was originally specified as
    /// the `name` when this repository rule was instantiated.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:133 StarlarkRepositoryContext.getName
    #[starlark(attribute)]
    fn name(this: &RepositoryCtx) -> starlark::Result<String> {
        Ok(this.repo_definition.name.clone())
    }

    /// The name that was originally specified as the `name` attribute when this repository rule
    /// was instantiated. This name is not necessarily unique among external repositories. Use
    /// `name` instead to get the canonical name of the external repository.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:145 StarlarkRepositoryContext.getOriginalName
    #[starlark(attribute)]
    fn original_name(this: &RepositoryCtx) -> starlark::Result<String> {
        // The original name isn't set for repositories backing Bazel modules, in which case the
        // original name doesn't matter as the restricted set of rules that can back
        // Bazel modules do not use the name.
        let repo_definition = &this.repo_definition;
        Ok(match repo_definition.original_name.as_deref() {
            None | Some("") => repo_definition.name.clone(),
            Some(original_name) => original_name.to_owned(),
        })
    }

    /// The path to the root workspace of the bazel invocation.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:158 StarlarkRepositoryContext.getWorkspaceRoot
    #[starlark(attribute)]
    fn workspace_root(this: &RepositoryCtx) -> starlark::Result<StarlarkPath> {
        Ok(StarlarkPath::new(this.workspace_root.clone()))
    }

    /// A struct to access the values of the attributes. The values are provided by the user (if
    /// not, a default value is used).
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:170 StarlarkRepositoryContext.getAttr
    #[starlark(attribute)]
    fn attr(this: &RepositoryCtx) -> starlark::Result<RepoAttrs> {
        Ok(RepoAttrs(this.repo_definition.clone()))
    }

    /// Creates a symlink on the filesystem.
    // This method must not be moved to ModuleExtensionContext as the FS-specific watch in its
    // implementation would cause lock files to differ across platforms. If this is ever needed,
    // add a `copy` method instead.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:218 StarlarkRepositoryContext.symlink
    fn symlink<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] target: Value<'v>,
        #[starlark(require = pos)] link_name: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_string_label_or_path("symlink", "target", target)?;
        check_string_label_or_path("symlink", "link_name", link_name)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        let target_path = ctx.get_path(target)?;
        let link_path = ctx.get_path(link_name)?;
        // PORT: WorkspaceRuleEvent.newSymlinkEvent not posted (see module doc).
        ctx.base
            .check_in_output_directory("write", &link_path)
            .map_err(repository_function_exception_eval)?;
        let created = (|| -> std::io::Result<bool> {
            StarlarkBaseExternalContext::make_directories(&link_path.path)?;
            link_path.path.create_symbolic_link(&target_path.path)?;
            Ok(!link_path
                .path
                .get_file_system()
                .supports_symbolic_links_natively(link_path.path.as_fragment()))
        })();
        match created {
            Ok(emulated) => {
                if emulated {
                    // The symlink may be emulated as a copy, which would need to be tracked for
                    // invalidation.
                    ctx.maybe_watch(&target_path, ShouldWatch::Auto)?;
                }
                Ok(NoneType)
            }
            Err(e) => Err(repository_function_exception_io(IoException::new(format!(
                "Could not create symlink from {} to {}: {}",
                target_path,
                link_path,
                IoException::from(e).get_message()
            )))),
        }
    }

    /// Generates a new file using a `template`. Every occurrence in `template` of a key of
    /// `substitutions` will be replaced by the corresponding value. The result is written in
    /// `path`. An optional `executable` argument (default to true) can be set to turn on or off
    /// the executable bit.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:311 StarlarkRepositoryContext.createFileFromTemplate
    fn template<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        #[starlark(require = pos)] template: Value<'v>,
        substitutions: Option<Value<'v>>,
        executable: Option<Value<'v>>,
        #[starlark(require = named)] watch_template: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_string_label_or_path("template", "path", path)?;
        check_string_label_or_path("template", "template", template)?;
        if let Some(substitutions) = substitutions {
            check_param_type(
                "template",
                "substitutions",
                substitutions,
                DictRef::from_value(substitutions).is_some(),
                "dict",
            )?;
        }
        if let Some(executable) = executable {
            check_param_type(
                "template",
                "executable",
                executable,
                executable.unpack_bool().is_some(),
                "bool",
            )?;
        }
        if let Some(watch_template) = watch_template {
            check_param_type(
                "template",
                "watch_template",
                watch_template,
                watch_template.unpack_str().is_some(),
                "string",
            )?;
        }
        let executable = executable.and_then(|v| v.unpack_bool()).unwrap_or(true);
        let watch_template = watch_template
            .and_then(|v| v.unpack_str())
            .unwrap_or("auto");

        let mut ctx = ExternalContext::from_eval(eval)?;
        let p = ctx.get_path(path)?;
        let t = ctx.get_path(template)?;
        let substitution_map = match substitutions {
            Some(substitutions) => {
                dict_cast_string_string(substitutions, "substitutions").map_err(eval_error)?
            }
            None => IndexMap::new(),
        };
        // PORT: WorkspaceRuleEvent.newTemplateEvent not posted (see module doc).
        if t.path.is_directory() {
            return Err(eval_error(format!(
                "attempting to use a directory as template: {}",
                t
            )));
        }
        let should_watch = ShouldWatch::from_string(watch_template)?;
        ctx.maybe_watch(&t, should_watch)?;
        ctx.base
            .check_in_output_directory("write", &p)
            .map_err(repository_function_exception_eval)?;
        let written = (|| -> std::io::Result<()> {
            StarlarkBaseExternalContext::make_directories(&p.path)?;
            // Read and write files as raw bytes by using the Latin-1 encoding, which matches the
            // encoding used by Bazel for strings.
            let mut tpl = blaze_repo_fetch::fs_utils::read_content(&t.path)?;
            for (key, value) in &substitution_map {
                tpl = replace_all_literal(&tpl, key.as_bytes(), value.as_bytes());
            }
            p.path.delete()?;
            {
                let mut stream = p.path.get_output_stream()?;
                stream.write_all(&tpl)?;
                stream.flush()?;
            }
            if executable {
                p.path.set_executable(true)?;
            }
            Ok(())
        })();
        written.map_err(|e| repository_function_exception_io(IoException::from(e)))?;
        Ok(NoneType)
    }

    /// Deletes a file or a directory. Returns a bool, indicating whether the file or directory
    /// was actually deleted by this call.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:391 StarlarkRepositoryContext.delete
    fn delete<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        check_param_type(
            "delete",
            "path",
            path,
            is_string_or_path(path),
            "string or path",
        )?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        let starlark_path = external_path(&mut ctx, "delete()", path)?;
        // PORT: WorkspaceRuleEvent.newDeleteEvent not posted (see module doc).
        let path = &starlark_path.path;
        let deleted = (|| -> std::io::Result<bool> {
            path.delete_trees_below()?;
            path.delete()
        })();
        deleted.map_err(|e| repository_function_exception_io(IoException::from(e)))
    }

    /// Renames the file or directory from `src` to `dst`. Parent directories are created as
    /// needed. Fails if the destination path already exists. Both paths must be located within
    /// the repository.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:442 StarlarkRepositoryContext.rename
    fn rename<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] src: Value<'v>,
        #[starlark(require = pos)] dst: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_string_label_or_path("rename", "src", src)?;
        check_string_label_or_path("rename", "dst", dst)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        let src_path = ctx.get_path(src)?;
        let dst_path = ctx.get_path(dst)?;
        // PORT: WorkspaceRuleEvent.newRenameEvent not posted (see module doc).
        ctx.base
            .check_in_output_directory("write", &src_path)
            .map_err(repository_function_exception_eval)?;
        ctx.base
            .check_in_output_directory("write", &dst_path)
            .map_err(repository_function_exception_eval)?;
        if dst_path.path.exists() {
            return Err(repository_function_exception_io(IoException::new(format!(
                "Could not rename {} to {}: already exists",
                src_path, dst_path
            ))));
        }
        let renamed = (|| -> std::io::Result<()> {
            StarlarkBaseExternalContext::make_directories(&dst_path.path)?;
            src_path.path.rename_to(&dst_path.path)
        })();
        renamed.map_err(|e| {
            repository_function_exception_io(IoException::new(format!(
                "Could not rename {} to {}: {}",
                src_path,
                dst_path,
                IoException::from(e).get_message()
            )))
        })?;
        Ok(NoneType)
    }

    /// Apply a patch file to the root directory of external repository, or to `directory` within
    /// it. The patch file should be a standard unified diff format file. The Bazel-native patch
    /// implementation doesn't support binary patch like the patch command line tool.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:536 StarlarkRepositoryContext.patch
    fn patch<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] patch_file: Value<'v>,
        strip: Option<Value<'v>>,
        #[starlark(require = named)] directory: Option<Value<'v>>,
        #[starlark(require = named)] watch_patch: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_string_label_or_path("patch", "patch_file", patch_file)?;
        if let Some(strip) = strip {
            check_param_type("patch", "strip", strip, is_int(strip), "int")?;
        }
        if let Some(directory) = directory {
            check_string_label_or_path("patch", "directory", directory)?;
        }
        if let Some(watch_patch) = watch_patch {
            check_param_type(
                "patch",
                "watch_patch",
                watch_patch,
                watch_patch.unpack_str().is_some(),
                "string",
            )?;
        }
        let strip = match strip {
            Some(strip) => to_int(strip, "strip")?,
            None => 0,
        };
        let directory = directory.unwrap_or_else(|| eval.heap().alloc_str("").to_value());
        let watch_patch = watch_patch.and_then(|v| v.unpack_str()).unwrap_or("auto");

        let mut ctx = ExternalContext::from_eval(eval)?;
        let starlark_path = ctx.get_path(patch_file)?;
        let directory_path = ctx.get_path(directory)?;
        ctx.base
            .check_in_output_directory("write", &directory_path)
            .map_err(repository_function_exception_eval)?;
        // PORT: WorkspaceRuleEvent.newPatchEvent not posted (see module doc).
        if starlark_path.path.is_directory() {
            return Err(eval_error(format!(
                "attempting to use a directory as patch file: {}",
                starlark_path
            )));
        }
        let should_watch = ShouldWatch::from_string(watch_patch)?;
        ctx.maybe_watch(&starlark_path, should_watch)?;
        match patch_util::apply(&starlark_path.path, strip, &directory_path.path) {
            Ok(()) => Ok(NoneType),
            Err(PatchError::Patch(e)) => Err(repository_function_exception_eval(format!(
                "Error applying patch {}: {}",
                starlark_path, e.message
            ))),
            Err(PatchError::Io(e)) => Err(repository_function_exception_io(e)),
        }
    }

    /// Tells Bazel to watch for changes to any files or directories transitively under the given
    /// path. Any changes to the contents of files, the existence of files or directories, file
    /// names or directory names, will cause this repo to be refetched.
    ///
    /// Note that attempting to watch paths inside the repo currently being fetched will result in
    /// an error.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:589 StarlarkRepositoryContext.watchTree
    fn watch_tree<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = pos)] path: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        check_string_label_or_path("watch_tree", "path", path)?;
        let mut ctx = ExternalContext::from_eval(eval)?;
        let p = ctx.get_path(path)?;
        if !p.path.is_directory() {
            return Err(eval_error(format!(
                "can't call watch_tree() on non-directory {}",
                p
            )));
        }
        let Some(repo_cache_friendly_path) =
            ctx.to_repo_cache_friendly_path(&p.path, ShouldWatch::Yes)?
        else {
            return Ok(NoneType);
        };
        // DirTree can only be recorded for directories, so we have to additionally track the type
        // of the file. When checking for invalidation, the type is verified first and if it
        // doesn't match, the directory entries are never requested.
        ctx.get_value_and_record_input(RepoRecordedInput::File(repo_cache_friendly_path.clone()))?;
        ctx.get_value_and_record_input(RepoRecordedInput::DirTree(repo_cache_friendly_path))?;
        Ok(NoneType)
    }

    /// Constructs an opaque object that can be returned from the repo rule's implementation
    /// function to provide metadata about its reproducibility.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:647 StarlarkRepositoryContext.repoMetadata
    fn repo_metadata<'v>(
        #[starlark(this)] _this: Value<'v>,
        #[starlark(require = named)] reproducible: Option<Value<'v>>,
        #[starlark(require = named)] attrs_for_reproducibility: Option<Value<'v>>,
    ) -> starlark::Result<RepoMetadataValue> {
        if let Some(reproducible) = reproducible {
            check_param_type(
                "repo_metadata",
                "reproducible",
                reproducible,
                reproducible.unpack_bool().is_some(),
                "bool",
            )?;
        }
        if let Some(attrs) = attrs_for_reproducibility {
            check_param_type(
                "repo_metadata",
                "attrs_for_reproducibility",
                attrs,
                DictRef::from_value(attrs).is_some(),
                "dict",
            )?;
        }
        let reproducible = reproducible.and_then(|v| v.unpack_bool()).unwrap_or(false);
        let attrs_is_empty = attrs_for_reproducibility
            .is_none_or(|v| DictRef::from_value(v).is_none_or(|d| d.is_empty()));
        if reproducible && !attrs_is_empty {
            return Err(eval_error(
                "attrs_for_reproducibility can only be specified if reproducible is False",
            ));
        }
        let attrs = match attrs_for_reproducibility {
            Some(attrs) => {
                attrs_from_starlark_dict(attrs, "attrs_for_reproducibility").map_err(eval_error)?
            }
            None => IndexMap::new(),
        };
        Ok(RepoMetadataValue(RepoMetadata::new(
            if reproducible {
                Reproducibility::Yes
            } else {
                Reproducibility::No
            },
            attrs,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::replace_all_literal;

    #[test]
    fn replace_all_literal_empty_literal_is_identity() {
        assert_eq!(replace_all_literal(b"abc", b"", b"x"), b"abc".to_vec());
    }

    #[test]
    fn replace_all_literal_replaces_left_to_right() {
        assert_eq!(replace_all_literal(b"aaa", b"aa", b"b"), b"ba".to_vec());
        assert_eq!(
            replace_all_literal(b"%{x} and %{x}", b"%{x}", b"1"),
            b"1 and 1".to_vec()
        );
        assert_eq!(
            replace_all_literal(b"\xff%{x}\xfe", b"%{x}", b"\xc3\xa9"),
            b"\xff\xc3\xa9\xfe".to_vec()
        );
    }
}
