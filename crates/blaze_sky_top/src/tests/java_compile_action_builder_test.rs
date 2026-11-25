//! Port of `src/test/java/com/google/devtools/build/lib/rules/java/JavaCompileActionBuilderTest.java`
//! (with `JavaCompileActionTestHelper.getJavacArguments`), on the harness's mock `rules_java`.

use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_cmdline::repository_name::MAIN;
use blaze_rules::java::java_compile_action::JavaCompileAction;
use blaze_rules::java::java_compile_action_context::{
    Dependencies, Dependency, JavaCompileActionContext,
};

use crate::testing::BuildViewTestCase;

/// `(JavaCompileAction) getGeneratingActionForLabel(label)`, handed to `f`.
fn with_java_compile_action<R>(
    t: &mut BuildViewTestCase,
    label: &str,
    f: impl FnOnce(&JavaCompileAction) -> R,
) -> R {
    let action = t
        .get_generating_action_for_label(label)
        .unwrap_or_else(|| panic!("no generating action for {label}"));
    let action = action
        .as_any()
        .downcast_ref::<JavaCompileAction>()
        .unwrap_or_else(|| {
            panic!(
                "ClassCastException: {} is not a JavaCompileAction",
                action.get_mnemonic()
            )
        });
    f(action)
}

/// `JavaCompileActionTestHelper.getJavacArguments(action)`: the arguments after the JavaBuilder
/// command.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionTestHelper.java:98 JavaCompileActionTestHelper.getJavacArguments
fn get_javac_arguments(action: &JavaCompileAction) -> Vec<String> {
    let args = action
        .get_command_lines()
        .all_arguments()
        .unwrap_or_else(|e| panic!("{e:?}"));
    let index = main_class_index(&args);
    args[index..].to_vec()
}

/// Find the index of the last argument of the JavaBuilder command, and before the first option
/// that is passed to JavaBuilder.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileActionTestHelper.java:105 JavaCompileActionTestHelper.mainClassIndex
fn main_class_index(args: &[String]) -> usize {
    for (idx, arg) in args.iter().enumerate() {
        if arg == "-jar" {
            return idx + 2;
        }
        if arg.contains("JavaBuilder") && !arg.ends_with(".jar") {
            return idx + 1;
        }
    }
    panic!("IllegalStateException: {args:?}");
}

/// `MoreAsserts.assertContainsSublist(arguments, expectedSublist...)`.
fn assert_contains_sublist(arguments: &[String], expected_sublist: &[&str]) {
    let found = arguments
        .windows(expected_sublist.len())
        .any(|w| w.iter().zip(expected_sublist).all(|(a, b)| a == b));
    assert!(
        found,
        "Did not find {expected_sublist:?} as a sublist of {arguments:?}"
    );
}

// Java: JavaCompileActionBuilderTest.testClassdirIsInBlazeOut
#[test]
fn test_classdir_is_in_blaze_out() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_binary")
java_binary(
    name = "a",
    srcs = ["a.java"],
)
"#],
    );
    let command = with_java_compile_action(&mut t, "//java/com/google/test:a.jar", |action| {
        get_javac_arguments(action)
    });
    let output = t
        .get_target_configuration()
        .get_bin_fragment(&MAIN)
        .get_relative(&blaze_vfs::PathFragment::create("java/com/google/test/a.jar"))
        .get_path_string()
        .to_string();
    assert_contains_sublist(&command, &["--output", &output]);
}

// Java: JavaCompileActionBuilderTest.progressMessage
#[test]
fn progress_message() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
    name = "a",
    srcs = [
        "a.java",
        "b.java",
    ],
)
"#],
    );
    let message = with_java_compile_action(&mut t, "//java/com/google/test:liba.jar", |action| {
        action.get_progress_message(None)
    });
    assert_eq!(
        message.as_deref(),
        Some("Building java/com/google/test/liba.jar (2 source files)")
    );
}

// Java: JavaCompileActionBuilderTest.progressMessageWithSourceJars
#[test]
fn progress_message_with_source_jars() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
    name = "a",
    srcs = [
        "a.java",
        "archive.srcjar",
        "b.java",
    ],
)
"#],
    );
    let message = with_java_compile_action(&mut t, "//java/com/google/test:liba.jar", |action| {
        action.get_progress_message(None)
    });
    assert_eq!(
        message.as_deref(),
        Some("Building java/com/google/test/liba.jar (2 source files, 1 source jar)")
    );
}

// Java: JavaCompileActionBuilderTest.progressMessageAnnotationProcessors
#[test]
fn progress_message_annotation_processors() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[
            r#"load("@rules_java//java:defs.bzl", "java_library", "java_plugin")
java_plugin(
    name = "foo",
    srcs = ["Foo.java"],
    processor_class = "Foo",
)
java_plugin(
    name = "bar",
    srcs = ["Bar.java"],
    processor_class = "com.google.Bar",
)
java_library(
    name = "a",
    srcs = [
        "a.java",
        "archive.srcjar",
        "b.java",
    ],
    plugins = [
        ":foo",
        ":bar",
    ],
)
"#,
        ],
    );
    let message = with_java_compile_action(&mut t, "//java/com/google/test:liba.jar", |action| {
        action.get_progress_message(None)
    });
    assert_eq!(
        message.as_deref(),
        Some(
            "Building java/com/google/test/liba.jar (2 source files, 1 source jar) and running \
             annotation processors (Foo, Bar)"
        )
    );
}

// Java: JavaCompileActionBuilderTest.testLocale
#[test]
fn test_locale() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
    name = "a",
    srcs = ["A.java"],
)
"#],
    );
    let env = with_java_compile_action(&mut t, "//java/com/google/test:liba.jar", |action| {
        action.get_incomplete_environment_for_testing()
    });
    // analysisMock.isThisBazel()
    assert_eq!(env.get("LC_CTYPE").map(String::as_str), Some("C.UTF-8"));
}

// Java: JavaCompileActionBuilderTest.testClasspathReduction
#[test]
fn test_classpath_reduction() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "java/com/google/test/BUILD",
        &[r#"load("@rules_java//java:defs.bzl", "java_library")
java_library(
    name = "a",
    srcs = ["A.java"],
    deps = [":b"],
)
java_library(
    name = "b",
    srcs = ["B.java"],
    deps = [
        ":c",
        ":d",
    ],
)
java_library(
    name = "c",
    srcs = ["C.java"],
)
java_library(
    name = "d",
    srcs = ["D.java"],
)
"#],
    );
    let b = t.get_configured_target("//java/com/google/test:b").unwrap();
    let b_jdeps = t.get_bin_artifact("libb-hjar.jdeps", &b);
    let libc_jar = t
        .get_configured_target("//java/com/google/test:libc.jar")
        .unwrap();
    let c_hjar = t.get_bin_artifact("libc-hjar.jar", &libc_jar);
    let context = JavaCompileActionContext::new();
    let dep = Dependency {
        path: c_hjar.get_exec_path_string().as_bytes().to_vec(),
        // Kind.EXPLICIT
        kind: 0,
        location: Vec::new(),
    };
    context.insert_dependencies(
        &b_jdeps,
        Arc::new(Dependencies {
            dependency: vec![dep],
            ..Default::default()
        }),
    );
    let reduced = with_java_compile_action(&mut t, "//java/com/google/test:liba.jar", |action| {
        action
            .get_reduced_classpath(&null_action_execution_context::NullContext, &context)
            .unwrap_or_else(|e| panic!("{e}"))
    });
    let strings: Vec<String> = t
        .artifacts_to_strings(reduced.to_list().iter())
        .into_iter()
        .collect();
    crate::testing::assert_contains_exactly(
        &strings,
        &[
            "bin java/com/google/test/libb-hjar.jar",
            "bin java/com/google/test/libc-hjar.jar",
        ],
    );
}

// Java: JavaCompileActionBuilderTest.testExtraArgsPropagated
#[test]
#[ignore = "BLAZE GAP: JavaStarlarkCommon.createCompilationAction with non-empty extra_args is unported! (java_starlark_common.rs)"]
fn test_extra_args_propagated() {}

/// `new ActionExecutionContextBuilder().build()`: an execution context whose collaborators are
/// all null (a call to one of them is a NullPointerException).
mod null_action_execution_context {
    use blaze_events::{Clock, ExtendedEventHandler};
    use blaze_exec::actions::{ActionExecutionMetadata, InputMetadataProvider, Spawn};
    use blaze_exec::exec::action_execution_context::{
        ActionContextRegistry, ActionExecutionContext,
    };
    use blaze_exec::exec::spawn_runner::ProgressStatus;
    use blaze_exec::util::FileOutErr;
    use blaze_vfs::Path;
    use indexmap::IndexMap;

    pub struct NullContext;

    fn npe(what: &str) -> ! {
        panic!("NullPointerException: ActionExecutionContextBuilder().build().{what}")
    }

    impl ActionContextRegistry for NullContext {}

    impl ActionExecutionContext for NullContext {
        fn get_exec_root(&self) -> &Path {
            npe("getExecRoot")
        }
        fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
            npe("getInputMetadataProvider")
        }
        fn get_file_out_err(&self) -> &FileOutErr {
            npe("getFileOutErr")
        }
        fn get_client_env(&self) -> &IndexMap<String, String> {
            npe("getClientEnv")
        }
        fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
            npe("getEventHandler")
        }
        fn get_clock(&self) -> &dyn Clock {
            npe("getClock")
        }
        fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {
            npe("maybeReportSubcommand")
        }
        fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {
            npe("postProgress")
        }
    }
}
