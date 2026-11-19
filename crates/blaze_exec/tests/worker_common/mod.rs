//! Port of the parts of src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java
//! that the worker tests share.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::action_execution_metadata::FakeOwner;
use blaze_exec::actions::execution_requirements::{self as er, WorkerProtocolFormat};
use blaze_exec::actions::{ResourceSet, SimpleSpawn, Spawn};
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_parser::WorkerParser;
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_options::Options;
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;

/// `new WorkerOptions()` (every field at its default).
pub fn worker_options() -> WorkerOptions {
    Options::<WorkerOptions>::get_defaults()
}

/// An in-memory-like file system: the real one, rooted nowhere in particular (paths are only
/// compared, never touched).
pub fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:50 WorkerTestUtils.createSpawn
pub fn create_spawn(arguments: &[&str], execution_info: IndexMap<String, String>) -> SimpleSpawn {
    SimpleSpawn::new_simple(
        // ActionsTestUtil.NullAction: mnemonic "Null".
        Arc::new(FakeOwner::new("Null", None, None)),
        arguments.iter().map(|s| s.to_string()).collect(),
        IndexMap::new(),
        execution_info,
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        Arc::new(ResourceSet::zero()),
    )
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:185 WorkerTestUtils.execRequirementsBuilder
pub fn exec_requirements_builder(mnemonic: &str) -> IndexMap<String, String> {
    let mut m = IndexMap::new();
    m.insert(er::WORKER_KEY_MNEMONIC.to_string(), mnemonic.to_string());
    m.insert(
        er::REQUIRES_WORKER_PROTOCOL.to_string(),
        "proto".to_string(),
    );
    m.insert(er::SUPPORTS_WORKERS.to_string(), "1".to_string());
    m.insert(er::SUPPORTS_MULTIPLEX_WORKERS.to_string(), "1".to_string());
    m
}

fn env() -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    env.insert("env1".to_string(), "foo".to_string());
    env.insert("env2".to_string(), "bar".to_string());
    env
}

/// `HashCode.fromInt(0).asBytes()`.
fn hash_zero() -> Vec<u8> {
    vec![0, 0, 0, 0]
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:161 WorkerTestUtils.createWorkerKeyWithRequirements
pub fn create_worker_key_with_requirements(
    output_base: &Path,
    worker_options: &WorkerOptions,
    mnemonic: &str,
    dynamic: bool,
    extra_requirements: &[&str],
) -> WorkerKey {
    let mut builder = exec_requirements_builder(mnemonic);
    for req in extra_requirements {
        builder.insert(req.to_string(), "1".to_string());
    }
    let spawn = create_spawn(&[], builder);
    WorkerParser::create_worker_key(
        &spawn,
        Vec::new(),
        env(),
        output_base.get_child("execroot"),
        hash_zero(),
        BTreeMap::new(),
        worker_options,
        dynamic,
        WorkerProtocolFormat::Proto,
    )
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:193 WorkerTestUtils.createWorkerKeyFromOptions
pub fn create_worker_key_from_options(
    protocol_format: WorkerProtocolFormat,
    output_base: &Path,
    worker_options: &WorkerOptions,
    dynamic: bool,
    spawn: &dyn Spawn,
    args: &[&str],
) -> WorkerKey {
    WorkerParser::create_worker_key(
        spawn,
        args.iter().map(|s| s.to_string()).collect(),
        env(),
        output_base.get_child("execroot"),
        hash_zero(),
        BTreeMap::new(),
        worker_options,
        dynamic,
        protocol_format,
    )
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:63 WorkerTestUtils.createWorkerKey
pub fn create_worker_key_full(
    protocol_format: WorkerProtocolFormat,
    fs: &FileSystemRef,
    mnemonic: &str,
    multiplex: bool,
    sandboxed: bool,
    dynamic: bool,
    args: &[&str],
) -> WorkerKey {
    let mut worker_options = worker_options();
    worker_options.worker_multiplex = multiplex;
    worker_options.worker_sandboxing = sandboxed;
    create_worker_key_from_options(
        protocol_format,
        &fs.get_path("/outputbase"),
        &worker_options,
        dynamic,
        &create_spawn(&[], exec_requirements_builder(mnemonic)),
        args,
    )
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:140 WorkerTestUtils.createWorkerKey
pub fn create_worker_key(
    fs: &FileSystemRef,
    multiplex: bool,
    sandboxed: bool,
    dynamic: bool,
) -> WorkerKey {
    create_worker_key_full(
        WorkerProtocolFormat::Proto,
        fs,
        "dummy",
        multiplex,
        sandboxed,
        dynamic,
        &["arg1", "arg2", "arg3"],
    )
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:100 WorkerTestUtils.createWorkerKey
pub fn create_worker_key_mnemonic(
    mnemonic: &str,
    fs: &FileSystemRef,
    multiplex: bool,
    sandboxed: bool,
) -> WorkerKey {
    create_worker_key_full(
        WorkerProtocolFormat::Proto,
        fs,
        mnemonic,
        multiplex,
        sandboxed,
        false,
        &[],
    )
}

/// `WorkerTestUtils.FakeSubprocess`: a process whose stdout is `bytes` and whose stdin is
/// recorded.
// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:243 WorkerTestUtils.FakeSubprocess
pub struct FakeSubprocess {
    input: parking_lot::Mutex<Option<Vec<u8>>>,
    pub output: Arc<parking_lot::Mutex<Vec<u8>>>,
    was_destroyed: std::sync::atomic::AtomicBool,
}

impl FakeSubprocess {
    pub fn new(bytes: Vec<u8>) -> Arc<FakeSubprocess> {
        Arc::new(FakeSubprocess {
            input: parking_lot::Mutex::new(Some(bytes)),
            output: Arc::new(parking_lot::Mutex::new(Vec::new())),
            was_destroyed: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

struct SharedWriter(Arc<parking_lot::Mutex<Vec<u8>>>);

impl std::io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The `Subprocess` handed to the worker; shares the fake's state.
pub struct FakeSubprocessHandle(pub Arc<FakeSubprocess>);

impl blaze_shell::Subprocess for FakeSubprocessHandle {
    fn destroy(&self) -> bool {
        self.0
            .was_destroyed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }
    fn exit_value(&self) -> i32 {
        0
    }
    fn finished(&self) -> bool {
        true
    }
    fn is_alive(&self) -> bool {
        !self
            .0
            .was_destroyed
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    fn timedout(&self) -> bool {
        false
    }
    fn wait_for(&self) -> Result<(), blaze_shell::InterruptedException> {
        Ok(())
    }
    fn get_output_stream(&self) -> Option<Box<dyn std::io::Write + Send>> {
        Some(Box::new(SharedWriter(self.0.output.clone())))
    }
    fn get_input_stream(&self) -> Option<Box<dyn std::io::Read + Send>> {
        let bytes = self.0.input.lock().take().unwrap_or_default();
        Some(Box::new(std::io::Cursor::new(bytes)))
    }
    fn get_error_stream(&self) -> Option<Box<dyn std::io::Read + Send>> {
        Some(Box::new(std::io::empty()))
    }
    fn get_process_id(&self) -> i64 {
        0
    }
    fn close(&self) {}
}

/// A `SubprocessFactory` that hands out one fake process.
pub struct FakeSubprocessFactory(pub Arc<FakeSubprocess>);

impl blaze_shell::SubprocessFactory for FakeSubprocessFactory {
    fn create(
        &self,
        _params: &blaze_shell::SubprocessBuilder,
    ) -> std::io::Result<Box<dyn blaze_shell::Subprocess>> {
        Ok(Box::new(FakeSubprocessHandle(self.0.clone())))
    }
}
