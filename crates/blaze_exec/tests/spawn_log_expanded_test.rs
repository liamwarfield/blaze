//! Port of `src/test/java/com/google/devtools/build/lib/exec/ExpandedSpawnLogContextTest.java`
//! (the tests of `SpawnLogContextTestBase` run against `ExpandedSpawnLogContext`).

mod spawn_log_test_base;

use std::sync::Arc;

use blaze_exec::exec::expanded_spawn_log_context::{Encoding, ExpandedSpawnLogContext};
use blaze_exec::exec::io::{BinaryInputStreamWrapper, MessageInputStream};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Options;
use blaze_protos::tools::protos::SpawnExec;
use blaze_vfs::DigestHashFunction;
use spawn_log_test_base as base;
use spawn_log_test_base::{Fixture, SpawnLogContextTest};

struct ExpandedSpawnLogContextTest;

impl SpawnLogContextTest for ExpandedSpawnLogContextTest {
    fn create_spawn_log_context_with(
        &self,
        f: &Fixture,
        platform_properties: &[(&str, &str)],
    ) -> Box<dyn SpawnLogContext> {
        let mut remote_options = Options::<RemoteOptions>::get_defaults();
        remote_options.remote_default_exec_properties = platform_properties
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        Box::new(
            ExpandedSpawnLogContext::new(
                f.scratch_path("log"),
                f.scratch_path("temp"),
                Encoding::Binary,
                /* sorted= */ false,
                f.exec_root.as_fragment().clone(),
                Some(Arc::new(remote_options)),
                DigestHashFunction::Sha256,
            )
            .unwrap(),
        )
    }

    fn close_and_assert_log(
        &self,
        f: &Fixture,
        context: Box<dyn SpawnLogContext>,
        expected: Vec<SpawnExec>,
    ) {
        context.close().unwrap();

        let mut actual = Vec::new();
        let mut input = BinaryInputStreamWrapper::<SpawnExec>::new(
            f.scratch_path("log").get_input_stream().unwrap(),
        );
        while let Some(ex) = input.read().unwrap() {
            actual.push(ex);
        }

        assert_eq!(actual, expected);
    }
}

const T: &ExpandedSpawnLogContextTest = &ExpandedSpawnLogContextTest;

#[test]
fn test_file_input() {
    base::test_file_input(T);
}

#[test]
fn test_file_input_with_directory_contents() {
    base::test_file_input_with_directory_contents(T);
}

#[test]
fn test_directory_input() {
    base::test_directory_input(T);
}

#[test]
fn test_tree_input() {
    base::test_tree_input(T);
}

#[test]
fn test_unresolved_symlink_input() {
    base::test_unresolved_symlink_input(T);
}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_file_input() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_tool_in_runfiles_tree() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_directory_input() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_empty_input() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_mixed_roots() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_external_only() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_files_collide() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_files_and_symlinks_collide() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_file_and_root_symlink_collide() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_cross_type_collision() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_post_order_collision() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_artifact_post_order_collision_with_duplicate() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_symlink_post_order_collision_with_semantic_duplicate() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_symlink_post_order_collision_with_equal_duplicate() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_symlink_targets() {}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfile_symlink_file_with_directory_contents() {}

#[test]
fn test_fileset_input() {
    base::test_fileset_input(T);
}

#[test]
fn test_param_file_input() {
    base::test_param_file_input(T);
}

#[test]
fn test_file_output() {
    base::test_file_output(T);
}

#[test]
fn test_file_output_with_invalid_type() {
    base::test_file_output_with_invalid_type(T);
}

#[test]
fn test_tree_output() {
    base::test_tree_output(T);
}

#[test]
fn test_tree_output_with_invalid_type() {
    base::test_tree_output_with_invalid_type(T);
}

#[test]
fn test_unresolved_symlink_output() {
    base::test_unresolved_symlink_output(T);
}

#[test]
fn test_unresolved_symlink_output_with_invalid_type() {
    base::test_unresolved_symlink_output_with_invalid_type(T);
}

#[test]
fn test_missing_output() {
    base::test_missing_output(T);
}

#[test]
fn test_environment() {
    base::test_environment(T);
}

#[test]
fn test_default_platform_properties() {
    base::test_default_platform_properties(T);
}

#[test]
fn test_spawn_platform_properties() {
    base::test_spawn_platform_properties(T);
}

#[test]
fn test_execution_info() {
    base::test_execution_info(T);
}

#[test]
fn test_cache_hit() {
    base::test_cache_hit(T);
}

#[test]
fn test_digest() {
    base::test_digest(T);
}

#[test]
fn test_timeout() {
    base::test_timeout(T);
}

#[test]
fn test_spawn_metrics() {
    base::test_spawn_metrics(T);
}

#[test]
fn test_status() {
    base::test_status(T);
}
