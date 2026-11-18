//! Ports of `packages/GlobCache.java`, `packages/NonSkyframeGlobber.java` and
//! `packages/GlobberUtils.java`: the non-Skyframe [`Globber`] over the filesystem.
//!
//! Agent D's Skyframe-backed globber (Java `PackageFunction`'s GLOBS-based globber) implements
//! the same [`Globber`] trait; the package builder holds whichever is in use.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use blaze_cmdline::PackageIdentifier;
use blaze_vfs::Dirent;
use blaze_vfs::FileStatus;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::Symlinks;
use parking_lot::Mutex;

use crate::build::unix_glob;
use crate::build::unix_glob::FilesystemOps;
use crate::build::unix_glob::UnixGlobBuilder;
use crate::build::unix_glob::UnixGlobError;
use crate::build::unix_glob::UnixGlobPathDiscriminator;
use crate::package::BadGlobException;
use crate::package::Globber;
use crate::package::GlobberError;
use crate::package::GlobberOperation;
use crate::package::GlobberToken;

/// Finds the BUILD file of a package, if it exists (Java `CachingPackageLocator`).
// Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:31 CachingPackageLocator
pub trait CachingPackageLocator: Send + Sync {
    // Java: com/google/devtools/build/lib/packages/CachingPackageLocator.java:45 CachingPackageLocator.getBuildFileForPackage
    fn get_build_file_for_package(&self, package_name: &PackageIdentifier) -> Option<Path>;
}

/// The part of Java `IgnoredSubdirectories` globbing uses (agent D's
/// `blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectories` implements it).
// Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:170 IgnoredSubdirectories.matchingEntry
pub trait IgnoredSubdirectoriesMatcher: Send + Sync {
    /// The ignore entry matching `directory`, or `None`.
    fn matching_entry(&self, directory: &PathFragment) -> Option<String>;
}

/// Throws the error for a pattern without matches (Java
/// `GlobberUtils.throwBadGlobExceptionEmptyResult`).
// Java: com/google/devtools/build/lib/packages/GlobberUtils.java:24 GlobberUtils.throwBadGlobExceptionEmptyResult
pub fn bad_glob_exception_empty_result(
    pattern: &str,
    globber_operation: GlobberOperation,
) -> BadGlobException {
    match globber_operation {
        GlobberOperation::Subpackages => BadGlobException(format!(
            "subpackages pattern '{pattern}' didn't match anything, but allow_empty is set to \
             False (the default value)"
        )),
        _ => BadGlobException(format!(
            "glob pattern '{pattern}' didn't match anything, but allow_empty is set to False (the \
             default value of allow_empty can be set with --incompatible_disallow_empty_glob)."
        )),
    }
}

/// Throws the error for a glob whose matches were all excluded (Java
/// `GlobberUtils.throwBadGlobExceptionAllExcluded`).
// Java: com/google/devtools/build/lib/packages/GlobberUtils.java:42 GlobberUtils.throwBadGlobExceptionAllExcluded
pub fn bad_glob_exception_all_excluded(globber_operation: GlobberOperation) -> BadGlobException {
    match globber_operation {
        GlobberOperation::Subpackages => BadGlobException(
            "all subpackages in subpackages() have been excluded, but allow_empty is set to False "
                .to_owned(),
        ),
        _ => BadGlobException(
            "all files in the glob have been excluded, but allow_empty is set to False (the \
             default value of allow_empty can be set with --incompatible_disallow_empty_glob)."
                .to_owned(),
        ),
    }
}

/// The result of one glob pattern (Java `Future<List<Path>>`, computed eagerly).
type GlobResult = Result<Arc<Vec<Path>>, Arc<io::Error>>;

/// Caches the results of glob patterns of one package (Java `GlobCache`).
// Java: com/google/devtools/build/lib/packages/GlobCache.java:60 GlobCache
pub struct GlobCache {
    /// A mapping from glob expressions (e.g. "*.java") to the list of files it matched (in the
    /// order returned by VFS) at the time the package was constructed. Required for sound
    /// dependency analysis.
    ///
    /// We don't use a Multimap because it provides no way to distinguish "key not present"
    /// from (key -> {}).
    glob_cache: Mutex<HashMap<(String, GlobberOperation), GlobResult>>,
    /// The directory in which our package's BUILD file resides.
    package_directory: Path,
    /// The name of the package we belong to.
    package_id: PackageIdentifier,
    /// The package locator-based directory traversal predicate.
    filesystem_ops: Arc<CountingFilesystemOps>,
    package_locator: Arc<dyn CachingPackageLocator>,
    ignored_subdirectories: Arc<dyn IgnoredSubdirectoriesMatcher>,
}

impl GlobCache {
    /// Creates a glob cache for the package `package_id` in `package_directory`.
    // PORT: the syscall cache is `SyscallCache.NO_CACHE`, and globs run on the calling thread
    // (no executor, no eager prefetching of `maxDirectoriesToEagerlyVisit` directories).
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:103 GlobCache.<init>
    pub fn new(
        package_directory: Path,
        package_id: PackageIdentifier,
        ignored_subdirectories: Arc<dyn IgnoredSubdirectoriesMatcher>,
        locator: Arc<dyn CachingPackageLocator>,
    ) -> Self {
        GlobCache {
            glob_cache: Mutex::new(HashMap::new()),
            package_directory,
            package_id,
            filesystem_ops: Arc::new(CountingFilesystemOps::default()),
            package_locator: locator,
            ignored_subdirectories,
        }
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:131 GlobCache.globCacheShouldTraverseDirectory
    fn glob_cache_should_traverse_directory(&self, directory: &Path) -> bool {
        if *directory == self.package_directory {
            return true;
        }

        let sub_package_path = self
            .package_id
            .get_package_fragment()
            .get_relative(&directory.relative_to(&self.package_directory));

        if self
            .ignored_subdirectories
            .matching_entry(&sub_package_path)
            .is_some()
        {
            return false;
        }

        !self.is_sub_package_id(&PackageIdentifier::create(
            self.package_id.get_repository().clone(),
            sub_package_path,
        ))
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:146 GlobCache.isSubPackage
    fn is_sub_package(&self, directory: &Path) -> bool {
        self.is_sub_package_id(&PackageIdentifier::create(
            self.package_id.get_repository().clone(),
            self.package_id
                .get_package_fragment()
                .get_relative(&directory.relative_to(&self.package_directory)),
        ))
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:146 GlobCache.isSubPackage
    fn is_sub_package_id(&self, sub_package_id: &PackageIdentifier) -> bool {
        self.package_locator
            .get_build_file_for_package(sub_package_id)
            .is_some()
    }

    /// Starts (here: computes) the glob of `pattern` unless it is cached.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:165 GlobCache.getGlobUnsortedAsync
    pub fn get_glob_unsorted_async(
        self: &Arc<Self>,
        pattern: &str,
        globber_operation: GlobberOperation,
    ) -> Result<GlobResult, BadGlobException> {
        let key = (pattern.to_owned(), globber_operation);
        if let Some(cached) = self.glob_cache.lock().get(&key) {
            return Ok(cached.clone());
        }
        let result = self.safe_glob_unsorted(pattern, globber_operation)?;
        self.set_glob_paths(pattern, globber_operation, result.clone());
        Ok(result)
    }

    /// The package-relative matches of `pattern` (files and directories).
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:179 GlobCache.getGlobUnsorted
    pub fn get_glob_unsorted_default(
        self: &Arc<Self>,
        pattern: &str,
    ) -> Result<Vec<String>, GlobberError> {
        self.get_glob_unsorted(pattern, GlobberOperation::FilesAndDirs)
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:179 GlobCache.getGlobUnsorted
    pub fn get_glob_unsorted(
        self: &Arc<Self>,
        pattern: &str,
        globber_operation: GlobberOperation,
    ) -> Result<Vec<String>, GlobberError> {
        let future_result = self.get_glob_unsorted_async(pattern, globber_operation)?;
        let glob_paths = future_result.map_err(|e| GlobberError::Io(e.to_string()))?;
        let mut result = Vec::with_capacity(glob_paths.len());
        for path in glob_paths.iter() {
            let relative = path.relative_to(&self.package_directory);
            let relative = relative.get_path_string();
            // Don't permit "" (meaning ".") in the glob expansion, since it's
            // invalid as a label, plus users should say explicitly if they
            // really want to name the package directory.
            if !relative.is_empty() {
                result.push(relative.to_owned());
            }
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:173 GlobCache.setGlobPaths
    fn set_glob_paths(
        &self,
        pattern: &str,
        globber_operation: GlobberOperation,
        result: GlobResult,
    ) {
        self.glob_cache
            .lock()
            .insert((pattern.to_owned(), globber_operation), result);
    }

    /// Actually execute a glob against the filesystem.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:172 GlobCache.safeGlobUnsorted
    pub fn safe_glob_unsorted(
        self: &Arc<Self>,
        pattern: &str,
        globber_operation: GlobberOperation,
    ) -> Result<GlobResult, BadGlobException> {
        // Forbidden patterns:
        if pattern.contains('?') {
            return Err(BadGlobException(format!(
                "glob pattern '{pattern}' contains forbidden '?' wildcard"
            )));
        }
        // Patterns forbidden by UnixGlob library:
        if let Some(error) = unix_glob::check_pattern_for_error(pattern) {
            return Err(BadGlobException(format!(
                "{error} (in glob pattern '{pattern}')"
            )));
        }
        let mut builder = UnixGlobBuilder::new(
            self.package_directory.clone(),
            self.filesystem_ops.clone() as Arc<dyn FilesystemOps>,
        );
        builder
            .add_pattern(pattern)
            .set_path_discriminator(Arc::new(GlobUnixPathDiscriminator {
                cache: self.clone(),
                globber_operation,
            }));
        match builder.glob() {
            Ok(paths) => Ok(Ok(Arc::new(paths))),
            Err(UnixGlobError::BadPattern(ex)) => Err(BadGlobException(ex.0)),
            Err(UnixGlobError::Io(e)) => Ok(Err(Arc::new(e))),
        }
    }

    /// Helper for evaluating the build language expression "glob(includes, excludes)" in the
    /// context of this package.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:260 GlobCache.globUnsorted
    pub fn glob_unsorted(
        self: &Arc<Self>,
        includes: &[String],
        excludes: &[String],
        globber_operation: GlobberOperation,
        allow_empty: bool,
    ) -> Result<Vec<String>, GlobberError> {
        // Start globbing all patterns in parallel. The getGlob() calls below will
        // block on an individual pattern's results, but the other globs can
        // continue in the background.
        for pattern in includes {
            let _possibly_ignored_error =
                self.get_glob_unsorted_async(pattern, globber_operation)?;
        }

        let mut results: HashSet<String> = HashSet::new();
        for pattern in includes {
            let items = self.get_glob_unsorted(pattern, globber_operation)?;
            if !allow_empty && items.is_empty() {
                return Err(bad_glob_exception_empty_result(pattern, globber_operation).into());
            }
            results.extend(items);
        }
        unix_glob::remove_excludes(&mut results, excludes)
            .map_err(|ex| GlobberError::BadGlob(BadGlobException(ex.0)))?;
        if !allow_empty && results.is_empty() {
            return Err(bad_glob_exception_all_excluded(globber_operation).into());
        }
        Ok(results.into_iter().collect())
    }

    /// The number of filesystem operations the globs performed.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:293 GlobCache.getGlobFilesystemOperationCost
    pub fn get_glob_filesystem_operation_cost(&self) -> i64 {
        self.filesystem_ops
            .filesystem_op_cost
            .load(Ordering::Relaxed)
    }

    /// The cached (pattern, operation) keys.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:297 GlobCache.getKeySet
    pub fn get_key_set(&self) -> HashSet<(String, GlobberOperation)> {
        self.glob_cache.lock().keys().cloned().collect()
    }

    /// Block on the completion of all potentially-abandoned background tasks.
    // PORT: globs run synchronously; nothing is in the background.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:302 GlobCache.finishBackgroundTasks
    pub fn finish_background_tasks(&self) {}

    // PORT: globs run synchronously; nothing is in the background.
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:317 GlobCache.cancelBackgroundTasks
    pub fn cancel_background_tasks(&self) {}
}

impl fmt::Display for GlobCache {
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:336 GlobCache.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GlobCache for {} in {}",
            self.package_id,
            self.package_directory.get_path_string()
        )
    }
}

/// Java `GlobCache.GlobUnixPathDiscriminator`.
// Java: com/google/devtools/build/lib/packages/GlobCache.java:349 GlobCache.GlobUnixPathDiscriminator
struct GlobUnixPathDiscriminator {
    cache: Arc<GlobCache>,
    globber_operation: GlobberOperation,
}

impl UnixGlobPathDiscriminator for GlobUnixPathDiscriminator {
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:357 GlobCache.GlobUnixPathDiscriminator.shouldTraverseDirectory
    fn should_traverse_directory(&self, directory: &Path) -> bool {
        self.cache.glob_cache_should_traverse_directory(directory)
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:362 GlobCache.GlobUnixPathDiscriminator.shouldIncludePathInResult
    fn should_include_path_in_result(&self, path: &Path, is_directory: bool) -> bool {
        match self.globber_operation {
            GlobberOperation::FilesAndDirs => !is_directory || !self.cache.is_sub_package(path),
            GlobberOperation::Subpackages => {
                // no files, or root pkg
                if !is_directory || *path == self.cache.package_directory {
                    return false;
                }
                self.cache.is_sub_package(path)
            }
            GlobberOperation::Files => !is_directory,
        }
    }
}

/// Filesystem operations that count their (unique) cost (Java
/// `GlobCache.CountingFilesystemOps`).
// Java: com/google/devtools/build/lib/packages/GlobCache.java:385 GlobCache.CountingFilesystemOps
#[derive(Default)]
struct CountingFilesystemOps {
    filesystem_op_cost: AtomicI64,
    paths_for_stat_if_found: Mutex<HashSet<Path>>,
    paths_for_readdir: Mutex<HashSet<Path>>,
}

impl FilesystemOps for CountingFilesystemOps {
    // Java: com/google/devtools/build/lib/packages/GlobCache.java:398 GlobCache.CountingFilesystemOps.statIfFound
    fn stat_if_found(&self, path: &Path) -> io::Result<Option<FileStatus>> {
        if self.paths_for_stat_if_found.lock().insert(path.clone()) {
            self.filesystem_op_cost.fetch_add(1, Ordering::Relaxed);
        }
        // Java: SyscallCache.statIfFound(path) follows symlinks.
        path.stat_if_found(Symlinks::Follow)
    }

    // Java: com/google/devtools/build/lib/packages/GlobCache.java:406 GlobCache.CountingFilesystemOps.readdir
    fn readdir(&self, path: &Path) -> io::Result<Vec<Dirent>> {
        let unique_op = self.paths_for_readdir.lock().insert(path.clone());
        if unique_op {
            self.filesystem_op_cost.fetch_add(1, Ordering::Relaxed);
        }
        let result = path.readdir(Symlinks::Nofollow)?;
        if unique_op {
            self.filesystem_op_cost
                .fetch_add(result.len() as i64, Ordering::Relaxed);
        }
        Ok(result)
    }
}

/// A [`Globber`] over a [`GlobCache`] (Java `NonSkyframeGlobber`).
// Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:22 NonSkyframeGlobber
pub struct NonSkyframeGlobber {
    glob_cache: Arc<GlobCache>,
    /// Java `NonSkyframeGlobber.Token`s by id.
    tokens: Mutex<HashMap<u64, NonSkyframeGlobberToken>>,
    next_token: AtomicU64,
}

/// Java `NonSkyframeGlobber.Token`.
// Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:30 NonSkyframeGlobber.Token
#[derive(Debug, Clone)]
struct NonSkyframeGlobberToken {
    includes: Vec<String>,
    excludes: Vec<String>,
    globber_operation: GlobberOperation,
    allow_empty: bool,
}

impl NonSkyframeGlobber {
    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:25 NonSkyframeGlobber.<init>
    pub fn new(glob_cache: Arc<GlobCache>) -> Self {
        NonSkyframeGlobber {
            glob_cache,
            tokens: Mutex::new(HashMap::new()),
            next_token: AtomicU64::new(0),
        }
    }

    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:81 NonSkyframeGlobber.getGlobFilesystemOperationCost
    pub fn get_glob_filesystem_operation_cost(&self) -> i64 {
        self.glob_cache.get_glob_filesystem_operation_cost()
    }
}

impl Globber for NonSkyframeGlobber {
    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:49 NonSkyframeGlobber.runAsync
    fn run_async(
        &self,
        includes: &[String],
        excludes: &[String],
        globber_operation: GlobberOperation,
        allow_empty: bool,
    ) -> Result<GlobberToken, GlobberError> {
        for pattern in includes {
            let _possibly_ignored_error = self
                .glob_cache
                .get_glob_unsorted_async(pattern, globber_operation)?;
        }
        let id = self.next_token.fetch_add(1, Ordering::Relaxed);
        self.tokens.lock().insert(
            id,
            NonSkyframeGlobberToken {
                includes: includes.to_vec(),
                excludes: excludes.to_vec(),
                globber_operation,
                allow_empty,
            },
        );
        Ok(GlobberToken(id))
    }

    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:64 NonSkyframeGlobber.fetchUnsorted
    fn fetch_unsorted(&self, token: GlobberToken) -> Result<Vec<String>, GlobberError> {
        let our_token = self
            .tokens
            .lock()
            .get(&token.0)
            .cloned()
            .expect("ClassCastException: not a NonSkyframeGlobber token");
        self.glob_cache.glob_unsorted(
            &our_token.includes,
            &our_token.excludes,
            our_token.globber_operation,
            our_token.allow_empty,
        )
    }

    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:72 NonSkyframeGlobber.onInterrupt
    fn on_interrupt(&self) {
        self.glob_cache.cancel_background_tasks();
    }

    // Java: com/google/devtools/build/lib/packages/NonSkyframeGlobber.java:77 NonSkyframeGlobber.onCompletion
    fn on_completion(&self) {
        self.glob_cache.finish_background_tasks();
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/packages/GlobCacheTest.java over a
    // real temporary directory standing in for Java's `Scratch("/workspace")`.
    use std::sync::atomic::AtomicU64;

    use blaze_vfs::DigestHashFunction;
    use blaze_vfs::FileSystemRef;
    use blaze_vfs::UnixFileSystem;

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: std::path::PathBuf,
        workspace: Path,
        package_directory: Path,
        cache: Arc<GlobCache>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    struct Locator {
        workspace: Path,
    }

    impl CachingPackageLocator for Locator {
        fn get_build_file_for_package(&self, package_id: &PackageIdentifier) -> Option<Path> {
            let package_name = package_id.get_package_fragment().get_path_string();
            if package_name == "isolated" {
                Some(self.workspace.get_relative_str("isolated/BUILD"))
            } else if package_name == "isolated/sub" {
                Some(self.workspace.get_relative_str("isolated/sub/BUILD"))
            } else {
                None
            }
        }
    }

    /// Java `IgnoredSubdirectories.of(prefixes)`: a directory matches a prefix it starts with.
    struct Ignored(Vec<PathFragment>);

    impl IgnoredSubdirectoriesMatcher for Ignored {
        fn matching_entry(&self, directory: &PathFragment) -> Option<String> {
            self.0
                .iter()
                .find(|p| directory.starts_with(p))
                .map(|p| p.get_path_string().to_owned())
        }
    }

    // Java: GlobCacheTest.createFiles
    fn fixture(ignored_directories: &[&str]) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "blaze-glob-cache-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let ws = root.join("workspace");
        for (file, content) in [
            ("isolated/BUILD", "# contents don't matter in this test"),
            ("isolated/sub/BUILD", "# contents don't matter in this test"),
            ("isolated/first.txt", "# this is first.txt"),
            ("isolated/second.txt", "# this is second.txt"),
            ("isolated/first.js", "# this is first.js"),
            ("isolated/second.js", "# this is second.js"),
            // Files in subdirectories
            ("isolated/foo/first.js", "# this is foo/first.js"),
            ("isolated/foo/second.js", "# this is foo/second.js"),
            ("isolated/bar/first.js", "# this is bar/first.js"),
            ("isolated/bar/second.js", "# this is bar/second.js"),
            ("isolated/sub/sub.js", "# this is sub/sub.js"),
        ] {
            let p = ws.join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
        let workspace = Path::create_from_str(ws.to_str().unwrap(), &fs);
        let package_directory = workspace.get_relative_str("isolated");
        let cache = Arc::new(GlobCache::new(
            package_directory.clone(),
            PackageIdentifier::create_in_main_repo_str("isolated"),
            Arc::new(Ignored(
                ignored_directories
                    .iter()
                    .map(|d| PathFragment::create(d))
                    .collect(),
            )),
            Arc::new(Locator {
                workspace: workspace.clone(),
            }),
        ));
        Fixture {
            root,
            workspace,
            package_directory,
            cache,
        }
    }

    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }

    fn strs(items: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = items.iter().map(|s| (*s).to_owned()).collect();
        v.sort();
        v
    }

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    impl Fixture {
        fn assert_paths_are(&self, paths: &[Path], strings: &[&str]) {
            let path_strings = sorted(
                paths
                    .iter()
                    .map(|p| p.relative_to(&self.workspace).get_path_string().to_owned())
                    .map(|s| format!("/workspace/{s}"))
                    .collect(),
            );
            assert_eq!(path_strings, strs(strings));
        }

        fn glob(&self, includes: &[&str], excludes: &[&str], op: GlobberOperation) -> Vec<String> {
            sorted(
                self.cache
                    .glob_unsorted(&list(includes), &list(excludes), op, true)
                    .unwrap(),
            )
        }
    }

    #[test]
    fn test_ignored_directory() {
        let f = fixture(&["isolated/foo"]);
        let paths = f
            .cache
            .safe_glob_unsorted("**/*.js", GlobberOperation::Files)
            .unwrap()
            .unwrap();
        f.assert_paths_are(
            &paths,
            &[
                "/workspace/isolated/first.js",
                "/workspace/isolated/second.js",
                "/workspace/isolated/bar/first.js",
                "/workspace/isolated/bar/second.js",
            ],
        );
    }

    #[test]
    fn test_safe_glob() {
        let f = fixture(&[]);
        let paths = f
            .cache
            .safe_glob_unsorted("*.js", GlobberOperation::FilesAndDirs)
            .unwrap()
            .unwrap();
        f.assert_paths_are(
            &paths,
            &[
                "/workspace/isolated/first.js",
                "/workspace/isolated/second.js",
            ],
        );
    }

    #[test]
    fn test_safe_glob_invalid_pattern() {
        let f = fixture(&[]);
        let invalid_pattern = "Foo?.txt";
        assert!(
            f.cache
                .safe_glob_unsorted(invalid_pattern, GlobberOperation::FilesAndDirs)
                .is_err()
        );
    }

    #[test]
    fn test_get_glob() {
        let f = fixture(&[]);
        let glob = f.cache.get_glob_unsorted_default("*.js").unwrap();
        assert_eq!(sorted(glob), strs(&["first.js", "second.js"]));
    }

    #[test]
    fn test_get_glob_subdirectory() {
        let f = fixture(&[]);
        let glob = f.cache.get_glob_unsorted_default("foo/*.js").unwrap();
        assert_eq!(sorted(glob), strs(&["foo/first.js", "foo/second.js"]));
    }

    #[test]
    fn test_get_key_set() {
        let f = fixture(&[]);
        let key = |p: &str, op| (p.to_owned(), op);
        let keys = |items: Vec<(String, GlobberOperation)>| -> HashSet<(String, GlobberOperation)> {
            items.into_iter().collect()
        };
        assert!(f.cache.get_key_set().is_empty());

        f.cache.get_glob_unsorted_default("*.java").unwrap();
        assert_eq!(
            f.cache.get_key_set(),
            keys(vec![key("*.java", GlobberOperation::FilesAndDirs)])
        );

        f.cache.get_glob_unsorted_default("*.java").unwrap();
        assert_eq!(
            f.cache.get_key_set(),
            keys(vec![key("*.java", GlobberOperation::FilesAndDirs)])
        );

        f.cache.get_glob_unsorted_default("*.js").unwrap();
        assert_eq!(
            f.cache.get_key_set(),
            keys(vec![
                key("*.java", GlobberOperation::FilesAndDirs),
                key("*.js", GlobberOperation::FilesAndDirs)
            ])
        );

        f.cache
            .get_glob_unsorted("*.java", GlobberOperation::Files)
            .unwrap();
        let three = keys(vec![
            key("*.java", GlobberOperation::FilesAndDirs),
            key("*.js", GlobberOperation::FilesAndDirs),
            key("*.java", GlobberOperation::Files),
        ]);
        assert_eq!(f.cache.get_key_set(), three);

        assert!(matches!(
            f.cache.get_glob_unsorted_default("invalid?"),
            Err(GlobberError::BadGlob(_))
        ));
        assert_eq!(f.cache.get_key_set(), three);

        f.cache.get_glob_unsorted_default("foo/first.*").unwrap();
        assert_eq!(
            f.cache.get_key_set(),
            keys(vec![
                key("*.java", GlobberOperation::FilesAndDirs),
                key("*.java", GlobberOperation::Files),
                key("*.js", GlobberOperation::FilesAndDirs),
                key("foo/first.*", GlobberOperation::FilesAndDirs),
            ])
        );
    }

    #[test]
    fn test_glob() {
        let f = fixture(&[]);
        assert!(f.glob(&["*.java"], &[], GlobberOperation::Files).is_empty());

        assert_eq!(
            f.glob(&["*.*"], &[], GlobberOperation::Files),
            strs(&["first.js", "first.txt", "second.js", "second.txt"])
        );

        assert_eq!(
            f.glob(&["*.*"], &["first.js"], GlobberOperation::Files),
            strs(&["first.txt", "second.js", "second.txt"])
        );

        assert_eq!(
            f.glob(&["*.txt", "first.*"], &[], GlobberOperation::Files),
            strs(&["first.txt", "second.txt", "first.js"])
        );
    }

    #[test]
    fn test_recursive_glob_does_not_match_subpackage() {
        let f = fixture(&[]);
        let glob = f.cache.get_glob_unsorted_default("**/*.js").unwrap();
        assert_eq!(
            sorted(glob),
            strs(&[
                "first.js",
                "second.js",
                "foo/first.js",
                "bar/first.js",
                "foo/second.js",
                "bar/second.js"
            ])
        );
    }

    #[test]
    fn test_single_file_exclude_star() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["*"], &["first.txt"], GlobberOperation::FilesAndDirs),
            strs(&["BUILD", "bar", "first.js", "foo", "second.js", "second.txt"])
        );
    }

    #[test]
    fn test_single_file_exclude_star_star() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["**"], &["first.txt"], GlobberOperation::FilesAndDirs),
            strs(&[
                "BUILD",
                "bar",
                "bar/first.js",
                "bar/second.js",
                "first.js",
                "foo",
                "foo/first.js",
                "foo/second.js",
                "second.js",
                "second.txt",
            ])
        );
    }

    #[test]
    fn test_exclude_all_star() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["*"], &["*"], GlobberOperation::FilesAndDirs)
                .is_empty()
        );
    }

    #[test]
    fn test_exclude_all_star_no_matches_anyway() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["nope"], &["*"], GlobberOperation::FilesAndDirs)
                .is_empty()
        );
    }

    #[test]
    fn test_exclude_all_star_star() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["**"], &["**"], GlobberOperation::FilesAndDirs)
                .is_empty()
        );
    }

    #[test]
    fn test_exclude_all_manual() {
        let f = fixture(&[]);
        assert!(
            f.glob(
                &["**"],
                &["*", "*/*", "*/*/*"],
                GlobberOperation::FilesAndDirs
            )
            .is_empty()
        );
    }

    #[test]
    fn test_single_file_exclude_doesnt_match() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(
                &["first.txt"],
                &["nope.txt"],
                GlobberOperation::FilesAndDirs
            ),
            strs(&["first.txt"])
        );
    }

    #[test]
    fn test_exclude_directory() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["foo/*"], &[], GlobberOperation::Files),
            strs(&["foo/first.js", "foo/second.js"])
        );
        assert_eq!(
            f.glob(&["foo/*"], &["foo"], GlobberOperation::FilesAndDirs),
            strs(&["foo/first.js", "foo/second.js"])
        );
    }

    #[test]
    fn test_child_glob_with_child_exclude() {
        let f = fixture(&[]);
        let op = GlobberOperation::FilesAndDirs;
        assert!(f.glob(&["foo/*"], &["foo/*"], op).is_empty());
        assert!(
            f.glob(&["foo/first.js", "foo/second.js"], &["foo/*"], op)
                .is_empty()
        );
        assert!(f.glob(&["foo/first.js"], &["foo/first.js"], op).is_empty());
        assert!(f.glob(&["foo/first.js"], &["*/first.js"], op).is_empty());
        assert!(f.glob(&["foo/first.js"], &["*/*"], op).is_empty());
    }

    #[test]
    fn test_subpackages_no_wildcard() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["sub/sub.js"], &[], GlobberOperation::Subpackages)
                .is_empty()
        );
    }

    #[test]
    fn test_subpackages_simple_double_star() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["**"], &[], GlobberOperation::Subpackages),
            strs(&["sub"])
        );
    }

    #[test]
    fn test_subpackages_only_sub() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["sub"], &[], GlobberOperation::Subpackages),
            strs(&["sub"])
        );
    }

    #[test]
    fn test_subpackages_single_stars_after_sub() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["sub/*"], &[], GlobberOperation::Subpackages)
                .is_empty()
        );
    }

    #[test]
    fn test_subpackages_double_stars_after_sub() {
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["sub/**"], &[], GlobberOperation::Subpackages),
            strs(&["sub"])
        );
    }

    #[test]
    fn test_subpackages_two_double_stars_after_sub() {
        // Both `**`s are considered to match no path fragments.
        let f = fixture(&[]);
        assert_eq!(
            f.glob(&["sub/**/**"], &[], GlobberOperation::Subpackages),
            strs(&["sub"])
        );
    }

    #[test]
    fn test_subpackages_double_stars_and_other_path_after_sub() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["sub/**/foo"], &[], GlobberOperation::Subpackages)
                .is_empty()
        );
    }

    #[test]
    fn test_subpackages_double_star_with_trailing_pattern() {
        let f = fixture(&[]);
        assert!(
            f.glob(&["**/bar"], &[], GlobberOperation::Subpackages)
                .is_empty()
        );
    }

    #[test]
    fn non_skyframe_globber_runs_tokens() {
        let f = fixture(&[]);
        let globber = NonSkyframeGlobber::new(f.cache.clone());
        let token = globber
            .run_async(&list(&["*.js"]), &[], GlobberOperation::Files, false)
            .unwrap();
        assert_eq!(
            sorted(globber.fetch_unsorted(token).unwrap()),
            strs(&["first.js", "second.js"])
        );
        let token = globber
            .run_async(&list(&["*.nothing"]), &[], GlobberOperation::Files, false)
            .unwrap();
        assert_eq!(
            globber.fetch_unsorted(token).unwrap_err().to_string(),
            "glob pattern '*.nothing' didn't match anything, but allow_empty is set to False \
             (the default value of allow_empty can be set with \
             --incompatible_disallow_empty_glob)."
        );
        let _ = &f.package_directory;
    }
}
