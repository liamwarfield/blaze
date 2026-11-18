//! `path`: a file path used by repository rules and module extensions.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java
//!
//! PORT: Java's `StarlarkPath` keeps a reference to its `StarlarkBaseExternalContext` (used by
//! `readdir` to record a watch). Starlark values cannot hold the context in Blaze, so `readdir`
//! fetches it from the evaluation thread ([`ExternalContext::from_eval`]).

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_starlark::printer::quote;
use blaze_starlark::structs::DeeplyImmutable;
use blaze_vfs::Path;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark_map::StarlarkHasher;

use super::base_external_context::ShouldWatch;
use super::base_external_context::eval_error;
use super::context::ExternalContext;

/// A Path object to be used in repo rules and module extensions.
///
/// This path object enables non-hermetic operations from Starlark and should not be returned by
/// something other than a `StarlarkBaseExternalContext`.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:49 StarlarkPath
#[derive(Clone, Debug, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkPath {
    #[allocative(skip)]
    pub path: Path,
}

starlark_simple_value!(StarlarkPath);

impl StarlarkPath {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:53 StarlarkPath.<init>
    pub fn new(path: Path) -> Self {
        StarlarkPath { path }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:58 StarlarkPath.getPath
    pub fn get_path(&self) -> &Path {
        &self.path
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:81 StarlarkPath.getBasename
    pub fn get_basename(&self) -> String {
        self.path.get_base_name().to_owned()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:133 StarlarkPath.getDirname
    pub fn get_dirname(&self) -> Option<StarlarkPath> {
        self.path.get_parent_directory().map(StarlarkPath::new)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:168 StarlarkPath.exists
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:182 StarlarkPath.isDir
    pub fn is_dir(&self) -> bool {
        self.path.is_directory()
    }
}

impl fmt::Display for StarlarkPath {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:199 StarlarkPath.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)
    }
}

#[starlark_value(type = "path")]
impl<'v> StarlarkValue<'v> for StarlarkPath {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("path", starlark_path_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:209 StarlarkPath.repr
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&quote(&self.path.to_string()));
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:204 StarlarkPath.str
    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&self.path.to_string());
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:68 StarlarkPath.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<StarlarkPath>()
            .is_some_and(|o| o.path == self.path))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:73 StarlarkPath.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.path.hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:63 StarlarkPath.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn starlark_path_methods(builder: &mut MethodsBuilder) {
    /// A string giving the basename of the file.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:81 StarlarkPath.getBasename
    #[starlark(attribute)]
    fn basename(this: &StarlarkPath) -> starlark::Result<String> {
        Ok(this.get_basename())
    }

    /// Returns the list of entries in the directory denoted by this path.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:110 StarlarkPath.readdir
    fn readdir<'v>(
        this: &StarlarkPath,
        #[starlark(require = named, default = "auto")] watch: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Vec<StarlarkPath>> {
        if !this.is_dir() {
            return Err(eval_error(format!(
                "can't readdir(), not a directory: {}",
                this.path
            )));
        }
        {
            let mut ctx = ExternalContext::from_eval(eval)?;
            ctx.maybe_watch_dirents(&this.path, ShouldWatch::from_string(watch)?)?;
        }
        match this.path.get_directory_entries() {
            Ok(entries) => Ok(entries.into_iter().map(StarlarkPath::new).collect()),
            // Java: RepositoryFunctionException(e, Transience.TRANSIENT)
            Err(e) => Err(eval_error(e.to_string())),
        }
    }

    /// The parent directory of this file, or None if this file does not have a parent.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:133 StarlarkPath.getDirname
    #[starlark(attribute)]
    fn dirname(this: &StarlarkPath) -> starlark::Result<NoneOr<StarlarkPath>> {
        Ok(match this.get_dirname() {
            Some(p) => NoneOr::Other(p),
            None => NoneOr::None,
        })
    }

    /// Returns the path obtained by joining this path with the given relative paths.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:149 StarlarkPath.getChild
    fn get_child<'v>(
        this: &StarlarkPath,
        #[starlark(args)] relative_paths: UnpackTuple<Value<'v>>,
    ) -> starlark::Result<StarlarkPath> {
        // Java: Sequence.cast(relativePaths, String.class, "relative_paths")
        let mut parts = Vec::with_capacity(relative_paths.items.len());
        for (i, v) in relative_paths.items.iter().enumerate() {
            match v.unpack_str() {
                Some(s) => parts.push(s),
                None => {
                    return Err(eval_error(format!(
                        "at index {} of relative_paths, got element of type {}, want string",
                        i,
                        v.get_type()
                    )));
                }
            }
        }
        Ok(StarlarkPath::new(
            this.path.get_relative_str(&parts.join("/")),
        ))
    }

    /// Returns true if the file or directory denoted by this path exists.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:168 StarlarkPath.exists
    #[starlark(attribute)]
    fn exists(this: &StarlarkPath) -> starlark::Result<bool> {
        Ok(this.exists())
    }

    /// Returns true if this path points to a directory.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:182 StarlarkPath.isDir
    #[starlark(attribute)]
    fn is_dir(this: &StarlarkPath) -> starlark::Result<bool> {
        Ok(this.is_dir())
    }

    /// Returns the canonical path for this path by repeatedly replacing all symbolic links with
    /// their referents.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPath.java:194 StarlarkPath.realpath
    #[starlark(attribute)]
    fn realpath(this: &StarlarkPath) -> starlark::Result<StarlarkPath> {
        // Java: the IOException becomes an EvalException carrying its message.
        this.path
            .resolve_symbolic_links()
            .map(StarlarkPath::new)
            .map_err(|e| eval_error(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    //! Port of src/test/java/com/google/devtools/build/lib/bazel/repository/starlark/StarlarkPathTest.java

    use std::sync::Arc;
    use std::sync::LazyLock;

    use blaze_vfs::DigestHashFunction;
    use blaze_vfs::FileSystemRef;
    use blaze_vfs::UnixFileSystem;
    use starlark::environment::Globals;
    use starlark::environment::Module;
    use starlark::eval::Evaluator;
    use starlark::syntax::AstModule;
    use starlark::syntax::Dialect;

    use super::*;

    /// One file system for all paths of the tests (`Path` equality compares file systems).
    static FS: LazyLock<FileSystemRef> =
        LazyLock::new(|| Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256)));

    fn wd() -> Path {
        Path::create_from_str("/wd", &FS)
    }

    /// Evaluates `expr` with `wd` bound to `StarlarkPath(wd)` and passes the result to `check`.
    fn eval_with_wd(expr: &str, check: impl FnOnce(Value<'_>)) {
        Module::with_temp_heap(|module| {
            module.set("wd", module.heap().alloc(StarlarkPath::new(wd())));
            let ast = AstModule::parse("test.bzl", expr.to_owned(), &Dialect::Standard).unwrap();
            let globals = Globals::standard();
            let mut eval = Evaluator::new(&module);
            let v = eval.eval_module(ast, &globals).unwrap();
            check(v);
        })
    }

    fn assert_path(expr: &str, expected: Path) {
        eval_with_wd(expr, |v| {
            assert_eq!(
                v.downcast_ref::<StarlarkPath>().map(|p| p.path.clone()),
                Some(expected)
            );
        });
    }

    #[test]
    fn test_starlark_path_get_child() {
        let wd = wd();
        assert_path("wd.get_child()", wd.clone());
        assert_path("wd.get_child('foo')", wd.get_child("foo"));
        assert_path(
            "wd.get_child('a','b/c','/d/')",
            wd.get_relative_str("a/b/c/d"),
        );
    }

    #[test]
    fn test_starlark_path_stringifications() {
        let wd = wd();
        eval_with_wd("repr(wd)", |v| {
            assert_eq!(v.unpack_str(), Some(quote(&wd.to_string()).as_str()))
        });
        eval_with_wd("str(wd)", |v| {
            assert_eq!(v.unpack_str(), Some(wd.to_string().as_str()))
        });
    }
}
