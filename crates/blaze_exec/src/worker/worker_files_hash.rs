//! Port of `lib/worker/WorkerFilesHash.java`.

use std::collections::BTreeMap;

use blaze_vfs::PathFragment;
use sha2::{Digest, Sha256};

use crate::actions::{FileStateType, InputMetadataProvider, Spawn, input_metadata_provider};

/// Calculates the hash based on the files, which should be unchanged on disk for a worker to get
/// reused.
// Java: com/google/devtools/build/lib/worker/WorkerFilesHash.java:38 WorkerFilesHash
pub struct WorkerFilesHash;

impl WorkerFilesHash {
    /// A `None` digest (Java's null) is a `NullPointerException` at `digest.length`, which
    /// crashes Bazel (review exec-2, E2-8).
    // Java: com/google/devtools/build/lib/worker/WorkerFilesHash.java:42 WorkerFilesHash.getCombinedHash
    pub fn get_combined_hash(
        worker_files_map: &BTreeMap<PathFragment, Option<Vec<u8>>>,
    ) -> Vec<u8> {
        // Hashing.sha256().newHasher(); Guava's putInt writes little-endian.
        let mut hasher = Sha256::new();
        for (exec_path, digest) in worker_files_map {
            let exec_path_bytes =
                blaze_util::string_encoding::latin1_encode(exec_path.get_path_string());
            hasher.update([0u8]);
            hasher.update((exec_path_bytes.len() as i32).to_le_bytes());
            hasher.update(&exec_path_bytes);

            let Some(digest) = digest else {
                panic!(
                    "java.lang.NullPointerException: Cannot read the array length because \"digest\" is null"
                );
            };
            hasher.update((digest.len() as i32).to_le_bytes());
            hasher.update(digest);
        }
        hasher.finalize().to_vec()
    }

    /// Return a map that contains the execroot relative path and hash of each tool and runfiles
    /// artifact of the given spawn.
    ///
    /// Panics with Java's `MissingInputException` message (an unchecked exception) if metadata
    /// is missing for any of the worker files. The `Err` is an `IOException` of the metadata
    /// provider.
    // Java: com/google/devtools/build/lib/worker/WorkerFilesHash.java:63 WorkerFilesHash.getWorkerFilesWithDigests
    pub fn get_worker_files_with_digests(
        spawn: &dyn Spawn,
        action_input_file_cache: &dyn InputMetadataProvider,
    ) -> std::io::Result<BTreeMap<PathFragment, Option<Vec<u8>>>> {
        let mut worker_files_map: BTreeMap<PathFragment, Option<Vec<u8>>> = BTreeMap::new();

        let tools = input_metadata_provider::expand_artifacts(
            action_input_file_cache,
            spawn.get_tool_files(),
            /* keep_empty_tree_artifacts= */ false,
            /* keep_runfiles_trees= */ true,
        );
        for tool in &tools {
            if tool.as_artifact().is_some_and(|a| a.is_runfiles_tree()) {
                let runfiles_tree = action_input_file_cache
                    .get_runfiles_metadata(tool)
                    .unwrap_or_else(|| panic!("NullPointerException: no runfiles metadata"))
                    .get_runfiles_tree()
                    .clone();
                let root = runfiles_tree.get_exec_path();
                blaze_util::check_state!(!root.is_absolute(), "{}", root);
                for (key, local_artifact) in runfiles_tree.get_mapping() {
                    if let Some(local_artifact) = local_artifact {
                        let metadata =
                            action_input_file_cache.get_input_metadata(&local_artifact)?;
                        let Some(metadata) = metadata else {
                            missing_input_exception(&local_artifact.get_exec_path_string());
                        };
                        match metadata.get_digest() {
                            Some(digest) => {
                                worker_files_map.insert(
                                    spawn.get_path_mapper().map(&root.get_relative(&key)),
                                    Some(digest.to_vec()),
                                );
                            }
                            None => {
                                // If BAZEL_TRACK_SOURCE_DIRECTORIES is explicitly disabled, the
                                // metadata may not have a digest.
                                blaze_util::check_state!(
                                    metadata.get_type() == FileStateType::Directory
                                );
                            }
                        }
                    }
                }

                continue;
            }

            let metadata = action_input_file_cache.get_input_metadata(tool)?;
            let Some(metadata) = metadata else {
                missing_input_exception(&tool.get_exec_path_string());
            };
            // Java puts `metadata.getDigest()` (possibly null) into the TreeMap.
            worker_files_map.insert(
                spawn.get_path_mapper().map(tool.get_exec_path()),
                metadata.get_digest().map(<[u8]>::to_vec),
            );
        }

        Ok(worker_files_map)
    }
}

/// `throw new MissingInputException(input)`: a `RuntimeException`, which crashes Bazel.
// Java: com/google/devtools/build/lib/worker/WorkerFilesHash.java:116 WorkerFilesHash.MissingInputException.<init>
fn missing_input_exception(exec_path_string: &str) -> ! {
    panic!(
        "com.google.devtools.build.lib.worker.WorkerFilesHash$MissingInputException: {}",
        format_args!("Missing input metadata for: '{exec_path_string}'")
    )
}
