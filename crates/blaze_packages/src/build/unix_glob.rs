//! Port of `vfs/UnixGlob.java` and `vfs/UnixGlobPathDiscriminator.java`: wildcard matching,
//! pattern checks, exclude filtering and the filesystem traversal behind non-Skyframe
//! globbing.
//!
//! PORT: move to blaze_vfs (Java `lib.vfs`). The traversal runs on the calling thread: Java's
//! executor only parallelises the same work (the result is a set, so order is not
//! observable), and Bazel 9 globs through Skyframe (GLOBS) in production.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use blaze_vfs::Dirent;
use blaze_vfs::DirentType;
use blaze_vfs::FileStatus;
use blaze_vfs::Path;
use blaze_vfs::Symlinks;
use indexmap::IndexSet;
use regex::Regex;

/// An invalid glob pattern (Java `UnixGlob.BadPattern`).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:72 UnixGlob.BadPattern
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BadPattern(pub String);

/// Errors of a glob traversal.
#[derive(Debug, thiserror::Error)]
pub enum UnixGlobError {
    #[error("{0}")]
    BadPattern(#[from] BadPattern),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// The filesystem operations a glob uses (Java `UnixGlob.FilesystemOps`).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:79 UnixGlob.FilesystemOps
pub trait FilesystemOps: Send + Sync {
    /// Java `statIfFound(path)` (not following symlinks).
    fn stat_if_found(&self, path: &Path) -> io::Result<Option<FileStatus>>;

    /// Java `readdir(path)` (not following symlinks).
    fn readdir(&self, path: &Path) -> io::Result<Vec<Dirent>>;
}

/// Java `FilesystemOps.DIRECT`: the path's own `statIfFound` / `readdir`, not following
/// symlinks.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:87 UnixGlob.FilesystemOps.DIRECT
pub struct DirectFilesystemOps;

impl FilesystemOps for DirectFilesystemOps {
    fn stat_if_found(&self, path: &Path) -> io::Result<Option<FileStatus>> {
        path.stat_if_found(Symlinks::Nofollow)
    }

    fn readdir(&self, path: &Path) -> io::Result<Vec<Dirent>> {
        path.readdir(Symlinks::Nofollow)
    }
}

/// Decides which directories a glob traverses and which paths it returns (Java
/// `UnixGlobPathDiscriminator`).
// Java: com/google/devtools/build/lib/vfs/UnixGlobPathDiscriminator.java:24 UnixGlobPathDiscriminator
pub trait UnixGlobPathDiscriminator: Send + Sync {
    /// Whether the glob descends into `path` (also asked for the traversal root).
    // Java: com/google/devtools/build/lib/vfs/UnixGlobPathDiscriminator.java:30 UnixGlobPathDiscriminator.shouldTraverseDirectory
    fn should_traverse_directory(&self, path: &Path) -> bool {
        let _ = path;
        true
    }

    /// Whether a matching `path` belongs in the result.
    // Java: com/google/devtools/build/lib/vfs/UnixGlobPathDiscriminator.java:38 UnixGlobPathDiscriminator.shouldIncludePathInResult
    fn should_include_path_in_result(&self, path: &Path, is_directory: bool) -> bool {
        let _ = (path, is_directory);
        true
    }
}

/// Java `UnixGlob.DEFAULT_DISCRIMINATOR`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:66 UnixGlob.DEFAULT_DISCRIMINATOR
pub struct DefaultDiscriminator;

impl UnixGlobPathDiscriminator for DefaultDiscriminator {}

// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:153 UnixGlob.checkAndSplitPatterns
fn check_and_split_patterns(patterns: &[String]) -> Result<Vec<Vec<String>>, BadPattern> {
    let mut list = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        if let Some(error) = check_pattern_for_error(pattern) {
            return Err(BadPattern(format!("{error} (in glob pattern '{pattern}')")));
        }
        list.push(pattern.split('/').map(str::to_owned).collect());
    }
    Ok(list)
}

/// Checks that `pattern` is a valid glob; returns the error message or `None`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:170 UnixGlob.checkPatternForError
pub fn check_pattern_for_error(pattern: &str) -> Option<String> {
    if pattern.is_empty() {
        return Some("pattern cannot be empty".to_owned());
    }
    if pattern.starts_with('/') {
        return Some("pattern cannot be absolute".to_owned());
    }
    for segment in pattern.split('/') {
        if segment.is_empty() {
            return Some("empty segment not permitted".to_owned());
        }
        if segment == "." || segment == ".." {
            return Some(format!("segment '{segment}' not permitted"));
        }
        if segment.contains("**") && segment != "**" {
            return Some("recursive wildcard must be its own segment".to_owned());
        }
    }
    None
}

/// Whether the split `pattern` matches the split `path` exactly.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:194 UnixGlob.matches
pub fn matches_segments(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, None, MatchMode::Exact)
}

/// A cache of compiled wildcard patterns (Java `Map<String, Pattern>`).
pub type PatternCache = HashMap<String, Regex>;

/// Whether the wildcard `pattern` (one segment: `*`, `?`, `**`) matches `str`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:199 UnixGlob.matches
pub fn matches(pattern: &str, s: &str) -> bool {
    matches_cached(pattern, s, None)
}

/// Like [`matches`], caching compiled patterns in `pattern_cache`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:212 UnixGlob.matches
pub fn matches_cached(pattern: &str, s: &str, pattern_cache: Option<&mut PatternCache>) -> bool {
    if pattern.is_empty() || s.is_empty() {
        return false;
    }

    // Common case: **
    if pattern == "**" {
        return true;
    }

    // Common case: *
    if pattern == "*" {
        return true;
    }

    // If a filename starts with '.', this char must be matched explicitly.
    if s.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }

    // Common case: *.xyz
    if pattern.starts_with('*') && pattern.rfind('*') == Some(0) {
        return s.ends_with(&pattern[1..]);
    }
    // Common case: xyz*
    let last_index = pattern.len() - 1;
    // The first clause of this if statement is unnecessary, but is an
    // optimization--charAt runs faster than indexOf.
    if pattern.ends_with('*') && pattern.find('*') == Some(last_index) {
        return s.starts_with(&pattern[..last_index]);
    }

    match pattern_cache {
        None => make_pattern_from_wildcard(pattern).is_match(s),
        Some(cache) => cache
            .entry(pattern.to_owned())
            .or_insert_with(|| make_pattern_from_wildcard(pattern))
            .is_match(s),
    }
}

/// Java's `.` without DOTALL: any char but a line terminator (`\n`, `\r`, U+0085, U+2028,
/// U+2029; java/util/regex/Pattern.java `CharPredicates.DOT`). The regex crate's `.` only
/// excludes `\n`. U+0085 matters for internal strings: it is the second byte of `Å` (C3 85), so
/// Bazel's `x*y.txt` does not match a file `xÅy.txt` (oracle).
const JAVA_DOT: &str = "[^\\n\\r\\x{85}\\x{2028}\\x{2029}]";

/// Compiles a wildcard to a regex matching whole strings (Java `Pattern.matches`).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:257 UnixGlob.makePatternFromWildcard
fn make_pattern_from_wildcard(pattern: &str) -> Regex {
    let chars: Vec<char> = pattern.chars().collect();
    let len = chars.len();
    let mut regexp = String::new();
    let mut i = 0;
    while i < len {
        let c = chars[i];
        match c {
            '*' => {
                let mut to_increment = 0;
                if len > i + 1 && chars[i + 1] == '*' {
                    // The pattern '**' is interpreted to match 0 or more directory separators,
                    // not 1 or more. We skip the next * and then find a trailing/leading '/' and
                    // get rid of it.
                    to_increment = 1;
                    if len > i + 2 && chars[i + 2] == '/' {
                        // We have '**/' -- skip the '/'.
                        to_increment = 2;
                    } else if len == i + 2 && i > 0 && chars[i - 1] == '/' {
                        // We have '/**' -- remove the '/'.
                        regexp.pop();
                    }
                }
                regexp.push_str(JAVA_DOT);
                regexp.push('*');
                i += to_increment;
            }
            '?' => regexp.push_str(JAVA_DOT),
            // escape the regexp special characters that are allowed in wildcards
            '^' | '$' | '|' | '+' | '{' | '}' | '[' | ']' | '\\' | '.' => {
                regexp.push('\\');
                regexp.push(c);
            }
            // The historical undocumented behavior of this function was to add '(' and ')' to
            // the regexp pattern string unescaped. That could have 2 effects: a no-op (if the
            // parentheses were properly paired) or a PatternSyntaxException leading to a Bazel
            // crash (if the parentheses were unpaired). The behavior was silly, but changing it
            // will break existing BUILD files which e.g. call `glob(["(*.foo)"])`. To keep such
            // BUILD files working while avoiding crashes, treat '(' and ')' as a safe no-op when
            // compiling to regexp.
            '(' | ')' => {}
            // PORT: characters Java's regex treats literally but the regex crate does not
            // (none in Java's escape list above) are escaped so they stay literal.
            c => regexp.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    // PORT: Java's `Matcher.matches()` matches the whole input; `.` is written as JAVA_DOT.
    Regex::new(&format!("^(?:{regexp})$")).expect("wildcard regex")
}

/// How [`matches_pattern`] should work (Java `UnixGlob.MatchMode`).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:952 UnixGlob.MatchMode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    /// The path should exactly match the pattern.
    Exact,
    /// The pattern should match a prefix of the path.
    Prefix,
    /// Whether there can be any path under the prefix that matches the pattern.
    CanMatchChild,
}

/// Returns whether any path under `path` can match `pattern`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:942 UnixGlob.canMatchChild
pub fn can_match_child(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, None, MatchMode::CanMatchChild)
}

/// Returns whether `pattern` matches a prefix of `path`.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:947 UnixGlob.matchesPrefix
pub fn matches_prefix(pattern: &[String], path: &[String]) -> bool {
    matches_pattern(pattern, path, 0, 0, None, MatchMode::Prefix)
}

/// Returns true if `pattern` matches `path` starting from the given segments.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:959 UnixGlob.matchesPattern
fn matches_pattern(
    pattern: &[String],
    path: &[String],
    i: usize,
    j: usize,
    mut pattern_cache: Option<&mut PatternCache>,
    match_mode: MatchMode,
) -> bool {
    if i == pattern.len() {
        return match_mode == MatchMode::Prefix || j == path.len();
    }
    if pattern[i] == "**" {
        return matches_pattern(
            pattern,
            path,
            i + 1,
            j,
            pattern_cache.as_deref_mut(),
            match_mode,
        ) || (j < path.len()
            && matches_pattern(pattern, path, i, j + 1, pattern_cache, match_mode));
    }
    if j == path.len() {
        return match_mode == MatchMode::CanMatchChild;
    }
    if matches_cached(&pattern[i], &path[j], pattern_cache.as_deref_mut()) {
        return matches_pattern(pattern, path, i + 1, j + 1, pattern_cache, match_mode);
    }
    false
}

// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:982 UnixGlob.isWildcardFree
fn is_wildcard_free(pattern: &str) -> bool {
    !pattern.contains('*') && !pattern.contains('?')
}

/// Filters out exclude patterns from a set of paths. Common cases such as wildcard-free
/// patterns or suffix patterns are special-cased to make this function efficient.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:889 UnixGlob.removeExcludes
pub fn remove_excludes(paths: &mut HashSet<String>, excludes: &[String]) -> Result<(), BadPattern> {
    let mut complex_patterns: Vec<String> = Vec::with_capacity(excludes.len());
    // PORT: Java's HashMap order is not observable (the patterns only remove elements).
    let mut starstar_slash_star_head_tail_pairs: HashMap<String, Vec<String>> = HashMap::new();
    for exclude in excludes {
        if is_wildcard_free(exclude) {
            paths.remove(exclude);
            continue;
        }
        if let Some(pattern_pos) = exclude.find("**/*") {
            let head = &exclude[..pattern_pos];
            let tail = &exclude[pattern_pos + 4..];
            if is_wildcard_free(head) && is_wildcard_free(tail) {
                starstar_slash_star_head_tail_pairs
                    .entry(head.to_owned())
                    .or_default()
                    .push(tail.to_owned());
                continue;
            }
        }
        complex_patterns.push(exclude.clone());
    }
    for (head, tails) in &starstar_slash_star_head_tail_pairs {
        paths.retain(|path| {
            !(path.starts_with(head.as_str()) && tails.iter().any(|t| path.ends_with(t.as_str())))
        });
    }
    if complex_patterns.is_empty() {
        return Ok(());
    }
    // TODO: b/361409364 - Fully validate exclude patterns. This is a breaking change, so there
    // needs to first be a depot cleanup.
    let split_patterns = check_and_split_patterns(&complex_patterns)?;
    let mut pattern_cache = PatternCache::new();
    paths.retain(|path| {
        let segments: Vec<String> = path.split('/').map(str::to_owned).collect();
        for split_pattern in &split_patterns {
            if matches_pattern(
                split_pattern,
                &segments,
                0,
                0,
                Some(&mut pattern_cache),
                MatchMode::Exact,
            ) {
                return false;
            }
        }
        true
    });
    Ok(())
}

/// Builder of a glob over a base directory (Java `UnixGlob.Builder`).
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:320 UnixGlob.Builder
pub struct UnixGlobBuilder {
    base: Path,
    patterns: Vec<String>,
    filesystem_ops: Arc<dyn FilesystemOps>,
    path_discriminator: Arc<dyn UnixGlobPathDiscriminator>,
}

impl UnixGlobBuilder {
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:328 UnixGlob.Builder.<init>
    pub fn new(base: Path, filesystem_ops: Arc<dyn FilesystemOps>) -> Self {
        UnixGlobBuilder {
            base,
            patterns: Vec::new(),
            filesystem_ops,
            path_discriminator: Arc::new(DefaultDiscriminator),
        }
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:339 UnixGlob.Builder.addPattern
    pub fn add_pattern(&mut self, pattern: &str) -> &mut Self {
        self.patterns.push(pattern.to_owned());
        self
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:350 UnixGlob.Builder.addPatterns
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:361 UnixGlob.Builder.addPatterns
    pub fn add_patterns(&mut self, patterns: &[&str]) -> &mut Self {
        self.patterns
            .extend(patterns.iter().map(|p| (*p).to_owned()));
        self
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:389 UnixGlob.Builder.setPathDiscriminator
    pub fn set_path_discriminator(
        &mut self,
        path_discriminator: Arc<dyn UnixGlobPathDiscriminator>,
    ) -> &mut Self {
        self.path_discriminator = path_discriminator;
        self
    }

    /// Executes the glob; the order of the result is unspecified.
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:396 UnixGlob.Builder.glob
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:406 UnixGlob.Builder.globInterruptible
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:101 UnixGlob.globInternal
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:112 UnixGlob.globInternalUninterruptible
    pub fn glob(&self) -> Result<Vec<Path>, UnixGlobError> {
        GlobVisitor::default().glob(
            &self.base,
            &self.patterns,
            self.path_discriminator.as_ref(),
            self.filesystem_ops.as_ref(),
        )
    }

    /// Java `globInterruptibleAndReturnNumGlobTasksForTesting`.
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:410 UnixGlob.Builder.globInterruptibleAndReturnNumGlobTasksForTesting
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:123 UnixGlob.globInternalAndReturnNumGlobTasksForTesting
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:659 UnixGlob.GlobVisitor.getNumGlobTasksForTesting
    pub fn glob_and_return_num_glob_tasks_for_testing(&self) -> Result<u64, UnixGlobError> {
        let mut visitor = GlobVisitor::default();
        visitor.glob(
            &self.base,
            &self.patterns,
            self.path_discriminator.as_ref(),
            self.filesystem_ops.as_ref(),
        )?;
        Ok(visitor.total_ops)
    }
}

// The glob runs on the calling thread over a work queue (Java: an executor, or inline when there
// is none); the queue loop of `GlobVisitor::glob` stands for `enqueue`'s wrapper (a task does not
// run once an error is recorded) and `decrementAndCheckDone` (the result or the first error when
// the queue is empty). Results are a set in both; Java's order is the concurrent set's, and every
// caller (`GlobCache.globUnsorted`) puts them in a `HashSet` and the package sorts them.
// PORT: an interrupt does not stop a glob (Java's `globInterruptible` waits on the future
// interruptibly); a glob has no executor to stop, it ends when its queue is empty.
// PORT: skipped, the synchronous visitor (no future, executor or cancellation): UnixGlob.globAsyncInternal, UnixGlob.setExecutor, UnixGlob.globAsync
// PORT: skipped, the synchronous visitor (no future, executor or cancellation): UnixGlob.delegate, UnixGlob.setException, UnixGlob.set, UnixGlob.cancel, UnixGlob.markCanceled
// PORT: skipped, the synchronous visitor (no future, executor or cancellation): UnixGlob.queueTask, UnixGlob.decrementAndCheckDone
// PORT: skipped, errors and panics: only I/O errors are values in Rust (a panic propagates): UnixGlob.getMostSeriousThrowableSoFar
// PORT: skipped, the (base, index) tuple of RecursiveGlobTaskContext's set: UnixGlob.equals, UnixGlob.hashCode

/// One top-level glob task's context (Java `GlobVisitor.GlobTaskContext` and
/// `RecursiveGlobTaskContext`).
struct GlobTaskContext<'a> {
    pattern_parts: Vec<String>,
    path_discriminator: &'a dyn UnixGlobPathDiscriminator,
    filesystem_ops: &'a dyn FilesystemOps,
    /// `Some` for a `RecursiveGlobTaskContext`: the visited (base, pattern index) subtasks.
    visited_glob_sub_tasks: Option<HashSet<(Path, usize)>>,
}

/// The glob traversal (Java `UnixGlob.GlobVisitor`), run on the calling thread.
// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:466 UnixGlob.GlobVisitor
#[derive(Default)]
struct GlobVisitor {
    /// Java's concurrent set; an insertion-ordered set here.
    results: IndexSet<Path>,
    total_ops: u64,
    /// Work queue (Java enqueues runnables on the executor).
    queue: std::collections::VecDeque<GlobTask>,
    io_exception: Option<io::Error>,
}

enum GlobTask {
    Glob {
        context: usize,
        base: Path,
        base_is_dir: bool,
        idx: usize,
    },
    Symlink {
        context: usize,
        path: Path,
        idx: usize,
    },
}

// Java: com/google/devtools/build/lib/vfs/UnixGlob.java:536 UnixGlob.GlobVisitor.isRecursivePattern
fn is_recursive_pattern(pattern: &str) -> bool {
    pattern == "**"
}

impl GlobVisitor {
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:544 UnixGlob.GlobVisitor.globAsync
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:502 UnixGlob.GlobVisitor.glob
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:518 UnixGlob.GlobVisitor.globUninterruptible
    fn glob(
        &mut self,
        base: &Path,
        patterns: &[String],
        path_discriminator: &dyn UnixGlobPathDiscriminator,
        filesystem_ops: &dyn FilesystemOps,
    ) -> Result<Vec<Path>, UnixGlobError> {
        let base_stat = filesystem_ops.stat_if_found(base)?;
        let Some(base_stat) = base_stat else {
            return Ok(Vec::new());
        };
        if patterns.is_empty() {
            return Ok(Vec::new());
        }

        // TODO(adonovan): validate pattern unconditionally, before I/O (potentially breaking
        // change).
        let split_patterns = check_and_split_patterns(patterns)?;

        // We do a dumb loop, even though it will likely duplicate logical work (note that the
        // physical filesystem operations are cached). In order to optimize, we would need to
        // keep track of which patterns shared sub-patterns and which did not (for example
        // consider the glob [*/*.java, sub/*.java, */*.txt]).
        let mut contexts: Vec<GlobTaskContext<'_>> = Vec::new();
        for split_pattern in split_patterns {
            let num_recursive_patterns = split_pattern
                .iter()
                .filter(|p| is_recursive_pattern(p))
                .count();
            contexts.push(GlobTaskContext {
                pattern_parts: split_pattern,
                path_discriminator,
                filesystem_ops,
                visited_glob_sub_tasks: if num_recursive_patterns > 1 {
                    Some(HashSet::new())
                } else {
                    None
                },
            });
            let context = contexts.len() - 1;
            self.queue_glob(
                &mut contexts,
                context,
                base.clone(),
                base_stat.is_directory(),
                0,
            );
        }
        while let Some(task) = self.queue.pop_front() {
            // Java: tasks do not run once an error was recorded.
            if self.io_exception.is_some() {
                continue;
            }
            let r = match task {
                GlobTask::Glob {
                    context,
                    base,
                    base_is_dir,
                    idx,
                } => self.really_glob(&mut contexts, context, &base, base_is_dir, idx),
                GlobTask::Symlink { context, path, idx } => {
                    self.process_symlink(&mut contexts, context, &path, idx)
                }
            };
            if let Err(e) = r {
                self.io_exception.get_or_insert(e);
            }
        }
        if let Some(e) = self.io_exception.take() {
            return Err(UnixGlobError::Io(e));
        }
        Ok(std::mem::take(&mut self.results).into_iter().collect())
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:604 UnixGlob.GlobVisitor.queueGlob
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:699 UnixGlob.GlobVisitor.GlobTaskContext.queueGlob
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:747 UnixGlob.GlobVisitor.RecursiveGlobTaskContext.queueGlob
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:637 UnixGlob.GlobVisitor.enqueue
    // and RecursiveGlobTaskContext.queueGlob (dedupes subtasks).
    fn queue_glob(
        &mut self,
        contexts: &mut [GlobTaskContext<'_>],
        context: usize,
        base: Path,
        base_is_dir: bool,
        idx: usize,
    ) {
        if let Some(visited) = &mut contexts[context].visited_glob_sub_tasks {
            // This is a unique glob task. For example of how duplicates can arise, consider:
            //   glob(['**/a/**/foo.txt'])
            // with the only file being
            //   a/a/foo.txt
            //
            // there are multiple ways to reach a/a/foo.txt: one route starts by recursively
            // globbing 'a/**/foo.txt' in the base directory of the package, and another route
            // starts by recursively globbing '**/a/**/foo.txt' in subdirectory 'a'.
            if !visited.insert((base.clone(), idx)) {
                return;
            }
        }
        self.total_ops += 1;
        self.queue.push_back(GlobTask::Glob {
            context,
            base,
            base_is_dir,
            idx,
        });
    }

    /// Expressed in Haskell:
    ///
    /// ```text
    ///  reallyGlob base []     = { base }
    ///  reallyGlob base [x:xs] = union { reallyGlob(f, xs) | f results "base/x" }
    /// ```
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:771 UnixGlob.GlobVisitor.reallyGlob
    fn really_glob(
        &mut self,
        contexts: &mut [GlobTaskContext<'_>],
        context: usize,
        base: &Path,
        base_is_dir: bool,
        idx: usize,
    ) -> io::Result<()> {
        let ctx = &contexts[context];
        if idx == ctx.pattern_parts.len() {
            // Base case.
            self.maybe_add_result(ctx, base, base_is_dir);
            return Ok(());
        }

        // Do an early readdir() call here if the pattern contains a wildcard (* or ?). The
        // reason is that we'll do so later anyway and doing this early avoids an additional
        // stat to determine the existence of a build file as part of the
        // shouldTraverseDirectory() call below (globs will no recurse into sub-packages, i.e.
        // directories that contain a build file). This optimizes for the common case where
        // there is no build file in the sub directory.
        let pattern = ctx.pattern_parts[idx].clone();
        let pattern_contains_wildcard = pattern.contains('*') || pattern.contains('?');
        let mut dents: Option<Vec<Dirent>> = None;
        if base_is_dir && pattern_contains_wildcard {
            dents = Some(ctx.filesystem_ops.readdir(base)?);
        }

        if base_is_dir && !ctx.path_discriminator.should_traverse_directory(base) {
            if Self::are_all_remaining_patterns_double_star(ctx, idx) {
                // For SUBPACKAGES, we encounter this when all remaining patterns in the glob
                // expression are `**`s. In that case we should include the subpackage's
                // PathFragment (relative to the package fragment) in the matching results.
                self.maybe_add_result(ctx, base, base_is_dir);
            }
            return Ok(());
        }

        if !base_is_dir {
            // Nothing to find here.
            return Ok(());
        }

        // ** is special: it can match nothing at all.
        // For example, x/** matches x, **/y matches y, and x/**/y matches x/y.
        if is_recursive_pattern(&pattern) {
            self.queue_glob(contexts, context, base.clone(), base_is_dir, idx + 1);
        }

        if !pattern_contains_wildcard {
            // We do not need to do a readdir in this case, just a stat.
            let child = base.get_child(&pattern);
            let status = contexts[context].filesystem_ops.stat_if_found(&child)?;
            match status {
                Some(status) if status.is_directory() || status.is_file() => {
                    self.queue_glob(contexts, context, child, status.is_directory(), idx + 1);
                }
                // The file is a dangling symlink, fifo, does not exist, etc.
                _ => {}
            }
            return Ok(());
        }

        let mut cache = PatternCache::new();
        for dent in dents.unwrap_or_default() {
            let child_type = dent.get_type();
            if child_type == DirentType::Unknown {
                // The file is a special file (fifo, etc.). No need to even match against the
                // pattern.
                continue;
            }
            if matches_cached(&pattern, dent.get_name(), Some(&mut cache)) {
                let child = base.get_child(dent.get_name());

                if child_type == DirentType::Symlink {
                    // Java: processSymlink queues a task.
                    self.total_ops += 1;
                    self.queue.push_back(GlobTask::Symlink {
                        context,
                        path: child,
                        idx,
                    });
                } else {
                    self.process_file_or_directory(
                        contexts,
                        context,
                        child,
                        child_type == DirentType::Directory,
                        idx,
                    );
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:842 UnixGlob.GlobVisitor.maybeAddResult
    fn maybe_add_result(&mut self, context: &GlobTaskContext<'_>, base: &Path, is_directory: bool) {
        if context
            .path_discriminator
            .should_include_path_in_result(base, is_directory)
        {
            self.results.insert(base.clone());
        }
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:848 UnixGlob.GlobVisitor.areAllRemainingPatternsDoubleStar
    fn are_all_remaining_patterns_double_star(
        context: &GlobTaskContext<'_>,
        start_idx: usize,
    ) -> bool {
        context.pattern_parts[start_idx..].iter().all(|p| p == "**")
    }

    /// Process symlinks asynchronously. If we should used readdir(..., Symlinks.FOLLOW), that
    /// would result in a sequential symlink resolution with many file system implementations.
    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:860 UnixGlob.GlobVisitor.processSymlink
    fn process_symlink(
        &mut self,
        contexts: &mut [GlobTaskContext<'_>],
        context: usize,
        path: &Path,
        idx: usize,
    ) -> io::Result<()> {
        // PORT: Java's `statIfFound(path)` on a symlink: FilesystemOps.DIRECT does not follow
        // it; the CountingFilesystemOps of GlobCache stats through the syscall cache, which
        // follows symlinks.
        let status = contexts[context].filesystem_ops.stat_if_found(path)?;
        if let Some(status) = status {
            self.process_file_or_directory(
                contexts,
                context,
                path.clone(),
                status.is_directory(),
                idx,
            );
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/vfs/UnixGlob.java:874 UnixGlob.GlobVisitor.processFileOrDirectory
    fn process_file_or_directory(
        &mut self,
        contexts: &mut [GlobTaskContext<'_>],
        context: usize,
        path: Path,
        is_dir: bool,
        idx: usize,
    ) {
        let is_recursive = is_recursive_pattern(&contexts[context].pattern_parts[idx]);
        if is_dir {
            self.queue_glob(
                contexts,
                context,
                path,
                /* base_is_dir= */ true,
                idx + if is_recursive { 0 } else { 1 },
            );
        } else if idx + 1 == contexts[context].pattern_parts.len() {
            let ctx = &contexts[context];
            self.maybe_add_result(ctx, &path, /* is_directory= */ false);
        }
    }
}

#[cfg(test)]
mod tests {
    // Ports of src/test/java/com/google/devtools/build/lib/vfs/GlobTest.java over a real
    // temporary directory (Java uses an InMemoryFileSystem, which blaze_vfs does not port).
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use blaze_vfs::DigestHashFunction;
    use blaze_vfs::FileSystemRef;
    use blaze_vfs::UnixFileSystem;

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        tmp_path: Path,
        root: std::path::PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The platform form of `path` (an internal string), for `std::fs`.
    fn platform(path: &Path) -> std::path::PathBuf {
        std::path::PathBuf::from(
            blaze_util::string_encoding::internal_to_platform(path.get_path_string()).into_owned(),
        )
    }

    // Java: GlobTest.initializeFileSystem
    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "blaze-unix-glob-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = root.join("globtmp");
        for dir in [
            "foo/bar/wiz",
            "foo/barnacle/wiz",
            "food/barnacle/wiz",
            "fool/barnacle/wiz",
        ] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
        }
        std::fs::write(tmp.join("foo/bar/wiz/file"), "").unwrap();
        let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
        Fixture {
            tmp_path: Path::create_from_str(
                &blaze_util::string_encoding::platform_to_internal(tmp.as_os_str()),
                &fs,
            ),
            root,
        }
    }

    impl Fixture {
        fn resolve_paths(&self, relative_paths: &[&str]) -> HashSet<Path> {
            relative_paths
                .iter()
                .map(|p| {
                    if *p == "." || p.is_empty() {
                        self.tmp_path.clone()
                    } else {
                        self.tmp_path.get_relative_str(p)
                    }
                })
                .collect()
        }

        fn glob(&self, base: &Path, patterns: &[&str]) -> Result<Vec<Path>, UnixGlobError> {
            let mut b = UnixGlobBuilder::new(base.clone(), Arc::new(DirectFilesystemOps));
            b.add_patterns(patterns);
            b.glob()
        }

        fn glob_with(
            &self,
            patterns: &[&str],
            discriminator: TestUnixGlobPathDiscriminator,
        ) -> HashSet<Path> {
            let mut b = UnixGlobBuilder::new(self.tmp_path.clone(), Arc::new(DirectFilesystemOps));
            b.add_patterns(patterns)
                .set_path_discriminator(Arc::new(discriminator));
            b.glob().unwrap().into_iter().collect()
        }

        fn assert_glob_matches(&self, patterns: &[&str], expecteds: &[&str]) {
            let result: HashSet<Path> = self
                .glob(&self.tmp_path, patterns)
                .unwrap()
                .into_iter()
                .collect();
            assert_eq!(
                result,
                self.resolve_paths(expecteds),
                "patterns {patterns:?}"
            );
        }

        fn assert_illegal_pattern(&self, pattern: &str) {
            match self.glob(&self.tmp_path, &[pattern]) {
                Err(UnixGlobError::BadPattern(e)) => assert!(e.0.contains("in glob pattern")),
                other => panic!("expected BadPattern for {pattern:?}, got {other:?}"),
            }
        }
    }

    struct TestUnixGlobPathDiscriminator {
        traversal_predicate: Box<dyn Fn(&Path) -> bool + Send + Sync>,
        result_predicate: Box<dyn Fn(&Path, bool) -> bool + Send + Sync>,
    }

    impl UnixGlobPathDiscriminator for TestUnixGlobPathDiscriminator {
        fn should_traverse_directory(&self, path: &Path) -> bool {
            (self.traversal_predicate)(path)
        }

        fn should_include_path_in_result(&self, path: &Path, is_directory: bool) -> bool {
            (self.result_predicate)(path, is_directory)
        }
    }

    #[test]
    fn test_question_mark_match() {
        fixture().assert_glob_matches(&["foo?"], &["food", "fool"]);
    }

    #[test]
    fn test_question_mark_no_match() {
        fixture().assert_glob_matches(&["food/bar?"], &[]);
    }

    #[test]
    fn test_starts_with_star() {
        fixture().assert_glob_matches(&["*oo"], &["foo"]);
    }

    #[test]
    fn test_starts_with_star_with_middle_star() {
        fixture().assert_glob_matches(&["*f*o"], &["foo"]);
    }

    #[test]
    fn test_ends_with_star() {
        fixture().assert_glob_matches(&["foo*"], &["foo", "food", "fool"]);
    }

    #[test]
    fn test_ends_with_star_with_middle_star() {
        fixture().assert_glob_matches(&["f*oo*"], &["foo", "food", "fool"]);
    }

    #[test]
    fn test_middle_star() {
        fixture().assert_glob_matches(&["f*o"], &["foo"]);
    }

    #[test]
    fn test_two_middle_stars() {
        fixture().assert_glob_matches(&["f*o*o"], &["foo"]);
    }

    #[test]
    fn test_single_star_pattern_with_named_child() {
        fixture().assert_glob_matches(&["*/bar"], &["foo/bar"]);
    }

    #[test]
    fn test_single_star_pattern_with_child_glob() {
        fixture().assert_glob_matches(
            &["*/bar*"],
            &["foo/bar", "foo/barnacle", "food/barnacle", "fool/barnacle"],
        );
    }

    #[test]
    fn test_single_star_as_child_glob() {
        fixture().assert_glob_matches(&["foo/*/wiz"], &["foo/bar/wiz", "foo/barnacle/wiz"]);
    }

    #[test]
    fn test_no_asterisk_and_files_dont_exist() {
        // Note un-UNIX like semantics:
        fixture().assert_glob_matches(&["ceci/n'est/pas/une/globbe"], &[]);
    }

    #[test]
    fn test_single_asterisk_under_nonexistent_directory() {
        // Note un-UNIX like semantics:
        fixture().assert_glob_matches(&["not-there/*"], &[]);
    }

    #[test]
    fn test_filtered_results_no_dirs() {
        let f = fixture();
        let result = f.glob_with(
            &["**"],
            TestUnixGlobPathDiscriminator {
                traversal_predicate: Box::new(|_| true),
                result_predicate: Box::new(|_, is_dir| !is_dir),
            },
        );
        assert_eq!(result, f.resolve_paths(&["foo/bar/wiz/file"]));
    }

    #[test]
    fn test_filtered_results_no_files() {
        let f = fixture();
        let result = f.glob_with(
            &["**"],
            TestUnixGlobPathDiscriminator {
                traversal_predicate: Box::new(|_| true),
                result_predicate: Box::new(|_, is_dir| is_dir),
            },
        );
        assert_eq!(
            result,
            f.resolve_paths(&[
                "",
                "foo",
                "foo/bar",
                "foo/bar/wiz",
                "foo/barnacle",
                "foo/barnacle/wiz",
                "food",
                "food/barnacle",
                "food/barnacle/wiz",
                "fool",
                "fool/barnacle",
                "fool/barnacle/wiz",
            ])
        );
    }

    #[test]
    fn test_filtered_results_path_match() {
        let f = fixture();
        let wanted = f.tmp_path.get_relative_str("food/barnacle/wiz");
        let w = wanted.clone();
        let result = f.glob_with(
            &["**"],
            TestUnixGlobPathDiscriminator {
                traversal_predicate: Box::new(|_| true),
                result_predicate: Box::new(move |path, _| *path == w),
            },
        );
        assert_eq!(result, [wanted].into_iter().collect());
    }

    #[test]
    fn test_traversal_only_foo() {
        // Use a directory traversal filter to only walk the root dir and "foo", but not "fool or
        // "food". So we'll end up the directories, "fool" and "food", but not sub-dirs.
        let f = fixture();
        let tmp = f.tmp_path.clone();
        let result = f.glob_with(
            &["**"],
            TestUnixGlobPathDiscriminator {
                traversal_predicate: Box::new(move |path| {
                    *path == tmp
                        || path.get_path_string().contains("foo/")
                        || path.get_path_string().ends_with("foo")
                }),
                result_predicate: Box::new(|_, _| true),
            },
        );
        assert_eq!(
            result,
            f.resolve_paths(&[
                "",
                "foo",
                "foo/bar",
                "foo/bar/wiz",
                "foo/bar/wiz/file",
                "foo/barnacle",
                "foo/barnacle/wiz",
                "fool",
                "food",
            ])
        );
    }

    #[test]
    fn test_glob_with_non_existent_base() {
        let f = fixture();
        let base = f.tmp_path.get_relative_str("does/not/exist");
        assert!(f.glob(&base, &["*.txt"]).unwrap().is_empty());
    }

    #[test]
    fn test_glob_under_file() {
        fixture().assert_glob_matches(&["foo/bar/wiz/file/*"], &[]);
    }

    /// `FilesystemOps` whose stat or readdir fails as a test asks.
    struct FailingOps {
        stat_error: bool,
        readdir_panics: bool,
        /// `throwOnReaddir`: a readdir of this path throws `FileNotFoundException(path)`.
        throw_on_readdir: Option<Path>,
    }

    impl FilesystemOps for FailingOps {
        fn stat_if_found(&self, path: &Path) -> io::Result<Option<FileStatus>> {
            if self.stat_error {
                return Err(io::Error::other("EIO"));
            }
            DirectFilesystemOps.stat_if_found(path)
        }

        fn readdir(&self, path: &Path) -> io::Result<Vec<Dirent>> {
            assert!(!self.readdir_panics, "IllegalStateException");
            if self.throw_on_readdir.as_ref() == Some(path) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    path.get_path_string().to_string(),
                ));
            }
            DirectFilesystemOps.readdir(path)
        }
    }

    #[test]
    fn test_io_failure_on_stat() {
        let f = fixture();
        let mut b = UnixGlobBuilder::new(
            f.tmp_path.clone(),
            Arc::new(FailingOps {
                stat_error: true,
                readdir_panics: true,
                throw_on_readdir: None,
            }),
        );
        b.add_pattern("foo/bar/wiz/file");
        match b.glob() {
            Err(UnixGlobError::Io(e)) => assert_eq!(e.to_string(), "EIO"),
            other => panic!("expected an IOException, got {other:?}"),
        }
    }

    #[test]
    fn test_glob_without_wildcards_does_not_call_readdir() {
        let f = fixture();
        let mut b = UnixGlobBuilder::new(
            f.tmp_path.clone(),
            Arc::new(FailingOps {
                stat_error: false,
                readdir_panics: true,
                throw_on_readdir: None,
            }),
        );
        b.add_pattern("foo/bar/wiz/file");
        assert_eq!(
            b.glob().unwrap(),
            vec![f.tmp_path.get_relative_str("foo/bar/wiz/file")]
        );
    }

    #[test]
    fn test_illegal_patterns() {
        let f = fixture();
        f.assert_illegal_pattern("foo**bar");
        f.assert_illegal_pattern("");
        f.assert_illegal_pattern(".");
        f.assert_illegal_pattern("/foo");
        f.assert_illegal_pattern("./foo");
        f.assert_illegal_pattern("foo/");
        f.assert_illegal_pattern("foo/./bar");
        f.assert_illegal_pattern("../foo/bar");
        f.assert_illegal_pattern("foo//bar");
    }

    /// Tests that globs can contain Java regular expression special characters.
    #[test]
    fn test_special_regex_character() {
        let f = fixture();
        let tmp_path2 = f.tmp_path.get_relative_str("../globtmp2");
        std::fs::create_dir_all(platform(&tmp_path2)).unwrap();
        let mk = |name: &str| {
            let p = tmp_path2.get_child(name);
            std::fs::write(platform(&p), "").unwrap();
            p
        };
        let a_dot_b = mk("a.b");
        let a_plus_b = mk("a+b");
        let a_word_character_b = mk("a\\wb");
        let disjunctions_and_brackets = mk("aab|a{1,2}[ab]");
        let line_noise = mk("\\|}[{[].+");
        mk("aab");
        // Note: these contain two asterisks because otherwise a RE is not built,
        // as an optimization.
        assert_eq!(f.glob(&tmp_path2, &["*a.b*"]).unwrap(), vec![a_dot_b]);
        assert_eq!(f.glob(&tmp_path2, &["*a+b*"]).unwrap(), vec![a_plus_b]);
        assert_eq!(
            f.glob(&tmp_path2, &["*a\\wb*"]).unwrap(),
            vec![a_word_character_b]
        );
        assert_eq!(
            f.glob(&tmp_path2, &["*aab|a{1,2}[ab]*"]).unwrap(),
            vec![disjunctions_and_brackets]
        );
        assert_eq!(
            f.glob(&tmp_path2, &["*\\|}[{[].+*"]).unwrap(),
            vec![line_noise]
        );
    }

    /// Test that '(' and ')' in glob patterns are ignored if the glob is compiled to regexp.
    #[test]
    fn test_parentheses_in_regex() {
        let f = fixture();
        let tmp_path3 = f.tmp_path.get_relative_str("../globtmp3");
        std::fs::create_dir_all(platform(&tmp_path3)).unwrap();
        let foo_bar = tmp_path3.get_child("foo bar");
        std::fs::write(platform(&foo_bar), "").unwrap();
        let foo_bar_in_parentheses = tmp_path3.get_child("foo (bar)");
        std::fs::write(platform(&foo_bar_in_parentheses), "").unwrap();
        // Note: these contain two asterisks because otherwise a RE is not built,
        // as an optimization.
        for pattern in [
            "*foo (bar)*",
            "(*foo bar*)",
            "*)((foo ))bar(*",
            "*foo (bar*",
            "*foo bar*)",
        ] {
            assert_eq!(
                f.glob(&tmp_path3, &[pattern]).unwrap(),
                vec![foo_bar.clone()],
                "{pattern}"
            );
        }
        // Note: the following glob pattern doesn't contain asterisks, and a RE wouldn't be
        // expected to be built.
        assert_eq!(
            f.glob(&tmp_path3, &["foo (bar)"]).unwrap(),
            vec![foo_bar_in_parentheses]
        );
    }

    #[test]
    fn test_matches_call_with_no_cache() {
        assert!(matches_cached("*a*b", "CaCb", None));
    }

    // Not a Java test (oracle, ~/.cache/blaze/work/bytes-C1): Java's `.` does not match a line
    // terminator, and file names are internal strings, so `x*y.txt` matches `xéy.txt`
    // (78 C3 A9 79 ...) but not `xÅy.txt` (78 C3 85 79 ...): U+0085 is a line terminator.
    #[test]
    fn wildcard_dot_does_not_match_java_line_terminators() {
        assert!(matches("x*y.txt", "x\u{c3}\u{a9}y.txt"));
        assert!(!matches("x*y.txt", "x\u{c3}\u{85}y.txt"));
        assert!(!matches("a?b", "a\rb"));
        assert!(!matches("a?b", "a\u{85}b"));
        assert!(matches("a?b", "a\u{a0}b"));
        assert!(matches("a?b", "a\u{ff}b"));
        // The common cases do not compile a regex: `*.xyz` and `xyz*` match any suffix.
        assert!(matches("x*", "x\u{c3}\u{85}"));
        // A 4-byte UTF-8 name (🐷) is four chars.
        assert!(matches("p????q", "p\u{f0}\u{9f}\u{90}\u{b7}q"));
    }

    #[test]
    fn test_multiple_patterns() {
        fixture().assert_glob_matches(&["foo", "fool"], &["foo", "fool"]);
    }

    #[test]
    fn test_matcher_method_recursive_below_dir() {
        let pattern = "foo/**/*";
        assert!(matches(pattern, "foo/bar"));
        assert!(matches(pattern, "foo/bar/baz"));
        assert!(!matches(pattern, "foo"));
        assert!(!matches(pattern, "foob"));
        assert!(matches("**/foo", "foo"));
    }

    #[test]
    fn test_multiple_patterns_with_overlap() {
        let f = fixture();
        f.assert_glob_matches(&["food", "foo?"], &["food", "fool"]);
        f.assert_glob_matches(&["food", "?ood", "f??d"], &["food"]);
        f.assert_glob_matches(&["food", "xxx", "*"], &["food", "fool", "foo"]);
    }

    #[test]
    fn test_hidden_files() {
        let f = fixture();
        for dir in [".hidden", "..also.hidden", "not.hidden"] {
            std::fs::create_dir_all(platform(&f.tmp_path.get_relative_str(dir))).unwrap();
        }
        // Note that these are not in the result: ".", ".."
        f.assert_glob_matches(
            &["*"],
            &[
                "not.hidden",
                "foo",
                "fool",
                "food",
                ".hidden",
                "..also.hidden",
            ],
        );
        f.assert_glob_matches(&["*.hidden"], &["not.hidden"]);
        f.assert_glob_matches(&[".*also*"], &["..also.hidden"]);
    }

    /// PORT: Java's file system throws on a readdir of `throwOnReaddir`; the failure is injected
    /// through `FilesystemOps` here (the glob sees the same error).
    #[test]
    fn test_io_exception() {
        let f = fixture();
        let throw_on_readdir = f.tmp_path.get_relative_str("throw_on_readdir");
        throw_on_readdir.create_directory().unwrap();
        let mut b = UnixGlobBuilder::new(
            throw_on_readdir.clone(),
            Arc::new(FailingOps {
                stat_error: false,
                readdir_panics: false,
                throw_on_readdir: Some(throw_on_readdir.clone()),
            }),
        );
        b.add_pattern("**");
        assert!(matches!(b.glob(), Err(UnixGlobError::Io(_))));
    }

    #[test]
    #[ignore = "Blaze globs on the calling thread; Java's executor interruption does not apply"]
    fn test_fast_failureith_interrupt() {}

    #[test]
    #[ignore = "Blaze globs on the calling thread; Java's executor interruption does not apply"]
    fn test_check_can_be_interrupted() {}

    #[test]
    #[ignore = "Blaze globs on the calling thread; Java's executor interruption does not apply"]
    fn test_check_cannot_be_interrupted() {}

    fn remove_excludes_of(paths: &[&str], excludes: &[&str]) -> HashSet<String> {
        let mut path_set: HashSet<String> = paths.iter().map(|s| (*s).to_owned()).collect();
        let excludes: Vec<String> = excludes.iter().map(|s| (*s).to_owned()).collect();
        remove_excludes(&mut path_set, &excludes).unwrap();
        path_set
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn test_exclude_filtering() {
        let paths = ["a/A.java", "a/B.java", "a/b/C.java", "c.cc"];
        assert_eq!(remove_excludes_of(&paths, &["**/*.java"]), set(&["c.cc"]));
        assert_eq!(remove_excludes_of(&paths, &["a/**/*.java"]), set(&["c.cc"]));
        assert_eq!(remove_excludes_of(&paths, &["**/nomatch.*"]), set(&paths));
        assert_eq!(
            remove_excludes_of(&paths, &["a/A.java"]),
            set(&["a/B.java", "a/b/C.java", "c.cc"])
        );
        assert_eq!(
            remove_excludes_of(&paths, &["a/?.java"]),
            set(&["a/b/C.java", "c.cc"])
        );
        assert_eq!(
            remove_excludes_of(&paths, &["a/*/C.java"]),
            set(&["a/A.java", "a/B.java", "c.cc"])
        );
        assert!(remove_excludes_of(&paths, &["**"]).is_empty());
        assert!(remove_excludes_of(&paths, &["**/**"]).is_empty());

        // Test filenames that look like code patterns.
        let paths = ["a/A.java", "a/B.java", "a/b/*.java", "a/b/C.java", "c.cc"];
        assert_eq!(remove_excludes_of(&paths, &["**/*.java"]), set(&["c.cc"]));
        assert_eq!(
            remove_excludes_of(&paths, &["**/A.java", "**/B.java", "**/C.java"]),
            set(&["a/b/*.java", "c.cc"])
        );
    }
}
