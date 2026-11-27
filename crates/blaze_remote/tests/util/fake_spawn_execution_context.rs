//! Port of the test class `lib/remote/util/FakeSpawnExecutionContext.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_exec::actions::{ExecException, InputMapping, InputMetadataProvider, Spawn};
use blaze_exec::exec::spawn_input_expander::SpawnInputExpander;
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_protos::tools::protos::Digest;
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// Execution context for tests.
///
/// PORT: Java's `actionContextRegistry` (`getContext`), `getPathResolver` and
/// `actionFileSystem` (`getActionFileSystem`) have no counterpart in blaze_exec's
/// `SpawnExecutionContext`; the remote action file system is the `RemoteOutputInjector` hook of
/// `RemoteExecutionService` (`blaze_remote::hooks`). `exec_root` is kept for the path resolver
/// (`ArtifactPathResolver.forExecRoot(execRoot)`).
// Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:39 FakeSpawnExecutionContext
#[allow(dead_code)]
pub struct FakeSpawnExecutionContext {
    lock_output_files_called: AtomicBool,

    spawn: Arc<dyn Spawn>,
    input_metadata_provider: Arc<dyn InputMetadataProvider>,
    exec_root: Path,
    out_err: Arc<FileOutErr>,

    digest: Mutex<Option<Digest>>,
    client_env: IndexMap<String, String>,
}

#[allow(dead_code)]
impl FakeSpawnExecutionContext {
    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:52 FakeSpawnExecutionContext.<init>
    pub fn new(
        spawn: Arc<dyn Spawn>,
        input_metadata_provider: Arc<dyn InputMetadataProvider>,
        exec_root: Path,
        out_err: Arc<FileOutErr>,
    ) -> Self {
        FakeSpawnExecutionContext {
            lock_output_files_called: AtomicBool::new(false),
            spawn,
            input_metadata_provider,
            exec_root,
            out_err,
            digest: Mutex::new(None),
            client_env: IndexMap::new(),
        }
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:67 FakeSpawnExecutionContext.isLockOutputFilesCalled
    pub fn is_lock_output_files_called(&self) -> bool {
        self.lock_output_files_called.load(Ordering::SeqCst)
    }

    /// The spawn this context was made for.
    pub fn spawn(&self) -> &Arc<dyn Spawn> {
        &self.spawn
    }

    /// `execRoot` (what `getPathResolver()` resolves against).
    pub fn exec_root(&self) -> &Path {
        &self.exec_root
    }
}

impl SpawnExecutionContext for FakeSpawnExecutionContext {
    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:72 FakeSpawnExecutionContext.getId
    fn get_id(&self) -> i32 {
        0
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:77 FakeSpawnExecutionContext.setDigest
    fn set_digest(&self, digest: Digest) {
        *self.digest.lock() = Some(digest);
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:82 FakeSpawnExecutionContext.getDigest
    fn get_digest(&self) -> Option<Digest> {
        self.digest.lock().clone()
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:87 FakeSpawnExecutionContext.prefetchInputs
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        panic!("UnsupportedOperationException");
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:102 FakeSpawnExecutionContext.getInputMetadataProvider
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &*self.input_metadata_provider
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:92 FakeSpawnExecutionContext.lockOutputFiles
    fn lock_output_files(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException> {
        self.lock_output_files_called.store(true, Ordering::SeqCst);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:97 FakeSpawnExecutionContext.speculating
    fn speculating(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:112 FakeSpawnExecutionContext.getTimeout
    fn get_timeout(&self) -> blaze_util::java_duration::JavaDuration {
        blaze_util::java_duration::JavaDuration::ZERO
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:117 FakeSpawnExecutionContext.getFileOutErr
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:122 FakeSpawnExecutionContext.getInputMapping
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        Arc::new(SpawnInputExpander::new().get_input_mapping(
            &*self.spawn,
            &*self.input_metadata_provider,
            base_directory,
        ))
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:128 FakeSpawnExecutionContext.report
    fn report(&self, _progress: ProgressStatus) {
        // Intentionally left empty.
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:138 FakeSpawnExecutionContext.isRewindingEnabled
    fn is_rewinding_enabled(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:143 FakeSpawnExecutionContext.checkForLostInputs
    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java:152 FakeSpawnExecutionContext.getClientEnv
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}
