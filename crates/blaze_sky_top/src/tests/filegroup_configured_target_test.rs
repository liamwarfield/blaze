//! Port of `src/test/java/com/google/devtools/build/lib/rules/filegroup/FilegroupConfiguredTargetTest.java`.

use crate::testing::{
    BuildViewTestCase, assert_contains_exactly, base_artifact_names, get_data_runfiles,
    get_default_runfiles, get_files_to_build, pretty_artifact_names,
};

// Java: FilegroupConfiguredTargetTest.testGroup
#[test]
fn test_group() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "nevermore/BUILD",
        &["filegroup(name  = 'staticdata',\n          srcs = ['staticdata/spam.txt', 'staticdata/good.txt'])\n"],
    );
    let group_target = t.get_configured_target("//nevermore:staticdata").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&group_target).to_list().iter()),
        &[
            "nevermore/staticdata/spam.txt",
            "nevermore/staticdata/good.txt",
        ],
    );
}

// Java: FilegroupConfiguredTargetTest.testDependencyGraph
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_binary from the mock rules_java)"]
fn test_dependency_graph() {}

// Java: FilegroupConfiguredTargetTest.testEmptyGroupIsAnOk
#[test]
fn test_empty_group_is_an_ok() {
    let mut t = BuildViewTestCase::new();
    t.scratch_configured_target("empty", "empty", &["filegroup(name='empty', srcs=[])"]);
}

// Java: FilegroupConfiguredTargetTest.testEmptyGroupInGenruleIsOk
#[test]
fn test_empty_group_in_genrule_is_ok() {
    let mut t = BuildViewTestCase::new();
    t.scratch_configured_target(
        "empty",
        "genempty",
        &[
            "filegroup(name='empty', srcs=[])",
            "genrule(name='genempty', tools=[':empty'], outs=['nothing'], cmd='touch $@')",
        ],
    );
}

fn write_test(t: &BuildViewTestCase) {
    t.scratch.file(
        "another/BUILD",
        &["filegroup(name  = 'another',\n          srcs = ['another.txt'])\n"],
    );
    t.scratch.file(
        "test/BUILD",
        &["filegroup(name  = 'a',\n          srcs = ['a.txt'])\nfilegroup(name  = 'b',\n          srcs = ['a.txt'])\nfilegroup(name  = 'c',\n          srcs = ['a', 'b.txt'])\nfilegroup(name  = 'd',\n          srcs = ['//another:another.txt'])\n"],
    );
}

// Java: FilegroupConfiguredTargetTest.testFileCanBeSrcsOfMultipleRules
#[test]
fn test_file_can_be_srcs_of_multiple_rules() {
    let mut t = BuildViewTestCase::new();
    write_test(&t);
    let a = t.get_configured_target("//test:a").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&a).to_list().iter()),
        &["test/a.txt"],
    );
    let b = t.get_configured_target("//test:b").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&b).to_list().iter()),
        &["test/a.txt"],
    );
}

// Java: FilegroupConfiguredTargetTest.testRuleCanBeSrcsOfOtherRule
#[test]
fn test_rule_can_be_srcs_of_other_rule() {
    let mut t = BuildViewTestCase::new();
    write_test(&t);
    let c = t.get_configured_target("//test:c").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&c).to_list().iter()),
        &["test/a.txt", "test/b.txt"],
    );
}

// Java: FilegroupConfiguredTargetTest.testOtherPackageCanBeSrcsOfRule
#[test]
fn test_other_package_can_be_srcs_of_rule() {
    let mut t = BuildViewTestCase::new();
    write_test(&t);
    let d = t.get_configured_target("//test:d").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&d).to_list().iter()),
        &["another/another.txt"],
    );
}

// Java: FilegroupConfiguredTargetTest.testIsNotExecutable
#[test]
fn test_is_not_executable() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "x/BUILD",
        &["filegroup(name = 'not_exec_two_files', srcs = ['bin', 'bin.sh'])"],
    );
    assert!(t.get_executable_of("//x:not_exec_two_files").is_none());
}

// Java: FilegroupConfiguredTargetTest.testIsExecutable
#[test]
fn test_is_executable() {
    let mut t = BuildViewTestCase::new();
    t.scratch
        .file("x/BUILD", &["filegroup(name = 'exec', srcs = ['bin'])"]);
    assert_eq!(
        t.get_executable_of("//x:exec")
            .unwrap()
            .get_exec_path()
            .get_path_string(),
        "x/bin"
    );
}

// Java: FilegroupConfiguredTargetTest.testNoDuplicate
#[test]
fn test_no_duplicate() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "x/BUILD",
        &["filegroup(name = 'a', srcs = ['file'])\nfilegroup(name = 'b', srcs = ['file'])\nfilegroup(name = 'c', srcs = [':a', ':b'])\n"],
    );
    let c = t.get_configured_target("//x:c").unwrap();
    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&c).to_list().iter()),
        &["x/file"],
    );
}

// Java: FilegroupConfiguredTargetTest.testGlobMatchesRuleOutputsInsteadOfFileWithTheSameName
#[test]
fn test_glob_matches_rule_outputs_instead_of_file_with_the_same_name() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("pkg/file_or_rule", &[]);
    t.scratch.file("pkg/a.txt", &[]);
    let target = t
        .scratch_configured_target(
            "pkg",
            "my_rule",
            &[
                "filegroup(name = 'file_or_rule', srcs = ['a.txt'])",
                "filegroup(name = 'my_rule', srcs = glob(['file_or_rule']))",
            ],
        )
        .unwrap();
    assert_contains_exactly(
        &base_artifact_names(get_files_to_build(&target).to_list().iter()),
        &["a.txt"],
    );
}

// Java: FilegroupConfiguredTargetTest.outputGroupSourceJars_extractsTransitiveSources
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn output_group_source_jars_extracts_transitive_sources() {}

// Java: FilegroupConfiguredTargetTest.outputGroupDirectSourceJars_extractsDirectSources
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (java_library from the mock rules_java)"]
fn output_group_direct_source_jars_extracts_direct_sources() {}

// Java: FilegroupConfiguredTargetTest.testErrorForIllegalOutputGroup
#[test]
#[ignore = "needs the mock C++/Java/Python toolchains (cc_library from the mock rules_cc)"]
fn test_error_for_illegal_output_group() {}

/// `testDefaultInfo(@TestParameter boolean filegroupRunfilesForData)`.
fn test_default_info(filegroup_runfiles_for_data: bool) {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "x/defs.bzl",
        &[r#"def _default_info_impl(ctx):
    files = depset(transitive = [t[DefaultInfo].files for t in ctx.attr.files])
    default_runfiles = ctx.runfiles(transitive_files = depset(transitive = [t[DefaultInfo].files for t in ctx.attr.default_runfiles]))
    data_runfiles = ctx.runfiles(transitive_files = depset(transitive = [t[DefaultInfo].files for t in ctx.attr.data_runfiles]))
    return [
        DefaultInfo(
            files = files,
            default_runfiles = default_runfiles,
            data_runfiles = data_runfiles,
        )
    ]
default_info = rule(
    implementation = _default_info_impl,
    attrs = {
        "files": attr.label_list(allow_files=True),
        "default_runfiles": attr.label_list(allow_files=True),
        "data_runfiles": attr.label_list(allow_files=True),
    },
)
"#],
    );
    t.scratch.file(
        "x/BUILD",
        &[r#"load(":defs.bzl", "default_info")

default_info(
    name = "default_info_srcs",
    files = ["srcs_files_file"],
    default_runfiles = ["srcs_default_runfiles_file"],
    data_runfiles = ["srcs_data_runfiles_file"],
)

default_info(
    name = "default_info_data",
    files = ["data_files"],
    default_runfiles = ["data_default_runfiles_file"],
    data_runfiles = ["data_data_runfiles_file"],
)

filegroup(
    name = "filegroup",
    srcs = [
        ":default_info_srcs",
        "srcs_file",
    ],
    data = [
        ":default_info_data",
        "data_file",
    ],
)
"#],
    );

    t.use_configuration(&[&format!(
        "--incompatible_filegroup_runfiles_for_data={filegroup_runfiles_for_data}"
    )]);
    let filegroup = t.get_configured_target("//x:filegroup").unwrap();

    assert_contains_exactly(
        &pretty_artifact_names(get_files_to_build(&filegroup).to_list().iter()),
        &["x/srcs_file", "x/srcs_files_file"],
    );
    assert_contains_exactly(
        &pretty_artifact_names(
            get_default_runfiles(&filegroup)
                .get_artifacts()
                .to_list()
                .iter(),
        ),
        &[
            "x/srcs_default_runfiles_file",
            "x/data_file",
            "x/data_files",
            "x/data_data_runfiles_file",
        ],
    );
    let mut expected_data_runfiles = vec![
        "x/srcs_file",
        "x/srcs_files_file",
        "x/data_file",
        "x/data_files",
        "x/data_data_runfiles_file",
    ];
    if filegroup_runfiles_for_data {
        expected_data_runfiles.push("x/srcs_data_runfiles_file");
    }
    assert_contains_exactly(
        &pretty_artifact_names(
            get_data_runfiles(&filegroup)
                .get_artifacts()
                .to_list()
                .iter(),
        ),
        &expected_data_runfiles,
    );
}

// Java: FilegroupConfiguredTargetTest.testDefaultInfo (filegroupRunfilesForData = false)
#[test]
fn test_default_info_false() {
    test_default_info(false);
}

// Java: FilegroupConfiguredTargetTest.testDefaultInfo (filegroupRunfilesForData = true)
#[test]
fn test_default_info_true() {
    test_default_info(true);
}
