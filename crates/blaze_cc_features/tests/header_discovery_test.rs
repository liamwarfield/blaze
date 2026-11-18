//! Port of `src/test/java/com/google/devtools/build/lib/rules/cpp/HeaderDiscoveryTest.java`.
//!
//! PORT: Mockito's `ArtifactResolver` mock becomes [`MockResolver`] (canned answers and call
//! counters); `ActionsTestUtil.NullAction` with a platform becomes [`TestAction`] with the
//! platform's OS; the in-memory file system becomes path objects on the Unix one (no I/O happens).

mod common;

use std::cell::Cell;
use std::sync::Arc;

use blaze_cc_features::actions::{
    ActionExecutionException, ArtifactRef, ArtifactResolver, NOOP, OS,
};
use blaze_cc_features::header_discovery::{DiscoveringAction, discover_inputs_from_dependencies};
use blaze_cmdline::RepositoryName;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use common::TestArtifact;

const DERIVED_SEGMENT: &str = "derived";

struct Fixture {
    fs: FileSystemRef,
    exec_root: Path,
    derived_root: Path,
}

impl Fixture {
    fn new() -> Fixture {
        let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
        let exec_root = Path::create_from_str("/execroot", &fs);
        let derived_root = exec_root.get_child(DERIVED_SEGMENT);
        Fixture {
            fs,
            exec_root,
            derived_root,
        }
    }

    fn path(&self, p: &str) -> Path {
        Path::create_from_str(p, &self.fs)
    }

    /// `ActionsTestUtil.createArtifact(derivedArtifactRoot, path)`.
    fn derived_artifact(&self, path: &Path) -> ArtifactRef {
        TestArtifact::derived(path.relative_to(&self.exec_root).get_path_string())
    }

    fn tree_artifact(&self, path: &Path) -> ArtifactRef {
        TestArtifact::tree(path.relative_to(&self.exec_root).get_path_string())
    }

    fn source_artifact(&self, exec_path: &str) -> ArtifactRef {
        TestArtifact::source(exec_path)
    }

    fn foo_cc(&self) -> ArtifactRef {
        self.derived_artifact(&self.derived_root.get_relative_str("foo.cc"))
    }
}

#[derive(Default)]
struct MockResolver {
    case_insensitive: Vec<(Option<PathFragment>, Vec<ArtifactRef>)>,
    exact: Vec<(PathFragment, ArtifactRef)>,
    resolve_source_artifact_calls: Cell<usize>,
    case_insensitive_calls: Cell<usize>,
}

impl ArtifactResolver for MockResolver {
    fn resolve_source_artifact(
        &self,
        exec_path: &PathFragment,
        repository_name: &RepositoryName,
    ) -> Option<ArtifactRef> {
        self.resolve_source_artifact_calls
            .set(self.resolve_source_artifact_calls.get() + 1);
        assert_eq!(repository_name.get_name(), "");
        self.exact
            .iter()
            .find(|(p, _)| p == exec_path)
            .map(|(_, a)| a.clone())
    }

    fn resolve_source_artifacts_ascii_case_insensitively(
        &self,
        exec_path: &PathFragment,
        _repository_name: &RepositoryName,
    ) -> Vec<ArtifactRef> {
        self.case_insensitive_calls
            .set(self.case_insensitive_calls.get() + 1);
        for (p, r) in &self.case_insensitive {
            if p.as_ref().is_none_or(|p| p == exec_path) {
                return r.clone();
            }
        }
        Vec::new()
    }

    fn is_derived_artifact(&self, _exec_path: &PathFragment) -> bool {
        false
    }
}

struct TestAction {
    os: OS,
    inputs: NestedSet<ArtifactRef>,
}

impl TestAction {
    fn windows() -> TestAction {
        TestAction {
            os: OS::WINDOWS,
            inputs: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    fn windows_with(inputs: NestedSet<ArtifactRef>) -> TestAction {
        TestAction {
            os: OS::WINDOWS,
            inputs,
        }
    }

    fn non_windows() -> TestAction {
        TestAction {
            os: OS::LINUX,
            inputs: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    /// `new ActionsTestUtil.NullAction()`: no platform, so the host OS.
    fn null_action() -> TestAction {
        TestAction::non_windows()
    }
}

impl DiscoveringAction for TestAction {
    fn get_owner_label(&self) -> String {
        "//pkg:target".to_string()
    }

    fn get_owner_repository_name(&self) -> String {
        String::new()
    }

    fn get_execution_os(&self) -> OS {
        self.os
    }

    fn get_inputs(&self) -> NestedSet<ArtifactRef> {
        self.inputs.clone()
    }
}

fn empty() -> NestedSet<ArtifactRef> {
    NestedSetBuilder::empty_set(Order::STABLE_ORDER)
}

fn discover_inputs(
    f: &Fixture,
    action: &TestAction,
    artifact_resolver: &MockResolver,
    dependencies: &[Path],
    allowed_derived_inputs: &NestedSet<ArtifactRef>,
) -> Result<NestedSet<ArtifactRef>, ActionExecutionException> {
    discover_inputs_from_dependencies(
        action,
        &f.foo_cc(),
        /* should_validate_inclusions= */ true,
        dependencies,
        /* permitted_system_include_prefixes= */ &[],
        allowed_derived_inputs,
        &f.exec_root,
        artifact_resolver,
        /* sibling_repository_layout= */ false,
        &NOOP,
    )
}

fn list(set: &NestedSet<ArtifactRef>) -> Vec<String> {
    set.to_list()
        .iter()
        .map(|a| a.get_exec_path_string())
        .collect()
}

fn paths(v: &[&ArtifactRef]) -> Vec<String> {
    v.iter().map(|a| a.get_exec_path_string()).collect()
}

#[test]
fn errors_when_missing_headers() {
    let f = Fixture::new();
    let artifact_resolver = MockResolver::default();
    let r = discover_inputs_from_dependencies(
        &TestAction::null_action(),
        &f.foo_cc(),
        true,
        &[
            f.derived_root.get_relative_str("tree_artifact1/foo.h"),
            f.derived_root
                .get_relative_str("tree_artifact1/subdir/foo.h"),
        ],
        &[],
        &NestedSetBuilder::create(
            Order::STABLE_ORDER,
            [f.tree_artifact(&f.derived_root.get_relative_str("tree_artifact2"))],
        ),
        &f.exec_root,
        &artifact_resolver,
        false,
        &NOOP,
    );
    let e = r.unwrap_err();
    assert_eq!(
        e.message,
        "undeclared inclusion(s) in rule '//pkg:target':\nthis rule is missing dependency \
         declarations for the following files included by 'derived/foo.cc':\n  \
         'derived/tree_artifact1/foo.h'\n  'derived/tree_artifact1/subdir/foo.h'"
    );
}

#[test]
fn windows_platform_uses_ascii_case_insensitive_resolution() {
    let f = Fixture::new();
    let resolved_artifact = f.source_artifact("pkg/Include/Header.h");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(
            Some(PathFragment::create("pkg/include/header.h")),
            vec![resolved_artifact.clone()],
        )],
        ..Default::default()
    };
    let result = discover_inputs(
        &f,
        &TestAction::windows(),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/include/header.h")],
        &empty(),
    )
    .unwrap();
    assert_eq!(list(&result), paths(&[&resolved_artifact]));
    assert_eq!(artifact_resolver.resolve_source_artifact_calls.get(), 0);
}

#[test]
fn non_windows_platform_uses_exact_case_resolution() {
    let f = Fixture::new();
    let resolved_artifact = f.source_artifact("pkg/header.h");
    let artifact_resolver = MockResolver {
        exact: vec![(
            PathFragment::create("pkg/header.h"),
            resolved_artifact.clone(),
        )],
        ..Default::default()
    };
    let result = discover_inputs(
        &f,
        &TestAction::non_windows(),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/header.h")],
        &empty(),
    )
    .unwrap();
    assert_eq!(list(&result), paths(&[&resolved_artifact]));
    assert_eq!(artifact_resolver.case_insensitive_calls.get(), 0);
}

#[test]
fn windows_platform_single_case_insensitive_match_adds_to_inputs() {
    let f = Fixture::new();
    let resolved_artifact = f.source_artifact("pkg/Include/BaseTsd.h");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(None, vec![resolved_artifact.clone()])],
        ..Default::default()
    };
    let result = discover_inputs(
        &f,
        &TestAction::windows(),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/include/basetsd.h")],
        &empty(),
    )
    .unwrap();
    assert_eq!(list(&result), paths(&[&resolved_artifact]));
}

#[test]
fn windows_platform_multiple_case_insensitive_matches_prefers_action_inputs() {
    let f = Fixture::new();
    let declared_input = f.source_artifact("pkg/Include/Header.h");
    let other_variant = f.source_artifact("pkg/include/header.h");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(None, vec![declared_input.clone(), other_variant.clone()])],
        ..Default::default()
    };
    // The action has declaredInput in its inputs.
    let action_inputs = NestedSetBuilder::create(Order::STABLE_ORDER, [declared_input.clone()]);
    let result = discover_inputs(
        &f,
        &TestAction::windows_with(action_inputs),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/include/header.h")],
        &empty(),
    )
    .unwrap();
    // Only the variant that matches the declared input should be used.
    assert_eq!(list(&result), paths(&[&declared_input]));
}

#[test]
fn windows_platform_multiple_case_insensitive_matches_no_action_input_match_adds_all() {
    let f = Fixture::new();
    let variant1 = f.source_artifact("pkg/Include/Header.h");
    let variant2 = f.source_artifact("pkg/include/header.h");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(None, vec![variant1.clone(), variant2.clone()])],
        ..Default::default()
    };
    // No matching source artifacts in the action inputs.
    let result = discover_inputs(
        &f,
        &TestAction::windows(),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/INCLUDE/HEADER.H")],
        &empty(),
    )
    .unwrap();
    // When none of the matches are in the action inputs, all variants are added.
    let mut actual = list(&result);
    actual.sort();
    let mut expected = paths(&[&variant1, &variant2]);
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn windows_platform_absolute_system_include_matches_case_insensitively() {
    let f = Fixture::new();
    let artifact_resolver = MockResolver::default();
    let system_include_dir = f.path("/C/Program Files/MSVC/include");
    // Compiler reports the path with different casing than the toolchain lists it.
    let dep = f.path("/c/program files/msvc/include/windows.h");

    // Should not throw — the absolute path should be filtered out as a system include.
    let result = discover_inputs_from_dependencies(
        &TestAction::windows(),
        &f.foo_cc(),
        true,
        &[dep],
        &[system_include_dir],
        &empty(),
        &f.exec_root,
        &artifact_resolver,
        false,
        &NOOP,
    )
    .unwrap();
    assert!(result.to_list().is_empty());
}

#[test]
fn windows_platform_absolute_system_include_exact_case_also_matches() {
    let f = Fixture::new();
    let artifact_resolver = MockResolver::default();
    let system_include_dir = f.path("/C/Program Files/MSVC/include");
    let dep = f.path("/C/Program Files/MSVC/include/windows.h");
    let result = discover_inputs_from_dependencies(
        &TestAction::windows(),
        &f.foo_cc(),
        true,
        &[dep],
        &[system_include_dir],
        &empty(),
        &f.exec_root,
        &artifact_resolver,
        false,
        &NOOP,
    )
    .unwrap();
    assert!(result.to_list().is_empty());
}

#[test]
fn windows_platform_source_file_filtered_out() {
    let f = Fixture::new();
    let source_file = f.source_artifact("pkg/foo.cc");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(None, vec![source_file.clone()])],
        ..Default::default()
    };
    let result = discover_inputs_from_dependencies(
        &TestAction::windows(),
        &source_file,
        true,
        &[f.exec_root.get_relative_str("pkg/foo.cc")],
        &[],
        &empty(),
        &f.exec_root,
        &artifact_resolver,
        false,
        &NOOP,
    )
    .unwrap();
    // The source file itself should be filtered out as it's a mandatory input.
    assert!(result.to_list().is_empty());
}

#[test]
fn windows_platform_derived_artifact_matched_exactly() {
    let f = Fixture::new();
    let artifact_resolver = MockResolver::default();
    let derived_artifact = f.derived_artifact(&f.derived_root.get_relative_str("gen/foo.h"));
    let result = discover_inputs(
        &f,
        &TestAction::windows(),
        &artifact_resolver,
        &[f.derived_root.get_relative_str("gen/foo.h")],
        &NestedSetBuilder::create(Order::STABLE_ORDER, [derived_artifact.clone()]),
    )
    .unwrap();
    // Derived artifacts should be matched by exact path, not going through case-insensitive
    // resolution.
    assert_eq!(list(&result), paths(&[&derived_artifact]));
    assert_eq!(artifact_resolver.case_insensitive_calls.get(), 0);
    assert_eq!(artifact_resolver.resolve_source_artifact_calls.get(), 0);
}

#[test]
fn windows_platform_unresolved_source_path_errors() {
    let f = Fixture::new();
    let artifact_resolver = MockResolver::default();
    assert!(
        discover_inputs(
            &f,
            &TestAction::windows(),
            &artifact_resolver,
            &[f.exec_root.get_relative_str("pkg/missing.h")],
            &empty(),
        )
        .is_err()
    );
}

#[test]
fn windows_platform_multiple_deps_mixed_resolution() {
    let f = Fixture::new();
    let source_header = f.source_artifact("pkg/source.h");
    let derived_header = f.derived_artifact(&f.derived_root.get_relative_str("gen/gen.h"));
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(
            Some(PathFragment::create("pkg/source.h")),
            vec![source_header.clone()],
        )],
        ..Default::default()
    };
    let result = discover_inputs(
        &f,
        &TestAction::windows(),
        &artifact_resolver,
        &[
            f.exec_root.get_relative_str("pkg/source.h"),
            f.derived_root.get_relative_str("gen/gen.h"),
        ],
        &NestedSetBuilder::create(Order::STABLE_ORDER, [derived_header.clone()]),
    )
    .unwrap();
    let mut actual = list(&result);
    actual.sort();
    let mut expected = paths(&[&source_header, &derived_header]);
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn windows_platform_multiple_case_insensitive_matches_only_one_in_action_inputs() {
    let f = Fixture::new();
    let variant1 = f.source_artifact("pkg/Header.h");
    let variant2 = f.source_artifact("pkg/header.h");
    let variant3 = f.source_artifact("pkg/HEADER.h");
    let artifact_resolver = MockResolver {
        case_insensitive: vec![(None, vec![variant1, variant2.clone(), variant3])],
        ..Default::default()
    };
    let action_inputs = NestedSetBuilder::create(Order::STABLE_ORDER, [variant2.clone()]);
    let result = discover_inputs(
        &f,
        &TestAction::windows_with(action_inputs),
        &artifact_resolver,
        &[f.exec_root.get_relative_str("pkg/HEADER.h")],
        &empty(),
    )
    .unwrap();
    // Only variant2 is in the action inputs, so only it should be selected.
    assert_eq!(list(&result), paths(&[&variant2]));
}
