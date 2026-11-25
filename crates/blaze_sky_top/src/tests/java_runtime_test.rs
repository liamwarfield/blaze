//! Port of `src/test/java/com/google/devtools/build/lib/rules/java/JavaRuntimeTest.java`, on the
//! harness's mock `rules_java` (`AbstractMockJavaSupport.BAZEL`).

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::downcast_info;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::platform::template_variable_info::TemplateVariableInfo;
use blaze_analysis::platform::toolchain_info::FrozenToolchainInfo;
use blaze_cmdline::repository_name::MAIN;
use blaze_packages::starlark_defs::platform_common::TEMPLATE_VARIABLE_INFO;
use blaze_rules::java::java_runtime_info::JavaRuntimeInfo;
use starlark::values::Heap;

use crate::testing::{
    BuildViewTestCase, assert_contains_exactly, get_data_runfiles, pretty_artifact_names,
};

/// `TestConstants.TOOLS_REPOSITORY`.
const TOOLS_REPOSITORY: &str = "@bazel_tools";

/// `@Before initializeJvmPackage`.
// Java: JavaRuntimeTest.initializeJvmPackage
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    t.scratch.file(
        "jvm/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_runtime")
java_runtime(
    name = "jvm-k8",
    srcs = [
        "k8/a",
        "k8/b",
    ],
    java_home = "k8",
)
"#],
    );
    t
}

/// `getJavaRuntimeInfo(collection)`: `JavaRuntimeInfo.wrap(toolchainInfo.getValue("java_runtime",
/// Info.class))`, handed to `f` (the wrapper borrows a heap).
// Java: JavaRuntimeTest.getJavaRuntimeInfo
fn with_java_runtime_info<R>(
    collection: &ConfiguredTarget,
    f: impl for<'v> FnOnce(JavaRuntimeInfo<'v>) -> R,
) -> R {
    let toolchain_info = platform_provider_utils::toolchain(Some(collection))
        .expect("NullPointerException: ToolchainInfo");
    let toolchain_info = downcast_info::<FrozenToolchainInfo>(&toolchain_info)
        .expect("ClassCastException: ToolchainInfo");
    let value = toolchain_info
        .get_value("java_runtime")
        .expect("NullPointerException: java_runtime")
        .unpack_frozen()
        .expect("a frozen value");
    Heap::temp(|heap| {
        let info =
            JavaRuntimeInfo::wrap(value.to_value(), heap).unwrap_or_else(|e| panic!("{e}"));
        f(info)
    })
}

// Java: JavaRuntimeTest.simple
#[test]
fn simple() {
    let mut t = new_test();
    let target = t.get_configured_target("//jvm:jvm-k8").unwrap();
    let inputs = with_java_runtime_info(&target, |info| {
        pretty_artifact_names(info.java_base_inputs().unwrap().to_list().iter())
    });
    assert_contains_exactly(&inputs, &["jvm/k8/a", "jvm/k8/b"]);
    assert_contains_exactly(
        &pretty_artifact_names(get_data_runfiles(&target).get_artifacts().to_list().iter()),
        &["jvm/k8/a", "jvm/k8/b"],
    );
}

// Java: JavaRuntimeTest.absoluteJavaHomeWithSrcs
#[test]
fn absolute_java_home_with_srcs() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[':dummy'], java_home='/absolute/path')",
        ],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:jvm");
    t.assert_contains_event("'java_home' with an absolute path requires 'srcs' to be empty.");
}

// Java: JavaRuntimeTest.absoluteJavaHomeWithJava
#[test]
fn absolute_java_home_with_java() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', java='bin/java', java_home='/absolute/path')",
        ],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:jvm");
    t.assert_contains_event("'java_home' with an absolute path requires 'java' to be empty.");
}

// Java: JavaRuntimeTest.binJavaPathName
#[test]
fn bin_java_path_name() {
    let mut t = new_test();
    t.scratch.file(
        "BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', java='java')",
        ],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//:jvm");
    t.assert_contains_event("the path to 'java' must end in 'bin/java'.");
}

// Java: JavaRuntimeTest.absoluteJavaHome
#[test]
fn absolute_java_home() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[], java_home='/absolute/path')",
        ],
    );
    t.remove_fail_fast_handler();
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    assert_eq!(
        with_java_runtime_info(&jvm, |info| info.java_home().unwrap()),
        Some("/absolute/path".to_string())
    );
}

// Java: JavaRuntimeTest.relativeJavaHome
#[test]
fn relative_java_home() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[], java_home='b/c')",
        ],
    );
    t.remove_fail_fast_handler();
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    assert_eq!(
        with_java_runtime_info(&jvm, |info| info.java_home().unwrap()),
        Some("a/b/c".to_string())
    );
}

// Java: JavaRuntimeTest.testRuntimeAlias
#[test]
fn test_runtime_alias() {
    let mut t = new_test();
    let reference = t
        .scratch_configured_target(
            "a",
            "ref",
            &[
                &format!(
                    "load('{TOOLS_REPOSITORY}//tools/jdk:java_toolchain_alias.bzl', 'java_runtime_alias')"
                ),
                "java_runtime_alias(name='ref')",
            ],
        )
        .expect("ref");
    assert!(platform_provider_utils::toolchain(Some(&reference)).is_some());
    assert!(reference.get(&TEMPLATE_VARIABLE_INFO.key()).is_some());
}

// Java: JavaRuntimeTest.javaHomeWithMakeVariables
#[test]
fn java_home_with_make_variables() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[], java_home='/opt/$(CMDLINE)')",
        ],
    );
    t.use_configuration(&["--define=CMDLINE=foo/bar"]);
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    assert_eq!(
        with_java_runtime_info(&jvm, |info| info.java_home().unwrap()),
        Some("/opt/foo/bar".to_string())
    );
}

// Java: JavaRuntimeTest.javaHomeWithInvalidMakeVariables
#[test]
fn java_home_with_invalid_make_variables() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[], java_home='/opt/$(WTF)')",
        ],
    );
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:jvm");
    t.assert_contains_event("$(WTF) not defined");
}

// Java: JavaRuntimeTest.makeVariables
#[test]
fn make_variables() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', srcs=[], java_home='/foo/bar')",
        ],
    );
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    let provider = jvm
        .get(&TEMPLATE_VARIABLE_INFO.key())
        .expect("TemplateVariableInfo");
    let runtime = downcast_info::<TemplateVariableInfo>(&provider)
        .expect("ClassCastException: TemplateVariableInfo")
        .get_variables();
    assert_eq!(
        runtime.get("JAVABASE").map(String::as_str),
        Some("/foo/bar")
    );
    // Windows has .exe suffix
    assert!(
        runtime
            .get("JAVA")
            .expect("JAVA")
            .starts_with("/foo/bar/bin/java")
    );
}

// Java: JavaRuntimeTest.noSrcs
#[test]
fn no_srcs() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_runtime')",
            "java_runtime(name='jvm', java_home='/opt/jvm')",
        ],
    );
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    let (java_home, inputs_empty) = with_java_runtime_info(&jvm, |provider| {
        (
            provider.java_home().unwrap(),
            provider.java_base_inputs().unwrap().to_list().is_empty(),
        )
    });
    assert_eq!(java_home, Some("/opt/jvm".to_string()));
    assert!(inputs_empty);
}

// Java: JavaRuntimeTest.invalidJavaBase
#[test]
fn invalid_java_base() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_java//java:defs.bzl', 'java_binary')",
            "java_binary(name='a', srcs=['A.java'])",
            "filegroup(name='fg')",
            "toolchain(",
            "    name = 'java_runtime_toolchain',",
            "    toolchain = ':fg',",
            &format!(
                "    toolchain_type = '{TOOLS_REPOSITORY}//tools/jdk:runtime_toolchain_type',"
            ),
            ")",
        ],
    );
    t.use_configuration(&["--extra_toolchains=//a:all"]);
    t.remove_fail_fast_handler();
    t.get_configured_target("//a:a");
    t.assert_contains_event("does not provide ToolchainInfo");
}

// Java: JavaRuntimeTest.javaHomeGenerated
#[test]
fn java_home_generated() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_runtime")
genrule(
    name = "gen",
    outs = ["generated_java_home/bin/java"],
    cmd = "",
)
java_runtime(
    name = "jvm",
    java = "generated_java_home/bin/java",
    java_home = "generated_java_home",
)
"#],
    );
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    // getGenfilesArtifactWithNoOwner("a/generated_java_home").getExecPathString()
    let expected = t
        .get_target_configuration()
        .get_genfiles_directory(&MAIN)
        .get_exec_path()
        .get_relative(&blaze_vfs::PathFragment::create("a/generated_java_home"))
        .get_path_string()
        .to_string();
    assert_eq!(
        with_java_runtime_info(&jvm, |info| info.java_home().unwrap()),
        Some(expected)
    );
}

// Java: JavaRuntimeTest.javaRuntimeVersion_isAccessibleByNativeCode
#[test]
fn java_runtime_version_is_accessible_by_native_code() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_runtime")
genrule(
    name = "gen",
    outs = ["generated_java_home/bin/java"],
    cmd = "",
)
java_runtime(
    name = "jvm",
    java = "generated_java_home/bin/java",
    java_home = "generated_java_home",
    version = 234,
)
"#],
    );
    let jvm = t.get_configured_target("//a:jvm").unwrap();
    assert_eq!(
        with_java_runtime_info(&jvm, |info| info.version().unwrap()),
        234
    );
}
