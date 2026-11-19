//! Port of src/test/cpp/rc_options_test.cc.

mod rc_support;

use std::cell::RefCell;

use blaze_options::rc::rc_file::{ParseError, RcFile, make_canonical, read_file_default};
use blaze_options::rc::sem_ver::SemVer;
use rc_support::*;

/// Java: rc_options_test.cc RcOptionsTest fixture.
struct RcOptionsTest {
    test_file_dir: String,
}

impl RcOptionsTest {
    fn new(name: &str) -> Self {
        RcOptionsTest {
            test_file_dir: fresh_tmpdir(name),
        }
    }

    fn write_rc(&self, filename: &str, contents: &str) {
        std::fs::write(join(&self.test_file_dir, filename), contents).unwrap();
    }

    fn parse(&self, filename: &str) -> Result<RcFile, (ParseError, String)> {
        let test_build_label = "8.4.2";
        let sem_ver = SemVer::parse(test_build_label);
        RcFile::parse(
            &join(&self.test_file_dir, filename),
            // Set workspace to test_file_dir so importing %workspace%/foo works.
            &self.test_file_dir,
            test_build_label,
            sem_ver.as_ref(),
            &read_file_default,
            &make_canonical,
        )
    }

    fn successfully_parse_rc_with_expected_args(
        &self,
        filename: &str,
        expected_args_map: &[(&str, &[&str])],
    ) {
        let rc = match self.parse(filename) {
            Ok(rc) => rc,
            Err((e, text)) => panic!("{:?}: {}", e, text),
        };
        let result = rc.options();
        assert_eq!(expected_args_map.len(), result.len());
        for (expected_command, expected_args) in expected_args_map {
            let result_args = result
                .get(*expected_command)
                .unwrap_or_else(|| panic!("missing command {}", expected_command));
            let got: Vec<&str> = result_args.iter().map(|o| o.option.as_str()).collect();
            assert_eq!(&got, expected_args);
        }
    }
}

macro_rules! rc_test {
    ($name:ident, [$(($file:expr, $contents:expr)),* $(,)?], $parse:expr, [$(($cmd:expr, [$($arg:expr),* $(,)?])),* $(,)?]) => {
        #[test]
        fn $name() {
            let t = RcOptionsTest::new(stringify!($name));
            $( t.write_rc($file, $contents); )*
            t.successfully_parse_rc_with_expected_args(
                $parse,
                &[$(($cmd, &[$($arg),*] as &[&str])),*],
            );
        }
    };
}

// Java: rc_options_test.cc RcOptionsTest.Empty
rc_test!(empty, [("empty.bazelrc", "")], "empty.bazelrc", []);

// Java: rc_options_test.cc RcOptionsTest.Whitespace
rc_test!(
    whitespace,
    [("whitespace.bazelrc", "      \n\t      ")],
    "whitespace.bazelrc",
    []
);

// Java: rc_options_test.cc RcOptionsTest.CommentedStartup
rc_test!(
    commented_startup,
    [("commented_startup.bazelrc", "# startup foo")],
    "commented_startup.bazelrc",
    []
);

// Java: rc_options_test.cc RcOptionsTest.EmptyStartupLine
rc_test!(
    empty_startup_line,
    [("empty_startup_line.bazelrc", "startup")],
    "empty_startup_line.bazelrc",
    []
);

// Java: rc_options_test.cc RcOptionsTest.StartupWithOnlyCommentedArg
rc_test!(
    startup_with_only_commented_arg,
    [("startup_with_comment.bazelrc", "startup # bar")],
    "startup_with_comment.bazelrc",
    []
);

// Java: rc_options_test.cc RcOptionsTest.SingleStartupArg
rc_test!(
    single_startup_arg,
    [("startup_foo.bazelrc", "startup foo")],
    "startup_foo.bazelrc",
    [("startup", ["foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.SingleStartupArgWithComment
rc_test!(
    single_startup_arg_with_comment,
    [("startup_foo_and_comment.bazelrc", "startup foo # comment")],
    "startup_foo_and_comment.bazelrc",
    [("startup", ["foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.TwoStartupArgsOnOneLine
rc_test!(
    two_startup_args_on_one_line,
    [("startup_foo_bar.bazelrc", "startup foo bar")],
    "startup_foo_bar.bazelrc",
    [("startup", ["foo", "bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.TwoStartupArgsOnOneLineTabSeparated
rc_test!(
    two_startup_args_on_one_line_tab_separated,
    [("startup_with_tabs.bazelrc", "startup\tfoo\tbar")],
    "startup_with_tabs.bazelrc",
    [("startup", ["foo", "bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.StartupOptWithSimpleValue
rc_test!(
    startup_opt_with_simple_value,
    [("startup_opt_with_simple_value.bazelrc", "startup --opt=foo")],
    "startup_opt_with_simple_value.bazelrc",
    [("startup", ["--opt=foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.StartupQuotedArg
rc_test!(
    startup_quoted_arg,
    [("startup_quoted_foo_bar.bazelrc", "startup \"foo bar\"")],
    "startup_quoted_foo_bar.bazelrc",
    [("startup", ["foo bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.QuotedValueStartupArgAfterEquals
rc_test!(
    quoted_value_startup_arg_after_equals,
    [(
        "startup_opt_quoted_arg.bazelrc",
        "startup --opt=\"foo bar\""
    )],
    "startup_opt_quoted_arg.bazelrc",
    [("startup", ["--opt=foo bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.QuotedValueStartupArgAfterWhitespace
rc_test!(
    quoted_value_startup_arg_after_whitespace,
    [(
        "startup_opt_quoted_arg_as_separate_token.bazelrc",
        "startup --opt \"foo bar\""
    )],
    "startup_opt_quoted_arg_as_separate_token.bazelrc",
    [("startup", ["--opt", "foo bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.QuotedValueStartupArgOnNewLine
rc_test!(
    quoted_value_startup_arg_on_new_line,
    [(
        "startup_opt_quoted_arg_different_line.bazelrc",
        "startup --opt\nstartup \"foo bar\""
    )],
    "startup_opt_quoted_arg_different_line.bazelrc",
    [("startup", ["--opt", "foo bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.TwoOptStartup
rc_test!(
    two_opt_startup,
    [(
        "startup_two_options.bazelrc",
        "startup --opt1\nstartup --opt2"
    )],
    "startup_two_options.bazelrc",
    [("startup", ["--opt1", "--opt2"])]
);

// Java: rc_options_test.cc RcOptionsTest.WhitespaceBeforeStartup
rc_test!(
    whitespace_before_startup,
    [(
        "whitespace_before_command.bazelrc",
        "  startup foo\n        # indented comments\nstartup bar\n\tstartup     \t baz"
    )],
    "whitespace_before_command.bazelrc",
    [("startup", ["foo", "bar", "baz"])]
);

// Java: rc_options_test.cc RcOptionsTest.StartupLineContinuation
rc_test!(
    startup_line_continuation,
    [(
        "startup_line_continuation.bazelrc",
        "startup \\\nfoo\nstartup bar \\\nbaz"
    )],
    "startup_line_continuation.bazelrc",
    [("startup", ["foo", "bar", "baz"])]
);

// Java: rc_options_test.cc RcOptionsTest.ManyArgStartup
rc_test!(
    many_arg_startup,
    [(
        "startup_with_many_args.bazelrc",
        "# Many arguments\n\
         startup foo # First argument has reasons.\n\
         startup --opt1   --opt2 # These arguments are split wide\n\
         #startup --this_is_not_an_arg\n\
         \n\n\n # A few empty lines for good measure\n\
         startup\t \"string input Value 123. \" --bar"
    )],
    "startup_with_many_args.bazelrc",
    [(
        "startup",
        [
            "foo",
            "--opt1",
            "--opt2",
            "string input Value 123. ",
            "--bar"
        ]
    )]
);

// Java: rc_options_test.cc RcOptionsTest.MultipleCommands
rc_test!(
    multiple_commands,
    [(
        "multiple_commands_intermixed.bazelrc",
        "startup foo\nbuild aaa\nstartup bar baz\nbuild bbb\nbuild ccc\n"
    )],
    "multiple_commands_intermixed.bazelrc",
    [
        ("startup", ["foo", "bar", "baz"]),
        ("build", ["aaa", "bbb", "ccc"])
    ]
);

// Java: rc_options_test.cc RcOptionsTest.SimpleImportFoo
rc_test!(
    simple_import_foo,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "import_simple.bazelrc",
            "import %workspace%/startup_foo.bazelrc"
        ),
    ],
    "import_simple.bazelrc",
    [("startup", ["foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.ImportFooThenAddBar
rc_test!(
    import_foo_then_add_bar,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "import_foo_then_bar.bazelrc",
            "import %workspace%/startup_foo.bazelrc\nstartup bar"
        ),
    ],
    "import_foo_then_bar.bazelrc",
    [("startup", ["foo", "bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.StartupBarThenImportFoo
rc_test!(
    startup_bar_then_import_foo,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "bar_then_import_foo.bazelrc",
            "startup bar\nimport %workspace%/startup_foo.bazelrc"
        ),
    ],
    "bar_then_import_foo.bazelrc",
    [("startup", ["bar", "foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.SimpleTryImportFoo
rc_test!(
    simple_try_import_foo,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "import_simple.bazelrc",
            "try-import %workspace%/startup_foo.bazelrc"
        ),
    ],
    "import_simple.bazelrc",
    [("startup", ["foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.ImportTryFooThenAddBar
rc_test!(
    import_try_foo_then_add_bar,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "import_foo_then_bar.bazelrc",
            "try-import %workspace%/startup_foo.bazelrc\nstartup bar"
        ),
    ],
    "import_foo_then_bar.bazelrc",
    [("startup", ["foo", "bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.StartupBarThenTryImportFoo
rc_test!(
    startup_bar_then_try_import_foo,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "bar_then_import_foo.bazelrc",
            "startup bar\ntry-import %workspace%/startup_foo.bazelrc"
        ),
    ],
    "bar_then_import_foo.bazelrc",
    [("startup", ["bar", "foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.ImportDiamond
rc_test!(
    import_diamond,
    [
        ("startup_foo.bazelrc", "startup foo"),
        (
            "import_foo_then_bar.bazelrc",
            "import %workspace%/startup_foo.bazelrc\nstartup bar"
        ),
        (
            "bar_then_import_foo.bazelrc",
            "startup bar\nimport %workspace%/startup_foo.bazelrc"
        ),
        (
            "import_diamond.bazelrc",
            "import %workspace%/import_foo_then_bar.bazelrc\nimport %workspace%/bar_then_import_foo.bazelrc"
        ),
    ],
    "import_diamond.bazelrc",
    [("startup", ["foo", "bar", "bar", "foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.ImportCycleFails
#[test]
fn import_cycle_fails() {
    let t = RcOptionsTest::new("import_cycle_fails");
    t.write_rc(
        "import_cycle_1.bazelrc",
        "import %workspace%/import_cycle_2.bazelrc",
    );
    t.write_rc(
        "import_cycle_2.bazelrc",
        "import %workspace%/import_cycle_1.bazelrc",
    );
    let (error, error_text) = t.parse("import_cycle_1.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::ImportLoop);
    assert!(
        matches_regex(
            &error_text,
            "Import loop detected:\n  .*import_cycle_1.bazelrc\n  .*import_cycle_2.bazelrc\n  .*import_cycle_1.bazelrc\n"
        ),
        "{}",
        error_text
    );
}

// Java: rc_options_test.cc RcOptionsTest.LongImportCycleFails
#[test]
fn long_import_cycle_fails() {
    let t = RcOptionsTest::new("long_import_cycle_fails");
    t.write_rc(
        "chain_to_cycle_1.bazelrc",
        "import %workspace%/chain_to_cycle_2.bazelrc",
    );
    t.write_rc(
        "chain_to_cycle_2.bazelrc",
        "import %workspace%/chain_to_cycle_3.bazelrc",
    );
    t.write_rc(
        "chain_to_cycle_3.bazelrc",
        "import %workspace%/chain_to_cycle_4.bazelrc",
    );
    t.write_rc(
        "chain_to_cycle_4.bazelrc",
        "import %workspace%/import_cycle_1.bazelrc",
    );
    t.write_rc(
        "import_cycle_1.bazelrc",
        "import %workspace%/import_cycle_2.bazelrc",
    );
    t.write_rc(
        "import_cycle_2.bazelrc",
        "import %workspace%/import_cycle_1.bazelrc",
    );
    let (error, error_text) = t.parse("chain_to_cycle_1.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::ImportLoop);
    assert!(
        matches_regex(
            &error_text,
            "Import loop detected:\n  .*chain_to_cycle_1.bazelrc\n  .*chain_to_cycle_2.bazelrc\n  .*chain_to_cycle_3.bazelrc\n  .*chain_to_cycle_4.bazelrc\n  .*import_cycle_1.bazelrc\n  .*import_cycle_2.bazelrc\n  .*import_cycle_1.bazelrc\n"
        ),
        "{}",
        error_text
    );
}

// Java: rc_options_test.cc RcOptionsTest.FileDoesNotExist
#[test]
fn file_does_not_exist() {
    let t = RcOptionsTest::new("file_does_not_exist");
    let (error, error_text) = t.parse("not_a_file.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::UnreadableFile);
    assert!(
        matches_regex(
            &error_text,
            "Unexpected error reading config file '.*not_a_file\\.bazelrc': \\(error: 2\\): No such file or directory"
        ),
        "{}",
        error_text
    );
}

// Java: rc_options_test.cc RcOptionsTest.ImportedFileDoesNotExist
#[test]
fn imported_file_does_not_exist() {
    let t = RcOptionsTest::new("imported_file_does_not_exist");
    t.write_rc("import_fake_file.bazelrc", "import somefile");
    let (error, error_text) = t.parse("import_fake_file.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::UnreadableFile);
    assert_eq!(
        error_text,
        "Unexpected error reading config file 'somefile': (error: 2): No such file or directory"
    );
}

// Java: rc_options_test.cc RcOptionsTest.TryImportedFileDoesNotExist
rc_test!(
    try_imported_file_does_not_exist,
    [("try_import_fake_file.bazelrc", "try-import somefile")],
    "try_import_fake_file.bazelrc",
    []
);

// Java: rc_options_test.cc RcOptionsTest.ImportHasTooManyArgs
#[test]
fn import_has_too_many_args() {
    let t = RcOptionsTest::new("import_has_too_many_args");
    t.write_rc("bad_import.bazelrc", "import somefile bar");
    let (error, error_text) = t.parse("bad_import.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::InvalidFormat);
    assert!(matches_regex(
        &error_text,
        "Invalid import declaration in config file '.*bad_import.bazelrc': 'import somefile bar'"
    ));
}

// Java: rc_options_test.cc RcOptionsTest.TryImportHasTooManyArgs
#[test]
fn try_import_has_too_many_args() {
    let t = RcOptionsTest::new("try_import_has_too_many_args");
    t.write_rc("bad_import.bazelrc", "try-import somefile bar");
    let (error, error_text) = t.parse("bad_import.bazelrc").unwrap_err();
    assert_eq!(error, ParseError::InvalidFormat);
    assert!(matches_regex(
        &error_text,
        "Invalid import declaration in config file '.*bad_import.bazelrc': 'try-import somefile bar'"
    ));
}

// Java: rc_options_test.cc RcOptionsTest.BadStartupLineContinuation_HasWhitespaceAfterSlash
rc_test!(
    bad_startup_line_continuation_has_whitespace_after_slash,
    [(
        "bad_startup_line_continuation.bazelrc",
        "startup foo \\ \nbar"
    )],
    "bad_startup_line_continuation.bazelrc",
    // Does not contain "bar" from the next line.
    [("startup", ["foo"])]
);

// Java: rc_options_test.cc RcOptionsTest.BadStartupLineContinuation_HasErroneousSlash
rc_test!(
    bad_startup_line_continuation_has_erroneous_slash,
    [(
        "bad_startup_line_continuation.bazelrc",
        "startup foo \\ bar"
    )],
    "bad_startup_line_continuation.bazelrc",
    // Whitespace between the slash and bar gets counted as part of the token.
    [("startup", ["foo", " bar"])]
);

// Java: rc_options_test.cc RcOptionsTest.BadStartupLineContinuation_HasCommentAfterSlash
rc_test!(
    bad_startup_line_continuation_has_comment_after_slash,
    [(
        "bad_startup_line_continuation.bazelrc",
        "startup foo \\ # comment\nbar"
    )],
    "bad_startup_line_continuation.bazelrc",
    [("startup", ["foo", " "])]
);

// Java: rc_options_test.cc RcOptionsTest.BadStartupLineContinuation_InterpretsNextLineAsNewline
rc_test!(
    bad_startup_line_continuation_interprets_next_line_as_newline,
    [(
        "bad_startup_line_continuation.bazelrc",
        "startup foo \\ #comment\nbar baz"
    )],
    "bad_startup_line_continuation.bazelrc",
    [("startup", ["foo", " "]), ("bar", ["baz"])]
);

// Java: rc_options_test.cc RemoteFileTest.ParsingRemoteFiles
#[test]
fn parsing_remote_files() {
    let read_calls = RefCell::new(Vec::new());
    let canon_calls = RefCell::new(Vec::new());
    let read_file = |f: &str| -> Result<String, String> {
        read_calls.borrow_mut().push(f.to_string());
        match f {
            "the base file" => Ok("import second.bazelrc".to_string()),
            "second.bazelrc" => Ok("startup --max_idle_secs=123".to_string()),
            _ => panic!("unexpected read of {}", f),
        }
    };
    let canonicalize_path = |f: &str| -> String {
        canon_calls.borrow_mut().push(f.to_string());
        f.to_string()
    };
    let test_build_label = "8.4.2";
    let sem_ver = SemVer::parse(test_build_label);
    let rcfile = RcFile::parse(
        "the base file",
        "my workspace",
        test_build_label,
        sem_ver.as_ref(),
        &read_file,
        &canonicalize_path,
    )
    .unwrap();
    // Each mock is expected to be called exactly once per file.
    assert_eq!(
        *read_calls.borrow(),
        vec!["the base file", "second.bazelrc"]
    );
    assert_eq!(
        *canon_calls.borrow(),
        vec!["the base file", "second.bazelrc"]
    );
    assert_eq!(
        rcfile.canonical_source_paths(),
        &["the base file".to_string(), "second.bazelrc".to_string()]
    );
    assert_eq!(rcfile.options().len(), 1);
    let startup: Vec<&str> = rcfile.options()["startup"]
        .iter()
        .map(|o| o.option.as_str())
        .collect();
    assert_eq!(startup, vec!["--max_idle_secs=123"]);
}

// Java: rc_options_test.cc RcOptionsTest.StartupConfigurationPlatformSpecific
rc_test!(
    startup_configuration_platform_specific,
    [(
        "startup_config.bazelrc",
        "startup --max_idle_secs=10800  # Default\n\
         startup:linux --max_idle_secs=3600  # Linux specific\n\
         startup:linux --connect_timeout_secs=60  # Linux specific\n\
         startup:macos --max_idle_secs=1800  # macOS Specific\n\
         startup:macos --connect_timeout_secs=120  # macOS Specific\n"
    )],
    "startup_config.bazelrc",
    [
        ("startup", ["--max_idle_secs=10800"]),
        (
            "startup:linux",
            ["--max_idle_secs=3600", "--connect_timeout_secs=60"]
        ),
        (
            "startup:macos",
            ["--max_idle_secs=1800", "--connect_timeout_secs=120"]
        ),
    ]
);

// Java: rc_options_test.cc RcOptionsTest.StartupConfigurationMultipleOptionsForPlatform
rc_test!(
    startup_configuration_multiple_options_for_platform,
    [(
        "startup_config_multiple.bazelrc",
        "startup:linux --host_jvm_args=-Xms256m\n\
         startup:linux --host_jvm_args=-Xmx2g\n\
         startup:linux --connect_timeout_secs=120\n"
    )],
    "startup_config_multiple.bazelrc",
    [(
        "startup:linux",
        [
            "--host_jvm_args=-Xms256m",
            "--host_jvm_args=-Xmx2g",
            "--connect_timeout_secs=120"
        ]
    )]
);

// Blaze: the C++ client keeps the rc file's raw bytes; Blaze holds them as an internal string
// (one char per byte). A non-ASCII comment is skipped, a value keeps its bytes (`é` is C3 A9,
// `🐷` is F0 9F 90 B7, FF is not UTF-8), a quoted value keeps its spaces, and a byte A0 (which
// Rust's `char::is_whitespace` would take for U+00A0) is not a separator.
#[test]
fn non_ascii_bytes_are_kept_one_char_per_byte() {
    let t = RcOptionsTest::new("non_ascii_bytes_are_kept_one_char_per_byte");
    let path = join(&t.test_file_dir, "non_ascii.bazelrc");
    std::fs::write(
        &path,
        b"# comment \xc3\xa9 \xf0\x9f\x90\xb7 \xff\n\
          build --define=x=\xc3\xa9 # trailing \xc3\xa9\n\
          build '--define=y=a \xf0\x9f\x90\xb7'\n\
          build --z=\xff\xa0w\n",
    )
    .unwrap();
    let rc = t.parse("non_ascii.bazelrc").unwrap();
    let got: Vec<&str> = rc.options()["build"]
        .iter()
        .map(|o| o.option.as_str())
        .collect();
    assert_eq!(
        got,
        [
            "--define=x=\u{c3}\u{a9}",
            "--define=y=a \u{f0}\u{9f}\u{90}\u{b7}",
            "--z=\u{ff}\u{a0}w",
        ]
    );
}
