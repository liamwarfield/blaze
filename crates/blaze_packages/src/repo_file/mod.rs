//! REPO.bazel: the `repo()` and `ignore_directories()` globals and their thread context.
//!
//! Java: com/google/devtools/build/lib/packages/{RepoFileGlobals,RepoThreadContext}.java.
//! Ownership: D (crates/blaze_packages/DESIGN.md). The SkyFunction that evaluates REPO.bazel is
//! `blaze_sky_loading::repo_file`.
//!
//! The `repo()` keyword arguments are Starlark values. They must outlive the evaluation, so
//! `repo()` stores them as a dict in the module's extra value (`Module::set_extra_value`); the
//! frozen module carries them out (`FrozenModule::owned_extra_value`). The thread context keeps
//! the flags and the ignored directories.

use blaze_starlark::ThreadContext;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneType;

/// Context object for a Starlark thread evaluating the REPO.bazel file. Stored as the
/// [`ThreadContext`] payload.
// Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:28 RepoThreadContext
#[derive(Debug, Default)]
pub struct RepoThreadContext {
    repo_function_called: bool,
    ignored_directories: Vec<String>,
    ignored_directories_set: bool,
}

impl RepoThreadContext {
    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:44 RepoThreadContext.<init>
    pub fn new() -> RepoThreadContext {
        RepoThreadContext::default()
    }

    /// Fails unless the thread evaluates a REPO.bazel file.
    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:35 RepoThreadContext.fromOrFail
    pub fn from_or_fail<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> anyhow::Result<std::cell::RefMut<'a, RepoThreadContext>> {
        let fail = || anyhow::anyhow!("{} can only be called from REPO.bazel", what);
        let cx = ThreadContext::get(eval).map_err(|_| fail())?;
        cx.payload::<RepoThreadContext>().ok_or_else(fail)
    }

    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:48 RepoThreadContext.isRepoFunctionCalled
    pub fn is_repo_function_called(&self) -> bool {
        self.repo_function_called
    }

    /// Records that `repo()` was called. PORT: the kwargs themselves are stored in the module's
    /// extra value by `repo()` (see the module doc), not here.
    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:52 RepoThreadContext.setPackageArgsMap
    pub fn set_package_args_map(&mut self) {
        self.repo_function_called = true;
    }

    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:61 RepoThreadContext.setIgnoredDirectories
    pub fn set_ignored_directories(&mut self, ignored_directories: Vec<String>) {
        self.ignored_directories_set = true;
        self.ignored_directories = ignored_directories;
    }

    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:66 RepoThreadContext.isIgnoredDirectoriesSet
    pub fn is_ignored_directories_set(&self) -> bool {
        self.ignored_directories_set
    }

    // Java: com/google/devtools/build/lib/packages/RepoThreadContext.java:70 RepoThreadContext.getIgnoredDirectories
    pub fn get_ignored_directories(&self) -> &[String] {
        &self.ignored_directories
    }
}

/// Definition of the `repo()` and `ignore_directories()` functions used in REPO.bazel files.
/// Register for `FileKind::RepoFile` (Java `StarlarkGlobalsImpl.getRepoToplevels`).
// Java: com/google/devtools/build/lib/packages/RepoFileGlobals.java:31 RepoFileGlobals
#[starlark_module]
pub fn register_repo_file_globals(builder: &mut GlobalsBuilder) {
    /// The list of directories to ignore in this repository.
    ///
    /// This function takes a list of strings and a directory is ignored if any of the given
    /// strings matches its repository-relative path according to the semantics of the `glob()`
    /// function. This function can be used to ignore directories that are implementation details
    /// of source control systems, output files of other build systems, etc.
    // Java: com/google/devtools/build/lib/packages/RepoFileGlobals.java:51 RepoFileGlobals.ignoreDirectories
    fn ignore_directories<'v>(
        dirs: UnpackListOrTuple<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut context = RepoThreadContext::from_or_fail(eval, "repo()")?;
        if context.is_ignored_directories_set() {
            return Err(anyhow::anyhow!(
                "'ignored_directories()' can only be called once"
            ));
        }
        context.set_ignored_directories(dirs.items);
        Ok(NoneType)
    }

    /// Declares metadata that applies to every rule in the repository. It must be called at most
    /// once per REPO.bazel file. If called, it must be the first call in the REPO.bazel file.
    ///
    /// The `repo()` function accepts exactly the same arguments as the `package()` function in
    /// BUILD files.
    // Java: com/google/devtools/build/lib/packages/RepoFileGlobals.java:78 RepoFileGlobals.repoCallable
    fn repo<'v>(
        #[starlark(kwargs)] kwargs: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        {
            let mut context = RepoThreadContext::from_or_fail(eval, "repo()")?;
            if context.is_repo_function_called() {
                return Err(anyhow::anyhow!(
                    "'repo' can only be called once in the REPO.bazel file"
                ));
            }
            if context.is_ignored_directories_set() {
                return Err(anyhow::anyhow!(
                    "if repo() is called, it must be called before any other functions"
                ));
            }
            if kwargs.length().map_err(starlark::Error::into_anyhow)? == 0 {
                return Err(anyhow::anyhow!(
                    "at least one argument must be given to the 'repo' function"
                ));
            }
            context.set_package_args_map();
        }
        eval.module().set_extra_value(kwargs);
        Ok(NoneType)
    }
}
