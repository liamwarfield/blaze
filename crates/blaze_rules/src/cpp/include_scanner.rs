//! `IncludeScanner` (the interface `lib/includescanning`'s scanners implement) and
//! `IncludeScanner.IncludeScanningHeaderData`, the pre-aggregated information the scanner needs
//! from the compile action.
//!
//! The design notes of the Java interface (how include scanning and `.d` file pruning fit
//! together) are on [`IncludeScanner`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::NestedSet;
use blaze_exec::actions::ExecException;
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

/// `Predicate<Artifact>`: tests whether a header is valid even if it is not declared.
pub type ValidUndeclaredHeaderPredicate = Arc<dyn Fn(&Artifact) -> bool + Send + Sync>;

/// The exceptions `IncludeScanner.processAsync` throws: `IOException`, `ExecException`,
/// `InterruptedException` (`NoSuchPackageException` comes only from include hints, which Bazel
/// does not use; see `includescanning::include_parser::Hints`).
#[derive(Debug)]
pub enum IncludeScanningError {
    Io(std::io::Error),
    Exec(ExecException),
    /// `InterruptedException` with its message (`checkForInterrupt`: "Include scanning
    /// interrupted while <operation> <source>").
    Interrupted(String),
}

impl From<std::io::Error> for IncludeScanningError {
    fn from(e: std::io::Error) -> Self {
        IncludeScanningError::Io(e)
    }
}

/// Scans source files to determine the bounding set of transitively referenced include files.
///
/// Include scanning is performance-critical code since it has to parse a lot of C++ code, mostly
/// on the same machine where Blaze runs.
///
/// Include scanning works by first adding all the potential header files to the "scheduling
/// dependencies" of the action. This makes Skyframe build these files (if they are generated)
/// before the action is executed. Then `Action.discoverInputs()` is called, which runs the
/// include scanning machinery and eventually calls `Action.updateInputs()`.
///
/// The include scanner consists of two parts: the part that parses the source files and extracts
/// the include directives (`IncludeParser`, or remotely `GrepIncludesAction`), and the logic that
/// evaluates the include directives and recursively parses the referenced files
/// (`LegacyIncludeScanner`). After the include scanner is done, the compile runs and the `.d`
/// file it writes prunes the headers the compiler did not use.
// Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:92 IncludeScanner
pub trait IncludeScanner: Send + Sync {
    /// Processes source files and a list of includes extracted from command line flags. Adds all
    /// found files to the provided set `includes`.
    ///
    /// The resulting set will include `main_source` and `sources`. `main_source` is the source
    /// file relative to which the `cmdline_includes` are interpreted.
    ///
    /// PORT: `actionExecutionMetadata` and `grepIncludesExecutionPlatform` are read only by the
    /// remote (`SpawnIncludeScanner`) path, which is `unported!`; they are not passed.
    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:126 IncludeScanner.processAsync
    fn process_async(
        &self,
        main_source: &Artifact,
        sources: &[Artifact],
        include_scanning_header_data: &IncludeScanningHeaderData,
        cmdline_includes: &[String],
        includes: &mut IndexSet<Artifact>,
        action_execution_context: &dyn ActionExecutionContext,
        grep_includes: Option<&Artifact>,
    ) -> Result<(), IncludeScanningError>;
}

/// Holds pre-aggregated information that the [`IncludeScanner`] needs from the compilation
/// action.
// Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:142 IncludeScanner.IncludeScanningHeaderData
pub struct IncludeScanningHeaderData {
    /// Lookup table to find the `Artifact`s of generated files based on their exec path.
    path_to_declared_header: HashMap<PathFragment, Artifact>,
    /// The set of headers that are modular, i.e. are going to be read as a serialized AST rather
    /// than from the textual source file.
    modular_headers: HashSet<Artifact>,
    /// The list of "-isystem" include paths that should be used by the IncludeScanner for this
    /// action.
    system_include_dirs: Vec<PathFragment>,
    /// A list of "-include" inclusions specified explicitly on the command line of this action.
    cmdline_includes: Vec<String>,
    /// Tests whether the given artifact is a valid header even if it is not declared, i.e. a
    /// transitive dependency. If `None`, assume all headers can be included.
    is_valid_undeclared_header: Option<ValidUndeclaredHeaderPredicate>,
}

impl IncludeScanningHeaderData {
    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:176 IncludeScanningHeaderData.<init>
    pub fn new(
        path_to_declared_header: HashMap<PathFragment, Artifact>,
        modular_headers: HashSet<Artifact>,
        system_include_dirs: Vec<PathFragment>,
        cmdline_includes: Vec<String>,
        is_valid_undeclared_header: Option<ValidUndeclaredHeaderPredicate>,
    ) -> IncludeScanningHeaderData {
        IncludeScanningHeaderData {
            path_to_declared_header,
            modular_headers,
            system_include_dirs,
            cmdline_includes,
            is_valid_undeclared_header,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:189 IncludeScanningHeaderData.isDeclaredHeader
    pub fn is_declared_header(&self, header: &PathFragment) -> bool {
        self.path_to_declared_header.contains_key(header)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:193 IncludeScanningHeaderData.getHeaderArtifact
    pub fn get_header_artifact(&self, header: &PathFragment) -> Option<&Artifact> {
        self.path_to_declared_header.get(header)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:197 IncludeScanningHeaderData.isModularHeader
    pub fn is_modular_header(&self, header: &Artifact) -> bool {
        self.modular_headers.contains(header)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:201 IncludeScanningHeaderData.getSystemIncludeDirs
    pub fn get_system_include_dirs(&self) -> &[PathFragment] {
        &self.system_include_dirs
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:205 IncludeScanningHeaderData.getCmdlineIncludes
    pub fn get_cmdline_includes(&self) -> &[String] {
        &self.cmdline_includes
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:209 IncludeScanningHeaderData.isLegalHeader
    pub fn is_legal_header(&self, header: &Artifact) -> bool {
        match &self.is_valid_undeclared_header {
            None => true,
            Some(is_valid_undeclared_header) => {
                self.path_to_declared_header
                    .contains_key(header.get_exec_path())
                    || is_valid_undeclared_header(header)
            }
        }
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:215 IncludeScanningHeaderData.Builder
pub struct IncludeScanningHeaderDataBuilder {
    path_to_declared_header: HashMap<PathFragment, Artifact>,
    modular_headers: HashSet<Artifact>,
    system_include_dirs: Vec<PathFragment>,
    cmdline_includes: Vec<String>,
    is_valid_undeclared_header: Option<ValidUndeclaredHeaderPredicate>,
}

impl IncludeScanningHeaderDataBuilder {
    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:222 Builder.<init>
    pub fn new(
        path_to_declared_header: HashMap<PathFragment, Artifact>,
        modular_headers: HashSet<Artifact>,
    ) -> IncludeScanningHeaderDataBuilder {
        IncludeScanningHeaderDataBuilder {
            path_to_declared_header,
            modular_headers,
            system_include_dirs: Vec::new(),
            cmdline_includes: Vec::new(),
            is_valid_undeclared_header: None,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:229 Builder.setSystemIncludeDirs
    pub fn set_system_include_dirs(mut self, system_include_dirs: Vec<PathFragment>) -> Self {
        self.system_include_dirs = system_include_dirs;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:235 Builder.setCmdlineIncludes
    pub fn set_cmdline_includes(mut self, cmdline_includes: Vec<String>) -> Self {
        self.cmdline_includes = cmdline_includes;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:241 Builder.setIsValidUndeclaredHeader
    pub fn set_is_valid_undeclared_header(
        mut self,
        is_valid_undeclared_header: Option<ValidUndeclaredHeaderPredicate>,
    ) -> Self {
        self.is_valid_undeclared_header = is_valid_undeclared_header;
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:248 Builder.addDeclaredHeaders
    pub fn add_declared_headers(mut self, headers: &NestedSet<Artifact>) -> Self {
        // pathToDeclaredHeader is a fresh, per-action map created by
        // CcCompilationContext#createIncludeScanningHeaderData. Mutating it in place here is
        // therefore safe -- it is not shared across actions.
        for header in headers.to_list().iter() {
            // Only generated (output-directory) headers need to be registered here. Source
            // headers already resolve via source-artifact lookup in the include scanner, so
            // registering them would be redundant. Tree artifacts are handled separately and are
            // skipped.
            if !header.is_source_artifact() && !header.is_tree_artifact() {
                // Use putIfAbsent so a header the target already declared for this exec path
                // takes precedence: the target's own mapping is authoritative and must not be
                // shadowed by a toolchain/prunable header that happens to share the path.
                self.path_to_declared_header
                    .entry(header.get_exec_path().clone())
                    .or_insert_with(|| header.clone());
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/IncludeScanner.java:269 Builder.build
    pub fn build(self) -> IncludeScanningHeaderData {
        IncludeScanningHeaderData::new(
            self.path_to_declared_header,
            self.modular_headers,
            self.system_include_dirs,
            self.cmdline_includes,
            self.is_valid_undeclared_header,
        )
    }
}
