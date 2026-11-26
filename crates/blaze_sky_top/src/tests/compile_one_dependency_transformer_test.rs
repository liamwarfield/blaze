//! Port of `src/test/java/com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformerTest.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryName, TargetParsingException, TargetPattern,
};
use blaze_events::ExtendedEventHandler;
use blaze_packages::package::Package;
use blaze_packages::target::Target;
use blaze_sky_loading::compile_one_dependency_transformer::CompileOneDependencyTransformer;
use blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectories;
use blaze_sky_loading::package_manager::{
    NoSuchPackageOrInterrupted, PreloadTargetPatternsError, SkyframePackageManager,
};
use blaze_sky_loading::target_pattern_eval::TargetPatternEvalError;
use blaze_sky_loading::target_pattern_function::{ProviderError, RecursivePackageProvider};
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::testing::BuildViewTestCase;

/// The test's `getPackageManager()` as the transformer's `PackageProvider`.
struct PackageManagerProvider {
    package_manager: Arc<SkyframePackageManager>,
    reporter: Arc<dyn ExtendedEventHandler>,
}

impl RecursivePackageProvider for PackageManagerProvider {
    // Java: com/google/devtools/build/lib/skyframe/SkyframePackageManager.java SkyframePackageManager.getPackage
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        match self
            .package_manager
            .get_package(self.reporter.clone(), package_name)
        {
            Ok(pkg) => Ok(pkg),
            Err(NoSuchPackageOrInterrupted::NoSuchPackage(e)) => Err(e.into()),
            Err(NoSuchPackageOrInterrupted::Interrupted) => {
                Err(TargetPatternEvalError::Interrupted.into())
            }
        }
    }

    fn is_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        _package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        unreachable!("CompileOneDependencyTransformer does not call isPackage")
    }

    fn stream_packages_under_directory(
        &self,
        _results: &mut dyn FnMut(Vec<PackageIdentifier>),
        _event_handler: &dyn ExtendedEventHandler,
        _repository: &RepositoryName,
        _directory: &PathFragment,
        _ignored_subdirectories: &IgnoredSubdirectories,
        _excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        unreachable!("CompileOneDependencyTransformer does not stream packages")
    }
}

// Java: CompileOneDependencyTransformerTest.setupLangRules (MockCcSupport.setup is part of the
// harness's default mock setup).
fn new_test() -> BuildViewTestCase {
    BuildViewTestCase::new()
}

// Java: CompileOneDependencyTransformerTest.writeSimpleExample
fn write_simple_example(t: &BuildViewTestCase) {
    t.scratch.file(
        "foo/rule.bzl",
        &[r#"def _impl(ctx):
    ctx.actions.do_nothing(mnemonic = "Mnemonic")
    return []

crule_without_srcs = rule(
    _impl,
    attrs = {
        "hdrs": attr.label_list(flags = ["DIRECT_COMPILE_TIME_INPUT"]),
    },
    fragments = ["cpp"],
)
"#],
    );
    t.scratch.file(
        "foo/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
load(":rule.bzl", "crule_without_srcs")

cc_library(
    name = "foo1",
    srcs = ["foo1.cc"],
    hdrs = ["foo1.h"],
)

crule_without_srcs(
    name = "foo2",
    hdrs = ["foo2.h"],
)

exports_files(["baz/bang"])
"#],
    );
    t.scratch.file(
        "foo/bar/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "bar1",
    alwayslink = 1,
)

cc_library(name = "bar2")

exports_files([
    "wiz/bang",
    "wiz/all",
    "baz",
    "baz/bang",
    "undeclared.h",
])
"#],
    );
}

// Java: CompileOneDependencyTransformerTest.labels
fn labels(label_strings: &[&str]) -> HashSet<Label> {
    label_strings
        .iter()
        .map(|s| Label::parse_canonical(s).unwrap_or_else(|e| panic!("{s}: {e}")))
        .collect()
}

// Java: CompileOneDependencyTransformerTest.parseCompileOneDep
fn parse_compile_one_dep(
    t: &mut BuildViewTestCase,
    patterns: &[&str],
) -> Result<Vec<Target>, TargetParsingException> {
    parse_list_compile_one_dep_with_offset(t, PathFragment::empty_fragment(), patterns)
}

// Java: CompileOneDependencyTransformerTest.parseListCompileOneDep
fn parse_list_compile_one_dep(t: &mut BuildViewTestCase, patterns: &[&str]) -> HashSet<Label> {
    targets_to_labels(parse_compile_one_dep(t, patterns).unwrap_or_else(|e| panic!("{e:?}")))
}

// Java: CompileOneDependencyTransformerTest.parseListCompileOneDepRelative
fn parse_list_compile_one_dep_relative(
    t: &mut BuildViewTestCase,
    patterns: &[&str],
) -> HashSet<Label> {
    let foo = t.scratch.dir("foo");
    let offset = foo.relative_to(&t.root_directory);
    targets_to_labels(
        parse_list_compile_one_dep_with_offset(t, offset, patterns)
            .unwrap_or_else(|e| panic!("{e:?}")),
    )
}

// Java: CompileOneDependencyTransformerTest.parseListCompileOneDepWithOffset
fn parse_list_compile_one_dep_with_offset(
    t: &mut BuildViewTestCase,
    offset: PathFragment,
    patterns: &[&str],
) -> Result<Vec<Target>, TargetParsingException> {
    let reporter = t.event_handler();
    let package_manager = t.skyframe_executor().get_package_manager().clone();
    // ImmutableSet.copyOf(patterns)
    let unique: IndexSet<String> = patterns.iter().map(|p| p.to_string()).collect();
    let unique: Vec<String> = unique.into_iter().collect();
    let resolved_targets_map = match package_manager.preload_target_patterns(
        reporter.clone(),
        &TargetPattern::main_repo_parser(offset),
        &unique,
        false,
    ) {
        Ok(m) => m,
        Err(PreloadTargetPatternsError::TargetParsing(e)) => return Err(e),
        Err(PreloadTargetPatternsError::Interrupted) => panic!("InterruptedException"),
    };
    // ResolvedTargets.Builder: a set of targets.
    let mut result: Vec<Target> = Vec::new();
    for pattern in patterns {
        for target in &resolved_targets_map[*pattern] {
            if !result.iter().any(|t| t.get_label() == target.get_label()) {
                result.push(target.clone());
            }
        }
    }
    let provider = PackageManagerProvider {
        package_manager,
        reporter: reporter.clone(),
    };
    match CompileOneDependencyTransformer::new(&provider)
        .transform_compile_one_dependency(&*reporter, result.iter())
    {
        Ok(targets) => Ok(targets),
        Err(TargetPatternEvalError::TargetParsing(e)) => Err(e),
        Err(other) => panic!("{other:?}"),
    }
}

// Java: CompileOneDependencyTransformerTest.targetsToLabels (and getFailFast: the transformed
// result never has an error)
fn targets_to_labels(targets: Vec<Target>) -> HashSet<Label> {
    targets.iter().map(|t| t.get_label().clone()).collect()
}

fn parse_error(t: &mut BuildViewTestCase, pattern: &str) -> String {
    match parse_compile_one_dep(t, &[pattern]) {
        Ok(targets) => panic!(
            "expected TargetParsingException, got {:?}",
            targets_to_labels(targets)
        ),
        Err(e) => e.get_message().to_string(),
    }
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDep
#[test]
fn test_compile_one_dep() {
    let mut t = new_test();
    write_simple_example(&t);
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["foo/foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["foo/foo1.h"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["foo:foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["//foo:foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep_relative(&mut t, &["//foo:foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep_relative(&mut t, &[":foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep_relative(&mut t, &["foo1.cc"]),
        labels(&["@//foo:foo1"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["foo/foo2.h"]),
        labels(&["@//foo:foo2"])
    );
}

/// Regression test for bug: "--compile_one_dependency should report error for missing input".
// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnMissingFile
#[test]
fn test_compile_one_dep_on_missing_file() {
    let mut t = new_test();
    write_simple_example(&t);
    t.remove_fail_fast_handler();
    // TestUtils.createMissingTargetAssertionString("missing.cc", "foo", "/workspace", "")
    let expected = format!(
        "no such target '//foo:missing.cc': target 'missing.cc' not declared in package 'foo' \
         defined by {}/foo/BUILD",
        t.root_directory.get_path_string()
    );
    assert_eq!(parse_error(&mut t, "//foo:missing.cc"), expected);

    // Also, try a valid input file which has no dependent rules in its package.
    assert_eq!(
        parse_error(&mut t, "//foo:baz/bang"),
        "Couldn't find dependency on target '//foo:baz/bang'"
    );

    // Try a header that is in a package but where no cc_library explicitly lists it.
    assert_eq!(
        parse_error(&mut t, "//foo/bar:undeclared.h"),
        "Couldn't find dependency on target '//foo/bar:undeclared.h'"
    );
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnNonSourceTarget
#[test]
fn test_compile_one_dep_on_non_source_target() {
    let mut t = new_test();
    write_simple_example(&t);
    assert_eq!(
        parse_error(&mut t, "//foo:foo1"),
        "--compile_one_dependency target '//foo:foo1' must be a file"
    );
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnTwoTargets
#[test]
fn test_compile_one_dep_on_two_targets() {
    let mut t = new_test();
    t.scratch.file(
        "recursive/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "x",
    srcs = ["foox.cc"],
)

cc_library(
    name = "y",
    srcs = ["fooy.cc"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["//recursive:foox.cc", "//recursive:fooy.cc"]),
        labels(&["//recursive:x", "//recursive:y"])
    );
}

/// Regression test for bug: "--compile_one_dependency should not crash in the presence of
/// mutually recursive targets"
// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnRecursiveTarget
#[test]
fn test_compile_one_dep_on_recursive_target() {
    let mut t = new_test();
    t.scratch.file(
        "recursive/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
filegroup(
    name = "x",
    srcs = [
        "foo.cc",
        ":y",
    ],
)

filegroup(
    name = "y",
    srcs = [":x"],
)

cc_library(
    name = "foo",
    srcs = [":y"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["//recursive:foo.cc"]),
        labels(&["//recursive:foo"])
    );
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnRecursiveNotFoundTarget
#[test]
fn test_compile_one_dep_on_recursive_not_found_target() {
    let mut t = new_test();
    t.scratch.file(
        "recursive/BUILD",
        &[r#"filegroup(
    name = "x",
    srcs = [":y"],
)

filegroup(
    name = "y",
    srcs = [":x"],
)

exports_files(["foo"])
"#],
    );
    assert_eq!(
        parse_error(&mut t, "//recursive:foo"),
        "Couldn't find dependency on target '//recursive:foo'"
    );
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnDeepRecursiveTarget
#[test]
fn test_compile_one_dep_on_deep_recursive_target() {
    let mut t = new_test();
    t.scratch.file(
        "recursive/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
filegroup(
    name = "x",
    srcs = [
        "foox.cc",
        ":y",
    ],
)

filegroup(
    name = "y",
    srcs = [
        "fooy.cc",
        ":z",
    ],
)

filegroup(
    name = "z",
    srcs = [
        "fooz.cc",
        ":x",
    ],
)

cc_library(
    name = "cc",
    srcs = [":x"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(
            &mut t,
            &[
                "//recursive:foox.cc",
                "//recursive:fooy.cc",
                "//recursive:fooy.cc"
            ]
        ),
        labels(&["//recursive:cc"])
    );
}

// Java: CompileOneDependencyTransformerTest.testCompileOneDepOnCrossPackageRecursiveTarget
#[test]
fn test_compile_one_dep_on_cross_package_recursive_target() {
    let mut t = new_test();
    t.scratch.file(
        "recursive/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
filegroup(
    name = "x",
    srcs = [
        "foo.cc",
        "//recursivetoo:x",
    ],
)

cc_library(
    name = "cc",
    srcs = [":x"],
)
"#],
    );
    t.scratch.file(
        "recursivetoo/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
filegroup(
    name = "x",
    srcs = [
        "foo.cc",
        "//recursive:x",
    ],
)

cc_library(
    name = "cc",
    srcs = [":x"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["//recursive:foo.cc", "//recursivetoo:foo.cc"]),
        labels(&["//recursive:cc", "//recursivetoo:cc"])
    );
}

/// Tests that when multiple rules match the target, the one that appears first in the BUILD
/// file is chosen.
// Java: CompileOneDependencyTransformerTest.testRuleChoiceOrdering
#[test]
fn test_rule_choice_ordering() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "foo_lib",
    srcs = ["file.cc"],
)

cc_library(
    name = "bar_lib",
    srcs = ["file.cc"],
)
"#],
    );
    t.scratch.file(
        "b/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
cc_library(
    name = "bar_lib",
    srcs = ["file.cc"],
)

cc_library(
    name = "foo_lib",
    srcs = ["file.cc"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/file.cc"]),
        labels(&["//a:foo_lib"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["b/file.cc"]),
        labels(&["//b:bar_lib"])
    );
}

/// Tests that when multiple rule match a target, language-specific rules take precedence.
// Java: CompileOneDependencyTransformerTest.testRuleChoiceLanguagePreferences
#[test]
fn test_rule_choice_language_preferences() {
    let mut t = new_test();
    let srcs = "srcs = [ 'a.cc', 'a.c', 'a.h', 'a.py', 'a.txt' ])";
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_cc//cc:cc_binary.bzl','cc_binary')",
            &format!("genrule(name = 'gen_rule', cmd = '', outs = [ 'out' ], {srcs}"),
            &format!("cc_binary(name = 'cc_rule', {srcs}"),
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.cc"]),
        labels(&["//a:cc_rule"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.c"]),
        labels(&["//a:cc_rule"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.h"]),
        labels(&["//a:cc_rule"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.txt"]),
        labels(&["//a:gen_rule"])
    );
}

// Java: CompileOneDependencyTransformerTest.testGeneratedFile
#[test]
fn test_generated_file() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
genrule(
    name = "gen_rule",
    outs = ["out.cc"],
    cmd = "",
)

cc_library(
    name = "cc",
    srcs = ["out.cc"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/out.cc"]),
        labels(&["//a:cc"])
    );
}

// Java: CompileOneDependencyTransformerTest.testGeneratedFileDepOnGenerator
#[test]
fn test_generated_file_dep_on_generator() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
genrule(
    name = "gen_rule",
    outs = ["out.cc"],
    cmd = "",
)

cc_library(
    name = "cc",
    srcs = [":gen_rule"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/out.cc"]),
        labels(&["//a:cc"])
    );
}

// Java: CompileOneDependencyTransformerTest.testHdrsFilegroup
#[test]
fn test_hdrs_filegroup() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
filegroup(
    name = "headers",
    srcs = ["a.h"],
)

cc_library(
    name = "cc",
    srcs = ["a.cc"],
    hdrs = [":headers"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.h"]),
        labels(&["//a:cc"])
    );
}

// Java: CompileOneDependencyTransformerTest.testConfigurableSrcs
#[test]
fn test_configurable_srcs() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
config_setting(
    name = "a",
    values = {"define": "foo=a"},
)

cc_library(
    name = "foo_select",
    srcs = select({
        ":a": ["b.cc"],
        ":b": ["c.cc"],
    }),
)

cc_library(
    name = "foo_always",
    srcs = ["a.cc"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.cc"]),
        labels(&["//a:foo_always"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/b.cc"]),
        labels(&["//a:foo_select"])
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/c.cc"]),
        labels(&["//a:foo_select"])
    );
}

// Java: CompileOneDependencyTransformerTest.testConfigurableCopts
#[test]
fn test_configurable_copts() {
    let mut t = new_test();
    // This configurable attribute doesn't preclude accurately knowing the srcs.
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
config_setting(
    name = "a",
    values = {"define": "foo=a"},
)

cc_library(
    name = "foo_select",
    srcs = ["a.cc"],
    copts = select({
        ":a": ["-DA"],
        ":b": ["-DB"],
    }),
)

cc_library(
    name = "foo_always",
    srcs = ["a.cc"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.cc"]),
        labels(&["//a:foo_select"])
    );
}

// Java: CompileOneDependencyTransformerTest.testHeaderOnlyLibrary
#[test]
fn test_header_only_library() {
    let mut t = new_test();
    // By default, we assume parse_headers is enabled (via --features + toolchain).
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['h.h'])",
            "cc_library(name = 'l', srcs = ['l.cc'], deps = [':h'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/h.h"]),
        labels(&["//a:h"])
    );

    // parse_headers explicitly disabled on the header-only target, use its reverse dep.
    t.scratch.file(
        "b/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['h.h'], features = ['-parse_headers'])",
            "cc_library(name = 'l', srcs = ['l.cc'], deps = [':h'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["b/h.h"]),
        labels(&["//b:l"])
    );

    // ... but if it has sources, the target itself is ok.
    t.scratch.file(
        "c/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['h.h'], srcs = ['h.cc'], features = ['-parse_headers'])",
            "cc_library(name = 'l', srcs = ['l.cc'], deps = [':h'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["c/h.h"]),
        labels(&["//c:h"])
    );

    // parse_headers disabled in the package
    t.scratch.file(
        "d/BUILD",
        &[
            "package(features = ['-parse_headers'])",
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['h.h'])",
            "cc_library(name = 'l', srcs = ['l.cc'], deps = [':h'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["d/h.h"]),
        labels(&["//d:l"])
    );

    // parse_headers disabled in the package and enabled on the target, so enabled
    t.scratch.file(
        "e/BUILD",
        &[
            "package(features = ['-parse_headers'])",
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['h.h'], features = ['parse_headers'])",
            "cc_library(name = 'l', srcs = ['l.cc'], deps = [':h'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["e/h.h"]),
        labels(&["//e:h"])
    );
}

// Java: CompileOneDependencyTransformerTest.testFallBackToHeaderOnlyLibrary
#[test]
fn test_fall_back_to_header_only_library() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[
            "load('@rules_cc//cc:cc_library.bzl', 'cc_library')",
            "cc_library(name = 'h', hdrs = ['a.h'], features = ['parse_headers'])",
        ],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.h"]),
        labels(&["//a:h"])
    );
}

// Java: CompileOneDependencyTransformerTest.doesNotCrashWhenPackageHasRuleWithDubiousSrcs
#[test]
fn does_not_crash_when_package_has_rule_with_dubious_srcs() {
    let mut t = new_test();
    t.scratch.file(
        "a/BUILD",
        &[r#"load("@rules_cc//cc:cc_library.bzl", "cc_library")
environment(name = "foo")

environment(name = "baz")

environment_group(
    name = "bar",
    defaults = [":baz"],
    environments = [
        ":baz",
        ":foo",
    ],
)

package_group(name = "pg")

cc_library(
    name = "h1",
    srcs = [
        ":bar",
        ":pg",
    ],
)

cc_library(
    name = "h2",
    hdrs = ["a.h"],
)
"#],
    );
    assert_eq!(
        parse_list_compile_one_dep(&mut t, &["a/a.h"]),
        labels(&["//a:h2"])
    );
}
