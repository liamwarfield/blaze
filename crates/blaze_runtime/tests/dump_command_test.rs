//! Port of `src/test/java/com/google/devtools/build/lib/runtime/commands/DumpCommandTest.java`.

use blaze_buildtool_testing::BuildIntegrationTestCase;
use blaze_runtime::commands::dump::WARNING_MESSAGE;

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/DumpCommandTest.java:50 DumpCommandTest.dump
fn dump(t: &mut BuildIntegrationTestCase, args: &[&str]) -> bool {
    let mut params = vec!["dump"];
    params.extend_from_slice(args);
    t.dispatch(&params).is_success()
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/DumpCommandTest.java:56 DumpCommandTest.doesNotContainWarningInStdout
#[test]
fn does_not_contain_warning_in_stdout() {
    let mut t = BuildIntegrationTestCase::new();
    assert!(dump(&mut t, &["--skyframe", "count"]));
    assert!(t.out_err().err_as_latin1().contains(WARNING_MESSAGE));
    assert!(!t.out_err().out_as_latin1().contains(WARNING_MESSAGE));
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/DumpCommandTest.java:63 DumpCommandTest.multiOptionSmoke
#[test]
fn multi_option_smoke() {
    let mut t = BuildIntegrationTestCase::new();
    t.write(
        "foo/BUILD",
        &["genrule(name = 'foo', outs = ['out'], cmd = 'touch $@')"],
    );
    t.add_options(&["--nobuild"]);
    let built = t.build_target(&["//foo:foo"]).is_success();
    assert!(built, "{}", t.out_err().err_as_latin1());
    assert!(
        dump(
            &mut t,
            &["--rule_classes", "--rules", "--skyframe", "summary"]
        ),
        "{}",
        t.out_err().err_as_latin1()
    );
    let out = t.out_err().out_as_latin1();
    assert!(out.contains("filegroup"), "{out}");
    assert!(out.contains("RULE"), "{out}");
    assert!(out.contains("Node count"), "{out}");
}
