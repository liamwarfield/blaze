//! Ports of EvalMacroFunctionTest and PackagePieceTest (lazy symbolic macro expansion).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/EvalMacroFunctionTest.java and
//! src/test/java/com/google/devtools/build/lib/packages/PackagePieceTest.java. Java's workspace
//! is `/workspace`; expected texts use `PackageLoadingTestCase::workspace_path`.

use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_events::StoredEventHandler;
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::{
    PackagePiece, PackagePieceIdentifier, PackagePieceIdentifierForBuildFile,
    PackagePieceIdentifierForMacro,
};
use blaze_packages::target::{RuleOrMacroInstance, Target};
use blaze_packages::types::AttrValue;
use blaze_skyframe::{EvaluationResult, SkyKey};

use crate::package_pieces::{EvalMacroKey, NoSuchMacroInstanceException};
use crate::test_util::evaluate;
use crate::tests::package_loading_test_case::{PackageLoadingTestCase, assert_contains_event};

// Java: EvalMacroFunctionTest.getBuildFileKey
fn get_build_file_key(pkg: &str) -> PackagePieceIdentifierForBuildFile {
    PackagePieceIdentifierForBuildFile::new(PackageIdentifier::create_in_main_repo_str(pkg))
}

// Java: EvalMacroFunctionTest.getMacroKey
fn get_macro_key(pkg: &str, macro_instances: &[&str]) -> PackagePieceIdentifierForMacro {
    assert!(!macro_instances.is_empty());
    let build_file_key = get_build_file_key(pkg);
    let mut macro_key: Option<PackagePieceIdentifierForMacro> = None;
    for macro_instance in macro_instances {
        let parent = match macro_key.take() {
            Some(k) => PackagePieceIdentifier::ForMacro(k),
            None => PackagePieceIdentifier::ForBuildFile(build_file_key.clone()),
        };
        macro_key = Some(PackagePieceIdentifierForMacro::new(
            build_file_key.get_package_identifier().clone(),
            parent,
            (*macro_instance).to_owned(),
        ));
    }
    macro_key.unwrap()
}

// Java: EvalMacroFunctionTest.evaluate
fn eval(
    t: &PackageLoadingTestCase,
    key: &PackagePieceIdentifierForMacro,
) -> (EvaluationResult, Vec<blaze_events::Event>) {
    let evaluator = t.setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(EvalMacroKey(key.clone()))],
        false,
        events.clone(),
    );
    (result, events.get_events())
}

// Java: EvalMacroFunctionTest.getPackagePiece
fn get_package_piece(
    t: &PackageLoadingTestCase,
    pkg: &str,
    macro_instances: &[&str],
) -> (Arc<PackagePiece>, Vec<blaze_events::Event>) {
    let key = get_macro_key(pkg, macro_instances);
    let (result, events) = eval(t, &key);
    assert!(!result.has_error(), "{result:?}");
    let value = result
        .get(&EvalMacroKey(key.clone()))
        .expect("the piece")
        .for_macro
        .clone();
    assert_eq!(
        value.get_identifier(),
        PackagePieceIdentifier::ForMacro(key)
    );
    (value, events)
}

// Java: EvalMacroFunctionTest.getPackagePieceWithoutErrors
fn get_package_piece_without_errors(
    t: &PackageLoadingTestCase,
    pkg: &str,
    macro_instances: &[&str],
) -> Arc<PackagePiece> {
    let (value, events) = get_package_piece(t, pkg, macro_instances);
    assert!(
        !value.contains_errors(),
        "{:?}",
        events.iter().map(|e| e.to_string()).collect::<Vec<_>>()
    );
    value
}

/// Java `getExceptionForPackagePiece`: the error's message.
fn get_exception_message_for_package_piece<E: std::error::Error + 'static>(
    t: &PackageLoadingTestCase,
    pkg: &str,
    macro_instances: &[&str],
) -> String {
    let key = get_macro_key(pkg, macro_instances);
    let (result, _) = eval(t, &key);
    assert!(result.has_error());
    let error = result
        .get_error(&SkyKey::new(EvalMacroKey(key)))
        .expect("an error");
    let exception = error
        .exception_as::<E>()
        .unwrap_or_else(|| panic!("expected {}, got {:?}", std::any::type_name::<E>(), error));
    exception.to_string()
}

fn string_list_attr(target: &Target, attr: &str) -> Vec<String> {
    let rule = target.as_rule().expect("a rule");
    match rule.get_attr(attr) {
        Some(AttrValue::StringList(l)) => l,
        other => panic!("{attr}: {other:?}"),
    }
}

// Java: EvalMacroFunctionTest.validMacro
#[test]
fn valid_macro() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "my_macro = macro(implementation = _impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_macro.bzl\", \"my_macro\")",
            "my_macro(name = \"foo\")",
        ],
    );
    let for_macro = get_package_piece_without_errors(&t, "pkg", &["foo"]);
    assert!(for_macro.get_targets().contains_key("foo"));
}

// Java: EvalMacroFunctionTest.validNestedMacro (suffix '' and '_inner')
#[test]
fn valid_nested_macro() {
    for suffix in ["", "_inner"] {
        let t = PackageLoadingTestCase::new();
        t.scratch_file(
            "pkg/inner_macro.bzl",
            &[
                "def _impl(name, visibility):",
                "    native.filegroup(name = name, visibility = visibility)",
                "inner_macro = macro(implementation = _impl)",
            ],
        );
        let line = format!("    inner_macro(name = name + \"{suffix}\", visibility = visibility)");
        t.scratch_file(
            "pkg/outer_macro.bzl",
            &[
                "load(\":inner_macro.bzl\", \"inner_macro\")",
                "",
                "def _impl(name, visibility):",
                &line,
                "outer_macro = macro(implementation = _impl)",
            ],
        );
        t.scratch_file(
            "pkg/BUILD",
            &[
                "load(\":outer_macro.bzl\", \"outer_macro\")",
                "outer_macro(name = \"foo\")",
            ],
        );
        let inner_macro_instance_name = format!("foo{suffix}");
        let for_inner_macro =
            get_package_piece_without_errors(&t, "pkg", &["foo", &inner_macro_instance_name]);
        assert!(
            for_inner_macro
                .get_targets()
                .contains_key(&inner_macro_instance_name)
        );
        let for_outer_macro = get_package_piece_without_errors(&t, "pkg", &["foo"]);
        // PORT: Java asserts the same instance across two evaluations of one Skyframe graph; each
        // evaluation here is from scratch, so the macro instances are compared by id and class.
        let a = for_outer_macro
            .get_macro_by_name(&inner_macro_instance_name)
            .expect("the inner macro");
        let b = for_inner_macro
            .get_evaluated_macro()
            .expect("evaluated macro");
        assert_eq!(a.get_id(), b.get_id());
        assert_eq!(
            a.get_macro_class().get_name(),
            b.get_macro_class().get_name()
        );
    }
}

// Java: EvalMacroFunctionTest.innerAndSiblingMacros_notExpandedUnlessRequested
#[test]
#[ignore = "needs SkyframeExecutor.getDoneSkyValueForIntrospection (looking up done nodes of a kept graph); the harness evaluates from scratch and does not expose the graph"]
fn inner_and_sibling_macros_not_expanded_unless_requested() {}

// Java: EvalMacroFunctionTest.buildFileChange_prunedAtMacroInstance
#[test]
#[ignore = "asserts change pruning on a kept Skyframe graph after a BUILD file edit (incremental evaluation is not ported)"]
fn build_file_change_pruned_at_macro_instance() {}

// Java: EvalMacroFunctionTest.maxComputationSteps_enforced
#[test]
#[ignore = "starlark-rust does not count Java's computation steps (StarlarkThread.getExecutedSteps); --max_computation_steps fails loudly (PackageFactory.afterDoneLoadingPackagePiece)"]
fn max_computation_steps_enforced() {}

// Java: EvalMacroFunctionTest.noBuildFile_failsCleanly
#[test]
fn no_build_file_fails_cleanly() {
    let t = PackageLoadingTestCase::new();
    let message = get_exception_message_for_package_piece::<NoSuchPackageException>(
        &t,
        "no_such_pkg",
        &["foo"],
    );
    assert!(
        message.contains("no such package 'no_such_pkg': BUILD file not found"),
        "{message}"
    );
}

// Java: EvalMacroFunctionTest.badBuildFile_failsCleanly
#[test]
fn bad_build_file_fails_cleanly() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "my_macro = macro(implementation = _impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_macro.bzl\", \"my_macro\")",
            "load(\":bad_load.bzl\", \"bad_value\")",
            "my_macro(name = \"foo\")",
        ],
    );
    let message =
        get_exception_message_for_package_piece::<NoSuchPackageException>(&t, "pkg", &["foo"]);
    assert!(
        message.contains("cannot load '//pkg:bad_load.bzl': no such file"),
        "{message}"
    );
}

// Java: EvalMacroFunctionTest.noMacroInstance_failsCleanly
#[test]
fn no_macro_instance_fails_cleanly() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "my_macro = macro(implementation = _impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_macro.bzl\", \"my_macro\")",
            "my_macro(name = \"foo\")",
        ],
    );
    let message = get_exception_message_for_package_piece::<NoSuchMacroInstanceException>(
        &t,
        "pkg",
        &["no_such_name"],
    );
    assert!(
        message.contains("Macro instance 'no_such_name' not found in top-level package piece"),
        "{message}"
    );
}

// Java: EvalMacroFunctionTest.badMacroImplementation_producesPackagePieceWithErrors
#[test]
fn bad_macro_implementation_produces_package_piece_with_errors() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "    fail(\"fail fail fail\")",
            "my_macro = macro(implementation = _impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_macro.bzl\", \"my_macro\")",
            "my_macro(name = \"foo\")",
        ],
    );
    let (for_macro, events) = get_package_piece(&t, "pkg", &["foo"]);
    assert!(for_macro.contains_errors());
    assert!(
        for_macro
            .get_target("foo")
            .unwrap()
            .as_rule()
            .unwrap()
            .contains_errors()
    );
    let bzl = t.workspace_path("pkg/my_macro.bzl");
    let build = t.workspace_path("pkg/BUILD");
    assert_contains_event(
        &events,
        &format!(
            "ERROR {bzl}:3:9: Traceback (most recent call last):\n\
             \tFile \"{build}\", line 2, column 9, in <toplevel>\n\
             \t\tmy_macro(name = \"foo\")\n\
             \tFile \"{bzl}\", line 4, column 1, in my_macro\n\
             \t\tmy_macro = macro(implementation = _impl)\n\
             \tFile \"{bzl}\", line 3, column 9, in _impl\n\
             \t\tfail(\"fail fail fail\")\n\
             Error in fail: fail fail fail"
        ),
    );
}

// Java: EvalMacroFunctionTest.finalizers_seeNonFinalizerDefinedRulesOrderedByName
#[test]
fn finalizers_see_non_finalizer_defined_rules_ordered_by_name() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_finalizer.bzl",
        &[
            "# Dummy rule used to save native.existing_rules() keys in a string list attribute.",
            "_existing_rules_saver = rule(",
            "    implementation = lambda ctx: [],",
            "    attrs = {\"existing_rules\": attr.string_list()},",
            ")",
            "",
            "def _impl(name, visibility):",
            "    _existing_rules_saver(name = name, existing_rules = list(native.existing_rules()))",
            "my_finalizer = macro(implementation = _impl, finalizer = True)",
        ],
    );
    t.scratch_file(
        "pkg/other_macro.bzl",
        &[
            "def _other_inner_macro_impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "other_inner_macro = macro(implementation = _other_inner_macro_impl)",
            "",
            "def _other_macro_impl(name, visibility):",
            "    other_inner_macro(name = name + \"_c_inner\", visibility = visibility)",
            "    native.filegroup(name = name + \"_b\", visibility = visibility)",
            "    other_inner_macro(name = name + \"_a_inner\", visibility = visibility)",
            "other_macro = macro(implementation = _other_macro_impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_finalizer.bzl\", \"my_finalizer\")",
            "load(\":other_macro.bzl\", \"other_macro\")",
            "my_finalizer(name = \"finalize\")",
            "other_macro(name = \"macro_declared\")",
            "filegroup(name = \"a_top_level\")",
            "filegroup(name = \"z_top_level\")",
        ],
    );
    let finalizer_piece = get_package_piece_without_errors(&t, "pkg", &["finalize"]);
    let existing_rules = string_list_attr(
        finalizer_piece.get_target("finalize").unwrap(),
        "existing_rules",
    );
    assert_eq!(
        existing_rules,
        vec![
            // Ordered by name.
            "a_top_level",
            "macro_declared_a_inner",
            "macro_declared_b",
            "macro_declared_c_inner",
            "z_top_level",
        ]
    );
}

// Java: EvalMacroFunctionTest.finalizers_doNotSeeFinalizerDefinedTargets
#[test]
fn finalizers_do_not_see_finalizer_defined_targets() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_finalizer.bzl",
        &[
            "# Dummy rule used to save native.existing_rules() keys in a string list attribute.",
            "_existing_rules_saver = rule(",
            "    implementation = lambda ctx: [],",
            "    attrs = {\"existing_rules\": attr.string_list()},",
            ")",
            "",
            "def _impl(name, visibility):",
            "    native.filegroup(name = name + \"_dummy_rule\")",
            "    _existing_rules_saver(name = name, existing_rules = list(native.existing_rules()))",
            "my_finalizer = macro(implementation = _impl, finalizer = True)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_finalizer.bzl\", \"my_finalizer\")",
            "my_finalizer(name = \"finalize\")",
            "my_finalizer(name = \"other_finalize\")",
        ],
    );
    let finalizer_piece = get_package_piece_without_errors(&t, "pkg", &["finalize"]);
    assert!(
        string_list_attr(
            finalizer_piece.get_target("finalize").unwrap(),
            "existing_rules"
        )
        .is_empty()
    );
    let other_finalizer_piece = get_package_piece_without_errors(&t, "pkg", &["other_finalize"]);
    assert!(
        string_list_attr(
            other_finalizer_piece.get_target("other_finalize").unwrap(),
            "existing_rules"
        )
        .is_empty()
    );
}

/// Java `assertContainsEventsInOrder`.
fn assert_contains_events_in_order(events: &[blaze_events::Event], expected: &[&str]) {
    let strings: Vec<String> = events.iter().map(|e| e.to_string()).collect();
    let mut i = 0;
    for s in &strings {
        if i < expected.len() && s.contains(expected[i]) {
            i += 1;
        }
    }
    assert_eq!(
        i,
        expected.len(),
        "events in order {expected:#?}; got {strings:#?}"
    );
}

// Java: EvalMacroFunctionTest.finalizers_notEvaluated_ifNonFinalizerPackagePieceInError
#[test]
fn finalizers_not_evaluated_if_non_finalizer_package_piece_in_error() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_finalizer.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name + \"_saw_rules\", srcs = native.existing_rules())",
            "my_finalizer = macro(implementation = _impl, finalizer = True)",
        ],
    );
    t.scratch_file(
        "pkg/fail_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name, visibility = visibility)",
            "    fail(\"fail fail fail\")",
            "",
            "fail_macro = macro(implementation = _impl)",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":fail_macro.bzl\", \"fail_macro\")",
            "load(\":my_finalizer.bzl\", \"my_finalizer\")",
            "my_finalizer(name = \"finalize\")",
            "filegroup(name = \"top_level_rule\")",
            "fail_macro(name = \"failing_macro\")",
        ],
    );
    let (finalizer_piece, events) = get_package_piece(&t, "pkg", &["finalize"]);
    assert!(finalizer_piece.contains_errors());
    assert!(finalizer_piece.get_targets().is_empty());
    let message = finalizer_piece
        .get_failure_detail()
        .expect("a failure detail")
        .message
        .clone();
    assert!(
        message.contains(
            "cannot compute package piece for finalizer macro //pkg:finalize defined by \
             //pkg:my_finalizer.bzl%my_finalizer: error in package piece for macro \
             //pkg:failing_macro defined by //pkg:fail_macro.bzl%fail_macro"
        ),
        "{message}"
    );
    assert!(
        get_package_piece(&t, "pkg", &["failing_macro"])
            .0
            .contains_errors()
    );
    let build = t.workspace_path("pkg/BUILD");
    let bzl = t.workspace_path("pkg/fail_macro.bzl");
    assert_contains_events_in_order(
        &events,
        &[
            &format!(
                "Traceback (most recent call last):\n\
                 \tFile \"{build}\", line 5, column 11, in <toplevel>\n\
                 \t\tfail_macro(name = \"failing_macro\")\n\
                 \tFile \"{bzl}\", line 5, column 1, in fail_macro\n\
                 \t\tfail_macro = macro(implementation = _impl)\n\
                 \tFile \"{bzl}\", line 3, column 9, in _impl\n\
                 \t\tfail(\"fail fail fail\")\n\
                 Error in fail: fail fail fail"
            ),
            "cannot compute package piece for finalizer macro //pkg:finalize defined by \
             //pkg:my_finalizer.bzl%my_finalizer",
        ],
    );
}

// Java: EvalMacroFunctionTest.finalizers_notEvaluated_ifNameConflictBetweenNonFinalizerPackagePieces
#[test]
fn finalizers_not_evaluated_if_name_conflict_between_non_finalizer_package_pieces() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/my_finalizer.bzl",
        &[
            "def _impl(name, visibility):",
            "    native.filegroup(name = name + \"_saw_rules\", srcs = native.existing_rules())",
            "my_finalizer = macro(implementation = _impl, finalizer = True)",
        ],
    );
    t.scratch_file(
        "pkg/name_conflict_macro.bzl",
        &[
            "def _impl(name, suffix, visibility):",
            "    native.filegroup(name = name + suffix, visibility = visibility)",
            "",
            "name_conflict_macro = macro(",
            "    implementation = _impl,",
            "    attrs = {\"suffix\": attr.string(configurable = False)},",
            ")",
        ],
    );
    t.scratch_file(
        "pkg/BUILD",
        &[
            "load(\":my_finalizer.bzl\", \"my_finalizer\")",
            "load(\":name_conflict_macro.bzl\", \"name_conflict_macro\")",
            "my_finalizer(name = \"finalize\")",
            "filegroup(name = \"top_level_rule\")",
            "name_conflict_macro(name = \"top\", suffix = \"_level_rule\")",
        ],
    );
    let (finalizer_piece, events) = get_package_piece(&t, "pkg", &["finalize"]);
    assert!(finalizer_piece.contains_errors());
    assert!(finalizer_piece.get_targets().is_empty());
    let message = finalizer_piece
        .get_failure_detail()
        .expect("a failure detail")
        .message
        .clone();
    assert!(
        message.contains(
            "cannot compute package piece for finalizer macro //pkg:finalize defined by \
             //pkg:my_finalizer.bzl%my_finalizer: filegroup rule 'top_level_rule' conflicts \
             with existing filegroup rule"
        ),
        "{message}"
    );
    // Note that individual non-finalizer package pieces are not in error - the conflict is in
    // the NonFinalizerPackagePiecesValue.
    get_package_piece_without_errors(&t, "pkg", &["top"]);
    let build = t.workspace_path("pkg/BUILD");
    let bzl = t.workspace_path("pkg/name_conflict_macro.bzl");
    assert_contains_events_in_order(
        &events,
        &[
            &format!(
                "Traceback (most recent call last):\n\
                 \tFile \"{build}\", line 5, column 20, in <toplevel>\n\
                 \t\tname_conflict_macro(name = \"top\", suffix = \"_level_rule\")\n\
                 \tFile \"{bzl}\", line 4, column 1, in name_conflict_macro\n\
                 \t\tname_conflict_macro = macro(\n\
                 \tFile \"{bzl}\", line 2, column 21, in _impl\n\
                 \t\tnative.filegroup(name = name + suffix, visibility = visibility)\n\
                 Error: filegroup rule 'top_level_rule' conflicts with existing filegroup rule, \
                 defined at {build}:4:10"
            ),
            "cannot compute package piece for finalizer macro //pkg:finalize defined by \
             //pkg:my_finalizer.bzl%my_finalizer",
        ],
    );
}

// Java: PackagePieceTest.identifier_equality
#[test]
fn identifier_equality() {
    let pkg = || PackageIdentifier::create_in_main_repo_str("test_pkg");
    let build =
        || PackagePieceIdentifier::ForBuildFile(PackagePieceIdentifierForBuildFile::new(pkg()));
    let foo = || {
        PackagePieceIdentifier::ForMacro(PackagePieceIdentifierForMacro::new(
            pkg(),
            build(),
            "foo".to_owned(),
        ))
    };
    let foo_bar = || {
        PackagePieceIdentifier::ForMacro(PackagePieceIdentifierForMacro::new(
            pkg(),
            foo(),
            "foo_bar".to_owned(),
        ))
    };
    let other_repo = PackagePieceIdentifier::ForBuildFile(PackagePieceIdentifierForBuildFile::new(
        PackageIdentifier::parse("@repo//test_pkg").unwrap(),
    ));
    crate::tests::equals_tester(&[
        vec![build(), build()],
        vec![other_repo],
        vec![foo(), foo()],
        vec![foo_bar(), foo_bar()],
    ]);
}

// Java: PackagePieceTest.packagePieceForBuildFileBuilder_basicFunctionality
#[test]
#[ignore = "builds a piece directly with PackagePiece.ForBuildFile.newBuilder and a test PackageSettings / rule class provider; not ported yet (the piece builders are covered through EVAL_MACRO and PACKAGE above)"]
fn package_piece_for_build_file_builder_basic_functionality() {}

// Java: PackagePieceTest.packagePieceForMacroBuilder_basicFunctionality
#[test]
#[ignore = "builds a piece directly with PackagePiece.ForMacro.newBuilder and a hand-made MacroInstance; not ported yet (the piece builders are covered through EVAL_MACRO and PACKAGE above)"]
fn package_piece_for_macro_builder_basic_functionality() {}
