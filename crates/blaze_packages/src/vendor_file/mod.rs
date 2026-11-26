//! VENDOR.bazel: the `ignore()` and `pin()` globals and their thread context.
//!
//! Java: com/google/devtools/build/lib/packages/{VendorFileGlobals,VendorThreadContext}.java. The
//! SkyFunction that evaluates VENDOR.bazel is
//! `blaze_repo_rules::bzlmod::vendor_file_function`.

use blaze_cmdline::RepositoryName;
use blaze_starlark::ThreadContext;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;

/// Context object for a Starlark thread evaluating the VENDOR.bazel file. Stored as the
/// [`ThreadContext`] payload.
// Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:25 VendorThreadContext
#[derive(Debug, Default)]
pub struct VendorThreadContext {
    ignored_repos: Vec<RepositoryName>,
    pinned_repos: Vec<RepositoryName>,
}

impl VendorThreadContext {
    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:41 VendorThreadContext.<init>
    pub fn new() -> VendorThreadContext {
        VendorThreadContext::default()
    }

    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:30 VendorThreadContext.fromOrFail
    pub fn from_or_fail<'a>(
        eval: &Evaluator<'_, 'a, '_>,
        what: &str,
    ) -> anyhow::Result<std::cell::RefMut<'a, VendorThreadContext>> {
        let fail = || anyhow::anyhow!("{} can only be called from VENDOR.bazel", what);
        let cx = ThreadContext::get(eval).map_err(|_| fail())?;
        cx.payload::<VendorThreadContext>().ok_or_else(fail)
    }

    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:43 VendorThreadContext.getIgnoredRepos
    pub fn get_ignored_repos(&self) -> Vec<RepositoryName> {
        self.ignored_repos.clone()
    }

    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:47 VendorThreadContext.getPinnedRepos
    pub fn get_pinned_repos(&self) -> Vec<RepositoryName> {
        self.pinned_repos.clone()
    }

    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:51 VendorThreadContext.addIgnoredRepo
    pub fn add_ignored_repo(&mut self, repo_name: RepositoryName) {
        self.ignored_repos.push(repo_name);
    }

    // Java: com/google/devtools/build/lib/packages/VendorThreadContext.java:55 VendorThreadContext.addPinnedRepo
    pub fn add_pinned_repo(&mut self, repo_name: RepositoryName) {
        self.pinned_repos.push(repo_name);
    }
}

/// `Sequence.cast(args, String.class, "args")`.
// Java: net/starlark/java/eval/Sequence.java Sequence.cast
fn cast_strings(args: &UnpackTuple<Value<'_>>) -> anyhow::Result<Vec<String>> {
    args.items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            v.unpack_str().map(str::to_string).ok_or_else(|| {
                anyhow::anyhow!(
                    "at index {} of args, got element of type {}, want string",
                    i,
                    v.get_type()
                )
            })
        })
        .collect()
}

// Java: com/google/devtools/build/lib/packages/VendorFileGlobals.java:68 VendorFileGlobals.getRepositoryName
fn get_repository_name(repo_name: &str) -> anyhow::Result<RepositoryName> {
    let Some(name) = repo_name.strip_prefix("@@") else {
        return Err(anyhow::anyhow!(
            "the canonical repository name must start with `@@`"
        ));
    };
    RepositoryName::create(name)
        .map_err(|e| anyhow::anyhow!("Invalid canonical repo name: {}", e.get_message()))
}

/// Definition of the functions used in VENDOR.bazel file. Register for `FileKind::VendorFile`
/// (Java `StarlarkGlobalsImpl.getVendorToplevels`).
// Java: com/google/devtools/build/lib/packages/VendorFileGlobals.java:28 VendorFileGlobals
#[starlark_module]
pub fn register_vendor_file_globals(builder: &mut GlobalsBuilder) {
    /// Ignore this repo from vendoring. Bazel will never vendor it or use the corresponding
    /// directory (if exists) while building in vendor mode.
    // Java: com/google/devtools/build/lib/packages/VendorFileGlobals.java:42 VendorFileGlobals.ignore
    fn ignore<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut context = VendorThreadContext::from_or_fail(eval, "ignore()")?;
        for repo_name in cast_strings(&args)? {
            context.add_ignored_repo(get_repository_name(&repo_name)?);
        }
        Ok(NoneType)
    }

    /// Pin the contents of this repo under the vendor directory. Bazel will not update this repo
    /// while vendoring, and will use the vendored source as if there is a --override_repository
    /// flag when building in vendor mode
    // Java: com/google/devtools/build/lib/packages/VendorFileGlobals.java:58 VendorFileGlobals.pin
    fn pin<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let mut context = VendorThreadContext::from_or_fail(eval, "pin()")?;
        for repo_name in cast_strings(&args)? {
            context.add_pinned_repo(get_repository_name(&repo_name)?);
        }
        Ok(NoneType)
    }
}
