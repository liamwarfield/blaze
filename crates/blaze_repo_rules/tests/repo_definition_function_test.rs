//! Port of RepoDefinitionFunctionTest (src/test/java/com/google/devtools/build/lib/bazel/
//! repository/RepoDefinitionFunctionTest.java).
//!
//! The Java tests extend `FoundationTestCase` with a fake registry and evaluate REPO_DEFINITION
//! over the full Bzlmod resolution (module files, registries, BAZEL_DEP_GRAPH) and BZL_LOAD of
//! `@bazel_tools`' repo rules; they wait for a shared Bzlmod + loading fixture.

#[test]
#[ignore = "needs a Bzlmod + BZL_LOAD fixture with a fake registry"]
fn test_repo_spec_bazel_module() {}

#[test]
#[ignore = "needs a Bzlmod + BZL_LOAD fixture with a fake registry"]
fn test_repo_spec_non_registry_override() {}

#[test]
#[ignore = "needs a Bzlmod + BZL_LOAD fixture with a fake registry"]
fn test_repo_spec_single_version_override() {}

#[test]
#[ignore = "needs a Bzlmod + BZL_LOAD fixture with a fake registry"]
fn test_repo_spec_multiple_version_override() {}

#[test]
#[ignore = "needs a Bzlmod + BZL_LOAD fixture with a fake registry"]
fn test_repo_spec_not_found() {}
