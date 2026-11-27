//! Java's `GraphOutputFormatterCallbackTest`
//! (src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java).
//!
//! The class extends `ConfiguredTargetQueryTest`, so Java also runs every inherited
//! `AbstractQueryTest`/`PostAnalysisQueryTest` test in it; those run once, in
//! `configured_target_query_semantics_test.rs`. This file has the class's own tests.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use blaze_cquery::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use blaze_cquery::cquery::graph_output_formatter_callback::GraphOutputFormatterCallback;
use blaze_optdefs::command::cquery_options::CqueryOptions;
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::Setting;
use blaze_query_testing::{EnvironmentKind, collect_target_patterns_with_default_functions};
use common::{AbstractQueryTest, Suite};

/// The fixture: the test with `defineSimpleRule` and `setUpCqueryOptions` run (`@Before`).
///
/// PORT: Java's `new CqueryOptions()` has its fields at Java's zero values, then sets
/// `graphNodeStringLimit = 512`; Blaze's options start at the options' defaults, so
/// `graph_factored` (default `true`, zero value `false`) is set to `false` here. The graph
/// callback reads only these two and the line terminator (`false` in both).
// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:47 GraphOutputFormatterCallbackTest.defineSimpleRule
// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:66 GraphOutputFormatterCallbackTest.setUpCqueryOptions
fn set_up() -> (AbstractQueryTest, CqueryOptions) {
    let t = AbstractQueryTest::new(Suite::Cquery);
    t.write_file(
        "defs/defs.bzl",
        &["def _impl(ctx):
    pass

simple_rule = rule(
    implementation = _impl,
    attrs = {
        \"deps\": attr.label_list(allow_files = True),
        \"tool_deps\": attr.label_list(cfg = \"exec\"),
    },
)
"],
    );
    t.write_file("defs/BUILD", &[]);
    let mut options = CqueryOptions::default();
    options.graph_factored = false;
    options.graph_node_string_limit = 512;
    (t, options)
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:72 GraphOutputFormatterCallbackTest.getOutput
fn get_output(
    t: &mut AbstractQueryTest,
    options: &CqueryOptions,
    query_expression: &str,
) -> Vec<String> {
    let target_pattern_set =
        collect_target_patterns_with_default_functions(EnvironmentKind::Cquery, query_expression);
    t.helper.set_query_settings(&[Setting::NoImplicitDeps]);
    let env = t
        .helper
        .get_configured_target_query_environment(&target_pattern_set, &[])
        .unwrap_or_else(|e| panic!("getPostAnalysisQueryEnvironment threw: {e:?}"));
    let expression = blaze_query_engine::QueryExpression::parse(query_expression, &env)
        .unwrap_or_else(|e| panic!("QuerySyntaxException: {}", e.get_message()));

    let output: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let reporter = t.helper.t.event_handler();
    {
        let executor = t.helper.t.skyframe_executor();
        // PORT: the deps retriever `ct -> env.getFwdDeps(ImmutableList.of(ct))` is the
        // callback's own (GraphOutputFormatterCallback.get_direct_deps).
        let callback = GraphOutputFormatterCallback::new(
            CqueryThreadsafeCallback::new(
                reporter,
                options,
                Some(output.clone()),
                executor,
                &env,
                /* uniquify_results= */ false,
            ),
            LabelPrinter::legacy(),
        );
        env.evaluate_query(&expression, &callback)
            .unwrap_or_else(|e| panic!("evaluateQuery threw: {e:?}"));
    }
    let text = String::from_utf8(output.borrow().clone()).expect("UTF-8 output");
    // String.split("\n") drops trailing empty strings.
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Convenience method for easily injecting a config hash into an expected output sequence.
// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:95 GraphOutputFormatterCallbackTest.withConfigHash
fn with_config_hash(config_hash: &str, pattern: &[&str]) -> Vec<String> {
    pattern
        .iter()
        .map(|entry| entry.replace("%s", config_hash))
        .collect()
}

/// `firstNode.substring(firstNode.indexOf("(") + 1, firstNode.length() - 2)`.
fn config_hash_of(node: &str) -> String {
    node[node.find('(').expect("a configuration") + 1..node.len() - 2].to_owned()
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:102 GraphOutputFormatterCallbackTest.basicGraph
#[test]
fn basic_graph() {
    let (mut t, options) = set_up();
    t.write_file(
        "test/BUILD",
        &["load(\"//defs:defs.bzl\", \"simple_rule\")

simple_rule(
    name = \"a\",
    deps = [
        \":b\",
        \":c\",
    ],
)

simple_rule(
    name = \"b\",
    deps = [\":d\"],
)

simple_rule(name = \"c\")

simple_rule(name = \"d\")
"],
    );
    let output = get_output(&mut t, &options, "deps(//test:a)");
    let config_hash = config_hash_of(&output[2]);
    assert_eq!(
        get_output(&mut t, &options, "deps(//test:a)"),
        with_config_hash(
            &config_hash,
            &[
                "digraph mygraph {",
                "  node [shape=box];",
                "  \"//test:a (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:b (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:c (%s)\"",
                "  \"//test:c (%s)\"",
                "  \"//test:b (%s)\"",
                "  \"//test:b (%s)\" -> \"//test:d (%s)\"",
                "  \"//test:d (%s)\"",
                "}",
            ]
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:145 GraphOutputFormatterCallbackTest.factorEquivalentNodes
#[test]
fn factor_equivalent_nodes() {
    let (mut t, mut options) = set_up();
    options.graph_factored = true;
    t.write_file(
        "test/BUILD",
        &["load(\"//defs:defs.bzl\", \"simple_rule\")

simple_rule(
    name = \"a\",
    deps = [
        \":b\",
        \":c\",
    ],
)

simple_rule(
    name = \"b\",
    deps = [\":d\"],
)

simple_rule(
    name = \"c\",
    deps = [\":d\"],
)

simple_rule(name = \"d\")
"],
    );
    let output = get_output(&mut t, &options, "deps(//test:a)");
    let config_hash = config_hash_of(&output[2]);
    assert_eq!(
        get_output(&mut t, &options, "deps(//test:a)"),
        with_config_hash(
            &config_hash,
            &[
                "digraph mygraph {",
                "  node [shape=box];",
                "  \"//test:a (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:b (%s)\\n//test:c (%s)\"",
                "  \"//test:b (%s)\\n//test:c (%s)\"",
                "  \"//test:b (%s)\\n//test:c (%s)\" -> \"//test:d (%s)\"",
                "  \"//test:d (%s)\"",
                "}",
            ]
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:190 GraphOutputFormatterCallbackTest.nullAndToolDeps
#[test]
fn null_and_tool_deps() {
    let (mut t, options) = set_up();
    t.write_file(
        "test/BUILD",
        &["load(\"//defs:defs.bzl\", \"simple_rule\")

simple_rule(
    name = \"a\",
    tool_deps = [\":tool_dep\"],
    deps = [
        \":b\",
        \":file.src\",
    ],
)

simple_rule(name = \"b\")

simple_rule(name = \"tool_dep\")
"],
    );
    t.write_file("test/file.src", &[]);
    let correction = t.get_dependency_correction();
    let output = get_output(&mut t, &options, &format!("deps(//test:a){correction}"));
    let config_hash = config_hash_of(&output[2]);
    let exec_config_hash = config_hash_of(&output[6]);
    let tool_edge = format!("  \"//test:a (%s)\" -> \"//test:tool_dep ({exec_config_hash})\"");
    let tool_node = format!("  \"//test:tool_dep ({exec_config_hash})\"");
    assert_eq!(
        output,
        with_config_hash(
            &config_hash,
            &[
                "digraph mygraph {",
                "  node [shape=box];",
                "  \"//test:a (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:b (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:file.src (null)\"",
                &tool_edge,
                &tool_node,
                "  \"//test:file.src (null)\"",
                "  \"//test:b (%s)\"",
                "}",
            ]
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallbackTest.java:232 GraphOutputFormatterCallbackTest.selectsResolvedAndRemoved
#[test]
fn selects_resolved_and_removed() {
    let (mut t, options) = set_up();
    t.write_file(
        "test/BUILD",
        &["load(\"//defs:defs.bzl\", \"simple_rule\")

config_setting(
    name = \"use_a\",
    define_values = {\"a\": \"1\"},
)

simple_rule(
    name = \"a\",
    deps = select({
        \":use_a\": [\":dep_with_a\"],
        \"//conditions:default\": [\":default_dep\"],
    }),
)

simple_rule(name = \"dep_with_a\")

simple_rule(name = \"default_dep\")
"],
    );
    t.helper.use_configuration(&["--define", "a=1"]);
    let output = get_output(&mut t, &options, "deps(//test:a)");
    let config_hash = config_hash_of(&output[2]);
    assert_eq!(
        get_output(&mut t, &options, "deps(//test:a)"),
        with_config_hash(
            &config_hash,
            &[
                "digraph mygraph {",
                "  node [shape=box];",
                "  \"//test:a (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:dep_with_a (%s)\"",
                "  \"//test:a (%s)\" -> \"//test:use_a (%s)\"",
                "  \"//test:use_a (%s)\"",
                "  \"//test:dep_with_a (%s)\"",
                "}",
            ]
        )
    );
}
