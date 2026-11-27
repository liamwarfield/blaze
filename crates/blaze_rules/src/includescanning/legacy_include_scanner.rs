//! `LegacyIncludeScanner` (which, despite its name, is not really legacy), its inclusion cache,
//! and `PathExistenceCache`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_vfs::{Path, PathFragment, Root};
use indexmap::IndexSet;
use parking_lot::Mutex;

use super::include_parser::{IncludeParser, Inclusion, Kind};
use super::inclusion_with_context::{IncludeScannerHelper, InclusionWithContext};
use crate::cpp::include_scanner::{
    IncludeScanner, IncludeScanningError, IncludeScanningHeaderData,
};

/// Cache to store file existence status for include paths. Only paths that are considered
/// immutable for the duration of the build (any path outside of blaze-out directory will satisfy
/// that criteria) are cached. This information is used by [`LegacyIncludeScanner`].
// Java: com/google/devtools/build/lib/includescanning/PathExistenceCache.java:33 PathExistenceCache
pub struct PathExistenceCache {
    exec_root: Path,
    artifact_factory: Arc<ArtifactFactory>,
    file_existence_cache: Mutex<HashMap<PathFragment, bool>>,
    directory_existence_cache: Mutex<HashMap<PathFragment, bool>>,
}

impl PathExistenceCache {
    // Java: com/google/devtools/build/lib/includescanning/PathExistenceCache.java:42 PathExistenceCache.<init>
    pub fn new(exec_root: Path, artifact_factory: Arc<ArtifactFactory>) -> PathExistenceCache {
        PathExistenceCache {
            exec_root,
            artifact_factory,
            file_existence_cache: Mutex::new(HashMap::new()),
            directory_existence_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if given path exists and is a file, false otherwise.
    // Java: com/google/devtools/build/lib/includescanning/PathExistenceCache.java:48 PathExistenceCache.fileExists
    pub fn file_exists(&self, exec_path: &PathFragment, is_source: bool) -> bool {
        if let Some(&exists) = self.file_existence_cache.lock().get(exec_path) {
            return exists;
        }
        let path = if is_source {
            self.artifact_factory
                .get_path_from_source_exec_path(&self.exec_root, exec_path)
        } else {
            self.exec_root.get_relative(exec_path)
        };
        let exists = path.is_file();
        *self
            .file_existence_cache
            .lock()
            .entry(exec_path.clone())
            .or_insert(exists)
    }

    /// Returns true if given path exists and is a directory, false otherwise.
    // Java: com/google/devtools/build/lib/includescanning/PathExistenceCache.java:70 PathExistenceCache.directoryExists
    pub fn directory_exists(&self, exec_path: &PathFragment) -> bool {
        if let Some(&exists) = self.directory_existence_cache.lock().get(exec_path) {
            return exists;
        }
        let path = self
            .artifact_factory
            .get_path_from_source_exec_path(&self.exec_root, exec_path);
        let exists = path.is_directory();
        *self
            .directory_existence_cache
            .lock()
            .entry(exec_path.clone())
            .or_insert(exists)
    }
}

/// `(artifact, contextKind, contextPathPos)`: one visit of the include traversal.
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:76 LegacyIncludeScanner.ArtifactWithInclusionContext
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:88 ArtifactWithInclusionContext.hashCode
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:93 ArtifactWithInclusionContext.equals
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ArtifactWithInclusionContext {
    artifact: Artifact,
    context_kind: Option<Kind>,
    context_path_pos: i32,
}

// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:371 LegacyIncludeScanner.LocateOnPathResult
#[derive(Clone, Debug)]
struct LocateOnPathResult {
    path: Option<Artifact>,
    include_position: i32,
    viewed_illegal_output_file: bool,
}

impl LocateOnPathResult {
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:387 LocateOnPathResult.create
    fn create(
        path: Artifact,
        include_position: i32,
        viewed_illegal_output_file: bool,
    ) -> LocateOnPathResult {
        LocateOnPathResult {
            path: Some(path),
            include_position,
            viewed_illegal_output_file,
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:393 LocateOnPathResult.createNotFound
    fn create_not_found(viewed_illegal_output_file: bool) -> LocateOnPathResult {
        LocateOnPathResult {
            path: None,
            include_position: -1,
            viewed_illegal_output_file,
        }
    }
}

/// C include scanner. Quickly scans C/C++ source files to determine the bounding set of
/// transitively referenced include files.
///
/// Maintains caches for parses and search-matches for performance.
///
/// PORT: the traversal runs in the calling thread (Java's `--experimental_include_scanning_
/// parallelism=0`, a direct executor), with an explicit work list in place of the recursion
/// (`process` → `findAndProcess` → `processAsyncIfNotExtracted`), and without the shuffling
/// (which Java does only to spread the work over the pool's threads). The set of visited
/// `(artifact, contextKind, contextPathPos)` triples, and so the result, is the same in any
/// order; only the documented "INCORRECT" reuse of a cached lookup (b/150307245) can depend on
/// which action populated the shared [`InclusionCache`] first, which is unordered in Java too.
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:75 LegacyIncludeScanner
pub struct LegacyIncludeScanner {
    exec_root: Path,
    artifact_factory: Arc<ArtifactFactory>,
    /// Externally-scoped cache of file path => parsed inclusion set mappings. Shared by scanners
    /// with different search paths.
    file_parse_cache: Arc<Mutex<HashMap<Artifact, Arc<Vec<Inclusion>>>>>,
    parser: Arc<IncludeParser>,
    /// Search path for searching for all quoted "xyz.h" includes, composed of all the -iquote, -I
    /// and -isystem paths (in this order).
    quote_include_paths: Vec<PathFragment>,
    /// The index position within quoteIncludePaths at which framework paths (-F) should be
    /// searched.
    quote_include_paths_framework_index: usize,
    /// Search path for searching for all includes, composed of all the -I and -isystem paths (in
    /// this order).
    include_paths: Vec<PathFragment>,
    /// Search path for searching for all includes from frameworks.
    framework_include_paths: Vec<PathFragment>,
    output_path_fragment: PathFragment,
    absolute_root: Root,
    /// Scanner-scoped cache of inclusions with their resolved files and include path entries.
    inclusion_cache: Mutex<HashMap<InclusionWithContext, LocateOnPathResult>>,
    path_cache: Arc<PathExistenceCache>,
}

impl LegacyIncludeScanner {
    /// Constructs a new IncludeScanner.
    ///
    /// PORT: `includePool`, `shouldShuffle` and `spawnIncludeScannerSupplier` are not taken (see
    /// the type's documentation and `IncludeParser::extract_inclusions_from_file`).
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:462 LegacyIncludeScanner.<init>
    pub fn new(
        parser: Arc<IncludeParser>,
        cache: Arc<Mutex<HashMap<Artifact, Arc<Vec<Inclusion>>>>>,
        path_cache: Arc<PathExistenceCache>,
        quote_include_paths: &[PathFragment],
        include_paths: &[PathFragment],
        framework_include_paths: &[PathFragment],
        output_path: &Path,
        exec_root: Path,
        artifact_factory: Arc<ArtifactFactory>,
    ) -> LegacyIncludeScanner {
        let mut all_quote_include_paths = quote_include_paths.to_vec();
        all_quote_include_paths.extend(include_paths.iter().cloned());
        let absolute_root = Root::absolute_root(exec_root.get_file_system());
        LegacyIncludeScanner {
            parser,
            file_parse_cache: cache,
            path_cache,
            artifact_factory,
            quote_include_paths: all_quote_include_paths,
            quote_include_paths_framework_index: quote_include_paths.len(),
            include_paths: include_paths.to_vec(),
            framework_include_paths: framework_include_paths.to_vec(),
            inclusion_cache: Mutex::new(HashMap::new()),
            output_path_fragment: output_path.relative_to(&exec_root),
            exec_root,
            absolute_root,
        }
    }

    /// Locates an included file along the search paths. The result is cacheable.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:124 InclusionCache.locateOnPaths
    fn locate_on_paths(
        &self,
        inclusion: &InclusionWithContext,
        header_data: &IncludeScanningHeaderData,
        only_check_generated: bool,
    ) -> LocateOnPathResult {
        let name = &inclusion.get_inclusion().path_fragment;

        // A framework header must begin with a framework name, followed by a path separator,
        // followed by the rest of the header path. We do not currently support include_next of
        // framework headers.
        let search_framework_include_paths = !self.framework_include_paths.is_empty()
            && !inclusion.get_inclusion().kind.is_next()
            && !name.contains_uplevel_references()
            && PathFragment::contains_separator(name.get_path_string());

        // For #include_next directives we start searching on the include path where we found the
        // previous inclusion.
        let search_start: i32 = if inclusion.get_inclusion().kind.is_next() {
            inclusion.get_context_path_pos()
        } else {
            0
        };

        // Search the header on the remaining paths.
        let paths = if inclusion.get_context_kind() == Some(Kind::Quote) {
            &self.quote_include_paths
        } else {
            &self.include_paths
        };
        let also_search_framework_at_index: i32 =
            if inclusion.get_context_kind() == Some(Kind::Quote) {
                self.quote_include_paths_framework_index as i32
            } else {
                0
            };
        let also_search_framework_at_index = also_search_framework_at_index.max(search_start);
        let mut viewed_illegal_output = false;
        let mut i = search_start;
        while i < paths.len() as i32 {
            if i == also_search_framework_at_index && search_framework_include_paths {
                let framework_name = format!("{}.framework", name.sub_fragment(0, 1));
                let rel_header_path = name.sub_fragment_from(1);
                let result = self.locate_on_framework_paths(
                    &framework_name,
                    &rel_header_path,
                    header_data,
                    only_check_generated,
                    viewed_illegal_output,
                );
                if result.path.is_some() {
                    return result;
                }
                viewed_illegal_output = viewed_illegal_output || result.viewed_illegal_output_file;
            }
            let mut file_fragment = paths[i as usize].get_relative(name);
            if file_fragment.contains_uplevel_references() {
                // Deal with fragments that escape the execroot. They most likely come right back
                // in.
                let exec_root_relative_path = self.exec_root.get_relative(&file_fragment);
                if exec_root_relative_path.starts_with(&self.exec_root) {
                    // Common case: transform #include "../execroot/foo.h" into #include "foo.h"
                    file_fragment = exec_root_relative_path.relative_to(&self.exec_root);
                } else {
                    // Ugh: successfully escaped the exec root. It's their funeral.
                    file_fragment = exec_root_relative_path.as_fragment().clone();
                }
                // This can happen when we are processing Windows paths with backslashes on Unix,
                // since we do not do any #ifdef processing. We can safely discard these here.
                if file_fragment.contains_uplevel_references() {
                    i += 1;
                    continue;
                }
            }
            if only_check_generated && !self.is_output_file(&file_fragment) {
                i += 1;
                continue;
            }
            viewed_illegal_output =
                viewed_illegal_output || self.is_illegal_output_file(&file_fragment, header_data);
            let is_output_directory = file_fragment.starts_with(&self.output_path_fragment);
            if !self.is_file(&file_fragment, name, !is_output_directory, header_data) {
                i += 1;
                continue;
            }
            let artifact = if is_output_directory {
                // May be a normal output file or an inc_library header.
                match header_data.get_header_artifact(&file_fragment) {
                    Some(artifact) => artifact.clone(),
                    None => {
                        // This happens if an included file exists in a cc_inc_library's output
                        // directory, but is not an output of the cc_inc_library.
                        return LocateOnPathResult::create_not_found(viewed_illegal_output);
                    }
                }
            } else if !file_fragment.is_absolute() {
                match self
                    .artifact_factory
                    .resolve_source_artifact(&file_fragment, &blaze_cmdline::repository_name::MAIN)
                {
                    Some(artifact) => artifact,
                    None => {
                        // There was a real file, but we couldn't resolve it, probably because it
                        // belonged to a package that wasn't actually loaded this build.
                        i += 1;
                        continue;
                    }
                }
            } else {
                // This file is given with an absolute path. We will error out after transitive
                // scanning of the top-level source is finished unless this corresponds to a
                // built-in include directory, and will ignore this artifact in any case, but
                // track it here so that its includes can be processed.
                self.artifact_factory
                    .get_source_artifact(&file_fragment, &self.absolute_root)
            };
            // +1 to account for the virtual entry for relative includes.
            return LocateOnPathResult::create(artifact, i + 1, viewed_illegal_output);
        }

        // Not found.
        LocateOnPathResult::create_not_found(viewed_illegal_output)
    }

    /// Locates an included file along the framework search paths. The result is cacheable.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:243 InclusionCache.locateOnFrameworkPaths
    fn locate_on_framework_paths(
        &self,
        framework_name: &str,
        rel_header_path: &PathFragment,
        header_data: &IncludeScanningHeaderData,
        only_check_generated: bool,
        mut viewed_illegal_output: bool,
    ) -> LocateOnPathResult {
        for include_path in &self.framework_include_paths {
            // Construct the full framework path path/to/foo.framework.
            let full_framework_path = include_path.get_relative_str(framework_name);

            if only_check_generated && !self.is_output_file(&full_framework_path) {
                return LocateOnPathResult::create_not_found(viewed_illegal_output);
            }

            // Look for header in path/to/foo.framework/Headers/
            let found_header_path;
            let mut full_header_path = full_framework_path
                .get_relative_str("Headers")
                .get_relative(rel_header_path);

            viewed_illegal_output = viewed_illegal_output
                || self.is_illegal_output_file(&full_header_path, header_data);
            let is_output_directory = full_header_path.starts_with(&self.output_path_fragment);
            if self.is_file(
                &full_header_path,
                rel_header_path,
                is_output_directory,
                header_data,
            ) {
                found_header_path = full_header_path;
            } else {
                // Look for header in path/to/foo.framework/PrivateHeaders/
                full_header_path = full_framework_path
                    .get_relative_str("PrivateHeaders")
                    .get_relative(rel_header_path);
                viewed_illegal_output = viewed_illegal_output
                    || self.is_illegal_output_file(&full_header_path, header_data);
                if self.is_file(
                    &full_header_path,
                    rel_header_path,
                    is_output_directory,
                    header_data,
                ) {
                    found_header_path = full_header_path;
                } else {
                    continue;
                }
            }

            let artifact = if is_output_directory {
                match header_data.get_header_artifact(&found_header_path) {
                    Some(artifact) => artifact.clone(),
                    // Note that the corresponding case for non-framework paths aborts the search
                    // here, but for framework paths, we keep going.
                    None => continue,
                }
            } else if !found_header_path.is_absolute() {
                match self.artifact_factory.resolve_source_artifact(
                    &found_header_path,
                    &blaze_cmdline::repository_name::MAIN,
                ) {
                    Some(artifact) => artifact,
                    None => continue,
                }
            } else {
                self.artifact_factory
                    .get_source_artifact(&found_header_path, &self.absolute_root)
            };
            // Reset contextPathPos to 0 so that include_next in a framework header searches the
            // include paths from the beginning.
            return LocateOnPathResult::create(artifact, 0, viewed_illegal_output);
        }
        // Not found.
        LocateOnPathResult::create_not_found(viewed_illegal_output)
    }

    /// Locates an included file along the search paths.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:328 InclusionCache.lookup
    fn lookup(
        &self,
        inclusion: &InclusionWithContext,
        header_data: &IncludeScanningHeaderData,
    ) -> LocateOnPathResult {
        let cached = self.inclusion_cache.lock().get(inclusion).cloned();
        let Some(result) = cached else {
            // Do not use computeIfAbsent() as the implementation of locateOnPaths might do
            // multiple file stats.
            let result = self.locate_on_paths(inclusion, header_data, false);
            self.inclusion_cache
                .lock()
                .insert(inclusion.clone(), result.clone());
            return result;
        };
        // If the previous computation for this inclusion had a different pathToDeclaredHeader
        // map, result may not be valid for this lookup. Because this is a hot spot, we tolerate a
        // known correctness bug but try to catch most issues.
        // (1) [correct]: The prior computation found an output file, but that file is not in the
        // current lookup's inputs. We don't reuse the computation. b/149935208.
        // (2) [correct]: The prior computation checked an output path not in its legal outputs,
        // and then didn't find a file anywhere. However, that output file is a legal input for
        // this lookup. We don't reuse the computation. b/2097998.
        // (3) [INCORRECT]: Same as (2), except that the prior computation found a file after
        // checking the output path not in its legal inputs. We incorrectly cache this
        // computation, assuming it is very rare. b/150307245.
        match &result.path {
            Some(path) => {
                if path.is_source_artifact()
                    || header_data.get_header_artifact(path.get_exec_path()) == Some(path)
                {
                    return result;
                }
            }
            None => {
                if !result.viewed_illegal_output_file {
                    return result;
                }
            }
        }

        let result = self.locate_on_paths(inclusion, header_data, true);
        if result.path.is_some() || !result.viewed_illegal_output_file {
            // In this case, the result is now cachable either because a file has been found or
            // because there are no more illegal output files.
            self.inclusion_cache
                .lock()
                .insert(inclusion.clone(), result.clone());
        }
        result
    }

    /// Locates an included file relative to the including file. The result is not cacheable.
    ///
    /// Returns the resolved artifact, or `None` if no file could be found.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:504 LegacyIncludeScanner.locateRelative
    fn locate_relative(
        &self,
        inclusion: &Inclusion,
        header_data: &IncludeScanningHeaderData,
        includer: &Artifact,
        parent: &PathFragment,
    ) -> Option<Artifact> {
        if inclusion.kind != Kind::Quote {
            return None;
        }
        let name = &inclusion.path_fragment;

        // The most effective way to see that something is not a relative inclusion is to see
        // whether the include statement starts with a directory (has a '/') and whether that
        // directory exists. We only do this for source files as we never match generated files
        // against the file system.
        if includer.is_source_artifact() && !name.contains_uplevel_references() {
            let first_segment = name.get_segment(0);
            if first_segment.len() < name.get_path_string().len()
                && !self
                    .path_cache
                    .directory_exists(&parent.get_relative_str(first_segment))
            {
                return None;
            }
        }
        let exec_path = parent.get_relative(name);
        if !self.is_file(&exec_path, name, includer.is_source_artifact(), header_data) {
            return None;
        }
        let parent_directory = includer
            .get_root_relative_path()
            .get_parent_directory()
            .unwrap_or_else(|| panic!("NullPointerException: getParentDirectory()"));
        let root_relative_path = parent_directory.get_relative(name);
        if root_relative_path.contains_uplevel_references() {
            // An include cannot break out of a (package path) root via a relative inclusion.
            return None;
        }
        if let Some(header) = header_data.get_header_artifact(&exec_path) {
            return Some(header.clone());
        }
        let root = includer.get_root();
        let source_artifact = self.artifact_factory.resolve_source_artifact_with_ancestor(
            name,
            Some(parent),
            Some(root),
            &blaze_cmdline::repository_name::MAIN,
        );
        if source_artifact.is_none() {
            // If the name had up-level references, this path may not be under any package.
            // Otherwise, we must have gotten an artifact, since it should be under the same
            // package as the including artifact.
            blaze_util::check_state!(
                name.contains_uplevel_references(),
                "{} {} {} {:?}",
                name,
                parent,
                root_relative_path,
                root
            );
        }
        source_artifact
    }

    /// Returns whether the given path exists in the filesystem.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:555 LegacyIncludeScanner.isFile
    fn is_file(
        &self,
        exec_path: &PathFragment,
        include_as_written: &PathFragment,
        is_source: bool,
        header_data: &IncludeScanningHeaderData,
    ) -> bool {
        if self.is_output_file(exec_path) {
            return header_data.is_declared_header(exec_path);
        }
        if is_source && !exec_path.is_absolute() && exec_path.ends_with(include_as_written) {
            // Verify that the directory of execPath exists as an optimization. If testing whether
            // "a/b/c.h" is a file beneath the include path "e/f/", verify that "e/f/a" and
            // "e/f/a/b" are valid directories (and cache the result).
            let exec_path_segments = exec_path.segment_count();
            let name_segments = include_as_written.segment_count();
            for i in (exec_path_segments - name_segments + 1)..exec_path_segments {
                if !self
                    .path_cache
                    .directory_exists(&exec_path.sub_fragment(0, i))
                {
                    return false;
                }
            }
        }
        // Shortcut: If this is a declared header, it's bound to exist.
        if header_data.is_declared_header(exec_path) {
            return true;
        }
        self.path_cache.file_exists(exec_path, is_source)
    }

    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:639 LegacyIncludeScanner.isIllegalOutputFile
    fn is_illegal_output_file(
        &self,
        include_file: &PathFragment,
        header_data: &IncludeScanningHeaderData,
    ) -> bool {
        self.is_output_file(include_file) && !header_data.is_declared_header(include_file)
    }

    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:644 LegacyIncludeScanner.isOutputFile
    fn is_output_file(&self, path: &PathFragment) -> bool {
        path.starts_with(&self.output_path_fragment)
    }
}

/// `checkForInterrupt(operation, source)`: an interrupt ends the scan with
/// `InterruptedException("Include scanning interrupted while <operation> <source>")`.
///
/// We require passing in the operation and the source to avoid intermediate String operations.
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:628 LegacyIncludeScanner.checkForInterrupt
fn check_for_interrupt(operation: &str, source: &Artifact) -> Result<(), IncludeScanningError> {
    if blaze_shell::thread_interrupt::interrupted() {
        return Err(IncludeScanningError::Interrupted(format!(
            "Include scanning interrupted while {operation} {source}"
        )));
    }
    Ok(())
}

/// One pending `process(source, contextPathPos, contextKind, visited)` call of the visitor.
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:849 IncludeVisitor.processAsyncIfNotExtracted
struct PendingVisit {
    source: Artifact,
    context_path_pos: i32,
    context_kind: Option<Kind>,
}

/// Implements the traversal over source files (see the PORT note on [`LegacyIncludeScanner`]).
// Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:675 LegacyIncludeScanner.IncludeVisitor
struct IncludeVisitor<'a> {
    scanner: &'a LegacyIncludeScanner,
    action_execution_context: &'a dyn ActionExecutionContext,
    grep_includes: Option<&'a Artifact>,
    header_data: &'a IncludeScanningHeaderData,
    /// The set of all processed inclusions, to avoid processing duplicate inclusions.
    visited_inclusions: HashSet<ArtifactWithInclusionContext>,
    pending: Vec<PendingVisit>,
}

impl IncludeVisitor<'_> {
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:703 IncludeVisitor.processInternal
    fn process_internal(
        &mut self,
        main_source: &Artifact,
        sources: &[Artifact],
        cmdline_includes: &[String],
        includes: &mut IndexSet<Artifact>,
        path_hints: &[Artifact],
    ) -> Result<(), IncludeScanningError> {
        // Process cmd line includes, if specified.
        if !cmdline_includes.is_empty() {
            self.process_cmdline_includes(main_source, cmdline_includes, includes)?;
            self.sync(includes)?;
        }

        self.process_bulk_async(sources, includes);
        self.sync(includes)?;

        // Process include hints
        let scanner = self.scanner;
        if let Some(hints) = scanner.parser.get_hints() {
            // Follow "path" hints.
            self.process_bulk_async(path_hints, includes);
            // Follow "file" hints for the primary sources.
            for source in sources {
                self.process_file_level_hints_async(
                    &hints.get_file_level_hinted_inclusions_legacy(source),
                    includes,
                );
            }
            self.sync(includes)?;

            // Follow "file" hints for all included headers, transitively.
            let mut frontier: Vec<Artifact> = includes.iter().cloned().collect();
            while !frontier.is_empty() {
                let mut adjacent: IndexSet<Artifact> = IndexSet::new();
                for include in &frontier {
                    self.process_file_level_hints_async(
                        &hints.get_file_level_hinted_inclusions_legacy(include),
                        &mut adjacent,
                    );
                }
                self.sync(&mut adjacent)?;
                // Keep novel nodes as the next frontier.
                adjacent.retain(|a| includes.insert(a.clone()));
                frontier = adjacent.into_iter().collect();
            }
        }
        Ok(())
    }

    /// Block for the completion of all outstanding visitations: here, run them.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:757 IncludeVisitor.sync
    fn sync(&mut self, visited: &mut IndexSet<Artifact>) -> Result<(), IncludeScanningError> {
        while let Some(visit) = self.pending.pop() {
            self.process(
                &visit.source,
                visit.context_path_pos,
                visit.context_kind,
                visited,
            )?;
        }
        Ok(())
    }

    /// Processes a given file for includes and populates the provided set with the visited
    /// includes.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:781 IncludeVisitor.process
    fn process(
        &mut self,
        source: &Artifact,
        context_path_pos: i32,
        context_kind: Option<Kind>,
        visited: &mut IndexSet<Artifact>,
    ) -> Result<(), IncludeScanningError> {
        check_for_interrupt("processing", source)?;

        let cached = self.scanner.file_parse_cache.lock().get(source).cloned();
        let inclusions = match cached {
            Some(inclusions) => inclusions,
            None => {
                let inclusions = Arc::new(self.scanner.parser.extract_inclusions_from_file(
                    source,
                    self.action_execution_context,
                    self.grep_includes,
                    self.scanner.is_output_file(source.get_exec_path()),
                )?);
                self.scanner
                    .file_parse_cache
                    .lock()
                    .entry(source.clone())
                    .or_insert(inclusions)
                    .clone()
            }
        };

        // For each inclusion: get or locate its target file & recursively process
        let scanner = self.scanner;
        let mut helper =
            IncludeScannerHelper::new(&scanner.include_paths, &scanner.quote_include_paths, source);
        let parent = source
            .get_exec_path()
            .get_parent_directory()
            .unwrap_or_else(|| panic!("NullPointerException: getParentDirectory()"));
        for inclusion in inclusions.iter() {
            let inclusion =
                helper.create_inclusion_with_context(inclusion, context_path_pos, context_kind);
            self.find_and_process(&inclusion, source, &parent, visited)?;
        }
        Ok(())
    }

    /// Visits an inclusion starting from a source file.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:873 IncludeVisitor.findAndProcess
    fn find_and_process(
        &mut self,
        inclusion: &InclusionWithContext,
        source: &Artifact,
        parent: &PathFragment,
        visited: &mut IndexSet<Artifact>,
    ) -> Result<(), IncludeScanningError> {
        // Try to find the included file relative to the file that contains the inclusion.
        // Relative inclusions are handled like the first entry on the quote include path.
        let mut include_file = self.scanner.locate_relative(
            inclusion.get_inclusion(),
            self.header_data,
            source,
            parent,
        );
        let mut context_path_pos = 0;
        let mut context_kind = None;

        check_for_interrupt("visiting", source)?;

        // If nothing has been found, get an inclusion from the cache. This will automatically
        // search on the include paths and populate the cache if necessary.
        if include_file.is_none() {
            let result = self.scanner.lookup(inclusion, self.header_data);
            include_file = result.path;
            context_path_pos = result.include_position;
            context_kind = inclusion.get_context_kind();
        }

        // Recursively process the found file (if not yet done).
        if let Some(include_file) = include_file
            && !self
                .scanner
                .is_illegal_output_file(include_file.get_exec_path(), self.header_data)
            && self.header_data.is_legal_header(&include_file)
            && self
                .visited_inclusions
                .insert(ArtifactWithInclusionContext {
                    artifact: include_file.clone(),
                    context_kind,
                    context_path_pos,
                })
        {
            visited.insert(include_file.clone());
            if self.header_data.is_modular_header(&include_file) {
                return Ok(());
            }
            // processAsyncIfNotExtracted
            self.pending.push(PendingVisit {
                source: include_file,
                context_path_pos,
                context_kind,
            });
        }
        Ok(())
    }

    /// Processes a given list of includes for a given base file and populates the provided set
    /// with the visited includes.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:916 IncludeVisitor.processCmdlineIncludes
    fn process_cmdline_includes(
        &mut self,
        source: &Artifact,
        includes: &[String],
        visited: &mut IndexSet<Artifact>,
    ) -> Result<(), IncludeScanningError> {
        let parent = source
            .get_exec_path()
            .get_parent_directory()
            .unwrap_or_else(|| panic!("NullPointerException: getParentDirectory()"));
        for incl in includes {
            let inclusion = InclusionWithContext::simple(incl, Kind::Quote);
            self.find_and_process(&inclusion, source, &parent, visited)?;
        }
        Ok(())
    }

    /// Processes a bunch sources and adds them and their included files to the provided set.
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:933 IncludeVisitor.processBulkAsync
    fn process_bulk_async(&mut self, sources: &[Artifact], visited: &mut IndexSet<Artifact>) {
        for source in sources {
            // TODO(djasper): This looks suspicious. We should only stop based on
            // visitedInclusions.
            if !visited.insert(source.clone()) {
                continue;
            }
            self.pending.push(PendingVisit {
                source: source.clone(),
                context_path_pos: -1,
                context_kind: None,
            });
        }
    }

    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:945 IncludeVisitor.processFileLevelHintsAsync
    fn process_file_level_hints_async(
        &mut self,
        sources: &[Artifact],
        also_visited: &mut IndexSet<Artifact>,
    ) {
        // Early-out if there's nothing to do to avoid enqueuing a closure
        if sources.is_empty() {
            return;
        }
        self.process_bulk_async(sources, also_visited);
    }
}

impl IncludeScanner for LegacyIncludeScanner {
    // Java: com/google/devtools/build/lib/includescanning/LegacyIncludeScanner.java:586 LegacyIncludeScanner.processAsync
    fn process_async(
        &self,
        main_source: &Artifact,
        sources: &[Artifact],
        include_scanning_header_data: &IncludeScanningHeaderData,
        cmdline_includes: &[String],
        includes: &mut IndexSet<Artifact>,
        action_execution_context: &dyn ActionExecutionContext,
        grep_includes: Option<&Artifact>,
    ) -> Result<(), IncludeScanningError> {
        let path_hints = match self.parser.get_hints() {
            None => Vec::new(),
            Some(hints) => hints.get_path_level_hinted_inclusions(&self.quote_include_paths),
        };

        let mut visitor = IncludeVisitor {
            scanner: self,
            action_execution_context,
            grep_includes,
            header_data: include_scanning_header_data,
            visited_inclusions: HashSet::new(),
            pending: Vec::new(),
        };
        visitor.process_internal(
            main_source,
            sources,
            cmdline_includes,
            includes,
            &path_hints,
        )
    }
}
