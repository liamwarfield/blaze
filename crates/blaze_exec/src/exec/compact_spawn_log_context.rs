//! Port of `lib/exec/CompactSpawnLogContext.java`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, BufWriter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_collect::{NestedSet, NestedSetBuilder, NestedSetNode, Order};
use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::tools::protos::{ExecLogEntry, exec_log_entry};
use blaze_vfs::{DigestHashFunction, DirentType, FileSystemRef, Path, PathFragment, Symlinks};
use parking_lot::ReentrantMutex;

use super::bin_tools::PathActionInput;
use super::io::{AsynchronousMessageOutputStream, ZstdOutputStream};
use super::spawn_log_context::{
    SpawnLogAction, SpawnLogContext, compute_digest, get_environment_variables, get_platform,
    get_spawn_metrics_proto, internal, is_input_directory, java_string_compare,
    spawn_log_io_exception,
};
use crate::actions::{
    ActionInputRef, ExecException, InputMapping, InputMetadataProvider, RunfilesTree, Spawn,
    SpawnResult, Status, spawns,
};

/// A runfiles symlink: its runfiles path and target artifact.
///
/// PORT: seam gap. Java's `RunfilesTree.getSymlinksForLogging()` and
/// `getRootSymlinksForLogging()` return a `NestedSet<SymlinkEntry>` whose node structure the
/// compact log mirrors (one `SymlinkEntrySet` per node, deduplicated by node). The seam's
/// `RunfilesTree` returns a flat list, which is logged here as a single-node set.
// Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java SymlinkEntry
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymlinkEntry {
    /// `getPathString()`.
    pub path: String,
    /// `getArtifact()`.
    pub artifact: ActionInputRef,
}

/// A key identifying an entry: a `NestedSet.Node` or the String path of a file, directory,
/// symlink or runfiles tree.
#[derive(Clone, PartialEq, Eq, Hash)]
enum EntryKey {
    InputSetNode(NestedSetNode<ActionInputRef>),
    SymlinkEntrySetNode(NestedSetNode<SymlinkEntry>),
    Path(String),
}

/// The `@GuardedBy("this")` state.
struct State {
    // Maps a key identifying an entry into its ID.
    // Only entries that are likely to be referenced by future entries are stored.
    entry_map: HashMap<EntryKey, i32>,
    // The next available entry ID.
    next_entry_id: i32,
}

/// A [`SpawnLogContext`] implementation that produces a log in compact format.
// Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:81 CompactSpawnLogContext
pub struct CompactSpawnLogContext {
    exec_root: PathFragment,
    workspace_name: String,
    sibling_repository_layout: bool,
    remote_options: Option<Arc<RemoteOptions>>,
    digest_hash_function: DigestHashFunction,
    invocation_id: String,
    reporter: Arc<dyn EventHandler>,
    output_logging_failed: AtomicBool,

    // Java's `synchronized` methods are reentrant: computing an entry logs the entries it
    // references while holding the lock.
    state: ReentrantMutex<RefCell<State>>,

    // Output stream to write to.
    output_stream: AsynchronousMessageOutputStream<ExecLogEntry>,
}

/// `EXEC_LOG_ENTRY_FILE_COMPARATOR`.
// Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:85 CompactSpawnLogContext.EXEC_LOG_ENTRY_FILE_COMPARATOR
fn sort_files(files: &mut [exec_log_entry::File]) {
    files.sort_by(|a, b| java_string_compare(&a.path, &b.path));
}

fn entry(t: exec_log_entry::Type) -> ExecLogEntry {
    ExecLogEntry {
        id: 0,
        r#type: Some(t),
    }
}

/// Whether `input` is a `CommandLines.ParamFileActionInput`.
///
/// PORT: seam gap. `ParamFileActionInput` lives in `blaze_actions` and the seam's
/// `VirtualActionInput` cannot name it. The virtual inputs reaching the execution log are
/// `EmptyActionInput` (never logged as a file), `BinTools.PathActionInput` and
/// `ParamFileActionInput`; every other virtual input is treated as a parameter file.
fn is_param_file_action_input(input: &ActionInputRef) -> bool {
    match input.as_virtual_action_input() {
        None => false,
        Some(v) => {
            !v.is_empty_action_input() && input.as_any().downcast_ref::<PathActionInput>().is_none()
        }
    }
}

impl CompactSpawnLogContext {
    /// PORT: the `XattrProvider` parameter is dropped (see `compute_digest`); `invocation_id` is
    /// the `UUID.toString()` form.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:177 CompactSpawnLogContext.<init>
    pub fn new(
        output_path: &Path,
        exec_root: PathFragment,
        workspace_name: String,
        sibling_repository_layout: bool,
        remote_options: Option<Arc<RemoteOptions>>,
        digest_hash_function: DigestHashFunction,
        invocation_id: String,
        reporter: Arc<dyn EventHandler>,
    ) -> io::Result<CompactSpawnLogContext> {
        let output_stream = Self::get_output_stream(output_path)?;
        let context = CompactSpawnLogContext {
            exec_root,
            workspace_name,
            sibling_repository_layout,
            remote_options,
            digest_hash_function,
            invocation_id,
            reporter,
            output_logging_failed: AtomicBool::new(false),
            state: ReentrantMutex::new(RefCell::new(State {
                entry_map: HashMap::new(),
                next_entry_id: 1,
            })),
            output_stream,
        };

        context.log_invocation()?;
        Ok(context)
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:201 CompactSpawnLogContext.getOutputStream
    fn get_output_stream(path: &Path) -> io::Result<AsynchronousMessageOutputStream<ExecLogEntry>> {
        // Use an AsynchronousMessageOutputStream so that compression and I/O occur in a separate
        // thread. This ensures concurrent writes don't tear and avoids blocking execution.
        Ok(AsynchronousMessageOutputStream::new(
            path.get_path_string(),
            Box::new(ZstdOutputStream::new(BufWriter::new(
                path.get_output_stream()?,
            ))?),
        ))
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:208 CompactSpawnLogContext.logInvocation
    fn log_invocation(&self) -> io::Result<()> {
        self.log_entry_without_id(&mut || {
            // PORT: StringEncoding identity (internalToUnicode).
            Ok(entry(exec_log_entry::Type::Invocation(
                exec_log_entry::Invocation {
                    hash_function_name: self.digest_hash_function.to_string(),
                    workspace_runfiles_directory: self.workspace_name.clone(),
                    sibling_repository_layout: self.sibling_repository_layout,
                    id: self.invocation_id.clone(),
                },
            )))
        })
    }

    fn get_path(&self, file_system: &FileSystemRef, exec_path: &PathFragment) -> Path {
        Path::create(self.exec_root.get_relative(exec_path), file_system)
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:227 CompactSpawnLogContext.logSpawn
    fn log_spawn_impl(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException> {
        // PORT: StringEncoding identity (internalToUnicode) for every string below.
        let mut builder = exec_log_entry::Spawn {
            args: spawn.get_arguments().to_vec(),
            env_vars: get_environment_variables(spawn),
            ..Default::default()
        };
        let platform = get_platform(spawn, self.remote_options.as_deref())?;
        if let Some(platform) = platform {
            builder.platform = Some(platform);
        }

        builder.input_set_id = self
            .log_inputs(spawn, input_metadata_provider, file_system)
            .map_err(|e| spawn_log_io_exception(&e))? as u32;
        builder.tool_set_id = self
            .log_tools(spawn, input_metadata_provider, file_system)
            .map_err(|e| spawn_log_io_exception(&e))? as u32;

        if let Some(target_label) = spawn.get_target_label() {
            builder.target_label = target_label.get_canonical_form();
        }
        builder.mnemonic = spawn.get_mnemonic();

        let mut warned = false;
        for output in spawn.get_output_files() {
            let path = self.get_path(file_system, output.get_exec_path());
            let output_type = (|| -> io::Result<exec_log_entry::output::Type> {
                Ok(
                    if !output.is_directory() && !output.is_symlink() && path.is_file() {
                        exec_log_entry::output::Type::OutputId(
                            self.log_file(output, &path, /* input_metadata_provider= */ None)?
                                as u32,
                        )
                    } else if output.is_directory() && path.is_directory() {
                        exec_log_entry::output::Type::OutputId(self.log_directory(
                            output, &path, /* input_metadata_provider= */ None,
                        )? as u32)
                    } else if output.is_symlink() && path.is_symbolic_link() {
                        exec_log_entry::output::Type::OutputId(self.log_unresolved_symlink(
                            output, &path, /* input_metadata_provider= */ None,
                        )? as u32)
                    } else {
                        exec_log_entry::output::Type::InvalidOutputPath(
                            output.get_exec_path_string(),
                        )
                    },
                )
            })();
            let output_type = match output_type {
                Ok(t) => t,
                Err(_e) => {
                    if !warned {
                        self.output_logging_failed.store(true, Ordering::SeqCst);
                        warned = true;
                        // PORT: logging dropped ("Failed to log outputs of spawn with mnemonic
                        // %s and primary output %s").
                    }
                    exec_log_entry::output::Type::InvalidOutputPath(output.get_exec_path_string())
                }
            };
            builder.outputs.push(exec_log_entry::Output {
                r#type: Some(output_type),
            });
        }

        builder.exit_code = result.exit_code();
        if result.status() != Status::Success {
            builder.status = result.status().name().to_string();
        }
        builder.runner = result.get_runner_name().to_string();
        builder.cache_hit = result.is_cache_hit();
        builder.remotable = spawns::may_be_executed_remotely(spawn);
        builder.cacheable = spawns::may_be_cached(spawn);
        builder.remote_cacheable = spawns::may_be_cached_remotely(spawn);

        if let Some(digest) = result.get_digest() {
            let mut digest = digest.clone();
            digest.hash_function_name = String::new();
            builder.digest = Some(digest);
        }

        builder.timeout_millis = timeout.as_millis() as i64;
        builder.metrics = Some(get_spawn_metrics_proto(result));

        let mut spawn_entry = Some(builder);
        self.log_entry_without_id(&mut || {
            Ok(entry(exec_log_entry::Type::Spawn(
                spawn_entry.take().expect("spawn entry logged twice"),
            )))
        })
        .map_err(|e| spawn_log_io_exception(&e))
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:305 CompactSpawnLogContext.logSymlinkAction
    fn log_symlink_action_impl(&self, action: &dyn SpawnLogAction) -> io::Result<()> {
        let Some(input) = action.get_primary_input() else {
            // Symlinks to absolute paths are only used by FDO and not worth logging as they can
            // be treated just like source files.
            return Ok(());
        };
        // PORT: StringEncoding identity (internalToUnicode).
        let mut builder = exec_log_entry::SymlinkAction {
            input_path: input.get_exec_path_string(),
            output_path: action.get_primary_output().get_exec_path_string(),
            ..Default::default()
        };

        if let Some(label) = action.get_owner_label() {
            builder.target_label = label.get_canonical_form();
        }
        builder.mnemonic = action.get_mnemonic();

        let mut symlink_entry = Some(builder);
        self.log_entry_without_id(&mut || {
            Ok(entry(exec_log_entry::Type::SymlinkAction(
                symlink_entry.take().expect("symlink action logged twice"),
            )))
        })
    }

    /// Logs the inputs.
    ///
    /// Returns the entry ID of the `ExecLogEntry.InputSet` describing the inputs, or 0 if there
    /// are no inputs.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:334 CompactSpawnLogContext.logInputs
    fn log_inputs(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
    ) -> io::Result<i32> {
        self.log_input_set(
            spawn.get_input_files(),
            input_metadata_provider,
            file_system,
            /* shared= */ false,
            spawn.get_mnemonic() == "TestRunner",
        )
    }

    /// Logs the tool inputs.
    ///
    /// Returns the entry ID of the `ExecLogEntry.InputSet` describing the tool inputs, or 0 if
    /// there are no tool inputs.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:352 CompactSpawnLogContext.logTools
    fn log_tools(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
    ) -> io::Result<i32> {
        self.log_input_set(
            spawn.get_tool_files(),
            input_metadata_provider,
            file_system,
            /* shared= */ true,
            spawn.get_mnemonic() == "TestRunner",
        )
    }

    /// Logs a nested set.
    ///
    /// - `shared`: whether this nested set is likely to be a transitive member of other sets
    /// - `is_test_runner_spawn`: whether this nested set is logged for a test runner spawn
    ///
    /// Returns the entry ID of the `ExecLogEntry.InputSet` describing the nested set, or 0 if the
    /// nested set is empty.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:372 CompactSpawnLogContext.logInputSet
    fn log_input_set(
        &self,
        set: &NestedSet<ActionInputRef>,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
        shared: bool,
        is_test_runner_spawn: bool,
    ) -> io::Result<i32> {
        if set.is_empty() {
            return Ok(0);
        }

        self.log_entry(
            if shared {
                Some(EntryKey::InputSetNode(set.to_node()))
            } else {
                None
            },
            &mut || {
                let mut builder = exec_log_entry::InputSet::default();

                for transitive in set.get_non_leaves() {
                    blaze_util::check_state!(!transitive.is_empty());
                    builder.transitive_set_ids.push(self.log_input_set(
                        &transitive,
                        input_metadata_provider,
                        file_system,
                        /* shared= */ true,
                        is_test_runner_spawn,
                    )? as u32);
                }

                for input in set.get_leaves() {
                    if input
                        .as_artifact()
                        .is_some_and(|artifact| artifact.is_runfiles_tree())
                    {
                        let runfiles_tree = input_metadata_provider
                            .get_runfiles_metadata(&input)
                            .expect("NullPointerException: getRunfilesMetadata")
                            .get_runfiles_tree()
                            .clone();
                        builder.input_ids.push(self.log_runfiles_tree(
                            &runfiles_tree,
                            input_metadata_provider,
                            file_system,
                            // Runfiles of non-test spawns are tool inputs and thus potentially
                            // reused between spawns. Runfiles of test spawns are reused if the
                            // test is attempted multiple times in the same build; in this case,
                            // the runfiles tree caches its mapping.
                            !is_test_runner_spawn || runfiles_tree.is_mapping_cached(),
                        )? as u32);
                        continue;
                    }

                    if input
                        .as_artifact()
                        .is_some_and(|artifact| artifact.is_fileset())
                    {
                        // The fileset symlink tree is always materialized on disk.
                        builder.input_ids.push(self.log_directory(
                            &input,
                            &self.get_path(file_system, input.get_exec_path()),
                            Some(input_metadata_provider),
                        )? as u32);
                    }

                    builder.input_ids.push(self.log_input(
                        &input,
                        input_metadata_provider,
                        file_system,
                    )? as u32);
                }

                Ok(entry(exec_log_entry::Type::InputSet(builder)))
            },
        )
    }

    /// Logs a nested set of [`SymlinkEntry`].
    ///
    /// Returns the entry ID of the `ExecLogEntry.SymlinkEntrySet` describing the nested set, or 0
    /// if the nested set is empty.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:438 CompactSpawnLogContext.logSymlinkEntries
    fn log_symlink_entries(
        &self,
        symlinks: &NestedSet<SymlinkEntry>,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
    ) -> io::Result<i32> {
        if symlinks.is_empty() {
            return Ok(0);
        }

        self.log_entry(
            Some(EntryKey::SymlinkEntrySetNode(symlinks.to_node())),
            &mut || {
                let mut builder = exec_log_entry::SymlinkEntrySet::default();

                for transitive in symlinks.get_non_leaves() {
                    blaze_util::check_state!(!transitive.is_empty());
                    builder.transitive_set_ids.push(self.log_symlink_entries(
                        &transitive,
                        input_metadata_provider,
                        file_system,
                    )? as u32);
                }

                for input in symlinks.get_leaves() {
                    // PORT: StringEncoding identity (internalToUnicode).
                    let key = input.path.clone();
                    let value =
                        self.log_input(&input.artifact, input_metadata_provider, file_system)?
                            as u32;
                    // `putDirectEntries`: a map put keeps the position of an existing key.
                    match builder.direct_entries.iter_mut().find(|e| e.key == key) {
                        Some(existing) => existing.value = value,
                        None => builder.direct_entries.push(
                            exec_log_entry::symlink_entry_set::DirectEntriesEntry { key, value },
                        ),
                    }
                }

                Ok(entry(exec_log_entry::Type::SymlinkEntrySet(builder)))
            },
        )
    }

    /// Logs a single input that is either a file, a directory or a symlink.
    ///
    /// Returns the entry ID of the `ExecLogEntry` describing the input.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:473 CompactSpawnLogContext.logInput
    fn log_input(
        &self,
        input: &ActionInputRef,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
    ) -> io::Result<i32> {
        let path = self.get_path(file_system, input.get_exec_path());
        if is_input_directory(input, input_metadata_provider)? {
            self.log_directory(input, &path, Some(input_metadata_provider))
        } else if input.is_symlink() {
            self.log_unresolved_symlink(input, &path, Some(input_metadata_provider))
        } else {
            self.log_file(input, &path, Some(input_metadata_provider))
        }
    }

    /// Logs a file.
    ///
    /// - `input`: the input representing the file.
    /// - `path`: the path to the file, which must have already been verified to be of the
    ///   correct type.
    /// - `input_metadata_provider`: provides metadata for inputs; `None` if logging an output
    ///
    /// Returns the entry ID of the `ExecLogEntry.File` describing the file.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:495 CompactSpawnLogContext.logFile
    fn log_file(
        &self,
        input: &ActionInputRef,
        path: &Path,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> io::Result<i32> {
        blaze_util::check_state!(
            !input
                .as_virtual_action_input()
                .is_some_and(|v| v.is_empty_action_input())
        );

        self.log_entry(
            // A ParamFileActionInput is never shared between spawns.
            if is_param_file_action_input(input) {
                None
            } else {
                Some(EntryKey::Path(input.get_exec_path_string()))
            },
            &mut || {
                let digest = compute_digest(
                    Some(input),
                    path,
                    input_metadata_provider,
                    self.digest_hash_function,
                    /* include_hash_function_name= */ false,
                )?;

                // PORT: StringEncoding identity (internalToUnicode).
                Ok(entry(exec_log_entry::Type::File(exec_log_entry::File {
                    path: input.get_exec_path_string(),
                    digest: Some(digest),
                })))
            },
        )
    }

    /// Logs a directory.
    ///
    /// This may be either a source directory, a fileset or an output directory. For runfiles,
    /// [`Self::log_runfiles_tree`] must be used instead.
    ///
    /// Returns the entry ID of the `ExecLogEntry.Directory` describing the directory.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:535 CompactSpawnLogContext.logDirectory
    fn log_directory(
        &self,
        input: &ActionInputRef,
        root: &Path,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> io::Result<i32> {
        self.log_entry(
            Some(EntryKey::Path(input.get_exec_path_string())),
            &mut || {
                // PORT: StringEncoding identity (internalToUnicode).
                Ok(entry(exec_log_entry::Type::Directory(
                    exec_log_entry::Directory {
                        path: input.get_exec_path_string(),
                        files: self.expand_directory(input, root, input_metadata_provider)?,
                    },
                )))
            },
        )
    }

    /// Logs a runfiles directory by storing the information in its `RunfilesTree`.
    ///
    /// Since runfiles trees can be very large and, for tests, are only used by a single spawn,
    /// we store them in the log as a special entry that references the nested set of artifacts
    /// instead of as a flat directory.
    ///
    /// - `shared`: whether this runfiles tree is likely to be contained in more than one Spawn's
    ///   inputs
    ///
    /// Returns the entry ID of the `ExecLogEntry.RunfilesTree` describing the directory.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:560 CompactSpawnLogContext.logRunfilesTree
    fn log_runfiles_tree(
        &self,
        runfiles_tree: &Arc<dyn RunfilesTree>,
        input_metadata_provider: &dyn InputMetadataProvider,
        file_system: &FileSystemRef,
        shared: bool,
    ) -> io::Result<i32> {
        self.log_entry(
            if shared {
                Some(EntryKey::Path(
                    runfiles_tree.get_exec_path().get_path_string().to_string(),
                ))
            } else {
                None
            },
            &mut || {
                blaze_util::check_state!(self.workspace_name == runfiles_tree.get_workspace_name());

                // PORT: StringEncoding identity (internalToUnicode).
                let mut builder = exec_log_entry::RunfilesTree {
                    path: runfiles_tree.get_exec_path().get_path_string().to_string(),
                    ..Default::default()
                };

                builder.input_set_id = self.log_input_set(
                    &runfiles_tree.get_artifacts_at_canonical_locations_for_logging(),
                    input_metadata_provider,
                    file_system,
                    // The runfiles tree itself is shared, but the nested set is unique to the tree
                    // as it contains the executable.
                    /* shared= */
                    false,
                    // This value only matters for nested sets that may contain runfiles trees, but
                    // these are never nested.
                    /* is_test_runner_spawn= */
                    false,
                )? as u32;
                builder.symlinks_id = self.log_symlink_entries(
                    &symlink_entries(runfiles_tree.get_symlinks_for_logging()),
                    input_metadata_provider,
                    file_system,
                )? as u32;
                builder.root_symlinks_id = self.log_symlink_entries(
                    &symlink_entries(runfiles_tree.get_root_symlinks_for_logging()),
                    input_metadata_provider,
                    file_system,
                )? as u32;
                // `PathFragment::getPathString` without internalToUnicode: internal strings.
                builder.empty_files.extend(
                    runfiles_tree
                        .get_empty_filenames_for_logging()
                        .iter()
                        .map(|p| internal(p.get_path_string())),
                );
                if let Some(repo_mapping_manifest) =
                    runfiles_tree.get_repo_mapping_manifest_for_logging()
                {
                    // PORT: `Artifact.getPath()` is the artifact's path in the execution root.
                    let manifest_path =
                        self.get_path(file_system, repo_mapping_manifest.get_exec_path());
                    builder.repo_mapping_manifest = Some(exec_log_entry::File {
                        digest: Some(compute_digest(
                            Some(&repo_mapping_manifest),
                            &manifest_path,
                            Some(input_metadata_provider),
                            self.digest_hash_function,
                            /* include_hash_function_name= */ false,
                        )?),
                        ..Default::default()
                    });
                }

                Ok(entry(exec_log_entry::Type::RunfilesTree(builder)))
            },
        )
    }

    /// Expands a directory.
    ///
    /// Returns the list of files transitively contained in the directory.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:621 CompactSpawnLogContext.expandDirectory
    fn expand_directory(
        &self,
        input: &ActionInputRef,
        root: &Path,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> io::Result<Vec<exec_log_entry::File>> {
        if let Some(input_metadata_provider) = input_metadata_provider
            && input
                .as_artifact()
                .is_some_and(|artifact| artifact.is_tree_artifact())
            && let Some(tree_metadata) = input_metadata_provider.get_tree_metadata(input)
        {
            // Using the metadata over a filesystem traversal is not just an optimization: an
            // empty tree artifact may not be materialized on disk.
            return self.expand_tree_artifact(&tree_metadata, root, input_metadata_provider);
        }
        self.expand_directory_from_file_system(root, input_metadata_provider)
    }

    /// Expands a tree artifact into its contents as recorded in its metadata.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:638 CompactSpawnLogContext.expandTreeArtifact
    fn expand_tree_artifact(
        &self,
        tree_metadata: &crate::actions::TreeArtifactValue,
        root: &Path,
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> io::Result<Vec<exec_log_entry::File>> {
        let children = tree_metadata.get_children();
        let mut files = Vec::with_capacity(children.len());
        for child in &children {
            let parent_relative_path = child
                .as_artifact()
                .and_then(|artifact| artifact.get_parent_relative_path())
                .expect("TreeFileArtifact.getParentRelativePath");
            let digest = compute_digest(
                Some(child),
                &root.get_relative(&parent_relative_path),
                Some(input_metadata_provider),
                self.digest_hash_function,
                /* include_hash_function_name= */ false,
            )?;
            // PORT: StringEncoding identity (internalToUnicode).
            files.push(exec_log_entry::File {
                path: parent_relative_path.get_path_string().to_string(),
                digest: Some(digest),
            });
        }
        sort_files(&mut files);
        Ok(files)
    }

    /// Expands a directory by traversing it on the filesystem.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:663 CompactSpawnLogContext.expandDirectoryFromFileSystem
    fn expand_directory_from_file_system(
        &self,
        root: &Path,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> io::Result<Vec<exec_log_entry::File>> {
        let mut files = Vec::new();
        visit_directory(root, &mut |child: &Path| {
            let digest = compute_digest(
                /* input= */ None,
                child,
                input_metadata_provider,
                self.digest_hash_function,
                /* include_hash_function_name= */ false,
            )?;

            // PORT: StringEncoding identity (internalToUnicode).
            files.push(exec_log_entry::File {
                path: child.relative_to(root).get_path_string().to_string(),
                digest: Some(digest),
            });
            Ok(())
        })?;

        sort_files(&mut files);

        Ok(files)
    }

    /// Logs an unresolved symlink.
    ///
    /// Returns the entry ID of the `ExecLogEntry.UnresolvedSymlink` describing the unresolved
    /// symlink.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:705 CompactSpawnLogContext.logUnresolvedSymlink
    fn log_unresolved_symlink(
        &self,
        input: &ActionInputRef,
        path: &Path,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> io::Result<i32> {
        self.log_entry(
            Some(EntryKey::Path(input.get_exec_path_string())),
            &mut || {
                let metadata = match input_metadata_provider {
                    Some(input_metadata_provider) => {
                        input_metadata_provider.get_input_metadata(input)?
                    }
                    None => None,
                };
                let target_path = match metadata {
                    Some(metadata) => {
                        if !metadata.get_type().is_symlink() {
                            panic!("IllegalStateException: {metadata}");
                        }
                        metadata
                            .get_unresolved_symlink_target()
                            .expect("NullPointerException: getUnresolvedSymlinkTarget")
                            .to_string()
                    }
                    None => path.read_symbolic_link()?.get_path_string().to_string(),
                };

                // PORT: StringEncoding identity (internalToUnicode).
                Ok(entry(exec_log_entry::Type::UnresolvedSymlink(
                    exec_log_entry::UnresolvedSymlink {
                        path: input.get_exec_path_string(),
                        target_path,
                    },
                )))
            },
        )
    }

    /// Ensures an entry is written to the log without an ID.
    ///
    /// `supplier` is called to compute the entry; it may cause other entries to be logged.
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:736 CompactSpawnLogContext.logEntryWithoutId
    fn log_entry_without_id(
        &self,
        supplier: &mut dyn FnMut() -> io::Result<ExecLogEntry>,
    ) -> io::Result<()> {
        // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:743 CompactSpawnLogContext.logEntryWithoutIdSynchronized
        let _guard = self.state.lock();
        self.output_stream.write(&supplier()?);
        Ok(())
    }

    /// Ensures an entry is written to the log and returns its assigned ID.
    ///
    /// If an entry with the same non-null key was previously added to the log, its recorded ID
    /// is returned. Otherwise, the entry is computed, assigned an ID, and written to the log.
    ///
    /// - `key`: the key, or `None` if the ID shouldn't be recorded
    /// - `supplier`: called to compute the entry; may cause other entries to be logged
    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:762 CompactSpawnLogContext.logEntry
    fn log_entry(
        &self,
        key: Option<EntryKey>,
        supplier: &mut dyn FnMut() -> io::Result<ExecLogEntry>,
    ) -> io::Result<i32> {
        // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:769 CompactSpawnLogContext.logEntrySynchronized
        let guard = self.state.lock();
        let Some(key) = key else {
            // No need to check for a previously added entry.
            let mut entry = supplier()?;
            let id = {
                let mut state = guard.borrow_mut();
                let id = state.next_entry_id;
                state.next_entry_id += 1;
                id
            };
            entry.id = id as u32;
            self.output_stream.write(&entry);
            return Ok(id);
        };

        // Check for a previously added entry.
        let id = guard.borrow().entry_map.get(&key).copied().unwrap_or(0);
        if id != 0 {
            return Ok(id);
        }

        // Compute a fresh entry and log it.
        // The following order of operations is crucial to ensure that this entry is preceded by
        // any entries it references, which in turn ensures the log can be parsed in a single
        // pass.
        let mut entry = supplier()?;
        let id = {
            let mut state = guard.borrow_mut();
            let id = state.next_entry_id;
            state.next_entry_id += 1;
            state.entry_map.insert(key, id);
            id
        };
        entry.id = id as u32;
        self.output_stream.write(&entry);
        Ok(id)
    }
}

/// Converts the seam's flat symlink list into a single-node set (see [`SymlinkEntry`]).
fn symlink_entries(entries: Vec<(String, ActionInputRef)>) -> NestedSet<SymlinkEntry> {
    NestedSetBuilder::wrap(
        Order::STABLE_ORDER,
        entries
            .into_iter()
            .map(|(path, artifact)| SymlinkEntry { path, artifact }),
    )
}

/// Visits a directory hierarchy.
///
/// Calls `child_visitor` for every descendant path of `root_dir` that isn't itself a directory,
/// following symlinks.
///
/// PORT: Java visits subdirectories in parallel on a shared `ForkJoinPool` (failing fast on the
/// first error); callers sort the results, so a sequential traversal is equivalent.
// Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:143 CompactSpawnLogContext.visitDirectory
fn visit_directory(
    root_dir: &Path,
    child_visitor: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    visit_subdirectory(root_dir, child_visitor)
}

// Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:120 DirectoryVisitor.visitSubdirectory
fn visit_subdirectory(
    dir: &Path,
    child_visitor: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    for dirent in dir.readdir(Symlinks::Follow)? {
        let child = dir.get_child(dirent.get_name());
        if dirent.get_type() == DirentType::Directory {
            visit_subdirectory(&child, child_visitor)?;
            continue;
        }
        child_visitor(&child)?;
    }
    Ok(())
}

impl SpawnLogContext for CompactSpawnLogContext {
    fn log_spawn(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        _input_map: &dyn Fn() -> Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException> {
        self.log_spawn_impl(spawn, input_metadata_provider, file_system, timeout, result)
    }

    fn log_symlink_action(&self, action: &dyn SpawnLogAction) -> io::Result<()> {
        self.log_symlink_action_impl(action)
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:800 CompactSpawnLogContext.close
    fn close(&self) -> io::Result<()> {
        if self.output_logging_failed.load(Ordering::SeqCst) {
            self.reporter.handle(&Event::warn(
                "The compact execution log is incomplete because some outputs could not be read. \
                 Refer to the server log file for details.",
            ));
        }
        self.output_stream.close()
    }

    // Java: com/google/devtools/build/lib/exec/CompactSpawnLogContext.java:221 CompactSpawnLogContext.shouldPublish
    fn should_publish(&self) -> bool {
        // The compact log is small enough to be uploaded to a remote store.
        true
    }
}
