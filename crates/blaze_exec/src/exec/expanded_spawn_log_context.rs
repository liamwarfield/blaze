//! Port of `lib/exec/ExpandedSpawnLogContext.java`.

use std::io::{self, BufWriter};
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::tools::protos::{File, SpawnExec};
use blaze_vfs::{DigestHashFunction, DirentType, FileSystemRef, Path, PathFragment, Symlinks};
use indexmap::IndexSet;

use super::io::{
    AsynchronousMessageOutputStream, BinaryInputStreamWrapper, BinaryOutputStreamWrapper,
    JsonOutputStreamWrapper, MessageInputStream, MessageOutputStream,
};
use super::spawn_log_context::{
    SpawnLogAction, SpawnLogContext, compute_digest, get_environment_variables, get_platform,
    get_spawn_metrics_proto, internal, is_input_directory, java_string_compare,
};
use super::stable_sort::stable_sort;
use crate::actions::{
    ActionInputRef, ExecException, InputMapping, InputMetadataProvider, Spawn, SpawnResult, Status,
    spawns,
};

/// The log encoding.
// Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:71 ExpandedSpawnLogContext.Encoding
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Length-delimited binary protos.
    Binary,
    /// Newline-delimited JSON messages.
    Json,
}

/// A [`SpawnLogContext`] implementation that produces a log in expanded format.
// Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:68 ExpandedSpawnLogContext
pub struct ExpandedSpawnLogContext {
    encoding: Encoding,
    sorted: bool,

    temp_path: Path,
    output_path: Path,

    exec_root: PathFragment,
    remote_options: Option<Arc<RemoteOptions>>,
    digest_hash_function: DigestHashFunction,

    /// Output stream to write directly into during execution.
    raw_output_stream: AsynchronousMessageOutputStream<SpawnExec>,
}

impl ExpandedSpawnLogContext {
    /// PORT: the `XattrProvider` parameter is dropped (see `compute_digest`).
    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:94 ExpandedSpawnLogContext.<init>
    pub fn new(
        output_path: Path,
        temp_path: Path,
        encoding: Encoding,
        sorted: bool,
        exec_root: PathFragment,
        remote_options: Option<Arc<RemoteOptions>>,
        digest_hash_function: DigestHashFunction,
    ) -> io::Result<ExpandedSpawnLogContext> {
        let raw_output_stream = if Self::needs_conversion_for(encoding, sorted) {
            // Write the unsorted binary format into a temporary path first, then convert into the
            // output format after execution. Delete a preexisting output file so that an
            // incomplete invocation doesn't appear to produce a nonsensical log.
            output_path.delete()?;
            Self::get_raw_output_stream(&temp_path)?
        } else {
            // The unsorted binary format can be written directly into the output path during
            // execution.
            Self::get_raw_output_stream(&output_path)?
        };
        Ok(ExpandedSpawnLogContext {
            encoding,
            sorted,
            temp_path,
            output_path,
            exec_root,
            remote_options,
            digest_hash_function,
            raw_output_stream,
        })
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:125 ExpandedSpawnLogContext.needsConversion
    fn needs_conversion(&self) -> bool {
        Self::needs_conversion_for(self.encoding, self.sorted)
    }

    fn needs_conversion_for(encoding: Encoding, sorted: bool) -> bool {
        encoding != Encoding::Binary || sorted
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:129 ExpandedSpawnLogContext.getRawOutputStream
    fn get_raw_output_stream(
        path: &Path,
    ) -> io::Result<AsynchronousMessageOutputStream<SpawnExec>> {
        // Use an AsynchronousMessageOutputStream so that writes occur in a separate thread.
        // This ensures concurrent writes don't tear and avoids blocking execution.
        AsynchronousMessageOutputStream::new_for_path(path)
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:135 ExpandedSpawnLogContext.getConvertedOutputStream
    fn get_converted_output_stream(
        &self,
        path: &Path,
    ) -> io::Result<Box<dyn MessageOutputStream<SpawnExec>>> {
        // PORT: buffered; Java writes through the file system's output stream directly.
        let stream = Box::new(BufWriter::new(path.get_output_stream()?));
        let out: Box<dyn MessageOutputStream<SpawnExec>> = match self.encoding {
            Encoding::Binary => Box::new(BinaryOutputStreamWrapper::<SpawnExec>::new(stream)),
            Encoding::Json => Box::new(JsonOutputStreamWrapper::<SpawnExec>::new(stream)),
        };
        Ok(out)
    }

    fn get_path(&self, file_system: &FileSystemRef, exec_path: &PathFragment) -> Path {
        Path::create(self.exec_root.get_relative(exec_path), file_system)
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:149 ExpandedSpawnLogContext.logSpawn
    fn log_spawn_impl(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        input_map: &dyn Fn() -> Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException> {
        let mut builder = SpawnExec {
            command_args: spawn.get_arguments().iter().map(|a| internal(a)).collect(),
            environment_variables: get_environment_variables(spawn),
            ..Default::default()
        };

        let tool_files: IndexSet<ActionInputRef> = spawn.get_tool_files().to_set();
        let tool_runfiles_directories: Vec<PathFragment> = tool_files
            .iter()
            .filter(|action_input| {
                action_input
                    .as_artifact()
                    .is_some_and(|artifact| artifact.is_runfiles_tree())
            })
            .map(|action_input| {
                input_metadata_provider
                    .get_runfiles_metadata(action_input)
                    .expect("NullPointerException: getRunfilesMetadata")
                    .get_runfiles_tree()
                    .get_exec_path()
            })
            .collect();

        let inputs_result = (|| -> io::Result<()> {
            for (display_path, input) in input_map().iter() {
                let display_path = &display_path.0;

                if input
                    .as_virtual_action_input()
                    .is_some_and(|v| v.is_empty_action_input())
                {
                    // Do not include a digest, as it's a waste of space.
                    builder.inputs.push(File {
                        path: internal(display_path.get_path_string()),
                        is_tool: tool_runfiles_directories
                            .iter()
                            .any(|d| display_path.starts_with(d)),
                        ..Default::default()
                    });
                    continue;
                }

                let is_tool = tool_files.contains(input)
                    || input.as_artifact().is_some_and(|artifact| {
                        artifact.is_child_of_declared_directory()
                            && artifact
                                .get_parent()
                                .is_some_and(|parent| tool_files.contains(&parent))
                    })
                    || tool_runfiles_directories
                        .iter()
                        .any(|d| display_path.starts_with(d));

                let content_path = self.get_path(file_system, input.get_exec_path());

                if is_input_directory(input, input_metadata_provider)? {
                    self.list_directory_contents(
                        display_path,
                        &content_path,
                        &mut builder.inputs,
                        Some(input_metadata_provider),
                        is_tool,
                    )?;
                    continue;
                }

                if input.is_symlink() {
                    let metadata = input_metadata_provider
                        .get_input_metadata(input)?
                        .expect("NullPointerException: getInputMetadata");
                    if !metadata.get_type().is_symlink() {
                        panic!("IllegalStateException: {metadata}");
                    }
                    builder.inputs.push(File {
                        path: internal(display_path.get_path_string()),
                        symlink_target_path: internal(
                            metadata
                                .get_unresolved_symlink_target()
                                .expect("NullPointerException: getUnresolvedSymlinkTarget"),
                        ),
                        is_tool,
                        ..Default::default()
                    });
                    continue;
                }

                let digest = compute_digest(
                    Some(input),
                    &content_path,
                    Some(input_metadata_provider),
                    self.digest_hash_function,
                    /* include_hash_function_name= */ true,
                )?;

                builder.inputs.push(File {
                    path: internal(display_path.get_path_string()),
                    digest: Some(digest),
                    is_tool,
                    ..Default::default()
                });
            }
            Ok(())
        })();
        // PORT: skipped, tracing (Java log) ("Error computing spawn input properties").
        let _ = inputs_result;

        let mut output_paths: Vec<String> = spawn
            .get_output_files()
            .iter()
            .map(|output| internal(&output.get_exec_path_string()))
            .collect();
        // `Collections.sort`: the internal strings in Java order.
        output_paths.sort_by(|a, b| java_string_compare(a, b));
        builder.listed_outputs.extend(output_paths);
        let outputs_result = (|| -> io::Result<()> {
            for output in spawn.get_output_files() {
                let path = self.get_path(file_system, output.get_exec_path());
                if !output.is_directory() && !output.is_symlink() && path.is_file() {
                    builder.actual_outputs.push(File {
                        path: internal(&output.get_exec_path_string()),
                        digest: Some(compute_digest(
                            Some(output),
                            &path,
                            Some(input_metadata_provider),
                            self.digest_hash_function,
                            /* include_hash_function_name= */ true,
                        )?),
                        ..Default::default()
                    });
                } else if output.is_directory() && path.is_directory() {
                    self.list_directory_contents(
                        output.get_exec_path(),
                        &path,
                        &mut builder.actual_outputs,
                        Some(input_metadata_provider),
                        /* is_tool= */ false,
                    )?;
                } else if output.is_symlink() && path.is_symbolic_link() {
                    builder.actual_outputs.push(File {
                        path: internal(&output.get_exec_path_string()),
                        symlink_target_path: internal(path.read_symbolic_link()?.get_path_string()),
                        ..Default::default()
                    });
                }
            }
            Ok(())
        })();
        // PORT: skipped, tracing (Java log) ("Error computing spawn output properties").
        let _ = outputs_result;
        builder.remotable = spawns::may_be_executed_remotely(spawn);

        let platform = get_platform(spawn, self.remote_options.as_deref())?;
        if let Some(platform) = platform {
            builder.platform = Some(platform);
        }
        if result.status() != Status::Success {
            builder.status = internal(result.status().name());
        }
        if !timeout.is_zero() {
            builder.timeout_millis = timeout.as_millis() as i64;
        }
        builder.cacheable = spawns::may_be_cached(spawn);
        builder.remote_cacheable = spawns::may_be_cached_remotely(spawn);
        builder.exit_code = result.exit_code();
        builder.cache_hit = result.is_cache_hit();
        builder.runner = internal(result.get_runner_name());

        if let Some(digest) = result.get_digest() {
            builder.digest = Some(digest.clone());
        }

        builder.mnemonic = internal(&spawn.get_mnemonic());

        if let Some(target_label) = spawn.get_target_label() {
            builder.target_label = internal(&target_label.to_string());
        }

        builder.metrics = Some(get_spawn_metrics_proto(result));

        self.raw_output_stream.write(&builder);
        Ok(())
    }

    /// Expands a directory into its contents.
    ///
    /// Note the difference between `display_path` and `content_path`: the first is where the
    /// spawn can find the directory, while the second is where Bazel can find it. They're not
    /// the same for a directory appearing in a runfiles or fileset tree.
    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:348 ExpandedSpawnLogContext.listDirectoryContents
    fn list_directory_contents(
        &self,
        display_path: &PathFragment,
        content_path: &Path,
        add_file: &mut Vec<File>,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        is_tool: bool,
    ) -> io::Result<()> {
        let mut sorted_dirent = content_path.readdir(Symlinks::Nofollow)?;
        // `Comparator.comparing(Dirent::getName)`: names are internal strings, whose Java order is
        // the byte order of their UTF-8 form.
        sorted_dirent.sort_by(|a, b| a.get_name().as_bytes().cmp(b.get_name().as_bytes()));

        for dirent in sorted_dirent {
            let name = dirent.get_name();
            let child_display_path = display_path.get_child(name);
            let child_content_path = content_path.get_child(name);

            if dirent.get_type() == DirentType::Directory {
                self.list_directory_contents(
                    &child_display_path,
                    &child_content_path,
                    add_file,
                    input_metadata_provider,
                    is_tool,
                )?;
                continue;
            }

            add_file.push(File {
                path: internal(child_display_path.get_path_string()),
                digest: Some(compute_digest(
                    None,
                    &child_content_path,
                    input_metadata_provider,
                    self.digest_hash_function,
                    /* include_hash_function_name= */ true,
                )?),
                is_tool,
                ..Default::default()
            });
        }
        Ok(())
    }
}

impl SpawnLogContext for ExpandedSpawnLogContext {
    fn log_spawn(
        &self,
        spawn: &dyn Spawn,
        input_metadata_provider: &dyn InputMetadataProvider,
        input_map: &dyn Fn() -> Arc<InputMapping>,
        file_system: &FileSystemRef,
        timeout: Duration,
        result: &SpawnResult,
    ) -> Result<(), ExecException> {
        self.log_spawn_impl(
            spawn,
            input_metadata_provider,
            input_map,
            file_system,
            timeout,
            result,
        )
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:307 ExpandedSpawnLogContext.logSymlinkAction
    fn log_symlink_action(&self, _action: &dyn SpawnLogAction) -> io::Result<()> {
        // The expanded log does not report symlink actions.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:312 ExpandedSpawnLogContext.close
    fn close(&self) -> io::Result<()> {
        self.raw_output_stream.close()?;

        if !self.needs_conversion() {
            return Ok(());
        }

        let result = (|| -> io::Result<()> {
            let mut raw_input_stream =
                BinaryInputStreamWrapper::<SpawnExec>::new(self.temp_path.get_input_stream()?);
            let mut converted_output_stream =
                match self.get_converted_output_stream(&self.output_path) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = raw_input_stream.close();
                        return Err(e);
                    }
                };
            let body = (|| -> io::Result<()> {
                if self.sorted {
                    stable_sort(&mut raw_input_stream, &mut *converted_output_stream)
                } else {
                    while let Some(ex) = raw_input_stream.read()? {
                        converted_output_stream.write(&ex)?;
                    }
                    Ok(())
                }
            })();
            // try-with-resources closes in reverse order; a close failure is suppressed by an
            // earlier failure.
            let close_output = converted_output_stream.close();
            let close_input = raw_input_stream.close();
            body?;
            close_output?;
            close_input
        })();
        // Intentionally ignored.
        let _ = self.temp_path.delete();
        result
    }

    // Java: com/google/devtools/build/lib/exec/ExpandedSpawnLogContext.java:143 ExpandedSpawnLogContext.shouldPublish
    fn should_publish(&self) -> bool {
        // The expanded log tends to be too large to be uploaded to a remote store.
        false
    }
}
