//! Bazel's static checks: DotBazelFileSyntaxChecker (BUILD), PackageFactory.checkBuildSyntax
//! extraction, Resolver top-level binding and load rules, .scl ASCII literals.

use blaze_starlark::FileKind;
use blaze_starlark::eval::compile;

fn errors(kind: FileKind, name: &str, src: &str) -> Vec<String> {
    match compile(kind, name, src.to_owned()) {
        Ok(_) => Vec::new(),
        Err(e) => e.message_with_stack().lines().map(str::to_owned).collect(),
    }
}

#[test]
fn build_file_restrictions() {
    let src = "\
load(\":a.bzl\", \"m\")
def f():
    pass
x = lambda: 1
for i in []:
    m(*[1])
if True:
    m(*[1])
m(**{\"a\": 1})
m(**d)
";
    assert_eq!(
        errors(FileKind::Build, "BUILD", src),
        [
            "BUILD:2:1: functions may not be defined in BUILD files. You may move the function to a .bzl file and load it.",
            "BUILD:4:5: functions may not be defined in BUILD files. You may move the function to a .bzl file and load it.",
            "BUILD:5:1: `for` statements are not allowed in BUILD files. You may inline the loop, move it to a function definition (in a .bzl file), or as a last resort use a list comprehension.",
            "BUILD:7:1: `if` statements are not allowed in BUILD files. You may move conditional logic to a function definition (in a .bzl file), or use an `if` expression for simple cases.",
            "BUILD:9:3: **kwargs arguments are not allowed in BUILD files. Pass the arguments in explicitly.",
            "BUILD:10:3: **kwargs arguments are not allowed in BUILD files. Pass the arguments in explicitly.",
            "BUILD:10:3: **kwargs arguments must be a literal dict in BUILD files.",
        ]
    );
}

#[test]
fn repo_and_module_files() {
    assert_eq!(
        errors(FileKind::RepoFile, "REPO.bazel", "load(':a.bzl', 'x')\n"),
        ["REPO.bazel:1:1: `load` statements may not be used in REPO.bazel files"]
    );
    // MODULE.bazel allows **{literal dict}.
    assert!(errors(FileKind::ModuleFile, "MODULE.bazel", "m(**{'a': 1})\n").is_empty());
    assert_eq!(
        errors(FileKind::ModuleFile, "MODULE.bazel", "def f():\n  pass\n"),
        ["MODULE.bazel:1:1: functions may not be defined in MODULE.bazel files."]
    );
}

#[test]
fn build_file_facts() {
    let src = "\
cc_library(name = \"lib\", srcs = glob([\"*.cc\", x], exclude = [\"a.cc\"]))
filegroup(name = \"all\", srcs = glob(include = [\"**\"], exclude_directories = 0))
subpackages(include = [\"sub/**\"])
my_macro(
    name = \"m\",
)
";
    let c = compile(FileKind::Build, "BUILD", src.to_owned()).unwrap();
    let f = c.build_facts;
    assert_eq!(f.globs, ["*.cc"]);
    assert_eq!(f.globs_with_dirs, ["**"]);
    assert_eq!(f.subpackages, ["sub/**"]);
    let names: Vec<String> = f
        .generator_name_by_location
        .iter()
        .map(|(l, n)| format!("{l} {n}"))
        .collect();
    assert_eq!(names, ["BUILD:1:11 lib", "BUILD:2:10 all", "BUILD:4:9 m"]);
}

#[test]
fn bzl_load_must_come_first() {
    assert_eq!(
        errors(
            FileKind::Bzl,
            "a.bzl",
            "\"\"\"doc\"\"\"\nx = 1\nload(':b.bzl', 'y')\n"
        ),
        [
            "a.bzl:3:1: load statements must appear before any other statement",
            "a.bzl:2:1: \tfirst non-load statement appears here",
        ]
    );
    // BUILD files do not require it.
    assert!(errors(FileKind::Build, "BUILD", "x = 1\nload(':b.bzl', 'y')\n").is_empty());
}

#[test]
fn bzl_toplevel_rebinding() {
    assert_eq!(
        errors(FileKind::Bzl, "a.bzl", "x = 1\ndef x():\n  pass\n"),
        [
            "a.bzl:2:5: 'x' redeclared at top level",
            "a.bzl:1:1: 'x' previously declared here",
        ]
    );
    assert_eq!(
        errors(FileKind::Bzl, "a.bzl", "x = 1\nx += 1\n"),
        [
            "a.bzl:2:1: 'x' redeclared at top level",
            "a.bzl:1:1: 'x' previously declared here",
        ]
    );
    // BUILD files allow rebinding, also of loaded names.
    assert!(
        errors(
            FileKind::Build,
            "BUILD",
            "load(':b.bzl', 'x')\nx = 1\nx = 2\n"
        )
        .is_empty()
    );
}

#[test]
fn bzl_load_global_conflict() {
    assert_eq!(
        errors(FileKind::Bzl, "a.bzl", "load(':b.bzl', 'x')\nx = 1\n"),
        [
            "a.bzl:2:1: conflicting global declaration of 'x'",
            // Java Parser.parseLoadSymbol: a quoted name's identifier starts after the quote
            // (Parser.java:1441 `token.start + 1`); interp case err_res_conflict_load_global.
            "a.bzl:1:17: 'x' previously declared as file-local here",
        ]
    );
}

#[test]
fn scl_string_literals_are_ascii_only() {
    assert_eq!(
        errors(FileKind::Scl, "a.scl", "x = \"caf\u{e9}\"\n"),
        ["a.scl:1:9: string literal contains non-ASCII character"]
    );
    assert!(errors(FileKind::Bzl, "a.bzl", "x = \"caf\u{e9}\"\n").is_empty());
}

#[test]
fn bzl_rejects_toplevel_if_and_for() {
    assert!(!errors(FileKind::Bzl, "a.bzl", "if True:\n  pass\n").is_empty());
    assert!(!errors(FileKind::Bzl, "a.bzl", "for x in []:\n  pass\n").is_empty());
}

/// Errors of `src` compiled as a .bzl file with type syntax (BZL_COMPILE's `allowTypeSyntax`).
fn typed_errors(src: &str) -> Vec<String> {
    let mut options = FileKind::Bzl.file_options();
    options.allow_type_syntax = true;
    match blaze_starlark::eval::compile_with_options(FileKind::Bzl, options, "a.bzl", src.into()) {
        Ok(_) => Vec::new(),
        Err(e) => e.message_with_stack().lines().map(str::to_owned).collect(),
    }
}

/// Java Resolver.java:562 (a var statement binds its name) and :1228 (an annotation only at the
/// first declaration); toplevelRebinding as for any other binding.
#[test]
fn bzl_var_statement_bindings() {
    assert!(typed_errors("x: int\ny: list[str] = []\n").is_empty());
    assert_eq!(
        typed_errors("x: int\nx = 1\n"),
        [
            "a.bzl:2:1: 'x' redeclared at top level",
            "a.bzl:1:1: 'x' previously declared here",
        ]
    );
    assert_eq!(
        typed_errors("x = 1\nx: int\n"),
        [
            "a.bzl:2:1: 'x' redeclared at top level",
            "a.bzl:1:1: 'x' previously declared here",
            "a.bzl:2:1: type annotation on 'x' may only appear at its first declaration",
            "a.bzl:1:1: 'x' first declared here",
        ]
    );
    // Var statements inside a function are allowed and bind a local.
    assert!(typed_errors("def f():\n    y: int\n    y = 1\n    return y\n").is_empty());
}

/// Bazel 9.3.2 accepts `cast` expressions in .bzl files (type syntax on for BZL_COMPILE;
/// oracle: interp case lex_cast_argument, accepted with print output `3 [1] [4]`).
#[test]
fn bzl_cast_expressions_with_type_syntax() {
    let mut options = FileKind::Bzl.file_options();
    options.allow_type_syntax = true;
    let r = blaze_starlark::eval::compile_with_options(
        FileKind::Bzl,
        options,
        "case.bzl",
        "probe_ok = 1\nprint(cast(int, 3), str(cast(list[int], [1])), [cast(int, 4)])\n".to_owned(),
    );
    assert!(r.is_ok(), "{}", r.err().unwrap());
}

/// `resolve` reads the file with the type syntax setting it was compiled with: `cast` is a
/// keyword there, not an undefined name.
#[test]
fn resolve_uses_the_compiled_type_syntax() {
    let mut options = FileKind::Bzl.file_options();
    options.allow_type_syntax = true;
    let compiled = blaze_starlark::eval::compile_with_options(
        FileKind::Bzl,
        options,
        "case.bzl",
        "x = cast(int, 3)\n".to_owned(),
    )
    .unwrap();
    let globals = blaze_starlark::env::NativeRegistry::with_blaze_starlark_builtins()
        .build(FileKind::Bzl, &blaze_starlark::StarlarkSemantics::default());
    assert!(blaze_starlark::eval::resolve(&compiled, &globals).is_ok());
}

/// Java Parser.tokenString prints a doc comment's text (DocComments.toString): oracle probes
/// c06 (`x = #: doc` in a BUILD file) and c07 (`if x #: doc` in a .bzl file) of loading-fix-1.
#[test]
fn doc_comment_token_in_syntax_error() {
    assert_eq!(
        errors(FileKind::Build, "BUILD", "x = #: doc\n"),
        ["BUILD:1:5: syntax error at '#: doc': expected expression"]
    );
    assert_eq!(
        errors(
            FileKind::Bzl,
            "a.bzl",
            "def f(x):\n    if x #: doc\n        pass\n"
        ),
        ["a.bzl:2:10: syntax error at '#: doc': expected :"]
    );
}

/// A .bzl file's parse errors are followed by the resolver's errors on the recovered tree
/// (BzlCompileFunction.java:234, Resolver.java:946 "contains syntax errors"); a BUILD file's
/// parse errors come alone (PackageFunction.java:1367). Expected lines: oracle probes s01, s02
/// of loading-fix-1.
#[test]
fn bzl_parse_errors_then_resolver_errors() {
    assert_eq!(
        errors(
            FileKind::Bzl,
            "a.bzl",
            "x = (1 +)\ny = 1\ny = 2\nz = [i for i in undefined]\n"
        ),
        [
            "a.bzl:1:9: syntax error at ')': expected expression",
            "a.bzl:1:5: contains syntax errors",
        ]
    );
    assert_eq!(
        errors(FileKind::Build, "BUILD", "x = (1 +)\ny = undefined\n"),
        ["BUILD:1:9: syntax error at ')': expected expression"]
    );
}
