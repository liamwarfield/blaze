#![cfg(feature = "wip")]
//! Port of `lib/remote/RemotePathResolverTest.java`.
//!
//! PORT: Java uses an `InMemoryFileSystem`; the tests do no I/O, so the paths are plain
//! `UnixFileSystem` paths that are never touched.

use std::sync::Arc;
use std::time::Duration;

use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, ExecException, HierarchicalPath, InputMapping,
    InputMetadataProvider,
};
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_protos::tools::protos::Digest;
use blaze_remote::common::remote_path_resolver::{
    RemotePathResolver, SiblingRepositoryLayoutResolver, create_default,
};
use blaze_vfs::{DigestHashFunction, FileSystemExt, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;

/// `mock(SpawnExecutionContext.class)` with `getInputMapping` stubbed as in `setup()`. The
/// methods that return references are not stubbed in Java either (a mock returns null).
struct MockSpawnExecutionContext {
    input: ActionInputRef,
}

impl SpawnExecutionContext for MockSpawnExecutionContext {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, _digest: Digest) {}
    fn get_digest(&self) -> Option<Digest> {
        None
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        Ok(())
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        unimplemented!("not stubbed: getInputMetadataProvider")
    }
    fn lock_output_files(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: &FileOutErr,
    ) -> Result<(), ExecException> {
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> Duration {
        Duration::ZERO
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        unimplemented!("not stubbed: getFileOutErr")
    }
    // Java: RemotePathResolverTest.setup (the `thenAnswer` of `getInputMapping`)
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        let mut input_map = InputMapping::new();
        input_map.insert(
            HierarchicalPath(base_directory.get_relative(self.input.get_exec_path())),
            self.input.clone(),
        );
        Arc::new(input_map)
    }
    fn report(&self, _progress: ProgressStatus) {}
    fn get_client_env(&self) -> &IndexMap<String, String> {
        unimplemented!("not stubbed: getClientEnv")
    }
}

struct Fixture {
    exec_root: Path,
    spawn_execution_context: MockSpawnExecutionContext,
    input: ActionInputRef,
}

// Java: RemotePathResolverTest.setup
fn setup() -> Fixture {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = fs.get_path("/execroot/main");

    let input = ActionInputRef::new(BasicActionInput::from_path("foo"));
    let spawn_execution_context = MockSpawnExecutionContext {
        input: input.clone(),
    };
    Fixture {
        exec_root,
        spawn_execution_context,
        input,
    }
}

#[test]
fn get_working_directory_default_is_input_root() {
    let f = setup();
    let remote_path_resolver = create_default(f.exec_root.clone());

    let working_directory = remote_path_resolver.get_working_directory();

    assert_eq!(working_directory.get_path_string(), "");
}

#[test]
fn get_working_directory_sibling_is_exec_root_base_name() {
    let f = setup();
    let remote_path_resolver = SiblingRepositoryLayoutResolver::new(f.exec_root.clone());

    let working_directory = remote_path_resolver.get_working_directory();

    assert_eq!(working_directory.get_path_string(), "main");
}

#[test]
fn get_input_mapping_default_inputs_relative_to_exec_root() {
    let f = setup();
    let remote_path_resolver = create_default(f.exec_root.clone());

    let inputs = remote_path_resolver.get_input_mapping(&f.spawn_execution_context, false);

    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs.get(&HierarchicalPath(PathFragment::create("foo"))),
        Some(&f.input)
    );
}

#[test]
fn get_input_mapping_sibling_inputs_relative_to_input_root() {
    let f = setup();
    let remote_path_resolver = SiblingRepositoryLayoutResolver::new(f.exec_root.clone());

    let inputs = remote_path_resolver.get_input_mapping(&f.spawn_execution_context, false);

    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs.get(&HierarchicalPath(PathFragment::create("main/foo"))),
        Some(&f.input)
    );
}

#[test]
fn convert_paths_default_relative_to_working_directory() {
    let f = setup();
    let remote_path_resolver = create_default(f.exec_root.clone());

    let output_path =
        remote_path_resolver.local_exec_path_to_output_path(&PathFragment::create("bar"));
    let local_path = remote_path_resolver.output_path_to_local_path(&output_path);

    assert_eq!(output_path, "bar");
    assert_eq!(local_path, f.exec_root.get_relative_str("bar"));
}

#[test]
fn convert_paths_sibling_compatible_relative_to_working_directory() {
    let f = setup();
    let remote_path_resolver = SiblingRepositoryLayoutResolver::new(f.exec_root.clone());

    let output_path =
        remote_path_resolver.local_exec_path_to_output_path(&PathFragment::create("bar"));
    let local_path = remote_path_resolver.output_path_to_local_path(&output_path);

    assert_eq!(output_path, "bar");
    assert_eq!(local_path, f.exec_root.get_relative_str("bar"));
}
