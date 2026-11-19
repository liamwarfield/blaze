//! Port of `src/test/java/com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java`
//! (a `BuildViewTestCase`): the constraints, platforms, toolchain types and toolchains that the
//! `skyframe/toolchains` tests (and other platform tests) start from, with the helpers of the
//! Truth subjects those tests use (`ToolchainContextSubject`, `EvaluationResultSubject`).
//!
//! Shared by the test files of this directory; private to the crate's tests.

#![allow(dead_code)]

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use blaze_analysis::platform::{
    ConstraintSettingInfo, ConstraintValueInfo, DeclaredToolchainInfo, PlatformInfo,
    ToolchainTypeInfo,
};
use blaze_analysis::toolchains::{
    ToolchainContext, ToolchainException, ToolchainExceptionKind, ToolchainTypeRequirement,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_cmdline::package_identifier::PackageIdentifier;
use blaze_skyframe::{EvaluationResult, Key, SkyKey};
use blaze_vfs::Path;

use crate::testing::BuildViewTestCase;

/// `Label.parseCanonicalUnchecked`.
pub fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// Java `ToolchainTestCase`: a `BuildViewTestCase` with the fields its `@Before` methods set.
// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:42 ToolchainTestCase
pub struct ToolchainTestCase {
    pub base: BuildViewTestCase,

    pub linux_platform: PlatformInfo,
    pub mac_platform: PlatformInfo,

    pub setting: ConstraintSettingInfo,
    pub defaulted_setting: ConstraintSettingInfo,
    pub linux_constraint: ConstraintValueInfo,
    pub mac_constraint: ConstraintValueInfo,
    pub defaulted_constraint: ConstraintValueInfo,

    pub test_toolchain_type_label: Label,
    pub test_toolchain_type: ToolchainTypeRequirement,
    pub test_toolchain_type_info: ToolchainTypeInfo,

    pub optional_toolchain_type_label: Label,
    pub optional_toolchain_type: ToolchainTypeRequirement,
    pub optional_toolchain_type_info: ToolchainTypeInfo,
}

impl Deref for ToolchainTestCase {
    type Target = BuildViewTestCase;
    fn deref(&self) -> &BuildViewTestCase {
        &self.base
    }
}

impl DerefMut for ToolchainTestCase {
    fn deref_mut(&mut self) -> &mut BuildViewTestCase {
        &mut self.base
    }
}

/// `ToolchainTestCase.formatConstraints`.
// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:100 ToolchainTestCase.formatConstraints
fn format_constraints(constraints: &[&str]) -> String {
    constraints
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ToolchainTestCase {
    /// The `BuildViewTestCase` setup, then `createConstraints` and `createToolchains`.
    pub fn new() -> ToolchainTestCase {
        let mut base = BuildViewTestCase::new();
        let constraints = create_constraints(&mut base);
        let toolchain_types = create_toolchain_types();
        let mut t = ToolchainTestCase {
            base,
            linux_platform: constraints.linux_platform,
            mac_platform: constraints.mac_platform,
            setting: constraints.setting,
            defaulted_setting: constraints.defaulted_setting,
            linux_constraint: constraints.linux_constraint,
            mac_constraint: constraints.mac_constraint,
            defaulted_constraint: constraints.defaulted_constraint,
            test_toolchain_type_label: toolchain_types.0,
            test_toolchain_type: toolchain_types.1,
            test_toolchain_type_info: toolchain_types.2,
            optional_toolchain_type_label: toolchain_types.3,
            optional_toolchain_type: toolchain_types.4,
            optional_toolchain_type_info: toolchain_types.5,
        };
        t.create_toolchains();
        t
    }

    // Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:178 ToolchainTestCase.addToolchain
    pub fn add_toolchain_with_type(
        &mut self,
        package_name: &str,
        toolchain_name: &str,
        toolchain_type: &Label,
        exec_constraints: &[&str],
        target_constraints: &[&str],
        data: &str,
    ) {
        let lines = [
            "load('//toolchain:toolchain_def.bzl', 'test_toolchain')".to_string(),
            "toolchain(".to_string(),
            format!("    name = '{toolchain_name}',"),
            format!("    toolchain_type = '{toolchain_type}',"),
            format!(
                "    exec_compatible_with = [{}],",
                format_constraints(exec_constraints)
            ),
            format!(
                "    target_compatible_with = [{}],",
                format_constraints(target_constraints)
            ),
            format!("    toolchain = ':{toolchain_name}_impl')"),
            "test_toolchain(".to_string(),
            format!("  name='{toolchain_name}_impl',"),
            format!("  data = '{data}')"),
        ];
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.base
            .scratch
            .append_file(&format!("{package_name}/BUILD"), &lines);
    }

    // Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:198 ToolchainTestCase.addToolchain
    pub fn add_toolchain(
        &mut self,
        package_name: &str,
        toolchain_name: &str,
        exec_constraints: &[&str],
        target_constraints: &[&str],
        data: &str,
    ) {
        let toolchain_type = self.test_toolchain_type_label.clone();
        self.add_toolchain_with_type(
            package_name,
            toolchain_name,
            &toolchain_type,
            exec_constraints,
            target_constraints,
            data,
        );
    }

    // Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:216 ToolchainTestCase.addOptionalToolchain
    pub fn add_optional_toolchain(
        &mut self,
        package_name: &str,
        toolchain_name: &str,
        exec_constraints: &[&str],
        target_constraints: &[&str],
        data: &str,
    ) {
        let toolchain_type = self.optional_toolchain_type_label.clone();
        self.add_toolchain_with_type(
            package_name,
            toolchain_name,
            &toolchain_type,
            exec_constraints,
            target_constraints,
            data,
        );
    }

    /// The toolchain half of `createToolchains` (the fields are set in [`Self::new`]).
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:233 ToolchainTestCase.createToolchains
    fn create_toolchains(&mut self) {
        self.rewrite_module_dot_bazel(&[
            "register_toolchains(\"//toolchain:toolchain_1\", \"//toolchain:toolchain_2\")\n",
        ]);

        self.scratch.file(
            "toolchain/toolchain_def.bzl",
            &[r#"def _impl(ctx):
    toolchain = platform_common.ToolchainInfo(
        data = ctx.attr.data,
    )
    return [toolchain]

test_toolchain = rule(
    implementation = _impl,
    attrs = {
        "data": attr.string(),
    },
)
"#],
        );

        self.scratch.file(
            "toolchain/BUILD",
            &[r#"toolchain_type(name = "test_toolchain")

toolchain_type(name = "optional_toolchain")

toolchain_type(name = "workspace_suffix_toolchain")
"#],
        );

        self.add_toolchain(
            "toolchain",
            "toolchain_1",
            &["//constraints:linux"],
            &["//constraints:mac"],
            "foo",
        );
        self.add_toolchain(
            "toolchain",
            "toolchain_2",
            &["//constraints:mac"],
            &["//constraints:linux"],
            "bar",
        );
        let suffix_toolchain_type_label = label("//toolchain:workspace_suffix_toolchain");
        self.add_toolchain_with_type(
            "toolchain",
            "suffix_toolchain_1",
            &suffix_toolchain_type_label,
            &[],
            &[],
            "suffix1",
        );
        self.add_toolchain_with_type(
            "toolchain",
            "suffix_toolchain_2",
            &suffix_toolchain_type_label,
            &[],
            &[],
            "suffix2",
        );
    }

    /// `targetConfigKey` (the key of the current target configuration).
    pub fn target_config_key(&mut self) -> BuildConfigurationKey {
        self.get_target_configuration().get_key()
    }

    /// `requestToolchainsFromSkyframe` and the tests' `invokeToolchainResolution` /
    /// `requestExecutionPlatformsFromSkyframe`: evaluates `key` with `keepGoing = false`.
    ///
    /// PORT: `enableAnalysis` has no Rust equivalent (the evaluator always allows analysis).
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:310 ToolchainTestCase.requestToolchainsFromSkyframe
    pub fn evaluate_key<K: Key + Clone>(&mut self, key: &K) -> EvaluationResult {
        self.base
            .evaluate(&[SkyKey::new(key.clone())], /* keep_going= */ false)
    }

    /// `moduleRoot`, the fake registry's directory.
    pub fn module_root(&self) -> Path {
        self.base.module_root.clone()
    }

    /// Java `registry.addModule(createModuleKey(name, version), moduleFileLines...)`.
    ///
    /// PORT: the harness's registry is a `file://` index registry in `moduleRoot` (Java's
    /// `FakeRegistry` keeps the module files in memory, see `build_view_test_case.rs`). The module
    /// file is written where an index registry keeps it, and `source.json` is the `local_path`
    /// repo spec `FakeRegistry.getRepoSpec` returns: `moduleRoot/<name>+<version>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/FakeRegistry.java FakeRegistry.addModule
    pub fn add_registry_module(&mut self, name: &str, version: &str, module_file_lines: &[&str]) {
        let root = self.module_root();
        let root = root.get_path_string();
        // FakeRegistry joins the lines with '\n'.
        let content = module_file_lines.join("\n");
        self.scratch.overwrite_file(
            &format!("{root}/modules/{name}/{version}/MODULE.bazel"),
            &[&content],
        );
        // Java: LocalPathRepoSpecs.create(rootPath + "/" + key.getCanonicalRepoNameWithVersion().getName())
        self.scratch.overwrite_file(
            &format!("{root}/modules/{name}/{version}/source.json"),
            &[&format!(
                "{{\"type\": \"local_path\", \"path\": \"{root}/{name}+{version}\"}}"
            )],
        );
    }
}

/// The fields `createConstraints` sets.
struct Constraints {
    linux_platform: PlatformInfo,
    mac_platform: PlatformInfo,
    setting: ConstraintSettingInfo,
    defaulted_setting: ConstraintSettingInfo,
    linux_constraint: ConstraintValueInfo,
    mac_constraint: ConstraintValueInfo,
    defaulted_constraint: ConstraintValueInfo,
}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:104 ToolchainTestCase.createConstraints
fn create_constraints(t: &mut BuildViewTestCase) -> Constraints {
    t.scratch.file(
        "constraints/BUILD",
        &[r#"constraint_setting(name = "os")

constraint_value(
    name = "linux",
    constraint_setting = ":os",
)

constraint_value(
    name = "mac",
    constraint_setting = ":os",
)

constraint_setting(
    name = "setting_with_default",
    default_constraint_value = ":default_value",
)

constraint_value(
    name = "default_value",
    constraint_setting = ":setting_with_default",
)

constraint_value(
    name = "non_default_value",
    constraint_setting = ":setting_with_default",
)
"#],
    );

    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(
    name = "linux",
    constraint_values = [
        "//constraints:linux",
        "//constraints:non_default_value",
    ],
)

platform(
    name = "mac",
    constraint_values = [
        "//constraints:mac",
        "//constraints:non_default_value",
    ],
)
"#],
    );

    let setting = ConstraintSettingInfo::create(label("//constraints:os"));
    let linux_constraint =
        ConstraintValueInfo::create(setting.clone(), label("//constraints:linux"));
    let mac_constraint = ConstraintValueInfo::create(setting.clone(), label("//constraints:mac"));
    let defaulted_setting =
        ConstraintSettingInfo::create(label("//constraints:setting_with_default"));
    let defaulted_constraint = ConstraintValueInfo::create(
        defaulted_setting.clone(),
        label("//constraints:non_default_value"),
    );

    let linux_platform = PlatformInfo::builder()
        .set_label(label("//platforms:linux"))
        .add_constraint(linux_constraint.clone())
        .add_constraint(defaulted_constraint.clone())
        .build()
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mac_platform = PlatformInfo::builder()
        .set_label(label("//platforms:mac"))
        .add_constraint(mac_constraint.clone())
        .add_constraint(defaulted_constraint.clone())
        .build()
        .unwrap_or_else(|e| panic!("{e:?}"));
    Constraints {
        linux_platform,
        mac_platform,
        setting,
        defaulted_setting,
        linux_constraint,
        mac_constraint,
        defaulted_constraint,
    }
}

/// The toolchain type fields `createToolchains` sets.
#[allow(clippy::type_complexity)]
fn create_toolchain_types() -> (
    Label,
    ToolchainTypeRequirement,
    ToolchainTypeInfo,
    Label,
    ToolchainTypeRequirement,
    ToolchainTypeInfo,
) {
    let test_toolchain_type_label = label("//toolchain:test_toolchain");
    let test_toolchain_type = ToolchainTypeRequirement::create(test_toolchain_type_label.clone());
    let test_toolchain_type_info = ToolchainTypeInfo::create(test_toolchain_type_label.clone());

    let optional_toolchain_type_label = label("//toolchain:optional_toolchain");
    let optional_toolchain_type = ToolchainTypeRequirement {
        toolchain_type: optional_toolchain_type_label.clone(),
        mandatory: false,
        ignore_if_invalid: false,
    };
    let optional_toolchain_type_info =
        ToolchainTypeInfo::create(optional_toolchain_type_label.clone());
    (
        test_toolchain_type_label,
        test_toolchain_type,
        test_toolchain_type_info,
        optional_toolchain_type_label,
        optional_toolchain_type,
        optional_toolchain_type_info,
    )
}

/// `ToolchainTypeRequirement.builder(label).mandatory(false).build()`.
pub fn optional_requirement(toolchain_type: Label) -> ToolchainTypeRequirement {
    ToolchainTypeRequirement {
        toolchain_type,
        mandatory: false,
        ignore_if_invalid: false,
    }
}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:73 ToolchainTestCase.collectToolchainLabels
pub fn collect_toolchain_labels(
    toolchains: &[DeclaredToolchainInfo],
    package_root: Option<&PackageIdentifier>,
) -> Vec<Label> {
    toolchains
        .iter()
        .map(|t| t.resolved_toolchain_label().clone())
        .filter(|l| filter_label(package_root, l))
        .collect()
}

// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:81 ToolchainTestCase.filterLabel
pub fn filter_label(package_root: Option<&PackageIdentifier>, label: &Label) -> bool {
    let Some(package_root) = package_root else {
        return true;
    };

    // Make sure the label is under the packageRoot.
    if label.get_repository() != package_root.get_repository() {
        return false;
    }

    label
        .get_package_identifier()
        .get_package_fragment()
        .starts_with(package_root.get_package_fragment())
}

/// `PackageIdentifier.createInMainRepo(name)`.
pub fn main_repo_package(name: &str) -> PackageIdentifier {
    PackageIdentifier::create_in_main_repo_str(name)
}

// ---------------------------------------------------------------------------------------------
// Truth subjects.

/// `assertThatEvaluationResult(result).hasNoError()`.
pub fn assert_has_no_error(result: &EvaluationResult) {
    assert!(
        !result.has_error(),
        "expected no error, got {:?}",
        result.error_map()
    );
}

/// `assertThatEvaluationResult(result).hasError()`.
pub fn assert_has_error(result: &EvaluationResult) {
    assert!(result.has_error(), "expected an error");
}

/// `assertThatEvaluationResult(result).hasErrorEntryForKeyThat(key).hasExceptionThat()`.
pub fn error_exception<K: Key + Clone>(
    result: &EvaluationResult,
    key: &K,
) -> blaze_skyframe::SkyError {
    let error = result
        .get_error(&SkyKey::new(key.clone()))
        .unwrap_or_else(|| panic!("no error entry for {key:?}"));
    error
        .get_exception()
        .unwrap_or_else(|| panic!("error entry for {key:?} has no exception: {error:?}"))
        .clone()
}

/// `assertThatEvaluationResult(result).hasErrorEntryForKeyThat(key).isNotNull()`.
pub fn assert_has_error_entry<K: Key + Clone>(result: &EvaluationResult, key: &K) {
    assert!(
        result.get_error(&SkyKey::new(key.clone())).is_some(),
        "no error entry for {key:?}"
    );
}

/// `hasErrorEntryForKeyThat(key).hasExceptionThat().hasMessageThat()`: Java's `getMessage()`.
pub fn error_message<K: Key + Clone>(result: &EvaluationResult, key: &K) -> String {
    error_exception(result, key).to_string()
}

/// `hasErrorEntryForKeyThat(key).hasExceptionThat().isInstanceOf(<ToolchainException
/// subclass>.class)`.
pub fn assert_error_is_toolchain_exception<K: Key + Clone>(
    result: &EvaluationResult,
    key: &K,
    kind: ToolchainExceptionKind,
) {
    let exception = error_exception(result, key);
    let toolchain_exception = exception
        .downcast_ref::<ToolchainException>()
        .unwrap_or_else(|| panic!("expected a ToolchainException ({kind:?}), got {exception:?}"));
    assert_eq!(
        toolchain_exception.kind(),
        kind,
        "exception: {toolchain_exception:?}"
    );
}

/// `ToolchainContextSubject.hasExecutionPlatform`.
// Java: com/google/devtools/build/lib/analysis/testing/ToolchainContextSubject.java ToolchainContextSubject.hasExecutionPlatform
pub fn assert_has_execution_platform(context: &dyn ToolchainContext, platform: &str) {
    assert_eq!(
        context.execution_platform().label(),
        &Label::parse_canonical(platform).expect("label"),
        "executionPlatform()"
    );
}

/// `ToolchainContextSubject.hasTargetPlatform`.
// Java: com/google/devtools/build/lib/analysis/testing/ToolchainContextSubject.java ToolchainContextSubject.hasTargetPlatform
pub fn assert_has_target_platform(context: &dyn ToolchainContext, platform: &str) {
    assert_eq!(
        context.target_platform().label(),
        &Label::parse_canonical(platform).expect("label"),
        "targetPlatform()"
    );
}

/// `ToolchainContextSubject.hasToolchainType`: a requirement for the label is among
/// `toolchainTypes()`.
// Java: com/google/devtools/build/lib/analysis/testing/ToolchainContextSubject.java ToolchainContextSubject.hasToolchainType
pub fn assert_has_toolchain_type(context: &dyn ToolchainContext, toolchain_type: &Label) {
    assert!(
        context
            .toolchain_types()
            .iter()
            .any(|r| &r.toolchain_type == toolchain_type),
        "toolchainType({toolchain_type}): expected not null; toolchainTypes() = {:?}",
        context.toolchain_types()
    );
}

/// `ToolchainContextSubject.doesntHaveToolchainType`.
// Java: com/google/devtools/build/lib/analysis/testing/ToolchainContextSubject.java ToolchainContextSubject.doesntHaveToolchainType
pub fn assert_doesnt_have_toolchain_type(context: &dyn ToolchainContext, toolchain_type: &str) {
    let toolchain_type = label(toolchain_type);
    assert!(
        !context
            .toolchain_types()
            .iter()
            .any(|r| r.toolchain_type == toolchain_type),
        "toolchainType({toolchain_type}): expected absent"
    );
}

/// `ToolchainContextSubject.hasResolvedToolchain`.
///
/// Java's `resolvedToolchainLabels().contains(resolvedToolchain)` asserts (Truth's `contains`
/// is an assertion).
// Java: com/google/devtools/build/lib/analysis/testing/ToolchainContextSubject.java ToolchainContextSubject.hasResolvedToolchain
pub fn assert_has_resolved_toolchain(context: &dyn ToolchainContext, resolved_toolchain: &str) {
    let resolved_toolchain = Label::parse_canonical(resolved_toolchain).expect("label");
    let labels = context.resolved_toolchain_labels();
    assert!(
        labels.contains(&resolved_toolchain),
        "resolvedToolchainLabels(): expected to contain {resolved_toolchain}, was {labels:?}"
    );
}

/// `assertThat(actual).containsAtLeast(expected...).inOrder()`.
pub fn assert_contains_at_least_in_order(actual: &[Label], expected: &[Label]) {
    let mut i = 0;
    for a in actual {
        if i < expected.len() && a == &expected[i] {
            i += 1;
        }
    }
    if i < expected.len() {
        for e in expected {
            assert!(
                actual.contains(e),
                "containsAtLeast: missing {e}; actual {actual:?}"
            );
        }
        panic!("containsAtLeast(...).inOrder(): actual {actual:?}, expected {expected:?}");
    }
}

/// `assertThat(actual).containsAtLeast(expected...)`.
pub fn assert_contains_at_least(actual: &[Label], expected: &[Label]) {
    for e in expected {
        assert!(
            actual.contains(e),
            "containsAtLeast: missing {e}; actual {actual:?}"
        );
    }
}

/// An `Arc` around a value from an evaluation (convenience for `result.get(key)`).
pub fn get_value<K: Key + Clone>(result: &EvaluationResult, key: &K) -> Arc<K::Value> {
    result
        .get(key)
        .unwrap_or_else(|| panic!("no value for {key:?}; errors: {:?}", result.error_map()))
}
