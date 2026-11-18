//! Port of src/test/java/com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFileTest.java.

use blaze_bzlmod::module_file::IncludeStatement;
use blaze_bzlmod::module_file::compiled_module_file::check_module_file_syntax;
use blaze_events::Location;
use blaze_starlark::error::syntax_errors_message;
use blaze_starlark::{FileKind, SyntaxError};
use starlark::syntax::AstModule;

fn check_syntax(str_: &str) -> Result<Vec<IncludeStatement>, Vec<SyntaxError>> {
    let file = AstModule::parse(
        "test file",
        str_.to_owned(),
        &FileKind::ModuleFile.dialect(),
    )
    .expect("parses");
    check_module_file_syntax(&file)
}

/// `assertThrows(SyntaxError.Exception.class, ...)`; returns the exception message.
fn check_syntax_error(str_: &str) -> String {
    match check_syntax(str_) {
        Ok(includes) => panic!("expected a syntax error, got {includes:?}"),
        Err(errors) => syntax_errors_message(&errors),
    }
}

fn include(label: &str, dev_dependency: bool, line: i32, column: i32) -> IncludeStatement {
    IncludeStatement {
        include_label: label.to_owned(),
        dev_dependency,
        location: Location::from_file_line_column("test file", line, column),
    }
}

#[test]
fn check_syntax_good() {
    let program = "abc()\ninclude(\"hullo\")\nfoo = bar\n";
    assert_eq!(
        check_syntax(program).unwrap(),
        vec![include("hullo", false, 2, 1)]
    );
}

#[test]
fn check_syntax_good_multiple() {
    let program = "abc()\ninclude(\"hullo\")\nfoo = bar\ninclude('world')\n";
    assert_eq!(
        check_syntax(program).unwrap(),
        vec![include("hullo", false, 2, 1), include("world", false, 4, 1)]
    );
}

#[test]
fn check_syntax_good_multiline_literal() {
    let program = "abc()\n# Ludicrous as this may be, it's still valid syntax. Your funeral, etc...\ninclude(\"\"\"hullo\nworld\"\"\")\n";
    assert_eq!(
        check_syntax(program).unwrap(),
        vec![include("hullo\nworld", false, 3, 1)]
    );
}

#[test]
fn check_syntax_good_dev_dependency() {
    let program = "include(\"dev.MODULE.bazel\", dev_dependency = True)\ninclude(\"prod.MODULE.bazel\", dev_dependency = False)\n";
    assert_eq!(
        check_syntax(program).unwrap(),
        vec![
            include("dev.MODULE.bazel", true, 1, 1),
            include("prod.MODULE.bazel", false, 2, 1)
        ]
    );
}

#[test]
fn check_syntax_good_benign_usage_of_include() {
    let program = "myext = use_extension('whatever')\nmyext.include(include=\"hullo\")\n";
    assert!(check_syntax(program).unwrap().is_empty());
}

#[test]
fn check_syntax_good_include_identifier_reassigned() {
    let program = "include('world')\ninclude = print\n# from this point on, we no longer check anything about `include` usage.\ninclude('hello')\nstr(include)\nexclude = include\n";
    assert_eq!(
        check_syntax(program).unwrap(),
        vec![include("world", false, 1, 1)]
    );
}

#[test]
fn check_syntax_bad_if() {
    let program = "abc()\nif d > 3:\n  pass\n";
    assert!(
        check_syntax_error(program)
            .contains("`if` statements are not allowed in MODULE.bazel files")
    );
}

#[test]
fn check_syntax_bad_assign_include_result() {
    let program = "foo = include('hello')\n";
    assert!(
        check_syntax_error(program)
            .contains("the `include` directive MUST be called directly at the top-level")
    );
}

#[test]
fn check_syntax_bad_assign_include_identifier() {
    let program = "foo = include\nfoo('hello')\n";
    assert!(
        check_syntax_error(program)
            .contains("the `include` directive MUST be called directly at the top-level")
    );
}

#[test]
fn check_syntax_bad_multiple_arguments_to_include() {
    let program = "include('hello', 'world')\n";
    assert!(
        check_syntax_error(program)
            .contains("the `include` directive MUST be called with one positional string literal")
    );
}

#[test]
fn check_syntax_bad_keyword_argument_to_include() {
    let program = "include(label='hello')\n";
    assert!(
        check_syntax_error(program)
            .contains("the `include` directive MUST be called with one positional string literal")
    );
}

#[test]
fn check_syntax_bad_non_literal_argument_to_include() {
    let program = "foo = 'hello'\ninclude(foo)\n";
    assert!(
        check_syntax_error(program)
            .contains("the `include` directive MUST be called with one positional string literal")
    );
}

#[test]
fn check_syntax_bad_non_literal_dev_dependency() {
    let program = "dev = True\ninclude('hello', dev_dependency = dev)\n";
    assert!(
        check_syntax_error(program).contains("optionally, `dev_dependency` as a boolean literal")
    );
}

/// `CompiledModuleFile` crosses threads and Skyframe nodes.
#[test]
fn compiled_module_file_is_send_sync_clone() {
    fn assert_traits<T: Send + Sync + Clone + std::fmt::Debug>() {}
    assert_traits::<blaze_bzlmod::module_file::CompiledModuleFile>();
    fn assert_send<T: Send>() {}
    assert_send::<blaze_bzlmod::module_file::ModuleThreadContext>();
}
