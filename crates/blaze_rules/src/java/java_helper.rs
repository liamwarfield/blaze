//! `rules/java/JavaHelper.java`: utility methods for use by Java-related parts of Bazel.

use blaze_actions::Artifact;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::config_util;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::{AttrValue, Type};
use blaze_shell::shell_utils::ShellUtils;
use blaze_vfs::PathFragment;

use super::java_semantics::JavaSemantics;

/// Flattens a set of javacopts and tokenizes the contents.
///
/// Since javac allows passing the same option multiple times, and the right-most one wins,
/// multiple instances of the same option+value get de-duped by the NestedSet. So when combining
/// multiple NestedSets, we store them in reverse order, and reverse again after flattening. This
/// preserves the right-most occurrence in its correct position, thus achieving the correct
/// semantics.
// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:47 JavaHelper.tokenizeJavaOptions
pub fn tokenize_java_options_nested_set(in_opts: &NestedSet<String>) -> Vec<String> {
    let mut list: Vec<String> = in_opts.to_list().to_vec();
    list.reverse();
    tokenize_java_options(&list)
}

/// Javac options require special processing - People use them and expect the options to be
/// tokenized.
// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:55 JavaHelper.tokenizeJavaOptions
pub fn tokenize_java_options<S: AsRef<str>>(in_opts: &[S]) -> Vec<String> {
    // Ideally, this would be in the options parser. Unfortunately,
    // the options parser can't handle a converter that expands
    // from a value X into a List<X> and allow-multiple at the
    // same time.
    let mut result: Vec<String> = Vec::new();
    for current in in_opts {
        let current = current.as_ref();
        if ShellUtils::tokenize(&mut result, current).is_err() {
            // Tokenization failed; this likely means that the user
            // did not want tokenization to happen on their argument.
            // (Any tokenization where we should produce an error
            // has already been done by the shell that invoked
            // blaze). Therefore, pass the argument through to
            // the tool, so that we can see the original error.
            result.push(current.to_owned());
        }
    }
    result
}

/// De-tokenizes a collection of `javac` options into a NestedSet.
///
/// Each option is shell-escaped to get back the original option as-is when we tokenize the
/// depset. Returns a NestedSet of the supplied options concatenated into a single string
/// separated by ' '.
// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:88 JavaHelper.detokenizeJavaOptions
pub fn detokenize_java_options(javac_opts: &[String]) -> NestedSet<String> {
    if javac_opts.is_empty() {
        return NestedSetBuilder::empty_set(Order::NAIVE_LINK_ORDER);
    }
    NestedSetBuilder::create(
        Order::NAIVE_LINK_ORDER,
        vec![
            javac_opts
                .iter()
                .map(|o| ShellUtils::shell_escape(o))
                .collect::<Vec<_>>()
                .join(" "),
        ],
    )
}

// Java: com/google/devtools/build/lib/rules/java/JavaHelper.java:97 JavaHelper.getJavaResourcePath
pub fn get_java_resource_path(
    semantics: &dyn JavaSemantics,
    rule_context: &RuleContext,
    resource: &Artifact,
) -> PathFragment {
    let sibling_repository_layout =
        config_util::is_sibling_repository_layout(rule_context.get_configuration());
    let mut resource_path = resource.get_output_dir_relative_path(sibling_repository_layout);
    let repo_exec_path = rule_context
        .get_label()
        .get_repository()
        .get_exec_path(sibling_repository_layout);
    if !repo_exec_path.is_empty() && resource_path.starts_with(&repo_exec_path) {
        resource_path = resource_path.relative_to(&repo_exec_path);
    }

    let attributes = rule_context.attributes();
    if !attributes.has_typed("resource_strip_prefix", Type::String)
        || !attributes.is_attribute_value_explicitly_specified("resource_strip_prefix")
    {
        return semantics.get_default_java_resource_path(&resource_path);
    }

    let prefix = match attributes.get("resource_strip_prefix", Type::String) {
        Some(AttrValue::String(s)) => PathFragment::create(&s),
        other => panic!("ClassCastException: resource_strip_prefix is {other:?}"),
    };

    if !resource_path.starts_with(&prefix) {
        rule_context.attribute_error(
            "resource_strip_prefix",
            &format!(
                "Resource file '{}' is not under the specified prefix to strip",
                resource_path
            ),
        );
        return resource_path;
    }

    resource_path.relative_to(&prefix)
}

#[cfg(test)]
mod tests {
    //! Java: src/test/java/com/google/devtools/build/lib/rules/java/JavaUtilTest.java
    use super::*;

    // Java: JavaUtilTest.testDetokenization
    #[test]
    fn test_detokenization() {
        let options: Vec<String> = [
            "-source",
            "8",
            "-target",
            "8",
            "-Xmx1G",
            "--arg=val",
            "-XepExcludedPaths:.*/\\\\$$?\\\\$$?AutoValue(Gson)?_.*\\.java",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let detokenized = detokenize_java_options(&options);
        let retokenized = tokenize_java_options_nested_set(&detokenized);

        assert_eq!(
            detokenized.to_list().to_vec(),
            vec![
                "-source 8 -target 8 -Xmx1G '--arg=val'".to_owned()
                    + " '-XepExcludedPaths:.*/\\\\$$?\\\\$$?AutoValue(Gson)?_.*\\.java'"
            ]
        );
        assert_eq!(retokenized, options);
    }
}
