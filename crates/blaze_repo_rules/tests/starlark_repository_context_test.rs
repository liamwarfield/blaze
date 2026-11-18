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

#[test]
#[ignore = "needs a harness running repository_ctx methods with an ExternalContext over a Skyframe environment"]
fn test_remote_exec() {}

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
