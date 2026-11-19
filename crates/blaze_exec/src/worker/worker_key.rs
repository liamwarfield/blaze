//! Port of `lib/worker/WorkerKey.java`.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_shell::{CommandDescriptionForm, CommandFailureUtils};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::actions::execution_requirements::WorkerProtocolFormat;

/// `String.hashCode()` of a Java string (chars as UTF-16 code units; an internal string's chars
/// are its bytes).
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// `Boolean.hashCode(b)`.
fn java_boolean_hash_code(b: bool) -> i32 {
    if b { 1231 } else { 1237 }
}

/// `List.hashCode()` of a list of strings.
fn java_list_hash_code(list: &[String]) -> i32 {
    let mut h: i32 = 1;
    for s in list {
        h = h.wrapping_mul(31).wrapping_add(java_string_hash_code(s));
    }
    h
}

/// `Map.hashCode()` of a string map: the sum of `key.hashCode() ^ value.hashCode()`.
fn java_map_hash_code(map: &IndexMap<String, String>) -> i32 {
    let mut h: i32 = 0;
    for (k, v) in map {
        h = h.wrapping_add(java_string_hash_code(k) ^ java_string_hash_code(v));
    }
    h
}

/// Data container that uniquely identifies a kind of worker process and is used as the key for
/// the `WorkerPoolImpl`.
///
/// We expect a small number of WorkerKeys per mnemonic. Unbounded creation of WorkerKeys will
/// break various things as well as render the workers less useful.
// Java: com/google/devtools/build/lib/worker/WorkerKey.java:37 WorkerKey
#[derive(Clone)]
pub struct WorkerKey {
    /// Build options.
    args: Vec<String>,
    /// Environment variables.
    env: IndexMap<String, String>,
    /// Execution root of Bazel process.
    exec_root: Path,
    /// Mnemonic of the worker.
    mnemonic: String,
    /// These are used during validation whether a worker is still usable. They are not used to
    /// uniquely identify a kind of worker, thus it is not to be used by the .equals() / .hashCode()
    /// methods.
    worker_files_combined_hash: Vec<u8>,
    /// Worker files with the corresponding digest.
    worker_files_with_digests: BTreeMap<PathFragment, Vec<u8>>,
    /// If true, the workers run inside a sandbox.
    sandboxed: bool,
    /// If true, the sandbox contents are tracked in memory to speed up cleanup.
    use_in_memory_tracking: bool,
    /// A WorkerProxy will be instantiated if true, instantiate a regular Worker if false.
    multiplex: bool,
    /// If true, the workers for this key are able to cancel work requests.
    cancellable: bool,
    /// Cached value for the hash of this key, because the value is expensive to calculate.
    hash: i32,
    /// The format of the worker protocol sent to and read from the worker.
    protocol_format: WorkerProtocolFormat,
}

impl WorkerKey {
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:85 WorkerKey.<init>
    pub fn new(
        args: Vec<String>,
        env: IndexMap<String, String>,
        exec_root: Path,
        mnemonic: String,
        worker_files_combined_hash: Vec<u8>,
        worker_files_with_digests: BTreeMap<PathFragment, Vec<u8>>,
        sandboxed: bool,
        use_in_memory_tracking: bool,
        multiplex: bool,
        cancellable: bool,
        protocol_format: WorkerProtocolFormat,
    ) -> WorkerKey {
        let mut key = WorkerKey {
            args,
            env,
            exec_root,
            mnemonic,
            worker_files_combined_hash,
            worker_files_with_digests,
            sandboxed,
            use_in_memory_tracking,
            multiplex,
            cancellable,
            hash: 0,
            protocol_format,
        };
        key.hash = key.calculate_hash_code();
        key
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:111 WorkerKey.getArgs
    pub fn get_args(&self) -> &[String] {
        &self.args
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:115 WorkerKey.getEnv
    pub fn get_env(&self) -> &IndexMap<String, String> {
        &self.env
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:119 WorkerKey.getExecRoot
    pub fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:123 WorkerKey.getMnemonic
    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:127 WorkerKey.getWorkerFilesCombinedHash
    pub fn get_worker_files_combined_hash(&self) -> &[u8] {
        &self.worker_files_combined_hash
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:131 WorkerKey.getWorkerFilesWithDigests
    pub fn get_worker_files_with_digests(&self) -> &BTreeMap<PathFragment, Vec<u8>> {
        &self.worker_files_with_digests
    }

    /// Returns true if workers are sandboxed.
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:136 WorkerKey.isSandboxed
    pub fn is_sandboxed(&self) -> bool {
        self.sandboxed
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:140 WorkerKey.useInMemoryTracking
    pub fn use_in_memory_tracking(&self) -> bool {
        self.use_in_memory_tracking
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:144 WorkerKey.isMultiplex
    pub fn is_multiplex(&self) -> bool {
        self.multiplex
    }

    /// `cancellable` (no Java getter; read by tests through equals).
    pub fn is_cancellable(&self) -> bool {
        self.cancellable
    }

    /// Returns the format of the worker protocol.
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:149 WorkerKey.getProtocolFormat
    pub fn get_protocol_format(&self) -> WorkerProtocolFormat {
        self.protocol_format
    }

    /// Returns a user-friendly name for this worker type.
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:154 WorkerKey.makeWorkerTypeName
    pub fn make_worker_type_name(proxied: bool, must_be_sandboxed: bool) -> &'static str {
        if proxied && !must_be_sandboxed {
            "multiplex-worker"
        } else {
            "worker"
        }
    }

    /// Returns a user-friendly name for this worker type.
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:163 WorkerKey.getWorkerTypeName
    pub fn get_worker_type_name(&self) -> &'static str {
        // Current implementation does not support sandboxing with multiplex workers, so keys
        // will only be proxied if they are not forced to be sandboxed due to dynamic execution.
        WorkerKey::make_worker_type_name(self.multiplex, false)
    }

    /// `hashCode()`.
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:208 WorkerKey.hashCode
    pub fn hash_code(&self) -> i32 {
        self.hash
    }

    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:212 WorkerKey.calculateHashCode
    fn calculate_hash_code(&self) -> i32 {
        // Use the string representation of the protocolFormat because the hash of the same enum
        // value can vary across instances.
        // Objects.hash(...) = Arrays.hashCode(Object[]).
        let elements = [
            java_list_hash_code(&self.args),
            java_map_hash_code(&self.env),
            // Path.hashCode() is its PathFragment's, the normalized string's hash.
            java_string_hash_code(self.exec_root.get_path_string()),
            java_string_hash_code(&self.mnemonic),
            java_boolean_hash_code(self.multiplex),
            java_boolean_hash_code(self.cancellable),
            java_boolean_hash_code(self.sandboxed),
            java_boolean_hash_code(self.use_in_memory_tracking),
            java_string_hash_code(&self.protocol_format.to_string()),
        ];
        let mut result: i32 = 1;
        for e in elements {
            result = result.wrapping_mul(31).wrapping_add(e);
        }
        result
    }
}

impl PartialEq for WorkerKey {
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:169 WorkerKey.equals
    fn eq(&self, worker_key: &WorkerKey) -> bool {
        if std::ptr::eq(self, worker_key) {
            return true;
        }
        if self.hash != worker_key.hash {
            return false;
        }
        if self.args != worker_key.args {
            return false;
        }
        if self.multiplex != worker_key.multiplex {
            return false;
        }
        if self.cancellable != worker_key.cancellable {
            return false;
        }
        if self.sandboxed != worker_key.sandboxed {
            return false;
        }
        if self.use_in_memory_tracking != worker_key.use_in_memory_tracking {
            return false;
        }
        // ImmutableMap.equals ignores order.
        if self.env.len() != worker_key.env.len()
            || self
                .env
                .iter()
                .any(|(k, v)| worker_key.env.get(k) != Some(v))
        {
            return false;
        }
        if self.exec_root != worker_key.exec_root {
            return false;
        }
        if self.protocol_format != worker_key.protocol_format {
            return false;
        }
        self.mnemonic == worker_key.mnemonic
    }
}

impl Eq for WorkerKey {}

impl Hash for WorkerKey {
    /// Since all fields involved in the `hashCode` are final, we cache the result.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl fmt::Display for WorkerKey {
    // Java: com/google/devtools/build/lib/worker/WorkerKey.java:228 WorkerKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // We print this command out in such a way that it can safely be
        // copied+pasted as a Bourne shell command.  This is extremely valuable for
        // debugging.
        f.write_str(&CommandFailureUtils::describe_command(
            CommandDescriptionForm::Complete,
            /* pretty_print_args= */ false,
            &self.args,
            Some(&self.env),
            /* environment_variables_to_clear= */ None,
            Some(self.exec_root.get_path_string()),
            /* configuration_checksum= */ None,
            /* execution_platform_label= */ None,
            /* spawn_runner= */ Some(self.get_worker_type_name()),
        ))
    }
}

impl fmt::Debug for WorkerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
