//! Port of `src/test/java/com/google/devtools/build/lib/analysis/RunfilesRepoMappingManifestTest.java`.
//!
//! Every test of the Java class runs under `BuildViewTestCase` with a Bazel module registry
//! (`bare_rule@1.0`), MODULE.bazel resolution and Starlark rule analysis, and inspects the
//! `RepoMappingManifestAction` that analysis registers. Blaze has no analysis layer yet, so the
//! tests are ignored stubs.

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry (bzlmod + Starlark rules)"]
fn diamond() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry and toolchain resolution"]
fn runfiles_from_toolchain() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry (bzlmod + Starlark rules)"]
fn action_reruns_on_repo_mapping_change_repo_name() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry (bzlmod + Starlark rules)"]
fn action_reruns_on_repo_mapping_change_new_entry() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry (bzlmod + Starlark rules)"]
fn has_mapping_for_symlinks() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with a module registry and FilesToRunProvider"]
fn repo_mapping_on_files_to_run_provider() {}

/// Java is parameterized over `compactRepoMapping` (true and false).
#[test]
#[ignore = "needs BuildViewTestCase analysis with module extensions (compactRepoMapping=true)"]
fn runfiles_from_extension_compact_repo_mapping_true() {}

#[test]
#[ignore = "needs BuildViewTestCase analysis with module extensions (compactRepoMapping=false)"]
fn runfiles_from_extension_compact_repo_mapping_false() {}
