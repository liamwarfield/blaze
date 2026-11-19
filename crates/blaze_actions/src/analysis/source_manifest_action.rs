//! Port of `lib/analysis/SourceManifestAction.java`.

use std::any::Any;
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, OnceLock};

use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventKind};
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_util::{Fingerprint, unported};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::action::{
    AbstractAction, Action, ComputeKeyError, FileContentsProvider, InputMetadataProvider,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::fingerprint_util::FingerprintPathExt;
use crate::actions::runfiles_tree::RunfilesMapping;
use crate::analysis::actions::abstract_file_write_action::{
    AbstractFileWriteAction, DeterministicWriter, NewDeterministicWriterError, exec_properties,
    new_abstract_file_write_action,
};
use crate::analysis::runfiles::{ConflictPolicy, Runfiles, RunfilesConflictReceiver};

// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:73 SourceManifestAction.GUID
const GUID: &str = "07459553-a3d0-4d37-9d78-18ed942470f4";

/// The `FailureDetail` message of the `UserExecException` `newDeterministicWriter` throws when
/// the runfiles contain a runfiles tree (code `Analysis.Code.INVALID_RUNFILES_TREE`).
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:269 SourceManifestAction.newDeterministicWriter
pub const INVALID_RUNFILES_TREE_MESSAGE: &str = "Cannot create input manifest for runfiles tree";

/// `ROOT_RELATIVE_PATH_ESCAPER`: ' ' -> "\s", '\n' -> "\n", '\\' -> "\b".
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:77 SourceManifestAction.ROOT_RELATIVE_PATH_ESCAPER
fn escape_root_relative_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("\\s"),
            '\n' => out.push_str("\\n"),
            '\\' => out.push_str("\\b"),
            c => out.push(c),
        }
    }
    out
}

/// `TARGET_PATH_ESCAPER`: '\n' -> "\n", '\\' -> "\b".
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:83 SourceManifestAction.TARGET_PATH_ESCAPER
fn escape_target_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\\' => out.push_str("\\b"),
            c => out.push(c),
        }
    }
    out
}

/// Interface for defining manifest formatting and reporting specifics. Implementations must be
/// immutable.
///
/// PORT: Java's `Writer` over ISO-8859-1 becomes a byte sink that is given what that writer
/// encodes (`string_encoding::latin1_encode_lossy`: the internal strings' bytes).
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:92 SourceManifestAction.ManifestWriter
pub trait ManifestWriter: Send + Sync + fmt::Debug {
    /// Writes a single line of manifest output.
    ///
    /// - `manifest_writer`: the output stream
    /// - `root_relative_path`: path of an entry relative to the manifest's root
    /// - `symlink_target`: target of the entry at `root_relative_path` if it is a symlink,
    ///   otherwise `None`
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:102 ManifestWriter.writeEntry
    fn write_entry(
        &self,
        manifest_writer: &mut dyn Write,
        root_relative_path: &PathFragment,
        symlink_target: Option<&PathFragment>,
    ) -> io::Result<()>;

    /// Fulfills `AbstractAction.getMnemonic()`.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:107 ManifestWriter.getMnemonic
    fn get_mnemonic(&self) -> String;

    /// Fulfills `AbstractAction.getRawProgressMessage()`.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:112 ManifestWriter.getRawProgressMessage
    fn get_raw_progress_message(&self) -> String;

    /// Fulfills `AbstractFileWriteAction.isRemotable()`.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:119 ManifestWriter.isRemotable
    fn is_remotable(&self) -> bool;

    /// Whether the manifest includes absolute paths to artifacts.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:122 ManifestWriter.emitsAbsolutePaths
    fn emits_absolute_paths(&self) -> bool;
}

/// Supported manifest writing strategies.
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:356 SourceManifestAction.ManifestType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ManifestType {
    /// Writes each line as:
    ///
    /// `[rootRelativePath] [resolvingSymlink]`
    ///
    /// If rootRelativePath contains spaces, then each backslash is replaced with '\b', each
    /// space is replaced with '\s' and the line is prefixed with a space.
    ///
    /// This strategy is suitable for creating an input manifest to a source view tree. Its
    /// output is a valid input to `SymlinkTreeAction`.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:369 ManifestType.SOURCE_SYMLINKS
    SOURCE_SYMLINKS,

    /// Writes each line as:
    ///
    /// `[rootRelativePath]`
    ///
    /// This strategy is suitable for an input into a packaging system (notably .par) that
    /// consumes a list of all source files but needs that list to be constant with respect to
    /// how the user has their client laid out on local disk.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:437 ManifestType.SOURCES_ONLY
    SOURCES_ONLY,
}

impl ManifestWriter for ManifestType {
    fn write_entry(
        &self,
        manifest_writer: &mut dyn Write,
        root_relative_path: &PathFragment,
        symlink_target: Option<&PathFragment>,
    ) -> io::Result<()> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:377 SOURCE_SYMLINKS.writeEntry
            ManifestType::SOURCE_SYMLINKS => {
                let root_relative_path_string = root_relative_path.get_path_string();
                // Source paths with spaces require escaping. Target paths with spaces don't as
                // consumers are expected to split on the first space. Newlines always need to be
                // escaped. Note that if any of these characters are present, then we also need
                // to escape the escape character (backslash) in both paths. We avoid doing so if
                // none of the problematic characters are present for backwards compatibility
                // with existing runfiles libraries. In particular, entries with a source path
                // that contains neither spaces nor newlines and target paths that contain both
                // spaces and backslashes require no escaping.
                let needs_escaping = root_relative_path_string.contains(' ')
                    || root_relative_path_string.contains('\n')
                    || symlink_target.is_some_and(|t| t.get_path_string().contains('\n'));
                if needs_escaping {
                    manifest_writer.write_all(b" ")?;
                    manifest_writer.write_all(&latin1_encode_lossy(&escape_root_relative_path(
                        root_relative_path_string,
                    )))?;
                } else {
                    manifest_writer.write_all(&latin1_encode_lossy(root_relative_path_string))?;
                }
                // This trailing whitespace is REQUIRED to process the single entry line
                // correctly.
                manifest_writer.write_all(b" ")?;
                if let Some(symlink_target) = symlink_target {
                    if needs_escaping {
                        manifest_writer.write_all(&latin1_encode_lossy(&escape_target_path(
                            symlink_target.get_path_string(),
                        )))?;
                    } else {
                        manifest_writer
                            .write_all(&latin1_encode_lossy(symlink_target.get_path_string()))?;
                    }
                }
                manifest_writer.write_all(b"\n")?;
                Ok(())
            }
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:441 SOURCES_ONLY.writeEntry
            ManifestType::SOURCES_ONLY => {
                manifest_writer
                    .write_all(&latin1_encode_lossy(root_relative_path.get_path_string()))?;
                manifest_writer.write_all(b"\n")?;
                manifest_writer.flush()?;
                Ok(())
            }
        }
    }

    fn get_mnemonic(&self) -> String {
        match self {
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:414 SOURCE_SYMLINKS.getMnemonic
            ManifestType::SOURCE_SYMLINKS => "SourceSymlinkManifest".to_string(),
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:451 SOURCES_ONLY.getMnemonic
            ManifestType::SOURCES_ONLY => "PackagingSourcesManifest".to_string(),
        }
    }

    fn get_raw_progress_message(&self) -> String {
        match self {
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:419 SOURCE_SYMLINKS.getRawProgressMessage
            ManifestType::SOURCE_SYMLINKS => "Creating source manifest".to_string(),
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:456 SOURCES_ONLY.getRawProgressMessage
            ManifestType::SOURCES_ONLY => "Creating file sources list".to_string(),
        }
    }

    fn is_remotable(&self) -> bool {
        match self {
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:424 SOURCE_SYMLINKS.isRemotable
            // There is little gain to remoting these, since they include absolute path names
            // inline.
            ManifestType::SOURCE_SYMLINKS => false,
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:461 SOURCES_ONLY.isRemotable
            // Source-only symlink manifest has root-relative paths and does not include absolute
            // paths.
            ManifestType::SOURCES_ONLY => true,
        }
    }

    fn emits_absolute_paths(&self) -> bool {
        match self {
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:430 SOURCE_SYMLINKS.emitsAbsolutePaths
            ManifestType::SOURCE_SYMLINKS => true,
            // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:466 SOURCES_ONLY.emitsAbsolutePaths
            ManifestType::SOURCES_ONLY => false,
        }
    }
}

/// The conflict receiver of `newDeterministicWriter`: stores the events to replay.
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:233 SourceManifestAction.newDeterministicWriter
struct StoringConflictReceiver<'a> {
    action: &'a SourceManifestAction,
    events: Vec<Event>,
    seen_nested_runfiles_tree: bool,
}

impl RunfilesConflictReceiver for StoringConflictReceiver<'_> {
    fn nested_runfiles_tree(&mut self, runfiles_tree: &Artifact) {
        self.seen_nested_runfiles_tree = true;
        self.events.push(Event::error_at(
            Some(self.action.get_owner().get_location().clone()),
            format!(
                "Runfiles must not contain runfiles tree artifacts: {}",
                runfiles_tree
            ),
        ));
    }

    fn prefix_conflict(&mut self, message: String) {
        let event_kind = match self.action.runfiles.get_conflict_policy() {
            ConflictPolicy::ERROR => EventKind::Error,
            ConflictPolicy::WARN => EventKind::Warning,
        };
        self.events.push(Event::of_at(
            event_kind,
            Some(self.action.get_owner().get_location().clone()),
            message,
        ));
    }
}

/// The runfiles mapping `newDeterministicWriter` computes, with the events Java replays on the
/// execution context's event handler.
pub struct ExecutionRunfilesInputs {
    pub runfiles_inputs: RunfilesMapping,
    /// Events to replay on `ctx.getEventHandler()`, in order.
    pub events: Vec<Event>,
    /// When true, Java throws `UserExecException` with [`INVALID_RUNFILES_TREE_MESSAGE`] after
    /// replaying the events.
    pub seen_nested_runfiles_tree: bool,
}

/// Creates a manifest file describing a symlink tree.
///
/// In addition to symlink trees (whose manifests are a tree position -> exec path map), this
/// action can also create manifest consisting of just exec paths for historical reasons.
///
/// This action carefully avoids building the manifest content in memory because it can be
/// large.
// Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:70 SourceManifestAction
#[derive(Debug)]
pub struct SourceManifestAction {
    base: AbstractAction,
    repo_mapping_manifest: Option<Artifact>,
    /// The strategy we use to write manifest entries.
    manifest_writer: Arc<dyn ManifestWriter>,
    /// The runfiles for which to create the symlink tree.
    runfiles: Runfiles,
    remotable_source_manifest_actions: bool,
    symlink_artifacts: OnceLock<NestedSet<Artifact>>,
}

impl SourceManifestAction {
    /// Creates a new AbstractSourceManifestAction instance using latin1 encoding to write the
    /// manifest file and with a specified root path for manifest entries.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:145 SourceManifestAction.<init>
    pub fn new_for_testing(
        manifest_writer: Arc<dyn ManifestWriter>,
        owner: ActionOwner,
        primary_output: Artifact,
        runfiles: Runfiles,
    ) -> SourceManifestAction {
        SourceManifestAction::new(
            manifest_writer,
            owner,
            primary_output,
            runfiles,
            None,
            false,
        )
    }

    /// Creates a new AbstractSourceManifestAction instance using latin1 encoding to write the
    /// manifest file and with a specified root path for manifest entries.
    ///
    /// - `manifest_writer`: the strategy to use to write manifest entries
    /// - `owner`: the action owner
    /// - `primary_output`: the file to which to write the manifest
    /// - `runfiles`: runfiles
    /// - `repo_mapping_manifest`: the repository mapping manifest for runfiles
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:160 SourceManifestAction.<init>
    pub fn new(
        manifest_writer: Arc<dyn ManifestWriter>,
        owner: ActionOwner,
        primary_output: Artifact,
        runfiles: Runfiles,
        repo_mapping_manifest: Option<Artifact>,
        remotable_source_manifest_actions: bool,
    ) -> SourceManifestAction {
        // The real set of inputs is computed in #getInputs().
        let base = new_abstract_file_write_action(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            primary_output,
        );
        SourceManifestAction {
            base,
            repo_mapping_manifest,
            manifest_writer,
            runfiles,
            remotable_source_manifest_actions,
            symlink_artifacts: OnceLock::new(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:210 SourceManifestAction.writeTo
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.write_file(
            out,
            &self
                .runfiles
                .get_runfiles_inputs(self.repo_mapping_manifest.as_ref()),
        )
    }

    /// `getStarlarkContent()`: the file contents.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:228 SourceManifestAction.getStarlarkContent
    pub fn get_starlark_content(&self) -> io::Result<String> {
        self.get_file_contents()
    }

    /// The part of `newDeterministicWriter(ctx)` that does not need the execution context: the
    /// runfiles mapping computed with the action's conflict receiver, the events Java replays on
    /// `ctx.getEventHandler()`, and whether a nested runfiles tree was seen (Java then throws a
    /// `UserExecException` with [`INVALID_RUNFILES_TREE_MESSAGE`]). The writer then calls
    /// [`SourceManifestAction::write_file`] with the mapping.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:233 SourceManifestAction.newDeterministicWriter
    pub fn compute_execution_runfiles_inputs(&self) -> ExecutionRunfilesInputs {
        let mut receiver = StoringConflictReceiver {
            action: self,
            events: Vec::new(),
            seen_nested_runfiles_tree: false,
        };
        let runfiles_inputs = self
            .runfiles
            .get_runfiles_inputs_with_receiver(&mut receiver, self.repo_mapping_manifest.as_ref());
        ExecutionRunfilesInputs {
            runfiles_inputs,
            events: receiver.events,
            seen_nested_runfiles_tree: receiver.seen_nested_runfiles_tree,
        }
    }

    /// Sort the entries in both the normal and root manifests and write the output file.
    ///
    /// PORT: Java takes an optional `InputMetadataProvider` to read the targets of unresolved
    /// symlinks during execution; without one (all analysis-time callers) it reads the symlink
    /// from the file system, which is what this does. The execution-time variant is W6.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:290 SourceManifestAction.writeFile
    pub fn write_file(&self, out: &mut dyn Write, output: &RunfilesMapping) -> io::Result<()> {
        let mut manifest_file = io::BufWriter::new(out);
        let mut sorted_manifest: Vec<(&PathFragment, &Option<Artifact>)> = output.iter().collect();
        // ENTRY_COMPARATOR: by path string.
        sorted_manifest.sort_by(|a, b| a.0.get_path_string().cmp(b.0.get_path_string()));
        for (path, artifact) in sorted_manifest {
            let symlink_target: Option<PathFragment> = match artifact {
                None => None,
                Some(artifact) if artifact.is_symlink() => {
                    Some(artifact.get_path().read_symbolic_link()?)
                }
                Some(artifact) => Some(artifact.get_path().as_fragment().clone()),
            };
            self.manifest_writer
                .write_entry(&mut manifest_file, path, symlink_target.as_ref())?;
        }

        manifest_file.flush()?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:258 SourceManifestAction.isRemotable
    pub fn is_remotable_impl(&self) -> bool {
        self.remotable_source_manifest_actions || self.manifest_writer.is_remotable()
    }
}

impl Action for SourceManifestAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:325 SourceManifestAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.manifest_writer.get_mnemonic()
    }

    /// The manifest entry for a symlink artifact should contain the target of the symlink rather
    /// than its exec path. Reading the symlink target requires that the symlink artifact is
    /// declared as an input of this action. Since declaring all runfiles as inputs of the
    /// manifest action would unnecessarily delay its execution, this action exceptionally
    /// overrides `getInputs()` and filters out the non-symlink runfiles by flattening the nested
    /// set of runfiles. Benchmarks confirmed that this does not regress performance.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:198 SourceManifestAction.getInputs
    fn get_inputs(&self) -> NestedSet<Artifact> {
        self.symlink_artifacts
            .get_or_init(|| {
                let symlinks: Vec<Artifact> = self
                    .runfiles
                    .get_artifacts()
                    .to_list()
                    .iter()
                    .filter(|a| a.is_symlink())
                    .cloned()
                    .collect();
                NestedSetBuilder::wrap(Order::STABLE_ORDER, symlinks)
            })
            .clone()
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:330 SourceManifestAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!(
            "{} for {}",
            self.manifest_writer.get_raw_progress_message(),
            match self.get_owner().get_label() {
                Some(label) => label.to_string(),
                None => "null".to_string(),
            }
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:333 SourceManifestAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(GUID);
        fp.add_boolean(self.remotable_source_manifest_actions);
        self.runfiles.fingerprint(
            action_key_context,
            fp,
            self.manifest_writer.emits_absolute_paths(),
        );
        fp.add_boolean(self.repo_mapping_manifest.is_some());
        if let Some(repo_mapping_manifest) = &self.repo_mapping_manifest {
            fp.add_path_fragment(repo_mapping_manifest.get_exec_path());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:347 SourceManifestAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "GUID: {}\nremotableSourceManifestActions: {}\nrunfiles: {}\n",
            GUID,
            self.remotable_source_manifest_actions,
            self.runfiles
                .describe_fingerprint(self.manifest_writer.emits_absolute_paths())
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        exec_properties()
    }

    fn as_file_contents_provider(&self) -> Option<&dyn FileContentsProvider> {
        Some(self)
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:228 SourceManifestAction.getStarlarkContent
    fn get_starlark_content(
        &self,
    ) -> Result<Option<String>, crate::actions::action::ActionApiError> {
        // Java: the IOException is reported to Starlark as `new EvalException(e)`.
        SourceManifestAction::get_starlark_content(self)
            .map(Some)
            .map_err(|e| crate::actions::action::ActionApiError::Eval(e.to_string()))
    }
}

impl AbstractFileWriteAction for SourceManifestAction {
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:233 SourceManifestAction.newDeterministicWriter
    // PORT: needs the ActionExecutionContext (event handler, input metadata provider): wave W6.
    // [`SourceManifestAction::compute_execution_runfiles_inputs`] holds the context-free part.
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        unported!("SourceManifestAction.newDeterministicWriter")
    }

    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:258 SourceManifestAction.isRemotable
    fn is_remotable(&self) -> bool {
        self.is_remotable_impl()
    }
}

impl FileContentsProvider for SourceManifestAction {
    /// Get the contents of a file internally using an in memory output stream.
    // Java: com/google/devtools/build/lib/analysis/SourceManifestAction.java:221 SourceManifestAction.getFileContents
    fn get_file_contents(&self) -> Result<String, io::Error> {
        let mut stream: Vec<u8> = Vec::new();
        self.write_to(&mut stream)?;
        // Java: `stream.toString(ISO_8859_1)`.
        Ok(blaze_util::string_encoding::latin1_decode(&stream).into_owned())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:55 AbstractFileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        AbstractFileWriteAction::make_executable(self)
    }
}
