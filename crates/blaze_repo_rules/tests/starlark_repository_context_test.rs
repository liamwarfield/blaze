//! Tests for `repository_ctx`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContextTest.java.
//! The Java tests build a `StarlarkRepositoryContext` over a mocked Skyframe `Environment` and
//! call its methods from Starlark. Not ported yet: a test harness that runs a repository rule
//! implementation with an `ExternalContext` payload (needs `RepositoryFetchFunction`'s setup).
//! `test_attr` runs a real repository rule in a scratch workspace instead (`common::Scratch`).

mod common;

use common::Scratch;
use common::run;

/// Java: `context.getAttr().getFieldNames()` contains "foo" and `getValue("foo")` is "bar" for a
/// repo `test` of a rule with the string attribute `foo`, instantiated with `foo = "bar"`.
/// Here the rule's implementation reads `dir(rctx.attr)` and `rctx.attr.foo`.
// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContextTest.java:231 StarlarkRepositoryContextTest.testAttr
#[test]
fn test_attr() {
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "my_rule = use_repo_rule('//:rule.bzl', 'my_rule')\n\
             my_rule(name = 'test', foo = 'bar')\n",
        );
        s.file("BUILD", "");
        s.file(
            "rule.bzl",
            "def _impl(rctx):\n\
             \x20 rctx.file('BUILD', '')\n\
             \x20 rctx.file('attr.txt', repr(['foo' in dir(rctx.attr), rctx.attr.foo]))\n\
             my_rule = repository_rule(implementation = _impl, attrs = {'foo': attr.string()})\n",
        );
        let ev = s.evaluator(&[]);
        let dir = s
            .fetch(&ev, "+my_rule+test")
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(s.read(&dir.join("attr.txt")), "[True, \"bar\"]");
    })
}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_which() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_file() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_delete() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_read() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_patch() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_patch_in_directory() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_patch_in_directory_outside_of_external_repository() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_cannot_find_file_to_patch() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_patch_outside_of_external_repository() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_patch_error_was_thrown() {}

/// Recording stand-in for Java's `Mockito.mock(RepositoryRemoteExecutor.class)`.
#[derive(Default)]
struct RecordingRemoteExecutor {
    calls: std::sync::Mutex<Vec<String>>,
}

impl blaze_repo_rules::repository::remote_helpers::RepositoryRemoteExecutor
    for RecordingRemoteExecutor
{
    fn execute(
        &self,
        arguments: &[String],
        input_files: &std::collections::BTreeMap<blaze_vfs::PathFragment, blaze_vfs::Path>,
        execution_properties: &indexmap::IndexMap<String, String>,
        environment: &indexmap::IndexMap<String, String>,
        working_directory: Option<&str>,
        timeout: std::time::Duration,
    ) -> std::io::Result<blaze_repo_rules::repository::remote_helpers::ExecutionResult> {
        self.calls.lock().unwrap().push(format!(
            "{arguments:?} {:?} {execution_properties:?} {environment:?} {working_directory:?} {timeout:?}",
            input_files.keys().collect::<Vec<_>>()
        ));
        Ok(
            blaze_repo_rules::repository::remote_helpers::ExecutionResult::new(
                0,
                b"test-stdout".to_vec(),
                b"test-stderr".to_vec(),
            ),
        )
    }
}

struct RecordingFactory(std::sync::Arc<RecordingRemoteExecutor>);

impl blaze_repo_rules::repository::remote_helpers::RepositoryRemoteHelpersFactory
    for RecordingFactory
{
    fn create_executor(
        &self,
    ) -> Option<
        std::sync::Arc<dyn blaze_repo_rules::repository::remote_helpers::RepositoryRemoteExecutor>,
    > {
        Some(self.0.clone())
    }

    fn create_repo_contents_cache(
        &self,
    ) -> Option<
        std::sync::Arc<dyn blaze_repo_rules::repository::remote_helpers::RemoteRepoContentsCache>,
    > {
        None
    }
}

/// Java: `context.execute(["/bin/cmd", "arg1"], timeout = 10, quiet = True)` in a remotable rule
/// with `exec_properties = {"OSFamily": "Linux"}` and `--experimental_repo_remote_exec` goes to
/// the remote executor with these arguments, no input files, the execution properties, an empty
/// environment, the working directory "" and 10 seconds, and returns its result. Here a real
/// repository rule runs, with a recording executor registered as the runtime's factory.
// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContextTest.java:488 StarlarkRepositoryContextTest.testRemoteExec
#[test]
fn test_remote_exec() {
    use blaze_repo_rules::repository::remote_helpers::{
        init_repository_remote_helpers_factory, reset_repository_remote_helpers_factory,
    };
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "configure = use_repo_rule('//:rule.bzl', 'configure')\n\
             configure(name = 'configure', exec_properties = {'OSFamily': 'Linux'})\n",
        );
        s.file("BUILD", "");
        s.file(
            "rule.bzl",
            "def _impl(rctx):\n\
             \x20 r = rctx.execute(['/bin/cmd', 'arg1'], timeout = 10, quiet = True)\n\
             \x20 rctx.file('BUILD', '')\n\
             \x20 rctx.file('result.txt', repr([r.return_code, r.stdout, r.stderr]))\n\
             configure = repository_rule(implementation = _impl, remotable = True)\n",
        );
        let executor = std::sync::Arc::new(RecordingRemoteExecutor::default());
        init_repository_remote_helpers_factory(std::sync::Arc::new(RecordingFactory(
            executor.clone(),
        )));
        let semantics = blaze_starlark::StarlarkSemantics {
            experimental_repo_remote_exec: true,
            ..blaze_starlark::StarlarkSemantics::default()
        };
        let ev =
            s.evaluator_with_semantics(&[], Default::default(), Scratch::default_env(), semantics);
        reset_repository_remote_helpers_factory();
        let dir = s
            .fetch(&ev, "+configure+configure")
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(
            *executor.calls.lock().unwrap(),
            vec![
                "[\"/bin/cmd\", \"arg1\"] [] {\"OSFamily\": \"Linux\"} {} Some(\"\") 10s"
                    .to_string()
            ]
        );
        assert_eq!(
            s.read(&dir.join("result.txt")),
            "[0, \"test-stdout\", \"test-stderr\"]"
        );
    })
}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_rename() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_rename_conflict() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_symlink() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_directory_listing() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_workspace_root() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_no_incompatible_no_implicit_watch_label() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_incompatible_no_implicit_watch_label() {}

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_recorded_file_input_is_locked_in_on_first_access() {}
