//! Port of `src/test/java/com/google/devtools/build/lib/exec/CompactSpawnLogContextTest.java`
//! (its own tests plus those of `SpawnLogContextTestBase`, run against `CompactSpawnLogContext`
//! and read back with `SpawnLogReconstructor`).

mod spawn_log_test_base;

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_events::EventHandler;
use blaze_exec::actions::{ActionInputRef, TreeArtifactValue};
use blaze_exec::exec::compact_spawn_log_context::CompactSpawnLogContext;
use blaze_exec::exec::io::{MessageInputStream, parse_delimited_from};
use blaze_exec::exec::spawn_log_context::{SpawnLogAction, SpawnLogContext};
use blaze_exec::exec::spawn_log_reconstructor::SpawnLogReconstructor;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Options;
use blaze_protos::tools::protos::{ExecLogEntry, SpawnExec, exec_log_entry};
use blaze_vfs::DigestHashFunction;
use spawn_log_test_base as base;
use spawn_log_test_base::{
    Fixture, InputsMode, OutputsMode, PRODUCT_NAME, SIBLING_REPOSITORY_LAYOUT, SpawnLogContextTest,
    WORKSPACE_NAME, create_artifact, create_tree_artifact_with_generating_action,
    default_spawn_builder, default_spawn_exec, default_spawn_result, default_timeout, file,
    write_file,
};

struct CompactSpawnLogContextTest;

impl SpawnLogContextTest for CompactSpawnLogContextTest {
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

        let reporter: Arc<dyn EventHandler> = f.stored_event_handler.clone();
        Box::new(
            CompactSpawnLogContext::new(
                &f.scratch_path("log"),
                f.exec_root.as_fragment().clone(),
                WORKSPACE_NAME.to_string(),
                f.sibling_repository_layout,
                Some(Arc::new(remote_options)),
                DigestHashFunction::Sha256,
                "00000000-0000-0000-0000-000000000000".to_string(),
                reporter,
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
        let mut reconstructor =
            SpawnLogReconstructor::new(f.scratch_path("log").get_input_stream().unwrap()).unwrap();
        while let Some(ex) = reconstructor.read().unwrap() {
            actual.push(ex);
        }
        reconstructor.close().unwrap();

        assert_eq!(actual, expected);
    }
}

const T: &CompactSpawnLogContextTest = &CompactSpawnLogContextTest;

/// `closeAndReadCompactLog(context)`.
fn close_and_read_compact_log(f: &Fixture, context: &dyn SpawnLogContext) -> Vec<ExecLogEntry> {
    context.close().unwrap();

    let mut entries = Vec::new();
    let input = f.scratch_path("log").get_input_stream().unwrap();
    let mut zstd_in = zstd::stream::read::Decoder::new(input).unwrap();
    while let Some(entry) = parse_delimited_from::<ExecLogEntry>(&mut zstd_in).unwrap() {
        entries.push(entry);
    }
    entries
}

#[test]
fn test_transitive_nested_set() {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for inputs_mode in InputsMode::ALL {
            let f = Fixture::new(sibling);
            let file1 = create_artifact(&f.root_dir, "file1");
            let file2 = create_artifact(&f.root_dir, "file2");
            let file3 = create_artifact(&f.root_dir, "file3");

            write_file(&f.path_of(&file1), "abc");
            write_file(&f.path_of(&file2), "def");
            write_file(&f.path_of(&file3), "ghi");

            let inputs = NestedSetBuilder::<ActionInputRef>::stable_order()
                .add(file1.clone())
                .add_transitive(
                    &NestedSetBuilder::<ActionInputRef>::stable_order()
                        .add(file2.clone())
                        .add(file3.clone())
                        .build(),
                )
                .build();

            assert_eq!(inputs.get_leaves().len(), 1);
            assert_eq!(inputs.get_non_leaves().len(), 1);

            let mut spawn = default_spawn_builder().with_input_set(&inputs);
            if inputs_mode.is_tool() {
                spawn = spawn.with_tool_set(&inputs);
            }

            let context = f.create_spawn_log_context(T);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[&file1, &file2, &file3]),
                f.create_input_map(&[&file1, &file2, &file3]),
                &f.fs,
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.inputs.push(file(
                "file1",
                Some(f.get_digest("abc")),
                inputs_mode.is_tool(),
            ));
            expected.inputs.push(file(
                "file2",
                Some(f.get_digest("def")),
                inputs_mode.is_tool(),
            ));
            expected.inputs.push(file(
                "file3",
                Some(f.get_digest("ghi")),
                inputs_mode.is_tool(),
            ));
            T.close_and_assert_log(&f, context, vec![expected]);
        }
    }
}

/// A `SymlinkAction.toArtifact(owner, source, target, "Creating symlink")`.
struct TestSymlinkAction {
    input: ActionInputRef,
    output: ActionInputRef,
    label: Label,
}

impl SpawnLogAction for TestSymlinkAction {
    fn get_primary_input(&self) -> Option<ActionInputRef> {
        Some(self.input.clone())
    }
    fn get_primary_output(&self) -> ActionInputRef {
        self.output.clone()
    }
    fn get_owner_label(&self) -> Option<Label> {
        Some(self.label.clone())
    }
    fn get_mnemonic(&self) -> String {
        "Symlink".to_string()
    }
}

#[test]
fn test_symlink_action() {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let source = create_artifact(&f.root_dir, "source");
        let target = create_artifact(&f.root_dir, "target");
        let symlink_action = TestSymlinkAction {
            input: source,
            output: target,
            label: Label::parse_canonical_unchecked("//pkg:symlink"),
        };

        let context = f.create_spawn_log_context(T);
        context.log_symlink_action(&symlink_action).unwrap();

        let entries = close_and_read_compact_log(&f, &*context);
        assert_eq!(
            entries,
            vec![
                ExecLogEntry {
                    id: 0,
                    r#type: Some(exec_log_entry::Type::Invocation(
                        exec_log_entry::Invocation {
                            hash_function_name: "SHA-256".to_string(),
                            workspace_runfiles_directory: WORKSPACE_NAME.to_string(),
                            sibling_repository_layout: sibling,
                            id: "00000000-0000-0000-0000-000000000000".to_string(),
                        }
                    )),
                },
                ExecLogEntry {
                    id: 0,
                    r#type: Some(exec_log_entry::Type::SymlinkAction(
                        exec_log_entry::SymlinkAction {
                            input_path: "source".to_string(),
                            output_path: "target".to_string(),
                            mnemonic: "Symlink".to_string(),
                            target_label: "//pkg:symlink".to_string(),
                        }
                    )),
                },
            ]
        );
    }
}

#[test]
#[ignore = "needs Runfiles, RunfilesSupport.RunfilesTreeImpl and SpawnInputExpander"]
fn test_runfiles_tree_reused_for_tool() {}

#[test]
fn test_unmaterialized_empty_tree_input() {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        let f = Fixture::new(sibling);
        let tree_input = create_tree_artifact_with_generating_action(&f.output_dir, "tree");

        // Deliberately don't create the directory: an empty tree artifact may not be
        // materialized on disk, so its (empty) contents are only known to in-memory metadata.
        assert!(!f.path_of(&tree_input).exists());

        let tree_metadata = TreeArtifactValue::default();
        let mut input_metadata_provider = f.create_input_metadata_provider(&[]);
        // PORT: `put(treeInput, treeMetadata.getMetadata())` has no counterpart: the seam's empty
        // TreeArtifactValue carries no aggregate metadata, and the log never reads it.
        input_metadata_provider.put_tree_artifact(tree_input.clone(), tree_metadata);

        let context = f.create_spawn_log_context(T);

        f.log_spawn(
            &*context,
            &default_spawn_builder().with_inputs(&[&tree_input]).build(),
            &input_metadata_provider,
            // SpawnInputExpander keeps empty tree artifacts in the input map.
            f.create_input_map(&[&tree_input]),
            &f.fs,
            default_timeout(),
            &default_spawn_result(),
        );

        T.close_and_assert_log(&f, context, vec![default_spawn_exec()]);
    }
}

#[test]
fn test_unreadable_outputs() {
    for sibling in SIBLING_REPOSITORY_LAYOUT {
        for outputs_mode in OutputsMode::ALL {
            let f = Fixture::new(sibling);
            let readable_file = create_artifact(&f.output_dir, "readable");
            let unreadable_file = create_artifact(&f.output_dir, "unreadable");
            let unreadable_file_dir =
                create_tree_artifact_with_generating_action(&f.output_dir, "unreadableFileDir");

            write_file(&f.path_of(&readable_file), "xyz");
            // Make the files unreadable.
            write_file(&f.path_of(&unreadable_file), "abc");
            f.path_of(&unreadable_file).set_readable(false).unwrap();
            write_file(&f.path_of(&unreadable_file_dir).get_child("file"), "def");
            f.path_of(&unreadable_file_dir)
                .get_child("file")
                .set_readable(false)
                .unwrap();

            let spawn = default_spawn_builder().with_outputs(&[
                &readable_file,
                &unreadable_file,
                &unreadable_file_dir,
            ]);

            let context = f.create_spawn_log_context(T);

            f.log_spawn(
                &*context,
                &spawn.build(),
                &f.create_input_metadata_provider(&[]),
                f.create_input_map(&[]),
                &f.get_action_file_system(outputs_mode),
                default_timeout(),
                &default_spawn_result(),
            );

            let mut expected = default_spawn_exec();
            expected.actual_outputs.push(file(
                &format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/readable"),
                Some(f.get_digest("xyz")),
                false,
            ));
            expected
                .listed_outputs
                .push(format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/readable"));
            expected
                .listed_outputs
                .push(format!("{PRODUCT_NAME}-out/k8-fastbuild/bin/unreadable"));
            expected.listed_outputs.push(format!(
                "{PRODUCT_NAME}-out/k8-fastbuild/bin/unreadableFileDir"
            ));
            T.close_and_assert_log(&f, context, vec![expected]);

            let events = f.stored_event_handler.get_events();
            assert!(events.iter().any(|e| e.get_message().contains(
                "The compact execution log is incomplete because some outputs could not be read. \
                 Refer to the server log file for details."
            )));
            assert_eq!(events.len(), 1);
            assert!(f.stored_event_handler.get_posts().is_empty());
        }
    }
}

// ---- The tests of SpawnLogContextTestBase. ----

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
