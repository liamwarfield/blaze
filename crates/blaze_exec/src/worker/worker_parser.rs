//! Port of `lib/worker/WorkerParser.java`.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use regex::Regex;

use super::worker_files_hash::WorkerFilesHash;
use super::worker_key::WorkerKey;
use crate::actions::execution_requirements::WorkerProtocolFormat;
use crate::actions::{ExecException, Spawn, UserExecException, spawns};
use crate::exec::bin_tools::BinTools;
use crate::exec::local::local_env_provider::LocalEnvProvider;
use crate::exec::spawn_runner::SpawnExecutionContext;

// Java: com/google/devtools/build/lib/worker/WorkerParser.java:46 WorkerParser.ERROR_MESSAGE_PREFIX
const ERROR_MESSAGE_PREFIX: &str = "Worker strategy cannot execute this %s action, ";
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:48 WorkerParser.REASON_NO_FLAGFILE
const REASON_NO_FLAGFILE: &str =
    "because the command-line arguments do not contain exactly one @flagfile or --flagfile=";
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:50 WorkerParser.REASON_EXCESS_FLAGFILE
const REASON_EXCESS_FLAGFILE: &str =
    "because the command-line arguments has a @flagfile or --flagfile= argument before the end";
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:52 WorkerParser.REASON_NO_FINAL_FLAGFILE
const REASON_NO_FINAL_FLAGFILE: &str =
    "because the command-line arguments does not end with a @flagfile or --flagfile= argument";

/// Pattern for @flagfile.txt and --flagfile=flagfile.txt. This doesn't handle @@-escapes, those
/// are checked for separately. (`Matcher.matches()`: anchored; `.` does not match line
/// terminators.)
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:59 WorkerParser.FLAG_FILE_PATTERN
static FLAG_FILE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:@|--?flagfile=)([^\n\r\u{85}\u{2028}\u{2029}]+)$").unwrap());

/// Legacy pattern for @flagfile.txt and --flagfile=flagfile.txt. This doesn't handle @@-escapes.
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:64 WorkerParser.LEGACY_FLAG_FILE_PATTERN
static LEGACY_FLAG_FILE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:@|--?flagfile=)([^\n\r\u{85}\u{2028}\u{2029}]+)$").unwrap());

/// What `WorkerParser.compute` throws: `ExecException` or `IOException`.
#[derive(Debug)]
pub enum WorkerParserError {
    Exec(ExecException),
    Io(std::io::Error),
}

impl From<ExecException> for WorkerParserError {
    fn from(e: ExecException) -> Self {
        WorkerParserError::Exec(e)
    }
}

impl From<std::io::Error> for WorkerParserError {
    fn from(e: std::io::Error) -> Self {
        WorkerParserError::Io(e)
    }
}

/// A helper class to process a `Spawn` into a [`WorkerKey`], which is used to select a persistent
/// worker process (actions with equal keys are allowed to use the same worker process), and a
/// separate list of flag files. The result is encapsulated as a [`WorkerConfig`].
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:45 WorkerParser
pub struct WorkerParser {
    /// The global execRoot.
    exec_root: Path,
    worker_options: Arc<WorkerOptions>,
    local_env_provider: Arc<dyn LocalEnvProvider>,
    bin_tools: Arc<BinTools>,
}

impl WorkerParser {
    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:74 WorkerParser.<init>
    pub fn new(
        exec_root: Path,
        worker_options: Arc<WorkerOptions>,
        local_env_provider: Arc<dyn LocalEnvProvider>,
        bin_tools: Arc<BinTools>,
    ) -> WorkerParser {
        WorkerParser {
            exec_root,
            worker_options,
            local_env_provider,
            bin_tools,
        }
    }

    /// Processes the given `Spawn` and `SpawnExecutionContext` to compute the worker key. This
    /// involves splitting the command line into the worker startup command and the separate list
    /// of flag files. Returns a pair of the [`WorkerKey`] and list of flag files.
    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:90 WorkerParser.compute
    pub fn compute(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<WorkerConfig, WorkerParserError> {
        // We assume that the spawn to be executed always gets at least one @flagfile.txt or
        // --flagfile=flagfile.txt argument, which contains the flags related to the work itself
        // (as opposed to start-up options for the executed tool). Thus, we can extract those
        // elements from its args and put them into the WorkRequest instead.
        let mut flag_files: Vec<String> = Vec::new();
        let worker_args = self.split_spawn_args_into_worker_args_and_flag_files(spawn, &mut flag_files)?;
        let env = self.local_env_provider.rewrite_local_env(
            spawn.get_environment(),
            &self.bin_tools,
            "/tmp",
        )?;

        let worker_files = WorkerFilesHash::get_worker_files_with_digests(
            spawn,
            context.get_input_metadata_provider(),
        )?;

        let worker_files_combined_hash = WorkerFilesHash::get_combined_hash(&worker_files);

        let protocol_format = spawns::get_worker_protocol_format(spawn).map_err(|m| {
            super::error_message::JavaException::io(m).into_io_error()
        })?;
        let key = WorkerParser::create_worker_key(
            spawn,
            worker_args,
            env,
            self.exec_root.clone(),
            worker_files_combined_hash,
            worker_files,
            &self.worker_options,
            context.speculating(),
            protocol_format,
        );
        Ok(WorkerConfig::new(key, flag_files))
    }

    /// This method handles the logic of creating a WorkerKey (e.g., if sandboxing should be
    /// enabled or not, when to use multiplex-workers)
    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:126 WorkerParser.createWorkerKey
    pub fn create_worker_key(
        spawn: &dyn Spawn,
        worker_args: Vec<String>,
        env: IndexMap<String, String>,
        exec_root: Path,
        worker_files_combined_hash: Vec<u8>,
        worker_files: BTreeMap<PathFragment, Vec<u8>>,
        options: &WorkerOptions,
        dynamic: bool,
        protocol_format: WorkerProtocolFormat,
    ) -> WorkerKey {
        let worker_key_mnemonic = spawns::get_worker_key_mnemonic(spawn);
        let must_sandbox = dynamic || spawns::uses_path_mapping(spawn);
        let should_multiplex = options.worker_multiplex && spawns::supports_multiplex_workers(spawn);
        let can_sandbox_multiplex =
            options.multiplex_sandboxing && spawns::supports_multiplex_sandboxing(spawn);
        let sandboxed;
        let multiplex;
        if must_sandbox {
            sandboxed = true;
            multiplex = should_multiplex && can_sandbox_multiplex;
        } else if should_multiplex {
            sandboxed = can_sandbox_multiplex;
            multiplex = true;
        } else {
            sandboxed = options.worker_sandboxing;
            multiplex = false;
        }
        let mut use_in_memory_tracking = false;
        if sandboxed {
            let mnemonics = &options.worker_sandbox_in_memory_tracking;
            use_in_memory_tracking = mnemonics.contains(&worker_key_mnemonic);
        }
        WorkerKey::new(
            worker_args,
            env,
            exec_root,
            worker_key_mnemonic,
            worker_files_combined_hash,
            worker_files,
            sandboxed,
            use_in_memory_tracking,
            multiplex,
            spawns::supports_worker_cancellation(spawn),
            protocol_format,
        )
    }

    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:170 WorkerParser.isFlagFileArg
    fn is_flag_file_arg(arg: &str) -> bool {
        FLAG_FILE_PATTERN.is_match(arg) && !arg.starts_with("@@")
    }

    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:174 WorkerParser.isLegacyFlagFileArg
    fn is_legacy_flag_file_arg(arg: &str) -> bool {
        LEGACY_FLAG_FILE_PATTERN.is_match(arg)
    }

    /// Splits the command-line arguments of the `Spawn` into the part that is used to start the
    /// persistent worker (`worker_args`) and the part that goes into the `WorkRequest` protobuf
    /// (`flag_files`).
    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:183 WorkerParser.splitSpawnArgsIntoWorkerArgsAndFlagFiles
    pub fn split_spawn_args_into_worker_args_and_flag_files(
        &self,
        spawn: &dyn Spawn,
        flag_files: &mut Vec<String>,
    ) -> Result<Vec<String>, ExecException> {
        let mut worker_args: Vec<String> = Vec::new();
        let args = spawn.get_arguments();
        if args.is_empty() {
            return Err(throw_flag_file_failure(REASON_NO_FLAGFILE, spawn));
        }
        if self.worker_options.strict_flagfiles {
            let last = args.last().unwrap();
            if !WorkerParser::is_flag_file_arg(last) {
                return Err(throw_flag_file_failure(REASON_NO_FINAL_FLAGFILE, spawn));
            }
            flag_files.push(last.clone());
            for arg in &args[..args.len() - 1] {
                if WorkerParser::is_flag_file_arg(arg) {
                    return Err(throw_flag_file_failure(REASON_EXCESS_FLAGFILE, spawn));
                } else {
                    worker_args.push(arg.clone());
                }
            }
        } else {
            for arg in args {
                if WorkerParser::is_legacy_flag_file_arg(arg) {
                    flag_files.push(arg.clone());
                } else {
                    worker_args.push(arg.clone());
                }
            }
            if flag_files.is_empty() {
                return Err(throw_flag_file_failure(REASON_NO_FLAGFILE, spawn));
            }
        }

        let mnemonic = spawns::get_worker_key_mnemonic(spawn);
        let mnemonic_flags: Vec<String> = self
            .worker_options
            .worker_extra_flags
            .iter()
            .filter(|(key, _)| *key == mnemonic)
            .map(|(_, value)| value.clone())
            .collect();

        worker_args.push("--persistent_worker".to_string());
        worker_args.extend(mnemonic_flags);
        Ok(worker_args)
    }
}

// Java: com/google/devtools/build/lib/worker/WorkerParser.java:220 WorkerParser.throwFlagFileFailure
fn throw_flag_file_failure(reason: &str, spawn: &dyn Spawn) -> ExecException {
    // String.format(ERROR_MESSAGE_PREFIX + reason + "%n%s", mnemonic, arguments): %n is the
    // line separator, the list prints as `[a, b]`.
    let message = format!(
        "{}{}\n[{}]",
        ERROR_MESSAGE_PREFIX.replace("%s", &spawn.get_mnemonic()),
        reason,
        spawn.get_arguments().join(", ")
    );
    UserExecException::new(FailureDetail {
        message,
        category: Some(failure_detail::Category::Worker(failure_details::Worker {
            code: failure_details::worker::Code::NoFlagfile as i32,
        })),
    })
    .into()
}

/// A pair of the [`WorkerKey`] and the list of flag files.
// Java: com/google/devtools/build/lib/worker/WorkerParser.java:233 WorkerParser.WorkerConfig
pub struct WorkerConfig {
    worker_key: WorkerKey,
    flag_files: Vec<String>,
}

impl WorkerConfig {
    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:237 WorkerParser.WorkerConfig.<init>
    pub fn new(worker_key: WorkerKey, flag_files: Vec<String>) -> WorkerConfig {
        WorkerConfig {
            worker_key,
            flag_files,
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:242 WorkerParser.WorkerConfig.getWorkerKey
    pub fn get_worker_key(&self) -> &WorkerKey {
        &self.worker_key
    }

    // Java: com/google/devtools/build/lib/worker/WorkerParser.java:246 WorkerParser.WorkerConfig.getFlagFiles
    pub fn get_flag_files(&self) -> &[String] {
        &self.flag_files
    }

    /// Takes the key.
    pub fn into_parts(self) -> (WorkerKey, Vec<String>) {
        (self.worker_key, self.flag_files)
    }
}
