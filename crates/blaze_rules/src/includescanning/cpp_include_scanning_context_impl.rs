//! `IncludeScannerSupplier` and `CppIncludeScanningContextImpl`.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_exec::actions::{EnvironmentalExecException, ExecException, UserExecException};
use blaze_exec::analysis::actions::get_input_path;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_util::profiler::{Profiler, ProfilerTask};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexSet;
use parking_lot::Mutex;

use super::include_parser::{IncludeParser, Inclusion};
use super::legacy_include_scanner::{LegacyIncludeScanner, PathExistenceCache};
use crate::cpp::cpp_compile_action::CppCompileAction;
use crate::cpp::cpp_include_scanning_context::CppIncludeScanningContext;
use crate::cpp::include_scanner::{
    IncludeScanner, IncludeScanningError, IncludeScanningHeaderData,
};

/// `(quoteIncludePaths, includePaths, frameworkIncludePaths)`: the key of a scanner.
// Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:41 IncludeScannerSupplier.IncludeScannerParams
// Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:56 IncludeScannerParams.hashCode
// Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:61 IncludeScannerParams.equals
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct IncludeScannerParams {
    quote_include_paths: Vec<PathFragment>,
    include_paths: Vec<PathFragment>,
    framework_include_paths: Vec<PathFragment>,
}

/// Creates include scanner instances.
///
/// Each include scanner is specific to a given triplet (-I, -isystem, -iquote) of include paths.
///
/// PORT: Java keeps the scanners with weak values (the garbage collector may drop a scanner, and
/// its inclusion cache, that no action uses at the moment); here they live as long as the
/// supplier, which is one command. The cached lookups are only an optimization (see
/// `LegacyIncludeScanner::lookup`).
// Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:40 IncludeScannerSupplier
pub struct IncludeScannerSupplier {
    include_parser: Mutex<Option<Arc<IncludeParser>>>,
    /// Cache of include scan results mapping source paths to sets of scanned inclusions. Shared
    /// by all scanner instances.
    include_parse_cache: Arc<Mutex<HashMap<Artifact, Arc<Vec<Inclusion>>>>>,
    /// Cache of include scanner instances mapped by include-path hashes.
    scanners: Mutex<HashMap<IncludeScannerParams, Arc<LegacyIncludeScanner>>>,
    path_cache: Arc<PathExistenceCache>,
    output_path: Path,
    exec_root: Path,
    artifact_factory: Arc<ArtifactFactory>,
}

impl IncludeScannerSupplier {
    /// `output_path` is `directories.getOutputPath(execRoot.getBaseName())`.
    ///
    /// PORT: `includePool`, `shouldShuffle` and `spawnIncludeScannerSupplier` are not taken (see
    /// [`LegacyIncludeScanner`]).
    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:86 IncludeScannerSupplier.<init>
    pub fn new(
        output_path: Path,
        exec_root: Path,
        artifact_factory: Arc<ArtifactFactory>,
    ) -> IncludeScannerSupplier {
        // Map of grepped include files from input (.cc or .h) to a header-grepped file.
        let path_cache = Arc::new(PathExistenceCache::new(
            exec_root.clone(),
            artifact_factory.clone(),
        ));
        IncludeScannerSupplier {
            include_parser: Mutex::new(None),
            include_parse_cache: Arc::new(Mutex::new(HashMap::new())),
            scanners: Mutex::new(HashMap::new()),
            path_cache,
            output_path,
            exec_root,
            artifact_factory,
        }
    }

    /// Returns the possibly shared scanner to be used for a given triplet of include paths. The
    /// paths are specified as PathFragments relative to the execution root.
    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:128 IncludeScannerSupplier.scannerFor
    pub fn scanner_for(
        &self,
        quote_include_paths: &[PathFragment],
        include_paths: &[PathFragment],
        framework_paths: &[PathFragment],
    ) -> Arc<dyn IncludeScanner> {
        let include_parser = self
            .include_parser
            .lock()
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: includeParser"));
        let key = IncludeScannerParams {
            quote_include_paths: quote_include_paths.to_vec(),
            include_paths: include_paths.to_vec(),
            framework_include_paths: framework_paths.to_vec(),
        };
        let mut scanners = self.scanners.lock();
        let scanner = scanners.entry(key).or_insert_with_key(|key| {
            Arc::new(LegacyIncludeScanner::new(
                include_parser,
                self.include_parse_cache.clone(),
                self.path_cache.clone(),
                &key.quote_include_paths,
                &key.include_paths,
                &key.framework_include_paths,
                &self.output_path,
                self.exec_root.clone(),
                self.artifact_factory.clone(),
            ))
        });
        scanner.clone()
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerSupplier.java:136 IncludeScannerSupplier.init
    pub fn init(&self, include_parser: IncludeParser) {
        let mut current = self.include_parser.lock();
        blaze_util::check_state!(
            current.is_none(),
            "Must only be initialized once: {:?} {:?}",
            current,
            include_parser
        );
        blaze_util::check_state!(self.include_parse_cache.lock().is_empty());
        blaze_util::check_state!(self.scanners.lock().is_empty());
        *current = Some(Arc::new(include_parser));
    }
}

/// Include scanning context implementation.
// Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:52 CppIncludeScanningContextImpl
pub struct CppIncludeScanningContextImpl {
    include_scanner_supplier: Arc<IncludeScannerSupplier>,
}

impl CppIncludeScanningContextImpl {
    // Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:55 CppIncludeScanningContextImpl.<init>
    pub fn new(
        include_scanner_supplier: Arc<IncludeScannerSupplier>,
    ) -> CppIncludeScanningContextImpl {
        CppIncludeScanningContextImpl {
            include_scanner_supplier,
        }
    }

    /// Returns a list of artifacts with all tree artifacts replaced by their expansion or `None`
    /// if we are missing a Skyframe dependency.
    ///
    /// PORT: the expansion asks Skyframe for the trees' values
    /// (`getEnvironmentForDiscoveringInputs()`), which the execution context does not give;
    /// a tree among the scanned sources is `unported!`.
    // Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:98 CppIncludeScanningContextImpl.expandTreeArtifacts
    fn expand_tree_artifacts(artifacts: Vec<Artifact>) -> Option<Vec<Artifact>> {
        if !artifacts.iter().any(|a| a.is_tree_artifact()) {
            return Some(artifacts);
        }
        blaze_util::unported!(
            "CppIncludeScanningContextImpl.expandTreeArtifacts",
            "a tree artifact among the include scanning sources needs \
             ActionExecutionContext.getEnvironmentForDiscoveringInputs"
        )
    }

    // Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:174 CppIncludeScanningContextImpl.collect
    fn collect(
        action_execution_context: &dyn ActionExecutionContext,
        includes: &IndexSet<Artifact>,
        absolute_built_in_include_dirs: &[PathFragment],
        source_file: &Artifact,
    ) -> Result<Vec<Artifact>, ExecException> {
        // Collect inputs and output
        let mut inputs = Vec::with_capacity(includes.len());
        for included in includes {
            // The include scanner adds the sources it scans to the includes, but the source file
            // is already a mandatory input of the action and thus doesn't need to be discovered.
            if included == source_file {
                continue;
            }
            // Check for absolute includes -- we assign the file system root as the root path for
            // such includes
            if included.get_root().get_root().is_absolute() {
                if starts_with_any(
                    get_input_path(action_execution_context, included).as_fragment(),
                    absolute_built_in_include_dirs,
                ) {
                    // Skip include files found in absolute include directories.
                    continue;
                }
                return Err(ExecException::User(UserExecException::new(
                    create_failure_detail(
                        &format!(
                            "illegal absolute path to include file: {}",
                            get_input_path(action_execution_context, included)
                        ),
                        failure_details::include_scanning::Code::IllegalAbsolutePath,
                    ),
                )));
            }
            match included.get_parent() {
                Some(parent) if parent.is_tree_artifact() => {
                    // Note that this means every file in the TreeArtifact becomes an input to the
                    // action, and we have spurious rebuilds if non-included files change.
                    inputs.push(parent.clone());
                }
                _ => inputs.push(included.clone()),
            }
        }
        Ok(inputs)
    }
}

impl CppIncludeScanningContext for CppIncludeScanningContextImpl {
    // Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:61 CppIncludeScanningContextImpl.findAdditionalInputs
    fn find_additional_inputs(
        &self,
        action: &CppCompileAction,
        action_execution_context: &dyn ActionExecutionContext,
        include_scanning_header_data: &IncludeScanningHeaderData,
    ) -> Result<Option<Vec<Artifact>>, ExecException> {
        let mut includes: IndexSet<Artifact> = IndexSet::new();
        includes.extend(action.get_built_in_include_files().iter().cloned());

        // Deduplicate include directories. This can occur especially with "built-in" and
        // "system" include directories because of the way we retrieve them. Duplicate include
        // directories really mess up #include_next directives.
        let mut include_dirs: IndexSet<PathFragment> =
            action.get_include_dirs().into_iter().collect();
        let quote_include_dirs = action.get_quote_include_dirs();
        let framework_include_dirs = action.get_framework_include_dirs();
        let cmdline_includes = include_scanning_header_data.get_cmdline_includes();

        include_dirs.extend(
            include_scanning_header_data
                .get_system_include_dirs()
                .iter()
                .cloned(),
        );

        // Add the system include paths to the list of include paths.
        let mut absolute_built_in_include_dirs = Vec::new();
        for path_fragment in action.get_built_in_include_directories() {
            if path_fragment.is_absolute() {
                absolute_built_in_include_dirs.push(path_fragment.clone());
            }
            include_dirs.insert(path_fragment.clone());
        }

        let include_dir_list: Vec<PathFragment> = include_dirs.into_iter().collect();
        let scanner = self.include_scanner_supplier.scanner_for(
            &quote_include_dirs,
            &include_dir_list,
            &framework_include_dirs,
        );

        let main_source = action.get_main_include_scanner_source();
        let Some(sources) = Self::expand_tree_artifacts(action.get_include_scanner_sources())
        else {
            return Ok(None);
        };

        let _c = Profiler::instance().profile(
            ProfilerTask::SCANNER,
            action.get_source_file().get_exec_path_string(),
        );
        match scanner.process_async(
            &main_source,
            &sources,
            include_scanning_header_data,
            cmdline_includes,
            &mut includes,
            action_execution_context,
            action.get_grep_includes(),
        ) {
            Ok(()) => {}
            Err(IncludeScanningError::Io(e)) => {
                return Err(ExecException::Environmental(
                    EnvironmentalExecException::with_cause(
                        &e,
                        create_failure_detail(
                            "Include scanning IOException",
                            failure_details::include_scanning::Code::ScanningIoException,
                        ),
                    ),
                ));
            }
            Err(IncludeScanningError::Exec(e)) => return Err(e),
            Err(IncludeScanningError::Interrupted) => return Err(ExecException::Interrupted),
        }
        // actionExecutionContext.getEnvironmentForDiscoveringInputs().valuesMissing(): the
        // scanner requests no Skyframe values here (see Hints).
        Self::collect(
            action_execution_context,
            &includes,
            &absolute_built_in_include_dirs,
            action.get_source_file(),
        )
        .map(Some)
    }
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:919 FileSystemUtils.startsWithAny
fn starts_with_any(path: &PathFragment, prefixes: &[PathFragment]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}

// Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImpl.java:125 CppIncludeScanningContextImpl.createFailureDetail
fn create_failure_detail(
    message: &str,
    detailed_code: failure_details::include_scanning::Code,
) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::IncludeScanning(
            failure_details::IncludeScanning {
                code: detailed_code as i32,
                ..Default::default()
            },
        )),
    }
}

/// `CppIncludeScanningContextImplTest`.
// Java: com/google/devtools/build/lib/includescanning/CppIncludeScanningContextImplTest.java
#[cfg(test)]
mod cpp_include_scanning_context_impl_test {
    #[test]
    #[ignore = "needs BuildViewTestCase and a Skyframe environment for discovering inputs; tree \
                artifact sources are unported! (expandTreeArtifacts)"]
    fn tree_artifact_header_scans_expanded_artifact() {}

    #[test]
    #[ignore = "needs BuildViewTestCase and a Skyframe environment for discovering inputs; tree \
                artifact sources are unported! (expandTreeArtifacts)"]
    fn tree_artifact_and_regular_header_scans_regular_and_expanded_artifact() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (analysis of a cc_library to a CppCompileAction) and a \
                mock IncludeScannerSupplier"]
    fn source_file_added_by_scanner_not_returned_as_input() {}

    #[test]
    #[ignore = "needs BuildViewTestCase (analysis of a cc_library to a CppCompileAction) and a \
                mock IncludeScannerSupplier"]
    fn tree_file_artifact_added_by_scanner_returns_parent_tree_artifact() {}

    #[test]
    #[ignore = "needs BuildViewTestCase and a Skyframe environment for discovering inputs; tree \
                artifact sources are unported! (expandTreeArtifacts)"]
    fn tree_artifact_header_missing_value_returns_null() {}
}
