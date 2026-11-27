//! Java's `FilesOutputFormatterCallbackTest`
//! (src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java).
//!
//! The class extends `ConfiguredTargetQueryTest`, so Java also runs every inherited
//! `AbstractQueryTest`/`PostAnalysisQueryTest` test in it; those run once, in
//! `configured_target_query_semantics_test.rs`. This file has the class's own tests.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use blaze_analysis::providers::output_group_info::{
    OutputGroupInfo, ValidationMode, default_groups,
};
use blaze_cquery::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use blaze_cquery::cquery::files_output_formatter_callback::FilesOutputFormatterCallback;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_query_engine::Setting;
use blaze_query_testing::{EnvironmentKind, collect_target_patterns_with_default_functions};
use common::{AbstractQueryTest, Suite};

/// The fixture: the test with `defineSimpleRule` run (`@Before`).
// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:49 FilesOutputFormatterCallbackTest.defineSimpleRule
fn set_up() -> AbstractQueryTest {
    let t = AbstractQueryTest::new(Suite::Cquery);
    t.write_file(
        "defs/rules.bzl",
        &[r#"AspectInfo = provider()
def _r_impl(ctx):
    default_file = ctx.actions.declare_file(ctx.attr.name + '_default_file')
    output_group_only = ctx.actions.declare_file(ctx.attr.name + '_output_group_only')
    runfile = ctx.actions.declare_file(ctx.attr.name + '_runfile')
    executable_only = ctx.actions.declare_file(ctx.attr.name + '_executable')
    files = [default_file, output_group_only, runfile, executable_only]
    ctx.actions.run_shell(
        outputs = files,
        command = '\n'.join(['touch %s' % file.path for file in files]),
    )
    return [
        DefaultInfo(
            executable = executable_only,
            files = depset(
                direct = [
                    default_file,
                    ctx.file._implicit_source_dep,
                    ctx.file.explicit_source_dep,
                ],
                transitive = [info[DefaultInfo].files for info in ctx.attr.deps]
            ),
            runfiles = ctx.runfiles([runfile]),
        ),
        OutputGroupInfo(
            foobar = [output_group_only, ctx.file.explicit_source_dep],
        ),
    ]
r = rule(
    implementation = _r_impl,
    executable = True,
    attrs = {
        'deps': attr.label_list(),
        '_implicit_source_dep': attr.label(default = 'rules.bzl', allow_single_file = True),
        'explicit_source_dep': attr.label(allow_single_file = True),
    },
)
def _a_impl(target, ctx):
    custom_output_group = ctx.actions.declare_file(target.label.name + '_custom_aspect_a_file')
    shared_output_group = ctx.actions.declare_file(target.label.name + '_shared_aspect_a_file')
    ctx.actions.run_shell(
        outputs = [custom_output_group, shared_output_group],
        command = "touch %s && touch %s" % (custom_output_group.path, shared_output_group.path),
    )
    return [
        OutputGroupInfo(
            aspect_files = depset([custom_output_group]),
            foobar = depset([shared_output_group]),
        ),
        # Collides with aspect b.
        AspectInfo(),
    ]
a = aspect(implementation = _a_impl)
def _b_impl(target, ctx):
    custom_output_group = ctx.actions.declare_file(target.label.name + '_custom_aspect_b_file')
    shared_output_group = ctx.actions.declare_file(target.label.name + '_shared_aspect_b_file')
    ctx.actions.run_shell(
        outputs = [custom_output_group, shared_output_group],
        command = "touch %s && touch %s" % (custom_output_group.path, shared_output_group.path),
    )
    return [
        OutputGroupInfo(
            aspect_files = depset([custom_output_group]),
            foobar = depset([shared_output_group]),
        ),
        # Collides with aspect a.
        AspectInfo(),
    ]
b = aspect(implementation = _b_impl)
"#],
    );
    t.write_file("defs/BUILD", &["exports_files(['rules.bzl'])"]);
    t.write_file(
        "pkg/BUILD",
        &[r#"load("//defs:rules.bzl", "r")

r(
    name = "main",
    explicit_source_dep = "BUILD",
)

r(
    name = "other",
    explicit_source_dep = "BUILD",
    deps = [":main"],
)
"#],
    );
    t
}

/// `getOutput(queryExpression, outputGroups, aspects...)`: the lines the files output formatter
/// prints for the query, analyzed with the given top-level aspects.
///
/// PORT: Java's `new CqueryOptions()` has its fields at Java's zero values; this uses the
/// options' defaults (`CqueryOptions::default()`). The files callback reads only the line
/// terminator, `false` in both.
// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:142 FilesOutputFormatterCallbackTest.getOutput
fn get_output(
    t: &mut AbstractQueryTest,
    query_expression: &str,
    output_groups: &[&str],
    aspects: &[&str],
) -> Vec<String> {
    let target_pattern_set =
        collect_target_patterns_with_default_functions(EnvironmentKind::Cquery, query_expression);
    let env = t
        .helper
        .get_configured_target_query_environment(&target_pattern_set, aspects)
        .unwrap_or_else(|e| panic!("getPostAnalysisQueryEnvironment threw: {e:?}"));
    let expression = blaze_query_engine::QueryExpression::parse(query_expression, &env)
        .unwrap_or_else(|e| panic!("QuerySyntaxException: {}", e.get_message()));

    let options = CqueryOptions::default();
    let output: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let reporter = t.helper.t.event_handler();
    // Based on BuildRequest#getTopLevelArtifactContext.
    let output_groups: Vec<String> = OutputGroupInfo::determine_output_groups(
        &default_groups(),
        &output_groups
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        ValidationMode::Off,
        false,
    )
    .into_iter()
    .collect();
    {
        let executor = t.helper.t.skyframe_executor();
        let callback = FilesOutputFormatterCallback::new(
            CqueryThreadsafeCallback::new(
                reporter,
                &options,
                Some(output.clone()),
                executor,
                &env,
                /* uniquify_results= */ true,
            ),
            output_groups,
        );
        env.evaluate_query(&expression, &callback)
            .unwrap_or_else(|e| panic!("evaluateQuery threw: {e:?}"));
    }
    let text = String::from_utf8(output.borrow().clone()).expect("UTF-8 output");
    text.split('\n')
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `partitioningBy(path -> path.matches("^[^/]*-out/.*"))`: (source files, generated files).
fn partition(output: Vec<String>) -> (Vec<String>, Vec<String>) {
    output.into_iter().partition(|path| {
        let first = path.split('/').next().unwrap_or("");
        !(path.contains('/') && first.ends_with("-out"))
    })
}

fn assert_contains_exactly(actual: &[String], expected: &[&str]) {
    let mut a: Vec<String> = actual.to_vec();
    a.sort();
    let mut e: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    e.sort();
    assert_eq!(a, e);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:284 FilesOutputFormatterCallbackTest.assertContainsExactlyWithBinDirPrefix
fn assert_contains_exactly_with_bin_dir_prefix(output: &[String], bin_dir_relative_paths: &[&str]) {
    if bin_dir_relative_paths.is_empty() {
        assert!(output.is_empty(), "{output:?}");
        return;
    }

    // Extract the configuration-dependent bin dir from the first output.
    assert!(!output.is_empty());
    let first_path = &output[0];
    let bin_dir = &first_path[..first_path.find("bin/").expect("a bin dir") + "bin/".len()];

    let expected: Vec<String> = bin_dir_relative_paths
        .iter()
        .map(|p| format!("{bin_dir}{p}"))
        .collect();
    let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    assert_contains_exactly(output, &expected);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:173 FilesOutputFormatterCallbackTest.basicQuery_defaultOutputGroup
#[test]
fn basic_query_default_output_group() {
    let mut t = set_up();
    let output = get_output(&mut t, "//pkg:all", &[], &[]);
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD", "defs/rules.bzl"]);
    assert_contains_exactly_with_bin_dir_prefix(
        &generated,
        &["pkg/main_default_file", "pkg/other_default_file"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:184 FilesOutputFormatterCallbackTest.basicQuery_defaultAndCustomOutputGroup
#[test]
fn basic_query_default_and_custom_output_group() {
    let mut t = set_up();
    let output = get_output(&mut t, "//pkg:main", &["+foobar"], &[]);
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD", "defs/rules.bzl"]);
    assert_contains_exactly_with_bin_dir_prefix(
        &generated,
        &["pkg/main_default_file", "pkg/main_output_group_only"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:195 FilesOutputFormatterCallbackTest.basicQuery_customOutputGroupOnly
#[test]
fn basic_query_custom_output_group_only() {
    let mut t = set_up();
    let output = get_output(&mut t, "//pkg:other", &["foobar"], &[]);
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD"]);
    assert_contains_exactly_with_bin_dir_prefix(&generated, &["pkg/other_output_group_only"]);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:206 FilesOutputFormatterCallbackTest.withAspect_customOutputGroupOnly
#[test]
fn with_aspect_custom_output_group_only() {
    let mut t = set_up();
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    let output = get_output(
        &mut t,
        "//pkg:other",
        &["aspect_files"],
        &["//defs:rules.bzl%a"],
    );
    let (source, generated) = partition(output);
    assert!(source.is_empty(), "{source:?}");
    assert_contains_exactly_with_bin_dir_prefix(&generated, &["pkg/other_custom_aspect_a_file"]);
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:219 FilesOutputFormatterCallbackTest.withAspect_sharedOutputGroupOnly
#[test]
fn with_aspect_shared_output_group_only() {
    let mut t = set_up();
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    let output = get_output(&mut t, "//pkg:other", &["foobar"], &["//defs:rules.bzl%a"]);
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD"]);
    assert_contains_exactly_with_bin_dir_prefix(
        &generated,
        &[
            "pkg/other_output_group_only",
            "pkg/other_shared_aspect_a_file",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:234 FilesOutputFormatterCallbackTest.withAspects_customOutputGroupOnly
#[test]
fn with_aspects_custom_output_group_only() {
    let mut t = set_up();
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    let output = get_output(
        &mut t,
        "//pkg:other",
        &["aspect_files"],
        &["//defs:rules.bzl%a", "//defs:rules.bzl%b"],
    );
    let (source, generated) = partition(output);
    assert!(source.is_empty(), "{source:?}");
    assert_contains_exactly_with_bin_dir_prefix(
        &generated,
        &[
            "pkg/other_custom_aspect_b_file",
            "pkg/other_custom_aspect_a_file",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:253 FilesOutputFormatterCallbackTest.withAspects_sharedOutputGroupOnly
#[test]
fn with_aspects_shared_output_group_only() {
    let mut t = set_up();
    t.helper.set_query_settings(&[Setting::IncludeAspects]);
    let output = get_output(
        &mut t,
        "//pkg:other",
        &["foobar"],
        &["//defs:rules.bzl%a", "//defs:rules.bzl%b"],
    );
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD"]);
    assert_contains_exactly_with_bin_dir_prefix(
        &generated,
        &[
            "pkg/other_output_group_only",
            "pkg/other_shared_aspect_b_file",
            "pkg/other_shared_aspect_a_file",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/FilesOutputFormatterCallbackTest.java:270 FilesOutputFormatterCallbackTest.withAspects_noIncludeAspects
#[test]
fn with_aspects_no_include_aspects() {
    let mut t = set_up();
    // Explicitly omit INCLUDE_ASPECTS.
    t.helper.set_query_settings(&[]);
    let output = get_output(
        &mut t,
        "//pkg:other",
        &["foobar"],
        &["//defs:rules.bzl%a", "//defs:rules.bzl%b"],
    );
    let (source, generated) = partition(output);
    assert_contains_exactly(&source, &["pkg/BUILD"]);
    assert_contains_exactly_with_bin_dir_prefix(&generated, &["pkg/other_output_group_only"]);
}
