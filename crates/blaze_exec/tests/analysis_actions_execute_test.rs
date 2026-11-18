//! Execution parts of the `analysis/actions` JUnit tests: `FileWriteActionTest` (via
//! `FileWriteActionTestCase`), `AbstractFileWriteActionTest`, `TemplateExpansionActionTest`,
//! `SymlinkActionTest` and `SpawnActionTest`, plus execution of the other `blaze_actions` action
//! classes (`UnresolvedSymlinkAction`, `SolibSymlinkAction`, `RunfilesTreeAction`,
//! `BazelWorkspaceStatusAction`, `ParameterFileWriteAction`).
//!
//! The analysis-only parts of these Java tests are ported in `blaze_actions/tests`.

#![allow(clippy::result_large_err)]

mod analysis_actions_common;

use analysis_actions_common::{TestDirs, null_action_owner, read, write};
use blaze_actions::Action;
use blaze_exec::actions::{ExecutableAction, as_executable};

fn execute(
    ctx: &analysis_actions_common::TestContext,
    action: &dyn Action,
) -> Result<blaze_exec::actions::ActionResult, blaze_exec::actions::ActionExecutionException> {
    let executable: &dyn ExecutableAction =
        as_executable(action).expect("action class has a registered ExecutableAction");
    executable.execute(ctx)
}

/// `FileWriteActionTest` / `FileWriteActionTestCase` (execution parts).
mod file_write_action_test {
    use super::*;
    use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};

    // Java: FileWriteActionTest.createAction
    fn create_action(
        output_artifact: blaze_actions::Artifact,
        data: &str,
        make_executable: bool,
    ) -> FileWriteAction {
        FileWriteAction::create(
            null_action_owner(),
            output_artifact,
            data,
            make_executable,
            Compression::DISALLOW,
        )
    }

    // Java: FileWriteActionTestCase.checkCanWriteNonExecutableFile
    #[test]
    fn test_can_write_non_executable_file() {
        let dirs = TestDirs::new();
        let output_artifact = dirs.bin_artifact("destination.txt");
        let output = output_artifact.get_path();
        output
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let action = create_action(output_artifact, "Hello World", false);
        let ctx = dirs.context();
        let action_result = execute(&ctx, &action).unwrap();
        assert!(action_result.spawn_results().is_empty());
        assert_eq!(read(&output), b"Hello World");
        assert!(!output.is_executable().unwrap());
    }

    // Java: FileWriteActionTestCase.checkCanWriteExecutableFile
    #[test]
    fn test_can_write_executable_file() {
        let dirs = TestDirs::new();
        let output_artifact = dirs.bin_artifact("hello");
        let output = output_artifact.get_path();
        output
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let action = create_action(output_artifact, "echo 'Hello World'", true);
        let ctx = dirs.context();
        let action_result = execute(&ctx, &action).unwrap();
        assert!(action_result.spawn_results().is_empty());
        assert_eq!(read(&output), b"echo 'Hello World'");
        assert!(output.is_executable().unwrap());
    }

    /// `FileWriteActionTest.testFileWriteActionWithLongStringAndCompression`, executed: the
    /// compressed action writes the original bytes.
    #[test]
    fn test_file_write_action_with_long_string_and_compression_executes() {
        let dirs = TestDirs::new();
        let output_artifact = dirs.bin_artifact("destination.txt");
        let output = output_artifact.get_path();
        output
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let contents: String = (0..100_000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let action = FileWriteAction::create(
            null_action_owner(),
            output_artifact,
            contents.clone(),
            false,
            Compression::ALLOW,
        );
        assert!(action.is_compressed());
        let ctx = dirs.context();
        execute(&ctx, &action).unwrap();
        assert_eq!(read(&output), contents.as_bytes());
    }

    /// A write failure becomes `EnvironmentalExecException(e, FILE_WRITE_IO_EXCEPTION)` and then
    /// `ActionExecutionException.fromExecException`.
    #[test]
    fn write_failure_is_file_write_io_exception() {
        let dirs = TestDirs::new();
        // The parent directory is missing, so opening the output fails.
        let output_artifact = dirs.bin_artifact("missing/destination.txt");
        let action = create_action(output_artifact, "x", false);
        let ctx = dirs.context();
        let e = execute(&ctx, &action).unwrap_err();
        assert!(
            e.get_message().starts_with("java.io.IOException: "),
            "{}",
            e.get_message()
        );
        let detail = e.get_failure_detail().unwrap();
        assert_eq!(
            detail.message,
            format!("{} failed: {}", action.describe(), e.get_message())
        );
        assert!(matches!(
            detail.category,
            Some(blaze_protos::failure_details::failure_detail::Category::Execution(ref x))
                if x.code == blaze_protos::failure_details::execution::Code::FileWriteIoException as i32
        ));
    }
}

/// `AbstractFileWriteActionTest`.
mod abstract_file_write_action_test {
    #[test]
    #[ignore = "needs an AbstractFileWriteAction subclass overriding afterWrite (no ported subclass does)"]
    fn execute_action_successful_write_calls_after_write() {}

    #[test]
    #[ignore = "needs an AbstractFileWriteAction subclass overriding afterWrite (no ported subclass does)"]
    fn execute_action_failed_write_does_not_call_after_write() {}
}

/// `TemplateExpansionActionTest` (execution parts).
mod template_expansion_action_test {
    use super::*;
    use blaze_actions::analysis::actions::substitution::Substitution;
    use blaze_actions::analysis::actions::template::Template;
    use blaze_actions::analysis::actions::template_expansion_action::TemplateExpansionAction;

    const TEMPLATE: &str = "key=%key%\nvalue=%value%";
    const SPECIAL_CHARS: &str = "Š©±½_strøget";

    struct Fixture {
        dirs: TestDirs,
        input_artifact: blaze_actions::Artifact,
        output_artifact: blaze_actions::Artifact,
        substitutions: Vec<Substitution>,
    }

    // Java: TemplateExpansionActionTest.createDirectoriesAndTools / createArtifacts
    fn fixture(template: &str) -> Fixture {
        let dirs = TestDirs::new();
        // scratch.overwriteFile appends a newline.
        let input_artifact = dirs.source_artifact("input.txt");
        write(&input_artifact.get_path(), &format!("{template}\n"));
        let output_artifact = dirs.bin_artifact("destination.txt");
        output_artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        Fixture {
            dirs,
            input_artifact,
            output_artifact,
            substitutions: vec![
                Substitution::of("%key%", "foo"),
                Substitution::of("%value%", "bar"),
            ],
        }
    }

    #[test]
    fn test_expansion() {
        let f = fixture(TEMPLATE);
        let action = TemplateExpansionAction::new(
            null_action_owner(),
            f.output_artifact.clone(),
            Template::for_string(TEMPLATE),
            f.substitutions.clone(),
            false,
        );
        let ctx = f.dirs.context();
        execute(&ctx, &action).unwrap();
        assert_eq!(read(&f.output_artifact.get_path()), b"key=foo\nvalue=bar");
    }

    // Java: TemplateExpansionActionTest.executeTemplateExpansion
    fn execute_template_expansion(f: &Fixture, expected: &str, substitutions: Vec<Substitution>) {
        let action = TemplateExpansionAction::for_template_artifact(
            null_action_owner(),
            f.input_artifact.clone(),
            f.output_artifact.clone(),
            substitutions,
            false,
        );
        let ctx = f.dirs.context();
        execute(&ctx, &action).unwrap();
        let actual = String::from_utf8(read(&f.output_artifact.get_path())).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_artifact_template_expansion() {
        let f = fixture(TEMPLATE);
        // The trailing "" is needed because scratch.overwriteFile implicitly appends "\n".
        execute_template_expansion(&f, "key=foo\nvalue=bar\n", f.substitutions.clone());
    }

    #[test]
    fn test_with_special_characters() {
        // We have to overwrite the artifacts since we need our template in "inputs"
        let f = fixture(&format!("{SPECIAL_CHARS}%key%"));
        // scratch.overwriteFile appends a newline, so we need an additional \n here
        let expected = format!("{SPECIAL_CHARS}{SPECIAL_CHARS}\n");
        // PORT: StringEncoding.unicodeToInternal is the identity on Rust strings.
        execute_template_expansion(
            &f,
            &expected,
            vec![Substitution::of("%key%", SPECIAL_CHARS)],
        );
    }

    /// A missing template artifact fails with `EnvironmentalExecException(e,
    /// LOCAL_TEMPLATE_EXPANSION_FAILURE)`, whose message is `"" + ": " + e.getMessage()`.
    #[test]
    fn missing_template_is_local_template_expansion_failure() {
        let f = fixture(TEMPLATE);
        std::fs::remove_file(f.input_artifact.get_path().get_path_string()).unwrap();
        let action = TemplateExpansionAction::for_template_artifact(
            null_action_owner(),
            f.input_artifact.clone(),
            f.output_artifact.clone(),
            f.substitutions.clone(),
            false,
        );
        let ctx = f.dirs.context();
        let e = execute(&ctx, &action).unwrap_err();
        assert!(
            e.get_message().starts_with(&format!(
                ": failed to load template file '{}' due to I/O error: ",
                f.input_artifact.get_path().get_path_string()
            )),
            "{}",
            e.get_message()
        );
    }
}

/// `SymlinkActionTest` (execution parts).
mod symlink_action_test {
    use super::*;
    use blaze_actions::SpecialArtifactType;
    use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
    use blaze_vfs::PathFragment;

    // Java: SymlinkActionTest.runSymlinkAction + execute
    fn run_symlink_action(dirs: &TestDirs, action: SymlinkAction) {
        assert_eq!(
            action.get_progress_message(None).as_deref(),
            Some("Test symlink action")
        );
        let ctx = dirs.context();
        for input in action.get_inputs().to_list().iter() {
            ctx.metadata.put_for_testing(input);
        }
        let action_result = execute(&ctx, &action).unwrap();
        assert!(action_result.spawn_results().is_empty());
    }

    fn symlink_to_source(directory: bool, use_exec_root_for_source: bool) {
        let dirs = TestDirs::new();
        let input_artifact = dirs.source_artifact("input");
        let output_artifact = dirs.bin_artifact("output");

        let input_path = dirs.exec_root.get_relative_str("input");
        if directory {
            input_path.create_directory_and_parents().unwrap();
        } else {
            write(&input_path, "");
        }
        input_artifact
            .get_path()
            .create_symbolic_link(&input_path)
            .unwrap();
        output_artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();

        let action = SymlinkAction::to_artifact_with_exec_root(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
            use_exec_root_for_source,
        );
        assert_eq!(
            action.get_inputs().to_list().as_slice(),
            std::slice::from_ref(&input_artifact)
        );
        assert_eq!(action.get_outputs(), vec![output_artifact.clone()]);
        run_symlink_action(&dirs, action);

        let expected_target = if use_exec_root_for_source {
            dirs.exec_root
                .get_relative(input_artifact.get_exec_path())
                .as_fragment()
                .clone()
        } else {
            input_artifact.get_path().as_fragment().clone()
        };
        assert!(output_artifact.get_path().is_symbolic_link());
        assert_eq!(
            output_artifact.get_path().read_symbolic_link().unwrap(),
            expected_target
        );
        // PORT: the SymlinkTargetType verification (verify(fs).createSymbolicLink(..., FILE)) is
        // Windows-only behaviour and not observable on Linux.
    }

    #[test]
    fn test_symlink_to_source_file_use_exec_root_for_source_false() {
        symlink_to_source(false, false);
    }

    #[test]
    fn test_symlink_to_source_file_use_exec_root_for_source_true() {
        symlink_to_source(false, true);
    }

    #[test]
    fn test_symlink_to_source_directory_use_exec_root_for_source_false() {
        symlink_to_source(true, false);
    }

    #[test]
    fn test_symlink_to_source_directory_use_exec_root_for_source_true() {
        symlink_to_source(true, true);
    }

    #[test]
    fn test_symlink_to_output_file() {
        let dirs = TestDirs::new();
        let input_artifact = dirs.bin_artifact("input");
        let output_artifact = dirs.bin_artifact("output");
        write(&input_artifact.get_path(), "hello world");

        let action = SymlinkAction::to_artifact(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
        );
        run_symlink_action(&dirs, action);

        assert!(output_artifact.get_path().is_symbolic_link());
        assert_eq!(
            &output_artifact.get_path().read_symbolic_link().unwrap(),
            input_artifact.get_path().as_fragment()
        );
    }

    #[test]
    fn test_symlink_to_output_tree() {
        let dirs = TestDirs::new();
        let input_artifact = dirs.special_artifact("input", SpecialArtifactType::TREE);
        let output_artifact = dirs.special_artifact("output", SpecialArtifactType::TREE);
        input_artifact
            .get_path()
            .create_directory_and_parents()
            .unwrap();
        // The output directory created prior to execution is deleted by the action.
        output_artifact
            .get_path()
            .create_directory_and_parents()
            .unwrap();

        let action = SymlinkAction::to_artifact(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
        );
        run_symlink_action(&dirs, action);

        assert!(output_artifact.get_path().is_symbolic_link());
        assert_eq!(
            &output_artifact.get_path().read_symbolic_link().unwrap(),
            input_artifact.get_path().as_fragment()
        );
    }

    #[test]
    fn test_symlink_to_absolute_path() {
        let dirs = TestDirs::new();
        let output_artifact = dirs.bin_artifact("output");
        output_artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();

        let action = SymlinkAction::to_absolute_path(
            null_action_owner(),
            PathFragment::create("/some/path"),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
        );
        assert!(action.get_inputs().is_empty());
        run_symlink_action(&dirs, action);

        assert!(output_artifact.get_path().is_symbolic_link());
        assert_eq!(
            output_artifact.get_path().read_symbolic_link().unwrap(),
            PathFragment::create("/some/path")
        );
    }

    #[test]
    #[ignore = "needs the Skyframe serialization framework (SerializationTester)"]
    fn test_codec() {}

    /// `ExecutableSymlink` to a non-executable file fails with Java's message.
    #[test]
    fn executable_symlink_to_non_executable_file_fails() {
        let dirs = TestDirs::new();
        let input_artifact = dirs.bin_artifact("input");
        let output_artifact = dirs.bin_artifact("output");
        write(&input_artifact.get_path(), "hello world");
        input_artifact.get_path().set_executable(false).unwrap();
        let action = SymlinkAction::to_executable(
            null_action_owner(),
            input_artifact.clone(),
            output_artifact.clone(),
            Some("Test symlink action".to_string()),
        );
        let ctx = dirs.context();
        ctx.metadata.put_for_testing(&input_artifact);
        let e = execute(&ctx, &action).unwrap_err();
        assert_eq!(
            e.get_message(),
            format!(
                "failed to create symbolic link '{}': file '{}' is not executable",
                output_artifact.get_exec_path_string(),
                input_artifact.get_exec_path_string()
            )
        );
        input_artifact.get_path().set_executable(true).unwrap();
        execute(&ctx, &action).unwrap();
        assert!(output_artifact.get_path().is_symbolic_link());
    }
}

/// `UnresolvedSymlinkAction`, `SolibSymlinkAction`, `RunfilesTreeAction`.
mod other_symlink_actions {
    use super::*;
    use blaze_actions::SpecialArtifactType;
    use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
    use blaze_actions::analysis::runfiles::RunfilesBuilder;
    use blaze_actions::analysis::runfiles_support::RunfilesTreeImpl;
    use blaze_actions::analysis::starlark::unresolved_symlink_action::{
        SymlinkTargetType, UnresolvedSymlinkAction,
    };
    use blaze_collect::{NestedSetBuilder, Order};
    use blaze_vfs::PathFragment;
    use std::sync::Arc;

    #[test]
    fn unresolved_symlink_action_creates_textual_symlink() {
        let dirs = TestDirs::new();
        let output = dirs.special_artifact("link", SpecialArtifactType::UNRESOLVED_SYMLINK);
        output
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let action = UnresolvedSymlinkAction::create(
            null_action_owner(),
            output.clone(),
            "../some/target",
            SymlinkTargetType::UNSPECIFIED,
            "Creating symlink",
        );
        let ctx = dirs.context();
        execute(&ctx, &action).unwrap();
        assert_eq!(
            output.get_path().read_symbolic_link().unwrap(),
            PathFragment::create("../some/target")
        );
    }

    #[test]
    fn runfiles_tree_action_sets_rich_artifact_data() {
        let dirs = TestDirs::new();
        let file = dirs.bin_artifact("pkg/file");
        let tree_artifact =
            dirs.special_artifact("pkg/bin.runfiles", SpecialArtifactType::RUNFILES);
        let mut b = RunfilesBuilder::new("_main");
        b.add_artifact(file.clone());
        let tree =
            RunfilesTreeImpl::new_for_testing(tree_artifact.get_exec_path().clone(), b.build());
        let action = RunfilesTreeAction::new(
            null_action_owner(),
            Arc::new(tree),
            NestedSetBuilder::create(Order::STABLE_ORDER, [file.clone()]),
            vec![tree_artifact.clone()],
        );
        let ctx = dirs.context();
        // createRunfilesArtifactValue reads the metadata of every input of the action.
        let file_metadata = blaze_exec::actions::FileArtifactValue::create_for_normal_file(
            Some(vec![1, 2, 3]),
            None,
            3,
        );
        ctx.metadata.metadata.lock().insert(
            file.get_exec_path_string().to_string(),
            file_metadata.clone(),
        );
        let result = execute(&ctx, &action).unwrap();
        assert!(result.is_empty());
        let data = ctx.rich_artifact_data.lock().clone().unwrap();
        // RunfilesArtifactValue.forEachFile: the action's inputs with their metadata.
        let mut files = Vec::new();
        data.for_each_file(|artifact, metadata| {
            files.push((artifact.get_exec_path().clone(), metadata.clone()));
        });
        assert_eq!(files, vec![(file.get_exec_path().clone(), file_metadata)]);
        // RunfilesArtifactValue.getMetadata: a runfiles proxy with the digest of the layout and
        // of the inputs.
        assert!(matches!(
            data.get_metadata(),
            blaze_exec::actions::FileArtifactValue::RunfilesProxy { .. }
        ));
        let t = data.get_runfiles_tree();
        assert_eq!(&t.get_exec_path(), tree_artifact.get_exec_path());
        assert_eq!(t.get_workspace_name(), "_main");
        let mapping = t.get_mapping();
        assert_eq!(mapping.len(), 1);
        assert_eq!(mapping[0].0, PathFragment::create("_main/pkg/file"));
        assert_eq!(
            mapping[0].1.as_ref().unwrap().0.get_exec_path(),
            file.get_exec_path()
        );
    }
}

/// `SpawnActionTest` (the parts that read the action's spawn), and SpawnAction execution
/// through a `SpawnStrategyResolver`.
mod spawn_action_test {
    use super::*;
    use std::sync::Arc;

    use blaze_actions::ActionEnvironment;
    use blaze_actions::actions::param_file_info::ParamFileInfo;
    use blaze_actions::actions::parameter_file::ParameterFileType;
    use blaze_actions::actions::path_mapper::OutputPathsMode;
    use blaze_actions::analysis::actions::custom_command_line::CustomCommandLine;
    use blaze_actions::analysis::actions::spawn_action::{
        SpawnAction, SpawnActionBuilder, SpawnActionConfiguration,
    };
    use blaze_exec::actions::spawn_strategy::SpawnStrategy;
    use blaze_exec::actions::{
        ExecException, ResourceSet, SpawnExecException, SpawnResult, SpawnResultBuilder, Status,
    };
    use blaze_exec::analysis::actions::spawn_action::get_spawn_with_env;
    use blaze_exec::exec::action_execution_context::{
        ActionContextRegistry, ActionExecutionContext,
    };
    use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
    use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
    use indexmap::{IndexMap, IndexSet};

    struct TestConfiguration;

    impl SpawnActionConfiguration for TestConfiguration {
        fn get_action_environment(&self) -> Arc<ActionEnvironment> {
            Arc::new(ActionEnvironment::create(IndexMap::new(), IndexSet::new()))
        }
        fn modified_execution_info(
            &self,
            execution_info: &IndexMap<String, String>,
            _mnemonic: &str,
        ) -> IndexMap<String, String> {
            execution_info.clone()
        }
        fn get_output_paths_mode(&self) -> OutputPathsMode {
            OutputPathsMode::OFF
        }
    }

    #[test]
    fn test_builder() {
        let dirs = TestDirs::new();
        let input = dirs.source_artifact("input");
        let output = dirs.bin_artifact("output");
        let mut b = SpawnActionBuilder::new();
        b.add_input(input.clone())
            .add_output(output.clone())
            .set_executable_path(&blaze_vfs::PathFragment::create("/bin/xxx"))
            .set_progress_message("Test");
        let action = b.build(null_action_owner(), &TestConfiguration);
        assert_eq!(action.get_inputs().to_list().as_slice(), &[input]);
        assert_eq!(action.get_outputs(), vec![output]);
        let ctx = dirs.context();
        let spawn = get_spawn_with_env(&action, &ctx, &IndexMap::new(), true).unwrap();
        use blaze_exec::actions::Spawn;
        // AbstractAction.DEFAULT_RESOURCE_SET = createWithRamCpu(250, 1)
        assert_eq!(
            spawn.get_local_resources().unwrap(),
            ResourceSet::create_with_ram_cpu(250.0, 1.0)
        );
        assert_eq!(spawn.get_arguments(), ["/bin/xxx".to_string()]);
        assert_eq!(action.get_progress_message(None).as_deref(), Some("Test"));
    }

    #[test]
    #[ignore = "needs SpawnAction.Builder.setJarExecutable (not ported in blaze_actions)"]
    fn test_builder_with_jar_executable_and_parameter_file() {}

    /// The param-file part of `testBuilderWithJarExecutableAndParameterFile`: the spawn's
    /// arguments reference `<output>-0.params` and its inputs hold the param file.
    #[test]
    fn get_spawn_with_parameter_file() {
        let dirs = TestDirs::new();
        let output = dirs.bin_artifact("output");
        let mut b = SpawnActionBuilder::new();
        b.add_output(output.clone())
            .set_executable_path(&blaze_vfs::PathFragment::create("/bin/java"))
            .add_command_line_with_param_file(
                Arc::new(CustomCommandLine::builder().add("-X").build()),
                // PORT: the Java test's target configuration has a small command line limit; the
                // null owner here is UNLIMITED, so the param file is forced instead.
                Some(
                    ParamFileInfo::builder(ParameterFileType::UNQUOTED)
                        .set_use_always(true)
                        .build(),
                ),
            );
        let action = b.build(null_action_owner(), &TestConfiguration);
        let ctx = dirs.context();
        let spawn = get_spawn_with_env(&action, &ctx, &IndexMap::new(), true).unwrap();
        use blaze_exec::actions::Spawn;
        let param_file_name = format!("{}-0.params", output.get_exec_path_string());
        assert_eq!(
            spawn.get_arguments(),
            ["/bin/java".to_string(), format!("@{param_file_name}")]
        );
        let inputs = spawn.get_input_files().to_list();
        let param_file = inputs
            .iter()
            .find_map(|i| i.0.as_virtual_action_input())
            .expect("param file input");
        let mut bytes = Vec::new();
        param_file.write_to(&mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap().trim(), "-X");
    }

    /// A strategy that fails every spawn like a genrule whose command exits 1.
    struct FailingStrategy;

    impl std::fmt::Display for FailingStrategy {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("failing")
        }
    }

    fn spawn_failure_detail() -> FailureDetail {
        FailureDetail {
            message: "error executing Genrule command".to_string(),
            category: Some(Category::Spawn(failure_details::Spawn {
                code: failure_details::spawn::Code::NonZeroExit as i32,
                ..Default::default()
            })),
        }
    }

    impl SpawnStrategy for FailingStrategy {
        fn exec(
            &self,
            _spawn: &dyn blaze_exec::actions::Spawn,
            _ctx: &dyn ActionExecutionContext,
        ) -> Result<Vec<SpawnResult>, ExecException> {
            let result = SpawnResultBuilder::new()
                .set_status(Status::NonZeroExit)
                .set_exit_code(1)
                .set_failure_detail(spawn_failure_detail())
                .set_runner_name("failing")
                .build();
            Err(SpawnExecException::new(
                "bash failed: error executing Genrule command (from target //:x) /bin/bash -c 'exit 1'"
                    .to_string(),
                result,
                false,
            )
            .into())
        }
        fn can_exec(
            &self,
            _spawn: &dyn blaze_exec::actions::Spawn,
            _registry: &dyn ActionContextRegistry,
        ) -> bool {
            true
        }
        fn get_simple_class_name(&self) -> String {
            "FailingStrategy".to_string()
        }
    }

    fn genrule_action(dirs: &TestDirs) -> SpawnAction {
        let output = dirs.bin_artifact("x.out");
        let mut b = SpawnActionBuilder::new();
        b.add_output(output)
            .set_executable_path(&blaze_vfs::PathFragment::create("/bin/bash"))
            .set_progress_message("Executing genrule //:x")
            .set_mnemonic("Genrule");
        b.build(null_action_owner(), &TestConfiguration)
    }

    /// `SpawnExecException.toActionExecutionException`: the message is the spawn result's detail
    /// message, so the user-visible "<describe> failed: <message>" reads like Bazel's.
    #[test]
    fn spawn_exec_exception_to_action_execution_exception() {
        let dirs = TestDirs::new();
        let action = genrule_action(&dirs);
        let mut ctx = dirs.context();
        let strategy: Arc<dyn SpawnStrategy> = Arc::new(FailingStrategy);
        ctx.spawn_strategy_registry = Some(Arc::new(
            SpawnStrategyRegistry::builder()
                .register_strategy(strategy, &["failing"])
                .build()
                .unwrap(),
        ));
        let e = execute(&ctx, &action).unwrap_err();
        assert_eq!(
            format!("{} failed: {}", action.describe(), e.get_message()),
            "Executing genrule //:x failed: (Exit 1): bash failed: error executing Genrule command (from target //:x) /bin/bash -c 'exit 1'"
        );
        assert_eq!(e.get_failure_detail(), Some(&spawn_failure_detail()));
        assert!(!e.is_catastrophe());
        let root_causes = e.get_root_causes();
        assert_eq!(root_causes.len(), 1);
        assert_eq!(
            root_causes[0].label,
            blaze_cmdline::Label::parse_canonical("//null/action:owner").unwrap()
        );
    }
}

/// `BazelWorkspaceStatusAction` execution.
mod workspace_status_action_test {
    use super::*;
    use std::sync::Arc;

    use blaze_actions::bazel::bazel_workspace_status_module::BazelWorkspaceStatusAction;
    use blaze_exec::analysis::actions::bazel_workspace_status_module::{
        BazelWorkspaceStatusActionContext, BuildInfoEvent, WorkspaceStatusOptions,
        workspace_status_context_entry,
    };
    use blaze_vfs::PathFragment;
    use indexmap::IndexMap;

    #[test]
    fn writes_stable_and_volatile_status() {
        let dirs = TestDirs::new();
        let stable = dirs.bin_artifact("stable-status.txt");
        let volatile = dirs.bin_artifact("volatile-status.txt");
        stable
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let action =
            BazelWorkspaceStatusAction::new(stable.clone(), volatile.clone(), "user", "host");
        let mut client_env = IndexMap::new();
        client_env.insert("SOURCE_DATE_EPOCH".to_string(), "1700000000".to_string());
        let mut ctx = dirs.context();
        ctx.register(workspace_status_context_entry(Arc::new(
            BazelWorkspaceStatusActionContext::new(
                WorkspaceStatusOptions {
                    embed_label: String::new(),
                    workspace_status_command: PathFragment::empty_fragment(),
                },
                client_env,
                dirs.workspace.clone(),
            ),
        )));
        execute(&ctx, &action).unwrap();
        assert_eq!(
            String::from_utf8(read(&stable.get_path())).unwrap(),
            "BUILD_EMBED_LABEL \nBUILD_HOST host\nBUILD_USER user\n"
        );
        assert_eq!(
            String::from_utf8(read(&volatile.get_path())).unwrap(),
            "BUILD_TIMESTAMP 1700000000\nFORMATTED_DATE 2023 Nov 14 22 13 20 Tue\n"
        );
        let posts = ctx.events.get_posts();
        let event = posts
            .iter()
            .find_map(|p| (p.as_ref() as &dyn std::any::Any).downcast_ref::<BuildInfoEvent>())
            .expect("BuildInfoEvent posted");
        assert_eq!(event.get_build_info_map().len(), 5);
    }
}
